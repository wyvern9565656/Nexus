use super::*;

impl NexusApp {
    pub(super) fn request_thumbnail(&mut self, path: &Path, ctx: &egui::Context) {
        // Enquanto a janela está em outro workspace, não inicia novas miniaturas:
        // cada repintura oculta manteria a fila girando sem necessidade.
        if !self.window_active {
            return;
        }
        if self.thumbnails.contains_key(path) {
            self.thumbnail_order.retain(|cached| cached != path);
            self.thumbnail_order.push_back(path.to_path_buf());
            return;
        }
        if self.thumbnail_pending.len() >= 1 || !self.thumbnail_pending.insert(path.to_path_buf()) {
            return;
        }
        let path = path.to_path_buf();
        let sender = self.thumbnail_tx.clone();
        let ctx = ctx.clone();
        rayon::spawn(move || {
            let color = image::ImageReader::open(&path)
                .ok()
                .and_then(|mut reader| {
                    let mut limits = image::Limits::default();
                    limits.max_image_width = Some(8192);
                    limits.max_image_height = Some(8192);
                    limits.max_alloc = Some(128 * 1024 * 1024);
                    reader.limits(limits);
                    reader.decode().ok()
                })
                .map(|image| {
                    let image = image.thumbnail(480, 360).to_rgba8();
                    let size = [image.width() as usize, image.height() as usize];
                    egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw())
                });
            let _ = sender.send((path, color));
            ctx.request_repaint();
        });
    }

    pub(super) fn request_quicklook_text(&mut self, path: &Path, ctx: &egui::Context) {
        if self
            .quicklook_text
            .as_ref()
            .is_some_and(|(cached_path, _)| cached_path == path)
            || !self.quicklook_text_pending.insert(path.to_path_buf())
        {
            return;
        }
        let path = path.to_path_buf();
        let sender = self.quicklook_text_tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = read_text_preview(&path, 2 * 1024 * 1024).ok_or_else(|| {
                "Não é um arquivo de texto UTF-8 ou excede o limite de prévia.".to_string()
            });
            let _ = sender.send((path, result));
            ctx.request_repaint();
        });
    }

    pub(super) fn request_quicklook_media(&mut self, path: &Path, ctx: &egui::Context) {
        if self.quicklook_media_error.as_deref() == Some(path) {
            return;
        }
        if self.quicklook_media_path.as_deref() == Some(path)
            && (self.quicklook_media_rx.is_some() || self.quicklook_media_texture.is_some())
        {
            return;
        }
        self.quicklook_media_path = Some(path.to_path_buf());
        self.quicklook_media_texture = None;
        self.quicklook_media_error = None;
        let Some(extension) = path
            .extension()
            .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
        else {
            self.quicklook_media_rx = None;
            return;
        };
        let is_gif = extension == "gif";
        let is_video = matches!(
            extension.as_str(),
            "mp4" | "mkv" | "mov" | "avi" | "webm" | "m4v" | "mpeg" | "mpg" | "wmv" | "flv" | "ogv"
        );
        if !is_gif && !is_video {
            self.quicklook_media_rx = None;
            return;
        }
        let (sender, receiver) = mpsc::sync_channel(2);
        let path = path.to_path_buf();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            if is_gif {
                loop {
                    let mut decoder =
                        match std::fs::File::open(&path)
                            .map(BufReader::new)
                            .and_then(|reader| {
                                image::codecs::gif::GifDecoder::new(reader)
                                    .map_err(io::Error::other)
                            }) {
                            Ok(decoder) => decoder,
                            Err(_) => return,
                        };
                    let mut limits = image::Limits::default();
                    limits.max_image_width = Some(4096);
                    limits.max_image_height = Some(4096);
                    limits.max_alloc = Some(96 * 1024 * 1024);
                    if decoder.set_limits(limits).is_err() {
                        return;
                    }
                    let mut had_frame = false;
                    for frame in decoder.into_frames() {
                        let Ok(frame) = frame else {
                            return;
                        };
                        had_frame = true;
                        let (numerator, denominator) = frame.delay().numer_denom_ms();
                        let delay = Duration::from_millis(
                            (numerator / denominator.max(1)).clamp(20, 10_000) as u64,
                        );
                        let image = image::DynamicImage::ImageRgba8(frame.into_buffer())
                            .thumbnail(720, 480)
                            .to_rgba8();
                        let size = [image.width() as usize, image.height() as usize];
                        let image = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
                        if sender.send((image, delay)).is_err() {
                            return;
                        }
                        ctx.request_repaint();
                    }
                    if !had_frame {
                        return;
                    }
                }
            } else {
                let mut child = match Command::new("ffmpeg")
                    .args(["-v", "error", "-nostdin", "-i"])
                    .arg(&path)
                    .args([
                        "-an",
                        "-vf",
                        "fps=8,scale=720:480:force_original_aspect_ratio=decrease,pad=720:480:(ow-iw)/2:(oh-ih)/2,format=rgba",
                        "-f",
                        "rawvideo",
                        "-pix_fmt",
                        "rgba",
                        "pipe:1",
                    ])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                {
                    Ok(child) => child,
                    Err(_) => return,
                };
                let Some(mut output) = child.stdout.take() else {
                    let _ = child.kill();
                    return;
                };
                let mut bytes = vec![0_u8; 720 * 480 * 4];
                loop {
                    if output.read_exact(&mut bytes).is_err() {
                        break;
                    }
                    let image = egui::ColorImage::from_rgba_unmultiplied([720, 480], &bytes);
                    if sender.send((image, Duration::from_millis(125))).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
                drop(output);
                let _ = child.wait();
            }
        });
        self.quicklook_media_rx = Some(receiver);
    }

    pub(super) fn poll_quicklook(&mut self, ctx: &egui::Context) {
        let focused = ctx.input(|input| {
            let viewport = input.viewport();
            viewport.focused.unwrap_or(true) && !viewport.minimized.unwrap_or(false)
        });
        if !focused {
            return;
        }
        for (path, result) in self.quicklook_text_rx.try_iter() {
            self.quicklook_text_pending.remove(&path);
            self.quicklook_text = Some((path, result));
        }
        let Some(receiver) = &self.quicklook_media_rx else {
            return;
        };
        let now = Instant::now();
        if now < self.quicklook_media_next {
            ctx.request_repaint_after(self.quicklook_media_next.saturating_duration_since(now));
            return;
        }
        match receiver.try_recv() {
            Ok((image, delay)) => {
                if let Some(texture) = &mut self.quicklook_media_texture {
                    texture.set(image, egui::TextureOptions::LINEAR);
                } else {
                    self.quicklook_media_texture = Some(ctx.load_texture(
                        "nexus-quicklook-media",
                        image,
                        egui::TextureOptions::LINEAR,
                    ));
                }
                self.quicklook_media_next = Instant::now() + delay;
                ctx.request_repaint_after(delay);
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.quicklook_media_rx = None;
                if self.quicklook_media_texture.is_none() {
                    self.quicklook_media_error = self.quicklook_media_path.clone();
                }
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }

    pub(super) fn ui_quicklook(&mut self, ctx: &egui::Context) {
        if !self.quicklook_open {
            return;
        }
        let selected = self.explorer.active_tab().selected_entry().cloned();
        let selected_media_path = selected.as_ref().and_then(|entry| {
            let extension = entry
                .path
                .extension()?
                .to_string_lossy()
                .to_ascii_lowercase();
            (extension == "gif"
                || matches!(
                    extension.as_str(),
                    "mp4"
                        | "mkv"
                        | "mov"
                        | "avi"
                        | "webm"
                        | "m4v"
                        | "mpeg"
                        | "mpg"
                        | "wmv"
                        | "flv"
                        | "ogv"
                ))
            .then(|| entry.path.clone())
        });
        if self.quicklook_media_path != selected_media_path {
            self.quicklook_media_rx = None;
            self.quicklook_media_texture = None;
            self.quicklook_media_error = None;
            self.quicklook_media_path = None;
        }
        let mut open = true;
        let mut launch_external = None;
        egui::Window::new("Prévia rápida")
            .id(egui::Id::new("nexus-quicklook"))
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .default_size([680.0, 500.0])
            .open(&mut open)
            .show(ctx, |ui| match selected {
                None => {
                    ui.weak("Nenhum item selecionado.");
                }
                Some(entry) if entry.is_dir => {
                    ui.heading(&entry.name);
                    ui.monospace(entry.path.to_string_lossy());
                    ui.weak("Pasta");
                }
                Some(entry) => {
                    let extension = entry
                        .path
                        .extension()
                        .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
                        .unwrap_or_default();
                    ui.horizontal(|ui| {
                        ui.strong(&entry.name);
                        ui.weak(human(entry.size));
                    });
                    ui.separator();
                    let is_video = matches!(
                        extension.as_str(),
                        "mp4"
                            | "mkv"
                            | "mov"
                            | "avi"
                            | "webm"
                            | "m4v"
                            | "mpeg"
                            | "mpg"
                            | "wmv"
                            | "flv"
                            | "ogv"
                    );
                    let is_image = matches!(
                        extension.as_str(),
                        "png"
                            | "jpg"
                            | "jpeg"
                            | "webp"
                            | "bmp"
                            | "gif"
                            | "tif"
                            | "tiff"
                            | "avif"
                            | "heic"
                    );
                    if is_video || extension == "gif" {
                        self.request_quicklook_media(&entry.path, ctx);
                        if let Some(texture) = &self.quicklook_media_texture {
                            let size = contain_size(ui.available_size(), texture.size_vec2());
                            ui.centered_and_justified(|ui| ui.image((texture.id(), size)));
                            ui.weak(if is_video {
                                "Reproduzindo vídeo"
                            } else {
                                "GIF animado"
                            });
                        } else if self.quicklook_media_error.as_deref() == Some(&entry.path) {
                            ui.weak("Não foi possível reproduzir a prévia neste formato.");
                            if ui.button("Abrir no aplicativo padrão").clicked() {
                                launch_external = Some(entry.path.clone());
                            }
                        } else {
                            ui.spinner();
                            ui.weak(if is_video {
                                "Preparando reprodução…"
                            } else {
                                "Carregando animação…"
                            });
                        }
                    } else if is_image {
                        self.request_thumbnail(&entry.path, ctx);
                        if let Some(texture) = self.thumbnails.get(&entry.path) {
                            let size = contain_size(ui.available_size(), texture.size_vec2());
                            ui.centered_and_justified(|ui| {
                                ui.image((texture.id(), size));
                            });
                        } else {
                            ui.spinner();
                            ui.weak("Carregando imagem…");
                        }
                    } else if entry.size <= 32 * 1024 * 1024 {
                        self.request_quicklook_text(&entry.path, ctx);
                        match self
                            .quicklook_text
                            .as_ref()
                            .filter(|(path, _)| path == &entry.path)
                            .map(|(_, result)| result)
                        {
                            Some(Ok(preview)) => {
                                let mut text = preview.clone();
                                egui::ScrollArea::both().show(ui, |ui| {
                                    ui.add(
                                        egui::TextEdit::multiline(&mut text)
                                            .font(egui::TextStyle::Monospace)
                                            .desired_width(f32::INFINITY)
                                            .interactive(false),
                                    );
                                });
                                if entry.size > 2 * 1024 * 1024 {
                                    ui.weak("Prévia limitada aos primeiros 2 MiB");
                                }
                            }
                            Some(Err(error)) => {
                                ui.weak(error);
                                if ui.button("Abrir no aplicativo padrão").clicked() {
                                    launch_external = Some(entry.path.clone());
                                }
                            }
                            None => {
                                ui.spinner();
                                ui.weak("Verificando e carregando texto…");
                            }
                        }
                    } else {
                        ui.weak("Prévia não disponível para este formato.");
                        if ui.button("Abrir no aplicativo padrão").clicked() {
                            launch_external = Some(entry.path.clone());
                        }
                    }
                }
            });
        if let Some(path) = launch_external {
            if let Err(error) = fileops::open_external(&path) {
                self.status = format!("Erro ao abrir: {error}");
            }
        }
        self.quicklook_open = open;
        if !open {
            self.quicklook_media_rx = None;
            self.quicklook_media_path = None;
            self.quicklook_media_texture = None;
            self.quicklook_media_error = None;
        }
    }

    pub(super) fn ui_preview(&mut self, ui: &mut egui::Ui) {
        ui.heading("Prévia");
        let entry = self.explorer.active_tab().selected_entry().cloned();
        let media_path = entry
            .as_ref()
            .filter(|entry| is_preview_video(&entry.path) || is_preview_gif(&entry.path))
            .map(|entry| entry.path.clone());
        if self.quicklook_media_path != media_path {
            self.quicklook_media_rx = None;
            self.quicklook_media_texture = None;
            self.quicklook_media_error = None;
            self.quicklook_media_path = None;
        }
        match &entry {
            None => {
                ui.weak("Selecione um item para ver detalhes.");
            }
            Some(e) => {
                ui.strong(&e.name);
                ui.monospace(e.path.to_string_lossy());
                ui.separator();
                ui.label(if e.is_dir {
                    "Pasta"
                } else if e.is_symlink {
                    "Link simbólico"
                } else {
                    "Arquivo"
                });
                if let Some(extension) = e.path.extension() {
                    ui.label(format!("Tipo: .{}", extension.to_string_lossy()));
                }
                ui.label(format!("Lógico: {}", human(e.size)));
                ui.label(format!("Em disco: {}", human(e.alloc)));
                ui.label(format!(
                    "Modificado: {}",
                    e.modified
                        .map(|time| core::iso_utc(core::to_secs(time)))
                        .unwrap_or_else(|| "Desconhecido".into())
                ));
                ui.label(format!("Inode: {} · links: {}", e.ino, e.nlink));
                if let Ok(metadata) = std::fs::symlink_metadata(&e.path) {
                    use std::os::unix::fs::PermissionsExt;
                    ui.label(format!(
                        "Permissões: {:04o}",
                        metadata.permissions().mode() & 0o7777
                    ));
                }
                if e.is_symlink {
                    ui.monospace(format!(
                        "Destino: {}",
                        std::fs::read_link(&e.path)
                            .map(|path| path.display().to_string())
                            .unwrap_or_else(|_| "indisponível".into())
                    ));
                }
                if !e.readable {
                    ui.colored_label(Color32::LIGHT_RED, "Metadados inacessíveis");
                }
                if e.is_dir {
                    if ui.button("Abrir pasta").clicked() {
                        let p = e.path.clone();
                        self.navigate_to(p);
                    }
                } else if is_preview_video(&e.path) || is_preview_gif(&e.path) {
                    ui.separator();
                    self.request_quicklook_media(&e.path, ui.ctx());
                    if let Some(texture) = &self.quicklook_media_texture {
                        let frame_size = contain_size(ui.available_size(), texture.size_vec2());
                        ui.centered_and_justified(|ui| {
                            ui.image((texture.id(), frame_size));
                        });
                        ui.weak(if is_preview_video(&e.path) {
                            "Reproduzindo vídeo"
                        } else {
                            "GIF animado"
                        });
                    } else if self.quicklook_media_error.as_deref() == Some(&e.path) {
                        ui.weak("Não foi possível reproduzir a prévia.");
                    } else {
                        ui.spinner();
                        ui.weak("Carregando mídia…");
                    }
                } else if is_preview_image(&e.path) {
                    ui.separator();
                    self.request_thumbnail(&e.path, ui.ctx());
                    if let Some(texture) = self.thumbnails.get(&e.path) {
                        let image_size = contain_size(ui.available_size(), texture.size_vec2());
                        ui.centered_and_justified(|ui| {
                            ui.image((texture.id(), image_size));
                        });
                    } else {
                        ui.spinner();
                        ui.weak("Carregando imagem…");
                    }
                } else if e.size <= 32 * 1024 * 1024 {
                    ui.separator();
                    self.request_quicklook_text(&e.path, ui.ctx());
                    match self
                        .quicklook_text
                        .as_ref()
                        .filter(|(path, _)| path == &e.path)
                        .map(|(_, result)| result)
                    {
                        Some(Ok(preview)) => {
                            let mut text = preview.clone();
                            egui::ScrollArea::both().show(ui, |ui| {
                                ui.add(
                                    egui::TextEdit::multiline(&mut text)
                                        .font(egui::TextStyle::Monospace)
                                        .desired_width(f32::INFINITY)
                                        .interactive(false),
                                );
                            });
                            if e.size > 2 * 1024 * 1024 {
                                ui.weak("Prévia limitada aos primeiros 2 MiB");
                            }
                        }
                        Some(Err(_)) => {
                            ui.weak("Prévia não disponível para este formato.");
                        }
                        None => {
                            ui.spinner();
                            ui.weak("Verificando arquivo…");
                        }
                    }
                }
            }
        }
    }
}
