//! Ícones de arquivos e pastas desenhados com o painter do egui.
//!
//! Cada tipo de arquivo vira uma "folha" colorida com canto dobrado, um
//! desenho próprio (glifo) e uma etiqueta com a extensão. Pastas especiais
//! (Downloads, Imagens, src...) ganham cor e glifo na frente. Em tamanhos
//! pequenos (listas) a etiqueta some e o glifo cresce para continuar legível.

use super::{darken, hash_hue, hsv, Entry};
use egui::{pos2, vec2, Align2, Color32, FontId, Painter, Pos2, Rect, Rounding, Shape, Stroke};
use std::f32::consts::{FRAC_PI_2, FRAC_PI_3, PI, TAU};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    // mídia
    Image,
    RawPhoto,
    Vector,
    Video,
    Audio,
    Playlist,
    Midi,
    Subtitle,
    // código
    Code,
    Python,
    Rust,
    Javascript,
    Typescript,
    C,
    Cpp,
    CSharp,
    Jvm,
    Go,
    Shell,
    Web,
    Style,
    Php,
    Ruby,
    Swift,
    Dart,
    RLang,
    Haskell,
    Elixir,
    Lua,
    // dados
    Json,
    Config,
    Log,
    Database,
    Notebook,
    // documentos
    Text,
    Markdown,
    Document,
    Pdf,
    Spreadsheet,
    Presentation,
    Ebook,
    Calendar,
    Contact,
    Email,
    // sistema
    Archive,
    Package,
    Apk,
    Disk,
    Application,
    Font,
    Model3d,
    Game,
    Key,
    Download,
    // arquivos de projeto reconhecidos pelo nome
    Git,
    Docker,
    Node,
    Cargo,
    CMake,
    Make,
    Readme,
    License,
    Generic,
}

fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

fn lighten(c: Color32, amount: u8) -> Color32 {
    Color32::from_rgb(
        c.r().saturating_add(amount),
        c.g().saturating_add(amount),
        c.b().saturating_add(amount),
    )
}

/// (cor da folha, cor do desenho)
fn palette(kind: Kind) -> (Color32, Color32) {
    let (b, i) = match kind {
        Kind::Image => ((30, 133, 111), (170, 255, 215)),
        Kind::RawPhoto => ((60, 90, 120), (190, 225, 255)),
        Kind::Vector => ((200, 90, 130), (255, 205, 225)),
        Kind::Video => ((196, 75, 70), (255, 200, 160)),
        Kind::Audio => ((115, 77, 185), (228, 200, 255)),
        Kind::Playlist => ((140, 70, 170), (235, 200, 255)),
        Kind::Midi => ((80, 80, 150), (210, 215, 255)),
        Kind::Subtitle => ((70, 110, 130), (200, 235, 250)),
        Kind::Code => ((36, 111, 157), (160, 230, 255)),
        Kind::Python => ((45, 100, 150), (200, 230, 255)),
        Kind::Rust => ((190, 85, 45), (255, 214, 190)),
        Kind::Javascript => ((170, 140, 30), (255, 245, 200)),
        Kind::Typescript => ((40, 100, 170), (205, 230, 255)),
        Kind::C => ((60, 120, 180), (215, 238, 255)),
        Kind::Cpp => ((0, 89, 156), (190, 225, 255)),
        Kind::CSharp => ((104, 33, 122), (230, 200, 245)),
        Kind::Jvm => ((176, 114, 25), (255, 225, 180)),
        Kind::Go => ((0, 145, 160), (190, 245, 250)),
        Kind::Shell => ((60, 70, 64), (150, 240, 140)),
        Kind::Web => ((199, 98, 40), (255, 220, 190)),
        Kind::Style => ((40, 120, 190), (200, 230, 255)),
        Kind::Php => ((115, 119, 173), (225, 226, 255)),
        Kind::Ruby => ((190, 45, 45), (255, 205, 205)),
        Kind::Swift => ((230, 80, 50), (255, 215, 200)),
        Kind::Dart => ((1, 117, 194), (200, 235, 255)),
        Kind::RLang => ((39, 109, 195), (205, 230, 255)),
        Kind::Haskell => ((94, 80, 134), (225, 215, 250)),
        Kind::Elixir => ((110, 74, 126), (235, 215, 245)),
        Kind::Lua => ((39, 61, 110), (121, 177, 255)),
        Kind::Json => ((150, 130, 40), (255, 240, 170)),
        Kind::Config => ((90, 100, 115), (200, 215, 235)),
        Kind::Log => ((95, 105, 95), (215, 230, 215)),
        Kind::Database => ((48, 117, 146), (168, 229, 255)),
        Kind::Notebook => ((200, 130, 45), (255, 225, 160)),
        Kind::Text => ((95, 110, 130), (220, 230, 245)),
        Kind::Markdown => ((70, 90, 120), (210, 225, 250)),
        Kind::Document => ((58, 112, 190), (188, 219, 255)),
        Kind::Pdf => ((181, 48, 57), (255, 193, 194)),
        Kind::Spreadsheet => ((35, 133, 87), (170, 247, 190)),
        Kind::Presentation => ((207, 100, 54), (255, 208, 153)),
        Kind::Ebook => ((166, 83, 143), (255, 199, 236)),
        Kind::Calendar => ((60, 120, 170), (200, 230, 255)),
        Kind::Contact => ((80, 120, 150), (210, 235, 250)),
        Kind::Email => ((70, 110, 160), (210, 228, 255)),
        Kind::Archive => ((183, 119, 43), (255, 220, 143)),
        Kind::Package => ((190, 83, 123), (255, 187, 217)),
        Kind::Apk => ((46, 140, 86), (164, 244, 190)),
        Kind::Disk => ((65, 103, 140), (177, 220, 255)),
        Kind::Application => ((38, 125, 151), (157, 241, 238)),
        Kind::Font => ((82, 91, 166), (211, 210, 255)),
        Kind::Model3d => ((91, 131, 79), (210, 248, 158)),
        Kind::Game => ((170, 70, 140), (255, 200, 240)),
        Kind::Key => ((160, 130, 40), (255, 235, 150)),
        Kind::Download => ((50, 140, 110), (180, 250, 220)),
        Kind::Git => ((220, 75, 45), (255, 215, 205)),
        Kind::Docker => ((30, 130, 215), (200, 235, 255)),
        Kind::Node => ((80, 145, 70), (210, 250, 190)),
        Kind::Cargo => ((190, 85, 45), (255, 214, 190)),
        Kind::CMake => ((6, 79, 140), (190, 225, 255)),
        Kind::Make => ((100, 118, 124), (220, 235, 238)),
        Kind::Readme => ((74, 125, 181), (215, 232, 255)),
        Kind::License => ((160, 122, 44), (255, 235, 170)),
        Kind::Generic => ((90, 100, 120), (238, 241, 248)),
    };
    (rgb(b.0, b.1, b.2), rgb(i.0, i.1, i.2))
}

/// Arquivos reconhecidos pelo nome (têm prioridade sobre a extensão).
fn special_kind(name: &str) -> Option<Kind> {
    if matches!(name, ".gitignore" | ".gitmodules" | ".gitattributes") {
        return Some(Kind::Git);
    }
    if matches!(
        name,
        "package.json" | "package-lock.json" | "yarn.lock" | "pnpm-lock.yaml"
    ) {
        return Some(Kind::Node);
    }
    if matches!(name, "cargo.toml" | "cargo.lock") {
        return Some(Kind::Cargo);
    }
    if name == "dockerfile"
        || name.starts_with("dockerfile.")
        || name.ends_with(".dockerfile")
        || matches!(
            name,
            "docker-compose.yml" | "docker-compose.yaml" | "compose.yml" | "compose.yaml"
        )
    {
        return Some(Kind::Docker);
    }
    if name == "cmakelists.txt" || name.ends_with(".cmake") {
        return Some(Kind::CMake);
    }
    if matches!(name, "makefile" | "gnumakefile" | "justfile") {
        return Some(Kind::Make);
    }
    if name.starts_with("readme")
        || name.starts_with("changelog")
        || name.starts_with("contributing")
        || name.starts_with("install")
        || name.starts_with("authors")
    {
        return Some(Kind::Readme);
    }
    if name.starts_with("license")
        || name.starts_with("licence")
        || name.starts_with("copying")
        || name.starts_with("notice")
        || name == "unlicense"
    {
        return Some(Kind::License);
    }
    None
}

fn classify(ext: &str) -> Kind {
    match ext {
        "png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif" | "heic" | "heif" | "tif" | "tiff"
        | "avif" | "ico" | "icns" | "xcf" | "psd" | "kra" => Kind::Image,
        "raw" | "arw" | "cr2" | "cr3" | "nef" | "dng" | "orf" | "rw2" => Kind::RawPhoto,
        "svg" | "ai" | "eps" | "fig" | "sketch" | "cdr" | "xd" => Kind::Vector,
        "mp4" | "mkv" | "mov" | "avi" | "webm" | "m4v" | "mpeg" | "mpg" | "wmv" | "flv"
        | "ogv" | "3gp" => Kind::Video,
        "mp3" | "wav" | "flac" | "ogg" | "oga" | "opus" | "m4a" | "aac" | "wma" | "aiff"
        | "ape" | "mka" => Kind::Audio,
        "m3u" | "m3u8" | "pls" | "xspf" | "cue" => Kind::Playlist,
        "mid" | "midi" | "kar" => Kind::Midi,
        "srt" | "vtt" | "ass" | "ssa" | "sub" => Kind::Subtitle,

        "py" | "pyi" | "pyw" | "pyc" => Kind::Python,
        "rs" => Kind::Rust,
        "js" | "mjs" | "cjs" | "jsx" => Kind::Javascript,
        "ts" | "tsx" | "mts" | "cts" => Kind::Typescript,
        "c" | "h" => Kind::C,
        "cpp" | "cxx" | "cc" | "hpp" | "hxx" | "hh" | "inl" => Kind::Cpp,
        "cs" | "csx" => Kind::CSharp,
        "java" | "kt" | "kts" | "scala" | "groovy" | "jar" | "class" => Kind::Jvm,
        "go" => Kind::Go,
        "sh" | "bash" | "zsh" | "fish" | "bat" | "cmd" | "ps1" => Kind::Shell,
        "html" | "htm" | "xhtml" | "vue" | "svelte" => Kind::Web,
        "css" | "scss" | "sass" | "less" => Kind::Style,
        "php" => Kind::Php,
        "rb" | "erb" | "gemspec" => Kind::Ruby,
        "swift" => Kind::Swift,
        "dart" => Kind::Dart,
        "r" => Kind::RLang,
        "hs" | "lhs" => Kind::Haskell,
        "ex" | "exs" => Kind::Elixir,
        "lua" => Kind::Lua,
        "xml" | "xsl" | "wasm" | "wat" | "vim" | "el" | "clj" | "pl" | "pm" | "jl" | "nim"
        | "zig" | "asm" | "s" | "diff" | "patch" | "gradle" | "proto" | "graphql" | "gql"
        | "vb" | "f90" | "m" | "mm" | "v" | "sv" | "vhd" => Kind::Code,

        "json" | "jsonc" | "jsonl" | "ndjson" => Kind::Json,
        "toml" | "yaml" | "yml" | "ini" | "conf" | "cfg" | "env" | "properties" | "lock"
        | "service" | "rc" | "editorconfig" | "plist" => Kind::Config,
        "log" | "out" | "err" | "trace" => Kind::Log,
        "db" | "sqlite" | "sqlite3" | "mdb" | "accdb" | "realm" | "sql" | "dump" | "parquet" => {
            Kind::Database
        }
        "ipynb" | "qmd" | "rmd" => Kind::Notebook,

        "txt" | "text" | "rtf" | "nfo" => Kind::Text,
        "md" | "markdown" | "mdx" | "rst" | "adoc" | "org" => Kind::Markdown,
        "doc" | "docx" | "odt" | "pages" | "wps" | "tex" | "dotx" | "wpd" => Kind::Document,
        "pdf" => Kind::Pdf,
        "csv" | "tsv" | "xls" | "xlsx" | "xlsm" | "ods" | "numbers" => Kind::Spreadsheet,
        "ppt" | "pptx" | "odp" | "pps" | "ppsx" => Kind::Presentation,
        "epub" | "mobi" | "azw" | "azw3" | "djvu" | "djv" | "cbz" | "cbr" | "fb2" => Kind::Ebook,
        "ics" | "ical" => Kind::Calendar,
        "vcf" | "vcard" => Kind::Contact,
        "eml" | "msg" | "mbox" | "emlx" => Kind::Email,

        "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "7z" | "rar" | "lz" | "lzma"
        | "cab" | "z" => Kind::Archive,
        "deb" | "rpm" | "pkg" | "flatpak" | "snap" | "whl" | "gem" | "crate"
        | "nupkg" | "xpi" | "vsix" => Kind::Package,
        "apk" | "aab" | "xapk" => Kind::Apk,
        "iso" | "img" | "dmg" | "vdi" | "vmdk" | "qcow2" | "vhdx" => Kind::Disk,
        "appimage" | "desktop" | "exe" | "msi" | "app" | "run" | "bin" => Kind::Application,
        "ttf" | "otf" | "woff" | "woff2" | "eot" | "fon" => Kind::Font,
        "obj" | "fbx" | "stl" | "gltf" | "glb" | "blend" | "dae" | "3mf" | "ply" | "usd"
        | "usdz" | "step" | "stp" | "dxf" | "dwg" => Kind::Model3d,
        "nes" | "sfc" | "smc" | "gb" | "gbc" | "gba" | "nds" | "3ds" | "cia" | "cxi" | "cci"
        | "z64" | "n64" | "v64" | "sav" | "srm" | "rom" | "wad" | "xci" | "nsp" | "gcm"
        | "rvz" | "pak" | "vpk" | "smd" | "gen" | "pck" => Kind::Game,
        "pem" | "crt" | "cer" | "key" | "pub" | "gpg" | "asc" | "pfx" | "p12" | "kdbx" => {
            Kind::Key
        }
        "torrent" | "part" | "crdownload" | "download" | "opdownload" => Kind::Download,
        _ => Kind::Generic,
    }
}

/// Pastas conhecidas ganham cor própria e um glifo na frente.
fn folder_style(name: &str) -> Option<(Color32, Kind)> {
    let style = match name {
        "downloads" | "transferências" | "transferencias" => (rgb(76, 160, 96), Kind::Download),
        "documents" | "documentos" | "docs" => (rgb(80, 130, 200), Kind::Document),
        "pictures" | "imagens" | "fotos" | "photos" | "screenshots" => {
            (rgb(60, 160, 160), Kind::Image)
        }
        "music" | "música" | "musica" | "músicas" | "musicas" => (rgb(150, 100, 200), Kind::Audio),
        "videos" | "vídeos" | "movies" | "filmes" => (rgb(200, 90, 90), Kind::Video),
        "desktop" | "área de trabalho" | "area de trabalho" => {
            (rgb(110, 125, 150), Kind::Application)
        }
        "src" | "source" | "code" | "projects" | "projetos" | "dev" => {
            (rgb(214, 130, 60), Kind::Code)
        }
        ".config" | "config" | "configs" => (rgb(120, 130, 140), Kind::Config),
        "games" | "jogos" | "roms" => (rgb(180, 90, 150), Kind::Game),
        "backup" | "backups" => (rgb(170, 140, 70), Kind::Archive),
        "bin" | "scripts" => (rgb(90, 110, 100), Kind::Shell),
        ".git" | ".github" | ".gitlab" => (rgb(220, 90, 60), Kind::Git),
        "node_modules" => (rgb(90, 150, 80), Kind::Node),
        _ => return None,
    };
    Some(style)
}

fn tag_text(kind: Kind, name: &str, ext: &str) -> String {
    match kind {
        Kind::Git => "GIT".into(),
        Kind::Docker => "DOCKER".into(),
        Kind::Node => "NODE".into(),
        Kind::Cargo => "CARGO".into(),
        Kind::CMake => "CMAKE".into(),
        Kind::Make => "MAKE".into(),
        Kind::Readme => "README".into(),
        Kind::License => "LIC".into(),
        _ => {
            if name.ends_with(".tar.gz") {
                "TAR.GZ".into()
            } else {
                ext.to_uppercase()
            }
        }
    }
}

// ───────────────────────── ponto de entrada ─────────────────────────

pub(super) fn draw_entry_icon(
    painter: &Painter,
    rect: Rect,
    e: &Entry,
    accent: Color32,
    custom_color: bool,
) {
    let name = e.name.to_lowercase();
    if e.is_dir {
        // cor escolhida pelo usuário para a pasta sempre vence a cor "especial"
        let tinted = Color32::from_rgb(
            accent.r().saturating_add(35),
            accent.g().saturating_add(18),
            accent.b().saturating_add(6),
        );
        let (color, kind) = match folder_style(&name) {
            Some((special, kind)) => (if custom_color { tinted } else { special }, Some(kind)),
            None => (tinted, None),
        };
        draw_folder(painter, rect, color, kind);
        return;
    }
    let ext = e
        .path
        .extension()
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let kind = match special_kind(&name) {
        Some(kind) => kind,
        None => classify(&ext),
    };
    // Mantém o ícone oficial (malha) de Swift, que é melhor.
    if kind == Kind::Swift {
        super::draw_swift_tile(painter, rect);
        return;
    }
    let (base, ink) = if kind == Kind::Generic {
        let key = if ext.is_empty() {
            e.name.as_str()
        } else {
            ext.as_str()
        };
        (hsv(hash_hue(key), 0.36, 0.62), rgb(238, 241, 248))
    } else {
        palette(kind)
    };
    let tag = tag_text(kind, &name, &ext);
    draw_sheet(painter, rect, kind, base, ink, &tag);
}

// ───────────────────────── folha de papel ─────────────────────────

fn arc(points: &mut Vec<Pos2>, center: Pos2, radius: f32, from: f32, to: f32, steps: usize) {
    for index in 0..=steps {
        let angle = from + (to - from) * index as f32 / steps as f32;
        points.push(pos2(
            center.x + angle.cos() * radius,
            center.y + angle.sin() * radius,
        ));
    }
}

/// Retângulo arredondado com o canto superior direito cortado (convexo).
fn sheet_points(r: Rect, fold: f32, radius: f32) -> Vec<Pos2> {
    let mut points = Vec::with_capacity(24);
    arc(
        &mut points,
        pos2(r.left() + radius, r.top() + radius),
        radius,
        PI,
        1.5 * PI,
        4,
    );
    points.push(pos2(r.right() - fold, r.top()));
    points.push(pos2(r.right(), r.top() + fold));
    arc(
        &mut points,
        pos2(r.right() - radius, r.bottom() - radius),
        radius,
        0.0,
        FRAC_PI_2,
        4,
    );
    arc(
        &mut points,
        pos2(r.left() + radius, r.bottom() - radius),
        radius,
        FRAC_PI_2,
        PI,
        4,
    );
    points
}

fn draw_sheet(painter: &Painter, rect: Rect, kind: Kind, base: Color32, ink: Color32, tag: &str) {
    let unit = rect.width().min(rect.height());
    if unit < 4.0 {
        return;
    }
    let detailed = unit >= 36.0;
    let center = rect.center();
    let sheet = Rect::from_center_size(center, vec2(unit * 0.68, unit * 0.86));
    let fold = unit * 0.20;
    let outline = Stroke::new((unit * 0.018).clamp(0.8, 2.0), darken(base, 0.6));
    let points = sheet_points(sheet, fold, unit * 0.07);

    let shadow: Vec<Pos2> = points
        .iter()
        .map(|point| *point + vec2(0.0, unit * 0.03))
        .collect();
    painter.add(Shape::convex_polygon(
        shadow,
        Color32::from_black_alpha(70),
        Stroke::NONE,
    ));
    painter.add(Shape::convex_polygon(points, base, outline));
    // canto dobrado
    painter.add(Shape::convex_polygon(
        vec![
            pos2(sheet.right() - fold, sheet.top()),
            pos2(sheet.right(), sheet.top() + fold),
            pos2(sheet.right() - fold, sheet.top() + fold),
        ],
        lighten(base, 40),
        outline,
    ));

    let (size, glyph_y) = if detailed {
        (unit * 0.36, sheet.top() + sheet.height() * 0.45)
    } else {
        (unit * 0.46, center.y + unit * 0.04)
    };
    let glyph_rect = Rect::from_center_size(pos2(center.x, glyph_y), vec2(size, size));
    glyph(
        &G {
            p: painter,
            r: glyph_rect,
            ink,
            base,
        },
        kind,
    );

    if detailed && !tag.is_empty() {
        draw_tag(painter, sheet, unit, tag, base);
    }
}

fn draw_tag(painter: &Painter, sheet: Rect, unit: f32, tag: &str, base: Color32) {
    let text: String = tag.chars().take(6).collect::<String>().to_uppercase();
    if text.is_empty() {
        return;
    }
    let height = unit * 0.15;
    let width = (unit * 0.085 * text.chars().count() as f32 + unit * 0.12)
        .clamp(unit * 0.28, sheet.width() - unit * 0.08);
    let pill = Rect::from_center_size(
        pos2(sheet.center().x, sheet.bottom() - height * 0.95),
        vec2(width, height),
    );
    painter.rect_filled(pill, Rounding::same(height * 0.5), darken(base, 0.55));
    painter.text(
        pill.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::monospace(height * 0.66),
        rgb(245, 247, 250),
    );
}

// ───────────────────────── pastas ─────────────────────────

fn draw_folder(painter: &Painter, rect: Rect, color: Color32, glyph_kind: Option<Kind>) {
    let unit = rect.width().min(rect.height());
    if unit < 4.0 {
        return;
    }
    let origin = rect.center() - vec2(unit * 0.5, unit * 0.5);
    let at = |x: f32, y: f32| pos2(origin.x + x * unit, origin.y + y * unit);
    let area = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(at(x0, y0), at(x1, y1));

    let back = darken(color, 0.66);
    let deep = darken(color, 0.82);
    let shine = lighten(color, 60);

    // sombra
    painter.rect_filled(
        area(0.06, 0.40, 0.94, 0.82)
            .translate(vec2(0.0, unit * 0.03))
            .expand(unit * 0.012),
        Rounding::same(unit * 0.09),
        Color32::from_black_alpha(70),
    );
    // aba e painel de trás
    painter.rect_filled(area(0.06, 0.15, 0.46, 0.34), Rounding::same(unit * 0.06), back);
    painter.rect_filled(area(0.06, 0.24, 0.94, 0.76), Rounding::same(unit * 0.07), back);

    // duas folhas aparecendo por trás (somem em tamanhos pequenos)
    if unit >= 24.0 {
        painter.rect_filled(
            area(0.17, 0.20, 0.88, 0.58),
            Rounding::same(unit * 0.03),
            Color32::from_rgb(205, 214, 228),
        );
        painter.rect_filled(
            area(0.12, 0.26, 0.84, 0.60),
            Rounding::same(unit * 0.03),
            Color32::from_rgb(246, 248, 251),
        );
        if unit >= 40.0 {
            let line = Stroke::new(unit * 0.018, Color32::from_rgb(200, 208, 222));
            painter.line_segment([at(0.20, 0.34), at(0.60, 0.34)], line);
            painter.line_segment([at(0.20, 0.41), at(0.46, 0.41)], line);
        }
    }

    // painel da frente: corpo, faixa de baixo mais escura, brilho e friso no topo
    painter.rect_filled(area(0.04, 0.40, 0.96, 0.82), Rounding::same(unit * 0.09), color);
    painter.rect_filled(
        area(0.04, 0.72, 0.96, 0.82),
        Rounding {
            nw: 0.0,
            ne: 0.0,
            sw: unit * 0.09,
            se: unit * 0.09,
        },
        deep,
    );
    painter.rect_filled(
        area(0.08, 0.415, 0.92, 0.44),
        Rounding::same(unit * 0.012),
        shine,
    );

    // símbolo na frente (pastas especiais)
    if unit >= 28.0 {
        if let Some(kind) = glyph_kind {
            let lum = 0.299 * color.r() as f32 + 0.587 * color.g() as f32 + 0.114 * color.b() as f32;
            let ink = if lum > 170.0 {
                darken(color, 0.35)
            } else {
                rgb(250, 250, 252)
            };
            let size = unit * 0.28;
            let glyph_rect = Rect::from_center_size(at(0.5, 0.62), vec2(size, size));
            glyph(
                &G {
                    p: painter,
                    r: glyph_rect,
                    ink,
                    base: color,
                },
                kind,
            );
        }
    }
}

// ───────────────────────── glifos ─────────────────────────

/// Ajudante de desenho: coordenadas de 0 a 1 dentro do quadrado do glifo.
struct G<'a> {
    p: &'a Painter,
    r: Rect,
    ink: Color32,
    base: Color32,
}

impl<'a> G<'a> {
    fn s(&self) -> f32 {
        self.r.width()
    }

    fn pt(&self, x: f32, y: f32) -> Pos2 {
        pos2(
            self.r.left() + x * self.r.width(),
            self.r.top() + y * self.r.height(),
        )
    }

    fn stroke(&self, w: f32, color: Color32) -> Stroke {
        Stroke::new((w * self.s()).max(1.0), color)
    }

    fn line(&self, pts: &[(f32, f32)], w: f32, color: Color32) {
        let points: Vec<Pos2> = pts.iter().map(|&(x, y)| self.pt(x, y)).collect();
        self.p.add(Shape::line(points, self.stroke(w, color)));
    }

    fn seg(&self, a: (f32, f32), b: (f32, f32), w: f32, color: Color32) {
        self.p
            .line_segment([self.pt(a.0, a.1), self.pt(b.0, b.1)], self.stroke(w, color));
    }

    fn poly(&self, pts: &[(f32, f32)], fill: Color32) {
        let points: Vec<Pos2> = pts.iter().map(|&(x, y)| self.pt(x, y)).collect();
        self.p
            .add(Shape::convex_polygon(points, fill, Stroke::NONE));
    }

    fn rect(&self, a: (f32, f32), b: (f32, f32), round: f32, fill: Color32) {
        self.p.rect_filled(
            Rect::from_min_max(self.pt(a.0, a.1), self.pt(b.0, b.1)),
            Rounding::same(round * self.s()),
            fill,
        );
    }

    fn rect_line(&self, a: (f32, f32), b: (f32, f32), round: f32, w: f32, color: Color32) {
        self.p.rect_stroke(
            Rect::from_min_max(self.pt(a.0, a.1), self.pt(b.0, b.1)),
            Rounding::same(round * self.s()),
            self.stroke(w, color),
        );
    }

    fn dot(&self, c: (f32, f32), r: f32, fill: Color32) {
        self.p.circle_filled(self.pt(c.0, c.1), r * self.s(), fill);
    }

    fn ring(&self, c: (f32, f32), r: f32, w: f32, color: Color32) {
        self.p
            .circle_stroke(self.pt(c.0, c.1), r * self.s(), self.stroke(w, color));
    }

    fn text(&self, c: (f32, f32), t: &str, size: f32, color: Color32) {
        let px = size * self.s();
        if px < 5.0 {
            return;
        }
        self.p.text(
            self.pt(c.0, c.1),
            Align2::CENTER_CENTER,
            t,
            FontId::monospace(px),
            color,
        );
    }

    fn ellipse(&self, c: (f32, f32), rx: f32, ry: f32, fill: Color32) {
        let points: Vec<Pos2> = (0..32)
            .map(|index| {
                let angle = TAU * index as f32 / 32.0;
                self.pt(c.0 + angle.cos() * rx, c.1 + angle.sin() * ry)
            })
            .collect();
        self.p
            .add(Shape::convex_polygon(points, fill, Stroke::NONE));
    }

    fn arc_line(&self, c: (f32, f32), rx: f32, ry: f32, from: f32, to: f32, w: f32, color: Color32) {
        let pts: Vec<(f32, f32)> = (0..=16)
            .map(|index| {
                let angle = from + (to - from) * index as f32 / 16.0;
                (c.0 + angle.cos() * rx, c.1 + angle.sin() * ry)
            })
            .collect();
        self.line(&pts, w, color);
    }
}

fn hexagon(g: &G, radius: f32, fill: Color32) {
    let pts: Vec<(f32, f32)> = (0..6)
        .map(|index| {
            let angle = -FRAC_PI_2 + FRAC_PI_3 * index as f32;
            (0.5 + angle.cos() * radius, 0.5 + angle.sin() * radius)
        })
        .collect();
    g.poly(&pts, fill);
}

fn shield(g: &G, fill: Color32) {
    g.poly(
        &[
            (0.08, 0.14),
            (0.50, 0.02),
            (0.92, 0.14),
            (0.86, 0.72),
            (0.50, 0.98),
            (0.14, 0.72),
        ],
        fill,
    );
}

fn cube(g: &G) {
    let ink = g.ink;
    g.poly(
        &[(0.50, 0.02), (0.96, 0.26), (0.50, 0.50), (0.04, 0.26)],
        ink,
    );
    g.poly(
        &[(0.04, 0.26), (0.50, 0.50), (0.50, 0.98), (0.04, 0.74)],
        darken(ink, 0.78),
    );
    g.poly(
        &[(0.50, 0.50), (0.96, 0.26), (0.96, 0.74), (0.50, 0.98)],
        darken(ink, 0.62),
    );
    g.seg((0.50, 0.50), (0.50, 0.98), 0.04, g.base);
}

fn glyph(g: &G, kind: Kind) {
    let ink = g.ink;
    let base = g.base;
    match kind {
        // ── mídia ──
        Kind::Image => {
            g.rect_line((0.02, 0.10), (0.98, 0.90), 0.10, 0.07, ink);
            g.dot((0.72, 0.32), 0.09, ink);
            g.poly(&[(0.08, 0.84), (0.38, 0.42), (0.62, 0.84)], ink);
            g.poly(&[(0.46, 0.84), (0.70, 0.56), (0.92, 0.84)], darken(ink, 0.8));
        }
        Kind::RawPhoto => {
            g.rect((0.02, 0.26), (0.98, 0.90), 0.12, ink);
            g.poly(&[(0.28, 0.26), (0.36, 0.10), (0.64, 0.10), (0.72, 0.26)], ink);
            g.dot((0.50, 0.58), 0.25, base);
            g.dot((0.50, 0.58), 0.16, ink);
            g.dot((0.50, 0.58), 0.06, base);
            g.dot((0.86, 0.38), 0.04, base);
        }
        Kind::Vector => {
            let a = (0.12_f32, 0.82_f32);
            let c = (0.10_f32, 0.12_f32);
            let b = (0.88_f32, 0.22_f32);
            let pts: Vec<(f32, f32)> = (0..=20)
                .map(|index| {
                    let t = index as f32 / 20.0;
                    let u = 1.0 - t;
                    (
                        u * u * a.0 + 2.0 * u * t * c.0 + t * t * b.0,
                        u * u * a.1 + 2.0 * u * t * c.1 + t * t * b.1,
                    )
                })
                .collect();
            g.line(&pts, 0.07, ink);
            g.seg(a, c, 0.03, ink);
            g.seg(b, c, 0.03, ink);
            g.dot(c, 0.06, ink);
            g.rect((a.0 - 0.08, a.1 - 0.08), (a.0 + 0.08, a.1 + 0.08), 0.0, ink);
            g.rect((b.0 - 0.08, b.1 - 0.08), (b.0 + 0.08, b.1 + 0.08), 0.0, ink);
        }
        Kind::Video => {
            g.rect_line((0.02, 0.12), (0.98, 0.88), 0.10, 0.07, ink);
            g.poly(&[(0.40, 0.30), (0.40, 0.70), (0.72, 0.50)], ink);
            for index in 0..4 {
                let y = 0.22 + index as f32 * 0.19;
                g.rect((0.07, y), (0.14, y + 0.08), 0.02, ink);
                g.rect((0.86, y), (0.93, y + 0.08), 0.02, ink);
            }
        }
        Kind::Audio => {
            g.dot((0.26, 0.80), 0.14, ink);
            g.dot((0.74, 0.68), 0.14, ink);
            g.seg((0.38, 0.80), (0.38, 0.22), 0.07, ink);
            g.seg((0.86, 0.68), (0.86, 0.10), 0.07, ink);
            g.poly(&[(0.38, 0.22), (0.86, 0.10), (0.86, 0.28), (0.38, 0.40)], ink);
        }
        Kind::Playlist => {
            for (index, &len) in [0.62_f32, 0.62, 0.45].iter().enumerate() {
                let y = 0.20 + index as f32 * 0.26;
                g.seg((0.02, y), (0.02 + len, y), 0.07, ink);
            }
            g.dot((0.80, 0.76), 0.11, ink);
            g.seg((0.90, 0.76), (0.90, 0.32), 0.06, ink);
            g.seg((0.90, 0.32), (0.78, 0.38), 0.05, ink);
        }
        Kind::Midi => {
            g.rect_line((0.02, 0.14), (0.98, 0.88), 0.06, 0.06, ink);
            for index in 1..7 {
                let x = 0.02 + 0.96 * index as f32 / 7.0;
                g.seg((x, 0.14), (x, 0.88), 0.03, ink);
            }
            for &index in &[1_usize, 2, 4, 5, 6] {
                let x = 0.02 + 0.96 * index as f32 / 7.0;
                g.rect((x - 0.045, 0.14), (x + 0.045, 0.58), 0.02, ink);
            }
        }
        Kind::Subtitle => {
            g.rect_line((0.02, 0.12), (0.98, 0.74), 0.12, 0.07, ink);
            g.poly(&[(0.20, 0.74), (0.44, 0.74), (0.20, 0.94)], ink);
            g.seg((0.18, 0.36), (0.62, 0.36), 0.06, ink);
            g.seg((0.18, 0.52), (0.82, 0.52), 0.06, ink);
        }

        // ── código ──
        Kind::Code => {
            g.line(&[(0.30, 0.20), (0.06, 0.50), (0.30, 0.80)], 0.09, ink);
            g.line(&[(0.70, 0.20), (0.94, 0.50), (0.70, 0.80)], 0.09, ink);
            g.seg((0.58, 0.12), (0.42, 0.88), 0.07, ink);
        }
        Kind::Python => {
            let yellow = rgb(255, 212, 59);
            let gap = 0.03;
            // 1) lóbulo e cauda da cobra azul (o canto interno da cauda é reto)
            g.rect((0.017, 0.26), (0.38, 0.64), 0.12, ink);
            g.rect((0.017, 0.26), (0.23, 0.755), 0.12, ink);
            g.rect((0.125, 0.63), (0.23, 0.755), 0.0, ink);
            // 2) vão em volta do corpo amarelo (só para cima e para a esquerda)
            g.rect((0.255 - gap, 0.52 - gap), (0.74, 1.0), 0.12 + gap, base);
            // 3) cobra amarela: braço, cauda (canto interno reto) e corpo
            g.rect((0.62, 0.36), (0.983, 0.74), 0.12, yellow);
            g.rect((0.77, 0.245), (0.983, 0.74), 0.12, yellow);
            g.rect((0.77, 0.245), (0.875, 0.37), 0.0, yellow);
            g.rect((0.255, 0.52), (0.74, 1.0), 0.12, yellow);
            // 4) vão em volta do bloco azul (só para a direita e para baixo) e o bloco
            g.rect((0.26, 0.0), (0.745 + gap, 0.49 + gap), 0.12 + gap, base);
            g.rect((0.26, 0.0), (0.745, 0.49), 0.12, ink);
            // 5) bocas e olhos
            g.rect((0.26, 0.229), (0.50, 0.26), 0.0, base);
            g.rect((0.50, 0.74), (0.74, 0.771), 0.0, base);
            g.dot((0.375, 0.12), 0.04, base);
            g.dot((0.625, 0.88), 0.04, base);
        }
        Kind::Rust => {
            for index in 0..8 {
                let a = TAU * index as f32 / 8.0;
                let (dx, dy) = (a.cos(), a.sin());
                let (px, py) = (-dy, dx);
                g.poly(
                    &[
                        (0.5 + dx * 0.26 + px * 0.075, 0.5 + dy * 0.26 + py * 0.075),
                        (0.5 + dx * 0.47 + px * 0.075, 0.5 + dy * 0.47 + py * 0.075),
                        (0.5 + dx * 0.47 - px * 0.075, 0.5 + dy * 0.47 - py * 0.075),
                        (0.5 + dx * 0.26 - px * 0.075, 0.5 + dy * 0.26 - py * 0.075),
                    ],
                    ink,
                );
            }
            g.dot((0.5, 0.5), 0.34, ink);
            g.dot((0.5, 0.5), 0.19, base);
            g.text((0.5, 0.52), "R", 0.30, ink);
        }
        Kind::Javascript => {
            g.rect((0.04, 0.04), (0.96, 0.96), 0.10, rgb(247, 223, 30));
            g.text((0.66, 0.74), "JS", 0.40, rgb(50, 50, 40));
        }
        Kind::Typescript => {
            g.rect((0.04, 0.04), (0.96, 0.96), 0.10, rgb(49, 120, 198));
            g.text((0.66, 0.74), "TS", 0.40, rgb(255, 255, 255));
        }
        Kind::C => {
            hexagon(g, 0.47, ink);
            g.text((0.5, 0.52), "C", 0.55, base);
        }
        Kind::Cpp => {
            hexagon(g, 0.47, ink);
            g.text((0.5, 0.52), "C++", 0.32, base);
        }
        Kind::CSharp => {
            hexagon(g, 0.47, ink);
            g.text((0.5, 0.52), "C#", 0.42, base);
        }
        Kind::Jvm => {
            g.poly(&[(0.12, 0.38), (0.78, 0.38), (0.70, 0.84), (0.20, 0.84)], ink);
            g.arc_line((0.78, 0.58), 0.14, 0.14, -FRAC_PI_2, FRAC_PI_2, 0.06, ink);
            g.rect((0.08, 0.88), (0.84, 0.95), 0.03, ink);
            g.line(&[(0.32, 0.10), (0.40, 0.18), (0.32, 0.26), (0.40, 0.34)], 0.045, ink);
            g.line(&[(0.52, 0.10), (0.60, 0.18), (0.52, 0.26), (0.60, 0.34)], 0.045, ink);
        }
        Kind::Go => {
            g.text((0.62, 0.50), "GO", 0.46, ink);
            g.seg((0.02, 0.38), (0.20, 0.38), 0.05, ink);
            g.seg((0.00, 0.50), (0.22, 0.50), 0.05, ink);
            g.seg((0.04, 0.62), (0.20, 0.62), 0.05, ink);
        }
        Kind::Shell => {
            g.rect((0.02, 0.12), (0.98, 0.88), 0.10, darken(base, 0.45));
            g.rect_line((0.02, 0.12), (0.98, 0.88), 0.10, 0.05, ink);
            g.line(&[(0.20, 0.36), (0.38, 0.50), (0.20, 0.64)], 0.07, rgb(140, 235, 140));
            g.seg((0.46, 0.66), (0.70, 0.66), 0.07, ink);
        }
        Kind::Web => {
            shield(g, ink);
            g.line(&[(0.38, 0.34), (0.26, 0.50), (0.38, 0.66)], 0.06, base);
            g.line(&[(0.62, 0.34), (0.74, 0.50), (0.62, 0.66)], 0.06, base);
        }
        Kind::Style => {
            shield(g, ink);
            g.text((0.5, 0.50), "{}", 0.40, base);
        }
        Kind::Php => {
            g.ellipse((0.5, 0.5), 0.48, 0.30, ink);
            g.text((0.5, 0.5), "php", 0.30, base);
        }
        Kind::Ruby => {
            g.poly(&[(0.22, 0.15), (0.78, 0.15), (0.97, 0.40), (0.03, 0.40)], ink);
            g.poly(&[(0.03, 0.40), (0.97, 0.40), (0.50, 0.92)], darken(ink, 0.8));
            g.line(&[(0.22, 0.15), (0.35, 0.40), (0.50, 0.92)], 0.03, base);
            g.line(&[(0.78, 0.15), (0.65, 0.40), (0.50, 0.92)], 0.03, base);
        }
        Kind::Swift => {
            g.rect((0.04, 0.04), (0.96, 0.96), 0.22, ink);
            g.text((0.5, 0.52), "SW", 0.42, base);
        }
        Kind::Dart => {
            g.poly(&[(0.50, 0.02), (0.98, 0.50), (0.50, 0.98), (0.02, 0.50)], ink);
            g.poly(&[(0.50, 0.28), (0.72, 0.50), (0.50, 0.72), (0.28, 0.50)], base);
        }
        Kind::RLang => {
            g.dot((0.5, 0.5), 0.47, ink);
            g.dot((0.5, 0.5), 0.31, base);
            g.text((0.5, 0.52), "R", 0.50, ink);
        }
        Kind::Haskell => {
            g.seg((0.18, 0.08), (0.82, 0.92), 0.10, ink);
            g.seg((0.46, 0.46), (0.16, 0.92), 0.10, ink);
        }
        Kind::Elixir => {
            g.dot((0.50, 0.64), 0.30, ink);
            g.poly(&[(0.50, 0.04), (0.76, 0.54), (0.24, 0.54)], ink);
            g.dot((0.40, 0.60), 0.07, base);
        }
        Kind::Lua => {
            g.dot((0.5, 0.5), 0.46, ink);
            g.dot((0.64, 0.38), 0.36, base);
            g.dot((0.82, 0.14), 0.07, rgb(255, 179, 71));
        }

        // ── dados ──
        Kind::Json => {
            g.text((0.5, 0.5), "{}", 0.70, ink);
        }
        Kind::Config => {
            for (index, &knob) in [0.68_f32, 0.30, 0.78].iter().enumerate() {
                let y = 0.20 + index as f32 * 0.30;
                g.seg((0.02, y), (0.98, y), 0.06, ink);
                g.dot((knob, y), 0.12, ink);
                g.dot((knob, y), 0.05, base);
            }
        }
        Kind::Log => {
            let colors = [
                rgb(235, 95, 85),
                rgb(240, 190, 70),
                rgb(110, 205, 125),
                rgb(110, 205, 125),
            ];
            for (index, &color) in colors.iter().enumerate() {
                let y = 0.14 + index as f32 * 0.24;
                g.dot((0.06, y), 0.06, color);
                g.seg((0.22, y), (0.98 - index as f32 * 0.08, y), 0.06, ink);
            }
        }
        Kind::Database => {
            g.rect((0.08, 0.20), (0.92, 0.80), 0.0, ink);
            g.ellipse((0.5, 0.80), 0.42, 0.14, ink);
            g.ellipse((0.5, 0.20), 0.42, 0.14, lighten(ink, 15));
            g.arc_line((0.5, 0.42), 0.42, 0.14, 0.0, PI, 0.04, base);
            g.arc_line((0.5, 0.62), 0.42, 0.14, 0.0, PI, 0.04, base);
        }
        Kind::Notebook => {
            g.rect_line((0.18, 0.04), (0.96, 0.96), 0.08, 0.06, ink);
            for index in 0..4 {
                g.dot((0.18, 0.20 + index as f32 * 0.20), 0.06, ink);
            }
            g.rect_line((0.34, 0.20), (0.84, 0.46), 0.04, 0.04, ink);
            g.seg((0.34, 0.62), (0.84, 0.62), 0.05, ink);
            g.seg((0.34, 0.78), (0.70, 0.78), 0.05, ink);
        }

        // ── documentos ──
        Kind::Text => {
            for index in 0..5 {
                let y = 0.10 + index as f32 * 0.20;
                let end = if index == 4 { 0.58 } else { 0.98 };
                g.seg((0.02, y), (end, y), 0.06, ink);
            }
        }
        Kind::Markdown => {
            g.rect_line((0.02, 0.20), (0.98, 0.80), 0.10, 0.06, ink);
            g.line(
                &[(0.16, 0.66), (0.16, 0.34), (0.30, 0.52), (0.44, 0.34), (0.44, 0.66)],
                0.07,
                ink,
            );
            g.seg((0.70, 0.34), (0.70, 0.64), 0.07, ink);
            g.line(&[(0.58, 0.54), (0.70, 0.68), (0.82, 0.54)], 0.07, ink);
        }
        Kind::Document => {
            g.rect((0.04, 0.06), (0.60, 0.22), 0.04, ink);
            for index in 0..4 {
                let y = 0.40 + index as f32 * 0.17;
                let end = if index == 3 { 0.62 } else { 0.96 };
                g.seg((0.04, y), (end, y), 0.06, ink);
            }
        }
        Kind::Pdf => {
            g.rect((0.0, 0.28), (1.0, 0.72), 0.08, rgb(255, 255, 255));
            g.text((0.5, 0.50), "PDF", 0.34, base);
        }
        Kind::Spreadsheet => {
            g.rect((0.02, 0.08), (0.98, 0.92), 0.06, ink);
            g.rect((0.02, 0.08), (0.98, 0.29), 0.06, darken(ink, 0.75));
            for index in 1..3 {
                let x = 0.02 + 0.96 * index as f32 / 3.0;
                g.seg((x, 0.08), (x, 0.92), 0.04, base);
            }
            for index in 1..4 {
                let y = 0.08 + 0.84 * index as f32 / 4.0;
                g.seg((0.02, y), (0.98, y), 0.04, base);
            }
        }
        Kind::Presentation => {
            g.rect_line((0.02, 0.08), (0.98, 0.66), 0.06, 0.06, ink);
            g.rect((0.16, 0.46), (0.26, 0.58), 0.0, ink);
            g.rect((0.36, 0.32), (0.46, 0.58), 0.0, ink);
            g.rect((0.56, 0.40), (0.66, 0.58), 0.0, ink);
            g.seg((0.50, 0.66), (0.50, 0.86), 0.06, ink);
            g.line(&[(0.30, 0.96), (0.50, 0.86), (0.70, 0.96)], 0.06, ink);
        }
        Kind::Ebook => {
            g.poly(&[(0.50, 0.24), (0.04, 0.12), (0.04, 0.78), (0.50, 0.90)], ink);
            g.poly(
                &[(0.50, 0.24), (0.96, 0.12), (0.96, 0.78), (0.50, 0.90)],
                darken(ink, 0.82),
            );
            for index in 0..3 {
                let y = 0.34 + index as f32 * 0.14;
                g.seg((0.12, y), (0.42, y + 0.04), 0.035, base);
                g.seg((0.58, y + 0.04), (0.88, y), 0.035, base);
            }
        }
        Kind::Calendar => {
            g.rect_line((0.04, 0.14), (0.96, 0.96), 0.08, 0.06, ink);
            g.rect((0.04, 0.14), (0.96, 0.38), 0.08, ink);
            g.rect((0.26, 0.02), (0.34, 0.22), 0.03, ink);
            g.rect((0.66, 0.02), (0.74, 0.22), 0.03, ink);
            for row in 0..2 {
                for col in 0..3 {
                    g.dot((0.24 + col as f32 * 0.26, 0.55 + row as f32 * 0.20), 0.05, ink);
                }
            }
        }
        Kind::Contact => {
            g.dot((0.5, 0.30), 0.20, ink);
            let shoulders: Vec<(f32, f32)> = (0..=16)
                .map(|index| {
                    let angle = PI + PI * index as f32 / 16.0;
                    (0.5 + angle.cos() * 0.44, 0.98 + angle.sin() * 0.40)
                })
                .collect();
            g.poly(&shoulders, ink);
        }
        Kind::Email => {
            g.rect_line((0.02, 0.18), (0.98, 0.84), 0.08, 0.06, ink);
            g.line(&[(0.04, 0.22), (0.50, 0.58), (0.96, 0.22)], 0.06, ink);
        }

        // ── sistema ──
        Kind::Archive => {
            for index in 0..8 {
                let y = 0.04 + index as f32 * 0.095;
                if index % 2 == 0 {
                    g.rect((0.34, y), (0.50, y + 0.06), 0.01, ink);
                } else {
                    g.rect((0.50, y), (0.66, y + 0.06), 0.01, ink);
                }
            }
            g.rect((0.38, 0.72), (0.62, 0.98), 0.08, ink);
            g.dot((0.5, 0.84), 0.05, base);
        }
        Kind::Package | Kind::Cargo => cube(g),
        Kind::Apk => {
            // cabeça (meia-lua), olhos e antenas
            let dome: Vec<(f32, f32)> = (0..=16)
                .map(|index| {
                    let angle = PI + PI * index as f32 / 16.0;
                    (0.5 + angle.cos() * 0.40, 0.46 + angle.sin() * 0.38)
                })
                .collect();
            g.poly(&dome, ink);
            g.dot((0.34, 0.30), 0.045, base);
            g.dot((0.66, 0.30), 0.045, base);
            g.seg((0.30, 0.14), (0.22, 0.02), 0.04, ink);
            g.seg((0.70, 0.14), (0.78, 0.02), 0.04, ink);
            // corpo e pernas
            g.rect((0.10, 0.54), (0.90, 0.90), 0.08, ink);
            g.rect((0.28, 0.86), (0.38, 0.99), 0.03, ink);
            g.rect((0.62, 0.86), (0.72, 0.99), 0.03, ink);
            g.text((0.5, 0.72), "APK", 0.26, base);
        }
        Kind::Disk => {
            g.dot((0.5, 0.5), 0.47, ink);
            g.dot((0.5, 0.5), 0.20, base);
            g.dot((0.5, 0.5), 0.07, ink);
            g.arc_line((0.5, 0.5), 0.34, 0.34, 3.6, 4.6, 0.04, base);
        }
        Kind::Application => {
            g.rect((0.03, 0.08), (0.97, 0.92), 0.10, ink);
            g.rect((0.03, 0.08), (0.97, 0.30), 0.10, darken(ink, 0.7));
            g.dot((0.14, 0.19), 0.035, ink);
            g.dot((0.24, 0.19), 0.035, ink);
            g.dot((0.34, 0.19), 0.035, ink);
            g.poly(&[(0.40, 0.46), (0.40, 0.80), (0.70, 0.63)], base);
        }
        Kind::Font => {
            g.text((0.5, 0.5), "Aa", 0.80, ink);
        }
        Kind::Model3d => {
            g.line(
                &[
                    (0.50, 0.02),
                    (0.96, 0.26),
                    (0.96, 0.74),
                    (0.50, 0.98),
                    (0.04, 0.74),
                    (0.04, 0.26),
                    (0.50, 0.02),
                ],
                0.06,
                ink,
            );
            g.line(&[(0.04, 0.26), (0.50, 0.50), (0.96, 0.26)], 0.05, ink);
            g.seg((0.50, 0.50), (0.50, 0.98), 0.05, ink);
            g.dot((0.50, 0.50), 0.05, ink);
        }
        Kind::Game => {
            g.rect((0.02, 0.26), (0.98, 0.82), 0.28, ink);
            g.rect((0.12, 0.47), (0.40, 0.57), 0.02, base);
            g.rect((0.21, 0.38), (0.31, 0.66), 0.02, base);
            g.dot((0.68, 0.50), 0.065, base);
            g.dot((0.80, 0.60), 0.065, base);
        }
        Kind::Key => {
            g.ring((0.26, 0.50), 0.18, 0.09, ink);
            g.seg((0.44, 0.50), (0.96, 0.50), 0.09, ink);
            g.seg((0.78, 0.50), (0.78, 0.74), 0.08, ink);
            g.seg((0.92, 0.50), (0.92, 0.68), 0.08, ink);
        }
        Kind::Download => {
            g.seg((0.50, 0.04), (0.50, 0.56), 0.10, ink);
            g.poly(&[(0.26, 0.44), (0.74, 0.44), (0.50, 0.72)], ink);
            g.line(&[(0.08, 0.70), (0.08, 0.96), (0.92, 0.96), (0.92, 0.70)], 0.07, ink);
        }

        // ── arquivos de projeto ──
        Kind::Git => {
            g.seg((0.30, 0.16), (0.30, 0.84), 0.07, ink);
            g.line(&[(0.30, 0.68), (0.70, 0.46), (0.70, 0.30)], 0.07, ink);
            for &(x, y) in &[(0.30_f32, 0.16_f32), (0.30, 0.84), (0.70, 0.26)] {
                g.dot((x, y), 0.11, ink);
                g.dot((x, y), 0.05, base);
            }
        }
        Kind::Docker => {
            for &(x, y) in &[
                (0.10_f32, 0.36_f32),
                (0.30, 0.36),
                (0.50, 0.36),
                (0.30, 0.16),
                (0.50, 0.16),
            ] {
                g.rect((x, y), (x + 0.16, y + 0.16), 0.02, ink);
            }
            g.poly(&[(0.02, 0.56), (0.92, 0.56), (0.78, 0.90), (0.20, 0.90)], ink);
        }
        Kind::Node => {
            hexagon(g, 0.47, ink);
            g.text((0.5, 0.52), "JS", 0.34, base);
        }
        Kind::CMake => {
            g.poly(&[(0.50, 0.06), (0.96, 0.92), (0.04, 0.92)], ink);
            g.poly(&[(0.50, 0.40), (0.70, 0.80), (0.30, 0.80)], base);
        }
        Kind::Make => glyph(g, Kind::Shell),
        Kind::Readme => {
            g.ring((0.5, 0.5), 0.44, 0.07, ink);
            g.dot((0.5, 0.30), 0.06, ink);
            g.seg((0.5, 0.46), (0.5, 0.76), 0.09, ink);
        }
        Kind::License => {
            shield(g, ink);
            g.line(&[(0.32, 0.52), (0.46, 0.68), (0.70, 0.34)], 0.09, base);
        }
        Kind::Generic => {
            for index in 0..3 {
                let y = 0.22 + index as f32 * 0.28;
                let end = if index == 2 { 0.58 } else { 0.98 };
                g.seg((0.02, y), (end, y), 0.07, ink);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_map_to_distinct_kinds() {
        assert_eq!(classify("rs"), Kind::Rust);
        assert_eq!(classify("py"), Kind::Python);
        assert_eq!(classify("png"), Kind::Image);
        assert_eq!(classify("apk"), Kind::Apk);
        assert_eq!(classify("svg"), Kind::Vector);
        assert_eq!(classify("arw"), Kind::RawPhoto);
        assert_eq!(classify("json"), Kind::Json);
        assert_eq!(classify("toml"), Kind::Config);
        assert_eq!(classify("3ds"), Kind::Game);
        assert_eq!(classify("key"), Kind::Key);
        assert_eq!(classify("torrent"), Kind::Download);
        assert_eq!(classify("xyz"), Kind::Generic);
    }

    #[test]
    fn special_names_win_over_extensions() {
        assert_eq!(special_kind("cargo.toml"), Some(Kind::Cargo));
        assert_eq!(special_kind("dockerfile"), Some(Kind::Docker));
        assert_eq!(special_kind("readme.md"), Some(Kind::Readme));
        assert_eq!(special_kind("main.rs"), None);
    }

    #[test]
    fn known_folders_get_a_style() {
        assert!(folder_style("downloads").is_some());
        assert!(folder_style("imagens").is_some());
        assert!(folder_style("minha pasta").is_none());
    }

    #[test]
    fn tags_use_special_labels_and_extensions() {
        assert_eq!(tag_text(Kind::Git, ".gitignore", ""), "GIT");
        assert_eq!(tag_text(Kind::Archive, "a.tar.gz", "gz"), "TAR.GZ");
        assert_eq!(tag_text(Kind::Rust, "a.rs", "rs"), "RS");
    }

    #[test]
    fn sheet_outline_is_convex() {
        let points = sheet_points(
            Rect::from_min_size(pos2(0.0, 0.0), vec2(68.0, 86.0)),
            20.0,
            7.0,
        );
        let n = points.len();
        let mut sign = 0.0_f32;
        for i in 0..n {
            let a = points[i];
            let b = points[(i + 1) % n];
            let c = points[(i + 2) % n];
            let cross = (b.x - a.x) * (c.y - b.y) - (b.y - a.y) * (c.x - b.x);
            if cross.abs() < 1e-3 {
                continue;
            }
            if sign == 0.0 {
                sign = cross.signum();
            }
            assert_eq!(cross.signum(), sign, "ponto {i} quebra a convexidade");
        }
    }
}
