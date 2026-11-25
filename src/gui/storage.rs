use super::*;

impl NexusApp {
    pub(super) fn load_index(&mut self, ctx: &egui::Context) {
        let db_path = self.db_path.clone();
        let input = core::expand_home(&self.scan_root);
        self.spawn(ctx, "Carregando índice...", move || {
            let conn = db::open(&db_path)?;
            let root = Path::new(&input)
                .canonicalize()
                .unwrap_or_else(|_| PathBuf::from(&input))
                .to_string_lossy()
                .into_owned();
            let tree = tree::load(&conn, &root)?;
            let top = tree::top(&conn, &root, 200)?;
            let text = if tree.nodes.len() <= 1 {
                "Pasta ainda não indexada — use Escanear".to_string()
            } else {
                format!(
                    "{} lógicos · {} em disco",
                    human(tree.nodes[0].size),
                    human(tree.nodes[0].alloc)
                )
            };
            Ok(Msg::Storage { tree, top, text })
        });
    }

    pub(super) fn start_scan(&mut self, ctx: &egui::Context) {
        let db_path = self.db_path.clone();
        let input = core::expand_home(&self.scan_root);
        self.spawn(ctx, "Escaneando...", move || {
            let mut conn = db::open(&db_path)?;
            let t = Instant::now();
            let s = scanner::scan(&mut conn, Path::new(&input))?;
            let root = Path::new(&input)
                .canonicalize()
                .unwrap_or_else(|_| PathBuf::from(&input))
                .to_string_lossy()
                .into_owned();
            let tree = tree::load(&conn, &root)?;
            let top = tree::top(&conn, &root, 200)?;
            let text = format!(
                "{} arquivos, {} pastas · {} lógicos · {} em disco · {} erros{} — {:.1?}",
                s.files,
                s.dirs,
                human(s.bytes),
                human(s.alloc),
                s.errors,
                if s.purged {
                    String::new()
                } else {
                    format!(" (índice preservado, {} não vistos)", s.stale)
                },
                t.elapsed()
            );
            Ok(Msg::Storage { tree, top, text })
        });
    }

    pub(super) fn start_dupes(&mut self, ctx: &egui::Context) {
        let db_path = self.db_path.clone();
        let min = self.min_kb.max(1) * 1024;
        let verify = self.verify_dupes;
        self.spawn(ctx, "Procurando duplicatas...", move || {
            let mut conn = db::open(&db_path)?;
            let t = Instant::now();
            let groups = dupes::find(&mut conn, min, verify)?;
            let total: u64 = groups.iter().map(|g| g.wasted()).sum();
            let text = format!(
                "{} grupos · {} recuperáveis · {}{} — {:.1?}",
                groups.len(),
                human(total),
                if verify {
                    "conteúdo verificado"
                } else {
                    "sem verificação final"
                },
                if groups.is_empty() {
                    ""
                } else {
                    " · ação só em grupos verificados"
                },
                t.elapsed()
            );
            Ok(Msg::Dupes { groups, text })
        });
    }

    pub(super) fn run_search(&mut self, ctx: &egui::Context) {
        let db_path = self.db_path.clone();
        let root = if self.search.here {
            Some(
                self.explorer
                    .active_tab()
                    .cwd
                    .to_string_lossy()
                    .into_owned(),
            )
        } else {
            None
        };
        let q = Query {
            text: self.search.text.clone(),
            ext: self.search.ext.clone(),
            min_size: self.search.min_mb.saturating_mul(1024 * 1024),
            max_size: self.search.max_mb.saturating_mul(1024 * 1024),
            root,
            files_only: self.search.files_only,
            sort: self.search.sort,
            limit: 500,
        };
        self.spawn(ctx, "Pesquisando no índice...", move || {
            let conn = db::open(&db_path)?;
            let hits = search::run(&conn, &q)?;
            Ok(Msg::Search(hits))
        });
    }

    pub(super) fn ui_storage(&mut self, ui: &mut egui::Ui) {
        if self.tree.is_none() && !self.busy {
            let ctx = ui.ctx().clone();
            self.load_index(&ctx);
        }
        ui.horizontal(|ui| {
            ui.label("Pasta:");
            ui.add(egui::TextEdit::singleline(&mut self.scan_root).desired_width(420.0));
            if ui
                .add_enabled(!self.busy, egui::Button::new("Escanear"))
                .clicked()
            {
                let ctx = ui.ctx().clone();
                self.start_scan(&ctx);
            }
            if ui.button("Carregar índice").clicked() {
                let ctx = ui.ctx().clone();
                self.load_index(&ctx);
            }
            if self.busy {
                ui.spinner();
            }
        });
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.storage_tab, StorageTab::Space, "Espaço");
            ui.selectable_value(&mut self.storage_tab, StorageTab::Dupes, "Duplicatas");
            ui.selectable_value(&mut self.storage_tab, StorageTab::Top, "Maiores");
        });
        ui.separator();
        match self.storage_tab {
            StorageTab::Space => self.ui_space(ui),
            StorageTab::Dupes => self.ui_dupes(ui),
            StorageTab::Top => self.ui_top(ui),
        }
    }

    pub(super) fn ui_space(&mut self, ui: &mut egui::Ui) {
        if self.tree.is_none() {
            ui.weak("Nenhum índice para essa pasta. Clique em Escanear.");
            return;
        }
        let mut new_cur: Option<usize> = None;
        let mut hover = String::new();
        {
            let tree = self.tree.as_ref().unwrap();
            let cur = self.cur.min(tree.nodes.len() - 1);
            ui.horizontal(|ui| {
                let parent = tree.nodes[cur].parent;
                if ui
                    .add_enabled(parent.is_some(), egui::Button::new("< Subir"))
                    .clicked()
                {
                    new_cur = parent;
                }
                ui.label(format!(
                    "{}  —  {} lógicos · {} em disco",
                    tree.nodes[cur].path,
                    human(tree.nodes[cur].size),
                    human(tree.nodes[cur].alloc)
                ));
            });

            if tree.nodes[cur].children.is_empty() {
                ui.weak("Pasta sem itens indexados.");
                return;
            }

            let (resp, painter) = ui.allocate_painter(ui.available_size(), egui::Sense::click());
            let rect = resp.rect;
            painter.rect_filled(rect, 0.0, Color32::from_gray(20));

            let use_alloc = tree.nodes[cur].alloc > 0;
            let mut items: Vec<(usize, f64)> = tree.nodes[cur]
                .children
                .iter()
                .map(|&c| {
                    (
                        c,
                        if use_alloc {
                            tree.nodes[c].alloc as f64
                        } else {
                            tree.nodes[c].size as f64
                        },
                    )
                })
                .filter(|(_, v)| *v > 0.0)
                .collect();
            items.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            items.truncate(400);

            let pointer = resp.hover_pos();
            for (idx, r) in treemap::layout(&items, rect) {
                let n = &tree.nodes[idx];
                let tr = r.shrink(1.0);
                if tr.width() < 1.0 || tr.height() < 1.0 {
                    continue;
                }
                let base = color_for(&n.name, n.is_dir);
                let hovered = pointer.map_or(false, |p| tr.contains(p));
                painter.rect_filled(tr, 2.0, if hovered { lighten(base) } else { base });
                ui.interact(
                    tr,
                    ui.id().with(("treemap-node", idx)),
                    egui::Sense::hover(),
                )
                .on_hover_text(format!(
                    "{}\n{}\nLógico: {}\nEm disco: {}{}",
                    n.name,
                    n.path,
                    human(n.size),
                    human(n.alloc),
                    if n.status == "unreadable" {
                        "\nInacessível"
                    } else {
                        ""
                    }
                ));
                if hovered {
                    hover = format!(
                        "{}  ({} lógicos · {} em disco{})",
                        n.path,
                        human(n.size),
                        human(n.alloc),
                        if n.status == "unreadable" {
                            " · inacessível"
                        } else {
                            ""
                        }
                    );
                    if resp.clicked() && n.is_dir && !n.children.is_empty() {
                        new_cur = Some(idx);
                    }
                }
                if tr.width() > 70.0 && tr.height() > 18.0 {
                    let name = text_to_fit(
                        &n.name,
                        (tr.width() - 8.0).max(0.0),
                        FontId::proportional(12.0),
                        &painter,
                    );
                    let label = if tr.height() > 34.0 {
                        format!("{}\n{}", name, human(n.alloc.max(n.size)))
                    } else {
                        name
                    };
                    painter.with_clip_rect(tr).text(
                        tr.left_top() + egui::vec2(4.0, 2.0),
                        Align2::LEFT_TOP,
                        label,
                        FontId::proportional(12.0),
                        Color32::WHITE,
                    );
                }
            }
        }
        if let Some(c) = new_cur {
            self.cur = c;
        }
        self.hover = hover;
    }

    pub(super) fn ui_dupes(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Ignorar arquivos menores que");
            ui.add(egui::DragValue::new(&mut self.min_kb).suffix(" KB"));
            ui.checkbox(
                &mut self.verify_dupes,
                "verificar conteúdo antes de excluir",
            );
            if ui
                .add_enabled(!self.busy, egui::Button::new("Procurar duplicatas"))
                .clicked()
            {
                let ctx = ui.ctx().clone();
                self.start_dupes(&ctx);
            }
        });
        ui.weak("Duplicatas são agrupadas por tamanho → hash parcial → hash completo. Só grupos verificados permitem ação.");
        ui.separator();
        let Some((groups, summary)) = &self.dupes else {
            ui.weak("Nenhuma busca nesta sessão.");
            return;
        };
        ui.label(summary.as_str());
        let groups = groups.clone();
        ui.separator();
        let mut trash: Option<String> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                for g in groups.iter().take(300) {
                    let title = format!(
                        "{} cópias × {}  (desperdício {}{})  {}{}{}",
                        g.copies,
                        human(g.size),
                        human(g.wasted()),
                        if g.wasted() != g.wasted_logical() {
                            format!(" em disco / {} lógicos", human(g.wasted_logical()))
                        } else {
                            String::new()
                        },
                        if g.verified {
                            "✓ verificado"
                        } else {
                            "⚠ não verificado"
                        },
                        if g.hardlinks() > 0 {
                            format!(" · {} hardlink(s)", g.hardlinks())
                        } else {
                            String::new()
                        },
                        format!(" [{}]", &g.hash[..8.min(g.hash.len())])
                    );
                    egui::CollapsingHeader::new(title).show(ui, |ui| {
                        for p in &g.paths {
                            ui.horizontal(|ui| {
                                if ui.small_button("Abrir pasta").clicked() {
                                    open_parent(p);
                                }
                                if g.verified && ui.small_button("Lixeira").clicked() {
                                    trash = Some(p.clone());
                                }
                                ui.label(p.as_str());
                            });
                        }
                    });
                }
            });
        if let Some(p) = trash {
            self.dup_confirm = Some(p);
        }
    }
}
