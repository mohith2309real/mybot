//! Look and feel.
//!
//! Layout follows the shape of Grok Bot's desktop app — a quiet, near-monochrome
//! window where the colour comes from the roster: every teammate gets its own
//! vivid tile, a role chip, the time and a one-line preview. The identity is
//! MyBot's own, shared with the 1.x console so the two read as one product:
//! warm near-black neutrals, one amber accent, and the hexagon mark (a shell —
//! the computer — around a solid core — the agent). Never anyone else's logo.

use egui::epaint::{Mesh, Shape};
use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Pos2, Rect, RichText, Stroke, TextStyle, Vec2};

// --- palette (the 1.x tokens, converted from oklch) ---------------------------

/// The roster rail: darker than the conversation, as in the reference.
pub const RAIL: Color32 = Color32::from_rgb(12, 11, 9);
/// The conversation canvas.
pub const BG: Color32 = Color32::from_rgb(18, 17, 15);
pub const PANEL: Color32 = Color32::from_rgb(25, 23, 21);
pub const RAISED: Color32 = Color32::from_rgb(33, 31, 28);
pub const RAISED2: Color32 = Color32::from_rgb(42, 39, 36);
pub const LINE: Color32 = Color32::from_rgb(48, 45, 41);
pub const LINE_SOFT: Color32 = Color32::from_rgb(34, 32, 29);
pub const TEXT: Color32 = Color32::from_rgb(235, 231, 226);
pub const TEXT2: Color32 = Color32::from_rgb(175, 171, 165);
pub const MUTED: Color32 = Color32::from_rgb(130, 126, 120);
pub const FAINT: Color32 = Color32::from_rgb(91, 87, 83);
pub const ACCENT: Color32 = Color32::from_rgb(247, 173, 55);
pub const ACCENT_INK: Color32 = Color32::from_rgb(34, 18, 0);
pub const ACCENT_WASH: Color32 = Color32::from_rgba_premultiplied(30, 21, 6, 230);
pub const ACCENT_EDGE: Color32 = Color32::from_rgb(110, 78, 28);
pub const GOOD: Color32 = Color32::from_rgb(103, 205, 135);
pub const WARN: Color32 = Color32::from_rgb(247, 173, 55);
pub const BAD: Color32 = Color32::from_rgb(251, 95, 105);
pub const THINK: Color32 = Color32::from_rgb(160, 150, 196);

// The mark's own colours (from the 1.x app icon).
const MARK_TILE: Color32 = Color32::from_rgb(23, 22, 20);
const MARK_LIGHT: Color32 = Color32::from_rgb(240, 168, 104);
const MARK_DEEP: Color32 = Color32::from_rgb(217, 119, 87);

/// The semibold face: names, headings, buttons. Always registered, so a
/// missing system font degrades to the default face instead of panicking.
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

// --- fonts ---------------------------------------------------------------------

/// The platform's own UI face, at two real weights.
///
/// egui's built-in face is what made the old window look like a Linux tool
/// rather than a desktop app. Nothing is bundled: on macOS this is SF (a
/// variable font, pinned to 400 and 590), on Windows Segoe UI and its semibold
/// cut. Anywhere else, or if a file is missing, the default face stays.
fn fonts(ctx: &egui::Context) {
    use egui::epaint::text::{FontData, FontDefinitions, VariationCoords};
    use std::sync::Arc;

    let mut defs = FontDefinitions::default();
    let read = |p: &str| std::fs::read(p).ok();
    let weighted = |bytes: Vec<u8>, wght: Option<f32>| {
        let mut d = FontData::from_owned(bytes);
        if let Some(w) = wght {
            d.tweak.coords = VariationCoords::new([(b"wght", w)]);
        }
        Arc::new(d)
    };

    let (regular, semi, mono) = if cfg!(target_os = "macos") {
        let sf = read("/System/Library/Fonts/SFNS.ttf");
        (
            sf.clone().map(|b| weighted(b, Some(400.0))),
            sf.map(|b| weighted(b, Some(590.0))),
            read("/System/Library/Fonts/SFNSMono.ttf").map(|b| weighted(b, Some(420.0))),
        )
    } else if cfg!(target_os = "windows") {
        (
            read("C:\\Windows\\Fonts\\segoeui.ttf").map(|b| weighted(b, None)),
            read("C:\\Windows\\Fonts\\seguisb.ttf").map(|b| weighted(b, None)),
            read("C:\\Windows\\Fonts\\consola.ttf").map(|b| weighted(b, None)),
        )
    } else {
        (None, None, None)
    };

    let base = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mut semi_family = base.clone();
    if let Some(d) = regular {
        defs.font_data.insert("ui".into(), d);
        defs.families.entry(FontFamily::Proportional).or_default().insert(0, "ui".into());
    }
    if let Some(d) = semi {
        defs.font_data.insert("ui-semibold".into(), d);
        semi_family.insert(0, "ui-semibold".into());
    } else if defs.font_data.contains_key("ui") {
        semi_family.insert(0, "ui".into());
    }
    defs.families.insert(FontFamily::Name("semibold".into()), semi_family);
    if let Some(d) = mono {
        defs.font_data.insert("ui-mono".into(), d);
        defs.families.entry(FontFamily::Monospace).or_default().insert(0, "ui-mono".into());
    }
    ctx.set_fonts(defs);
}

pub fn apply(ctx: &egui::Context) {
    fonts(ctx);
    let mut style = (*ctx.global_style()).clone();
    style.text_styles = [
        (TextStyle::Heading, semibold(19.0)),
        (TextStyle::Body, FontId::proportional(14.0)),
        (TextStyle::Button, FontId::proportional(13.5)),
        (TextStyle::Small, FontId::proportional(11.5)),
        (TextStyle::Monospace, FontId::monospace(12.5)),
    ]
    .into();
    style.spacing.item_spacing = Vec2::new(8.0, 7.0);
    style.spacing.button_padding = Vec2::new(12.0, 6.0);
    style.spacing.interact_size.y = 30.0;
    style.spacing.scroll.floating = true;
    style.spacing.scroll.bar_width = 6.0;
    // Text in the thread opts in with `.selectable(true)`; everywhere else a
    // label must not swallow the click meant for the row it sits on.
    style.interaction.selectable_labels = false;

    let v = &mut style.visuals;
    *v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.extreme_bg_color = Color32::from_rgb(14, 13, 11);
    v.faint_bg_color = PANEL;
    v.code_bg_color = RAISED;
    v.window_stroke = Stroke::new(1.0, LINE);
    v.window_corner_radius = CornerRadius::same(16);
    v.window_shadow = egui::Shadow { offset: [0, 18], blur: 48, spread: 0, color: Color32::from_black_alpha(160) };
    v.popup_shadow = egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(140) };
    v.menu_corner_radius = CornerRadius::same(12);
    v.selection.bg_fill = Color32::from_rgb(92, 64, 20);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.hyperlink_color = ACCENT;
    v.override_text_color = Some(TEXT);
    v.text_cursor.stroke = Stroke::new(2.0, ACCENT);
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open, &mut v.widgets.noninteractive] {
        w.corner_radius = CornerRadius::same(10);
    }
    v.widgets.inactive.weak_bg_fill = RAISED;
    v.widgets.inactive.bg_fill = RAISED;
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT2);
    v.widgets.hovered.weak_bg_fill = RAISED2;
    v.widgets.hovered.bg_fill = RAISED2;
    v.widgets.hovered.bg_stroke = Stroke::NONE;
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.active.weak_bg_fill = Color32::from_rgb(52, 48, 44);
    v.widgets.active.bg_stroke = Stroke::NONE;
    v.widgets.open.weak_bg_fill = RAISED2;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE_SOFT);
    v.widgets.noninteractive.weak_bg_fill = PANEL;
    ctx.set_global_style(style);
}

// --- colour --------------------------------------------------------------------

/// OKLCH → sRGB, so tiles here match the 1.x console's CSS exactly.
fn oklch(l: f32, c: f32, h_deg: f32) -> Color32 {
    let (a, b) = (c * h_deg.to_radians().cos(), c * h_deg.to_radians().sin());
    let l_ = l + 0.396_337_8 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_35 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;
    let (l3, m3, s3) = (l_.powi(3), m_.powi(3), s_.powi(3));
    let r = 4.076_741_7 * l3 - 3.307_711_6 * m3 + 0.230_969_94 * s3;
    let g = -1.268_438 * l3 + 2.609_757_4 * m3 - 0.341_319_38 * s3;
    let bl = -0.004_196_086_3 * l3 - 0.703_418_6 * m3 + 1.707_614_7 * s3;
    let enc = |x: f32| {
        let x = x.clamp(0.0, 1.0);
        let v = if x <= 0.003_130_8 { 12.92 * x } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 };
        (v * 255.0).round() as u8
    };
    Color32::from_rgb(enc(r), enc(g), enc(bl))
}

/// The same hue-from-name hash as the 1.x console, so a teammate called
/// "researcher" is the same colour in both.
fn hue_deg(name: &str) -> f32 {
    name.encode_utf16().fold(0u32, |h, c| (h * 31 + c as u32) % 360) as f32
}

fn tile_colors(name: &str) -> (Color32, Color32, Color32) {
    let h = hue_deg(name);
    (oklch(0.70, 0.17, h), oklch(0.55, 0.16, h + 28.0), oklch(0.16, 0.03, h))
}

pub fn initials(name: &str) -> String {
    let parts: Vec<&str> = name.split(|c: char| !c.is_alphanumeric()).filter(|p| !p.is_empty()).collect();
    match parts.as_slice() {
        [a, b, ..] => format!("{}{}", a.chars().next().unwrap_or(' '), b.chars().next().unwrap_or(' ')).to_uppercase(),
        [a] => a.chars().take(2).collect::<String>().to_uppercase(),
        [] => "?".into(),
    }
}

// --- shapes --------------------------------------------------------------------

/// Fill a convex polygon with a top-to-bottom gradient, edges antialiased.
fn gradient_fill(painter: &egui::Painter, pts: &[Pos2], top: Color32, bottom: Color32) {
    if pts.len() < 3 {
        return;
    }
    let (y0, y1) = pts.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.y), b.max(p.y)));
    let at = |y: f32| top.lerp_to_gamma(bottom, ((y - y0) / (y1 - y0).max(1.0)).clamp(0.0, 1.0));
    let c = pts.iter().fold(Vec2::ZERO, |s, p| s + p.to_vec2()) / pts.len() as f32;
    let mut mesh = Mesh::default();
    mesh.colored_vertex(c.to_pos2(), at(c.y));
    for p in pts {
        mesh.colored_vertex(*p, at(p.y));
    }
    let n = pts.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    painter.add(Shape::mesh(mesh));
    // The mesh has no antialiasing; a hairline in the mid colour gives it one.
    painter.add(Shape::closed_line(pts.to_vec(), Stroke::new(1.0, at((y0 + y1) / 2.0))));
}

fn rounded_rect_points(rect: Rect, r: f32) -> Vec<Pos2> {
    let r = r.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let corners = [
        (rect.right_top() + Vec2::new(-r, r), -90.0f32),
        (rect.right_bottom() + Vec2::new(-r, -r), 0.0),
        (rect.left_bottom() + Vec2::new(r, -r), 90.0),
        (rect.left_top() + Vec2::new(r, r), 180.0),
    ];
    let mut pts = Vec::with_capacity(32);
    for (c, start) in corners {
        for i in 0..=7 {
            let a = (start + 90.0 * i as f32 / 7.0).to_radians();
            pts.push(c + Vec2::new(a.cos(), a.sin()) * r);
        }
    }
    pts
}

/// A teammate's tile: rounded square in its own colour, initials, and an
/// optional presence dot (working, or waiting on you).
pub fn avatar(ui: &mut egui::Ui, name: &str, size: f32, state: Option<Color32>) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    paint_avatar(ui.painter(), rect, name, state, BG);
    resp
}

pub fn paint_avatar(p: &egui::Painter, rect: Rect, name: &str, state: Option<Color32>, behind: Color32) {
    let size = rect.width();
    let (top, bottom, ink) = tile_colors(name);
    gradient_fill(p, &rounded_rect_points(rect, size * 0.29), top, bottom);
    p.text(rect.center() + Vec2::new(0.0, size * 0.01), egui::Align2::CENTER_CENTER, initials(name), semibold(size * 0.34), ink);
    if let Some(c) = state {
        let center = rect.right_bottom() - Vec2::splat(size * 0.06);
        p.circle_filled(center, size * 0.17, behind);
        p.circle_filled(center, size * 0.11, c);
    }
}

/// The MyBot mark — hexagonal shell, solid core, tally light — the same
/// drawing as the app icon, at any size, with no tile behind it.
pub fn mark(ui: &mut egui::Ui, size: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    paint_mark(ui.painter(), rect, false);
    resp
}

/// The full app icon (tile included), for the lock screen and the like.
pub fn app_tile(ui: &mut egui::Ui, size: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    paint_mark(ui.painter(), rect, true);
    resp
}

/// Geometry in the icon's own 512-unit space (see `assets/icon.svg`).
const SHELL: [(f32, f32); 6] = [(256.0, 92.0), (404.0, 178.0), (404.0, 334.0), (256.0, 420.0), (108.0, 334.0), (108.0, 178.0)];
const CORE: [(f32, f32); 6] = [(256.0, 178.0), (326.0, 219.0), (326.0, 301.0), (256.0, 342.0), (186.0, 301.0), (186.0, 219.0)];

fn paint_mark(p: &egui::Painter, rect: Rect, with_tile: bool) {
    // Without a tile the mark fills its box; with one, it sits at icon scale.
    let (s, o) = if with_tile {
        (rect.width() / 512.0, rect.min.to_vec2())
    } else {
        let s = rect.width() / 360.0;
        (s, rect.center().to_vec2() - Vec2::splat(256.0 * s))
    };
    let at = |(x, y): (f32, f32)| Pos2::new(x * s, y * s) + o;
    if with_tile {
        p.rect_filled(rect, CornerRadius::same((114.0 * s).round() as u8), MARK_TILE);
    }
    let shell: Vec<Pos2> = SHELL.iter().map(|&q| at(q)).collect();
    p.add(Shape::closed_line(shell, Stroke::new((26.0 * s).max(1.2), MARK_DEEP.gamma_multiply(0.42))));
    let core: Vec<Pos2> = CORE.iter().map(|&q| at(q)).collect();
    gradient_fill(p, &core, MARK_LIGHT, MARK_DEEP);
    let behind = if with_tile { MARK_TILE } else { RAIL };
    p.circle_filled(at((404.0, 130.0)), 27.0 * s, behind);
    p.circle_filled(at((404.0, 130.0)), 16.0 * s, MARK_LIGHT);
}

/// The window/Dock icon, rasterised here (no image files at runtime).
/// Supersampled 4×4 so the edges are smooth. `margin` is the transparent
/// border as a fraction of the size: macOS icons sit on a grid with the tile
/// at 824/1024 (≈0.1 each side); elsewhere the tile fills the square.
pub fn icon_rgba(n: usize, margin: f32) -> Vec<u8> {
    let inner = n as f32 * (1.0 - 2.0 * margin);
    let offset = n as f32 * margin;
    let scale = 512.0 / inner;
    let inside = |poly: &[(f32, f32)], x: f32, y: f32| -> bool {
        let mut c = false;
        let mut j = poly.len() - 1;
        for i in 0..poly.len() {
            let (xi, yi) = poly[i];
            let (xj, yj) = poly[j];
            if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
                c = !c;
            }
            j = i;
        }
        c
    };
    let seg_dist = |px: f32, py: f32, (ax, ay): (f32, f32), (bx, by): (f32, f32)| -> f32 {
        let (dx, dy) = (bx - ax, by - ay);
        let t = (((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        ((px - ax - t * dx).powi(2) + (py - ay - t * dy).powi(2)).sqrt()
    };
    let in_tile = |x: f32, y: f32| -> bool {
        let r = 114.0;
        let cx = x.clamp(r, 512.0 - r);
        let cy = y.clamp(r, 512.0 - r);
        (0.0..=512.0).contains(&x) && (0.0..=512.0).contains(&y) && (x - cx).powi(2) + (y - cy).powi(2) <= r * r
    };
    let mix = |a: [f32; 4], b: [f32; 3], alpha: f32| -> [f32; 4] {
        [a[0] + (b[0] - a[0]) * alpha, a[1] + (b[1] - a[1]) * alpha, a[2] + (b[2] - a[2]) * alpha, a[3] + (1.0 - a[3]) * alpha]
    };
    let rgb = |c: Color32| [c.r() as f32, c.g() as f32, c.b() as f32];
    let (tile, light, deep) = (rgb(MARK_TILE), rgb(MARK_LIGHT), rgb(MARK_DEEP));

    let mut out = vec![0u8; n * n * 4];
    const SS: usize = 4;
    for py in 0..n {
        for px in 0..n {
            let mut acc = [0.0f32; 4];
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = (px as f32 + (sx as f32 + 0.5) / SS as f32 - offset) * scale;
                    let y = (py as f32 + (sy as f32 + 0.5) / SS as f32 - offset) * scale;
                    let mut c = [0.0f32; 4];
                    if in_tile(x, y) {
                        c = [tile[0], tile[1], tile[2], 1.0];
                        let shell = (0..6).any(|i| seg_dist(x, y, SHELL[i], SHELL[(i + 1) % 6]) <= 13.0);
                        if shell {
                            c = mix(c, deep, 0.42);
                        }
                        if inside(&CORE, x, y) {
                            let t = ((y - 178.0) / (342.0 - 178.0)).clamp(0.0, 1.0);
                            c = [light[0] + (deep[0] - light[0]) * t, light[1] + (deep[1] - light[1]) * t, light[2] + (deep[2] - light[2]) * t, 1.0];
                        }
                        let d = ((x - 404.0).powi(2) + (y - 130.0).powi(2)).sqrt();
                        if d <= 27.0 {
                            c = [tile[0], tile[1], tile[2], 1.0];
                        }
                        if d <= 16.0 {
                            c = [light[0], light[1], light[2], 1.0];
                        }
                    }
                    for k in 0..3 {
                        acc[k] += c[k] * c[3];
                    }
                    acc[3] += c[3];
                }
            }
            let i = (py * n + px) * 4;
            let a = acc[3] / (SS * SS) as f32;
            if a > 0.0 {
                for k in 0..3 {
                    out[i + k] = (acc[k] / acc[3]).round().clamp(0.0, 255.0) as u8;
                }
                out[i + 3] = (a * 255.0).round() as u8;
            }
        }
    }
    out
}

// --- widgets -------------------------------------------------------------------

/// A one-line text field with room to breathe.
pub fn input(s: &mut String) -> egui::TextEdit<'_> {
    egui::TextEdit::singleline(s).margin(Margin::symmetric(10, 6))
}

/// The one loud button on a screen.
pub fn accent_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).font(semibold(13.5)).color(ACCENT_INK)).fill(ACCENT).stroke(Stroke::NONE).corner_radius(CornerRadius::same(10))
}

/// Light pill: the hand-back button in the agent computer, as in the reference.
pub fn light_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).font(semibold(13.0)).color(Color32::from_rgb(22, 20, 18))).fill(TEXT).stroke(Stroke::NONE).corner_radius(CornerRadius::same(255))
}

pub fn ghost_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).color(TEXT2)).fill(Color32::TRANSPARENT).stroke(Stroke::new(1.0, LINE)).corner_radius(CornerRadius::same(10))
}

pub fn danger_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).color(BAD)).fill(Color32::TRANSPARENT).stroke(Stroke::new(1.0, Color32::from_rgb(88, 40, 42))).corner_radius(CornerRadius::same(10))
}

/// Text-only link-style button.
pub fn link_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).color(TEXT2).size(13.0)).frame(false)
}

/// A small grey pill, like the role chips beside names in the reference.
pub fn chip(ui: &mut egui::Ui, text: &str) {
    chip_colored(ui, text, MUTED, RAISED2);
}

pub fn chip_colored(ui: &mut egui::Ui, text: &str, fg: Color32, bg: Color32) {
    egui::Frame::NONE.fill(bg).corner_radius(5).inner_margin(Margin::symmetric(6, 1)).show(ui, |ui| {
        ui.label(RichText::new(text).font(semibold(10.5)).color(fg));
    });
}

pub fn card() -> egui::Frame {
    egui::Frame::NONE.fill(PANEL).stroke(Stroke::new(1.0, LINE_SOFT)).corner_radius(CornerRadius::same(14)).inner_margin(Margin::same(16))
}

pub fn muted(t: impl Into<String>) -> RichText {
    RichText::new(t).color(MUTED)
}

pub fn small(t: impl Into<String>) -> RichText {
    RichText::new(t).color(MUTED).size(12.0)
}

pub fn heading(t: impl Into<String>) -> RichText {
    RichText::new(t).font(semibold(19.0)).color(TEXT)
}

pub fn strong(t: impl Into<String>) -> RichText {
    RichText::new(t).font(semibold(14.0)).color(TEXT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_match_the_1x_console() {
        // 1.x: hueOf("researcher") with h = (h*31 + code) % 360.
        let h = "researcher".chars().fold(0u32, |h, c| (h * 31 + c as u32) % 360);
        assert_eq!(hue_deg("researcher"), h as f32);
        // The 1.x accent, oklch(0.80 0.152 74), is the amber used here.
        let a = oklch(0.80, 0.152, 74.0);
        assert!((a.r() as i32 - ACCENT.r() as i32).abs() <= 2 && (a.g() as i32 - ACCENT.g() as i32).abs() <= 2, "{a:?}");
    }

    #[test]
    fn icon_is_opaque_inside_and_clear_at_the_corner() {
        let n = 64;
        let px = icon_rgba(n, 0.0);
        assert_eq!(px.len(), n * n * 4);
        assert_eq!(px[3], 0, "the rounded corner is transparent");
        let centre = ((n / 2) * n + n / 2) * 4;
        assert_eq!(px[centre + 3], 255, "the core is solid");
        assert!(px[centre] > px[centre + 2], "the core is warm (more red than blue)");
        let padded = icon_rgba(n, 0.1);
        let edge = ((n / 2) * n + 2) * 4;
        assert_eq!(padded[edge + 3], 0, "the macOS margin is transparent");
    }

    /// Writes the macOS iconset and the Windows icon sizes
    /// (`MYBOT_ICON_OUT=dir cargo test -- --ignored`), from the same drawing as
    /// the window icon; `iconutil` makes the .icns, `assets/make-ico.py` the .ico.
    #[test]
    #[ignore]
    fn write_iconset() {
        let Some(dir) = std::env::var_os("MYBOT_ICON_OUT") else { return };
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (px, name) in [(16, "16x16"), (32, "16x16@2x"), (32, "32x32"), (64, "32x32@2x"), (128, "128x128"), (256, "128x128@2x"), (256, "256x256"), (512, "256x256@2x"), (512, "512x512"), (1024, "512x512@2x")] {
            let rgba = icon_rgba(px, 0.1);
            std::fs::write(dir.join(format!("icon_{name}.png")), super::super::snapshot::png(px as u32, px as u32, &rgba)).unwrap();
        }
        // Windows sizes, full-bleed (Windows icons carry no margin of their own).
        let win = dir.join("windows");
        std::fs::create_dir_all(&win).unwrap();
        for px in [16u32, 24, 32, 48, 64, 128, 256] {
            let rgba = icon_rgba(px as usize, 0.03);
            std::fs::write(win.join(format!("{px}.png")), super::super::snapshot::png(px, px, &rgba)).unwrap();
        }
    }
}
