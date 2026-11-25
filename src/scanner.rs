use crate::{core, db};
use anyhow::Result;
use jwalk::WalkDir;
use rusqlite::{params, Connection};
use std::collections::HashSet;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Default)]
pub struct ScanStats {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    /// Espaço realmente alocado (st_blocks × 512), sem contar hardlinks repetidos.
    pub alloc: u64,
    pub removed: usize,
    pub errors: u64,
    pub unreadable: u64,
    /// Falso quando a purga foi adiada por causa de erros de leitura.
    pub purged: bool,
    pub stale: u64,
}

const UPSERT: &str = "
INSERT INTO files(path,parent,name,ext,kind,size,alloc,mtime,dev,ino,nlink,hash,status,scan)
VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,NULL,'ok',?12)
ON CONFLICT(path) DO UPDATE SET
    parent = excluded.parent,
    name   = excluded.name,
    ext    = excluded.ext,
    kind   = excluded.kind,
    hash   = CASE WHEN files.size = excluded.size AND files.mtime = excluded.mtime
                       AND files.kind = excluded.kind
                  THEN files.hash ELSE NULL END,
    size   = excluded.size,
    alloc  = excluded.alloc,
    mtime  = excluded.mtime,
    dev    = excluded.dev,
    ino    = excluded.ino,
    nlink  = excluded.nlink,
    status = 'ok',
    scan   = excluded.scan";

pub fn scan(conn: &mut Connection, root: &Path) -> Result<ScanStats> {
    let root = root.canonicalize()?;
    let root_str = root.to_string_lossy().into_owned();
    let scan_id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0);
    let started = core::now_secs();
    let t0 = Instant::now();

    let mut stats = ScanStats {
        purged: true,
        ..Default::default()
    };
    // hardlinks apontam para o mesmo espaço: conta alloc só uma vez
    let mut seen_inodes: HashSet<(i64, i64)> = HashSet::new();

    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare(UPSERT)?;
        let mut walker = WalkDir::new(&root)
            .skip_hidden(false)
            .follow_links(false)
            .into_iter();
        while let Some(entry) = walker.next() {
            let entry = match entry {
                Ok(e) => e,
                Err(err) => {
                    stats.errors += 1;
                    stats.purged = false;
                    if let Some(p) = err.path() {
                        db::record_scan_error(
                            &tx,
                            &p.to_string_lossy(),
                            scan_id,
                            &err.to_string(),
                        )?;
                    }
                    continue;
                }
            };
            let path = entry.path();
            let ft = entry.file_type();
            // jwalk não devolve Err quando falha ao LISTAR uma subpasta:
            // o erro vem anexado à entrada do próprio diretório.
            let read_error = entry.read_children_error.as_ref().map(|e| e.to_string());

            // symlink_metadata: nunca segue links, então o link é indexado como link
            let md = match std::fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(err) => {
                    stats.errors += 1;
                    stats.purged = false;
                    let p = path.to_string_lossy();
                    db::record_scan_error(&tx, &p, scan_id, &err.to_string())?;
                    db::mark_unreadable(&tx, &p, scan_id)?;
                    stats.unreadable += 1;
                    continue;
                }
            };

            let (kind, size, alloc) = if ft.is_dir() {
                (("dir"), 0, 0)
            } else if ft.is_symlink() {
                ("symlink", md.len(), md.blocks() * 512)
            } else if ft.is_file() {
                ("file", md.len(), md.blocks() * 512)
            } else {
                ("other", md.len(), md.blocks() * 512)
            };

            let parent = path
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| root_str.clone());
            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();

            stmt.execute(params![
                path.to_string_lossy().as_ref(),
                parent,
                name,
                ext,
                kind,
                size as i64,
                alloc as i64,
                md.mtime(),
                md.dev() as i64,
                md.ino() as i64,
                md.nlink() as i64,
                scan_id
            ])?;

            if let Some(err) = read_error {
                stats.errors += 1;
                stats.purged = false;
                stats.unreadable += 1;
                let p = path.to_string_lossy();
                db::record_scan_error(&tx, &p, scan_id, &err)?;
                db::mark_unreadable(&tx, &p, scan_id)?;
            }

            match kind {
                "file" => {
                    stats.files += 1;
                    // hardlinks: mesmo conteúdo conta uma vez (como o `du`)
                    if seen_inodes.insert((md.dev() as i64, md.ino() as i64)) {
                        stats.bytes += size;
                        stats.alloc += alloc;
                    }
                }
                "symlink" => {
                    if seen_inodes.insert((md.dev() as i64, md.ino() as i64)) {
                        stats.bytes += size;
                        stats.alloc += alloc;
                    }
                }
                "dir" => stats.dirs += 1,
                _ => {}
            }
        }
    }

    // Purga: só remove entradas ausentes se a varredura não encontrou erros.
    // Com qualquer erro de leitura uma subárvore pode estar invisível, então
    // preservar o índice é mais seguro do que apagar arquivos que ainda existem.
    stats.stale = db::stale_count(&tx, &root_str, scan_id)?;
    if stats.errors == 0 {
        let (lo, hi) = core::subtree_bounds(&root_str);
        stats.removed = tx.execute(
            "DELETE FROM files WHERE scan != ?1 AND (path = ?2 OR (path >= ?3 AND path < ?4))",
            params![scan_id, root_str, lo, hi],
        )?;
    } else {
        stats.purged = false;
    }

    tx.execute(
        "INSERT INTO scan_runs(root, started, elapsed_ms, files, dirs, bytes, alloc, errors, purged, removed)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            root_str,
            started,
            t0.elapsed().as_millis() as i64,
            stats.files as i64,
            stats.dirs as i64,
            stats.bytes as i64,
            stats.alloc as i64,
            stats.errors as i64,
            stats.purged as i64,
            stats.removed as i64
        ],
    )?;
    tx.commit()?;
    Ok(stats)
}
