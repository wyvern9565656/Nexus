use crate::core;
use anyhow::Result;
use rusqlite::{params, Connection};
use std::collections::{HashMap, HashSet};

pub struct TNode {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub alloc: u64,
    pub is_dir: bool,
    pub status: String,
    pub children: Vec<usize>,
    pub parent: Option<usize>,
}

/// Árvore de diretórios em memória (arena), construída a partir do índice.
/// Diretórios vazios também viram nós; arquivos inacessíveis entram com
/// `status = "unreadable"` para a interface poder sinalizá-los.
pub struct Tree {
    pub nodes: Vec<TNode>,
    prefix: String,
    dirs: HashMap<String, usize>,
}

impl Tree {
    fn new(root: &str, prefix: &str) -> Self {
        let name = std::path::Path::new(root)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.to_string());
        Tree {
            nodes: vec![TNode {
                name,
                path: root.to_string(),
                size: 0,
                alloc: 0,
                is_dir: true,
                status: "ok".into(),
                children: vec![],
                parent: None,
            }],
            prefix: prefix.to_string(),
            dirs: HashMap::new(),
        }
    }

    /// Cria (se preciso) um nó de diretório e todos os ancestrais dentro da raiz.
    fn ensure_dir(&mut self, path: &str) -> Option<usize> {
        if path == self.nodes[0].path {
            return Some(0);
        }
        let rel = path.strip_prefix(self.prefix.as_str())?;
        if rel.is_empty() {
            return Some(0);
        }
        let comps: Vec<&str> = rel.split('/').filter(|c| !c.is_empty()).collect();
        let mut cur = 0usize;
        let mut cur_path = self.prefix.clone();
        for (i, c) in comps.iter().enumerate() {
            cur_path.push_str(c);
            let idx = match self.dirs.get(&cur_path) {
                Some(&j) => j,
                None => {
                    let idx = self.nodes.len();
                    self.nodes.push(TNode {
                        name: c.to_string(),
                        path: cur_path.clone(),
                        size: 0,
                        alloc: 0,
                        is_dir: true,
                        status: "ok".into(),
                        children: vec![],
                        parent: Some(cur),
                    });
                    self.nodes[cur].children.push(idx);
                    self.dirs.insert(cur_path.clone(), idx);
                    idx
                }
            };
            cur = idx;
            if i + 1 < comps.len() {
                cur_path.push('/');
            }
        }
        Some(cur)
    }

    fn insert_file(&mut self, path: &str, size: u64, alloc: u64, status: &str) {
        let Some(parent_idx) = std::path::Path::new(path)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .and_then(|p| self.ensure_dir(&p))
        else {
            return;
        };
        let name = std::path::Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string());
        let idx = self.nodes.len();
        self.nodes.push(TNode {
            name,
            path: path.to_string(),
            size,
            alloc,
            is_dir: false,
            status: status.to_string(),
            children: vec![],
            parent: Some(parent_idx),
        });
        self.nodes[parent_idx].children.push(idx);
        let mut a = Some(parent_idx);
        while let Some(j) = a {
            self.nodes[j].size += size;
            self.nodes[j].alloc += alloc;
            a = self.nodes[j].parent;
        }
    }
}

pub fn load(conn: &Connection, root: &str) -> Result<Tree> {
    let (lo, hi) = core::subtree_bounds(root);
    let mut tree = Tree::new(root, &lo);
    let mut stmt = conn.prepare(
        "SELECT path,size,alloc,kind,status,nlink,dev,ino FROM files WHERE path>=?1 AND path<?2",
    )?;
    let rows = stmt.query_map(params![lo, hi], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)? as u64,
            r.get::<_, i64>(2)? as u64,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, i64>(5)? as u64,
            r.get::<_, i64>(6)?,
            r.get::<_, i64>(7)?,
        ))
    })?;
    // hardlinks: o mesmo inode conta uma vez só no espaço exibido
    let mut seen_inodes: HashSet<(i64, i64)> = HashSet::new();
    for r in rows {
        let (p, size, alloc, kind, status, nlink, dev, ino) = r?;
        if kind == "dir" {
            tree.ensure_dir(&p);
            continue;
        }
        if nlink > 1 && !seen_inodes.insert((dev, ino)) {
            continue;
        }
        tree.insert_file(&p, size, alloc, &status);
    }
    tree.dirs.clear();
    Ok(tree)
}

pub fn top(conn: &Connection, root: &str, n: usize) -> Result<Vec<(String, u64, u64)>> {
    let (lo, hi) = core::subtree_bounds(root);
    let mut stmt = conn.prepare(
        "SELECT path,size,alloc FROM files
         WHERE kind = 'file' AND path>=?1 AND path<?2
         ORDER BY size DESC LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![lo, hi, n as i64], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)? as u64,
            r.get::<_, i64>(2)? as u64,
        ))
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}
