mod archive;
mod core;
mod db;
mod explorer;
mod gui;
mod scanner;
mod search;
mod storage;
mod transfer;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::time::Instant;

#[derive(Parser)]
#[command(
    name = "nexus",
    about = "NEXUS — gerenciador de arquivos e análise de armazenamento"
)]
struct Cli {
    /// Caminho do índice SQLite (padrão: ~/.local/share/nexus/index.db)
    #[arg(long, global = true)]
    db: Option<PathBuf>,
    /// Sem subcomando, abre a interface gráfica
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Abre a interface gráfica (Explorer, Pesquisa e Armazenamento)
    Gui,
    /// Abre o NEXUS como seletor de arquivos e imprime o(s) caminho(s) escolhido(s)
    Pick {
        /// Pasta inicial
        path: Option<PathBuf>,
        /// Permite selecionar vários itens
        #[arg(long)]
        multiple: bool,
    },
    /// Varre uma pasta e atualiza o índice (indexa também pastas vazias)
    Scan { path: PathBuf },
    /// Lista grupos de arquivos duplicados do índice
    Dupes {
        /// Ignora arquivos menores que isso (bytes)
        #[arg(long, default_value_t = 1)]
        min_size: u64,
        /// Quantos grupos mostrar
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Não refaz a leitura de confirmação do conteúdo
        #[arg(long)]
        no_verify: bool,
    },
    /// Maiores arquivos do índice
    Top {
        #[arg(short, long, default_value_t = 20)]
        n: usize,
    },
    /// Pesquisa no índice por nome, extensão e tamanho
    Search {
        /// Trecho do nome
        #[arg(default_value = "")]
        query: String,
        /// Extensão (sem ponto)
        #[arg(long, default_value = "")]
        ext: String,
        /// Tamanho mínimo em MB
        #[arg(long, default_value_t = 0)]
        min_mb: u64,
        /// Restringe a uma pasta
        #[arg(long)]
        root: Option<String>,
        /// Inclui diretórios
        #[arg(long)]
        all: bool,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let db_path = cli.db.unwrap_or_else(core::default_db);
    let cmd = match cli.cmd {
        None | Some(Cmd::Gui) => return gui::run(db_path),
        Some(Cmd::Pick { path, multiple }) => return gui::run_picker(db_path, path, multiple),
        Some(c) => c,
    };
    let conn = db::open(&db_path)?;

    match cmd {
        Cmd::Gui => {}
        Cmd::Pick { .. } => {}
        Cmd::Scan { path } => {
            let t = Instant::now();
            let mut conn = conn;
            let s = scanner::scan(&mut conn, &path)?;
            println!(
                "{} arquivos, {} pastas ({} lógicos, {} em disco), {} removidos, {} erros — {:.1?}",
                s.files,
                s.dirs,
                core::human(s.bytes),
                core::human(s.alloc),
                s.removed,
                s.errors,
                t.elapsed()
            );
            if !s.purged {
                println!(
                    "Índice preservado: {} entradas não vistas e {} erros de leitura.",
                    s.stale, s.errors
                );
            }
        }
        Cmd::Dupes {
            min_size,
            limit,
            no_verify,
        } => {
            let t = Instant::now();
            let mut conn = conn;
            let groups = storage::dupes::find(&mut conn, min_size, !no_verify)?;
            let total: u64 = groups.iter().map(|g| g.wasted()).sum();
            for g in groups.iter().take(limit) {
                println!(
                    "\n{} cópias × {}  (desperdício {}{})  {}{}{}",
                    g.copies,
                    core::human(g.size),
                    core::human(g.wasted()),
                    if g.wasted() != g.wasted_logical() {
                        format!(" em disco / {} lógicos", core::human(g.wasted_logical()))
                    } else {
                        String::new()
                    },
                    if g.verified {
                        "verificado"
                    } else {
                        "NÃO verificado"
                    },
                    if g.hardlinks() > 0 {
                        format!(" · {} hardlink(s)", g.hardlinks())
                    } else {
                        String::new()
                    },
                    format!(" [{}]", &g.hash[..8.min(g.hash.len())])
                );
                for p in &g.paths {
                    println!("  {p}");
                }
            }
            println!(
                "\n{} grupos, {} recuperáveis no total — {:.1?}",
                groups.len(),
                core::human(total),
                t.elapsed()
            );
        }
        Cmd::Top { n } => {
            let mut stmt = conn.prepare(
                "SELECT path,size,alloc FROM files
                 WHERE kind = 'file' ORDER BY size DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map([n as i64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)? as u64,
                    r.get::<_, i64>(2)? as u64,
                ))
            })?;
            for r in rows {
                let (p, s, a) = r?;
                println!("{:>10}  {:>10}  {p}", core::human(s), core::human(a));
            }
        }
        Cmd::Search {
            query,
            ext,
            min_mb,
            root,
            all,
            limit,
        } => {
            let t = Instant::now();
            let q = search::Query {
                text: query,
                ext,
                min_size: min_mb.saturating_mul(1024 * 1024),
                max_size: 0,
                root,
                files_only: !all,
                sort: search::SortOrder::Size,
                limit,
            };
            let hits = search::run(&conn, &q)?;
            for h in &hits {
                println!(
                    "{:>10}  {:>10}  {}{}",
                    core::human(h.size),
                    core::human(h.alloc),
                    h.path,
                    if h.kind == "dir" { "/" } else { "" }
                );
            }
            println!("\n{} resultados — {:.1?}", hits.len(), t.elapsed());
        }
    }
    Ok(())
}
