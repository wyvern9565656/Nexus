use super::*;

impl NexusApp {
    pub(super) fn prepare_transfer(&mut self, move_sources: bool) {
        if self.transfer.is_some() {
            return;
        }
        self.transfer_paths = self.selected_items.iter().cloned().collect();
        if self.transfer_paths.is_empty() {
            if let Some(path) = self.explorer.active_tab().selected.clone() {
                self.transfer_paths.push(path);
            }
        }
        self.transfer_paths.sort();
        self.transfer_destination = self
            .explorer
            .active_tab()
            .cwd
            .to_string_lossy()
            .into_owned();
        self.transfer_move = move_sources;
        self.transfer_open = !self.transfer_paths.is_empty();
    }

    pub(super) fn start_transfer(&mut self) {
        if self.transfer.is_some() {
            self.status = "Já existe uma transferência em andamento.".into();
            return;
        }
        let destination = PathBuf::from(core::expand_home(&self.transfer_destination));
        if !destination.is_dir() {
            self.status = "A pasta de destino não existe ou não pode ser lida.".into();
            return;
        }
        if self.transfer_paths.is_empty() {
            self.status = "Nenhum item selecionado para transferir.".into();
            return;
        }
        let paused = Arc::new((Mutex::new(false), Condvar::new()));
        let cancelled = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        let sources = self.transfer_paths.clone();
        let move_sources = self.transfer_move;
        let worker_paused = paused.clone();
        let worker_cancelled = cancelled.clone();
        std::thread::spawn(move || {
            transfer::run(
                sources,
                destination,
                move_sources,
                worker_paused,
                worker_cancelled,
                sender,
            );
        });
        self.transfer = Some(ActiveTransfer {
            receiver,
            paused,
            cancelled,
            done: 0,
            total: 0,
            started: Instant::now(),
            label: if move_sources { "Movendo" } else { "Copiando" }.into(),
        });
        self.transfer_open = false;
        self.status = "Transferência iniciada".into();
    }

    pub(super) fn ui_command_palette(&mut self, ctx: &egui::Context) {
        if !self.command_open {
            return;
        }
        let cwd = self.explorer.active_tab().cwd.clone();
        let mut items = vec![
            ("Criar pasta".to_string(), CommandAction::CreateFolder),
            (
                "Alternar arquivos ocultos".to_string(),
                CommandAction::ToggleHidden,
            ),
            (
                "Abrir terminal aqui".to_string(),
                CommandAction::Terminal(cwd.clone()),
            ),
            (
                "Abrir pasta no VS Code".to_string(),
                CommandAction::Editor(cwd),
            ),
        ];
        for path in &self.recent_paths {
            items.push((
                format!("Recente  {}", path.display()),
                CommandAction::Navigate(path.clone()),
            ));
        }
        for path in &self.favorites {
            items.push((
                format!("Favorito  {path}"),
                CommandAction::Navigate(PathBuf::from(path)),
            ));
        }
        for entry in &self.explorer.active_tab().entries {
            items.push((
                format!(
                    "{}  {}",
                    if entry.is_dir { "Pasta" } else { "Arquivo" },
                    entry.name
                ),
                CommandAction::Open(entry.path.clone()),
            ));
        }
        let query = self.command_query.to_lowercase();
        let filtered: Vec<usize> = items
            .iter()
            .enumerate()
            .filter(|(_, (label, _))| label.to_lowercase().contains(&query))
            .map(|(index, _)| index)
            .collect();
        self.command_index = self.command_index.min(filtered.len().saturating_sub(1));
        let mut chosen = None;
        let mut open = true;
        egui::Window::new("⌘  Ir para ou executar")
            .id(egui::Id::new("nexus-command-palette"))
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .collapsible(false)
            .resizable(false)
            .default_width(520.0)
            .open(&mut open)
            .show(ctx, |ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.command_query)
                        .id(egui::Id::new("nexus-command-query"))
                        .desired_width(f32::INFINITY)
                        .hint_text("Buscar pasta, arquivo ou ação…"),
                );
                if response.changed() {
                    self.command_index = 0;
                }
                egui::ScrollArea::vertical()
                    .max_height(340.0)
                    .show(ui, |ui| {
                        for (visible_index, item_index) in filtered.iter().enumerate() {
                            let (label, action) = &items[*item_index];
                            if ui
                                .selectable_label(visible_index == self.command_index, label)
                                .clicked()
                            {
                                chosen = Some(action.clone());
                            }
                        }
                    });
                ui.weak("Ctrl+K / Ctrl+P  ·  ↑↓ navegar  ·  Enter abrir  ·  Esc fechar");
            });
        let (up, down, enter, escape) = ctx.input(|input| {
            (
                input.key_pressed(egui::Key::ArrowUp),
                input.key_pressed(egui::Key::ArrowDown),
                input.key_pressed(egui::Key::Enter),
                input.key_pressed(egui::Key::Escape),
            )
        });
        if up {
            self.command_index = self.command_index.saturating_sub(1);
        }
        if down && !filtered.is_empty() {
            self.command_index = (self.command_index + 1).min(filtered.len() - 1);
        }
        if enter {
            chosen = filtered
                .get(self.command_index)
                .and_then(|index| items.get(*index))
                .map(|(_, action)| action.clone());
        }
        if escape {
            open = false;
        }
        if let Some(action) = chosen {
            self.execute_command(ctx, action);
        } else if !open {
            self.command_open = false;
        }
    }

    pub(super) fn ui_transfer_dialog(&mut self, ctx: &egui::Context) {
        if !self.transfer_open {
            return;
        }
        let mut open = true;
        let mut start = false;
        let mut cancel = false;
        egui::Window::new(if self.transfer_move {
            "Mover itens"
        } else {
            "Copiar itens"
        })
        .id(egui::Id::new("nexus-transfer-dialog"))
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(format!(
                "{} item(ns) selecionado(s)",
                self.transfer_paths.len()
            ));
            ui.horizontal(|ui| {
                ui.label("Destino:");
                ui.text_edit_singleline(&mut self.transfer_destination);
            });
            ui.horizontal(|ui| {
                start = ui
                    .add_enabled(self.transfer.is_none(), egui::Button::new("Iniciar"))
                    .clicked();
                if ui.button("Cancelar").clicked() {
                    cancel = true;
                }
            });
        });
        if cancel {
            open = false;
        }
        self.transfer_open = open;
        if start {
            self.start_transfer();
        }
    }

    pub(super) fn bulk_rename_preview(&self) -> Result<Vec<(PathBuf, PathBuf)>, String> {
        let mut paths: Vec<PathBuf> = self.selected_items.iter().cloned().collect();
        paths.sort();
        let regex = if self.bulk_regex && !self.bulk_find.is_empty() {
            Some(Regex::new(&self.bulk_find).map_err(|error| error.to_string())?)
        } else {
            None
        };
        let mut destinations = HashSet::new();
        let mut preview = Vec::with_capacity(paths.len());
        for (index, path) in paths.into_iter().enumerate() {
            let file_name = path
                .file_name()
                .ok_or_else(|| format!("Caminho inválido: {}", path.display()))?
                .to_string_lossy();
            let stem = path
                .file_stem()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| file_name.to_string());
            let extension = path
                .extension()
                .map(|ext| ext.to_string_lossy().into_owned())
                .unwrap_or_default();
            let mut name = self
                .bulk_template
                .replace("{name}", &stem)
                .replace("{n}", &format!("{:02}", index + 1))
                .replace("{ext}", &extension);
            if !self.bulk_template.contains("{ext}") && !extension.is_empty() {
                name.push('.');
                name.push_str(&extension);
            }
            if !self.bulk_find.is_empty() {
                name = if let Some(regex) = &regex {
                    regex
                        .replace_all(&name, self.bulk_replace.as_str())
                        .into_owned()
                } else {
                    name.replace(&self.bulk_find, &self.bulk_replace)
                };
            }
            if name.trim().is_empty() || name == "." || name == ".." || name.contains('/') {
                return Err(format!("Nome inválido gerado para {}", file_name));
            }
            let destination = path
                .parent()
                .ok_or_else(|| "Item sem pasta pai".to_string())?
                .join(name);
            if destination.exists() && destination != path {
                return Err(format!("Já existe: {}", destination.display()));
            }
            if !destinations.insert(destination.clone()) {
                return Err(format!("Destino repetido: {}", destination.display()));
            }
            preview.push((path, destination));
        }
        Ok(preview)
    }

    pub(super) fn ui_bulk_rename(&mut self, ctx: &egui::Context) {
        if !self.bulk_rename_open {
            return;
        }
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        let preview = self.bulk_rename_preview();
        egui::Window::new("Renomeação em lote")
            .id(egui::Id::new("nexus-bulk-rename"))
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(620.0)
            .resizable(true)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("Padrão; tokens disponíveis: {name}, {n}, {ext}");
                ui.text_edit_singleline(&mut self.bulk_template);
                ui.horizontal(|ui| {
                    ui.label("Localizar:");
                    ui.text_edit_singleline(&mut self.bulk_find);
                    ui.label("Substituir:");
                    ui.text_edit_singleline(&mut self.bulk_replace);
                    ui.checkbox(&mut self.bulk_regex, "Regex");
                });
                ui.separator();
                match &preview {
                    Ok(items) => {
                        egui::ScrollArea::vertical()
                            .max_height(280.0)
                            .show(ui, |ui| {
                                for (source, destination) in items {
                                    ui.horizontal(|ui| {
                                        ui.monospace(
                                            source
                                                .file_name()
                                                .unwrap_or_default()
                                                .to_string_lossy(),
                                        );
                                        ui.label("→");
                                        ui.monospace(
                                            destination
                                                .file_name()
                                                .unwrap_or_default()
                                                .to_string_lossy(),
                                        );
                                    });
                                }
                            });
                    }
                    Err(error) => {
                        ui.colored_label(Color32::LIGHT_RED, error);
                    }
                }
                ui.horizontal(|ui| {
                    apply = ui
                        .add_enabled(preview.is_ok(), egui::Button::new("Aplicar renomes"))
                        .clicked();
                    if ui.button("Cancelar").clicked() {
                        cancel = true;
                    }
                });
            });
        if cancel {
            open = false;
        }
        self.bulk_rename_open = open;
        if apply {
            match self.bulk_rename_preview() {
                Ok(items) => {
                    let mut renamed = Vec::with_capacity(items.len());
                    let mut error = None;
                    for (source, destination) in items {
                        let name = destination
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy();
                        match fileops::rename(&source, &name) {
                            Ok(path) => renamed.push(path),
                            Err(cause) => {
                                error = Some(format!("{}: {cause}", source.display()));
                                break;
                            }
                        }
                    }
                    if let Some(error) = error {
                        self.status = format!("Renomeação parcial: {error}");
                    } else {
                        self.status = format!("{} itens renomeados", renamed.len());
                    }
                    self.selected_items = renamed.into_iter().collect();
                    let tab = self.explorer.active_tab_mut();
                    tab.refresh();
                    tab.selected = self.selected_items.iter().next().cloned();
                    self.bulk_rename_open = false;
                }
                Err(error) => self.status = format!("Renomeação cancelada: {error}"),
            }
        }
    }

    pub(super) fn ui_settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let mut open = true;
        let mut changed = false;
        let mut reload_bg = false;
        egui::Window::new("Aparência")
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.strong("Fundo");
                ui.horizontal(|ui| {
                    ui.label("Cor:");
                    changed |= ui
                        .color_edit_button_srgb(&mut self.settings.bg_color)
                        .changed();
                });
                ui.horizontal(|ui| {
                    ui.label("Imagem:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings.bg_image)
                            .desired_width(280.0)
                            .hint_text("/caminho/para/fundo.jpg"),
                    );
                    if ui.button("Aplicar").clicked() {
                        reload_bg = true;
                        changed = true;
                    }
                    if ui.button("Limpar").clicked() {
                        self.settings.bg_image.clear();
                        reload_bg = true;
                        changed = true;
                    }
                });
                if let Some(err) = &self.bg_error {
                    ui.colored_label(Color32::LIGHT_RED, err);
                }
                changed |= ui
                    .add(
                        egui::Slider::new(&mut self.settings.bg_dim, 0.0..=0.9)
                            .text("escurecer fundo"),
                    )
                    .changed();

                ui.separator();
                ui.strong("Mídia lateral");
                ui.horizontal(|ui| {
                    ui.label("Imagem/GIF:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings.sidebar_image)
                            .desired_width(240.0)
                            .hint_text("/caminho/para/midia.gif"),
                    );
                });
                ui.horizontal(|ui| {
                    if ui.button("Aplicar").clicked() {
                        self.sidebar_reload = true;
                        changed = true;
                    }
                    if ui.button("Limpar").clicked() {
                        self.settings.sidebar_image.clear();
                        self.sidebar_reload = true;
                        changed = true;
                    }
                });
                if let Some(err) = &self.sidebar_error {
                    ui.colored_label(Color32::LIGHT_RED, err);
                }

                ui.separator();
                ui.strong("Transparência");
                changed |= ui
                    .add(
                        egui::Slider::new(&mut self.settings.sidebar_alpha, 0.1..=1.0)
                            .text("painel lateral"),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(&mut self.settings.preview_alpha, 0.1..=1.0)
                            .text("prévia"),
                    )
                    .changed();

                ui.separator();
                ui.strong("Interface");
                ui.horizontal(|ui| {
                    ui.label("Destaque:");
                    changed |= ui
                        .color_edit_button_srgb(&mut self.settings.accent)
                        .changed();
                });
                changed |= ui
                    .add(
                        egui::Slider::new(&mut self.settings.tile_size, 64.0..=180.0)
                            .text("tamanho dos ícones"),
                    )
                    .changed();
                ui.horizontal(|ui| {
                    ui.label("Visualização:");
                    changed |= ui
                        .selectable_value(&mut self.settings.view, ViewMode::Grid, "Grade")
                        .changed();
                    changed |= ui
                        .selectable_value(&mut self.settings.view, ViewMode::List, "Lista")
                        .changed();
                });
                ui.add_space(4.0);
                ui.weak("Fundo aceita PNG, JPEG, WebP, GIF e BMP.");
            });
        if reload_bg {
            self.bg_reload = true;
        }
        if changed {
            self.save_settings();
        }
        self.settings_open = open;
    }

    pub(super) fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some((path, mut name)) = self.rename_target.take() {
            let mut open = true;
            let mut cancel = false;
            let mut apply = false;
            egui::Window::new("Renomear")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.label(path.to_string_lossy());
                    ui.add(egui::TextEdit::singleline(&mut name).desired_width(280.0));
                    ui.horizontal(|ui| {
                        if ui.button("Renomear").clicked() {
                            apply = true;
                        }
                        if ui.button("Cancelar").clicked() {
                            cancel = true;
                        }
                    });
                });
            if apply {
                match fileops::rename(&path, &name) {
                    Ok(new_path) => {
                        self.status = format!("Renomeado para {}", new_path.display());
                        let tab = self.explorer.active_tab_mut();
                        tab.selected = Some(new_path);
                        tab.refresh();
                    }
                    Err(e) => self.status = format!("Erro ao renomear: {e}"),
                }
            } else if open && !cancel {
                self.rename_target = Some((path, name));
            }
        }

        if let Some(paths) = self.trash_confirm.clone() {
            let mut open = true;
            let mut cancel = false;
            let mut apply = false;
            egui::Window::new("Mover para a lixeira?")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.label("O item vai para a lixeira do sistema (não é apagado de imediato):");
                    ui.label(format!("{} item(ns):", paths.len()));
                    egui::ScrollArea::vertical()
                        .max_height(180.0)
                        .show(ui, |ui| {
                            for path in paths.iter().take(12) {
                                ui.monospace(path.to_string_lossy());
                            }
                            if paths.len() > 12 {
                                ui.weak(format!("e mais {} item(ns)…", paths.len() - 12));
                            }
                        });
                    ui.horizontal(|ui| {
                        if ui.button("Mover para a lixeira").clicked() {
                            apply = true;
                        }
                        if ui.button("Cancelar").clicked() {
                            cancel = true;
                        }
                    });
                });
            if apply {
                let mut moved = 0;
                let mut errors = Vec::new();
                for path in &paths {
                    match core::move_to_trash(path) {
                        Ok(_) => moved += 1,
                        Err(error) => errors.push(format!("{}: {error}", path.display())),
                    }
                }
                self.status = if errors.is_empty() {
                    format!("{moved} item(ns) movido(s) para a lixeira")
                } else {
                    format!("{moved} movido(s); {} erro(s): {}", errors.len(), errors[0])
                };
                for path in &paths {
                    self.selected_items.remove(path);
                }
                self.trash_confirm = None;
                let tab = self.explorer.active_tab_mut();
                tab.selected = None;
                tab.refresh();
            } else if !open || cancel {
                self.trash_confirm = None;
            }
        }

        if let Some(path) = self.dup_confirm.clone() {
            let mut open = true;
            let mut cancel = false;
            let mut apply = false;
            egui::Window::new("Mover cópia para a lixeira?")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.label("O grupo será conferido de novo antes da ação.");
                    ui.monospace(&path);
                    ui.horizontal(|ui| {
                        if ui.button("Conferir e mover").clicked() {
                            apply = true;
                        }
                        if ui.button("Cancelar").clicked() {
                            cancel = true;
                        }
                    });
                });
            if apply {
                let group = self.dupes.as_ref().and_then(|(gs, _)| {
                    gs.iter()
                        .find(|g| g.verified && g.paths.iter().any(|p| p == &path))
                        .cloned()
                });
                match group {
                    Some(g) => match dupes::recheck(&g) {
                        Ok(()) => match core::move_to_trash(Path::new(&path)) {
                            Ok(dest) => self.status = format!("Na lixeira: {}", dest.display()),
                            Err(e) => self.status = format!("Erro na lixeira: {e}"),
                        },
                        Err(e) => self.status = format!("{e}"),
                    },
                    None => {
                        self.status = "Grupo não verificado ou inexistente; ação cancelada.".into()
                    }
                }
                self.dup_confirm = None;
            } else if !open || cancel {
                self.dup_confirm = None;
            }
        }
    }
}
