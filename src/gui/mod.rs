use crate::archive;
use crate::core::{self, human};
use crate::db;
use crate::explorer::{self as fileops, Entry, Explorer, SortKey};
use crate::scanner;
use crate::search::{self, Hit, Query};
use crate::storage::{dupes, tree, treemap};
use crate::transfer;
use egui::{Align2, Color32, FontId, Sense};
use image::{AnimationDecoder, ImageDecoder};
use regex::Regex;
use rusqlite::Connection;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Explorer,
    Search,
    Storage,
}

#[derive(Clone, Copy, PartialEq)]
enum StorageTab {
    Space,
    Dupes,
    Top,
}

#[derive(Clone, Copy, PartialEq)]
enum ViewMode {
    Grid,
    List,
}

#[derive(Clone, Copy)]
enum NavIcon {
    Explorer,
    Search,
    Storage,
    Home,
    Desktop,
    Documents,
    Download,
    Images,
    Music,
    Videos,
    Favorite,
    Trash,
}

#[derive(Clone)]
enum CommandAction {
    Navigate(PathBuf),
    Open(PathBuf),
    QuickLook(PathBuf),
    Run(PathBuf),
    CopyLocation(PathBuf),
    Rename(PathBuf, String),
    RenameSelection(PathBuf),
    TrashSelection(PathBuf),
    CopySelection(PathBuf),
    MoveSelection(PathBuf),
    CopyItems(PathBuf),
    CutItems(PathBuf),
    PasteItems,
    PasteInto(PathBuf),
    Compress(PathBuf, archive::Format),
    Extract(PathBuf),
    CreateFolder,
    ToggleHidden,
    Terminal(PathBuf),
    Editor(PathBuf),
    FolderColor(PathBuf, Option<[u8; 3]>),
}

struct ActiveTransfer {
    receiver: Receiver<transfer::Event>,
    paused: Arc<(Mutex<bool>, Condvar)>,
    cancelled: Arc<AtomicBool>,
    done: u64,
    total: u64,
    started: Instant,
    label: String,
}

struct Settings {
    bg_color: [u8; 3],
    bg_image: String,
    sidebar_image: String,
    sidebar_alpha: f32,
    preview_alpha: f32,
    bg_dim: f32,
    accent: [u8; 3],
    tile_size: f32,
    view: ViewMode,
    show_preview: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            bg_color: [22, 22, 28],
            bg_image: String::new(),
            sidebar_image: String::new(),
            sidebar_alpha: 0.91,
            preview_alpha: 0.88,
            bg_dim: 0.20,
            accent: [74, 124, 255],
            tile_size: 96.0,
            view: ViewMode::Grid,
            show_preview: false,
        }
    }
}

enum Msg {
    Storage {
        tree: tree::Tree,
        top: Vec<(String, u64, u64)>,
        text: String,
    },
    Dupes {
        groups: Vec<dupes::DupGroup>,
        text: String,
    },
    Search(Vec<Hit>),
    Notice(String),
    Err(String),
}

struct SearchState {
    text: String,
    ext: String,
    min_mb: u64,
    max_mb: u64,
    files_only: bool,
    here: bool,
    sort: search::SortOrder,
    note: String,
    hits: Vec<Hit>,
}

pub struct NexusApp {
    db_path: PathBuf,
    conn: Option<Connection>,
    mode: Mode,
    explorer: Explorer,
    favorites: Vec<String>,
    recent_paths: Vec<PathBuf>,
    selected_items: HashSet<PathBuf>,
    file_clipboard: Option<(Vec<PathBuf>, bool)>,
    file_clipboard_external: bool,
    selection_anchor: Option<PathBuf>,
    selection_drag_start: Option<egui::Pos2>,
    selection_drag_base: HashSet<PathBuf>,
    list_drag_selecting: bool,
    folder_colors: HashMap<PathBuf, [u8; 3]>,
    status: String,
    hover: String,
    busy: bool,
    rx: Option<Receiver<Msg>>,

    settings: Settings,
    settings_open: bool,
    applied_accent: Option<[u8; 3]>,
    bg_tex: Option<(String, egui::TextureHandle)>,
    bg_frames: Option<Receiver<(egui::ColorImage, Duration)>>,
    bg_next_frame: Instant,
    bg_error: Option<String>,
    bg_reload: bool,
    sidebar_tex: Option<(String, egui::TextureHandle)>,
    sidebar_frames: Option<Receiver<(egui::ColorImage, Duration)>>,
    sidebar_next_frame: Instant,
    sidebar_error: Option<String>,
    sidebar_reload: bool,
    window_active: bool,
    last_trim: Instant,
    path_edit: bool,
    command_open: bool,
    command_query: String,
    command_index: usize,
    quicklook_open: bool,
    quicklook_media_path: Option<PathBuf>,
    quicklook_media_rx: Option<Receiver<(egui::ColorImage, Duration)>>,
    quicklook_media_texture: Option<egui::TextureHandle>,
    quicklook_media_error: Option<PathBuf>,
    quicklook_media_next: Instant,
    quicklook_text: Option<(PathBuf, Result<String, String>)>,
    quicklook_text_pending: HashSet<PathBuf>,
    quicklook_text_tx: Sender<(PathBuf, Result<String, String>)>,
    quicklook_text_rx: Receiver<(PathBuf, Result<String, String>)>,
    thumbnails: HashMap<PathBuf, egui::TextureHandle>,
    thumbnail_order: VecDeque<PathBuf>,
    thumbnail_pending: HashSet<PathBuf>,
    thumbnail_tx: Sender<(PathBuf, Option<egui::ColorImage>)>,
    thumbnail_rx: Receiver<(PathBuf, Option<egui::ColorImage>)>,

    // explorer
    new_dir_name: String,
    new_dir_open: bool,
    rename_target: Option<(PathBuf, String)>,
    trash_confirm: Option<Vec<PathBuf>>,
    bulk_rename_open: bool,
    bulk_template: String,
    bulk_find: String,
    bulk_replace: String,
    bulk_regex: bool,
    transfer_open: bool,
    transfer_move: bool,
    transfer_destination: String,
    transfer_paths: Vec<PathBuf>,
    transfer: Option<ActiveTransfer>,
    user_locations: Vec<(String, PathBuf, NavIcon)>,

    // seletor de arquivos
    picker_mode: bool,
    picker_multiple: bool,

    // search
    search: SearchState,

    // storage
    scan_root: String,
    storage_tab: StorageTab,
    tree: Option<tree::Tree>,
    cur: usize,
    top: Vec<(String, u64, u64)>,
    min_kb: u64,
    verify_dupes: bool,
    dupes: Option<(Vec<dupes::DupGroup>, String)>,
    dup_confirm: Option<String>,
}

pub fn run(db_path: PathBuf) -> anyhow::Result<()> {
    run_with_options(db_path, false, None, false)
}

pub fn run_picker(
    db_path: PathBuf,
    start: Option<PathBuf>,
    multiple: bool,
) -> anyhow::Result<()> {
    run_with_options(db_path, true, start, multiple)
}

fn run_with_options(
    db_path: PathBuf,
    picker: bool,
    start: Option<PathBuf>,
    multiple: bool,
) -> anyhow::Result<()> {
    let icon_path = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/home/user"))
        .join("Imagens/algumacoisa.jpg");
    let icon = image::open(icon_path).ok().map(|image| {
        let rgba = image.thumbnail(128, 128).to_rgba8();
        let (width, height) = (rgba.width(), rgba.height());
        egui::IconData {
            rgba: rgba.into_raw(),
            width,
            height,
        }
    });
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size(if picker {
            [760.0, 520.0]
        } else {
            [1180.0, 760.0]
        })
        .with_title(if picker {
            "Selecionar arquivo — NEXUS"
        } else {
            "NEXUS"
        })
        .with_app_id(if picker { "nexus-picker" } else { "nexus" });
    if picker {
        viewport = viewport
            .with_always_on_top()
            .with_window_type(egui::viewport::X11WindowType::Dialog)
            .with_resizable(true);
    }
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }
    let opts = eframe::NativeOptions {
        viewport,
        // Mantém o vsync para limitar o FPS. O repaint é pausado quando a janela
        // não está ativa (ver window_active), evitando o swap bloqueado no Wayland.
        vsync: true,
        ..Default::default()
    };
    if picker && std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        std::thread::spawn(|| {
            let script = "function() local wins=hl.get_windows(); for _,w in ipairs(wins) do if w.class=='nexus-picker' and not w.floating then hl.dispatch(hl.dsp.window.float(w)); hl.dispatch(hl.dsp.window.resize({window=w, x=760, y=520, relative=false})); hl.dispatch(hl.dsp.window.center({window=w})); hl.dispatch(hl.dsp.window.pin(w)); end end end";
            for _ in 0..40 {
                std::thread::sleep(Duration::from_millis(50));
                let clients = std::process::Command::new("hyprctl")
                    .args(["clients", "-j"])
                    .output();
                if let Ok(o) = clients {
                    let s = String::from_utf8_lossy(&o.stdout);
                    if s.contains("nexus-picker") {
                        let _ = std::process::Command::new("hyprctl")
                            .args(["dispatch", script])
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .status();
                        break;
                    }
                }
            }
        });
    }
    eframe::run_native(
        "NEXUS",
        opts,
        Box::new(move |cc| Ok(Box::new(NexusApp::new(cc, db_path, picker, start, multiple)))),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

mod dialogs;
mod explorer;
mod icons;
mod logo_mesh;
mod preview;
mod storage;

impl NexusApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        db_path: PathBuf,
        picker: bool,
        start: Option<PathBuf>,
        multiple: bool,
    ) -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
        let conn = db::open(&db_path).ok();
        let favorites = conn
            .as_ref()
            .and_then(|c| db::favorites(c).ok())
            .unwrap_or_default();
        let settings = load_settings(conn.as_ref());
        let folder_colors = conn
            .as_ref()
            .and_then(|conn| db::folder_colors(conn).ok())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(path, value)| parse_rgb(&value).map(|color| (PathBuf::from(path), color)))
            .collect();
        let (thumbnail_tx, thumbnail_rx) = mpsc::channel();
        let (quicklook_text_tx, quicklook_text_rx) = mpsc::channel();
        let user_locations = default_user_locations(&home);
        let mut app = NexusApp {
            db_path,
            conn,
            mode: Mode::Explorer,
            explorer: Explorer::new(start.clone().unwrap_or_else(|| PathBuf::from(&home))),
            favorites,
            recent_paths: Vec::new(),
            selected_items: HashSet::new(),
            file_clipboard: None,
            file_clipboard_external: false,
            selection_anchor: None,
            selection_drag_start: None,
            selection_drag_base: HashSet::new(),
            list_drag_selecting: false,
            folder_colors,
            status: "Pronto".into(),
            hover: String::new(),
            busy: false,
            rx: None,
            settings,
            settings_open: false,
            applied_accent: None,
            bg_tex: None,
            bg_frames: None,
            bg_next_frame: Instant::now(),
            bg_error: None,
            bg_reload: true,
            sidebar_tex: None,
            sidebar_frames: None,
            sidebar_next_frame: Instant::now(),
            sidebar_error: None,
            sidebar_reload: true,
            window_active: true,
            last_trim: Instant::now(),
            path_edit: false,
            command_open: false,
            command_query: String::new(),
            command_index: 0,
            quicklook_open: false,
            quicklook_media_path: None,
            quicklook_media_rx: None,
            quicklook_media_texture: None,
            quicklook_media_error: None,
            quicklook_media_next: Instant::now(),
            quicklook_text: None,
            quicklook_text_pending: HashSet::new(),
            quicklook_text_tx,
            quicklook_text_rx,
            thumbnails: HashMap::new(),
            thumbnail_order: VecDeque::new(),
            thumbnail_pending: HashSet::new(),
            thumbnail_tx,
            thumbnail_rx,
            new_dir_name: String::new(),
            new_dir_open: false,
            rename_target: None,
            trash_confirm: None,
            bulk_rename_open: false,
            bulk_template: "{name}_{n}".into(),
            bulk_find: String::new(),
            bulk_replace: String::new(),
            bulk_regex: false,
            transfer_open: false,
            transfer_move: false,
            transfer_destination: home.clone(),
            transfer_paths: Vec::new(),
            transfer: None,
            user_locations,
            picker_mode: picker,
            picker_multiple: multiple,
            search: SearchState {
                text: String::new(),
                ext: String::new(),
                min_mb: 1,
                max_mb: 0,
                files_only: true,
                here: true,
                sort: search::SortOrder::Size,
                note: String::new(),
                hits: Vec::new(),
            },
            scan_root: home,
            storage_tab: StorageTab::Space,
            tree: None,
            cur: 0,
            top: Vec::new(),
            min_kb: 1,
            verify_dupes: true,
            dupes: None,
            dup_confirm: None,
        };
        app
    }

    fn spawn(
        &mut self,
        ctx: &egui::Context,
        label: &str,
        job: impl FnOnce() -> anyhow::Result<Msg> + Send + 'static,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.status = label.into();
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let m = job().unwrap_or_else(|e| Msg::Err(e.to_string()));
            let _ = tx.send(m);
            ctx.request_repaint();
        });
    }

    fn poll(&mut self, ctx: &egui::Context) {
        for (path, image) in self.thumbnail_rx.try_iter() {
            self.thumbnail_pending.remove(&path);
            if let Some(image) = image {
                let texture = ctx.load_texture(
                    format!("nexus-thumb-{}", path.display()),
                    image,
                    egui::TextureOptions::LINEAR,
                );
                self.thumbnail_order.retain(|cached| cached != &path);
                self.thumbnail_order.push_back(path.clone());
                self.thumbnails.insert(path, texture);
                while self.thumbnail_order.len() > 40 {
                    if let Some(oldest) = self.thumbnail_order.pop_front() {
                        self.thumbnails.remove(&oldest);
                    }
                }
            }
        }
        let msgs: Vec<Msg> = self
            .rx
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for m in msgs {
            self.busy = false;
            self.rx = None;
            trim_memory();
            match m {
                Msg::Storage { tree, top, text } => {
                    self.status = text;
                    self.top = top;
                    self.cur = 0;
                    self.dupes = None;
                    self.tree = Some(tree);
                }
                Msg::Dupes { groups, text } => {
                    self.status = text.clone();
                    self.dupes = Some((groups, text));
                    self.dup_confirm = None;
                }
                Msg::Search(hits) => {
                    self.status = format!("{} resultados no índice", hits.len());
                    self.search.note = self.status.clone();
                    self.search.hits = hits;
                }
                Msg::Notice(text) => {
                    self.status = text;
                    self.explorer.active_tab_mut().refresh();
                }
                Msg::Err(e) => self.status = format!("Erro: {e}"),
            }
        }
        let transfer_events: Vec<transfer::Event> = self
            .transfer
            .as_ref()
            .map(|active| active.receiver.try_iter().collect())
            .unwrap_or_default();
        let mut transfer_finished = false;
        for event in transfer_events {
            match event {
                transfer::Event::Progress { done, total } => {
                    if let Some(active) = &mut self.transfer {
                        active.done = done;
                        active.total = total;
                    }
                }
                transfer::Event::Finished(result) => {
                    match result {
                        Ok(()) => {
                            self.status = "Transferência concluída".into();
                            if self
                                .transfer
                                .as_ref()
                                .is_some_and(|active| active.label == "Movendo")
                            {
                                self.file_clipboard = None;
                            }
                        }
                        Err(error) => self.status = format!("Transferência: {error}"),
                    }
                    transfer_finished = true;
                }
            }
        }
        if transfer_finished {
            self.transfer = None;
            self.explorer.active_tab_mut().refresh();
        }
    }

    fn reveal(&mut self, file: &str) {
        let path = PathBuf::from(file);
        if let Some(parent) = path.parent().map(|p| p.to_path_buf()) {
            self.explorer.active_tab_mut().navigate(parent);
            self.explorer.active_tab_mut().selected = Some(path);
            self.mode = Mode::Explorer;
        }
    }

    fn accent(&self) -> Color32 {
        let [r, g, b] = self.settings.accent;
        Color32::from_rgb(r, g, b)
    }

    fn confirm_picker(&mut self, ctx: &egui::Context) {
        let mut paths: Vec<PathBuf> = if self.selected_items.is_empty() {
            self.explorer
                .active_tab()
                .selected
                .clone()
                .into_iter()
                .collect()
        } else {
            let mut v: Vec<PathBuf> = self.selected_items.iter().cloned().collect();
            v.sort();
            v
        };
        if paths.is_empty() {
            return;
        }
        if !self.picker_multiple && paths.len() > 1 {
            paths.truncate(1);
        }
        for path in &paths {
            println!("{}", path.display());
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn cancel_picker(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn apply_theme(&self, ctx: &egui::Context) {
        let accent = self.accent();
        let mut v = egui::Visuals::dark();
        v.selection.bg_fill =
            Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 90);
        v.selection.stroke = egui::Stroke::new(1.0_f32, accent);
        v.hyperlink_color = accent;
        v.window_fill = Color32::from_rgb(24, 24, 30);
        v.panel_fill = Color32::from_rgba_unmultiplied(18, 18, 23, 218);
        v.extreme_bg_color = Color32::from_rgb(30, 30, 38);
        v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, Color32::from_gray(52));
        v.widgets.inactive.weak_bg_fill = Color32::from_white_alpha(8);
        v.widgets.hovered.weak_bg_fill = Color32::from_white_alpha(18);
        v.widgets.active.weak_bg_fill = Color32::from_white_alpha(30);
        ctx.set_visuals(v);
    }

    fn ensure_background(&mut self, ctx: &egui::Context) {
        if !self.bg_reload {
            return;
        }
        self.bg_reload = false;
        self.bg_error = None;
        let raw = self.settings.bg_image.trim();
        if raw.is_empty() {
            self.bg_tex = None;
            self.bg_frames = None;
            return;
        }
        let path = core::expand_home(raw);
        let is_gif = Path::new(&path)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("gif"));
        if is_gif {
            let (sender, receiver) = mpsc::sync_channel(2);
            let worker_path = path.clone();
            let worker_ctx = ctx.clone();
            std::thread::spawn(move || loop {
                let mut decoder = match std::fs::File::open(&worker_path)
                    .map(BufReader::new)
                    .and_then(|reader| {
                        image::codecs::gif::GifDecoder::new(reader).map_err(std::io::Error::other)
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
                    let first_frame = !had_frame;
                    had_frame = true;
                    let (numerator, denominator) = frame.delay().numer_denom_ms();
                    let delay = Duration::from_millis(
                        (numerator / denominator.max(1)).clamp(16, 10_000) as u64,
                    );
                    let image = frame.into_buffer();
                    let image = image::DynamicImage::ImageRgba8(image)
                        .thumbnail(1920, 1080)
                        .to_rgba8();
                    let size = [image.width() as usize, image.height() as usize];
                    let color = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
                    if sender.send((color, delay)).is_err() {
                        return;
                    }
                    if first_frame {
                        worker_ctx.request_repaint();
                    }
                }
                if !had_frame {
                    return;
                }
            });
            self.bg_tex = None;
            self.bg_frames = Some(receiver);
            return;
        }

        self.bg_frames = None;
        match image::open(&path) {
            Ok(image) => {
                let image = image.thumbnail(2560, 1600).to_rgba8();
                let size = [image.width() as usize, image.height() as usize];
                let color = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
                let texture = ctx.load_texture("nexus-bg", color, egui::TextureOptions::LINEAR);
                self.bg_tex = Some((path, texture));
            }
            Err(error) => {
                self.bg_tex = None;
                self.bg_error = Some(format!("fundo: {error}"));
            }
        }
    }

    fn advance_background(&mut self, ctx: &egui::Context) {
        if self.bg_frames.is_none() {
            return;
        }
        let now = Instant::now();
        if self.bg_tex.is_some() && now < self.bg_next_frame {
            ctx.request_repaint_after(self.bg_next_frame.saturating_duration_since(now));
            return;
        }
        let next = self.bg_frames.as_ref().map(Receiver::try_recv);
        match next {
            Some(Ok((image, delay))) => {
                if let Some((path, texture)) = &mut self.bg_tex {
                    *path = self.settings.bg_image.clone();
                    texture.set(image, egui::TextureOptions::LINEAR);
                } else {
                    let texture =
                        ctx.load_texture("nexus-bg-frame", image, egui::TextureOptions::LINEAR);
                    self.bg_tex = Some((self.settings.bg_image.clone(), texture));
                }
                self.bg_next_frame = Instant::now() + delay;
                ctx.request_repaint_after(delay);
            }
            Some(Err(mpsc::TryRecvError::Empty)) => {
                ctx.request_repaint_after(Duration::from_millis(20));
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.bg_frames = None;
                self.bg_error = Some("fundo: não foi possível decodificar o GIF".into());
            }
            None => {}
        }
    }

    fn ensure_sidebar(&mut self, ctx: &egui::Context) {
        if !self.sidebar_reload {
            return;
        }
        self.sidebar_reload = false;
        self.sidebar_error = None;
        let raw = self.settings.sidebar_image.trim();
        if raw.is_empty() {
            self.sidebar_tex = None;
            self.sidebar_frames = None;
            return;
        }
        let path = core::expand_home(raw);
        let is_gif = Path::new(&path)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("gif"));
        if is_gif {
            let (sender, receiver) = mpsc::sync_channel(2);
            let worker_path = path.clone();
            let worker_ctx = ctx.clone();
            std::thread::spawn(move || loop {
                let mut decoder = match std::fs::File::open(&worker_path)
                    .map(BufReader::new)
                    .and_then(|reader| {
                        image::codecs::gif::GifDecoder::new(reader).map_err(std::io::Error::other)
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
                    let first_frame = !had_frame;
                    had_frame = true;
                    let (numerator, denominator) = frame.delay().numer_denom_ms();
                    let delay = Duration::from_millis(
                        (numerator / denominator.max(1)).clamp(16, 10_000) as u64,
                    );
                    let image = frame.into_buffer();
                    let image = image::DynamicImage::ImageRgba8(image)
                        .thumbnail(480, 360)
                        .to_rgba8();
                    let size = [image.width() as usize, image.height() as usize];
                    let color = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
                    if sender.send((color, delay)).is_err() {
                        return;
                    }
                    if first_frame {
                        worker_ctx.request_repaint();
                    }
                }
                if !had_frame {
                    return;
                }
            });
            self.sidebar_tex = None;
            self.sidebar_frames = Some(receiver);
            return;
        }

        self.sidebar_frames = None;
        match image::open(&path) {
            Ok(image) => {
                let image = image.thumbnail(480, 360).to_rgba8();
                let size = [image.width() as usize, image.height() as usize];
                let color = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
                let texture = ctx.load_texture("nexus-sidebar", color, egui::TextureOptions::LINEAR);
                self.sidebar_tex = Some((path, texture));
            }
            Err(error) => {
                self.sidebar_tex = None;
                self.sidebar_error = Some(format!("mídia: {error}"));
            }
        }
    }

    fn advance_sidebar(&mut self, ctx: &egui::Context) {
        if self.sidebar_frames.is_none() {
            return;
        }
        let now = Instant::now();
        if self.sidebar_tex.is_some() && now < self.sidebar_next_frame {
            ctx.request_repaint_after(self.sidebar_next_frame.saturating_duration_since(now));
            return;
        }
        let next = self.sidebar_frames.as_ref().map(Receiver::try_recv);
        match next {
            Some(Ok((image, delay))) => {
                if let Some((path, texture)) = &mut self.sidebar_tex {
                    *path = self.settings.sidebar_image.clone();
                    texture.set(image, egui::TextureOptions::LINEAR);
                } else {
                    let texture =
                        ctx.load_texture("nexus-sidebar-frame", image, egui::TextureOptions::LINEAR);
                    self.sidebar_tex = Some((self.settings.sidebar_image.clone(), texture));
                }
                self.sidebar_next_frame = Instant::now() + delay;
                ctx.request_repaint_after(delay);
            }
            Some(Err(mpsc::TryRecvError::Empty)) => {
                ctx.request_repaint_after(Duration::from_millis(20));
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.sidebar_frames = None;
                self.sidebar_error = Some("mídia: não foi possível decodificar o GIF".into());
            }
            None => {}
        }
    }

    fn draw_background(&mut self, ctx: &egui::Context) {
        let rect = ctx.screen_rect();
        let painter = ctx.layer_painter(egui::LayerId::background());
        let [r, g, b] = self.settings.bg_color;
        painter.rect_filled(rect, 0.0, Color32::from_rgb(r, g, b));
        if let Some((_, tex)) = &self.bg_tex {
            let img = tex.size_vec2();
            let screen = rect.size();
            let uv = cover_uv(screen.x / screen.y.max(1.0), img.x / img.y.max(1.0));
            painter.image(tex.id(), rect, uv, Color32::WHITE);
        }
        let alpha = (self.settings.bg_dim.clamp(0.0, 0.95) * 255.0) as u8;
        if alpha > 0 {
            painter.rect_filled(rect, 0.0, Color32::from_black_alpha(alpha));
        }
    }

    fn save_settings(&mut self) {
        let Some(conn) = &self.conn else {
            return;
        };
        let s = &self.settings;
        let pairs = [
            (
                "bg_color",
                format!("{},{},{}", s.bg_color[0], s.bg_color[1], s.bg_color[2]),
            ),
            ("bg_image", s.bg_image.clone()),
            ("sidebar_image", s.sidebar_image.clone()),
            ("sidebar_alpha", format!("{:.3}", s.sidebar_alpha)),
            ("preview_alpha", format!("{:.3}", s.preview_alpha)),
            ("bg_dim", format!("{:.3}", s.bg_dim)),
            (
                "accent",
                format!("{},{},{}", s.accent[0], s.accent[1], s.accent[2]),
            ),
            ("tile_size", format!("{:.1}", s.tile_size)),
            (
                "view",
                match s.view {
                    ViewMode::Grid => "grid",
                    ViewMode::List => "list",
                }
                .to_string(),
            ),
            (
                "show_preview",
                if s.show_preview { "1" } else { "0" }.to_string(),
            ),
        ];
        if let Err(e) = db::set_settings(conn, &pairs) {
            self.status = format!("Erro ao salvar aparência: {e}");
        }
    }

    // ---------- sidebar ----------

    // ---------- explorer ----------

    // ---------- pesquisa ----------

    // ---------- armazenamento ----------

    // ---------- janelas ----------

}

impl eframe::App for NexusApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
        self.poll_quicklook(ctx);
        self.handle_shortcuts(ctx);
        let window_active = ctx.input(|input| {
            let viewport = input.viewport();
            viewport.focused.unwrap_or(true) && !viewport.minimized.unwrap_or(false)
        });
        self.window_active = window_active;
        if !self.busy && self.last_trim.elapsed() >= Duration::from_secs(30) {
            trim_memory();
            self.last_trim = Instant::now();
        }
        if self.transfer.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        if self.applied_accent != Some(self.settings.accent) {
            self.apply_theme(ctx);
            self.applied_accent = Some(self.settings.accent);
        }
        self.ensure_background(ctx);
        if window_active {
            self.advance_background(ctx);
        }
        self.ensure_sidebar(ctx);
        if window_active {
            self.advance_sidebar(ctx);
        }
        self.draw_background(ctx);

        let sidebar_alpha = (self.settings.sidebar_alpha * 255.0).round() as u8;
        egui::SidePanel::left("nav")
            .resizable(false)
            .exact_width(205.0)
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgba_unmultiplied(14, 14, 19, sidebar_alpha))
                    .inner_margin(egui::Margin::same(10.0)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| self.ui_sidebar(ui));
            });

        egui::TopBottomPanel::bottom("status")
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgba_unmultiplied(14, 14, 19, 232))
                    .inner_margin(egui::Margin::symmetric(8.0, 4.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if self.busy {
                        ui.spinner();
                    }
                    ui.label(self.status.as_str());
                    ui.separator();
                    ui.label(self.hover.as_str());
                    if self.picker_mode {
                        ui.separator();
                        if ui.button("Cancelar").clicked() {
                            self.cancel_picker(ctx);
                        }
                        let has_sel = !self.selected_items.is_empty();
                        if ui
                            .add_enabled(has_sel, egui::Button::new("Selecionar"))
                            .clicked()
                        {
                            self.confirm_picker(ctx);
                        }
                    }
                    if let Some(active) = &self.transfer {
                        ui.separator();
                        ui.label(&active.label);
                        let fraction = if active.total == 0 {
                            0.0
                        } else {
                            (active.done as f32 / active.total as f32).clamp(0.0, 1.0)
                        };
                        ui.add(
                            egui::ProgressBar::new(fraction)
                                .desired_width(130.0)
                                .show_percentage(),
                        );
                        let speed = active.done as f64
                            / active.started.elapsed().as_secs_f64().max(0.1)
                            / 1_000_000.0;
                        ui.weak(format!("{} · {speed:.1} MB/s", human(active.done)));
                        let (lock, wake) = &*active.paused;
                        let paused = lock.lock().map(|state| *state).unwrap_or(false);
                        if ui
                            .small_button(if paused { "▶" } else { "Ⅱ" })
                            .on_hover_text(if paused { "Retomar" } else { "Pausar" })
                            .clicked()
                        {
                            if let Ok(mut state) = lock.lock() {
                                *state = !*state;
                                wake.notify_all();
                            }
                        }
                        if ui
                            .small_button("×")
                            .on_hover_text("Cancelar transferência")
                            .clicked()
                        {
                            active.cancelled.store(true, Ordering::Relaxed);
                            wake.notify_all();
                        }
                    }
                });
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().inner_margin(egui::Margin::symmetric(12.0, 8.0)))
            .show(ctx, |ui| match self.mode {
                Mode::Explorer => self.ui_explorer(ui),
                Mode::Search => self.ui_search(ui),
                Mode::Storage => self.ui_storage(ui),
            });

        self.ui_settings_window(ctx);
        self.dialogs(ctx);
        self.ui_transfer_dialog(ctx);
        self.ui_quicklook(ctx);
        self.ui_command_palette(ctx);
    }
}

fn default_user_locations(home: &str) -> Vec<(String, PathBuf, NavIcon)> {
    let locations = [
        ("Área de Trabalho", "DESKTOP", "Desktop", NavIcon::Desktop),
        ("Documentos", "DOCUMENTS", "Documents", NavIcon::Documents),
        ("Downloads", "DOWNLOAD", "Downloads", NavIcon::Download),
        ("Músicas", "MUSIC", "Music", NavIcon::Music),
        ("Imagens", "PICTURES", "Pictures", NavIcon::Images),
        ("Vídeos", "VIDEOS", "Videos", NavIcon::Videos),
        ("Público", "PUBLIC", "Public", NavIcon::Documents),
        ("Modelos", "TEMPLATES", "Templates", NavIcon::Documents),
    ];
    let mut seen = HashSet::new();
    let home_path = PathBuf::from(home);
    locations
        .into_iter()
        .filter_map(|(label, key, fallback, icon)| {
            let configured = Command::new("xdg-user-dir")
                .arg(key)
                .output()
                .ok()
                .filter(|output| output.status.success())
                .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
                .filter(|path| !path.is_empty() && path != home);
            let path = configured
                .map(PathBuf::from)
                .unwrap_or_else(|| home_path.join(fallback));
            if path.is_dir() && seen.insert(path.clone()) {
                Some((label.to_string(), path, icon))
            } else {
                None
            }
        })
        .collect()
}

fn entry_tooltip(entry: &Entry) -> String {
    let kind = if entry.is_dir {
        "Pasta".to_string()
    } else if entry.is_symlink {
        "Link simbólico".to_string()
    } else {
        entry
            .path
            .extension()
            .map(|extension| format!("Arquivo {}", extension.to_string_lossy().to_uppercase()))
            .unwrap_or_else(|| "Arquivo".to_string())
    };
    let modified = entry
        .modified
        .map(|time| core::iso_utc(core::to_secs(time)))
        .unwrap_or_else(|| "Desconhecido".into());
    format!(
        "{}\n{}\nTipo: {}\nTamanho: {}\nEm disco: {}\nModificado: {}{}",
        entry.name,
        entry.path.display(),
        kind,
        human(entry.size),
        human(entry.alloc),
        modified,
        if entry.is_symlink {
            format!(
                "\nDestino: {}",
                std::fs::read_link(&entry.path)
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| "indisponível".into())
            )
        } else {
            String::new()
        }
    )
}

fn open_terminal(path: &Path) -> io::Result<()> {
    let path = path.to_string_lossy().into_owned();
    let candidates = [
        ("x-terminal-emulator", Vec::new()),
        ("konsole", vec!["--workdir".into(), path.clone()]),
        (
            "gnome-terminal",
            vec![format!("--working-directory={path}")],
        ),
        (
            "xfce4-terminal",
            vec![format!("--working-directory={path}")],
        ),
        ("kitty", vec!["--directory".into(), path.clone()]),
        ("foot", vec!["--working-directory".into(), path.clone()]),
    ];
    let mut last_error = None;
    for (program, args) in candidates {
        let mut command = Command::new(program);
        command.args(args).current_dir(&path);
        match core::spawn_and_reap(command) {
            Ok(()) => return Ok(()),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| io::Error::other("nenhum terminal disponível")))
}

fn open_editor(path: &Path) -> io::Result<()> {
    let mut last_error = None;
    for program in ["code", "codium", "code-oss"] {
        let mut command = Command::new(program);
        command.arg(path);
        match core::spawn_and_reap(command) {
            Ok(()) => return Ok(()),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| io::Error::other("nenhum editor compatível encontrado")))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    if path.is_dir() {
        return false;
    }
    let mode = std::fs::metadata(path)
        .map(|metadata| metadata.permissions().mode())
        .unwrap_or(0);
    if mode & 0o111 != 0 {
        return true;
    }
    path.extension()
        .map(|ext| {
            matches!(
                ext.to_string_lossy().to_ascii_lowercase().as_str(),
                "sh" | "bash" | "run" | "appimage"
            )
        })
        .unwrap_or(false)
}

fn run_executable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mode = std::fs::metadata(path)?.permissions().mode();
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    if mode & 0o111 != 0 {
        let mut command = Command::new(path);
        command.current_dir(parent);
        return core::spawn_and_reap(command);
    }
    let extension = path
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if extension == "sh" || extension == "bash" {
        let mut command = Command::new("sh");
        command.arg(path).current_dir(parent);
        return core::spawn_and_reap(command);
    }
    Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        "o arquivo não tem permissão de execução",
    ))
}

fn surrender_explorer_text_focus(ctx: &egui::Context) {
    ctx.memory_mut(|memory| {
        memory.surrender_focus(egui::Id::new("nexus-explorer-search"));
        memory.surrender_focus(egui::Id::new("nexus-path-input"));
    });
}

fn count_suffix(count: usize) -> String {
    if count > 1 {
        format!(" ({count})")
    } else {
        String::new()
    }
}

fn set_system_clipboard(mime: &str, content: &str) -> io::Result<()> {
    let mut options = wl_clipboard_rs::copy::Options::new();
    options.clipboard(wl_clipboard_rs::copy::ClipboardType::Regular);
    if let Err(error) = wl_clipboard_rs::copy::copy(
        options,
        wl_clipboard_rs::copy::Source::Bytes(content.as_bytes().into()),
        wl_clipboard_rs::copy::MimeType::Specific(mime.to_string()),
    ) {
        return set_xclip(mime, content).map_err(|xclip_error| {
            io::Error::other(format!("Wayland: {error}; X11: {xclip_error}"))
        });
    }
    Ok(())
}

fn set_system_file_clipboard(paths: &[PathBuf], cut: bool) -> io::Result<()> {
    let uri_list = format!(
        "{}\r\n",
        paths
            .iter()
            .map(|path| file_uri(path))
            .collect::<Vec<_>>()
            .join("\r\n")
    );
    let gnome_list = format!(
        "{}\n{}\n",
        if cut { "cut" } else { "copy" },
        paths
            .iter()
            .map(|path| file_uri(path))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let kde_cut = if cut { "1" } else { "0" };
    let sources = vec![
        wl_clipboard_rs::copy::MimeSource {
            source: wl_clipboard_rs::copy::Source::Bytes(uri_list.into_bytes().into()),
            mime_type: wl_clipboard_rs::copy::MimeType::Specific("text/uri-list".into()),
        },
        wl_clipboard_rs::copy::MimeSource {
            source: wl_clipboard_rs::copy::Source::Bytes(gnome_list.into_bytes().into()),
            mime_type: wl_clipboard_rs::copy::MimeType::Specific(
                "x-special/gnome-copied-files".into(),
            ),
        },
        wl_clipboard_rs::copy::MimeSource {
            source: wl_clipboard_rs::copy::Source::Bytes(kde_cut.as_bytes().into()),
            mime_type: wl_clipboard_rs::copy::MimeType::Specific(
                "application/x-kde-cutselection".into(),
            ),
        },
    ];
    let mut options = wl_clipboard_rs::copy::Options::new();
    options.clipboard(wl_clipboard_rs::copy::ClipboardType::Regular);
    match wl_clipboard_rs::copy::copy_multi(options, sources) {
        Ok(()) => Ok(()),
        Err(wayland_error) => {
            let (mime, data) = if cut {
                ("x-special/gnome-copied-files", gnome_list_for(paths, true))
            } else {
                (
                    "text/uri-list",
                    format!(
                        "{}\r\n",
                        paths
                            .iter()
                            .map(|path| file_uri(path))
                            .collect::<Vec<_>>()
                            .join("\r\n")
                    ),
                )
            };
            set_xclip(mime, &data).map_err(|xclip_error| {
                io::Error::other(format!("Wayland: {wayland_error}; X11: {xclip_error}"))
            })
        }
    }
}

fn gnome_list_for(paths: &[PathBuf], cut: bool) -> String {
    format!(
        "{}\n{}\n",
        if cut { "cut" } else { "copy" },
        paths
            .iter()
            .map(|path| file_uri(path))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

fn set_xclip(mime: &str, content: &str) -> io::Result<()> {
    let mut last_error = None;
    for (program, args) in [(
        "xclip",
        vec!["-selection", "clipboard", "-in", "-target", mime],
    )] {
        let mut child = match Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                last_error = Some(error);
                continue;
            }
        };
        let result = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("clipboard sem entrada de dados"))
            .and_then(|mut stdin| stdin.write_all(content.as_bytes()));
        match result {
            Ok(()) => return Ok(()),
            Err(error) => {
                let _ = child.kill();
                last_error = Some(error);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| io::Error::other("wl-copy/xclip indisponível")))
}

fn read_system_file_clipboard() -> Result<Option<(Vec<PathBuf>, bool)>, String> {
    use wl_clipboard_rs::paste::{ClipboardType, Seat};

    let wayland = (|| {
        let mime_types =
            wl_clipboard_rs::paste::get_mime_types(ClipboardType::Regular, Seat::Unspecified)
                .map_err(|error| error.to_string())?;
        let mime_types = mime_types.into_iter().collect::<Vec<_>>();
        read_file_clipboard_from_mime_types(&mime_types, read_wayland_mime)
    })();
    match wayland {
        Ok(Some(clipboard)) => Ok(Some(clipboard)),
        Ok(None) => Ok(read_xclip_file_clipboard().ok().flatten()),
        Err(wayland_error) => read_xclip_file_clipboard()
            .map_err(|xclip_error| format!("Wayland: {wayland_error}; X11: {xclip_error}")),
    }
}

fn read_file_clipboard_from_mime_types(
    mime_types: &[String],
    mut read_mime: impl FnMut(&str) -> Result<String, String>,
) -> Result<Option<(Vec<PathBuf>, bool)>, String> {
    if mime_types
        .iter()
        .any(|mime| mime == "x-special/gnome-copied-files")
    {
        if let Ok(content) = read_mime("x-special/gnome-copied-files") {
            if let Some(clipboard) = parse_gnome_file_clipboard(&content) {
                return Ok(Some(clipboard));
            }
        }
    }

    if mime_types.iter().any(|mime| mime == "text/uri-list") {
        let content = read_mime("text/uri-list")?;
        let paths = decode_uri_lines(content.lines());
        if !paths.is_empty() {
            let cut = if mime_types
                .iter()
                .any(|mime| mime == "application/x-kde-cutselection")
            {
                read_mime("application/x-kde-cutselection")
                    .map(|value| value.trim() == "1")
                    .unwrap_or(false)
            } else {
                false
            };
            return Ok(Some((paths, cut)));
        }
    }
    Ok(None)
}

fn read_xclip_file_clipboard() -> Result<Option<(Vec<PathBuf>, bool)>, String> {
    let targets = read_xclip_mime("TARGETS")?;
    let mime_types = targets
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();
    read_file_clipboard_from_mime_types(&mime_types, read_xclip_mime)
}

fn read_xclip_mime(mime: &str) -> Result<String, String> {
    let output = Command::new("xclip")
        .args(["-selection", "clipboard", "-out", "-target", mime])
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn parse_gnome_file_clipboard(content: &str) -> Option<(Vec<PathBuf>, bool)> {
    let mut lines = content.lines();
    let operation = lines.next()?.trim();
    if !operation.eq_ignore_ascii_case("copy") && !operation.eq_ignore_ascii_case("cut") {
        return None;
    }
    let paths = decode_uri_lines(lines);
    (!paths.is_empty()).then_some((paths, operation.eq_ignore_ascii_case("cut")))
}

fn read_wayland_mime(mime: &str) -> Result<String, String> {
    use wl_clipboard_rs::paste::{ClipboardType, MimeType, Seat};

    let (mut reader, _) = wl_clipboard_rs::paste::get_contents(
        ClipboardType::Regular,
        Seat::Unspecified,
        MimeType::Specific(mime),
    )
    .map_err(|error| error.to_string())?;
    let mut content = String::new();
    reader
        .read_to_string(&mut content)
        .map_err(|error| error.to_string())?;
    Ok(content)
}

fn decode_uri_lines<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<PathBuf> {
    use std::os::unix::ffi::OsStringExt;

    lines
        .filter_map(|line| {
            let uri = line.trim();
            if uri.is_empty() || uri.starts_with('#') || !uri.starts_with("file://") {
                return None;
            }
            let local_path = uri.strip_prefix("file://")?;
            let local_path = if let Some(path) = local_path.strip_prefix("localhost/") {
                format!("/{path}")
            } else if local_path.starts_with('/') {
                local_path.to_string()
            } else {
                return None;
            };
            let bytes = local_path.as_bytes();
            let mut decoded = Vec::with_capacity(bytes.len());
            let mut index = 0;
            while index < bytes.len() {
                if bytes[index] == b'%' && index + 2 < bytes.len() {
                    let high = (bytes[index + 1] as char).to_digit(16)? as u8;
                    let low = (bytes[index + 2] as char).to_digit(16)? as u8;
                    decoded.push((high << 4) | low);
                    index += 3;
                } else {
                    decoded.push(bytes[index]);
                    index += 1;
                }
            }
            Some(PathBuf::from(std::ffi::OsString::from_vec(decoded)))
        })
        .collect()
}

fn file_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;

    let absolute = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut uri = String::from("file://");
    for byte in absolute.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(*byte, b'/' | b'-' | b'_' | b'.' | b'~') {
            uri.push(*byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

fn load_settings(conn: Option<&Connection>) -> Settings {
    let mut s = Settings::default();
    let Some(conn) = conn else {
        return s;
    };
    if let Some(v) = db::get_setting(conn, "bg_color") {
        if let Some(c) = parse_rgb(&v) {
            s.bg_color = c;
        }
    }
    if let Some(v) = db::get_setting(conn, "bg_image") {
        s.bg_image = v;
    }
    if let Some(v) = db::get_setting(conn, "sidebar_image") {
        s.sidebar_image = v;
    }
    if let Some(v) = db::get_setting(conn, "sidebar_alpha") {
        if let Ok(v) = v.parse::<f32>() {
            s.sidebar_alpha = v.clamp(0.1, 1.0);
        }
    }
    if let Some(v) = db::get_setting(conn, "preview_alpha") {
        if let Ok(v) = v.parse::<f32>() {
            s.preview_alpha = v.clamp(0.1, 1.0);
        }
    }
    if let Some(v) = db::get_setting(conn, "bg_dim") {
        if let Ok(v) = v.parse::<f32>() {
            s.bg_dim = v.clamp(0.0, 0.95);
        }
    }
    if let Some(v) = db::get_setting(conn, "accent") {
        if let Some(c) = parse_rgb(&v) {
            s.accent = c;
        }
    }
    if let Some(v) = db::get_setting(conn, "tile_size") {
        if let Ok(v) = v.parse::<f32>() {
            s.tile_size = v.clamp(48.0, 220.0);
        }
    }
    if let Some(v) = db::get_setting(conn, "view") {
        s.view = if v == "list" {
            ViewMode::List
        } else {
            ViewMode::Grid
        };
    }
    if let Some(v) = db::get_setting(conn, "show_preview") {
        s.show_preview = v == "1";
    }
    s
}

fn parse_rgb(s: &str) -> Option<[u8; 3]> {
    let mut it = s.split(',').map(|x| x.trim().parse::<u8>().ok());
    Some([it.next()??, it.next()??, it.next()??])
}

fn cover_uv(screen_aspect: f32, image_aspect: f32) -> egui::Rect {
    use egui::{pos2, Rect};
    if image_aspect > screen_aspect {
        let w = screen_aspect / image_aspect;
        Rect::from_min_max(pos2((1.0 - w) / 2.0, 0.0), pos2((1.0 + w) / 2.0, 1.0))
    } else {
        let h = image_aspect / screen_aspect;
        Rect::from_min_max(pos2(0.0, (1.0 - h) / 2.0), pos2(1.0, (1.0 + h) / 2.0))
    }
}

fn text_to_fit(text: &str, max_width: f32, font: FontId, painter: &egui::Painter) -> String {
    if max_width <= 0.0 {
        return String::new();
    }
    if painter
        .layout_no_wrap(text.to_string(), font.clone(), Color32::WHITE)
        .size()
        .x
        <= max_width
    {
        return text.to_string();
    }
    let mut max_chars = text.chars().count().saturating_sub(1);
    while max_chars > 0 {
        let candidate = short_treemap_name(text, max_chars);
        if painter
            .layout_no_wrap(candidate.clone(), font.clone(), Color32::WHITE)
            .size()
            .x
            <= max_width
        {
            return candidate;
        }
        max_chars -= 1;
    }
    String::new()
}

fn sidebar_item(
    ui: &mut egui::Ui,
    selected: bool,
    label: &str,
    icon: NavIcon,
    accent: Color32,
) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 25.0), Sense::click());
    if selected || response.hovered() {
        let fill = if selected {
            Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 64)
        } else {
            Color32::from_white_alpha(12)
        };
        ui.painter().rect_filled(rect, 4.0, fill);
    }
    let color = nav_icon_color(icon, selected, accent);
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 15.0, rect.center().y),
        egui::vec2(16.0, 16.0),
    );
    if selected || response.hovered() {
        ui.painter().circle_filled(
            icon_rect.center(),
            9.5,
            Color32::from_rgba_unmultiplied(
                color.r(),
                color.g(),
                color.b(),
                if selected { 44 } else { 24 },
            ),
        );
    }
    draw_nav_icon(ui.painter(), icon_rect, icon, color);
    ui.painter().text(
        egui::pos2(rect.left() + 30.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(13.0),
        if selected {
            Color32::WHITE
        } else {
            Color32::from_gray(210)
        },
    );
    response.clicked()
}

fn nav_icon_color(icon: NavIcon, selected: bool, accent: Color32) -> Color32 {
    let base = match icon {
        NavIcon::Explorer => Color32::from_rgb(73, 157, 238),
        NavIcon::Search => Color32::from_rgb(62, 195, 205),
        NavIcon::Storage => Color32::from_rgb(98, 190, 130),
        NavIcon::Home => Color32::from_rgb(236, 164, 83),
        NavIcon::Desktop => Color32::from_rgb(91, 156, 239),
        NavIcon::Documents => Color32::from_rgb(232, 183, 76),
        NavIcon::Download => Color32::from_rgb(77, 194, 146),
        NavIcon::Images => Color32::from_rgb(224, 112, 165),
        NavIcon::Music => Color32::from_rgb(166, 132, 235),
        NavIcon::Videos => Color32::from_rgb(232, 112, 83),
        NavIcon::Favorite => Color32::from_rgb(245, 196, 74),
        NavIcon::Trash => Color32::from_rgb(220, 90, 90),
    };
    if selected {
        Color32::from_rgb(
            ((u16::from(base.r()) + u16::from(accent.r())) / 2) as u8,
            ((u16::from(base.g()) + u16::from(accent.g())) / 2) as u8,
            ((u16::from(base.b()) + u16::from(accent.b())) / 2) as u8,
        )
    } else {
        base
    }
}

fn draw_nav_icon(painter: &egui::Painter, rect: egui::Rect, icon: NavIcon, color: Color32) {
    let stroke = egui::Stroke::new(1.8_f32, color);
    let center = rect.center();
    let left = rect.left() + 2.0;
    let right = rect.right() - 2.0;
    let top = rect.top() + 2.0;
    let bottom = rect.bottom() - 2.0;
    match icon {
        NavIcon::Explorer => {
            let tab = egui::Rect::from_min_max(
                egui::pos2(left, top + 1.0),
                egui::pos2(center.x, top + 5.0),
            );
            let body =
                egui::Rect::from_min_max(egui::pos2(left, top + 4.0), egui::pos2(right, bottom));
            painter.rect_filled(tab, 1.5, color);
            painter.rect_filled(body, 2.0, color);
        }
        NavIcon::Search => {
            painter.circle_stroke(center - egui::vec2(1.5, 1.5), 4.5, stroke);
            painter.line_segment(
                [
                    center + egui::vec2(2.0, 2.0),
                    rect.right_bottom() - egui::vec2(2.0, 2.0),
                ],
                stroke,
            );
        }
        NavIcon::Storage => {
            let drive = egui::Rect::from_min_max(
                egui::pos2(left, top + 2.0),
                egui::pos2(right, bottom - 1.0),
            );
            painter.rect_stroke(drive, 2.0, stroke);
            painter.line_segment(
                [
                    egui::pos2(left + 1.0, center.y + 1.0),
                    egui::pos2(right - 1.0, center.y + 1.0),
                ],
                stroke,
            );
            painter.circle_filled(egui::pos2(right - 3.0, bottom - 2.0), 1.0, color);
        }
        NavIcon::Home => {
            painter.add(egui::Shape::line(
                vec![
                    egui::pos2(left, center.y),
                    egui::pos2(center.x, top),
                    egui::pos2(right, center.y),
                ],
                stroke,
            ));
            painter.rect_stroke(
                egui::Rect::from_min_max(
                    egui::pos2(left + 2.0, center.y),
                    egui::pos2(right - 2.0, bottom),
                ),
                1.0,
                stroke,
            );
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(center.x - 1.5, bottom - 5.0),
                    egui::pos2(center.x + 1.5, bottom),
                ),
                0.5,
                color,
            );
        }
        NavIcon::Desktop => {
            painter.rect_stroke(
                egui::Rect::from_min_max(egui::pos2(left, top), egui::pos2(right, center.y + 2.0)),
                1.5,
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x, center.y + 2.0),
                    egui::pos2(center.x, bottom - 1.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x - 4.0, bottom - 1.0),
                    egui::pos2(center.x + 4.0, bottom - 1.0),
                ],
                stroke,
            );
        }
        NavIcon::Documents => {
            let page = egui::Rect::from_min_max(
                egui::pos2(left + 1.0, top),
                egui::pos2(right - 1.0, bottom),
            );
            painter.rect_stroke(page, 1.0, stroke);
            for offset in [4.0, 7.0, 10.0] {
                painter.line_segment(
                    [
                        egui::pos2(left + 4.0, top + offset),
                        egui::pos2(right - 3.0, top + offset),
                    ],
                    stroke,
                );
            }
        }
        NavIcon::Download => {
            painter.line_segment(
                [
                    egui::pos2(center.x, top),
                    egui::pos2(center.x, center.y + 2.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x - 4.0, center.y - 1.0),
                    egui::pos2(center.x, center.y + 3.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x + 4.0, center.y - 1.0),
                    egui::pos2(center.x, center.y + 3.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(left, bottom - 2.0),
                    egui::pos2(right, bottom - 2.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(left, bottom - 2.0),
                    egui::pos2(left, center.y + 4.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(right, bottom - 2.0),
                    egui::pos2(right, center.y + 4.0),
                ],
                stroke,
            );
        }
        NavIcon::Images => {
            let frame = egui::Rect::from_min_max(
                egui::pos2(left, top + 1.0),
                egui::pos2(right, bottom - 1.0),
            );
            painter.rect_stroke(frame, 1.0, stroke);
            painter.circle_filled(egui::pos2(right - 3.0, top + 4.0), 1.4, color);
            painter.add(egui::Shape::line(
                vec![
                    egui::pos2(left + 1.0, bottom - 3.0),
                    egui::pos2(center.x - 2.0, center.y),
                    egui::pos2(center.x + 1.0, center.y + 2.0),
                    egui::pos2(right - 1.0, center.y - 1.0),
                ],
                stroke,
            ));
        }
        NavIcon::Music => {
            painter.line_segment(
                [
                    egui::pos2(center.x + 3.0, top),
                    egui::pos2(center.x + 3.0, bottom - 3.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x + 3.0, top),
                    egui::pos2(right, top + 2.0),
                ],
                stroke,
            );
            painter.circle_filled(egui::pos2(center.x - 1.0, bottom - 2.0), 2.4, color);
            painter.circle_filled(egui::pos2(center.x + 2.0, bottom - 3.0), 2.4, color);
        }
        NavIcon::Videos => {
            painter.rect_stroke(
                egui::Rect::from_min_max(
                    egui::pos2(left, top + 2.0),
                    egui::pos2(right, bottom - 2.0),
                ),
                2.0,
                stroke,
            );
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(center.x - 2.0, center.y - 3.0),
                    egui::pos2(center.x + 3.0, center.y),
                    egui::pos2(center.x - 2.0, center.y + 3.0),
                ],
                color,
                egui::Stroke::NONE,
            ));
        }
        NavIcon::Favorite => {
            let points = (0..10)
                .map(|index| {
                    let angle =
                        -std::f32::consts::FRAC_PI_2 + index as f32 * std::f32::consts::PI / 5.0;
                    let radius = if index % 2 == 0 { 7.0 } else { 3.2 };
                    center + egui::vec2(angle.cos() * radius, angle.sin() * radius)
                })
                .collect();
            painter.add(egui::Shape::convex_polygon(
                points,
                color,
                egui::Stroke::NONE,
            ));
        }
        NavIcon::Trash => {
            let body = egui::Rect::from_min_max(
                egui::pos2(left + 2.0, top + 4.0),
                egui::pos2(right - 2.0, bottom - 2.0),
            );
            painter.rect_filled(body, 2.0, color);
            let rim = egui::Rect::from_min_max(
                egui::pos2(left, top + 2.0),
                egui::pos2(right, top + 5.0),
            );
            painter.rect_filled(rim, 1.5, color);
            painter.line_segment(
                [
                    egui::pos2(center.x - 2.0, top),
                    egui::pos2(center.x + 2.0, top),
                ],
                stroke,
            );
        }
    }
}

fn trim_memory() {
    extern "C" {
        fn malloc_trim(pad: i32) -> i32;
    }
    unsafe {
        let _ = malloc_trim(0);
    }
}

fn contain_size(bounds: egui::Vec2, image: egui::Vec2) -> egui::Vec2 {
    if image.x <= 0.0 || image.y <= 0.0 {
        return bounds;
    }
    let scale = (bounds.x / image.x).min(bounds.y / image.y);
    image * scale
}

fn contain_rect(bounds: egui::Rect, image: egui::Vec2) -> egui::Rect {
    egui::Rect::from_center_size(bounds.center(), contain_size(bounds.size(), image))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileIconKind {
    Image,
    Video,
    Audio,
    Archive,
    ZipArchive,
    Code,
    Python,
    C,
    Cpp,
    Rust,
    Javascript,
    Typescript,
    Jvm,
    Go,
    Shell,
    Web,
    Php,
    Ruby,
    Swift,
    Dart,
    RLang,
    Haskell,
    Elixir,
    Lua,
    Pdf,
    Document,
    Spreadsheet,
    Presentation,
    Font,
    Application,
    Package,
    Android,
    Disk,
    Database,
    Ebook,
    Notebook,
    Model3d,
    Generic,
}

fn draw_ellipse_outline(
    painter: &egui::Painter,
    center: egui::Pos2,
    radii: egui::Vec2,
    stroke: egui::Stroke,
) {
    let points = (0..=32)
        .map(|index| {
            let angle = std::f32::consts::TAU * index as f32 / 32.0;
            egui::pos2(
                center.x + angle.cos() * radii.x,
                center.y + angle.sin() * radii.y,
            )
        })
        .collect();
    painter.add(egui::Shape::line(points, stroke));
}

fn mix_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    Color32::from_rgb(
        (a.r() as f32 + (b.r() as f32 - a.r() as f32) * t) as u8,
        (a.g() as f32 + (b.g() as f32 - a.g() as f32) * t) as u8,
        (a.b() as f32 + (b.b() as f32 - a.b() as f32) * t) as u8,
    )
}

fn file_icon_kind(extension: &str) -> FileIconKind {
    match extension {
        "png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif" | "svg" | "heic" | "tif" | "tiff"
        | "avif" | "raw" | "arw" | "cr2" | "nef" | "jxl" | "jp2" | "j2k" | "exr" | "hdr"
        | "icns" | "psd" | "xcf" | "kra" | "ora" | "ai" | "eps" | "indd" | "afdesign"
        | "afphoto" | "afpub" | "fig" | "sketch" | "xd" => FileIconKind::Image,
        "mp4" | "mkv" | "mov" | "avi" | "webm" | "m4v" | "mpeg" | "mpg" | "wmv" | "flv"
        | "ogv" | "3gp" | "3g2" | "m2ts" | "vob" | "mxf" | "asf" | "rm" | "rmvb" | "amv" => {
            FileIconKind::Video
        }
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "opus" | "wma" | "aiff" | "mid"
        | "midi" | "alac" | "ape" | "amr" | "ac3" | "dts" | "caf" | "dsf" | "dff" | "wv"
        | "mka" => FileIconKind::Audio,
        "zip" => FileIconKind::ZipArchive,
        "7z" | "rar" | "tar" | "gz" | "xz" | "bz2" | "zst" | "tgz" | "lz" | "lz4" | "cab" => {
            FileIconKind::Archive
        }
        "lua" => FileIconKind::Lua,
        "py" | "pyi" | "pyw" => FileIconKind::Python,
        "c" | "h" => FileIconKind::C,
        "cpp" | "cxx" | "cc" | "hpp" | "hxx" | "hh" | "inl" => FileIconKind::Cpp,
        "rs" => FileIconKind::Rust,
        "js" | "mjs" | "cjs" | "jsx" => FileIconKind::Javascript,
        "ts" | "tsx" | "mts" | "cts" => FileIconKind::Typescript,
        "java" | "kt" | "kts" | "scala" | "groovy" => FileIconKind::Jvm,
        "go" => FileIconKind::Go,
        "sh" | "bash" | "zsh" | "fish" => FileIconKind::Shell,
        "html" | "htm" | "css" | "scss" | "sass" | "less" | "vue" | "svelte" | "astro" => {
            FileIconKind::Web
        }
        "php" => FileIconKind::Php,
        "rb" | "erb" => FileIconKind::Ruby,
        "swift" => FileIconKind::Swift,
        "dart" => FileIconKind::Dart,
        "r" => FileIconKind::RLang,
        "hs" | "lhs" => FileIconKind::Haskell,
        "ex" | "exs" => FileIconKind::Elixir,
        "pl" | "erl" | "clj" | "cljs" | "elm" | "fs" | "fsx" | "json" | "toml" | "yaml"
        | "yml" | "xml" | "sql" | "makefile" | "dockerfile" | "gradle" | "cmake" | "diff"
        | "patch" | "asm" | "s" | "zig" | "nix" | "v" | "sv" | "svh" | "vhd" | "vhdl"
        | "wat" | "wasm" | "graphql" | "gql" | "proto" | "thrift" | "prisma" | "tf"
        | "tfvars" | "hcl" | "sol" | "move" | "purs" | "cr" | "jl" | "m" | "mm" | "raku" => {
            FileIconKind::Code
        }
        "pdf" => FileIconKind::Pdf,
        "txt" | "md" | "mdx" | "rst" | "org" | "nfo" | "log" | "ini" | "conf" | "cfg" | "env"
        | "lock" | "properties" | "gitignore" | "editorconfig" | "doc" | "docx" | "odt" | "rtf"
        | "tex" | "pages" | "wps" => FileIconKind::Document,
        "csv" | "tsv" | "xls" | "xlsx" | "ods" | "numbers" => FileIconKind::Spreadsheet,
        "ppt" | "pptx" | "odp" | "key" => FileIconKind::Presentation,
        "ipynb" | "rmd" | "qmd" => FileIconKind::Notebook,
        "ttf" | "otf" | "woff" | "woff2" | "eot" => FileIconKind::Font,
        "appimage" | "desktop" | "exe" | "msi" | "app" | "run" | "bin" => FileIconKind::Application,
        "apk" | "aab" => FileIconKind::Android,
        "deb" | "rpm" | "pkg" | "flatpak" | "snap" => FileIconKind::Package,
        "iso" | "img" | "dmg" | "vdi" | "vmdk" | "qcow2" => FileIconKind::Disk,
        "db" | "sqlite" | "sqlite3" | "mdb" | "accdb" | "realm" => FileIconKind::Database,
        "epub" | "mobi" | "azw" | "azw3" | "djvu" | "djv" | "cbz" | "cbr" => FileIconKind::Ebook,
        "obj" | "stl" | "fbx" | "gltf" | "glb" | "blend" | "3ds" | "step" | "stp" | "dae"
        | "ply" | "usd" | "usdz" => FileIconKind::Model3d,
        _ => FileIconKind::Generic,
    }
}

#[cfg(test)]
mod file_icon_tests {
    use super::{file_icon_kind, FileIconKind};

    #[test]
    fn common_extensions_get_distinct_icon_families() {
        let cases = [
            ("jpg", FileIconKind::Image),
            ("jxl", FileIconKind::Image),
            ("psd", FileIconKind::Image),
            ("vob", FileIconKind::Video),
            ("mka", FileIconKind::Audio),
            ("zig", FileIconKind::Code),
            ("wasm", FileIconKind::Code),
            ("sol", FileIconKind::Code),
            ("mp4", FileIconKind::Video),
            ("mp3", FileIconKind::Audio),
            ("zip", FileIconKind::ZipArchive),
            ("lua", FileIconKind::Lua),
            ("rs", FileIconKind::Rust),
            ("py", FileIconKind::Python),
            ("c", FileIconKind::C),
            ("cpp", FileIconKind::Cpp),
            ("js", FileIconKind::Javascript),
            ("ts", FileIconKind::Typescript),
            ("java", FileIconKind::Jvm),
            ("go", FileIconKind::Go),
            ("sh", FileIconKind::Shell),
            ("html", FileIconKind::Web),
            ("php", FileIconKind::Php),
            ("rb", FileIconKind::Ruby),
            ("swift", FileIconKind::Swift),
            ("dart", FileIconKind::Dart),
            ("r", FileIconKind::RLang),
            ("hs", FileIconKind::Haskell),
            ("ex", FileIconKind::Elixir),
            ("pdf", FileIconKind::Pdf),
            ("docx", FileIconKind::Document),
            ("xlsx", FileIconKind::Spreadsheet),
            ("pptx", FileIconKind::Presentation),
            ("ttf", FileIconKind::Font),
            ("appimage", FileIconKind::Application),
            ("deb", FileIconKind::Package),
            ("apk", FileIconKind::Android),
            ("aab", FileIconKind::Android),
            ("iso", FileIconKind::Disk),
            ("sqlite", FileIconKind::Database),
            ("epub", FileIconKind::Ebook),
            ("ipynb", FileIconKind::Notebook),
            ("blend", FileIconKind::Model3d),
            ("unrecognized", FileIconKind::Generic),
        ];
        for (extension, expected) in cases {
            assert_eq!(
                file_icon_kind(extension),
                expected,
                "extension: {extension}"
            );
        }
    }

    #[test]
    fn special_file_names_take_priority_over_extension() {
        let label = |name: &str, is_dir: bool| super::special_icon(name, is_dir).map(|(_, label)| label);
        assert_eq!(label("package.json", false), Some("NODE"));
        assert_eq!(label("Cargo.toml", false), Some("CARGO"));
        assert_eq!(label("Dockerfile", false), Some("DOCKER"));
        assert_eq!(label("docker-compose.yml", false), Some("DOCKER"));
        assert_eq!(label("CMakeLists.txt", false), Some("CMAKE"));
        assert_eq!(label("Makefile", false), Some("MAKE"));
        assert_eq!(label("Brewfile", false), Some("BREW"));
        assert_eq!(label("Procfile", false), Some("PROC"));
        assert_eq!(label("Vagrantfile", false), Some("VGR"));
        assert_eq!(label("requirements.txt", false), Some("REQ"));
        assert_eq!(label(".npmrc", false), Some("NPM"));
        assert_eq!(label("README.md", false), Some("DOC"));
        assert_eq!(label("LICENSE", false), Some("LIC"));
        assert_eq!(label(".git", true), Some("GIT"));
        assert_eq!(label("notes.txt", false), None);
    }

    #[test]
    fn file_clipboard_uri_escapes_spaces_and_unicode() {
        assert_eq!(
            super::file_uri(std::path::Path::new("/tmp/Minha pasta/ação.txt")),
            "file:///tmp/Minha%20pasta/a%C3%A7%C3%A3o.txt"
        );
    }

    #[test]
    fn parses_gnome_copy_and_cut_file_clipboards() {
        let copied = super::parse_gnome_file_clipboard(
            "copy\nfile:///tmp/one.txt\nfile:///tmp/two%20files.txt\n",
        )
        .unwrap();
        assert!(!copied.1);
        assert_eq!(copied.0.len(), 2);
        assert_eq!(copied.0[1], std::path::PathBuf::from("/tmp/two files.txt"));

        let cut = super::parse_gnome_file_clipboard("cut\nfile:///tmp/one.txt\n").unwrap();
        assert!(cut.1);
        assert_eq!(cut.0, [std::path::PathBuf::from("/tmp/one.txt")]);
        assert!(super::parse_gnome_file_clipboard("some text").is_none());
    }

    #[test]
    fn parses_uri_list_and_kde_cut_selection() {
        let mime_types = vec![
            "text/uri-list".to_string(),
            "application/x-kde-cutselection".to_string(),
        ];
        let clipboard =
            super::read_file_clipboard_from_mime_types(&mime_types, |mime| match mime {
                "text/uri-list" => Ok("file:///tmp/two%20files.txt\r\n".to_string()),
                "application/x-kde-cutselection" => Ok("1".to_string()),
                _ => Err(format!("MIME inesperado: {mime}")),
            })
            .unwrap()
            .unwrap();

        assert!(clipboard.1);
        assert_eq!(
            clipboard.0,
            [std::path::PathBuf::from("/tmp/two files.txt")]
        );
    }

    #[test]
    fn treemap_label_does_not_replace_search_or_root_name() {
        assert_eq!(super::short_treemap_name("Videos", 6), "Videos");
        assert_eq!(
            super::short_treemap_name("Street.Fighter.30th.Anniversary.Collection", 14),
            "Street.Fighte…"
        );
        assert_eq!(super::short_treemap_name("long-name", 1), "…");
    }

    #[test]
    fn text_preview_detects_content_without_relying_on_extension() {
        let path = std::env::temp_dir().join(format!(
            "nexus-preview-text-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, "print('hello')\nλ = 42\n").unwrap();
        assert_eq!(
            super::read_text_preview(&path, 1024).as_deref(),
            Some("print('hello')\nλ = 42\n")
        );
        std::fs::write(&path, [0_u8, 159, 146, 150]).unwrap();
        assert!(super::read_text_preview(&path, 1024).is_none());
        std::fs::remove_file(path).unwrap();
    }
}

fn short_treemap_name(name: &str, max_chars: usize) -> String {
    if name.chars().count() <= max_chars {
        return name.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }
    let mut shortened: String = name.chars().take(max_chars.saturating_sub(1)).collect();
    shortened.push('…');
    shortened
}

fn special_icon(name: &str, is_dir: bool) -> Option<(Color32, &'static str)> {
    let lower = name.to_ascii_lowercase();
    let node = (Color32::from_rgb(95, 160, 78), "NODE");
    let cargo = (Color32::from_rgb(196, 90, 50), "CARGO");
    let docker = (Color32::from_rgb(36, 150, 237), "DOCKER");
    let cmake = (Color32::from_rgb(6, 79, 140), "CMAKE");
    let build = (Color32::from_rgb(109, 128, 134), "MAKE");
    let git = (Color32::from_rgb(240, 80, 51), "GIT");
    let docs = (Color32::from_rgb(74, 125, 181), "DOC");
    let license = (Color32::from_rgb(160, 122, 44), "LIC");

    if matches!(
        lower.as_str(),
        ".git" | ".github" | ".gitlab" | ".gitignore" | ".gitmodules" | ".gitattributes"
    ) {
        return Some(git);
    }
    if matches!(
        lower.as_str(),
        "package.json" | "package-lock.json" | "yarn.lock" | "pnpm-lock.yaml"
    ) || (is_dir && lower == "node_modules")
    {
        return Some(node);
    }
    if matches!(lower.as_str(), "cargo.toml" | "cargo.lock") {
        return Some(cargo);
    }
    if lower == "dockerfile"
        || lower.starts_with("dockerfile.")
        || lower.ends_with(".dockerfile")
        || matches!(
            lower.as_str(),
            "docker-compose.yml" | "docker-compose.yaml" | "compose.yml" | "compose.yaml"
        )
    {
        return Some(docker);
    }
    if lower == "cmakelists.txt" || lower.ends_with(".cmake") {
        return Some(cmake);
    }
    if matches!(lower.as_str(), "makefile" | "gnumakefile" | "justfile") {
        return Some(build);
    }
    if lower.starts_with("readme")
        || lower == "changelog"
        || lower.starts_with("changelog.")
        || lower == "contributing"
        || lower.starts_with("contributing.")
        || lower == "install"
        || lower.starts_with("install.")
        || lower == "authors"
        || lower.starts_with("authors.")
    {
        return Some(docs);
    }
    if lower == "license"
        || lower == "licence"
        || lower.starts_with("license.")
        || lower.starts_with("licence.")
        || lower == "copying"
        || lower.starts_with("copying.")
        || lower == "notice"
        || lower.starts_with("notice.")
        || lower == "unlicense"
    {
        return Some(license);
    }
    if lower == "brewfile" {
        return Some((Color32::from_rgb(240, 160, 42), "BREW"));
    }
    if lower == "procfile" {
        return Some((Color32::from_rgb(108, 99, 255), "PROC"));
    }
    if lower == "vagrantfile" {
        return Some((Color32::from_rgb(21, 110, 160), "VGR"));
    }
    if lower == "jenkinsfile" {
        return Some((Color32::from_rgb(210, 70, 70), "JENK"));
    }
    if lower == ".editorconfig" {
        return Some((Color32::from_rgb(120, 140, 155), "EDIT"));
    }
    if lower == ".npmrc" {
        return Some((Color32::from_rgb(203, 56, 55), "NPM"));
    }
    if lower == ".nvmrc" {
        return Some((Color32::from_rgb(90, 160, 90), "NVM"));
    }
    if lower == ".python-version" {
        return Some((Color32::from_rgb(55, 118, 171), "PY"));
    }
    if lower == "requirements.txt" {
        return Some((Color32::from_rgb(55, 118, 171), "REQ"));
    }
    if lower == "pipfile" {
        return Some((Color32::from_rgb(55, 118, 171), "PIP"));
    }
    if lower == "poetry.lock" || lower == "uv.lock" {
        return Some((Color32::from_rgb(55, 118, 171), "PY"));
    }
    None
}

fn draw_entry_badges(painter: &egui::Painter, rect: egui::Rect, e: &Entry) {
    let unit = rect.width().min(rect.height());
    if e.name.starts_with('.') {
        painter.rect_filled(
            egui::Rect::from_center_size(rect.center(), egui::vec2(unit * 0.94, unit * 0.94)),
            egui::Rounding::same(unit * 0.12),
            Color32::from_rgba_unmultiplied(96, 100, 112, 80),
        );
    }
    let badge = (unit * 0.34).clamp(9.0, 18.0);
    let ring = egui::Stroke::new(1.0_f32, Color32::from_gray(115));
    if e.is_symlink {
        let center = egui::pos2(rect.left() + badge * 0.55, rect.bottom() - badge * 0.55);
        painter.circle_filled(center, badge * 0.5, Color32::from_rgb(24, 28, 38));
        painter.circle_stroke(center, badge * 0.5, ring);
        let white = Color32::from_rgb(235, 240, 250);
        let line = egui::Stroke::new(badge * 0.10, white);
        let start = center + egui::vec2(-badge * 0.18, badge * 0.18);
        let end = center + egui::vec2(badge * 0.17, -badge * 0.17);
        painter.line_segment([start, end], line);
        painter.line_segment(
            [end, end + egui::vec2(-badge * 0.16, 0.0)],
            line,
        );
        painter.line_segment([end, end + egui::vec2(0.0, badge * 0.16)], line);
    }
    if e.is_dir {
        return;
    }
    if !e.readable {
        let center = egui::pos2(rect.right() - badge * 0.55, rect.bottom() - badge * 0.55);
        painter.circle_filled(center, badge * 0.5, Color32::from_rgb(24, 28, 38));
        painter.circle_stroke(center, badge * 0.5, ring);
        let color = Color32::from_rgb(240, 190, 70);
        let shackle = center + egui::vec2(0.0, -badge * 0.06);
        let points: Vec<_> = (0..=10)
            .map(|index| {
                let angle =
                    std::f32::consts::PI + std::f32::consts::PI * index as f32 / 10.0;
                egui::pos2(
                    shackle.x + angle.cos() * badge * 0.16,
                    shackle.y + angle.sin() * badge * 0.15,
                )
            })
            .collect();
        painter.add(egui::Shape::line(
            points,
            egui::Stroke::new(badge * 0.09, color),
        ));
        painter.rect_filled(
            egui::Rect::from_center_size(
                center + egui::vec2(0.0, badge * 0.10),
                egui::vec2(badge * 0.36, badge * 0.28),
            ),
            badge * 0.06,
            color,
        );
    } else if e.executable {
        let center = egui::pos2(rect.right() - badge * 0.55, rect.bottom() - badge * 0.55);
        painter.circle_filled(center, badge * 0.5, Color32::from_rgb(34, 88, 52));
        painter.circle_stroke(
            center,
            badge * 0.5,
            egui::Stroke::new(1.0_f32, Color32::from_rgb(90, 200, 120)),
        );
        painter.add(egui::Shape::convex_polygon(
            vec![
                center + egui::vec2(-badge * 0.10, -badge * 0.17),
                center + egui::vec2(badge * 0.17, 0.0),
                center + egui::vec2(-badge * 0.10, badge * 0.17),
            ],
            Color32::from_rgb(160, 245, 180),
            egui::Stroke::NONE,
        ));
    }
}

fn file_icon_color(kind: FileIconKind) -> Color32 {
    match kind {
        FileIconKind::Image => Color32::from_rgb(46, 160, 112),
        FileIconKind::Video => Color32::from_rgb(204, 76, 71),
        FileIconKind::Audio => Color32::from_rgb(139, 92, 206),
        FileIconKind::Archive => Color32::from_rgb(196, 142, 54),
        FileIconKind::ZipArchive => Color32::from_rgb(228, 178, 76),
        FileIconKind::Code => Color32::from_rgb(52, 120, 168),
        FileIconKind::Python => Color32::from_rgb(55, 118, 171),
        FileIconKind::C => Color32::from_rgb(86, 156, 214),
        FileIconKind::Cpp => Color32::from_rgb(0, 100, 165),
        FileIconKind::Rust => Color32::from_rgb(188, 92, 54),
        FileIconKind::Javascript => Color32::from_rgb(203, 170, 46),
        FileIconKind::Typescript => Color32::from_rgb(49, 120, 198),
        FileIconKind::Jvm => Color32::from_rgb(176, 114, 25),
        FileIconKind::Go => Color32::from_rgb(0, 145, 160),
        FileIconKind::Shell => Color32::from_rgb(86, 154, 62),
        FileIconKind::Web => Color32::from_rgb(198, 98, 42),
        FileIconKind::Php => Color32::from_rgb(104, 108, 164),
        FileIconKind::Ruby => Color32::from_rgb(198, 56, 48),
        FileIconKind::Swift => Color32::from_rgb(236, 87, 60),
        FileIconKind::Dart => Color32::from_rgb(1, 117, 194),
        FileIconKind::RLang => Color32::from_rgb(39, 109, 195),
        FileIconKind::Haskell => Color32::from_rgb(94, 80, 134),
        FileIconKind::Elixir => Color32::from_rgb(110, 74, 126),
        FileIconKind::Lua => Color32::from_rgb(48, 74, 130),
        FileIconKind::Pdf => Color32::from_rgb(206, 68, 76),
        FileIconKind::Spreadsheet => Color32::from_rgb(43, 148, 96),
        FileIconKind::Presentation => Color32::from_rgb(212, 104, 54),
        FileIconKind::Font => Color32::from_rgb(86, 96, 170),
        FileIconKind::Application => Color32::from_rgb(42, 138, 160),
        FileIconKind::Package => Color32::from_rgb(188, 82, 122),
        FileIconKind::Android => Color32::from_rgb(61, 220, 132),
        FileIconKind::Disk => Color32::from_rgb(74, 112, 148),
        FileIconKind::Database => Color32::from_rgb(48, 117, 146),
        FileIconKind::Ebook => Color32::from_rgb(166, 83, 143),
        FileIconKind::Notebook => Color32::from_rgb(200, 138, 48),
        FileIconKind::Model3d => Color32::from_rgb(92, 136, 80),
        FileIconKind::Document | FileIconKind::Generic => Color32::from_rgb(240, 243, 249),
    }
}

/// Extensões de código que usam o ícone genérico de editor, mas com cor e sigla próprias.
fn language_details(extension: &str) -> Option<(Color32, &'static str)> {
    let details = match extension {
        "zig" => (Color32::from_rgb(247, 162, 39), "ZIG"),
        "nix" => (Color32::from_rgb(82, 124, 190), "NIX"),
        "v" => (Color32::from_rgb(85, 120, 200), "V"),
        "sv" | "svh" => (Color32::from_rgb(96, 150, 90), "SV"),
        "vhd" | "vhdl" => (Color32::from_rgb(90, 140, 160), "VHDL"),
        "asm" | "s" => (Color32::from_rgb(120, 124, 136), "ASM"),
        "wat" | "wasm" => (Color32::from_rgb(101, 79, 163), "WASM"),
        "graphql" | "gql" => (Color32::from_rgb(211, 72, 150), "GQL"),
        "proto" => (Color32::from_rgb(70, 150, 170), "PROTO"),
        "thrift" => (Color32::from_rgb(140, 100, 160), "THRIFT"),
        "prisma" => (Color32::from_rgb(80, 90, 160), "PRISMA"),
        "tf" | "tfvars" => (Color32::from_rgb(120, 90, 180), "TF"),
        "hcl" => (Color32::from_rgb(120, 90, 180), "HCL"),
        "sol" => (Color32::from_rgb(110, 110, 120), "SOL"),
        "move" => (Color32::from_rgb(70, 160, 150), "MOVE"),
        "purs" => (Color32::from_rgb(120, 120, 200), "PURS"),
        "cr" => (Color32::from_rgb(90, 160, 180), "CR"),
        "jl" => (Color32::from_rgb(149, 88, 178), "JL"),
        "m" => (Color32::from_rgb(90, 120, 180), "M"),
        "mm" => (Color32::from_rgb(90, 120, 180), "MM"),
        "raku" => (Color32::from_rgb(110, 130, 200), "RAKU"),
        "elm" => (Color32::from_rgb(90, 170, 180), "ELM"),
        "json" => (Color32::from_rgb(120, 124, 136), "JSON"),
        "toml" => (Color32::from_rgb(150, 104, 78), "TOML"),
        "yaml" | "yml" => (Color32::from_rgb(190, 80, 90), "YAML"),
        "xml" => (Color32::from_rgb(110, 150, 120), "XML"),
        "sql" => (Color32::from_rgb(60, 130, 170), "SQL"),
        "gradle" => (Color32::from_rgb(60, 150, 150), "GRADLE"),
        "cmake" => (Color32::from_rgb(6, 79, 140), "CMAKE"),
        _ => return None,
    };
    Some(details)
}

fn icon_unit(rect: egui::Rect) -> f32 {
    rect.width().min(rect.height())
}

fn draw_tile(painter: &egui::Painter, rect: egui::Rect, color: Color32) -> egui::Rect {
    let unit = icon_unit(rect);
    let tile = egui::Rect::from_center_size(rect.center(), egui::vec2(unit * 0.80, unit * 0.80));
    painter.rect_filled(tile, unit * 0.18, color);
    tile
}

fn push_arc(
    points: &mut Vec<egui::Pos2>,
    center: egui::Pos2,
    radius: f32,
    from: f32,
    to: f32,
) {
    for index in 1..=4 {
        let angle = from + (to - from) * index as f32 / 4.0;
        points.push(center + egui::vec2(angle.cos() * radius, angle.sin() * radius));
    }
}

/// Página com cantos arredondados no topo e canto dobrado no rodapé direito.
fn page_points(page: egui::Rect, radius: f32, fold: f32) -> Vec<egui::Pos2> {
    let r = radius
        .min(page.width() * 0.3)
        .min(page.height() * 0.3);
    let fold = fold.min(page.width() * 0.4).min(page.height() * 0.4);
    let mut points = Vec::with_capacity(28);
    points.push(egui::pos2(page.left() + r, page.top()));
    points.push(egui::pos2(page.right() - r, page.top()));
    push_arc(
        &mut points,
        egui::pos2(page.right() - r, page.top() + r),
        r,
        -std::f32::consts::FRAC_PI_2,
        0.0,
    );
    points.push(egui::pos2(page.right(), page.bottom() - fold));
    points.push(egui::pos2(page.right() - fold, page.bottom()));
    points.push(egui::pos2(page.left() + r, page.bottom()));
    push_arc(
        &mut points,
        egui::pos2(page.left() + r, page.bottom() - r),
        r,
        std::f32::consts::FRAC_PI_2,
        std::f32::consts::PI,
    );
    points.push(egui::pos2(page.left(), page.top() + r));
    push_arc(
        &mut points,
        egui::pos2(page.left() + r, page.top() + r),
        r,
        std::f32::consts::PI,
        1.5 * std::f32::consts::PI,
    );
    points
}

fn draw_page(
    painter: &egui::Painter,
    rect: egui::Rect,
    base: Color32,
    flap: Color32,
) -> egui::Rect {
    let unit = icon_unit(rect);
    let page = egui::Rect::from_center_size(rect.center(), egui::vec2(unit * 0.74, unit * 0.84));
    let fold = unit * 0.20;
    painter.add(egui::Shape::convex_polygon(
        page_points(page, unit * 0.07, fold),
        base,
        egui::Stroke::NONE,
    ));
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(page.right() - fold, page.bottom()),
            egui::pos2(page.right(), page.bottom() - fold),
            egui::pos2(page.right() - fold, page.bottom() - fold),
        ],
        flap,
        egui::Stroke::NONE,
    ));
    page
}

fn alpha(color: Color32, value: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), value)
}

fn draw_code_tile(painter: &egui::Painter, rect: egui::Rect, base: Color32, label: &str) {
    let unit = icon_unit(rect);
    let tile = draw_tile(painter, rect, base);
    let window = egui::Rect::from_center_size(
        tile.center() - egui::vec2(0.0, unit * 0.035),
        egui::vec2(unit * 0.58, unit * 0.44),
    );
    painter.rect_filled(window, unit * 0.07, Color32::from_rgb(245, 247, 251));
    let dots = [
        Color32::from_rgb(232, 106, 94),
        Color32::from_rgb(240, 190, 80),
        Color32::from_rgb(98, 196, 120),
    ];
    for (index, color) in dots.iter().enumerate() {
        painter.circle_filled(
            egui::pos2(
                window.left() + unit * (0.075 + index as f32 * 0.052),
                window.top() + unit * 0.060,
            ),
            unit * 0.016,
            *color,
        );
    }
    painter.line_segment(
        [
            egui::pos2(window.left() + unit * 0.045, window.top() + unit * 0.105),
            egui::pos2(window.right() - unit * 0.045, window.top() + unit * 0.105),
        ],
        egui::Stroke::new(unit * 0.010, alpha(base, 70)),
    );
    painter.text(
        window.center() + egui::vec2(0.0, unit * 0.050),
        Align2::CENTER_CENTER,
        "</>",
        FontId::monospace(unit * 0.142),
        base,
    );
    let size = match label.chars().count() {
        0 => 0.0,
        1 | 2 => unit * 0.112,
        3 => unit * 0.098,
        4 => unit * 0.084,
        5 => unit * 0.072,
        _ => unit * 0.062,
    };
    if size > 0.0 && unit >= 26.0 {
        painter.text(
            egui::pos2(tile.center().x, tile.bottom() - unit * 0.085),
            Align2::CENTER_CENTER,
            label,
            FontId::monospace(size),
            Color32::from_white_alpha(228),
        );
    }
}

fn tile_point(tile: egui::Rect, x: f32, y: f32) -> egui::Pos2 {
    egui::pos2(
        tile.left() + tile.width() * x,
        tile.top() + tile.height() * y,
    )
}

fn tile_rect(tile: egui::Rect, x0: f32, y0: f32, x1: f32, y1: f32) -> egui::Rect {
    egui::Rect::from_min_max(tile_point(tile, x0, y0), tile_point(tile, x1, y1))
}

fn tile_round_rect(
    painter: &egui::Painter,
    tile: egui::Rect,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    radius: f32,
    color: Color32,
) {
    painter.rect_filled(
        tile_rect(tile, x0, y0, x1, y1),
        tile.width() * radius,
        color,
    );
}

fn tile_polygon(
    painter: &egui::Painter,
    tile: egui::Rect,
    points: &[(f32, f32)],
    color: Color32,
) {
    painter.add(egui::Shape::convex_polygon(
        points
            .iter()
            .map(|&(x, y)| tile_point(tile, x, y))
            .collect(),
        color,
        egui::Stroke::NONE,
    ));
}

fn tile_ellipse(
    painter: &egui::Painter,
    tile: egui::Rect,
    cx: f32,
    cy: f32,
    rx: f32,
    ry: f32,
    color: Color32,
) {
    let points = (0..=48)
        .map(|index| {
            let angle = std::f32::consts::TAU * index as f32 / 48.0;
            tile_point(tile, cx + angle.cos() * rx, cy + angle.sin() * ry)
        })
        .collect();
    painter.add(egui::Shape::convex_polygon(
        points,
        color,
        egui::Stroke::NONE,
    ));
}

fn tile_arc(
    painter: &egui::Painter,
    tile: egui::Rect,
    cx: f32,
    cy: f32,
    rx: f32,
    ry: f32,
    from: f32,
    to: f32,
    stroke: egui::Stroke,
) {
    let points = (0..=20)
        .map(|index| {
            let angle = from + (to - from) * index as f32 / 20.0;
            tile_point(tile, cx + angle.cos() * rx, cy + angle.sin() * ry)
        })
        .collect();
    painter.add(egui::Shape::line(points, stroke));
}

fn tile_text(
    painter: &egui::Painter,
    tile: egui::Rect,
    text: &str,
    x: f32,
    y: f32,
    size: f32,
    color: Color32,
) {
    painter.text(
        tile_point(tile, x, y),
        Align2::CENTER_CENTER,
        text,
        FontId::monospace(tile.width() * size),
        color,
    );
}

fn tile_text_bold(
    painter: &egui::Painter,
    tile: egui::Rect,
    text: &str,
    x: f32,
    y: f32,
    size: f32,
    color: Color32,
) {
    tile_text(painter, tile, text, x, y, size, color);
    let offset = tile.width() * 0.006;
    for (dx, dy) in [(offset, 0.0), (0.0, offset)] {
        painter.text(
            tile_point(tile, x, y) + egui::vec2(dx, dy),
            Align2::CENTER_CENTER,
            text,
            FontId::monospace(tile.width() * size),
            color,
        );
    }
}

fn draw_logo_mesh(
    painter: &egui::Painter,
    rect: egui::Rect,
    triangles: &[[f32; 6]],
    color: Color32,
) {
    let mut mesh = egui::Mesh::default();
    for triangle in triangles {
        let start = mesh.vertices.len() as u32;
        for corner in 0..3 {
            mesh.colored_vertex(
                egui::pos2(
                    rect.left() + triangle[corner * 2] * rect.width(),
                    rect.top() + triangle[corner * 2 + 1] * rect.height(),
                ),
                color,
            );
        }
        mesh.add_triangle(start, start + 1, start + 2);
    }
    painter.add(egui::Shape::mesh(mesh));
}

fn draw_python_tile(painter: &egui::Painter, rect: egui::Rect) {
    let base = Color32::from_rgb(45, 100, 150);
    let ink = Color32::from_rgb(200, 230, 255);
    let yellow = Color32::from_rgb(255, 212, 59);
    let gap = 0.03_f32;
    let tile = draw_tile(painter, rect, base);
    let min = tile.min;
    let w = tile.width();
    let h = tile.height();
    let mut r = |x1: f32, y1: f32, x2: f32, y2: f32, rad: f32, color: Color32| {
        let rr = egui::Rect::from_min_max(
            egui::pos2(min.x + x1 * w, min.y + y1 * h),
            egui::pos2(min.x + x2 * w, min.y + y2 * h),
        );
        let rounding = if rad <= 0.0 {
            egui::Rounding::ZERO
        } else {
            egui::Rounding::same(rad * w)
        };
        painter.rect_filled(rr, rounding, color);
    };
    let dot = |x: f32, y: f32, rr: f32, color: Color32| {
        painter.circle_filled(egui::pos2(min.x + x * w, min.y + y * h), rr * w, color);
    };
    // 1) lóbulo e cauda da cobra azul
    r(0.017, 0.26, 0.38, 0.64, 0.12, ink);
    r(0.017, 0.26, 0.23, 0.755, 0.12, ink);
    r(0.125, 0.63, 0.23, 0.755, 0.0, ink);
    // 2) vão em volta do corpo amarelo
    r(0.255 - gap, 0.52 - gap, 0.74, 1.0, 0.12 + gap, base);
    // 3) cobra amarela
    r(0.62, 0.36, 0.983, 0.74, 0.12, yellow);
    r(0.77, 0.245, 0.983, 0.74, 0.12, yellow);
    r(0.77, 0.245, 0.875, 0.37, 0.0, yellow);
    r(0.255, 0.52, 0.74, 1.0, 0.12, yellow);
    // 4) vão em volta do bloco azul e o bloco
    r(0.26, 0.0, 0.745 + gap, 0.49 + gap, 0.12 + gap, base);
    r(0.26, 0.0, 0.745, 0.49, 0.12, ink);
    // 5) bocas e olhos
    r(0.26, 0.229, 0.50, 0.26, 0.0, base);
    r(0.50, 0.74, 0.74, 0.771, 0.0, base);
    dot(0.375, 0.12, 0.04, base);
    dot(0.625, 0.88, 0.04, base);
}

fn draw_swift_tile(painter: &egui::Painter, rect: egui::Rect) {
    let unit = icon_unit(rect);
    let tile = draw_tile(painter, rect, Color32::from_rgb(240, 81, 56));
    let logo = egui::Rect::from_center_size(tile.center(), egui::vec2(unit * 0.70, unit * 0.70));
    draw_logo_mesh(painter, logo, logo_mesh::SWIFT_BIRD, Color32::WHITE);
}

fn draw_letter_tile(
    painter: &egui::Painter,
    rect: egui::Rect,
    background: Color32,
    text: &str,
    size: f32,
) {
    let tile = draw_tile(painter, rect, background);
    tile_text_bold(painter, tile, text, 0.5, 0.53, size, Color32::WHITE);
}

fn draw_rust_tile(painter: &egui::Painter, rect: egui::Rect) {
    let unit = icon_unit(rect);
    let tile = draw_tile(painter, rect, Color32::from_rgb(206, 66, 43));
    let cream = Color32::from_rgb(244, 236, 230);
    let mut mesh = egui::Mesh::default();
    let center = mesh.vertices.len() as u32;
    mesh.colored_vertex(tile.center(), cream);
    for index in 0..16 {
        let a1 = std::f32::consts::TAU * index as f32 / 16.0;
        let a2 = std::f32::consts::TAU * (index + 1) as f32 / 16.0;
        let r1 = if index % 2 == 0 { 0.30 } else { 0.225 };
        let r2 = if index % 2 == 0 { 0.225 } else { 0.30 };
        let p1 = mesh.vertices.len() as u32;
        mesh.colored_vertex(
            tile_point(tile, 0.5 + a1.cos() * r1, 0.5 + a1.sin() * r1),
            cream,
        );
        let p2 = mesh.vertices.len() as u32;
        mesh.colored_vertex(
            tile_point(tile, 0.5 + a2.cos() * r2, 0.5 + a2.sin() * r2),
            cream,
        );
        mesh.add_triangle(center, p1, p2);
    }
    painter.add(egui::Shape::mesh(mesh));
    painter.circle_filled(tile.center(), unit * 0.136, Color32::from_rgb(206, 66, 43));
    tile_text_bold(painter, tile, "R", 0.5, 0.53, 0.25, cream);
}

fn draw_java_tile(painter: &egui::Painter, rect: egui::Rect) {
    let unit = icon_unit(rect);
    let tile = draw_tile(painter, rect, Color32::from_rgb(83, 130, 161));
    let cream = Color32::from_rgb(247, 243, 234);
    tile_polygon(
        painter,
        tile,
        &[(0.30, 0.42), (0.68, 0.42), (0.62, 0.71), (0.36, 0.71)],
        cream,
    );
    tile_arc(
        painter,
        tile,
        0.715,
        0.55,
        0.105,
        0.11,
        -std::f32::consts::FRAC_PI_2,
        std::f32::consts::FRAC_PI_2,
        egui::Stroke::new(unit * 0.045, cream),
    );
    tile_round_rect(painter, tile, 0.24, 0.74, 0.76, 0.79, 0.02, cream);
    for x in [0.40_f32, 0.52] {
        tile_arc(
            painter,
            tile,
            x,
            0.28,
            0.05,
            0.10,
            std::f32::consts::FRAC_PI_2,
            3.0 * std::f32::consts::FRAC_PI_2,
            egui::Stroke::new(unit * 0.025, cream),
        );
    }
}

fn draw_go_tile(painter: &egui::Painter, rect: egui::Rect) {
    let tile = draw_tile(painter, rect, Color32::from_rgb(0, 173, 216));
    let face = Color32::from_rgb(234, 247, 249);
    let ink = Color32::from_rgb(18, 58, 68);
    tile_ellipse(painter, tile, 0.28, 0.42, 0.075, 0.075, face);
    tile_ellipse(painter, tile, 0.72, 0.42, 0.075, 0.075, face);
    tile_ellipse(painter, tile, 0.50, 0.57, 0.28, 0.28, face);
    tile_ellipse(painter, tile, 0.40, 0.53, 0.075, 0.075, Color32::WHITE);
    tile_ellipse(painter, tile, 0.60, 0.53, 0.075, 0.075, Color32::WHITE);
    tile_ellipse(painter, tile, 0.42, 0.55, 0.037, 0.037, ink);
    tile_ellipse(painter, tile, 0.58, 0.55, 0.037, 0.037, ink);
    tile_polygon(
        painter,
        tile,
        &[(0.46, 0.66), (0.54, 0.66), (0.50, 0.72)],
        ink,
    );
    tile_round_rect(painter, tile, 0.435, 0.735, 0.475, 0.775, 0.02, Color32::WHITE);
    tile_round_rect(painter, tile, 0.525, 0.735, 0.565, 0.775, 0.02, Color32::WHITE);
}

fn draw_php_tile(painter: &egui::Painter, rect: egui::Rect) {
    let tile = draw_tile(painter, rect, Color32::from_rgb(119, 123, 179));
    tile_ellipse(
        painter,
        tile,
        0.5,
        0.5,
        0.35,
        0.22,
        Color32::from_rgb(250, 250, 253),
    );
    tile_text(
        painter,
        tile,
        "php",
        0.5,
        0.53,
        0.20,
        Color32::from_rgb(119, 123, 179),
    );
}

fn draw_ruby_tile(painter: &egui::Painter, rect: egui::Rect) {
    let unit = icon_unit(rect);
    let tile = draw_tile(painter, rect, Color32::from_rgb(204, 52, 45));
    let red = Color32::from_rgb(204, 52, 45);
    tile_polygon(
        painter,
        tile,
        &[
            (0.50, 0.16),
            (0.73, 0.33),
            (0.63, 0.78),
            (0.37, 0.78),
            (0.27, 0.33),
        ],
        Color32::WHITE,
    );
    let line = egui::Stroke::new(unit * 0.013, red);
    painter.line_segment(
        [tile_point(tile, 0.27, 0.33), tile_point(tile, 0.73, 0.33)],
        line,
    );
    for x in [0.38_f32, 0.62] {
        painter.line_segment(
            [tile_point(tile, x, 0.33), tile_point(tile, 0.50, 0.78)],
            egui::Stroke::new(unit * 0.010, red),
        );
    }
}

fn draw_dart_tile(painter: &egui::Painter, rect: egui::Rect) {
    let tile = draw_tile(painter, rect, Color32::from_rgb(1, 117, 194));
    tile_polygon(
        painter,
        tile,
        &[(0.25, 0.75), (0.72, 0.24), (0.76, 0.55), (0.53, 0.78)],
        Color32::WHITE,
    );
    tile_polygon(
        painter,
        tile,
        &[(0.25, 0.75), (0.42, 0.52), (0.48, 0.62)],
        Color32::from_rgb(142, 208, 245),
    );
}

fn draw_elixir_tile(painter: &egui::Painter, rect: egui::Rect) {
    let tile = draw_tile(painter, rect, Color32::from_rgb(110, 74, 126));
    tile_polygon(
        painter,
        tile,
        &[(0.50, 0.14), (0.29, 0.60), (0.71, 0.60)],
        Color32::WHITE,
    );
    tile_ellipse(painter, tile, 0.5, 0.58, 0.23, 0.23, Color32::WHITE);
    tile_ellipse(
        painter,
        tile,
        0.42,
        0.52,
        0.055,
        0.075,
        Color32::from_rgb(210, 190, 220),
    );
}

fn draw_lua_tile(painter: &egui::Painter, rect: egui::Rect) {
    let unit = icon_unit(rect);
    let base = Color32::from_rgb(48, 74, 130);
    let tile = draw_tile(painter, rect, base);
    let cream = Color32::from_rgb(246, 246, 240);
    painter.circle_filled(tile.center(), unit * 0.27, cream);
    painter.circle_filled(
        tile.center() + egui::vec2(unit * 0.12, -unit * 0.09),
        unit * 0.235,
        base,
    );
    for (x, y, size) in [
        (-0.17_f32, -0.19_f32, 0.026_f32),
        (0.16, 0.12, 0.020),
        (-0.05, 0.21, 0.016),
    ] {
        painter.circle_filled(
            tile.center() + egui::vec2(unit * x, unit * y),
            unit * size,
            Color32::from_rgb(247, 205, 120),
        );
    }
}

fn draw_android_tile(painter: &egui::Painter, rect: egui::Rect) {
    let unit = icon_unit(rect);
    let tile = draw_tile(painter, rect, Color32::from_rgb(31, 42, 46));
    let green = Color32::from_rgb(61, 220, 132);
    let ink = Color32::from_rgb(31, 42, 46);
    for (from, to) in [
        ((0.36, 0.36), (0.29, 0.23)),
        ((0.64, 0.36), (0.71, 0.23)),
    ] {
        painter.line_segment(
            [tile_point(tile, from.0, from.1), tile_point(tile, to.0, to.1)],
            egui::Stroke::new(unit * 0.026, green),
        );
        painter.circle_filled(tile_point(tile, to.0, to.1), unit * 0.030, green);
    }
    let dome = (0..=24)
        .map(|index| {
            let angle = std::f32::consts::PI
                + std::f32::consts::PI * index as f32 / 24.0;
            tile_point(tile, 0.50 + angle.cos() * 0.28, 0.55 + angle.sin() * 0.25)
        })
        .collect();
    painter.add(egui::Shape::convex_polygon(
        dome,
        green,
        egui::Stroke::NONE,
    ));
    tile_round_rect(painter, tile, 0.22, 0.55, 0.78, 0.68, 0.06, green);
    tile_ellipse(painter, tile, 0.40, 0.53, 0.045, 0.045, ink);
    tile_ellipse(painter, tile, 0.60, 0.53, 0.045, 0.045, ink);
}

fn draw_application_tile(painter: &egui::Painter, rect: egui::Rect) {
    let tile = draw_tile(painter, rect, Color32::from_rgb(51, 65, 92));
    let cream = Color32::from_rgb(247, 249, 252);
    let bar = Color32::from_rgb(34, 48, 74);
    tile_round_rect(painter, tile, 0.18, 0.18, 0.82, 0.82, 0.10, cream);
    tile_round_rect(painter, tile, 0.18, 0.18, 0.82, 0.34, 0.10, bar);
    painter.rect_filled(tile_rect(tile, 0.18, 0.30, 0.82, 0.36), 0.0, bar);
    for index in 0..3 {
        tile_ellipse(
            painter,
            tile,
            0.27 + index as f32 * 0.07,
            0.26,
            0.018,
            0.018,
            Color32::from_rgb(221, 229, 242),
        );
    }
    tile_ellipse(
        painter,
        tile,
        0.50,
        0.59,
        0.13,
        0.13,
        Color32::from_rgb(61, 111, 208),
    );
    tile_polygon(
        painter,
        tile,
        &[(0.46, 0.51), (0.58, 0.59), (0.46, 0.67)],
        cream,
    );
}

fn draw_monogram_tile(
    painter: &egui::Painter,
    rect: egui::Rect,
    background: Color32,
    label: &str,
) {
    if label.is_empty() {
        draw_code_tile(painter, rect, background, "");
        return;
    }
    let tile = draw_tile(painter, rect, background);
    let size = match label.chars().count() {
        1 => 0.50,
        2 => 0.40,
        3 => 0.29,
        4 => 0.235,
        5 => 0.19,
        _ => 0.155,
    };
    tile_text_bold(
        painter,
        tile,
        label,
        0.5,
        0.53,
        size,
        Color32::from_rgb(247, 249, 252),
    );
}

fn draw_language_icon(painter: &egui::Painter, rect: egui::Rect, kind: FileIconKind) {
    match kind {
        FileIconKind::Python => draw_python_tile(painter, rect),
        FileIconKind::C => draw_letter_tile(
            painter,
            rect,
            Color32::from_rgb(92, 156, 214),
            "C",
            0.60,
        ),
        FileIconKind::Cpp => draw_letter_tile(
            painter,
            rect,
            Color32::from_rgb(0, 89, 156),
            "C++",
            0.33,
        ),
        FileIconKind::Rust => draw_rust_tile(painter, rect),
        FileIconKind::Jvm => draw_java_tile(painter, rect),
        FileIconKind::Go => draw_go_tile(painter, rect),
        FileIconKind::Javascript => {
            let tile = draw_tile(painter, rect, Color32::from_rgb(247, 223, 30));
            tile_text_bold(
                painter,
                tile,
                "JS",
                0.5,
                0.53,
                0.42,
                Color32::from_rgb(26, 26, 26),
            );
        }
        FileIconKind::Typescript => draw_letter_tile(
            painter,
            rect,
            Color32::from_rgb(49, 120, 198),
            "TS",
            0.42,
        ),
        FileIconKind::Shell => {
            let tile = draw_tile(painter, rect, Color32::from_rgb(44, 49, 60));
            tile_text(
                painter,
                tile,
                ">_",
                0.5,
                0.53,
                0.40,
                Color32::from_rgb(155, 227, 109),
            );
        }
        FileIconKind::Web => {
            let tile = draw_tile(painter, rect, Color32::from_rgb(228, 77, 38));
            tile_text(painter, tile, "</>", 0.5, 0.53, 0.30, Color32::WHITE);
        }
        FileIconKind::Php => draw_php_tile(painter, rect),
        FileIconKind::Ruby => draw_ruby_tile(painter, rect),
        FileIconKind::Swift => draw_swift_tile(painter, rect),
        FileIconKind::Dart => draw_dart_tile(painter, rect),
        FileIconKind::RLang => draw_letter_tile(
            painter,
            rect,
            Color32::from_rgb(39, 109, 195),
            "R",
            0.55,
        ),
        FileIconKind::Haskell => {
            let tile = draw_tile(painter, rect, Color32::from_rgb(93, 79, 133));
            tile_text_bold(painter, tile, "λ", 0.5, 0.55, 0.55, Color32::WHITE);
        }
        FileIconKind::Elixir => draw_elixir_tile(painter, rect),
        FileIconKind::Lua => draw_lua_tile(painter, rect),
        _ => draw_code_tile(painter, rect, Color32::from_rgb(52, 120, 168), ""),
    }
}

fn draw_document_page(painter: &egui::Painter, rect: egui::Rect, pdf: bool, extension: &str) {
    let unit = icon_unit(rect);
    let page = draw_page(
        painter,
        rect,
        Color32::from_rgb(246, 248, 252),
        Color32::from_rgb(206, 216, 230),
    );
    if pdf {
        let chip = egui::Rect::from_center_size(
            page.center() - egui::vec2(0.0, unit * 0.035),
            egui::vec2(unit * 0.42, unit * 0.20),
        );
        painter.rect_filled(chip, unit * 0.05, Color32::from_rgb(206, 68, 76));
        painter.text(
            chip.center(),
            Align2::CENTER_CENTER,
            "PDF",
            FontId::monospace(unit * 0.120),
            Color32::WHITE,
        );
        for (index, width) in [0.60_f32, 0.44].iter().enumerate() {
            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(
                        page.left() + unit * 0.11,
                        page.center().y + unit * (0.13 + index as f32 * 0.10),
                    ),
                    egui::vec2((page.width() - unit * 0.22) * width, unit * 0.038),
                ),
                unit * 0.019,
                Color32::from_rgb(160, 176, 200),
            );
        }
    } else {
        let bookmark_x = page.left() + unit * 0.12;
        let bookmark_width = unit * 0.085;
        let bookmark_bottom = page.top() + unit * 0.20;
        painter.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(bookmark_x, page.top()),
                egui::pos2(bookmark_x + bookmark_width, page.top()),
                egui::pos2(bookmark_x + bookmark_width, bookmark_bottom),
                egui::pos2(
                    bookmark_x + bookmark_width * 0.5,
                    bookmark_bottom - unit * 0.045,
                ),
                egui::pos2(bookmark_x, bookmark_bottom),
            ],
            Color32::from_rgb(74, 125, 181),
            egui::Stroke::NONE,
        ));
        let start_x = page.left() + unit * 0.30;
        let widths = [0.90_f32, 0.72, 0.84, 0.52];
        for (index, width) in widths.iter().enumerate() {
            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(start_x, page.top() + unit * (0.15 + index as f32 * 0.115)),
                    egui::vec2((page.right() - unit * 0.10 - start_x) * width, unit * 0.038),
                ),
                unit * 0.019,
                Color32::from_rgb(150, 170, 198),
            );
        }
        if !extension.is_empty() && unit >= 34.0 {
            painter.text(
                egui::pos2(page.left() + unit * 0.10, page.bottom() - unit * 0.065),
                Align2::LEFT_BOTTOM,
                extension.chars().take(4).collect::<String>().to_uppercase(),
                FontId::monospace(unit * 0.068),
                Color32::from_rgb(150, 162, 178),
            );
        }
    }
}

fn draw_zip_tile(painter: &egui::Painter, rect: egui::Rect, base: Color32, label: &str) {
    let unit = icon_unit(rect);
    let tile = draw_tile(painter, rect, base);
    let cream = mix_color(base, Color32::WHITE, 0.86);
    let track = egui::Rect::from_center_size(
        tile.center() - egui::vec2(0.0, unit * 0.02),
        egui::vec2(unit * 0.075, unit * 0.44),
    );
    painter.rect_filled(track, unit * 0.038, cream);
    for index in 0..6 {
        let y = track.top() + unit * 0.030 + index as f32 * unit * 0.068;
        if y > track.bottom() - unit * 0.095 {
            break;
        }
        let on_left = index % 2 == 0;
        let x = if on_left {
            track.left() - unit * 0.055
        } else {
            track.right()
        };
        painter.rect_filled(
            egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(unit * 0.055, unit * 0.022)),
            unit * 0.008,
            cream,
        );
    }
    painter.rect_filled(
        egui::Rect::from_center_size(
            egui::pos2(track.center().x, track.top() + unit * 0.018),
            egui::vec2(unit * 0.13, unit * 0.028),
        ),
        unit * 0.012,
        cream,
    );
    let slider = egui::Rect::from_center_size(
        egui::pos2(track.center().x, track.center().y + unit * 0.135),
        egui::vec2(unit * 0.19, unit * 0.10),
    );
    painter.rect_filled(slider, unit * 0.035, cream);
    painter.rect_filled(
        slider.shrink(unit * 0.025),
        unit * 0.020,
        darken(base, 0.70),
    );
    painter.circle_stroke(
        egui::pos2(track.center().x, slider.bottom() + unit * 0.052),
        unit * 0.045,
        egui::Stroke::new(unit * 0.016, cream),
    );
    if !label.is_empty() && unit >= 30.0 {
        painter.text(
            egui::pos2(tile.center().x, tile.bottom() - unit * 0.082),
            Align2::CENTER_CENTER,
            label,
            FontId::monospace(unit * 0.085),
            Color32::from_white_alpha(232),
        );
    }
}

fn draw_folder_icon(painter: &egui::Painter, rect: egui::Rect, accent: Color32) {
    let unit = icon_unit(rect);
    let folder = Color32::from_rgb(
        accent.r().saturating_add(35),
        accent.g().saturating_add(22),
        accent.b().saturating_add(10),
    );
    let tab = egui::Rect::from_min_size(
        egui::pos2(rect.left() + unit * 0.09, rect.top() + unit * 0.21),
        egui::vec2(unit * 0.40, unit * 0.17),
    );
    painter.rect_filled(
        tab,
        egui::Rounding {
            nw: unit * 0.035,
            ne: unit * 0.035,
            sw: 0.0,
            se: 0.0,
        },
        darken(folder, 0.82),
    );
    let body = egui::Rect::from_min_size(
        egui::pos2(rect.left() + unit * 0.06, rect.top() + unit * 0.32),
        egui::vec2(unit * 0.88, unit * 0.48),
    );
    painter.rect_filled(body, unit * 0.05, folder);
}

fn draw_entry_icon(painter: &egui::Painter, rect: egui::Rect, e: &Entry, accent: Color32) {
    if let Some((color, label)) = special_icon(&e.name, e.is_dir) {
        if e.is_dir {
            draw_folder_icon(painter, rect, color);
        } else {
            draw_code_tile(painter, rect, color, label);
        }
        return;
    }
    if e.is_dir {
        draw_folder_icon(painter, rect, accent);
        return;
    }
    let extension = e
        .path
        .extension()
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let mut kind = file_icon_kind(&extension);
    if e.executable && kind == FileIconKind::Generic {
        kind = FileIconKind::Application;
    }
    let base = file_icon_color(kind);
    let unit = icon_unit(rect);
    let cream = Color32::from_rgb(246, 246, 240);

    match kind {
        FileIconKind::Image => {
            let tile = draw_tile(painter, rect, base);
            let back = egui::Rect::from_center_size(
                tile.center() + egui::vec2(unit * 0.045, -unit * 0.030),
                egui::vec2(unit * 0.58, unit * 0.48),
            );
            painter.rect_filled(back, unit * 0.05, Color32::from_white_alpha(55));
            let photo = egui::Rect::from_center_size(
                tile.center() - egui::vec2(unit * 0.015, unit * 0.005),
                egui::vec2(unit * 0.58, unit * 0.48),
            );
            painter.rect_filled(photo, unit * 0.05, cream);
            let inner = photo.shrink(unit * 0.05);
            painter.rect_filled(inner, unit * 0.035, mix_color(base, Color32::WHITE, 0.16));
            painter.circle_filled(
                egui::pos2(inner.right() - unit * 0.075, inner.top() + unit * 0.075),
                unit * 0.048,
                cream,
            );
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(inner.left(), inner.bottom()),
                    egui::pos2(inner.left() + inner.width() * 0.34, inner.center().y),
                    egui::pos2(inner.left() + inner.width() * 0.55, inner.bottom()),
                ],
                darken(base, 0.82),
                egui::Stroke::NONE,
            ));
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(inner.left() + inner.width() * 0.36, inner.bottom()),
                    egui::pos2(
                        inner.left() + inner.width() * 0.68,
                        inner.center().y + unit * 0.07,
                    ),
                    egui::pos2(inner.right(), inner.bottom()),
                ],
                mix_color(base, Color32::WHITE, 0.10),
                egui::Stroke::NONE,
            ));
        }
        FileIconKind::Video => {
            let tile = draw_tile(painter, rect, base);
            let body = egui::Rect::from_center_size(
                tile.center() + egui::vec2(0.0, unit * 0.075),
                egui::vec2(unit * 0.58, unit * 0.28),
            );
            painter.rect_filled(body, unit * 0.05, cream);
            let bar = egui::Rect::from_center_size(
                egui::pos2(tile.center().x, body.top() - unit * 0.045),
                egui::vec2(unit * 0.58, unit * 0.12),
            );
            painter.rect_filled(bar, unit * 0.035, cream);
            for index in 0..4 {
                let x = bar.left() + bar.width() * (0.12 + index as f32 * 0.24);
                painter.line_segment(
                    [
                        egui::pos2(x, bar.bottom()),
                        egui::pos2(x + unit * 0.045, bar.top()),
                    ],
                    egui::Stroke::new(unit * 0.030, base),
                );
            }
            painter.circle_filled(
                egui::pos2(bar.left() + unit * 0.030, bar.center().y),
                unit * 0.024,
                base,
            );
            let play = body.center();
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(play.x - unit * 0.052, play.y - unit * 0.072),
                    egui::pos2(play.x + unit * 0.090, play.y),
                    egui::pos2(play.x - unit * 0.052, play.y + unit * 0.072),
                ],
                base,
                egui::Stroke::NONE,
            ));
        }
        FileIconKind::Audio => {
            let tile = draw_tile(painter, rect, base);
            let disc = tile.center() + egui::vec2(0.0, unit * 0.010);
            painter.circle_filled(disc, unit * 0.27, cream);
            for radius in [0.21_f32, 0.15, 0.09] {
                painter.circle_stroke(
                    disc,
                    unit * radius,
                    egui::Stroke::new(unit * 0.014, alpha(base, 95)),
                );
            }
            painter.circle_filled(disc, unit * 0.065, base);
            painter.circle_filled(disc, unit * 0.016, cream);
            let note = egui::pos2(tile.right() - unit * 0.13, tile.top() + unit * 0.16);
            painter.circle_filled(note, unit * 0.040, cream);
            painter.line_segment(
                [
                    egui::pos2(note.x + unit * 0.034, note.y),
                    egui::pos2(note.x + unit * 0.034, note.y - unit * 0.13),
                ],
                egui::Stroke::new(unit * 0.024, cream),
            );
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(note.x + unit * 0.034, note.y - unit * 0.13),
                    egui::pos2(note.x + unit * 0.092, note.y - unit * 0.11),
                    egui::pos2(note.x + unit * 0.092, note.y - unit * 0.055),
                    egui::pos2(note.x + unit * 0.034, note.y - unit * 0.075),
                ],
                cream,
                egui::Stroke::NONE,
            ));
        }
        FileIconKind::Archive | FileIconKind::ZipArchive => {
            let label = if e.name.to_ascii_lowercase().ends_with(".tar.gz") {
                "TAR".to_string()
            } else {
                match extension.as_str() {
                    "zip" => "ZIP".to_string(),
                    "7z" => "7Z".to_string(),
                    "rar" => "RAR".to_string(),
                    "tar" => "TAR".to_string(),
                    "gz" => "GZ".to_string(),
                    "xz" => "XZ".to_string(),
                    "bz2" => "BZ2".to_string(),
                    "zst" => "ZST".to_string(),
                    "tgz" => "TGZ".to_string(),
                    "cab" => "CAB".to_string(),
                    "lz" => "LZ".to_string(),
                    "lz4" => "LZ4".to_string(),
                    _ => String::new(),
                }
            };
            draw_zip_tile(painter, rect, base, &label);
        }
        FileIconKind::Code => {
            if let Some((color, label)) = language_details(&extension) {
                draw_monogram_tile(painter, rect, color, label);
            } else {
                draw_code_tile(painter, rect, base, "");
            }
        }
        FileIconKind::Python
        | FileIconKind::C
        | FileIconKind::Cpp
        | FileIconKind::Rust
        | FileIconKind::Javascript
        | FileIconKind::Typescript
        | FileIconKind::Jvm
        | FileIconKind::Go
        | FileIconKind::Shell
        | FileIconKind::Web
        | FileIconKind::Php
        | FileIconKind::Ruby
        | FileIconKind::Swift
        | FileIconKind::Dart
        | FileIconKind::RLang
        | FileIconKind::Haskell
        | FileIconKind::Elixir
        | FileIconKind::Lua => draw_language_icon(painter, rect, kind),
        FileIconKind::Pdf => draw_document_page(painter, rect, true, &extension),
        FileIconKind::Document | FileIconKind::Generic => {
            draw_document_page(painter, rect, false, &extension)
        }
        FileIconKind::Spreadsheet => {
            let tile = draw_tile(painter, rect, base);
            let table = egui::Rect::from_center_size(
                tile.center(),
                egui::vec2(unit * 0.60, unit * 0.56),
            );
            painter.rect_filled(table, unit * 0.05, cream);
            let header = egui::Rect::from_min_size(
                table.left_top(),
                egui::vec2(table.width(), unit * 0.13),
            );
            painter.rect_filled(
                header,
                egui::Rounding {
                    nw: unit * 0.05,
                    ne: unit * 0.05,
                    sw: 0.0,
                    se: 0.0,
                },
                darken(base, 0.88),
            );
            let grid = egui::Stroke::new(unit * 0.012, alpha(base, 95));
            for column in 1..3 {
                let x = table.left() + table.width() * column as f32 / 3.0;
                painter.line_segment(
                    [
                        egui::pos2(x, header.bottom()),
                        egui::pos2(x, table.bottom() - unit * 0.04),
                    ],
                    grid,
                );
            }
            for row in 1..3 {
                let y = header.bottom()
                    + (table.bottom() - unit * 0.04 - header.bottom()) * row as f32 / 3.0;
                painter.line_segment(
                    [
                        egui::pos2(table.left() + unit * 0.03, y),
                        egui::pos2(table.right() - unit * 0.03, y),
                    ],
                    grid,
                );
            }
            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(
                        table.left() + table.width() / 3.0 + unit * 0.025,
                        header.bottom() + unit * 0.035,
                    ),
                    egui::vec2(unit * 0.075, unit * 0.060),
                ),
                unit * 0.012,
                Color32::from_rgb(240, 190, 80),
            );
        }
        FileIconKind::Presentation => {
            let tile = draw_tile(painter, rect, base);
            let slide = egui::Rect::from_center_size(
                tile.center() - egui::vec2(0.0, unit * 0.06),
                egui::vec2(unit * 0.58, unit * 0.36),
            );
            painter.rect_filled(slide, unit * 0.04, cream);
            for (index, (offset, height)) in
                [(0.10_f32, 0.08_f32), (0.26, 0.16), (0.42, 0.11)]
                    .iter()
                    .enumerate()
            {
                painter.rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(
                            slide.left() + unit * offset,
                            slide.bottom() - unit * (0.045 + height),
                        ),
                        egui::vec2(unit * 0.09, unit * height),
                    ),
                    unit * 0.012,
                    if index == 1 { base } else { mix_color(base, Color32::WHITE, 0.25) },
                );
            }
            painter.circle_filled(
                egui::pos2(slide.left() + unit * 0.075, slide.center().y - unit * 0.06),
                unit * 0.022,
                base,
            );
            let stand = egui::Stroke::new(unit * 0.018, cream);
            painter.line_segment(
                [
                    egui::pos2(tile.center().x, slide.bottom()),
                    egui::pos2(tile.center().x, tile.bottom() - unit * 0.10),
                ],
                stand,
            );
            painter.line_segment(
                [
                    egui::pos2(tile.center().x - unit * 0.10, tile.bottom() - unit * 0.10),
                    egui::pos2(tile.center().x + unit * 0.10, tile.bottom() - unit * 0.10),
                ],
                stand,
            );
        }
        FileIconKind::Font => {
            let tile = draw_tile(painter, rect, base);
            painter.text(
                tile.center() - egui::vec2(0.0, unit * 0.010),
                Align2::CENTER_CENTER,
                "Aa",
                FontId::proportional(unit * 0.36),
                cream,
            );
            painter.line_segment(
                [
                    egui::pos2(tile.left() + unit * 0.16, tile.center().y + unit * 0.17),
                    egui::pos2(tile.right() - unit * 0.16, tile.center().y + unit * 0.17),
                ],
                egui::Stroke::new(unit * 0.015, alpha(cream, 90)),
            );
            painter.text(
                egui::pos2(tile.right() - unit * 0.13, tile.top() + unit * 0.14),
                Align2::CENTER_CENTER,
                "12",
                FontId::monospace(unit * 0.075),
                alpha(cream, 150),
            );
        }
        FileIconKind::Application => draw_application_tile(painter, rect),
        FileIconKind::Android => draw_android_tile(painter, rect),
        FileIconKind::Package => {
            let tile = draw_tile(painter, rect, base);
            let center = tile.center();
            let top = center - egui::vec2(0.0, unit * 0.22);
            let left = egui::pos2(center.x - unit * 0.29, center.y - unit * 0.10);
            let right = egui::pos2(center.x + unit * 0.29, center.y - unit * 0.10);
            let front = center + egui::vec2(0.0, unit * 0.02);
            let lower = egui::pos2(center.x, center.y + unit * 0.31);
            let bottom_left = egui::pos2(left.x, center.y + unit * 0.23);
            let bottom_right = egui::pos2(right.x, center.y + unit * 0.23);
            painter.add(egui::Shape::convex_polygon(
                vec![top, right, front, left],
                mix_color(base, Color32::WHITE, 0.24),
                egui::Stroke::NONE,
            ));
            painter.add(egui::Shape::convex_polygon(
                vec![left, front, lower, bottom_left],
                darken(base, 0.86),
                egui::Stroke::NONE,
            ));
            painter.add(egui::Shape::convex_polygon(
                vec![front, right, bottom_right, lower],
                darken(base, 0.96),
                egui::Stroke::NONE,
            ));
            let tape = alpha(cream, 215);
            painter.line_segment([top, front], egui::Stroke::new(unit * 0.080, tape));
            painter.line_segment([front, lower], egui::Stroke::new(unit * 0.070, tape));
        }
        FileIconKind::Disk => {
            let tile = draw_tile(painter, rect, base);
            let body = egui::Rect::from_center_size(
                tile.center() - egui::vec2(0.0, unit * 0.010),
                egui::vec2(unit * 0.62, unit * 0.46),
            );
            painter.rect_filled(body, unit * 0.06, cream);
            let platter = egui::pos2(body.left() + unit * 0.21, body.center().y);
            painter.circle_filled(platter, unit * 0.14, base);
            painter.circle_stroke(
                platter,
                unit * 0.10,
                egui::Stroke::new(unit * 0.012, alpha(cream, 190)),
            );
            painter.circle_filled(platter, unit * 0.024, cream);
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(body.right() - unit * 0.085, body.top() + unit * 0.075),
                    egui::pos2(platter.x + unit * 0.085, platter.y - unit * 0.055),
                    egui::pos2(platter.x + unit * 0.030, platter.y + unit * 0.015),
                ],
                base,
                egui::Stroke::NONE,
            ));
            painter.circle_filled(
                egui::pos2(body.right() - unit * 0.060, body.bottom() - unit * 0.060),
                unit * 0.018,
                Color32::from_rgb(98, 200, 120),
            );
        }
        FileIconKind::Database => {
            let tile = draw_tile(painter, rect, base);
            let body = egui::Rect::from_center_size(
                tile.center(),
                egui::vec2(unit * 0.50, unit * 0.42),
            );
            painter.rect_filled(body, unit * 0.16, cream);
            for index in 0..3 {
                let y = body.top() + unit * (0.075 + index as f32 * 0.135);
                draw_ellipse_outline(
                    painter,
                    egui::pos2(tile.center().x, y),
                    egui::vec2(unit * 0.245, unit * 0.075),
                    egui::Stroke::new(unit * 0.026, alpha(base, 120)),
                );
            }
            painter.circle_filled(
                egui::pos2(tile.center().x, body.top() + unit * 0.075),
                unit * 0.020,
                base,
            );
        }
        FileIconKind::Ebook => {
            let tile = draw_tile(painter, rect, base);
            let center = tile.center();
            let left_page = vec![
                center,
                center + egui::vec2(-unit * 0.035, -unit * 0.24),
                center + egui::vec2(-unit * 0.30, -unit * 0.155),
                center + egui::vec2(-unit * 0.30, unit * 0.22),
            ];
            let right_page = vec![
                center,
                center + egui::vec2(unit * 0.035, -unit * 0.24),
                center + egui::vec2(unit * 0.30, -unit * 0.155),
                center + egui::vec2(unit * 0.30, unit * 0.22),
            ];
            painter.add(egui::Shape::convex_polygon(
                left_page,
                cream,
                egui::Stroke::NONE,
            ));
            painter.add(egui::Shape::convex_polygon(
                right_page,
                Color32::from_rgb(226, 226, 220),
                egui::Stroke::NONE,
            ));
            for offset in [0.02_f32, 0.11] {
                painter.line_segment(
                    [
                        center + egui::vec2(-unit * 0.24, unit * offset),
                        center + egui::vec2(-unit * 0.06, unit * (offset - 0.025)),
                    ],
                    egui::Stroke::new(unit * 0.015, alpha(base, 100)),
                );
                painter.line_segment(
                    [
                        center + egui::vec2(unit * 0.06, unit * (offset - 0.025)),
                        center + egui::vec2(unit * 0.24, unit * offset),
                    ],
                    egui::Stroke::new(unit * 0.015, alpha(base, 100)),
                );
            }
            let ribbon_top = center.y - unit * 0.235;
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(center.x - unit * 0.030, ribbon_top),
                    egui::pos2(center.x + unit * 0.030, ribbon_top),
                    egui::pos2(center.x + unit * 0.030, ribbon_top + unit * 0.15),
                    egui::pos2(center.x, ribbon_top + unit * 0.115),
                    egui::pos2(center.x - unit * 0.030, ribbon_top + unit * 0.15),
                ],
                Color32::from_rgb(214, 92, 92),
                egui::Stroke::NONE,
            ));
        }
        FileIconKind::Notebook => {
            let tile = draw_tile(painter, rect, base);
            let page = egui::Rect::from_center_size(
                tile.center() - egui::vec2(unit * 0.010, unit * 0.0),
                egui::vec2(unit * 0.52, unit * 0.60),
            );
            painter.rect_filled(page, unit * 0.05, cream);
            painter.rect_filled(
                egui::Rect::from_min_size(page.left_top(), egui::vec2(unit * 0.085, page.height())),
                egui::Rounding {
                    nw: unit * 0.05,
                    ne: 0.0,
                    sw: unit * 0.05,
                    se: 0.0,
                },
                darken(base, 0.84),
            );
            for index in 0..4 {
                let y = page.top() + unit * (0.13 + index as f32 * 0.115);
                painter.circle_filled(
                    egui::pos2(page.left() + unit * 0.040, y),
                    unit * 0.024,
                    cream,
                );
                painter.circle_filled(
                    egui::pos2(page.left() + unit * 0.040, y),
                    unit * 0.011,
                    darken(base, 0.70),
                );
                painter.rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(page.left() + unit * 0.155, y - unit * 0.012),
                        egui::vec2(page.width() - unit * 0.21, unit * 0.024),
                    ),
                    unit * 0.012,
                    alpha(base, 80),
                );
            }
            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(page.right() - unit * 0.035, page.center().y),
                    egui::vec2(unit * 0.070, unit * 0.11),
                ),
                unit * 0.012,
                Color32::from_rgb(214, 92, 92),
            );
        }
        FileIconKind::Model3d => {
            let tile = draw_tile(painter, rect, base);
            let center = tile.center();
            let top = center - egui::vec2(0.0, unit * 0.28);
            let left = center - egui::vec2(unit * 0.27, unit * 0.14);
            let right = center + egui::vec2(unit * 0.27, -unit * 0.14);
            let middle = center + egui::vec2(0.0, unit * 0.02);
            let lower = center + egui::vec2(0.0, unit * 0.30);
            let left_bottom = center + egui::vec2(-unit * 0.27, unit * 0.16);
            let right_bottom = center + egui::vec2(unit * 0.27, unit * 0.16);
            let edge = egui::Stroke::new(unit * 0.016, alpha(cream, 210));
            painter.add(egui::Shape::convex_polygon(
                vec![top, right, middle, left],
                mix_color(base, Color32::WHITE, 0.20),
                edge,
            ));
            painter.add(egui::Shape::convex_polygon(
                vec![left, middle, lower, left_bottom],
                darken(base, 0.84),
                edge,
            ));
            painter.add(egui::Shape::convex_polygon(
                vec![middle, right, right_bottom, lower],
                darken(base, 0.94),
                edge,
            ));
            for point in [top, left, right, middle] {
                painter.circle_filled(point, unit * 0.018, cream);
            }
        }
    }
}

fn darken(c: Color32, factor: f32) -> Color32 {
    Color32::from_rgb(
        (c.r() as f32 * factor) as u8,
        (c.g() as f32 * factor) as u8,
        (c.b() as f32 * factor) as u8,
    )
}

fn name_lines(name: &str, max: usize) -> (String, Option<String>) {
    let chars: Vec<char> = name.chars().collect();
    if chars.len() <= max {
        return (name.to_string(), None);
    }
    let mut cut = max;
    for i in (max / 2..max).rev() {
        if matches!(chars[i], ' ' | '.' | '-' | '_') {
            cut = i + 1;
            break;
        }
    }
    let first: String = chars[..cut].iter().collect();
    let rest: String = chars[cut..].iter().collect();
    let rest = truncate(&rest, max);
    (first.trim_end().to_string(), Some(rest))
}

fn truncate(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn open_parent(p: &str) {
    if let Some(d) = Path::new(p).parent() {
        let _ = fileops::open_external(d);
    }
}

fn preview_extension(path: &Path) -> String {
    path.extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

fn is_preview_gif(path: &Path) -> bool {
    preview_extension(path) == "gif"
}

fn is_preview_video(path: &Path) -> bool {
    matches!(
        preview_extension(path).as_str(),
        "mp4" | "mkv" | "mov" | "avi" | "webm" | "m4v" | "mpeg" | "mpg" | "wmv" | "flv" | "ogv"
    )
}

fn is_preview_image(path: &Path) -> bool {
    matches!(
        preview_extension(path).as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "bmp" | "tif" | "tiff" | "avif" | "heic"
    )
}

fn read_text_preview(path: &Path, max_bytes: usize) -> Option<String> {
    let mut buf = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(max_bytes as u64 + 1)
        .read_to_end(&mut buf)
        .ok()?;
    let truncated = buf.len() > max_bytes;
    buf.truncate(max_bytes);
    if buf.contains(&0) {
        return None;
    }
    let mut text = match String::from_utf8(buf) {
        Ok(text) => text,
        Err(error) if truncated => {
            let valid = error.utf8_error().valid_up_to();
            String::from_utf8(error.into_bytes()[..valid].to_vec()).ok()?
        }
        Err(_) => return None,
    };
    if text.starts_with('\u{feff}') {
        text.remove(0);
    }
    let unusual_controls = text
        .chars()
        .filter(|character| {
            character.is_control() && !matches!(character, '\n' | '\r' | '\t' | '\u{0c}')
        })
        .count();
    if unusual_controls > text.chars().count() / 100 + 2 {
        return None;
    }
    if truncated {
        text.push_str("\n\n[Prévia limitada ao início do arquivo]");
    }
    Some(text)
}

fn hash_hue(s: &str) -> f32 {
    let mut h: u32 = 2166136261;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    (h % 360) as f32 / 360.0
}

fn hsv(h: f32, s: f32, v: f32) -> Color32 {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    let (r, g, b) = match (i as i32) % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

fn color_for(name: &str, is_dir: bool) -> Color32 {
    if is_dir {
        hsv(hash_hue(name), 0.55, 0.55)
    } else {
        let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
        hsv(hash_hue(ext), 0.30, 0.45)
    }
}

fn lighten(c: Color32) -> Color32 {
    Color32::from_rgb(
        c.r().saturating_add(35),
        c.g().saturating_add(35),
        c.b().saturating_add(35),
    )
}
