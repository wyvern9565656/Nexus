use anyhow::Result;
use rusqlite::{params, Connection};
use std::collections::HashSet;
use std::path::Path;

const CREATE_FILES: &str = "
CREATE TABLE IF NOT EXISTS files(
    path   TEXT PRIMARY KEY,
    parent TEXT NOT NULL,
    name   TEXT NOT NULL DEFAULT '',
    ext    TEXT NOT NULL DEFAULT '',
    kind   TEXT NOT NULL DEFAULT 'file',
    size   INTEGER NOT NULL DEFAULT 0,
    alloc  INTEGER NOT NULL DEFAULT 0,
    mtime  INTEGER NOT NULL DEFAULT 0,
    dev    INTEGER NOT NULL DEFAULT 0,
    ino    INTEGER NOT NULL DEFAULT 0,
    nlink  INTEGER NOT NULL DEFAULT 1,
    hash   TEXT,
    status TEXT NOT NULL DEFAULT 'ok',
    scan   INTEGER NOT NULL DEFAULT 0
);";

pub fn open(path: &Path) -> Result<Connection> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         PRAGMA busy_timeout=5000;",
    )?;
    migrate(&conn)?;
    Ok(conn)
}

/// Cria o schema atual e atualiza índices antigos (v1) sem perder dados.
fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(CREATE_FILES)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS favorites(
             path  TEXT PRIMARY KEY,
             added INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS scan_runs(
             id        INTEGER PRIMARY KEY,
             root      TEXT NOT NULL,
             started   INTEGER NOT NULL,
             elapsed_ms INTEGER NOT NULL,
             files     INTEGER NOT NULL,
             dirs      INTEGER NOT NULL,
             bytes     INTEGER NOT NULL,
             alloc     INTEGER NOT NULL,
             errors    INTEGER NOT NULL,
             purged    INTEGER NOT NULL,
             removed   INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS scan_errors(
             path    TEXT PRIMARY KEY,
             scan    INTEGER NOT NULL,
             message TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS settings(
             key   TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );",
    )?;

    let cols: HashSet<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(files)")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
        rows.collect::<std::result::Result<_, _>>()?
    };
    let missing: &[(&str, &str)] = &[
        (
            "name",
            "ALTER TABLE files ADD COLUMN name TEXT NOT NULL DEFAULT ''",
        ),
        (
            "kind",
            "ALTER TABLE files ADD COLUMN kind TEXT NOT NULL DEFAULT 'file'",
        ),
        (
            "alloc",
            "ALTER TABLE files ADD COLUMN alloc INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "nlink",
            "ALTER TABLE files ADD COLUMN nlink INTEGER NOT NULL DEFAULT 1",
        ),
        (
            "status",
            "ALTER TABLE files ADD COLUMN status TEXT NOT NULL DEFAULT 'ok'",
        ),
    ];
    for (col, ddl) in missing {
        if !cols.contains(*col) {
            conn.execute_batch(ddl)?;
        }
    }

    // preenche `name` das linhas migradas (schema v1 só guardava o path)
    {
        let mut stmt = conn.prepare("SELECT path FROM files WHERE name = ''")?;
        let paths = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut up = conn.prepare("UPDATE files SET name = ?1 WHERE path = ?2")?;
        for path in paths {
            let name = Path::new(&path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone());
            up.execute(params![name, path])?;
        }
    }

    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_files_size   ON files(size);
         CREATE INDEX IF NOT EXISTS idx_files_parent ON files(parent);
         CREATE INDEX IF NOT EXISTS idx_files_ext    ON files(ext);
         CREATE INDEX IF NOT EXISTS idx_files_kind   ON files(kind, status);
         PRAGMA user_version = 2;",
    )?;
    Ok(())
}

pub fn favorites(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT path FROM favorites ORDER BY added")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

pub fn get_setting(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
        r.get::<_, String>(0)
    })
    .ok()
}

pub fn folder_colors(conn: &Connection) -> Result<Vec<(String, String)>> {
    let mut stmt = conn
        .prepare("SELECT key, value FROM settings WHERE key LIKE 'folder_color:%' ORDER BY key")?;
    let rows = stmt.query_map([], |row| {
        let key: String = row.get(0)?;
        let value: String = row.get(1)?;
        Ok((key[13..].to_string(), value))
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

pub fn remove_setting(conn: &Connection, key: &str) -> Result<()> {
    conn.execute("DELETE FROM settings WHERE key = ?1", [key])?;
    Ok(())
}

pub fn set_settings(conn: &Connection, pairs: &[(&str, String)]) -> Result<()> {
    let mut stmt = conn.prepare(
        "INSERT INTO settings(key, value) VALUES(?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )?;
    for (k, v) in pairs {
        stmt.execute(params![k, v])?;
    }
    Ok(())
}

pub fn add_favorite(conn: &Connection, path: &str) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO favorites(path, added) VALUES(?1, ?2)",
        params![path, crate::core::now_secs()],
    )?;
    Ok(())
}

pub fn remove_favorite(conn: &Connection, path: &str) -> Result<()> {
    conn.execute("DELETE FROM favorites WHERE path = ?1", [path])?;
    Ok(())
}

pub fn record_scan_error(conn: &Connection, path: &str, scan: i64, message: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO scan_errors(path, scan, message) VALUES(?1, ?2, ?3)
         ON CONFLICT(path) DO UPDATE SET scan = excluded.scan, message = excluded.message",
        params![path, scan, message],
    )?;
    Ok(())
}

/// Marca como inacessíveis os paths que já estavam no índice mas falharam
/// nesta varredura. Eles não são removidos.
pub fn mark_unreadable(conn: &Connection, path: &str, scan: i64) -> Result<()> {
    conn.execute(
        "UPDATE files SET scan = ?1, status = 'unreadable' WHERE path = ?2",
        params![scan, path],
    )?;
    Ok(())
}

/// Contagem de entradas que não foram vistas nesta varredura (só consulta).
pub fn stale_count(conn: &Connection, root: &str, scan: i64) -> Result<u64> {
    let (lo, hi) = crate::core::subtree_bounds(root);
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM files
         WHERE scan != ?1 AND (path = ?2 OR (path >= ?3 AND path < ?4))",
        params![scan, root, lo, hi],
        |r| r.get(0),
    )?;
    Ok(n as u64)
}
