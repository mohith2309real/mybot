//! Look and feel: a dark, quiet canvas with one warm accent — the layout of
//! a team messenger (roster · conversation · agent computer), MyBot's own
//! name and mark.

use egui::{Color32, CornerRadius, FontId, Margin, RichText, Stroke, TextStyle, Vec2};

pub const BG: Color32 = Color32::from_rgb(13, 13, 15);
pub const RAIL: Color32 = Color32::from_rgb(19, 19, 22);
pub const PANEL: Color32 = Color32::from_rgb(26, 26, 30);
pub const RAISED: Color32 = Color32::from_rgb(34, 34, 40);
pub const LINE: Color32 = Color32::from_rgb(44, 44, 52);
pub const TEXT: Color32 = Color32::from_rgb(236, 236, 241);
pub const TEXT2: Color32 = Color32::from_rgb(196, 196, 206);
pub const MUTED: Color32 = Color32::from_rgb(142, 142, 153);
pub const FAINT: Color32 = Color32::from_rgb(98, 98, 110);
pub const ACCENT: Color32 = Color32::from_rgb(245, 166, 35);
pub const ACCENT_INK: Color32 = Color32::from_rgb(28, 20, 6);
pub const ACCENT_WASH: Color32 = Color32::from_rgba_premultiplied(40, 28, 8, 220);
pub const GOOD: Color32 = Color32::from_rgb(76, 194, 128);
pub const WARN: Color32 = Color32::from_rgb(240, 160, 64);
pub const BAD: Color32 = Color32::from_rgb(235, 87, 87);
pub const THINK: Color32 = Color32::from_rgb(150, 140, 190);

pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.global_style()).clone();
    style.text_styles = [
        (TextStyle::Heading, FontId::proportional(20.0)),
        (TextStyle::Body, FontId::proportional(14.5)),
        (TextStyle::Button, FontId::proportional(14.0)),
        (TextStyle::Small, FontId::proportional(12.0)),
        (TextStyle::Monospace, FontId::monospace(13.0)),
    ]
    .into();
    style.spacing.item_spacing = Vec2::new(8.0, 7.0);
    style.spacing.button_padding = Vec2::new(12.0, 6.0);
    style.spacing.interact_size.y = 30.0;
    // Text in the thread opts in with `.selectable(true)`; everywhere else a
    // label must not swallow the click meant for the row it sits on.
    style.interaction.selectable_labels = false;

    let v = &mut style.visuals;
    *v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.extreme_bg_color = Color32::from_rgb(10, 10, 12);
    v.faint_bg_color = PANEL;
    v.window_stroke = Stroke::new(1.0, LINE);
    v.window_corner_radius = CornerRadius::same(14);
    v.menu_corner_radius = CornerRadius::same(10);
    v.selection.bg_fill = Color32::from_rgb(92, 64, 16);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.hyperlink_color = ACCENT;
    v.override_text_color = Some(TEXT);
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open, &mut v.widgets.noninteractive] {
        w.corner_radius = CornerRadius::same(9);
    }
    v.widgets.inactive.weak_bg_fill = RAISED;
    v.widgets.inactive.bg_fill = RAISED;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(44, 44, 52);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(70, 70, 80));
    v.widgets.active.weak_bg_fill = Color32::from_rgb(54, 54, 62);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    ctx.set_global_style(style);
}

/// A stable hue per name, for avatars.
pub fn hue(name: &str) -> Color32 {
    let h: u32 = name.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    let palette = [
        Color32::from_rgb(197, 120, 221),
        Color32::from_rgb(96, 165, 250),
        Color32::from_rgb(52, 211, 153),
        Color32::from_rgb(251, 146, 60),
        Color32::from_rgb(244, 114, 182),
        Color32::from_rgb(250, 204, 21),
        Color32::from_rgb(129, 140, 248),
        Color32::from_rgb(45, 212, 191),
    ];
    palette[(h % palette.len() as u32) as usize]
}

pub fn initials(name: &str) -> String {
    let parts: Vec<&str> = name.split(|c: char| !c.is_alphanumeric()).filter(|p| !p.is_empty()).collect();
    match parts.as_slice() {
        [a, b, ..] => format!("{}{}", a.chars().next().unwrap_or(' '), b.chars().next().unwrap_or(' ')).to_uppercase(),
        [a] => a.chars().take(2).collect::<String>().to_uppercase(),
        [] => "?".into(),
    }
}

/// Avatar: rounded square, initials, optional state dot.
pub fn avatar(ui: &mut egui::Ui, name: &str, size: f32, state: Option<Color32>) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same((size * 0.28) as u8), hue(name));
    p.text(rect.center(), egui::Align2::CENTER_CENTER, initials(name), FontId::proportional(size * 0.36), Color32::from_rgb(20, 16, 24));
    if let Some(c) = state {
        let center = rect.right_bottom() - Vec2::splat(size * 0.1);
        p.circle_filled(center, size * 0.15, BG);
        p.circle_filled(center, size * 0.1, c);
    }
    resp
}

/// A one-line text field with room to breathe.
pub fn input(s: &mut String) -> egui::TextEdit<'_> {
    egui::TextEdit::singleline(s).margin(Margin::symmetric(8, 5))
}

pub fn accent_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).color(ACCENT_INK).strong()).fill(ACCENT).corner_radius(CornerRadius::same(9))
}

pub fn ghost_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).color(TEXT2)).fill(Color32::TRANSPARENT).stroke(Stroke::new(1.0, LINE))
}

pub fn danger_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).color(BAD)).fill(Color32::TRANSPARENT).stroke(Stroke::new(1.0, Color32::from_rgb(90, 40, 40)))
}

pub fn card() -> egui::Frame {
    egui::Frame::NONE.fill(PANEL).stroke(Stroke::new(1.0, LINE)).corner_radius(CornerRadius::same(12)).inner_margin(Margin::same(14))
}

pub fn muted(t: impl Into<String>) -> RichText {
    RichText::new(t).color(MUTED)
}

pub fn small(t: impl Into<String>) -> RichText {
    RichText::new(t).color(MUTED).size(12.0)
}

pub fn heading(t: impl Into<String>) -> RichText {
    RichText::new(t).size(19.0).strong().color(TEXT)
}

/// The MyBot mark: a hexagon with a filled core (never anyone else's logo).
pub fn mark(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    let c = rect.center();
    let r = size * 0.46;
    let hex = |rr: f32| -> Vec<egui::Pos2> {
        (0..6).map(|i| {
            let a = std::f32::consts::FRAC_PI_3 * i as f32 - std::f32::consts::FRAC_PI_2;
            c + Vec2::new(a.cos(), a.sin()) * rr
        }).collect()
    };
    ui.painter().add(egui::Shape::closed_line(hex(r), Stroke::new(1.6, ACCENT)));
    ui.painter().add(egui::Shape::convex_polygon(hex(r * 0.46), ACCENT, Stroke::NONE));
}
