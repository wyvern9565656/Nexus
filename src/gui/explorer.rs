use super::*;

impl NexusApp {
    pub(super) fn select_context_target(&mut self, path: &Path) {
        if !self.selected_items.contains(path) {
            self.selected_items.clear();
            self.selected_items.insert(path.to_path_buf());
        }
        self.selection_anchor = Some(path.to_path_buf());
        let current = if self.selected_items.contains(path) {
            Some(path.to_path_buf())
        } else {
            self.selected_items.iter().next().cloned()
        };
        self.explorer.active_tab_mut().selected = current;
    }

    pub(super) fn set_file_clipboard(&mut self, path: &Path, cut: bool) {
        self.select_context_target(path);
        let mut paths: Vec<_> = self.selected_items.iter().cloned().collect();
        paths.sort();
        if paths.is_empty() {
            self.status = "Nenhum arquivo selecionado para copiar.".into();
            return;
        }
        self.file_clipboard = Some((paths.clone(), cut));
        let self_copy = paths
            .iter()
            .any(|source| source.parent() == Some(self.explorer.active_tab().cwd.as_path()));
        let clipboard_status = match set_system_file_clipboard(&paths, cut) {
            Ok(()) => {
                self.file_clipboard_external = false;
                "no clipboard do sistema".to_string()
            }
            Err(error) => {
                self.file_clipboard_external = false;
                format!("clipboard apenas interno: {error}")
            }
        };
        self.status = format!(
            "{} {} item(ns) {clipboard_status}{}",
            if cut { "Recortados" } else { "Copiados" },
            paths.len(),
            if self_copy {
                " · colar na pasta atual não altera nada"
            } else {
                ""
            }
        );
    }

    pub(super) fn paste_file_clipboard(&mut self) {
        let destination = self.explorer.active_tab().cwd.clone();
        self.paste_file_clipboard_into(destination);
    }

    pub(super) fn paste_file_clipboard_into(&mut self, destination: PathBuf) {
        if self.transfer.is_some() {
            self.status = "Aguarde a transferência atual terminar antes de colar.".into();
            return;
        }
        let (paths, cut) = match read_system_file_clipboard() {
            Ok(Some(clipboard)) => {
                self.file_clipboard = Some(clipboard.clone());
                self.file_clipboard_external = true;
                clipboard
            }
            Ok(None) if !self.file_clipboard_external => {
                let Some(clipboard) = self.file_clipboard.clone() else {
                    self.status = "Clipboard não contém arquivos para colar.".into();
                    return;
                };
                clipboard
            }
            Ok(None) => {
                self.status = "Clipboard do sistema não contém arquivos.".into();
                return;
            }
            Err(error) if !self.file_clipboard_external => {
                let Some(clipboard) = self.file_clipboard.clone() else {
                    self.status = format!("Não foi possível ler o clipboard: {error}");
                    return;
                };
                clipboard
            }
            Err(error) => {
                self.status = format!("Não foi possível ler o clipboard: {error}");
                return;
            }
        };
        if paths.is_empty() {
            self.status = "Clipboard não contém arquivos para colar.".into();
            return;
        }
        self.transfer_paths = paths;
        self.transfer_destination = destination.to_string_lossy().into_owned();
        self.transfer_move = cut;
        self.transfer_open = false;
        self.start_transfer();
    }

    pub(super) fn compress_selection(
        &mut self,
        ctx: &egui::Context,
        target: &Path,
        format: archive::Format,
    ) {
        let cwd = self.explorer.active_tab().cwd.clone();
        if target != cwd {
            self.select_context_target(target);
        }
        let mut paths: Vec<_> = self.selected_items.iter().cloned().collect();
        paths.sort();
        if paths.is_empty() {
            self.status = "Selecione os itens que deseja compactar.".into();
            return;
        }
        if self.busy {
            self.status = "Aguarde a operação atual terminar.".into();
            return;
        }
        if paths.iter().any(|path| path == &cwd) {
            self.status = "Não é possível compactar a pasta atual dentro dela mesma.".into();
            return;
        }
        let label = format!("Compactando em {}...", format.label());
        self.spawn(ctx, &label, move || {
            let archive_path = archive::compress(&paths, &cwd, format)?;
            Ok(Msg::Notice(format!(
                "Arquivo criado: {}",
                archive_path.display()
            )))
        });
    }

    pub(super) fn extract_archive(&mut self, ctx: &egui::Context, path: &Path) {
        if self.busy {
            self.status = "Aguarde a operação atual terminar.".into();
            return;
        }
        let path = path.to_path_buf();
        self.spawn(ctx, "Extraindo arquivo...", move || {
            let destination = archive::extract(&path)?;
            Ok(Msg::Notice(format!(
                "Extraído em: {}",
                destination.display()
            )))
        });
    }

    pub(super) fn select_clicked(&mut self, path: &Path, entries: &[Entry], additive: bool, range: bool) {
        if range {
            let anchor = self
                .selection_anchor
                .as_ref()
                .and_then(|anchor| entries.iter().position(|entry| &entry.path == anchor))
                .unwrap_or_else(|| {
                    entries
                        .iter()
                        .position(|entry| entry.path == path)
                        .unwrap_or(0)
                });
            let target = entries
                .iter()
                .position(|entry| entry.path == path)
                .unwrap_or(anchor);
            if !additive {
                self.selected_items.clear();
            }
            for entry in &entries[anchor.min(target)..=anchor.max(target)] {
                self.selected_items.insert(entry.path.clone());
            }
        } else if additive {
            if !self.selected_items.remove(path) {
                self.selected_items.insert(path.to_path_buf());
            }
            self.selection_anchor = Some(path.to_path_buf());
        } else {
            self.selected_items.clear();
            self.selected_items.insert(path.to_path_buf());
            self.selection_anchor = Some(path.to_path_buf());
        }
        let current = if self.selected_items.contains(path) {
            Some(path.to_path_buf())
        } else {
            self.selected_items.iter().next().cloned()
        };
        self.explorer.active_tab_mut().selected = current;
    }

    /// Desenha uma pequena miniatura do item arrastado junto ao cursor.
    pub(super) fn paint_drag_ghost(&self, ctx: &egui::Context, entries: &[Entry]) {
        let Some(paths) = egui::DragAndDrop::payload::<Vec<PathBuf>>(ctx) else {
            return;
        };
        if paths.is_empty() || !ctx.input(|input| input.pointer.primary_down()) {
            return;
        }
        let Some(pointer) = ctx.pointer_interact_pos() else {
            return;
        };
        let accent = self.accent();
        // A seleção vem de um HashSet: ordena para o fantasma não trocar de item a cada frame.
        let mut ordered: Vec<&PathBuf> = paths.iter().collect();
        ordered.sort();
        let first = ordered[0];
        let entry = entries.iter().find(|entry| &entry.path == first);
        let name = entry
            .map(|entry| entry.name.clone())
            .or_else(|| {
                first
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| first.display().to_string());
        let extra = paths.len().saturating_sub(1);
        let label = if extra > 0 {
            format!("{name}  +{extra}")
        } else {
            name
        };

        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::new("nexus-drag-ghost"),
        ));
        let galley = painter.layout_no_wrap(
            label,
            egui::FontId::proportional(13.0),
            Color32::from_gray(235),
        );
        let width = (galley.size().x + 54.0).clamp(120.0, 280.0);
        let rect = egui::Rect::from_min_size(
            pointer + egui::vec2(18.0, 12.0),
            egui::vec2(width, 40.0),
        );
        painter.rect_filled(rect, 9.0, Color32::from_rgba_unmultiplied(24, 24, 32, 235));
        painter.rect_stroke(rect, 9.0, egui::Stroke::new(1.0_f32, accent));
        let icon_rect = egui::Rect::from_min_size(
            rect.min + egui::vec2(8.0, 8.0),
            egui::vec2(24.0, 24.0),
        );
        if let Some(entry) = entry {
            let folder_accent = self
                .folder_colors
                .get(&entry.path)
                .map(|[red, green, blue]| Color32::from_rgb(*red, *green, *blue))
                .unwrap_or(accent);
            super::icons::draw_entry_icon(
                &painter,
                icon_rect,
                entry,
                folder_accent,
                self.folder_colors.contains_key(&entry.path),
            );
        }
        let text_pos = egui::pos2(
            icon_rect.right() + 8.0,
            rect.center().y - galley.size().y / 2.0,
        );
        painter
            .with_clip_rect(rect.shrink(4.0))
            .galley(text_pos, galley, Color32::from_gray(235));
    }

    pub(super) fn refresh_favorites(&mut self) {
        if let Some(conn) = &self.conn {
            self.favorites = db::favorites(conn).unwrap_or_default();
        }
    }

    pub(super) fn toggle_favorite(&mut self, path: &str) {
        let Some(conn) = &self.conn else {
            return;
        };
        let is_fav = self.favorites.iter().any(|f| f == path);
        let res = if is_fav {
            db::remove_favorite(conn, path)
        } else {
            db::add_favorite(conn, path)
        };
        if let Err(e) = res {
            self.status = format!("Erro ao salvar favorito: {e}");
        }
        self.refresh_favorites();
    }

    pub(super) fn navigate_to(&mut self, path: PathBuf) {
        if self.explorer.active_tab_mut().navigate(path) {
            self.selected_items.clear();
            let cwd = self.explorer.active_tab().cwd.clone();
            self.recent_paths.retain(|recent| recent != &cwd);
            self.recent_paths.insert(0, cwd);
            self.recent_paths.truncate(8);
            self.mode = Mode::Explorer;
        } else {
            self.status = self
                .explorer
                .active_tab()
                .error
                .clone()
                .unwrap_or_else(|| "Não foi possível abrir a pasta".into());
        }
    }

    pub(super) fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let (
            palette,
            slash,
            up,
            down,
            left,
            right,
            enter,
            backspace,
            space,
            escape,
            copy,
            cut,
            paste,
            select_all,
            delete,
        ) = ctx.input(|input| {
            let command = input.modifiers.ctrl || input.modifiers.command;
            (
                command && (input.key_pressed(egui::Key::K) || input.key_pressed(egui::Key::P)),
                input.key_pressed(egui::Key::Slash),
                input.key_pressed(egui::Key::ArrowUp) || input.key_pressed(egui::Key::K),
                input.key_pressed(egui::Key::ArrowDown) || input.key_pressed(egui::Key::J),
                input.key_pressed(egui::Key::ArrowLeft) || input.key_pressed(egui::Key::H),
                input.key_pressed(egui::Key::ArrowRight) || input.key_pressed(egui::Key::L),
                input.key_pressed(egui::Key::Enter),
                input.key_pressed(egui::Key::Backspace),
                input.key_pressed(egui::Key::Space),
                input.key_pressed(egui::Key::Escape),
                command && input.key_pressed(egui::Key::C),
                command && input.key_pressed(egui::Key::X),
                command && input.key_pressed(egui::Key::V),
                command && input.key_pressed(egui::Key::A),
                input.key_pressed(egui::Key::Delete),
            )
        });
        if palette {
            self.command_open = true;
            self.command_query.clear();
            self.command_index = 0;
            ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("nexus-command-query")));
            return;
        }
        if self.quicklook_open && escape {
            self.quicklook_open = false;
            return;
        }
        if self.picker_mode && escape {
            self.cancel_picker(ctx);
            return;
        }
        if self.command_open || self.mode != Mode::Explorer || ctx.wants_keyboard_input() {
            return;
        }
        if select_all {
            self.selected_items = self
                .explorer
                .active_tab()
                .entries
                .iter()
                .map(|entry| entry.path.clone())
                .collect();
            self.explorer.active_tab_mut().selected = self.selected_items.iter().next().cloned();
            return;
        }
        if delete && !self.selected_items.is_empty() {
            let mut paths: Vec<_> = self.selected_items.iter().cloned().collect();
            paths.sort();
            self.trash_confirm = Some(paths);
            return;
        }
        if paste {
            self.paste_file_clipboard();
            return;
        }
        if copy || cut {
            if let Some(path) = self
                .explorer
                .active_tab()
                .selected
                .clone()
                .or_else(|| self.selected_items.iter().next().cloned())
            {
                self.set_file_clipboard(&path, cut);
            }
            return;
        }
        if space {
            self.quicklook_open = self.explorer.active_tab().selected.is_some();
            return;
        }
        if slash {
            ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("nexus-explorer-search")));
            return;
        }
        if backspace || left {
            self.explorer.active_tab_mut().parent();
            return;
        }
        if right {
            if let Some(selected) = self.explorer.active_tab().selected_entry().cloned() {
                if selected.is_dir {
                    self.navigate_to(selected.path);
                }
            }
            return;
        }
        if up || down {
            let tab = self.explorer.active_tab_mut();
            if !tab.entries.is_empty() {
                let current = tab
                    .selected
                    .as_ref()
                    .and_then(|path| tab.entries.iter().position(|entry| &entry.path == path));
                let next = match (current, up, down) {
                    (Some(index), true, _) => index.saturating_sub(1),
                    (Some(index), _, true) => (index + 1).min(tab.entries.len() - 1),
                    (_, true, _) => 0,
                    _ => 0,
                };
                tab.selected = Some(tab.entries[next].path.clone());
                self.selected_items.clear();
                self.selected_items.insert(tab.entries[next].path.clone());
            }
            return;
        }
        if enter {
            if self.picker_mode {
                if let Some(selected) = self.explorer.active_tab().selected_entry().cloned() {
                    if selected.is_dir {
                        self.navigate_to(selected.path);
                        return;
                    }
                    if self.selected_items.is_empty() {
                        self.selected_items.insert(selected.path.clone());
                        self.explorer.active_tab_mut().selected = Some(selected.path);
                    }
                }
                if !self.selected_items.is_empty() {
                    self.confirm_picker(ctx);
                }
            } else if let Some(selected) = self.explorer.active_tab().selected_entry().cloned() {
                if selected.is_dir {
                    self.navigate_to(selected.path);
                } else if let Err(error) = fileops::open_external(&selected.path) {
                    self.status = format!("Erro ao abrir: {error}");
                }
            }
        }
    }

    pub(super) fn execute_command(&mut self, ctx: &egui::Context, action: CommandAction) {
        match action {
            CommandAction::Navigate(path) => self.navigate_to(path),
            CommandAction::Open(path) => {
                if path.is_dir() {
                    self.navigate_to(path);
                } else if let Err(error) = fileops::open_external(&path) {
                    self.status = format!("Erro ao abrir: {error}");
                }
            }
            CommandAction::QuickLook(path) => {
                self.select_context_target(&path);
                self.quicklook_open = true;
            }
            CommandAction::Run(path) => {
                if let Err(error) = run_executable(&path) {
                    self.status = format!("Não foi possível executar: {error}");
                } else {
                    self.status = format!("Executando: {}", path.display());
                }
            }
            CommandAction::CopyLocation(path) => {
                match set_system_clipboard("text/plain;charset=utf-8", &path.to_string_lossy()) {
                    Ok(()) => self.status = format!("Localização copiada: {}", path.display()),
                    Err(error) => {
                        self.status = format!("Não foi possível copiar a localização: {error}")
                    }
                }
            }
            CommandAction::Rename(path, name) => {
                self.select_context_target(&path);
                self.rename_target = Some((path, name));
            }
            CommandAction::RenameSelection(path) => {
                self.select_context_target(&path);
                if self.selected_items.len() > 1 {
                    self.bulk_rename_open = true;
                } else if let Some(path) = self.selected_items.iter().next().cloned() {
                    self.select_context_target(&path);
                    let name = path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    self.rename_target = Some((path, name));
                }
            }
            CommandAction::TrashSelection(path) => {
                self.select_context_target(&path);
                let mut paths: Vec<_> = self.selected_items.iter().cloned().collect();
                paths.sort();
                self.trash_confirm = Some(paths);
            }
            CommandAction::CopySelection(path) => {
                self.select_context_target(&path);
                self.prepare_transfer(false);
            }
            CommandAction::MoveSelection(path) => {
                self.select_context_target(&path);
                self.prepare_transfer(true);
            }
            CommandAction::CopyItems(path) => self.set_file_clipboard(&path, false),
            CommandAction::CutItems(path) => self.set_file_clipboard(&path, true),
            CommandAction::PasteItems => self.paste_file_clipboard(),
            CommandAction::PasteInto(path) => self.paste_file_clipboard_into(path),
            CommandAction::Compress(path, format) => self.compress_selection(ctx, &path, format),
            CommandAction::Extract(path) => self.extract_archive(ctx, &path),
            CommandAction::CreateFolder => self.new_dir_open = true,
            CommandAction::ToggleHidden => {
                let tab = self.explorer.active_tab_mut();
                tab.show_hidden = !tab.show_hidden;
                tab.refresh();
            }
            CommandAction::Terminal(path) => {
                if let Err(error) = open_terminal(&path) {
                    self.status = format!("Não foi possível abrir o terminal: {error}");
                }
            }
            CommandAction::Editor(path) => {
                if let Err(error) = open_editor(&path) {
                    self.status = format!("Não foi possível abrir o editor: {error}");
                }
            }
            CommandAction::FolderColor(path, color) => self.set_folder_color(path, color),
        }
        self.command_open = false;
    }

    pub(super) fn set_folder_color(&mut self, path: PathBuf, color: Option<[u8; 3]>) {
        let Some(conn) = &self.conn else {
            self.status = "Banco de dados indisponível para salvar a cor.".into();
            return;
        };
        let key = format!("folder_color:{}", path.to_string_lossy());
        let result = if let Some([red, green, blue]) = color {
            db::set_settings(conn, &[(key.as_str(), format!("{red},{green},{blue}"))])
        } else {
            db::remove_setting(conn, &key)
        };
        match result {
            Ok(()) => {
                if let Some(color) = color {
                    self.folder_colors.insert(path, color);
                } else {
                    self.folder_colors.remove(&path);
                }
            }
            Err(error) => self.status = format!("Erro ao salvar cor da pasta: {error}"),
        }
    }

    pub(super) fn ui_entry_context_menu(&self, ui: &mut egui::Ui, entry: &Entry) -> Option<CommandAction> {
        let selected_count = if self.selected_items.contains(&entry.path) {
            self.selected_items.len()
        } else {
            1
        };
        let count = count_suffix(selected_count);
        let mut action = None;
        if ui.button("Abrir").clicked() {
            action = Some(CommandAction::Open(entry.path.clone()));
            ui.close_menu();
        }
        if ui.button("Prévia rápida").clicked() {
            action = Some(CommandAction::QuickLook(entry.path.clone()));
            ui.close_menu();
        }
        if !entry.is_dir && is_executable(&entry.path) && ui.button("Executar").clicked() {
            action = Some(CommandAction::Run(entry.path.clone()));
            ui.close_menu();
        }
        if ui.button("Copiar localização").clicked() {
            action = Some(CommandAction::CopyLocation(entry.path.clone()));
            ui.close_menu();
        }
        ui.separator();
        if ui.button("Renomear").clicked() {
            let name = entry.name.clone();
            action = Some(CommandAction::Rename(entry.path.clone(), name));
            ui.close_menu();
        }
        if selected_count > 1
            && ui
                .button(format!("Renomear em lote{count}"))
                .clicked()
        {
            action = Some(CommandAction::RenameSelection(entry.path.clone()));
            ui.close_menu();
        }
        if ui
            .button(format!("Copiar para pasta…{count}"))
            .clicked()
        {
            action = Some(CommandAction::CopySelection(entry.path.clone()));
            ui.close_menu();
        }
        if ui
            .button(format!("Mover para pasta…{count}"))
            .clicked()
        {
            action = Some(CommandAction::MoveSelection(entry.path.clone()));
            ui.close_menu();
        }
        ui.separator();
        if ui.button(format!("Copiar{count}  Ctrl+C")).clicked() {
            action = Some(CommandAction::CopyItems(entry.path.clone()));
            ui.close_menu();
        }
        if ui.button(format!("Recortar{count}  Ctrl+X")).clicked() {
            action = Some(CommandAction::CutItems(entry.path.clone()));
            ui.close_menu();
        }
        let paste_target = if entry.is_dir {
            entry.path.clone()
        } else {
            self.explorer.active_tab().cwd.clone()
        };
        let paste_label = if entry.is_dir {
            format!("Colar em «{}»  Ctrl+V", entry.name)
        } else {
            "Colar aqui  Ctrl+V".to_string()
        };
        if ui
            .add_enabled(
                self.transfer.is_none(),
                egui::Button::new(paste_label),
            )
            .clicked()
        {
            action = Some(CommandAction::PasteInto(paste_target));
            ui.close_menu();
        }
        if ui
            .button(format!("Mover para lixeira{count}"))
            .clicked()
        {
            action = Some(CommandAction::TrashSelection(entry.path.clone()));
            ui.close_menu();
        }
        ui.separator();
        if ui.button("Compactar em ZIP").clicked() {
            action = Some(CommandAction::Compress(entry.path.clone(), archive::Format::Zip));
            ui.close_menu();
        }
        if ui.button("Compactar em tar.gz").clicked() {
            action = Some(CommandAction::Compress(
                entry.path.clone(),
                archive::Format::TarGz,
            ));
            ui.close_menu();
        }
        if !entry.is_dir
            && archive::is_archive(&entry.path)
            && ui.button("Extrair aqui").clicked()
        {
            action = Some(CommandAction::Extract(entry.path.clone()));
            ui.close_menu();
        }
        ui.separator();
        if ui.button("Abrir no VS Code").clicked() {
            action = Some(CommandAction::Editor(entry.path.clone()));
            ui.close_menu();
        }
        if ui.button("Abrir terminal aqui").clicked() {
            let dir = if entry.is_dir {
                entry.path.clone()
            } else {
                entry.path.parent().unwrap_or(&entry.path).to_path_buf()
            };
            action = Some(CommandAction::Terminal(dir));
            ui.close_menu();
        }
        if entry.is_dir {
            ui.menu_button("Cor da pasta", |ui| {
                for (label, color) in [
                    ("Azul", Some([72, 132, 255])),
                    ("Verde", Some([70, 178, 116])),
                    ("Âmbar", Some([228, 166, 64])),
                    ("Rosa", Some([218, 104, 148])),
                    ("Ciano", Some([48, 180, 190])),
                    ("Padrão", None),
                ] {
                    if ui.button(label).clicked() {
                        action = Some(CommandAction::FolderColor(entry.path.clone(), color));
                        ui.close_menu();
                    }
                }
            });
        }
        action
    }

    pub(super) fn ui_folder_context_menu(&self, ui: &mut egui::Ui) -> Option<CommandAction> {
        let selected = self
            .explorer
            .active_tab()
            .selected
            .as_ref()
            .filter(|path| self.selected_items.contains(*path))
            .cloned()
            .or_else(|| self.selected_items.iter().next().cloned());
        let count = count_suffix(self.selected_items.len());
        let mut action = None;

        if let Some(path) = selected.clone() {
            if ui.button(format!("Copiar{count}  Ctrl+C")).clicked() {
                action = Some(CommandAction::CopyItems(path.clone()));
                ui.close_menu();
            }
            if ui.button(format!("Recortar{count}  Ctrl+X")).clicked() {
                action = Some(CommandAction::CutItems(path));
                ui.close_menu();
            }
            ui.separator();
        }

        if ui
            .add_enabled(
                self.transfer.is_none(),
                egui::Button::new("Colar aqui  Ctrl+V"),
            )
            .clicked()
        {
            action = Some(CommandAction::PasteItems);
            ui.close_menu();
        }
        if ui.button("Nova pasta").clicked() {
            action = Some(CommandAction::CreateFolder);
            ui.close_menu();
        }
        if let Some(path) = selected {
            ui.separator();
            if ui.button("Compactar em ZIP").clicked() {
                action = Some(CommandAction::Compress(path.clone(), archive::Format::Zip));
                ui.close_menu();
            }
            if ui.button("Compactar em tar.gz").clicked() {
                action = Some(CommandAction::Compress(path.clone(), archive::Format::TarGz));
                ui.close_menu();
            }
            if !path.is_dir() && archive::is_archive(&path) && ui.button("Extrair aqui").clicked() {
                action = Some(CommandAction::Extract(path));
                ui.close_menu();
            }
        }
        action
    }

    pub(super) fn ui_sidebar(&mut self, ui: &mut egui::Ui) {
        let accent = self.accent();
        ui.add_space(4.0);
        ui.heading("NEXUS");
        ui.weak("arquivos & armazenamento");
        ui.add_space(10.0);

        if sidebar_item(
            ui,
            self.mode == Mode::Explorer,
            "Explorer",
            NavIcon::Explorer,
            accent,
        ) {
            self.mode = Mode::Explorer;
        }
        if sidebar_item(
            ui,
            self.mode == Mode::Search,
            "Pesquisa",
            NavIcon::Search,
            accent,
        ) {
            self.mode = Mode::Search;
        }
        if sidebar_item(
            ui,
            self.mode == Mode::Storage,
            "Armazenamento",
            NavIcon::Storage,
            accent,
        ) {
            self.mode = Mode::Storage;
        }

        ui.add_space(6.0);
        ui.separator();
        ui.add_space(4.0);
        ui.strong("Locais");

        let mut nav: Option<PathBuf> = None;
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
        if sidebar_item(ui, false, "Início", NavIcon::Home, accent) {
            nav = Some(PathBuf::from(&home));
        }
        for (label, path, icon) in &self.user_locations {
            if sidebar_item(ui, false, label, *icon, accent) {
                nav = Some(path.clone());
            }
        }
        let trash_path = PathBuf::from(&home).join(".local/share/Trash/files");
        if trash_path.is_dir() && sidebar_item(ui, false, "Lixeira", NavIcon::Trash, accent) {
            nav = Some(trash_path);
        }
        ui.add_space(6.0);
        ui.separator();
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.strong("Favoritos");
            let cwd = self
                .explorer
                .active_tab()
                .cwd
                .to_string_lossy()
                .into_owned();
            let is_fav = self.favorites.iter().any(|f| f == &cwd);
            if ui
                .small_button(if is_fav { "−" } else { "+" })
                .on_hover_text(if is_fav {
                    "Remover pasta atual dos favoritos"
                } else {
                    "Favoritar pasta atual"
                })
                .clicked()
            {
                self.toggle_favorite(&cwd);
            }
        });

        let mut remove: Option<String> = None;
        let mut open: Option<PathBuf> = None;
        for f in &self.favorites {
            ui.horizontal(|ui| {
                if ui.small_button("×").clicked() {
                    remove = Some(f.clone());
                }
                let name = Path::new(f)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| f.clone());
                if sidebar_item(ui, false, &name, NavIcon::Favorite, accent) {
                    open = Some(PathBuf::from(f));
                }
            });
        }
        if self.favorites.is_empty() {
            ui.weak("Use + para guardar uma pasta.");
        }

        ui.add_space(8.0);
        ui.separator();
        ui.strong("Mídia");
        let dropped: Vec<egui::DroppedFile> = ui.ctx().input(|i| i.raw.dropped_files.clone());
        for file in dropped {
            let path = if let Some(path) = file.path.clone() {
                path
            } else if let Some(bytes) = file.bytes.clone() {
                let name = if file.name.trim().is_empty() {
                    "media".to_string()
                } else {
                    file.name.clone()
                };
                let dir = std::env::temp_dir().join("nexus-media");
                let _ = std::fs::create_dir_all(&dir);
                let target = dir.join(name);
                if std::fs::write(&target, &*bytes).is_ok() {
                    target
                } else {
                    continue;
                }
            } else {
                continue;
            };
            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .or_else(|| {
                    file.name
                        .rsplit('.')
                        .next()
                        .map(|e| e.to_ascii_lowercase())
                })
                .unwrap_or_default();
            if matches!(
                ext.as_str(),
                "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp"
            ) {
                self.settings.sidebar_image = path.to_string_lossy().into_owned();
                self.sidebar_reload = true;
                self.save_settings();
            }
        }
        let width = ui.available_width();
        let height = ui.available_height().max(60.0);
        if let Some((_, tex)) = &self.sidebar_tex {
            ui.add_sized(
                egui::vec2(width, height),
                egui::Image::new(tex).fit_to_exact_size(egui::vec2(width, height)),
            );
        } else {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
            ui.painter().rect_stroke(
                rect,
                6.0,
                egui::Stroke::new(1.0, egui::Color32::from_gray(70)),
            );
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Arraste imagem/GIF aqui",
                egui::FontId::proportional(11.0),
                egui::Color32::from_gray(150),
            );
        }
        if let Some(err) = &self.sidebar_error {
            ui.colored_label(egui::Color32::LIGHT_RED, err);
        }

        if let Some(p) = nav {
            self.navigate_to(p);
        }
        if let Some(f) = remove {
            self.toggle_favorite(&f);
        }
        if let Some(p) = open {
            self.navigate_to(p);
        }
    }

    pub(super) fn ui_tabs(&mut self, ui: &mut egui::Ui) {
        let mut switch_tab: Option<usize> = None;
        let mut close_tab: Option<usize> = None;
        let mut new_tab = false;
        ui.horizontal(|ui| {
            egui::ScrollArea::horizontal()
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (i, t) in self.explorer.tabs.iter().enumerate() {
                            if ui
                                .selectable_label(self.explorer.active == i, t.title())
                                .clicked()
                            {
                                switch_tab = Some(i);
                            }
                            if self.explorer.tabs.len() > 1 && ui.small_button("×").clicked() {
                                close_tab = Some(i);
                            }
                            ui.separator();
                        }
                        if ui.small_button("+ aba").clicked() {
                            new_tab = true;
                        }
                    });
                });
        });
        if let Some(i) = switch_tab {
            self.explorer.active = i;
            self.selected_items.clear();
            if let Some(path) = self.explorer.active_tab().selected.clone() {
                self.selected_items.insert(path);
            }
            self.selection_anchor = self.explorer.active_tab().selected.clone();
        }
        if let Some(i) = close_tab {
            self.explorer.close_tab(i);
        }
        if new_tab {
            let cwd = self.explorer.active_tab().cwd.clone();
            self.explorer.open_tab(cwd);
        }
    }

    pub(super) fn breadcrumb(&mut self, ui: &mut egui::Ui) {
        let cwd = self.explorer.active_tab().cwd.clone();
        let mut nav: Option<PathBuf> = None;
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new("/").frame(false)).clicked() {
                nav = Some(PathBuf::from("/"));
            }
            let parts: Vec<String> = cwd
                .to_string_lossy()
                .split('/')
                .filter(|p| !p.is_empty())
                .map(|p| p.to_string())
                .collect();
            let skip = parts.len().saturating_sub(5);
            if skip > 0 {
                ui.weak("…");
            }
            let mut acc = PathBuf::from("/");
            for (i, part) in parts.iter().enumerate() {
                if i < skip {
                    acc.push(part);
                    continue;
                }
                acc.push(part);
                ui.weak("/");
                if ui.add(egui::Button::new(part).frame(false)).clicked() {
                    nav = Some(acc.clone());
                }
            }
        });
        if let Some(p) = nav {
            self.navigate_to(p);
        }
    }

    pub(super) fn ui_toolbar(&mut self, ui: &mut egui::Ui) {
        let (can_back, can_fwd, can_up) = {
            let t = self.explorer.active_tab();
            (t.can_back(), t.can_forward(), t.cwd.parent().is_some())
        };
        let mut go_back = false;
        let mut go_fwd = false;
        let mut go_up = false;
        let mut do_refresh = false;
        let mut search_go = false;
        let mut view_changed = false;

        ui.horizontal(|ui| {
            if ui.add_enabled(can_back, egui::Button::new("←")).clicked() {
                go_back = true;
            }
            if ui.add_enabled(can_fwd, egui::Button::new("→")).clicked() {
                go_fwd = true;
            }
            if ui.add_enabled(can_up, egui::Button::new("↑")).clicked() {
                go_up = true;
            }
            if ui.small_button("⟳").on_hover_text("Atualizar").clicked() {
                do_refresh = true;
            }
            if ui
                .small_button("⌘")
                .on_hover_text("Paleta de comandos · Ctrl+K / Ctrl+P")
                .clicked()
            {
                self.command_open = true;
                self.command_query.clear();
                self.command_index = 0;
            }

            if self.path_edit {
                let tab = self.explorer.active_tab_mut();
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut tab.path_input)
                        .id(egui::Id::new("nexus-path-input"))
                        .desired_width(360.0)
                        .hint_text("caminho"),
                );
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                if resp.lost_focus() && enter {
                    let _ = tab.navigate_input();
                }
                if ui.small_button("✕").clicked() {
                    self.path_edit = false;
                }
            } else {
                self.breadcrumb(ui);
                if ui
                    .small_button("✎")
                    .on_hover_text("Digitar caminho")
                    .clicked()
                {
                    self.path_edit = true;
                    self.explorer.active_tab_mut().path_input = self
                        .explorer
                        .active_tab()
                        .cwd
                        .to_string_lossy()
                        .into_owned();
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Aparência…").clicked() {
                    self.settings_open = true;
                }
                if ui
                    .selectable_label(self.settings.show_preview, "Prévia")
                    .clicked()
                {
                    self.settings.show_preview = !self.settings.show_preview;
                    self.save_settings();
                }
                if ui
                    .selectable_label(self.settings.view == ViewMode::Grid, "▦")
                    .on_hover_text("Grade")
                    .clicked()
                {
                    self.settings.view = ViewMode::Grid;
                    view_changed = true;
                }
                if ui
                    .selectable_label(self.settings.view == ViewMode::List, "☰")
                    .on_hover_text("Lista")
                    .clicked()
                {
                    self.settings.view = ViewMode::List;
                    view_changed = true;
                }
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.search.text)
                        .id(egui::Id::new("nexus-explorer-search"))
                        .desired_width(180.0)
                        .hint_text("Pesquisar…"),
                );
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    search_go = true;
                }
            });
        });

        {
            let tab = self.explorer.active_tab_mut();
            if go_back {
                tab.back();
            }
            if go_fwd {
                tab.forward();
            }
            if go_up {
                tab.parent();
            }
            if do_refresh {
                tab.refresh();
            }
        }
        if view_changed {
            self.save_settings();
        }
        if search_go {
            self.mode = Mode::Search;
            let ctx = ui.ctx().clone();
            self.run_search(&ctx);
        }
    }

    pub(super) fn ui_actions(&mut self, ui: &mut egui::Ui) {
        let selected = self.explorer.active_tab().selected.clone();
        let selected_entry = self.explorer.active_tab().selected_entry().cloned();
        let mut create = false;
        let mut rename = false;
        let mut bulk_rename = false;
        let mut trash = false;
        let mut open = false;
        let mut copy_to = false;
        let mut move_to = false;
        let mut paste = false;

        ui.horizontal_wrapped(|ui| {
            if ui.button("Nova pasta").clicked() {
                create = true;
            }
            if ui
                .add_enabled(selected.is_some(), egui::Button::new("Renomear"))
                .clicked()
            {
                rename = true;
            }
            if ui
                .add_enabled(
                    self.selected_items.len() > 1,
                    egui::Button::new(format!(
                        "Renomear lote{}",
                        count_suffix(self.selected_items.len())
                    )),
                )
                .clicked()
            {
                bulk_rename = true;
            }
            if ui
                .add_enabled(selected.is_some(), egui::Button::new("Lixeira"))
                .clicked()
            {
                trash = true;
            }
            if ui
                .add_enabled(selected.is_some(), egui::Button::new("Abrir"))
                .clicked()
            {
                open = true;
            }
            if ui
                .add_enabled(
                    !self.selected_items.is_empty() && self.transfer.is_none(),
                    egui::Button::new("Copiar para…"),
                )
                .clicked()
            {
                copy_to = true;
            }
            if ui
                .add_enabled(
                    !self.selected_items.is_empty() && self.transfer.is_none(),
                    egui::Button::new("Mover para…"),
                )
                .clicked()
            {
                move_to = true;
            }
            if ui
                .add_enabled(self.transfer.is_none(), egui::Button::new("Colar"))
                .clicked()
            {
                paste = true;
            }
            ui.separator();
            let tab = self.explorer.active_tab_mut();
            if ui.checkbox(&mut tab.show_hidden, "Ocultos").changed() {
                tab.refresh();
            }
            ui.separator();
            ui.label("Ordenar:");
            let sort = self.explorer.active_tab().sort;
            let ascending = self.explorer.active_tab().ascending;
            let sort_label = match (sort, ascending) {
                (SortKey::Name, true) => "Nome (A–Z)",
                (SortKey::Name, false) => "Nome (Z–A)",
                (SortKey::Size, true) => "Tamanho (menor)",
                (SortKey::Size, false) => "Tamanho (maior)",
                (SortKey::Modified, false) => "Data (mais recente)",
                (SortKey::Modified, true) => "Data (mais antiga)",
            };
            egui::ComboBox::from_id_salt("sort-key")
                .selected_text(sort_label)
                .show_ui(ui, |ui| {
                    let tab = self.explorer.active_tab_mut();
                    let mut changed = false;
                    if ui
                        .selectable_value(&mut tab.sort, SortKey::Name, "Nome")
                        .changed()
                    {
                        changed = true;
                    }
                    if ui
                        .selectable_value(&mut tab.sort, SortKey::Size, "Tamanho")
                        .changed()
                    {
                        changed = true;
                    }
                    ui.separator();
                    if ui
                        .selectable_label(
                            tab.sort == SortKey::Modified && !tab.ascending,
                            "Data (mais recente)",
                        )
                        .clicked()
                    {
                        tab.sort = SortKey::Modified;
                        tab.ascending = false;
                        changed = true;
                    }
                    if ui
                        .selectable_label(
                            tab.sort == SortKey::Modified && tab.ascending,
                            "Data (mais antiga)",
                        )
                        .clicked()
                    {
                        tab.sort = SortKey::Modified;
                        tab.ascending = true;
                        changed = true;
                    }
                    if changed {
                        tab.refresh();
                    }
                });
            let asc = self.explorer.active_tab().ascending;
            if ui.small_button(if asc { "↑" } else { "↓" }).clicked() {
                let tab = self.explorer.active_tab_mut();
                tab.ascending = !tab.ascending;
                tab.refresh();
            }
            ui.separator();
            ui.weak(format!(
                "{} itens · {} arquivos",
                self.explorer.active_tab().entries.len(),
                self.explorer
                    .active_tab()
                    .entries
                    .iter()
                    .filter(|e| !e.is_dir)
                    .count()
            ));
        });

        if create {
            self.new_dir_open = true;
        }
        if rename {
            if let Some(e) = &selected_entry {
                self.rename_target = Some((e.path.clone(), e.name.clone()));
            }
        }
        if bulk_rename {
            self.bulk_rename_open = true;
        }
        if trash {
            if let Some(p) = selected {
                self.select_context_target(&p);
                let mut paths: Vec<_> = self.selected_items.iter().cloned().collect();
                paths.sort();
                self.trash_confirm = Some(paths);
            }
        }
        if open {
            if let Some(e) = selected_entry {
                if e.is_dir {
                    self.navigate_to(e.path);
                } else if let Err(err) = fileops::open_external(&e.path) {
                    self.status = format!("Erro ao abrir: {err}");
                }
            }
        }
        if copy_to {
            self.prepare_transfer(false);
        }
        if move_to {
            self.prepare_transfer(true);
        }
        if paste {
            self.paste_file_clipboard();
        }

        if self.new_dir_open {
            let text_id = ui.id().with("new-dir-name");
            let mut submit = false;
            ui.horizontal(|ui| {
                ui.label("Nome da pasta:");
                let te = ui.add(
                    egui::TextEdit::singleline(&mut self.new_dir_name)
                        .id(text_id)
                        .desired_width(220.0),
                );
                if !te.has_focus() {
                    ui.ctx().memory_mut(|m| m.request_focus(text_id));
                }
                if te.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    submit = true;
                }
                if ui.button("Criar").clicked() {
                    submit = true;
                }
                if ui.button("Cancelar").clicked() {
                    self.new_dir_open = false;
                    self.new_dir_name.clear();
                }
            });
            if submit {
                let cwd = self.explorer.active_tab().cwd.clone();
                match fileops::create_dir(&cwd, &self.new_dir_name) {
                    Ok(p) => {
                        self.status = format!("Pasta criada: {}", p.display());
                        self.new_dir_name.clear();
                        self.new_dir_open = false;
                        self.explorer.active_tab_mut().refresh();
                    }
                    Err(e) => self.status = format!("Erro ao criar pasta: {e}"),
                }
            }
        }
        self.ui_bulk_rename(ui.ctx());
    }

    pub(super) fn grid_body(&mut self, ui: &mut egui::Ui) -> (Option<PathBuf>, Option<PathBuf>) {
        let entries = self.explorer.active_tab().entries.clone();
        let ctx = ui.ctx().clone();
        let selected = self.explorer.active_tab().selected.clone();
        let accent = self.accent();
        let icon = self.settings.tile_size.clamp(48.0, 220.0);
        let tile_w = icon + 28.0;
        let tile_h = icon + 58.0;
        let mut new_sel = None;
        let mut enter = None;
        let mut menu_action = None;

        let selection_area = ui.available_rect_before_wrap();
        let selection_response = ui.interact(
            selection_area,
            ui.id().with("file-selection-marquee"),
            Sense::click_and_drag(),
        );
        let mut background_action = None;
        selection_response.context_menu(|ui| {
            background_action = self.ui_folder_context_menu(ui);
        });
        if entries.is_empty() {
            ui.add_space(24.0);
            ui.weak("Pasta vazia ou sem permissão de leitura.");
            if let Some(action) = background_action {
                surrender_explorer_text_focus(ui.ctx());
                self.execute_command(ui.ctx(), action);
            }
            return (new_sel, enter);
        }
        if selection_response.drag_started() {
            self.selection_drag_start = selection_response.interact_pointer_pos();
            let additive = ui.input(|input| input.modifiers.ctrl || input.modifiers.command);
            self.selection_drag_base = if additive {
                self.selected_items.clone()
            } else {
                HashSet::new()
            };
            self.selected_items = self.selection_drag_base.clone();
        }
        let drag_rect = self.selection_drag_start.and_then(|start| {
            selection_response
                .interact_pointer_pos()
                .filter(|_| selection_response.dragged() || selection_response.drag_stopped())
                .map(|pointer| egui::Rect::from_two_pos(start, pointer))
        });
        if selection_response.dragged() {
            self.selected_items = self.selection_drag_base.clone();
        }

        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            // Sem arrastar-para-rolar: o arrasto no fundo é da seleção por retângulo.
            .drag_to_scroll(false)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                ui.horizontal_wrapped(|ui| {
                    for e in &entries {
                        let (rect, resp) =
                            ui.allocate_exact_size(egui::vec2(tile_w, tile_h), Sense::click_and_drag());
                        let drag_payload = if self.selected_items.contains(&e.path)
                            && !self.selected_items.is_empty()
                        {
                            self.selected_items.iter().cloned().collect::<Vec<_>>()
                        } else {
                            vec![e.path.clone()]
                        };
                        resp.dnd_set_drag_payload(drag_payload.clone());
                        if e.is_dir {
                            if resp.dnd_hover_payload::<Vec<PathBuf>>().is_some() {
                                ui.painter().rect_stroke(rect, 8.0, egui::Stroke::new(2.0, accent));
                            }
                            if let Some(payload) = resp.dnd_release_payload::<Vec<PathBuf>>() {
                                let paths: Vec<PathBuf> = payload.iter().cloned().collect();
                                if self.transfer.is_none()
                                    && !paths.is_empty()
                                    && !paths.iter().any(|p| p == &e.path)
                                {
                                    self.transfer_paths = paths;
                                    self.transfer_destination = e.path.to_string_lossy().into_owned();
                                    self.transfer_move = true;
                                    self.transfer_open = false;
                                    self.start_transfer();
                                }
                            }
                        }
                        if let Some(selection_rect) = drag_rect {
                            if rect.intersects(selection_rect) {
                                self.selected_items.insert(e.path.clone());
                            }
                        }
                        let is_sel = selected.as_ref() == Some(&e.path)
                            || self.selected_items.contains(&e.path);
                        let painter = ui.painter();
                        if is_sel || resp.hovered() {
                            let fill = if is_sel {
                                Color32::from_rgba_unmultiplied(
                                    accent.r(),
                                    accent.g(),
                                    accent.b(),
                                    70,
                                )
                            } else {
                                Color32::from_white_alpha(14)
                            };
                            painter.rect_filled(rect, 8.0, fill);
                        }
                        if is_sel {
                            painter.rect_stroke(rect, 8.0, egui::Stroke::new(1.2_f32, accent));
                        }

                        let icon_rect = egui::Rect::from_center_size(
                            egui::pos2(rect.center().x, rect.top() + 10.0 + icon / 2.0),
                            egui::vec2(icon, icon),
                        );
                        let is_image = !e.is_dir
                            && e.path.extension().is_some_and(|ext| {
                                matches!(
                                    ext.to_string_lossy().to_ascii_lowercase().as_str(),
                                    "png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif"
                                )
                            });
                        if is_image && ui.is_rect_visible(rect) {
                            self.request_thumbnail(&e.path, &ctx);
                        }
                        if let Some(texture) = self.thumbnails.get(&e.path) {
                            painter.image(
                                texture.id(),
                                contain_rect(icon_rect, texture.size_vec2()),
                                egui::Rect::from_min_max(
                                    egui::pos2(0.0, 0.0),
                                    egui::pos2(1.0, 1.0),
                                ),
                                Color32::WHITE,
                            );
                        } else {
                            let folder_accent = self
                                .folder_colors
                                .get(&e.path)
                                .map(|[red, green, blue]| Color32::from_rgb(*red, *green, *blue))
                                .unwrap_or(accent);
                            super::icons::draw_entry_icon(
                                painter,
                                icon_rect,
                                e,
                                folder_accent,
                                self.folder_colors.contains_key(&e.path),
                            );
                        }
                        draw_entry_badges(painter, icon_rect, e);

                        let max_chars = ((tile_w / 7.0) as usize).max(6);
                        let (l1, l2) = name_lines(&e.name, max_chars);
                        let text_color = if is_sel {
                            Color32::WHITE
                        } else {
                            Color32::from_gray(228)
                        };
                        painter.with_clip_rect(rect).text(
                            egui::pos2(rect.center().x, icon_rect.bottom() + 5.0),
                            Align2::CENTER_TOP,
                            l1,
                            FontId::proportional(12.0),
                            text_color,
                        );
                        if let Some(l2) = l2 {
                            painter.with_clip_rect(rect).text(
                                egui::pos2(rect.center().x, icon_rect.bottom() + 19.0),
                                Align2::CENTER_TOP,
                                l2,
                                FontId::proportional(12.0),
                                text_color,
                            );
                        }
                        let subtitle = if e.is_dir {
                            "Pasta".to_string()
                        } else {
                            human(e.size)
                        };
                        painter.with_clip_rect(rect).text(
                            egui::pos2(rect.center().x, rect.bottom() - 4.0),
                            Align2::CENTER_BOTTOM,
                            subtitle,
                            FontId::proportional(10.0),
                            Color32::from_gray(160),
                        );

                        let resp = resp.on_hover_text(entry_tooltip(e));
                        let mut context_action = None;
                        resp.context_menu(|ui| {
                            context_action = self.ui_entry_context_menu(ui, e);
                        });
                        if let Some(action) = context_action {
                            menu_action = Some(action);
                        }
                        if resp.clicked() {
                            surrender_explorer_text_focus(ui.ctx());
                            let (multi, range) = ui.input(|input| {
                                (
                                    input.modifiers.ctrl || input.modifiers.command,
                                    input.modifiers.shift,
                                )
                            });
                            self.select_clicked(&e.path, &entries, multi, range);
                            new_sel = Some(e.path.clone());
                        }
                        if resp.double_clicked() && e.is_dir {
                            enter = Some(e.path.clone());
                        }
                    }
                });
            });

        if selection_response.dragged() {
            self.explorer.active_tab_mut().selected = entries
                .iter()
                .find(|entry| self.selected_items.contains(&entry.path))
                .map(|entry| entry.path.clone());
        }
        if menu_action.is_none() {
            menu_action = background_action;
        }
        if let Some(action) = menu_action {
            surrender_explorer_text_focus(ui.ctx());
            self.execute_command(ui.ctx(), action);
        }
        if let Some(selection_rect) = drag_rect.filter(|_| selection_response.dragged()) {
            ui.painter().rect_filled(
                selection_rect,
                2.0,
                Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 38),
            );
            ui.painter()
                .rect_stroke(selection_rect, 2.0, egui::Stroke::new(1.0_f32, accent));
        }
        if selection_response.drag_stopped() {
            self.selection_drag_start = None;
            self.selection_drag_base.clear();
            self.selection_anchor = entries
                .iter()
                .find(|entry| self.selected_items.contains(&entry.path))
                .map(|entry| entry.path.clone());
            self.explorer.active_tab_mut().selected = self.selection_anchor.clone();
        }
        self.paint_drag_ghost(&ctx, &entries);
        (new_sel, enter)
    }

    pub(super) fn list_body(&mut self, ui: &mut egui::Ui) -> (Option<PathBuf>, Option<PathBuf>) {
        let entries = self.explorer.active_tab().entries.clone();
        let selected = self.explorer.active_tab().selected.clone();
        let accent = self.accent();
        let mut new_sel = None;
        let mut enter = None;
        let mut menu_action = None;
        let background_response = ui.interact(
            ui.available_rect_before_wrap(),
            ui.id().with("folder-background-menu"),
            Sense::click(),
        );
        let mut background_action = None;
        background_response.context_menu(|ui| {
            background_action = self.ui_folder_context_menu(ui);
        });

        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                egui::Grid::new("file-grid")
                    .num_columns(4)
                    .striped(true)
                    .spacing([10.0, 3.0])
                    .show(ui, |ui| {
                        ui.strong("Nome");
                        ui.strong("Lógico");
                        ui.strong("Em disco");
                        ui.strong("Modificado");
                        ui.end_row();
                        for e in &entries {
                            let folder_accent = self
                                .folder_colors
                                .get(&e.path)
                                .map(|[red, green, blue]| Color32::from_rgb(*red, *green, *blue))
                                .unwrap_or(accent);
                            let resp = ui
                                .horizontal(|ui| {
                                    let (icon_rect, _) = ui.allocate_exact_size(
                                        egui::vec2(18.0, 18.0),
                                        Sense::hover(),
                                    );
                                    super::icons::draw_entry_icon(
                                        ui.painter(),
                                        icon_rect,
                                        e,
                                        folder_accent,
                                        self.folder_colors.contains_key(&e.path),
                                    );
                                    draw_entry_badges(ui.painter(), icon_rect, e);
                                    ui.add(
                                        egui::Button::new(&e.name)
                                            .selected(
                                                selected.as_ref() == Some(&e.path)
                                                    || self.selected_items.contains(&e.path),
                                            )
                                            .sense(Sense::click_and_drag()),
                                    )
                                    .on_hover_text(entry_tooltip(e))
                                })
                                .inner;
                            let list_payload = if self.selected_items.contains(&e.path)
                                && !self.selected_items.is_empty()
                            {
                                self.selected_items.iter().cloned().collect::<Vec<_>>()
                            } else {
                                vec![e.path.clone()]
                            };
                            resp.dnd_set_drag_payload(list_payload);
                            if e.is_dir {
                                if let Some(payload) = resp.dnd_release_payload::<Vec<PathBuf>>() {
                                    let paths: Vec<PathBuf> = payload.iter().cloned().collect();
                                    if self.transfer.is_none()
                                        && !paths.is_empty()
                                        && !paths.iter().any(|p| p == &e.path)
                                    {
                                        self.transfer_paths = paths;
                                        self.transfer_destination =
                                            e.path.to_string_lossy().into_owned();
                                        self.transfer_move = true;
                                        self.transfer_open = false;
                                        self.start_transfer();
                                    }
                                }
                            }
                            if resp.drag_started() {
                                self.list_drag_selecting = true;
                                let additive = ui
                                    .input(|input| input.modifiers.ctrl || input.modifiers.command);
                                self.select_clicked(&e.path, &entries, additive, false);
                            }
                            let (pointer_down, pointer) = ui.input(|input| {
                                (input.pointer.primary_down(), input.pointer.interact_pos())
                            });
                            if self.list_drag_selecting
                                && pointer_down
                                && pointer.is_some_and(|position| resp.rect.contains(position))
                            {
                                self.selected_items.insert(e.path.clone());
                                self.explorer.active_tab_mut().selected = Some(e.path.clone());
                            }
                            let mut context_action = None;
                            resp.context_menu(|ui| {
                                context_action = self.ui_entry_context_menu(ui, e);
                            });
                            if let Some(action) = context_action {
                                menu_action = Some(action);
                            }
                            if resp.clicked() {
                                surrender_explorer_text_focus(ui.ctx());
                                let (multi, range) = ui.input(|input| {
                                    (
                                        input.modifiers.ctrl || input.modifiers.command,
                                        input.modifiers.shift,
                                    )
                                });
                                self.select_clicked(&e.path, &entries, multi, range);
                                new_sel = Some(e.path.clone());
                            }
                            if resp.drag_stopped() {
                                self.list_drag_selecting = false;
                            }
                            if resp.double_clicked() && e.is_dir {
                                enter = Some(e.path.clone());
                            }
                            if e.is_dir {
                                ui.weak("—");
                                ui.weak("—");
                            } else {
                                ui.monospace(human(e.size));
                                ui.monospace(human(e.alloc));
                            }
                            ui.weak(
                                e.modified
                                    .map(|t| {
                                        let iso = core::iso_utc(core::to_secs(t));
                                        iso[..10].to_string()
                                    })
                                    .unwrap_or_else(|| "—".into()),
                            );
                            ui.end_row();
                        }
                    });
            });

        if menu_action.is_none() {
            menu_action = background_action;
        }
        if let Some(action) = menu_action {
            surrender_explorer_text_focus(ui.ctx());
            self.execute_command(ui.ctx(), action);
        }
        if !ui.input(|input| input.pointer.primary_down()) {
            self.list_drag_selecting = false;
        }
        let ctx = ui.ctx().clone();
        self.paint_drag_ghost(&ctx, &entries);
        (new_sel, enter)
    }

    pub(super) fn ui_explorer(&mut self, ui: &mut egui::Ui) {
        self.ui_tabs(ui);
        ui.add_space(4.0);
        self.ui_toolbar(ui);
        ui.add_space(2.0);
        self.ui_actions(ui);
        if let Some(err) = self.explorer.active_tab().error.clone() {
            ui.colored_label(Color32::LIGHT_RED, err);
        }
        ui.separator();

        if self.settings.show_preview {
            egui::SidePanel::right("preview")
                .resizable(true)
                .default_width(300.0)
                .frame(
                    egui::Frame::none()
                        .fill(Color32::from_rgba_unmultiplied(18, 18, 24, (self.settings.preview_alpha * 255.0).round() as u8))
                        .inner_margin(egui::Margin::same(10.0)),
                )
                .show_inside(ui, |ui| self.ui_preview(ui));
        }

        let (_new_sel, enter) = match self.settings.view {
            ViewMode::Grid => self.grid_body(ui),
            ViewMode::List => self.list_body(ui),
        };
        if let Some(p) = enter {
            let _ = self.explorer.active_tab_mut().navigate(p);
            self.selected_items.clear();
            self.selection_anchor = None;
        }
    }

    pub(super) fn ui_search(&mut self, ui: &mut egui::Ui) {
        let mut run_search = false;
        ui.heading("Pesquisa");
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let search_width = (ui.available_width() - 150.0).clamp(180.0, 620.0);
            let query = ui.add_sized(
                [search_width, 34.0],
                egui::TextEdit::singleline(&mut self.search.text)
                    .hint_text("Buscar nome ou caminho…"),
            );
            run_search |=
                query.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if ui
                .add_enabled(!self.busy, egui::Button::new("Pesquisar"))
                .clicked()
            {
                run_search = true;
            }
            if ui.small_button("Limpar").clicked() {
                self.search.text.clear();
                self.search.ext.clear();
                self.search.min_mb = 0;
                self.search.max_mb = 0;
                self.search.hits.clear();
                self.search.note.clear();
            }
            if self.busy {
                ui.spinner();
            }
        });
        egui::Frame::none()
            .fill(Color32::from_white_alpha(7))
            .inner_margin(egui::Margin::symmetric(8.0, 6.0))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.weak("Extensão");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.search.ext)
                            .desired_width(88.0)
                            .hint_text("lua, zip…"),
                    );
                    ui.separator();
                    ui.weak("Tamanho");
                    ui.add(egui::DragValue::new(&mut self.search.min_mb).suffix(" MB"));
                    ui.label("a");
                    ui.add(egui::DragValue::new(&mut self.search.max_mb).suffix(" MB"))
                        .on_hover_text("0 = sem limite máximo");
                    ui.separator();
                    ui.checkbox(&mut self.search.files_only, "Arquivos");
                    ui.checkbox(&mut self.search.here, "Nesta pasta");
                    ui.separator();
                    ui.weak("Ordenar");
                    egui::ComboBox::from_id_salt("search-sort")
                        .selected_text(match self.search.sort {
                            search::SortOrder::Name => "Nome",
                            search::SortOrder::Modified => "Modificado",
                            search::SortOrder::Size => "Tamanho",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.search.sort,
                                search::SortOrder::Name,
                                "Nome",
                            );
                            ui.selectable_value(
                                &mut self.search.sort,
                                search::SortOrder::Modified,
                                "Mais recentes",
                            );
                            ui.selectable_value(
                                &mut self.search.sort,
                                search::SortOrder::Size,
                                "Maiores primeiro",
                            );
                        });
                });
            });
        if self.search.max_mb > 0 && self.search.max_mb < self.search.min_mb {
            ui.colored_label(
                Color32::LIGHT_RED,
                "O tamanho máximo deve ser maior que o mínimo.",
            );
        } else if run_search && !self.busy {
            let ctx = ui.ctx().clone();
            self.run_search(&ctx);
        }
        ui.add_space(8.0);
        if !self.search.note.is_empty() {
            ui.horizontal(|ui| {
                ui.strong(self.search.note.as_str());
                if self.search.here {
                    ui.weak(self.explorer.active_tab().cwd.display().to_string());
                }
            });
        }
        if self.search.hits.is_empty() {
            if !self.search.note.is_empty() {
                ui.add_space(18.0);
                ui.weak("Nenhum item corresponde a esta pesquisa.");
            }
            return;
        }
        let hits = self.search.hits.clone();
        let mut reveal: Option<String> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.weak("RESULTADO");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.weak("MODIFICADO");
                        ui.add_space(44.0);
                        ui.weak("EM DISCO");
                        ui.add_space(28.0);
                        ui.weak("TAMANHO");
                    });
                });
                ui.separator();
                for (row, hit) in hits.iter().enumerate() {
                    let modified = core::iso_utc(hit.mtime);
                    let kind = match hit.kind.as_str() {
                        "dir" => "Pasta".to_string(),
                        "symlink" => "Link".to_string(),
                        _ => hit
                            .path
                            .rsplit_once('.')
                            .map(|(_, extension)| extension.to_uppercase())
                            .unwrap_or_else(|| "Arquivo".into()),
                    };
                    egui::Frame::none()
                        .fill(if row % 2 == 0 {
                            Color32::from_white_alpha(5)
                        } else {
                            Color32::TRANSPARENT
                        })
                        .inner_margin(egui::Margin::symmetric(7.0, 5.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let name_width = (ui.available_width() * 0.43).clamp(150.0, 420.0);
                                ui.allocate_ui_with_layout(
                                    egui::vec2(name_width, 38.0),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        let name_text = if hit.status == "ok" {
                                            egui::RichText::new(&hit.name).strong()
                                        } else {
                                            egui::RichText::new(format!(
                                                "{} · inacessível",
                                                hit.name
                                            ))
                                            .strong()
                                            .color(Color32::LIGHT_RED)
                                        };
                                        let name = ui.add(
                                            egui::Label::new(name_text)
                                                .truncate()
                                                .sense(egui::Sense::click()),
                                        );
                                        name.on_hover_text(format!(
                                            "{}\n{}\n{} · {}\nModificado: {}\nStatus: {}",
                                            hit.name,
                                            hit.path,
                                            human(hit.size),
                                            human(hit.alloc),
                                            modified,
                                            hit.status
                                        ));
                                        ui.weak(&hit.path);
                                    },
                                );
                                ui.weak(kind);
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.small_button("Mostrar").clicked() {
                                            reveal = Some(hit.path.clone());
                                        }
                                        ui.monospace(&modified[..16]);
                                        ui.monospace(human(hit.alloc));
                                        ui.monospace(human(hit.size));
                                    },
                                );
                            });
                        });
                }
            });
        if let Some(p) = reveal {
            self.reveal(&p);
        }
    }

    pub(super) fn ui_top(&mut self, ui: &mut egui::Ui) {
        if self.top.is_empty() {
            ui.weak("Nenhum índice para essa pasta.");
            return;
        }
        let mut reveal: Option<String> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                egui::Grid::new("top-grid")
                    .num_columns(4)
                    .striped(true)
                    .spacing([10.0, 3.0])
                    .show(ui, |ui| {
                        ui.strong("Lógico");
                        ui.strong("Em disco");
                        ui.strong("Caminho");
                        ui.strong("");
                        ui.end_row();
                        for (p, size, alloc) in &self.top {
                            ui.monospace(human(*size));
                            ui.monospace(human(*alloc));
                            ui.label(p.as_str());
                            if ui.small_button("Abrir pasta").clicked() {
                                reveal = Some(p.clone());
                            }
                            ui.end_row();
                        }
                    });
            });
        if let Some(p) = reveal {
            open_parent(&p);
        }
    }
}
