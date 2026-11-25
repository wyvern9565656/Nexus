use crate::core;
use anyhow::Result;
use rusqlite::{Connection, ToSql};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Name,
    Modified,
    Size,
}

pub struct Query {
    /// Trecho do nome do arquivo (vazio = todos).
    pub text: String,
    /// Extensão sem ponto (vazio = todas).
    pub ext: String,
    /// Tamanho mínimo em bytes.
    pub min_size: u64,
    /// Tamanho máximo em bytes (0 = sem limite).
    pub max_size: u64,
    /// Restringe a busca a uma pasta (e subpastas).
    pub root: Option<String>,
    /// Só arquivos (ignora diretórios).
    pub files_only: bool,
    pub sort: SortOrder,
    pub limit: usize,
}

impl Default for Query {
    fn default() -> Self {
        Query {
            text: String::new(),
            ext: String::new(),
            min_size: 0,
            max_size: 0,
            root: None,
            files_only: false,
            sort: SortOrder::Size,
            limit: 200,
        }
    }
}

#[derive(Clone)]
pub struct Hit {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub size: u64,
    pub alloc: u64,
    pub mtime: i64,
    pub status: String,
}

fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

pub fn run(conn: &Connection, q: &Query) -> Result<Vec<Hit>> {
    let mut sql =
        String::from("SELECT path,name,kind,size,alloc,mtime,status FROM files WHERE 1=1");
    let mut args: Vec<Box<dyn ToSql>> = Vec::new();

    if !q.text.trim().is_empty() {
        sql.push_str(" AND (name LIKE ? ESCAPE '\\' OR path LIKE ? ESCAPE '\\')");
        let needle = format!("%{}%", escape_like(q.text.trim()));
        args.push(Box::new(needle.clone()));
        args.push(Box::new(needle));
    }
    if !q.ext.trim().is_empty() {
        let ext = q.ext.trim().trim_start_matches('.').to_lowercase();
        sql.push_str(" AND ext = ?");
        args.push(Box::new(ext));
    }
    if q.min_size > 0 {
        sql.push_str(" AND size >= ?");
        args.push(Box::new(q.min_size as i64));
    }
    if q.max_size > 0 {
        sql.push_str(" AND size <= ?");
        args.push(Box::new(q.max_size as i64));
    }
    if q.files_only {
        sql.push_str(" AND kind = 'file'");
    }
    if let Some(root) = &q.root {
        let (lo, hi) = core::subtree_bounds(root);
        sql.push_str(" AND path >= ? AND path < ?");
        args.push(Box::new(lo));
        args.push(Box::new(hi));
    }
    sql.push_str(match q.sort {
        SortOrder::Name => " ORDER BY name COLLATE NOCASE ASC, path ASC LIMIT ?",
        SortOrder::Modified => " ORDER BY mtime DESC, name COLLATE NOCASE ASC LIMIT ?",
        SortOrder::Size => " ORDER BY size DESC, name COLLATE NOCASE ASC LIMIT ?",
    });
    args.push(Box::new(q.limit.clamp(1, 5000) as i64));

    let mut stmt = conn.prepare(&sql)?;
    let refs: Vec<&dyn ToSql> = args.iter().map(|a| a.as_ref()).collect();
    let rows = stmt.query_map(refs.as_slice(), |r| {
        Ok(Hit {
            path: r.get(0)?,
            name: r.get(1)?,
            kind: r.get(2)?,
            size: r.get::<_, i64>(3)? as u64,
            alloc: r.get::<_, i64>(4)? as u64,
            mtime: r.get(5)?,
            status: r.get(6)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::{run, Query, SortOrder};
    use rusqlite::Connection;

    fn test_index() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE files(
                path TEXT, name TEXT, kind TEXT, size INTEGER, alloc INTEGER,
                mtime INTEGER, status TEXT, ext TEXT
            );
            INSERT INTO files VALUES
                ('/home/user/src/main.rs', 'main.rs', 'file', 900, 1024, 10, 'ok', 'rs'),
                ('/home/user/docs/notes.txt', 'notes.txt', 'file', 100, 512, 30, 'ok', 'txt'),
                ('/home/user/docs/report.txt', 'report.txt', 'file', 400, 512, 20, 'ok', 'txt');",
        )
        .unwrap();
        conn
    }

    #[test]
    fn filters_by_path_and_max_size_then_sorts_by_modified() {
        let conn = test_index();
        let query = Query {
            text: "docs".into(),
            max_size: 500,
            sort: SortOrder::Modified,
            ..Query::default()
        };
        let hits = run(&conn, &query).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].name, "notes.txt");
        assert_eq!(hits[1].name, "report.txt");
    }

    #[test]
    fn sorts_by_name_case_insensitively() {
        let conn = test_index();
        let query = Query {
            sort: SortOrder::Name,
            ..Query::default()
        };
        let hits = run(&conn, &query).unwrap();
        assert_eq!(
            hits.iter().map(|hit| hit.name.as_str()).collect::<Vec<_>>(),
            ["main.rs", "notes.txt", "report.txt"]
        );
    }
}
