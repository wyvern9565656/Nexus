use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};

pub enum Event {
    Progress { done: u64, total: u64 },
    Finished(Result<(), String>),
}

pub fn run(
    sources: Vec<PathBuf>,
    destination: PathBuf,
    move_sources: bool,
    paused: Arc<(Mutex<bool>, Condvar)>,
    cancelled: Arc<AtomicBool>,
    sender: Sender<Event>,
) {
    let result = transfer_all(
        &sources,
        &destination,
        move_sources,
        &paused,
        &cancelled,
        &sender,
    );
    let _ = sender.send(Event::Finished(result));
}

fn transfer_all(
    sources: &[PathBuf],
    destination: &Path,
    move_sources: bool,
    paused: &Arc<(Mutex<bool>, Condvar)>,
    cancelled: &AtomicBool,
    sender: &Sender<Event>,
) -> Result<(), String> {
    if !destination.is_dir() {
        return Err(format!(
            "Destino não é uma pasta: {}",
            destination.display()
        ));
    }
    let total = sources
        .iter()
        .try_fold(0_u64, |sum, path| {
            total_size(path).map(|size| sum.saturating_add(size))
        })
        .map_err(|error| error.to_string())?;
    let mut done = 0_u64;
    for source in sources {
        wait_if_paused(paused, cancelled)?;
        let name = source
            .file_name()
            .ok_or_else(|| format!("Caminho inválido: {}", source.display()))?;
        let target = destination.join(name);
        if target.exists() {
            return Err(format!("Destino já existe: {}", target.display()));
        }
        if source.is_dir() && destination.starts_with(source) {
            return Err(format!(
                "Não é possível copiar uma pasta para dentro dela: {}",
                source.display()
            ));
        }
        if move_sources {
            let size = total_size(source).unwrap_or(0);
            match fs::rename(source, &target) {
                // Mesma partição: mover é atômico e não duplica I/O.
                Ok(()) => {
                    done = done.saturating_add(size);
                    let _ = sender.send(Event::Progress { done, total });
                    continue;
                }
                Err(error) if is_cross_device(&error) => {}
                Err(error) => return Err(format!("{}: {error}", source.display())),
            }
        }
        if let Err(error) = copy_node(source, &target, &mut done, total, paused, cancelled, sender)
        {
            let _ = remove_node(&target);
            return Err(format!("{}: {error}", source.display()));
        }
        if move_sources {
            if let Err(error) = remove_node(source) {
                return Err(format!(
                    "Cópia concluída, mas não foi possível remover {}: {error}",
                    source.display()
                ));
            }
        }
    }
    let _ = sender.send(Event::Progress { done: total, total });
    Ok(())
}

fn is_cross_device(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::CrossesDevices || error.raw_os_error() == Some(18)
}

fn total_size(path: &Path) -> io::Result<u64> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Ok(metadata.len());
    }
    let mut total = 0_u64;
    for entry in fs::read_dir(path)? {
        total = total.saturating_add(total_size(&entry?.path())?);
    }
    Ok(total)
}

fn copy_node(
    source: &Path,
    destination: &Path,
    done: &mut u64,
    total: u64,
    paused: &Arc<(Mutex<bool>, Condvar)>,
    cancelled: &AtomicBool,
    sender: &Sender<Event>,
) -> io::Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        std::os::unix::fs::symlink(fs::read_link(source)?, destination)?;
        return Ok(());
    }
    if metadata.is_dir() {
        fs::create_dir(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_node(
                &entry.path(),
                &destination.join(entry.file_name()),
                done,
                total,
                paused,
                cancelled,
                sender,
            )?;
        }
        fs::set_permissions(destination, metadata.permissions())?;
        return Ok(());
    }

    let mut input = File::open(source)?;
    let mut output = File::create(destination)?;
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        wait_if_paused(paused, cancelled)
            .map_err(|_| io::Error::new(io::ErrorKind::Interrupted, "transferência cancelada"))?;
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        output.write_all(&buffer[..count])?;
        *done = done.saturating_add(count as u64);
        let _ = sender.send(Event::Progress { done: *done, total });
    }
    output.flush()?;
    fs::set_permissions(destination, metadata.permissions())?;
    Ok(())
}

fn wait_if_paused(
    paused: &Arc<(Mutex<bool>, Condvar)>,
    cancelled: &AtomicBool,
) -> Result<(), String> {
    let (lock, wake) = &**paused;
    let mut is_paused = lock.lock().map_err(|error| error.to_string())?;
    while *is_paused && !cancelled.load(Ordering::Relaxed) {
        is_paused = wake.wait(is_paused).map_err(|error| error.to_string())?;
    }
    if cancelled.load(Ordering::Relaxed) {
        Err("transferência cancelada".into())
    } else {
        Ok(())
    }
}

fn remove_node(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_root() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("nexus-transfer-{}-{nonce}", std::process::id()))
    }

    #[test]
    fn moves_directory_tree_and_reports_completion() {
        let root = test_root();
        let source = root.join("source");
        let nested = source.join("nested");
        let destination = root.join("destination");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir_all(&destination).unwrap();
        fs::write(nested.join("data.txt"), b"nexus transfer").unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();

        run(
            vec![source.clone()],
            destination.clone(),
            true,
            Arc::new((Mutex::new(false), Condvar::new())),
            Arc::new(AtomicBool::new(false)),
            sender,
        );

        assert!(matches!(
            receiver.try_iter().last(),
            Some(Event::Finished(Ok(())))
        ));
        assert!(!source.exists());
        assert_eq!(
            fs::read(destination.join("source/nested/data.txt")).unwrap(),
            b"nexus transfer"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cancelled_transfer_does_not_create_destination() {
        let root = test_root();
        let source = root.join("source.txt");
        let destination = root.join("destination");
        fs::create_dir_all(&destination).unwrap();
        fs::write(&source, b"data").unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(true));

        run(
            vec![source.clone()],
            destination.clone(),
            false,
            Arc::new((Mutex::new(true), Condvar::new())),
            cancelled,
            sender,
        );

        assert!(matches!(
            receiver.try_iter().last(),
            Some(Event::Finished(Err(_)))
        ));
        assert!(!destination.join("source.txt").exists());
        assert!(source.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
