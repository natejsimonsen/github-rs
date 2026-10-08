//! GitHub's look: Primer colors (light and dark) and the same system font
//! stack github.com uses (-apple-system, Segoe UI, Noto Sans, ...).

use egui::epaint::text::{FontTweak, VariationCoords};
use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId, Stroke, Visuals};
use std::sync::Arc;

pub const BOLD: &str = "bold";
pub const MONO_BOLD: &str = "mono-bold";

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(BOLD.into()))
}
pub fn body(size: f32) -> FontId {
    FontId::proportional(size)
}
pub fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

/// Primer color tokens. Names match GitHub's CSS variables where possible.
#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub canvas: Color32,
    pub canvas_subtle: Color32,
    pub header: Color32,
    pub border: Color32,
    pub border_muted: Color32,
    pub fg: Color32,
    pub fg_muted: Color32,
    pub accent: Color32,
    pub accent_subtle: Color32,
    pub open: Color32,
    pub merged: Color32,
    pub closed: Color32,
    pub attention: Color32,
    pub neutral: Color32,
    pub btn_bg: Color32,
    pub btn_primary: Color32,
    pub selected_row: Color32,
    pub hover_row: Color32,
    pub diff_add: Color32,
    pub diff_add_num: Color32,
    pub diff_del: Color32,
    pub diff_del_num: Color32,
    /// The changed words inside a changed line.
    pub diff_add_word: Color32,
    pub diff_del_word: Color32,
    pub diff_hunk: Color32,
    pub diff_hunk_num: Color32,
    /// Strong fills: selected segmented buttons, the Open badge.
    pub accent_emphasis: Color32,
    pub open_emphasis: Color32,
    pub merged_emphasis: Color32,
    pub closed_emphasis: Color32,
    pub neutral_emphasis: Color32,
    /// Error banners.
    pub danger_subtle: Color32,
    pub danger_border: Color32,
    /// Behind dialogs, and the dialog itself.
    pub backdrop: Color32,
    pub overlay: Color32,
    /// Tooltips and toasts: dark in both themes, like Primer.
    pub tooltip: Color32,
}

const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

pub const LIGHT: Palette = Palette {
    dark: false,
    canvas: hex(0xffffff),
    canvas_subtle: hex(0xf6f8fa),
    header: hex(0xf6f8fa),
    border: hex(0xd1d9e0),
    border_muted: hex(0xd8dee4),
    fg: hex(0x1f2328),
    fg_muted: hex(0x59636e),
    accent: hex(0x0969da),
    accent_subtle: hex(0xddf4ff),
    open: hex(0x1a7f37),
    merged: hex(0x8250df),
    closed: hex(0xcf222e),
    attention: hex(0x9a6700),
    neutral: hex(0x59636e),
    btn_bg: hex(0xf6f8fa),
    btn_primary: hex(0x1f883d),
    selected_row: hex(0xddf4ff),
    hover_row: hex(0xeff2f5),
    diff_add: hex(0xdafbe1),
    diff_add_num: hex(0xaceebb),
    diff_del: hex(0xffebe9),
    diff_del_num: hex(0xffcecb),
    diff_add_word: hex(0xabf2bc),
    diff_del_word: hex(0xffcecb),
    diff_hunk: hex(0xddf4ff),
    diff_hunk_num: hex(0xb6e3ff),
    accent_emphasis: hex(0x0969da),
    open_emphasis: hex(0x1f883d),
    merged_emphasis: hex(0x8250df),
    closed_emphasis: hex(0xcf222e),
    neutral_emphasis: hex(0x59636e),
    danger_subtle: hex(0xffebe9),
    danger_border: hex(0xffcecb),
    backdrop: Color32::from_rgba_premultiplied(16, 18, 20, 128),
    overlay: hex(0xffffff),
    tooltip: hex(0x25292e),
};

pub const DARK: Palette = Palette {
    dark: true,
    canvas: hex(0x0d1117),
    canvas_subtle: hex(0x151b23),
    header: hex(0x010409),
    border: hex(0x3d444d),
    border_muted: hex(0x2f353d),
    fg: hex(0xf0f6fc),
    fg_muted: hex(0x9198a1),
    accent: hex(0x4493f8),
    accent_subtle: hex(0x121d2f),
    open: hex(0x3fb950),
    merged: hex(0xab7df8),
    closed: hex(0xf85149),
    attention: hex(0xd29922),
    neutral: hex(0x9198a1),
    btn_bg: hex(0x212830),
    btn_primary: hex(0x238636),
    selected_row: hex(0x121d2f),
    hover_row: hex(0x1f242c),
    diff_add: hex(0x12261e),
    diff_add_num: hex(0x1c4328),
    diff_del: hex(0x25171c),
    diff_del_num: hex(0x4c2123),
    diff_add_word: hex(0x1a4a29),
    diff_del_word: hex(0x6b2b2b),
    diff_hunk: Color32::from_rgb(0x12, 0x1d, 0x2f),
    diff_hunk_num: Color32::from_rgb(0x1a, 0x2c, 0x4a),
    accent_emphasis: hex(0x1f6feb),
    open_emphasis: hex(0x238636),
    merged_emphasis: hex(0x8957e5),
    closed_emphasis: hex(0xda3633),
    neutral_emphasis: hex(0x656c76),
    danger_subtle: hex(0x25171c),
    danger_border: hex(0x5d2a2d),
    backdrop: Color32::from_rgba_premultiplied(1, 3, 7, 204),
    overlay: hex(0x151b23),
    tooltip: hex(0x3d444d),
};

pub fn palette(ctx: &egui::Context) -> &'static Palette {
    if ctx.global_style().visuals.dark_mode { &DARK } else { &LIGHT }
}

pub fn visuals(p: &Palette) -> Visuals {
    let mut v = if p.dark { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = p.canvas;
    v.extreme_bg_color = p.canvas_subtle;
    v.faint_bg_color = p.canvas_subtle;
    v.code_bg_color = if p.dark { hex(0x262c36) } else { hex(0xeff1f3) };
    v.hyperlink_color = p.accent;
    v.override_text_color = Some(p.fg);
    v.window_stroke = Stroke::new(1.0, p.border);
    v.selection.bg_fill = p.accent.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, p.fg);
    for w in [&mut v.widgets.inactive, &mut v.widgets.noninteractive] {
        w.bg_stroke = Stroke::new(1.0, p.border);
        w.fg_stroke = Stroke::new(1.0, p.fg);
    }
    v.widgets.noninteractive.bg_fill = p.canvas;
    v.widgets.inactive.bg_fill = p.btn_bg;
    v.widgets.inactive.weak_bg_fill = p.btn_bg;
    v.widgets.hovered.weak_bg_fill = if p.dark { hex(0x262c36) } else { hex(0xeff2f5) };
    v.widgets.hovered.bg_fill = v.widgets.hovered.weak_bg_fill;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.active.weak_bg_fill = if p.dark { hex(0x2a313c) } else { hex(0xe6eaef) };
    v.widgets.active.bg_stroke = Stroke::new(1.0, p.accent);
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.noninteractive] {
        w.corner_radius = 6.into();
    }
    // Menus and popups: Primer's overlay look.
    v.menu_corner_radius = 12.into();
    v.window_corner_radius = 12.into();
    v.popup_shadow = egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(if p.dark { 150 } else { 45 }) };
    v.window_fill = p.overlay;
    v
}

pub fn apply(ctx: &egui::Context) {
    ctx.set_visuals_of(egui::Theme::Light, visuals(&LIGHT));
    ctx.set_visuals_of(egui::Theme::Dark, visuals(&DARK));
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(12.0, 5.0);
        s.spacing.interact_size.y = 28.0;
        // egui's edge fade blurs the last row of a list; scroll bars are enough.
        s.spacing.scroll.fade.strength = 0.0;
        s.text_styles.insert(egui::TextStyle::Body, body(14.0));
        s.text_styles.insert(egui::TextStyle::Button, body(14.0));
        s.text_styles.insert(egui::TextStyle::Small, body(12.0));
        s.text_styles.insert(egui::TextStyle::Monospace, mono(12.0));
        s.text_styles.insert(egui::TextStyle::Heading, bold(20.0));
    });
    ctx.set_fonts(fonts());
}

/// A font file on disk, with a face index (for .ttc collections) and an
/// optional variable-font weight.
struct Face {
    path: &'static str,
    index: u32,
    weight: Option<f32>,
}

const fn f(path: &'static str, index: u32, weight: Option<f32>) -> Face {
    Face { path, index, weight }
}

// Candidate fonts per role, in GitHub's font-stack order. The first file that
// exists wins; egui's built-in fonts stay as a fallback for missing glyphs.
#[cfg(target_os = "macos")]
const REGULAR: &[Face] = &[f("/System/Library/Fonts/SFNS.ttf", 0, Some(400.0)), f("/System/Library/Fonts/HelveticaNeue.ttc", 0, None)];
#[cfg(target_os = "macos")]
const SEMIBOLD: &[Face] = &[f("/System/Library/Fonts/SFNS.ttf", 0, Some(600.0)), f("/System/Library/Fonts/HelveticaNeue.ttc", 1, None)];
#[cfg(target_os = "macos")]
const MONO: &[Face] = &[f("/System/Library/Fonts/SFNSMono.ttf", 0, Some(400.0)), f("/System/Library/Fonts/Menlo.ttc", 0, None)];
#[cfg(target_os = "macos")]
const MONO_SEMIBOLD: &[Face] = &[f("/System/Library/Fonts/SFNSMono.ttf", 0, Some(600.0)), f("/System/Library/Fonts/Menlo.ttc", 1, None)];

#[cfg(target_os = "windows")]
const REGULAR: &[Face] = &[f("C:\\Windows\\Fonts\\segoeui.ttf", 0, None)];
#[cfg(target_os = "windows")]
const SEMIBOLD: &[Face] = &[f("C:\\Windows\\Fonts\\seguisb.ttf", 0, None), f("C:\\Windows\\Fonts\\segoeuib.ttf", 0, None)];
#[cfg(target_os = "windows")]
const MONO: &[Face] = &[f("C:\\Windows\\Fonts\\consola.ttf", 0, None)];
#[cfg(target_os = "windows")]
const MONO_SEMIBOLD: &[Face] = &[f("C:\\Windows\\Fonts\\consolab.ttf", 0, None)];

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const REGULAR: &[Face] = &[
    f("/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf", 0, None),
    f("/usr/share/fonts/noto/NotoSans-Regular.ttf", 0, None),
    f("/usr/share/fonts/google-noto/NotoSans-Regular.ttf", 0, None),
    f("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 0, None),
];
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const SEMIBOLD: &[Face] = &[
    f("/usr/share/fonts/truetype/noto/NotoSans-SemiBold.ttf", 0, None),
    f("/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf", 0, None),
    f("/usr/share/fonts/noto/NotoSans-SemiBold.ttf", 0, None),
    f("/usr/share/fonts/noto/NotoSans-Bold.ttf", 0, None),
    f("/usr/share/fonts/google-noto/NotoSans-Bold.ttf", 0, None),
    f("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf", 0, None),
];
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const MONO: &[Face] = &[
    f("/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf", 0, None),
    f("/usr/share/fonts/liberation-mono/LiberationMono-Regular.ttf", 0, None),
    f("/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf", 0, None),
];
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const MONO_SEMIBOLD: &[Face] = &[
    f("/usr/share/fonts/truetype/liberation/LiberationMono-Bold.ttf", 0, None),
    f("/usr/share/fonts/liberation-mono/LiberationMono-Bold.ttf", 0, None),
    f("/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf", 0, None),
];

fn load(faces: &[Face]) -> Option<FontData> {
    faces.iter().find_map(|face| {
        let bytes = std::fs::read(face.path).ok()?;
        let mut data = FontData::from_owned(bytes);
        data.index = face.index;
        if let Some(w) = face.weight {
            data = data.tweak(FontTweak { coords: VariationCoords::new([(b"wght", w)]), ..Default::default() });
        }
        Some(data)
    })
}

fn fonts() -> FontDefinitions {
    let mut defs = FontDefinitions::default();
    let builtin_prop = defs.families[&FontFamily::Proportional].clone();
    let builtin_mono = defs.families[&FontFamily::Monospace].clone();

    let mut family = |name: &str, faces: &[Face], fallback: &[String]| -> Vec<String> {
        let mut list = Vec::new();
        if let Some(data) = load(faces) {
            defs.font_data.insert(name.to_string(), Arc::new(data));
            list.push(name.to_string());
        }
        list.extend(fallback.iter().cloned());
        list
    };
    let prop = family("system", REGULAR, &builtin_prop);
    let bold = family("system-bold", SEMIBOLD, &prop);
    let mono = family("system-mono", MONO, &builtin_mono);
    let mono_bold = family("system-mono-bold", MONO_SEMIBOLD, &mono);

    defs.families.insert(FontFamily::Proportional, prop);
    defs.families.insert(FontFamily::Name(BOLD.into()), bold);
    defs.families.insert(FontFamily::Monospace, mono);
    defs.families.insert(FontFamily::Name(MONO_BOLD.into()), mono_bold);
    defs
}

/// Text color for a GitHub label pill, picked for contrast like github.com.
pub fn label_colors(hex_color: &str, p: &Palette) -> (Color32, Color32, Color32) {
    let v = u32::from_str_radix(hex_color.trim_start_matches('#'), 16).unwrap_or(0x888888);
    let c = hex(v);
    let lum = 0.2126 * c.r() as f32 + 0.7152 * c.g() as f32 + 0.0722 * c.b() as f32;
    if p.dark {
        // Dark mode: tinted background, colored border and text.
        let text = if lum < 110.0 { lighten(c, 0.55) } else { c };
        (c.gamma_multiply(0.22), text, c.gamma_multiply(0.5))
    } else {
        let text = if lum > 150.0 { hex(0x1f2328) } else { Color32::WHITE };
        (c, text, Color32::TRANSPARENT)
    }
}

fn lighten(c: Color32, t: f32) -> Color32 {
    let m = |x: u8| (x as f32 + (255.0 - x as f32) * t) as u8;
    Color32::from_rgb(m(c.r()), m(c.g()), m(c.b()))
}
