use crate::core;
use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Size,
    Modified,
}

#[derive(Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub alloc: u64,
    pub modified: Option<SystemTime>,
    pub ino: u64,
    pub nlink: u64,
    pub readable: bool,
    pub executable: bool,
}

/// Lê uma pasta do sistema de arquivos (não do índice).
pub fn list(
    dir: &Path,
    show_hidden: bool,
    sort: SortKey,
    ascending: bool,
) -> io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for item in fs::read_dir(dir)? {
        let item = match item {
            Ok(i) => i,
            Err(_) => continue,
        };
        let path = item.path();
        let name = item.file_name().to_string_lossy().into_owned();
        if !show_hidden && name.starts_with('.') {
            continue;
        }
        let ft = match item.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        let is_symlink = ft.is_symlink();
        let is_dir = ft.is_dir() || (is_symlink && path.is_dir());
        let (size, alloc, modified, ino, nlink, readable, executable) =
            match fs::symlink_metadata(&path) {
                Ok(md) => (
                    md.len(),
                    md.blocks() * 512,
                    Some(md.modified().unwrap_or(SystemTime::UNIX_EPOCH)),
                    md.ino(),
                    md.nlink(),
                    true,
                    md.permissions().mode() & 0o111 != 0,
                ),
                Err(_) => (0, 0, None, 0, 1, false, false),
            };
        entries.push(Entry {
            path,
            name,
            is_dir,
            is_symlink,
            size,
            alloc,
            modified,
            ino,
            nlink,
            readable,
            executable,
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then_with(|| {
            let ord = match sort {
                SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortKey::Size => a.size.cmp(&b.size),
                SortKey::Modified => a
                    .modified
                    .partial_cmp(&b.modified)
                    .unwrap_or(std::cmp::Ordering::Equal),
            };
            if ascending {
                ord
            } else {
                ord.reverse()
            }
        })
    });
    Ok(entries)
}

pub struct Tab {
    pub cwd: PathBuf,
    pub path_input: String,
    pub entries: Vec<Entry>,
    pub selected: Option<PathBuf>,
    pub show_hidden: bool,
    pub sort: SortKey,
    pub ascending: bool,
    pub error: Option<String>,
    history: Vec<PathBuf>,
    hist: usize,
}

impl Tab {
    pub fn new(cwd: PathBuf) -> Self {
        let mut tab = Tab {
            path_input: cwd.to_string_lossy().into_owned(),
            cwd: cwd.clone(),
            entries: Vec::new(),
            selected: None,
            show_hidden: false,
            sort: SortKey::Name,
            ascending: true,
            error: None,
            history: vec![cwd],
            hist: 0,
        };
        tab.refresh();
        tab
    }

    pub fn refresh(&mut self) {
        match list(&self.cwd, self.show_hidden, self.sort, self.ascending) {
            Ok(entries) => {
                // preserva a seleção se o caminho ainda existir
                if let Some(sel) = &self.selected {
                    if !entries.iter().any(|e| &e.path == sel) {
                        self.selected = None;
                    }
                }
                self.entries = entries;
                self.error = None;
            }
            Err(e) => {
                self.entries.clear();
                self.error = Some(e.to_string());
            }
        }
        self.path_input = self.cwd.to_string_lossy().into_owned();
    }

    /// Navega para um path do sistema de arquivos, registrando no histórico.
    pub fn navigate(&mut self, path: PathBuf) -> bool {
        let path = match path.canonicalize() {
            Ok(p) if p.is_dir() => p,
            Ok(p) => {
                // arquivo: vai para a pasta que o contém
                match p.parent() {
                    Some(parent) => parent.to_path_buf(),
                    None => {
                        self.error = Some(format!("{} não é uma pasta", p.display()));
                        return false;
                    }
                }
            }
            Err(e) => {
                self.error = Some(format!("{}: {e}", path.display()));
                return false;
            }
        };
        if self.hist + 1 < self.history.len() {
            self.history.truncate(self.hist + 1);
        }
        self.history.push(path.clone());
        self.hist = self.history.len() - 1;
        self.cwd = path;
        self.selected = None;
        self.refresh();
        true
    }

    pub fn navigate_input(&mut self) -> bool {
        let expanded = core::expand_home(&self.path_input);
        self.navigate(PathBuf::from(expanded))
    }

    pub fn parent(&mut self) -> bool {
        match self.cwd.parent() {
            Some(p) => {
                let p = p.to_path_buf();
                self.navigate(p)
            }
            None => false,
        }
    }

    pub fn can_back(&self) -> bool {
        self.hist > 0
    }

    pub fn can_forward(&self) -> bool {
        self.hist + 1 < self.history.len()
    }

    pub fn back(&mut self) -> bool {
        if self.hist == 0 {
            return false;
        }
        self.hist -= 1;
        self.cwd = self.history[self.hist].clone();
        self.selected = None;
        self.refresh();
        true
    }

    pub fn forward(&mut self) -> bool {
        if self.hist + 1 >= self.history.len() {
            return false;
        }
        self.hist += 1;
        self.cwd = self.history[self.hist].clone();
        self.selected = None;
        self.refresh();
        true
    }

    pub fn title(&self) -> String {
        self.cwd
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.cwd.to_string_lossy().into_owned())
    }

    pub fn selected_entry(&self) -> Option<&Entry> {
        let sel = self.selected.as_ref()?;
        self.entries.iter().find(|e| &e.path == sel)
    }
}

pub struct Explorer {
    pub tabs: Vec<Tab>,
    pub active: usize,
}

impl Explorer {
    pub fn new(start: PathBuf) -> Self {
        Explorer {
            tabs: vec![Tab::new(start)],
            active: 0,
        }
    }

    pub fn active_tab(&self) -> &Tab {
        &self.tabs[self.active.min(self.tabs.len() - 1)]
    }

    pub fn active_tab_mut(&mut self) -> &mut Tab {
        let i = self.active.min(self.tabs.len() - 1);
        &mut self.tabs[i]
    }

    pub fn open_tab(&mut self, path: PathBuf) {
        let resolved = path.canonicalize().unwrap_or(path);
        self.tabs.push(Tab::new(resolved));
        self.active = self.tabs.len() - 1;
    }

    pub fn close_tab(&mut self, i: usize) {
        if self.tabs.len() <= 1 {
            return;
        }
        self.tabs.remove(i);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        } else if self.active > i {
            self.active -= 1;
        }
    }
}

fn valid_name(name: &str) -> io::Result<&str> {
    let name = name.trim();
    if name.is_empty() || name == "." || name == ".." || name.contains('/') {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "nome inválido"));
    }
    Ok(name)
}

pub fn create_dir(parent: &Path, name: &str) -> io::Result<PathBuf> {
    let name = valid_name(name)?;
    let path = parent.join(name);
    fs::create_dir(&path)?;
    Ok(path)
}

pub fn rename(path: &Path, new_name: &str) -> io::Result<PathBuf> {
    let new_name = valid_name(new_name)?;
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "sem pasta pai"))?;
    let dest = parent.join(new_name);
    if dest.exists() && dest != path {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "já existe um item com esse nome",
        ));
    }
    fs::rename(path, &dest)?;
    Ok(dest)
}

pub fn open_external(path: &Path) -> io::Result<()> {
    let mut command = std::process::Command::new("xdg-open");
    command.arg(path);
    core::spawn_and_reap(command)
}
