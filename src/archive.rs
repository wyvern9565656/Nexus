use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Seek, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Zip,
    TarGz,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Zip => "zip",
            Format::TarGz => "tar.gz",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Format::Zip => "ZIP",
            Format::TarGz => "tar.gz",
        }
    }
}

pub fn is_archive(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    name.ends_with(".zip")
        || name.ends_with(".tar")
        || name.ends_with(".tar.gz")
        || name.ends_with(".tgz")
}

/// Compacta os itens em um arquivo novo dentro de `destination`.
pub fn compress(sources: &[PathBuf], destination: &Path, format: Format) -> Result<PathBuf> {
    if sources.is_empty() {
        anyhow::bail!("nenhum item selecionado");
    }
    let stem = archive_stem(sources, destination);
    let target = unique_path(&destination.join(format!("{stem}.{}", format.extension())));
    match format {
        Format::Zip => compress_zip(sources, &target)?,
        Format::TarGz => compress_tar_gz(sources, &target)?,
    }
    Ok(target)
}

fn archive_stem(sources: &[PathBuf], destination: &Path) -> String {
    if let [source] = sources {
        if let Some(name) = source.file_name() {
            let name = name.to_string_lossy().trim().to_string();
            if !name.is_empty() {
                return name;
            }
        }
    }
    destination
        .file_name()
        .map(|name| name.to_string_lossy().trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "arquivo".into())
}

fn compress_zip(sources: &[PathBuf], target: &Path) -> Result<()> {
    let file =
        File::create(target).with_context(|| format!("criar {}", target.display()))?;
    let mut writer = zip::ZipWriter::new(BufWriter::new(file));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);
    for source in sources {
        let Some(base) = source.file_name() else {
            continue;
        };
        add_to_zip(&mut writer, source, Path::new(base), options)?;
    }
    writer.finish()?;
    Ok(())
}

fn add_to_zip<W: Write + Seek>(
    writer: &mut zip::ZipWriter<W>,
    path: &Path,
    name: &Path,
    options: zip::write::SimpleFileOptions,
) -> Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() && fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false) {
        // não segue links para pastas (evita ciclos)
        return Ok(());
    }
    let meta = fs::metadata(path).unwrap_or(meta);
    let options = options.unix_permissions(meta.permissions().mode());
    let entry_name = zip_name(name);
    if meta.is_dir() {
        writer.add_directory(format!("{entry_name}/"), options)?;
        let mut children: Vec<_> = fs::read_dir(path)?.filter_map(|child| child.ok()).collect();
        children.sort_by_key(|child| child.file_name());
        for child in children {
            add_to_zip(writer, &child.path(), &name.join(child.file_name()), options)?;
        }
    } else {
        writer.start_file(entry_name, options)?;
        let mut input = BufReader::new(File::open(path)?);
        io::copy(&mut input, writer)?;
    }
    Ok(())
}

fn zip_name(name: &Path) -> String {
    name.components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn compress_tar_gz(sources: &[PathBuf], target: &Path) -> Result<()> {
    let file =
        File::create(target).with_context(|| format!("criar {}", target.display()))?;
    let encoder = GzEncoder::new(BufWriter::new(file), Compression::default());
    let mut builder = tar::Builder::new(encoder);
    for source in sources {
        let Some(base) = source.file_name() else {
            continue;
        };
        add_to_tar(&mut builder, source, Path::new(base))?;
    }
    let encoder = builder.into_inner()?;
    encoder.finish()?;
    Ok(())
}

fn add_to_tar<W: Write>(builder: &mut tar::Builder<W>, path: &Path, name: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() && fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false) {
        // não segue links para pastas (evita ciclos)
        return Ok(());
    }
    let meta = fs::metadata(path).unwrap_or(meta);
    if meta.is_dir() {
        builder.append_dir(name, path)?;
        let mut children: Vec<_> = fs::read_dir(path)?.filter_map(|child| child.ok()).collect();
        children.sort_by_key(|child| child.file_name());
        for child in children {
            add_to_tar(builder, &child.path(), &name.join(child.file_name()))?;
        }
    } else {
        builder.append_path_with_name(path, name)?;
    }
    Ok(())
}

/// Extrai o arquivo para uma pasta nova ao lado dele e devolve o caminho criado.
pub fn extract(archive_path: &Path) -> Result<PathBuf> {
    let parent = archive_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = folder_name(archive_path);
    let destination = unique_path(&parent.join(stem));
    let name = archive_path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    fs::create_dir_all(&destination)?;
    let result = if name.ends_with(".zip") {
        extract_zip(archive_path, &destination)
    } else if name.ends_with(".tar") {
        tar::Archive::new(BufReader::new(File::open(archive_path)?))
            .unpack(&destination)
            .map_err(Into::into)
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        tar::Archive::new(GzDecoder::new(BufReader::new(File::open(archive_path)?)))
            .unpack(&destination)
            .map_err(Into::into)
    } else {
        anyhow::bail!("formato não suportado: {}", archive_path.display());
    };
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&destination);
        return Err(error);
    }
    Ok(destination)
}

fn extract_zip(archive_path: &Path, destination: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let file = File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(BufReader::new(file))?;
    let mut dir_modes = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let Some(relative) = entry.enclosed_name() else {
            continue;
        };
        let out = destination.join(relative);
        // Só permissões de acesso: descarta setuid/setgid/sticky de zips de terceiros.
        let mode = entry.unix_mode().map(|mode| mode & 0o777);
        if entry.is_dir() {
            fs::create_dir_all(&out)?;
            if let Some(mode) = mode {
                dir_modes.push((out.clone(), mode));
            }
        } else {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut output = File::create(&out)?;
            io::copy(&mut entry, &mut output)?;
            if let Some(mode) = mode {
                fs::set_permissions(&out, fs::Permissions::from_mode(mode))?;
            }
        }
    }
    // Pastas só recebem as permissões no fim, para não bloquear os filhos.
    for (dir, mode) in dir_modes.into_iter().rev() {
        fs::set_permissions(dir, fs::Permissions::from_mode(mode))?;
    }
    Ok(())
}

fn folder_name(archive_path: &Path) -> String {
    let name = archive_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = split_extension(&name).0.trim().to_string();
    if stem.is_empty() {
        "extraido".into()
    } else {
        stem
    }
}

fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (stem, extension) = split_extension(&name);
    for number in 2..10_000 {
        let candidate = parent.join(format!("{stem} ({number}){extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn test_root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "nexus-archive-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn zip_roundtrip_keeps_executable_bit() {
        let root = test_root();
        let source = root.join("pasta");
        fs::create_dir_all(&source).unwrap();
        let script = source.join("rodar.sh");
        fs::write(&script, b"#!/bin/sh\necho ok\n").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

        let archive = compress(std::slice::from_ref(&source), &root, Format::Zip).unwrap();
        let extracted = extract(&archive).unwrap();
        let mode = fs::metadata(extracted.join("pasta/rodar.sh"))
            .unwrap()
            .permissions()
            .mode();
        assert_ne!(mode & 0o111, 0, "modo restaurado: {mode:o}");

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn zip_extraction_strips_special_permission_bits() {
        let root = test_root();
        let source = root.join("pasta");
        fs::create_dir_all(&source).unwrap();
        let script = source.join("rodar.sh");
        fs::write(&script, b"#!/bin/sh\n").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o4755)).unwrap();

        let archive = compress(std::slice::from_ref(&source), &root, Format::Zip).unwrap();
        let extracted = extract(&archive).unwrap();
        let mode = fs::metadata(extracted.join("pasta/rodar.sh"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o7777, 0o755, "modo extraído: {mode:o}");

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn tar_gz_roundtrip_keeps_contents() {
        let root = test_root();
        let source = root.join("pasta");
        fs::create_dir_all(source.join("sub")).unwrap();
        fs::write(source.join("a.txt"), b"a").unwrap();
        fs::write(source.join("sub/b.txt"), b"b").unwrap();

        let archive = compress(std::slice::from_ref(&source), &root, Format::TarGz).unwrap();
        let extracted = extract(&archive).unwrap();
        assert_eq!(fs::read(extracted.join("pasta/a.txt")).unwrap(), b"a");
        assert_eq!(fs::read(extracted.join("pasta/sub/b.txt")).unwrap(), b"b");

        fs::remove_dir_all(&root).unwrap();
    }
}

fn split_extension(name: &str) -> (String, String) {
    let lower = name.to_ascii_lowercase();
    for extension in [".tar.gz", ".tar.bz2", ".tar.xz"] {
        if lower.ends_with(extension) {
            let split = name.len() - extension.len();
            return (name[..split].to_string(), name[split..].to_string());
        }
    }
    match name.rfind('.') {
        Some(index) if index > 0 => (name[..index].to_string(), name[index..].to_string()),
        _ => (name.to_string(), String::new()),
    }
}
