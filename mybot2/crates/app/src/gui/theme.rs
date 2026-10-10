//! Look and feel.
//!
//! Layout follows the shape of Grok Bot's desktop app: a quiet window, a
//! roster of teammates (tile, role chip, time, one-line preview) and the
//! conversation. The identity is MyBot's own and monochrome: true greys on
//! black, white as the only accent, and the mark — a single-line "m" with a
//! dot beside it, the teammate that's live. Colour is kept for meaning only:
//! red for recording and for things that delete. Never anyone else's logo.

use egui::epaint::{Mesh, Shape};
use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Pos2, Rect, RichText, Stroke, TextStyle, Vec2};

// --- palette (true greys; white is the accent) ---------------------------------

/// The roster rail: darker than the conversation, as in the reference.
pub const RAIL: Color32 = Color32::from_rgb(8, 8, 8);
/// The conversation canvas.
pub const BG: Color32 = Color32::from_rgb(14, 14, 14);
pub const PANEL: Color32 = Color32::from_rgb(21, 21, 21);
pub const RAISED: Color32 = Color32::from_rgb(29, 29, 29);
pub const RAISED2: Color32 = Color32::from_rgb(39, 39, 39);
pub const LINE: Color32 = Color32::from_rgb(46, 46, 46);
pub const LINE_SOFT: Color32 = Color32::from_rgb(31, 31, 31);
pub const TEXT: Color32 = Color32::from_rgb(236, 236, 236);
pub const TEXT2: Color32 = Color32::from_rgb(170, 170, 170);
pub const MUTED: Color32 = Color32::from_rgb(124, 124, 124);
pub const FAINT: Color32 = Color32::from_rgb(86, 86, 86);
/// Hover on the rail.
pub const HOVER: Color32 = Color32::from_rgb(20, 20, 20);
pub const ACCENT: Color32 = Color32::from_rgb(250, 250, 250);
pub const ACCENT_INK: Color32 = Color32::from_rgb(8, 8, 8);
pub const ACCENT_WASH: Color32 = Color32::from_rgb(24, 24, 24);
pub const ACCENT_EDGE: Color32 = Color32::from_rgb(92, 92, 92);
/// Working, done, on: a light grey — the ✓ or the word carries the meaning.
pub const GOOD: Color32 = Color32::from_rgb(178, 178, 178);
/// Needs you: the brightest thing in the window.
pub const WARN: Color32 = ACCENT;
pub const BAD: Color32 = Color32::from_rgb(240, 92, 98);
pub const THINK: Color32 = Color32::from_rgb(132, 132, 132);

// The mark's own colours: white on a black tile.
const MARK_TILE: Color32 = Color32::from_rgb(10, 10, 10);
const MARK_INK: Color32 = Color32::from_rgb(250, 250, 250);

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
    v.extreme_bg_color = Color32::from_rgb(12, 12, 12);
    v.faint_bg_color = PANEL;
    v.code_bg_color = RAISED;
    v.window_stroke = Stroke::new(1.0, LINE);
    v.window_corner_radius = CornerRadius::same(16);
    v.window_shadow = egui::Shadow { offset: [0, 18], blur: 48, spread: 0, color: Color32::from_black_alpha(160) };
    v.popup_shadow = egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(140) };
    v.menu_corner_radius = CornerRadius::same(12);
    v.selection.bg_fill = Color32::from_rgb(70, 70, 70);
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
    v.widgets.active.weak_bg_fill = Color32::from_rgb(50, 50, 50);
    v.widgets.active.bg_stroke = Stroke::NONE;
    v.widgets.open.weak_bg_fill = RAISED2;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE_SOFT);
    v.widgets.noninteractive.weak_bg_fill = PANEL;
    ctx.set_global_style(style);
}

// --- colour --------------------------------------------------------------------

/// OKLCH → sRGB (the 1.x console's colour space; tiles use its lightness).
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

/// The same name hash as the 1.x console, so a teammate keeps its tile.
fn hue_deg(name: &str) -> f32 {
    name.encode_utf16().fold(0u32, |h, c| (h * 31 + c as u32) % 360) as f32
}

/// A teammate's tile: one of four close graphite greys, picked by the name
/// hash, with a slight top-to-bottom fall-off and white initials — enough to
/// tell neighbours apart without any tile shouting.
fn tile_colors(name: &str) -> (Color32, Color32, Color32) {
    let level = [0.31f32, 0.37, 0.43, 0.49][hue_deg(name) as usize % 4];
    (oklch(level, 0.0, 0.0), oklch(level - 0.06, 0.0, 0.0), oklch(0.98, 0.0, 0.0))
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

/// A site's tile (saved logins): initials on graphite.
pub fn initials_tile(ui: &mut egui::Ui, name: &str, size: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    let (top, bottom, ink) = tile_colors(name);
    let p = ui.painter();
    gradient_fill(p, &rounded_rect_points(rect, size * 0.29), top, bottom);
    p.text(rect.center() + Vec2::new(0.0, size * 0.01), egui::Align2::CENTER_CENTER, initials(name), semibold(size * 0.34), ink);
    resp
}

// --- faces ---------------------------------------------------------------------
//
// Every teammate is a face: a head shape and a pair of eyes, white on black,
// drawn in code. The face is the teammate's; the expression is what it's doing
// right now, and every face moves in its own ways:
//
//   working   reading, typing, thinking, scanning or humming — a face rotates
//             through its own few, a few seconds each
//   idle      blinks, and now and then glances aside, breathes or dozes
//   needs you wide eyes up at you, with a bounce, a wiggle or a pulse
//   done      ^ ^, with a hop, a sway or a sparkle
//
// Only "working" animates continuously (a task is running anyway); the other
// moods wake the window just for each short move. The app icon is "Glance".

/// What a teammate is doing; its face shows it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mood {
    Idle,
    Working,
    NeedsYou,
    Done,
}

#[derive(Clone, Copy)]
enum Head {
    /// Width and height as a fraction of the box; corner radii (nw, ne, sw, se)
    /// as a fraction of the box too.
    Rect(f32, f32, [f32; 4]),
    /// A circle, diameter as a fraction of the box.
    Round(f32),
}

#[derive(Clone, Copy)]
enum Eyes {
    Capsule,
    Dot,
    Bar,
    Square,
}

/// Ways of working.
#[derive(Clone, Copy, Debug)]
enum Work {
    /// Eyes hop along a line in short jumps, then sweep back.
    Read,
    /// Looking down at the keys, the head bobbing with each burst.
    Type,
    /// Eyes up and drifting, one narrowed.
    Think,
    /// Eyes circling the page.
    Scan,
    /// Eyes half shut, swaying to itself.
    Hum,
}

/// Small moves while idle (blinks happen anyway).
#[derive(Clone, Copy, Debug)]
enum Fidget {
    Glance,
    Breathe,
    Doze,
}

/// Getting your attention.
#[derive(Clone, Copy, Debug)]
enum Call {
    Bounce,
    Wiggle,
    Pulse,
}

/// Celebrating a finished job.
#[derive(Clone, Copy, Debug)]
enum Cheer {
    Hop,
    Sway,
    Sparkle,
}

#[derive(Clone, Copy)]
pub struct Face {
    pub name: &'static str,
    head: Head,
    eyes: Eyes,
    /// Each eye's distance from the middle, and where the pair looks (x, y),
    /// as fractions of the box; the right eye's size relative to the left.
    gap: f32,
    gaze: (f32, f32),
    right: f32,
    work: &'static [Work],
    idle: &'static [Fidget],
    call: &'static [Call],
    cheer: &'static [Cheer],
}

use Call::*;
use Cheer::*;
use Fidget::*;
use Work::*;

pub const FACES: [Face; 8] = [
    Face { name: "Pip", head: Head::Rect(0.84, 0.80, [0.30; 4]), eyes: Eyes::Capsule, gap: 0.13, gaze: (0.0, -0.03), right: 1.0, work: &[Read, Type, Scan], idle: &[Glance, Breathe], call: &[Bounce, Pulse], cheer: &[Hop, Sparkle] },
    Face { name: "Glance", head: Head::Rect(0.84, 0.81, [0.29; 4]), eyes: Eyes::Capsule, gap: 0.132, gaze: (0.053, -0.078), right: 1.1, work: &[Scan, Think, Read], idle: &[Glance, Breathe], call: &[Pulse, Wiggle], cheer: &[Sparkle, Hop] },
    Face { name: "Bean", head: Head::Rect(0.62, 0.88, [0.31; 4]), eyes: Eyes::Dot, gap: 0.10, gaze: (0.0, -0.10), right: 1.0, work: &[Type, Hum, Read], idle: &[Breathe, Doze], call: &[Bounce, Wiggle], cheer: &[Hop, Sway] },
    Face { name: "Brick", head: Head::Rect(0.92, 0.64, [0.20; 4]), eyes: Eyes::Bar, gap: 0.16, gaze: (0.0, -0.02), right: 1.0, work: &[Read, Type], idle: &[Doze, Glance], call: &[Wiggle, Pulse], cheer: &[Sway, Sparkle] },
    Face { name: "Leaf", head: Head::Rect(0.82, 0.82, [0.06, 0.41, 0.41, 0.06]), eyes: Eyes::Capsule, gap: 0.12, gaze: (0.02, -0.02), right: 1.0, work: &[Think, Scan, Hum], idle: &[Glance, Breathe], call: &[Pulse, Bounce], cheer: &[Sparkle, Sway] },
    Face { name: "Ghost", head: Head::Rect(0.78, 0.86, [0.39, 0.39, 0.10, 0.10]), eyes: Eyes::Dot, gap: 0.12, gaze: (0.0, -0.08), right: 1.0, work: &[Hum, Scan], idle: &[Breathe, Glance], call: &[Wiggle, Bounce], cheer: &[Sway, Hop] },
    Face { name: "Moon", head: Head::Round(0.86), eyes: Eyes::Square, gap: 0.13, gaze: (0.0, -0.03), right: 1.0, work: &[Scan, Read, Think], idle: &[Doze, Glance], call: &[Bounce, Pulse], cheer: &[Hop, Sparkle] },
    Face { name: "Blob", head: Head::Rect(0.88, 0.78, [0.40, 0.28, 0.28, 0.40]), eyes: Eyes::Capsule, gap: 0.14, gaze: (-0.04, -0.04), right: 0.9, work: &[Hum, Type, Think], idle: &[Breathe, Glance], call: &[Pulse, Wiggle], cheer: &[Sway, Hop] },
];
/// The app icon's face.
pub const GLANCE: usize = 1;
const EYE_INK: Color32 = Color32::from_rgb(14, 14, 14);

/// Faces chosen in "Edit teammate", by lowercased name; everyone else gets one
/// from the 1.x name hash.
fn chosen() -> &'static std::sync::RwLock<std::collections::HashMap<String, usize>> {
    static CHOSEN: std::sync::OnceLock<std::sync::RwLock<std::collections::HashMap<String, usize>>> = std::sync::OnceLock::new();
    CHOSEN.get_or_init(Default::default)
}

/// Where a teammate's chosen face is saved (settings table).
pub fn face_key(name: &str) -> String {
    format!("face.{}", name.trim().to_lowercase())
}

pub fn set_face(name: &str, face: usize) {
    if let Ok(mut m) = chosen().write() {
        m.insert(name.trim().to_lowercase(), face % FACES.len());
    }
}

pub fn face_of(name: &str) -> usize {
    let key = name.trim().to_lowercase();
    chosen().read().ok().and_then(|m| m.get(&key).copied()).unwrap_or(hue_deg(&key) as usize % FACES.len())
}

/// The shades heads come in, for showing faces outside a roster.
pub fn face_shade(i: usize) -> Color32 {
    oklch([0.97f32, 0.90, 0.83, 0.76][i % 4], 0.0, 0.0)
}

/// The head's shade: four light greys, by name, so neighbours differ a little.
fn head_shade(name: &str) -> Color32 {
    let l = [0.97f32, 0.90, 0.83, 0.76][(hue_deg(&name.trim().to_lowercase()) as usize / 8) % 4];
    oklch(l, 0.0, 0.0)
}

fn corner(px: f32) -> u8 {
    px.round().clamp(0.0, 255.0) as u8
}

/// The faces' clock: the window's time, or a fixed one while recording frames.
static CLOCK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(u64::MAX);

/// Pin the faces' clock to `t` seconds (snapshot sequences), or None for live.
pub fn set_face_clock(t: Option<f64>) {
    CLOCK.store(t.map_or(u64::MAX, f64::to_bits), std::sync::atomic::Ordering::Relaxed);
}

fn face_time(ctx: &egui::Context) -> f64 {
    match CLOCK.load(std::sync::atomic::Ordering::Relaxed) {
        u64::MAX => ctx.input(|i| i.time),
        bits => f64::from_bits(bits),
    }
}

fn ease(u: f32) -> f32 {
    let u = u.clamp(0.0, 1.0);
    u * u * (3.0 - 2.0 * u)
}

/// 0 → 1 → 0 over u in 0..1.
fn bump(u: f32) -> f32 {
    (std::f32::consts::PI * u.clamp(0.0, 1.0)).sin()
}

/// Up quickly, hold, down quickly: for moves that stay a moment.
fn hold(u: f32) -> f32 {
    ease(u / 0.2) * ease((1.0 - u) / 0.2)
}

/// A move of `len` seconds every `every` seconds. Returns which one this is,
/// its progress while it plays, and how long until it starts or stops.
fn episode(t: f64, every: f64, len: f64, offset: f64) -> (u64, Option<f32>, f64) {
    let x = t + offset;
    let n = (x / every).floor();
    let local = x - n * every;
    if local < len { (n as u64, Some((local / len) as f32), len - local) } else { (n as u64, None, every - local) }
}

/// Everything that moves, as offsets from the face at rest.
#[derive(Clone, Copy)]
struct Pose {
    /// Head offset and scale, as fractions of the box.
    head: Vec2,
    squash: Vec2,
    /// Eye-pair offset on top of the face's own gaze.
    gaze: Vec2,
    /// Size of each eye relative to rest.
    eye: [Vec2; 2],
    /// 1: the ^ ^ of a finished job.
    happy: f32,
    /// 0..1 while the sparkle plays.
    sparkle: f32,
}

impl Pose {
    const REST: Pose = Pose { head: Vec2::ZERO, squash: Vec2::splat(1.0), gaze: Vec2::ZERO, eye: [Vec2::splat(1.0); 2], happy: 0.0, sparkle: 0.0 };

    fn lerp(a: Pose, b: Pose, t: f32) -> Pose {
        let l = |x: Vec2, y: Vec2| x + (y - x) * t;
        Pose {
            head: l(a.head, b.head),
            squash: l(a.squash, b.squash),
            gaze: l(a.gaze, b.gaze),
            eye: [l(a.eye[0], b.eye[0]), l(a.eye[1], b.eye[1])],
            happy: a.happy + (b.happy - a.happy) * t,
            sparkle: a.sparkle + (b.sparkle - a.sparkle) * t,
        }
    }
}

fn work_pose(w: Work, t: f32) -> Pose {
    let mut p = Pose::REST;
    match w {
        Read => {
            // Four hops along a line, then back to the start of the next.
            let local = (t / 2.0).fract();
            let x = if local < 0.86 {
                let v = local / 0.86 * 4.0;
                (v.floor() + ease(v.fract() / 0.25)) / 4.0
            } else {
                1.0 - ease((local - 0.86) / 0.14)
            };
            p.gaze = Vec2::new(-0.07 + 0.14 * x, 0.01);
            p.eye = [Vec2::new(1.0, 0.8); 2];
        }
        Type => {
            p.gaze = Vec2::new(0.012 * (t * 7.0).sin(), 0.055);
            p.head.y = 0.014 * (t * 13.0).sin().abs() * (0.5 + 0.5 * (t * 1.7).sin()).max(0.0);
            p.eye = [Vec2::new(1.0, 0.62); 2];
        }
        Think => {
            p.gaze = Vec2::new(0.05 * (t * 0.8).sin(), -0.085);
            p.eye = [Vec2::splat(1.0), Vec2::new(1.0, 0.6)];
            p.head.x = 0.012 * (t * 0.8).sin();
        }
        Scan => {
            p.gaze = Vec2::new(0.065 * (t * 2.4).cos(), -0.01 + 0.035 * (t * 2.4).sin());
        }
        Hum => {
            p.eye = [Vec2::new(1.0, 0.32); 2];
            p.head = Vec2::new(0.022 * (t * 2.8).sin(), -0.008 * (t * 5.6).sin().abs());
        }
    }
    p
}

/// The face's pose for a mood at time `t`, and how soon it next changes.
fn pose(face: &Face, mood: Mood, t: f64, phase: f64) -> (Pose, f64) {
    let mut p = Pose::REST;
    let mut next = f64::MAX;
    match mood {
        Mood::Working => {
            // Its own ways of working, five and a half seconds each, eased
            // into one another.
            let n = face.work.len();
            let x = t + phase * 7.0;
            let i = (x / 5.5).floor() as usize;
            let local = (x - (x / 5.5).floor() * 5.5) as f32;
            let tt = t as f32;
            let now = work_pose(face.work[i % n], tt);
            p = if local > 5.1 { Pose::lerp(now, work_pose(face.work[(i + 1) % n], tt), ease((local - 5.1) / 0.4)) } else { now };
            next = 1.0 / 30.0;
        }
        Mood::Idle => {
            let (i, u, wait) = episode(t, 6.5, 1.6, phase * 4.1);
            next = next.min(wait);
            if let Some(u) = u {
                match face.idle[i as usize % face.idle.len()] {
                    Glance => p.gaze.x = 0.075 * if i % 2 == 0 { 1.0 } else { -1.0 } * hold(u),
                    Breathe => {
                        let b = bump(u);
                        p.squash = Vec2::new(1.0 - 0.025 * b, 1.0 + 0.045 * b);
                        p.head.y = -0.012 * b;
                    }
                    Doze => {
                        let h = hold(u);
                        p.eye = [Vec2::new(1.0, 1.0 - 0.72 * h); 2];
                        p.head.y = 0.016 * h;
                    }
                }
                next = 1.0 / 30.0;
            }
        }
        Mood::NeedsYou => {
            p.gaze = Vec2::new(-face.gaze.0, -0.07 - face.gaze.1);
            p.eye = [Vec2::splat(1.22); 2];
            let (i, u, wait) = episode(t, 2.4, 0.8, phase);
            next = next.min(wait);
            if let Some(u) = u {
                let b = bump(u);
                match face.call[i as usize % face.call.len()] {
                    Bounce => {
                        p.head.y = -0.07 * b;
                        p.squash = Vec2::new(1.0 - 0.03 * b, 1.0 + 0.05 * b);
                    }
                    Wiggle => p.head.x = 0.04 * (u * 6.0 * std::f32::consts::PI).sin() * (1.0 - u),
                    Pulse => p.eye = [Vec2::splat(1.22 * (1.0 + 0.3 * b)); 2],
                }
                next = 1.0 / 30.0;
            }
        }
        Mood::Done => {
            p.happy = 1.0;
            let (i, u, wait) = episode(t, 3.0, 0.9, phase);
            next = next.min(wait);
            if let Some(u) = u {
                let b = bump(u);
                match face.cheer[i as usize % face.cheer.len()] {
                    Hop => {
                        p.head.y = -0.06 * b;
                        p.squash = Vec2::new(1.0 - 0.03 * b, 1.0 + 0.04 * b);
                    }
                    Sway => p.head.x = 0.035 * (u * 2.0 * std::f32::consts::PI).sin(),
                    Sparkle => {
                        p.sparkle = u;
                        p.head.y = -0.025 * b;
                    }
                }
                next = 1.0 / 30.0;
            }
        }
    }
    // Blinks, out of step between teammates (not while working or happy).
    if matches!(mood, Mood::Idle | Mood::NeedsYou) {
        let (_, u, wait) = episode(t, 4.3, 0.16, phase * 2.3);
        if let Some(u) = u {
            for e in &mut p.eye {
                e.y *= 1.0 - 0.94 * bump(u);
            }
            next = 1.0 / 30.0;
        } else {
            next = next.min(wait);
        }
    }
    (p, next)
}

fn eye_base(face: &Face) -> Vec2 {
    match face.eyes {
        Eyes::Capsule => Vec2::new(0.10, 0.25),
        Eyes::Dot => Vec2::new(0.13, 0.13),
        Eyes::Bar => Vec2::new(0.19, 0.075),
        Eyes::Square => Vec2::new(0.12, 0.12),
    }
}

/// The eyes at rest: (centre, size) each, in a box of side 1 at the origin.
fn rest_eyes(face: &Face) -> [(Vec2, Vec2); 2] {
    let b = eye_base(face);
    [(Vec2::new(-face.gap + face.gaze.0, face.gaze.1), b), (Vec2::new(face.gap + face.gaze.0, face.gaze.1), b * face.right)]
}

/// The head's size and radii (nw, ne, sw, se) in a box of side 1.
fn head_shape(face: &Face) -> (Vec2, [f32; 4]) {
    match face.head {
        Head::Rect(w, h, r) => (Vec2::new(w, h), r),
        Head::Round(d) => (Vec2::splat(d), [d / 2.0; 4]),
    }
}

/// Paint a face filling `rect`. `head` is its colour; `phase` puts teammates
/// out of step with each other.
pub fn paint_face(p: &egui::Painter, rect: Rect, face: usize, mood: Mood, head: Color32, phase: f64) {
    let face = &FACES[face % FACES.len()];
    let s = rect.width();
    let t = face_time(p.ctx());
    let (pose, next) = pose(face, mood, t, phase);

    let (size, radii) = head_shape(face);
    let centre = rect.center() + pose.head * s;
    let hr = Rect::from_center_size(centre, size * pose.squash * s);
    let k = pose.squash.x.min(pose.squash.y) * s;
    p.rect_filled(hr, CornerRadius { nw: corner(radii[0] * k), ne: corner(radii[1] * k), sw: corner(radii[2] * k), se: corner(radii[3] * k) }, head);

    let eyes = rest_eyes(face);
    if pose.happy > 0.5 {
        // ^ ^
        for (c, _) in eyes {
            let e = centre + (c + pose.gaze + Vec2::new(0.0, -0.01)) * s;
            let (w, h) = (0.075 * s, 0.06 * s);
            p.add(Shape::line(vec![e + Vec2::new(-w, h / 2.0), e + Vec2::new(0.0, -h / 2.0), e + Vec2::new(w, h / 2.0)], Stroke::new((0.045 * s).max(1.2), EYE_INK)));
        }
    } else {
        for (i, (c, base)) in eyes.into_iter().enumerate() {
            let size = base * pose.eye[i] * s;
            let size = Vec2::new(size.x, size.y.max(0.03 * s));
            let r = match face.eyes {
                Eyes::Square => size.x.min(size.y) * 0.28,
                _ => size.x.min(size.y) / 2.0,
            };
            p.rect_filled(Rect::from_center_size(centre + (c + pose.gaze) * s, size), corner(r), EYE_INK);
        }
    }
    if pose.sparkle > 0.0 {
        // A four-pointed glint off the top corner: two thin diamonds, crossed.
        let g = bump(pose.sparkle);
        let at = hr.right_top() + Vec2::new(-0.02, 0.04) * s + Vec2::new(0.02, -0.03) * s * pose.sparkle;
        let (long, thin) = (0.11 * s * g, 0.025 * s * g);
        for (a, b) in [(Vec2::new(long, 0.0), Vec2::new(0.0, thin)), (Vec2::new(0.0, long), Vec2::new(thin, 0.0))] {
            p.add(Shape::convex_polygon(vec![at - a, at - b, at + a, at + b], head, Stroke::NONE));
        }
    }

    // Wake for the next move, or every frame while one plays.
    if CLOCK.load(std::sync::atomic::Ordering::Relaxed) == u64::MAX && next < f64::MAX {
        p.ctx().request_repaint_after(std::time::Duration::from_secs_f64(next.max(1.0 / 60.0)));
    }
}

/// A face as it would look on this teammate — `face` overrides the one the
/// name would get (the picker's preview).
pub fn face_preview(ui: &mut egui::Ui, name: &str, face: Option<usize>, size: f32, mood: Mood) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    paint_face(ui.painter(), rect, face.unwrap_or_else(|| face_of(name)), mood, head_shade(name), 0.0);
    resp
}

/// A teammate's avatar: its face, showing its mood.
pub fn avatar(ui: &mut egui::Ui, name: &str, size: f32, mood: Mood) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    paint_avatar(ui.painter(), rect, name, mood);
    resp
}

pub fn paint_avatar(p: &egui::Painter, rect: Rect, name: &str, mood: Mood) {
    paint_face(p, rect, face_of(name), mood, head_shade(name), hue_deg(&name.trim().to_lowercase()) as f64 / 37.0);
}

/// The full app icon: Glance on its black tile, as in `assets/icon.svg`.
pub fn app_tile(ui: &mut egui::Ui, size: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(corner(114.0 / 512.0 * size)), MARK_TILE);
    let face = Rect::from_center_size(rect.center(), Vec2::splat(ICON_FACE * size));
    paint_face(p, face, GLANCE, Mood::Idle, MARK_INK, 0.0);
    resp
}

/// The face's box as a fraction of the icon tile (assets/icon.svg: the head is
/// 300 of 512 wide, 0.84 of this box).
const ICON_FACE: f32 = 0.6975;

/// Is (x, y) — in a face box of side 1 centred on the origin — inside a rounded
/// rectangle centred at `c` with this size and these radii (nw, ne, sw, se)?
fn in_round_rect(x: f32, y: f32, c: (f32, f32), size: (f32, f32), r: [f32; 4]) -> bool {
    let (dx, dy) = (x - c.0, y - c.1);
    let (hw, hh) = (size.0 / 2.0, size.1 / 2.0);
    if dx.abs() > hw || dy.abs() > hh {
        return false;
    }
    let r = match (dx < 0.0, dy < 0.0) {
        (true, true) => r[0],
        (false, true) => r[1],
        (true, false) => r[2],
        (false, false) => r[3],
    }
    .min(hw)
    .min(hh);
    let (qx, qy) = (dx.abs() - (hw - r), dy.abs() - (hh - r));
    qx <= 0.0 || qy <= 0.0 || qx * qx + qy * qy <= r * r
}

/// The window/Dock icon, rasterised here (no image files at runtime).
/// Supersampled 4×4 so the edges are smooth. `margin` is the transparent
/// border as a fraction of the size: macOS icons sit on a grid with the tile
/// at 824/1024 (≈0.1 each side); elsewhere the tile fills the square.
pub fn icon_rgba(n: usize, margin: f32) -> Vec<u8> {
    let inner = n as f32 * (1.0 - 2.0 * margin);
    let offset = n as f32 * margin;
    let face = &FACES[GLANCE];
    let (head, hr) = head_shape(face);
    let (hw, hh) = (head.x, head.y);
    let eyes = rest_eyes(face);
    let rgb = |c: Color32| [c.r() as f32, c.g() as f32, c.b() as f32];
    let (tile, ink, eye) = (rgb(MARK_TILE), rgb(MARK_INK), rgb(EYE_INK));

    let mut out = vec![0u8; n * n * 4];
    const SS: usize = 4;
    for py in 0..n {
        for px in 0..n {
            let mut acc = [0.0f32; 4];
            for sy in 0..SS {
                for sx in 0..SS {
                    // Tile space: 0..1 across the tile.
                    let x = (px as f32 + (sx as f32 + 0.5) / SS as f32 - offset) / inner;
                    let y = (py as f32 + (sy as f32 + 0.5) / SS as f32 - offset) / inner;
                    if !in_round_rect(x, y, (0.5, 0.5), (1.0, 1.0), [114.0 / 512.0; 4]) {
                        continue;
                    }
                    // Face space: a box of side 1 centred on the origin.
                    let (fx, fy) = ((x - 0.5) / ICON_FACE, (y - 0.5) / ICON_FACE);
                    let mut c = tile;
                    if in_round_rect(fx, fy, (0.0, 0.0), (hw, hh), hr) {
                        c = ink;
                        for (centre, size) in eyes {
                            if in_round_rect(fx, fy, (centre.x, centre.y), (size.x, size.y), [size.x / 2.0; 4]) {
                                c = eye;
                            }
                        }
                    }
                    for k in 0..3 {
                        acc[k] += c[k];
                    }
                    acc[3] += 1.0;
                }
            }
            let i = (py * n + px) * 4;
            if acc[3] > 0.0 {
                for k in 0..3 {
                    out[i + k] = (acc[k] / acc[3]).round().clamp(0.0, 255.0) as u8;
                }
                out[i + 3] = (acc[3] / (SS * SS) as f32 * 255.0).round() as u8;
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
    fn tiles_are_grey_and_keep_the_1x_hash() {
        // 1.x: hueOf("researcher") with h = (h*31 + code) % 360.
        let h = "researcher".chars().fold(0u32, |h, c| (h * 31 + c as u32) % 360);
        assert_eq!(hue_deg("researcher"), h as f32);
        for name in ["Researcher", "Ops", "Shopper", "Inbox", "Scout"] {
            let (top, bottom, ink) = tile_colors(name);
            for c in [top, bottom, ink] {
                assert!(c.r() == c.g() && c.g() == c.b(), "{name}: {c:?} is not grey");
            }
            // Ink always reads on its tile.
            assert!((top.r() as i32 - ink.r() as i32).abs() > 110, "{name}: {top:?} vs {ink:?}");
        }
    }

    #[test]
    fn icon_is_opaque_inside_and_clear_at_the_corner() {
        let n = 64;
        let px = icon_rgba(n, 0.0);
        assert_eq!(px.len(), n * n * 4);
        assert_eq!(px[3], 0, "the rounded corner is transparent");
        // Low in the face, below the eyes: the white head.
        let head = ((n * 3 / 4) * n + n / 2) * 4;
        assert_eq!(px[head + 3], 255, "the head is solid");
        assert!(px[head] > 200 && px[head] == px[head + 2], "the head is white");
        // The left eye: 0.5 + (-0.132 + 0.053) * 0.6975 across, 0.5 - 0.078 * 0.6975 down.
        let eye = (((0.5 - 0.078 * ICON_FACE) * n as f32) as usize * n + ((0.5 - 0.079 * ICON_FACE) * n as f32) as usize) * 4;
        assert!(px[eye] < 40, "the eye is dark");
        let tile = ((n / 2) * n + n / 16) * 4;
        assert!(px[tile] < 20, "the tile is black");
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
