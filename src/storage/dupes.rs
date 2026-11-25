use anyhow::Result;
use rayon::prelude::*;
use rusqlite::{params, Connection};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;

const CHUNK: u64 = 64 * 1024;

struct Row {
    path: String,
    size: u64,
    alloc: u64,
    hash: Option<String>,
}

#[derive(Clone)]
pub struct DupGroup {
    pub size: u64,
    pub alloc: u64,
    pub hash: String,
    /// Todos os caminhos, inclusive hardlinks entre si.
    pub paths: Vec<String>,
    /// Quantidade de arquivos distintos (inodes); hardlinks não contam como cópia.
    pub copies: u64,
    /// True quando o conteúdo foi lido e conferido novamente (hashes completos
    /// recalculados, com tamanho/mtime estáveis durante a leitura).
    pub verified: bool,
}

impl DupGroup {
    pub fn wasted(&self) -> u64 {
        self.alloc.saturating_mul(self.copies.saturating_sub(1))
    }

    pub fn wasted_logical(&self) -> u64 {
        self.size.saturating_mul(self.copies.saturating_sub(1))
    }

    pub fn hardlinks(&self) -> usize {
        self.paths.len().saturating_sub(self.copies as usize)
    }
}

/// Monta o grupo contando arquivos distintos por (dev, ino). Se todos os
/// caminhos forem hardlinks do mesmo inode, não é duplicata.
fn finish_group(
    size: u64,
    alloc: u64,
    hash: String,
    mut paths: Vec<String>,
    verified: bool,
) -> Option<DupGroup> {
    paths.sort();
    let mut inodes = HashSet::new();
    let mut alloc_max = alloc;
    for p in &paths {
        match std::fs::metadata(p) {
            Ok(m) => {
                inodes.insert((m.dev(), m.ino()));
                alloc_max = alloc_max.max(m.blocks() * 512);
            }
            Err(_) => {
                // conservador: caminho ilegível conta como arquivo distinto
                inodes.insert((u64::MAX, hash.len() as u64));
            }
        }
    }
    let copies = inodes.len() as u64;
    if copies < 2 {
        return None;
    }
    Some(DupGroup {
        size,
        alloc: alloc_max,
        hash,
        paths,
        copies,
        verified,
    })
}

/// Hash rápido: início + fim do arquivo (descarta a maioria dos falsos candidatos).
fn partial_hash(path: &str, size: u64) -> io::Result<String> {
    let mut f = File::open(path)?;
    let mut h = blake3::Hasher::new();
    io::copy(&mut (&mut f).take(CHUNK), &mut h)?;
    if size > CHUNK * 2 {
        f.seek(SeekFrom::End(-(CHUNK as i64)))?;
        io::copy(&mut (&mut f).take(CHUNK), &mut h)?;
    }
    Ok(h.finalize().to_hex().to_string())
}

/// Hash completo com checagem de estabilidade: se o arquivo mudar de tamanho
/// ou mtime enquanto é lido, o resultado é descartado.
fn full_hash(path: &str) -> io::Result<String> {
    let before = std::fs::metadata(path)?;
    let mut h = blake3::Hasher::new();
    h.update_reader(File::open(path)?)?;
    let after = std::fs::metadata(path)?;
    if before.len() != after.len()
        || before.mtime() != after.mtime()
        || before.mtime_nsec() != after.mtime_nsec()
    {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "arquivo mudou durante a leitura",
        ));
    }
    Ok(h.finalize().to_hex().to_string())
}

fn group_rows(hashed: Vec<(Row, String)>, verified: bool) -> Vec<DupGroup> {
    let mut by_hash: HashMap<(u64, String), Vec<Row>> = HashMap::new();
    for (r, h) in hashed {
        by_hash.entry((r.size, h)).or_default().push(r);
    }
    by_hash
        .into_iter()
        .filter(|(_, v)| v.len() > 1)
        .filter_map(|((size, hash), members)| {
            let alloc = members.iter().map(|r| r.alloc).max().unwrap_or(0);
            finish_group(
                size,
                alloc,
                hash,
                members.into_iter().map(|r| r.path).collect(),
                verified,
            )
        })
        .collect()
}

/// Agrupa por conteúdo com três etapas: tamanho → hash parcial → hash completo.
/// Com `verify = true`, os grupos sobreviventes são lidos outra vez do zero e
/// só continuam se todos os membros ainda tiverem o mesmo conteúdo. Nada é
/// apagado aqui; a interface só permite ações sobre grupos verificados.
pub fn find(conn: &mut Connection, min_size: u64, verify: bool) -> Result<Vec<DupGroup>> {
    // 1) candidatos: só tamanhos que aparecem mais de uma vez
    let rows: Vec<Row> = {
        let mut stmt = conn.prepare(
            "SELECT path,size,alloc,hash FROM files
             WHERE kind = 'file' AND status = 'ok' AND size >= ?1 AND size IN (
                 SELECT size FROM files
                 WHERE kind = 'file' AND status = 'ok' AND size >= ?1
                 GROUP BY size HAVING COUNT(*) > 1)",
        )?;
        let it = stmt.query_map(params![min_size as i64], |r| {
            Ok(Row {
                path: r.get(0)?,
                size: r.get::<_, i64>(1)? as u64,
                alloc: r.get::<_, i64>(2)? as u64,
                hash: r.get(3)?,
            })
        })?;
        it.collect::<std::result::Result<Vec<Row>, _>>()?
    };

    // 2) hash parcial em paralelo (hardlinks são agrupados normalmente;
    //    o desperdício é calculado por inode em `finish_group`)
    let partial: Vec<(Row, String)> = rows
        .into_par_iter()
        .filter_map(|r| partial_hash(&r.path, r.size).ok().map(|h| (r, h)))
        .collect();
    let mut by_partial: HashMap<(u64, String), Vec<Row>> = HashMap::new();
    for (r, h) in partial {
        by_partial.entry((r.size, h)).or_default().push(r);
    }
    let candidates: Vec<Row> = by_partial
        .into_values()
        .filter(|v| v.len() > 1)
        .flatten()
        .collect();

    // 3) hash completo. Com verificação, lê tudo de novo e ignora o cache:
    //    o grupo só é aceito se o conteúdo atual coincidir. Sem verificação,
    //    reaproveita o hash do índice e marca o grupo como não verificado.
    let groups: Vec<DupGroup> = if verify {
        let fresh: Vec<(Row, String)> = candidates
            .into_par_iter()
            .filter_map(|r| full_hash(&r.path).ok().map(|h| (r, h)))
            .collect();
        group_rows(fresh, true)
    } else {
        let hashed: Vec<(Row, String)> = candidates
            .into_par_iter()
            .filter_map(|r| {
                let h = match &r.hash {
                    Some(h) => Some(h.clone()),
                    None => full_hash(&r.path).ok(),
                };
                h.map(|h| (r, h))
            })
            .collect();
        group_rows(hashed, false)
    };

    let tx = conn.transaction()?;
    {
        let mut up = tx.prepare(
            "UPDATE files SET hash = ?1 WHERE path = ?2 AND status = 'ok' AND kind = 'file'",
        )?;
        for g in &groups {
            for p in &g.paths {
                up.execute(params![g.hash, p])?;
            }
        }
    }
    tx.commit()?;

    let mut groups = groups;
    groups.sort_by(|a, b| b.wasted().cmp(&a.wasted()));
    Ok(groups)
}

/// Confere, na hora de qualquer exclusão, que todos os membros do grupo ainda
/// existem e têm o mesmo conteúdo. A UI usa isto antes de mandar para a lixeira.
pub fn recheck(group: &DupGroup) -> Result<()> {
    let mut hashes = HashSet::new();
    for p in &group.paths {
        hashes.insert(full_hash(p)?);
    }
    anyhow::ensure!(
        hashes.len() == 1 && hashes.contains(&group.hash),
        "os arquivos mudaram desde a verificação; refaça a busca"
    );
    Ok(())
}
