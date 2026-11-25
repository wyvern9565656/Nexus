use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Inicia o processo e faz `wait` em segundo plano, sem deixar zumbis.
pub fn spawn_and_reap(mut command: std::process::Command) -> io::Result<()> {
    let mut child = command.spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

pub fn human(b: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < 4 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

pub fn default_db() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("nexus/index.db")
}

pub fn expand_home(s: &str) -> String {
    let s = s.trim();
    if s == "~" {
        return std::env::var("HOME").unwrap_or_else(|_| "/".into());
    }
    if let Some(rest) = s.strip_prefix("~/") {
        if let Ok(h) = std::env::var("HOME") {
            return format!("{h}/{rest}");
        }
    }
    s.to_string()
}

pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Faixa de paths que cobre `root` e tudo abaixo dele.
/// `lo` = `root/` e `hi` é o menor prefixo seguinte (`root0`), então
/// `path >= lo AND path < hi` usa o índice da chave primária.
pub fn subtree_bounds(root: &str) -> (String, String) {
    let mut lo = root.to_string();
    if !lo.ends_with('/') {
        lo.push('/');
    }
    let mut hi = lo.clone();
    hi.pop();
    hi.push('0'); // '0' (0x30) vem logo depois de '/' (0x2F)
    (lo, hi)
}

pub fn to_secs(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Data UTC no formato aceito pelo spec da lixeira (ISO 8601, segundos).
pub fn iso_utc(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Howard Hinnant, days_from_civil invertido.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    if mo <= 2 {
        y += 1;
    }
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}")
}

fn percent_encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        let keep = b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~');
        if keep {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn move_path(from: &Path, to: &Path) -> io::Result<()> {
    match fs::rename(from, to) {
        Ok(()) => return Ok(()),
        // Só cai em copiar+apagar quando o destino está em outro sistema de arquivos.
        Err(error)
            if error.kind() == io::ErrorKind::CrossesDevices
                || error.raw_os_error() == Some(18) => {}
        Err(error) => return Err(error),
    }
    let md = fs::symlink_metadata(from)?;
    if md.is_dir() {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            move_path(&entry.path(), &to.join(entry.file_name()))?;
        }
        fs::remove_dir(from)
    } else if md.file_type().is_symlink() {
        #[cfg(unix)]
        {
            let target = fs::read_link(from)?;
            std::os::unix::fs::symlink(target, to)
        }
        #[cfg(not(unix))]
        {
            Err(io::Error::new(io::ErrorKind::Unsupported, "symlink"))
        }
    } else {
        fs::copy(from, to)?;
        fs::remove_file(from)
    }
}

/// Move um caminho para a lixeira do usuário (spec XDG, lixeira do home).
/// Nunca apaga de forma definitiva: se a lixeira não puder ser usada,
/// devolve erro e o arquivo fica onde está.
pub fn move_to_trash(path: &Path) -> io::Result<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME indefinido"))?;
    move_to_trash_in(&data, path)
}

fn move_to_trash_in(data: &Path, path: &Path) -> io::Result<PathBuf> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let trash = data.join("Trash");
    let files = trash.join("files");
    let info = trash.join("info");
    fs::create_dir_all(&files)?;
    fs::create_dir_all(&info)?;

    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "caminho sem nome"))?
        .to_os_string();
    // O nome precisa estar livre tanto em files/ quanto em info/.
    let mut n = 0_u32;
    let (dest, info_path) = loop {
        let nm = if n == 0 {
            name.to_string_lossy().into_owned()
        } else {
            format!("{}.{n}", name.to_string_lossy())
        };
        let dest = files.join(&nm);
        let info_path = info.join(format!("{nm}.trashinfo"));
        if !dest.exists() && !info_path.exists() {
            break (dest, info_path);
        }
        n += 1;
    };
    move_path(&path, &dest)?;
    let content = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        percent_encode_path(&path.to_string_lossy()),
        iso_utc(now_secs())
    );
    fs::write(&info_path, content)?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_cover_subtree() {
        let (lo, hi) = subtree_bounds("/tmp/root");
        assert_eq!(lo, "/tmp/root/");
        assert!(
            "/tmp/root/a".as_bytes() >= lo.as_bytes() && "/tmp/root/a".as_bytes() < hi.as_bytes()
        );
        assert!("/tmp/root2".as_bytes() >= hi.as_bytes());
    }

    #[test]
    fn iso_date_epoch() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00");
        assert_eq!(iso_utc(1_700_000_000), "2023-11-14T22:13:20");
    }

    #[test]
    fn trash_picks_name_free_in_files_and_info() {
        let root = std::env::temp_dir().join(format!(
            "nexus-trash-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let trash = root.join("Trash");
        fs::create_dir_all(trash.join("files")).unwrap();
        fs::create_dir_all(trash.join("info")).unwrap();
        // Info órfã: o nome está livre em files/ mas ocupado em info/.
        fs::write(trash.join("info/arquivo.txt.trashinfo"), "[Trash Info]\n").unwrap();
        let file = root.join("arquivo.txt");
        fs::write(&file, b"conteudo").unwrap();
        let dest = move_to_trash_in(&root, &file).unwrap();

        assert_eq!(dest, trash.join("files/arquivo.txt.1"));
        assert!(trash.join("info/arquivo.txt.1.trashinfo").exists());
        assert!(!file.exists());
        assert_eq!(fs::read(&dest).unwrap(), b"conteudo");
        fs::remove_dir_all(&root).unwrap();
    }
}
