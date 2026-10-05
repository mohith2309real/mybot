//! The desktop window: a roster of teammates on the left, the conversation in
//! the middle, the agent's computer on the right — with sign-in approvals,
//! skills, routines, logins and settings one click away. Pure Rust (egui), no
//! web view anywhere.

mod chat;
mod computer;
mod screens;
pub mod state;
pub mod theme;

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{Align, Align2, Color32, Id, Layout, Margin, RichText, Sense, Stroke, Vec2};
use mybot_core::agent::AgentEvent;
use mybot_core::db::{Bot, LoginRequest};

use crate::engine::Engine;
use state::Threads;
use theme::*;

pub type Sink = crate::engine::EventSink;

/// Work started from the UI and finished on the runtime; the UI polls it.
pub struct Job<T>(Arc<Mutex<Option<T>>>);

impl<T: Send + 'static> Job<T> {
    pub fn spawn(rt: &tokio::runtime::Handle, ctx: &egui::Context, fut: impl Future<Output = T> + Send + 'static) -> Self {
        let slot = Arc::new(Mutex::new(None));
        let (s, ctx) = (slot.clone(), ctx.clone());
        rt.spawn(async move {
            let v = fut.await;
            *s.lock().unwrap() = Some(v);
            ctx.request_repaint();
        });
        Self(slot)
    }

    /// For CPU-bound or blocking work (key derivation, file imports).
    pub fn blocking(rt: &tokio::runtime::Handle, ctx: &egui::Context, f: impl FnOnce() -> T + Send + 'static) -> Self {
        let slot = Arc::new(Mutex::new(None));
        let (s, ctx) = (slot.clone(), ctx.clone());
        rt.spawn_blocking(move || {
            let v = f();
            *s.lock().unwrap() = Some(v);
            ctx.request_repaint();
        });
        Self(slot)
    }

    pub fn take(&self) -> Option<T> {
        self.0.lock().unwrap().take()
    }
}

/// Poll an optional job; clears it when done.
pub fn poll<T: Send + 'static>(job: &mut Option<Job<T>>) -> Option<T> {
    let v = job.as_ref()?.take();
    if v.is_some() {
        *job = None;
    }
    v
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Chat,
    Skills,
    Routines,
    Logins,
    Settings,
}

struct Unlock {
    pass: String,
    confirm: String,
    remember: bool,
    error: Option<String>,
    job: Option<Job<Result<(), String>>>,
}

struct Toast {
    text: String,
    bad: bool,
    at: Instant,
}

pub struct App {
    engine: Arc<Engine>,
    rt: tokio::runtime::Handle,
    threads: Threads,
    sink: Sink,
    view: View,
    bots: Vec<Bot>,
    selected: Option<String>,
    drafts: HashMap<String, String>,
    show_computer: bool,
    unlock: Option<Unlock>,
    toasts: Vec<Toast>,
    login_requests: Vec<LoginRequest>,
    last_poll: Instant,
    last_tick: Instant,
    chat: chat::ChatState,
    computer: computer::ComputerPanel,
    screens: screens::Screens,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, engine: Arc<Engine>, rt: tokio::runtime::Handle) -> Self {
        theme::apply(&cc.egui_ctx);
        let threads = Threads::default();
        let ctx = cc.egui_ctx.clone();
        let th = threads.clone();
        let sink: Sink = Arc::new(move |bot: &str, e: AgentEvent| {
            th.apply(bot, e);
            ctx.request_repaint();
        });
        let unlocked = engine.auto_unlock();
        let selected = cc.storage.and_then(|s| s.get_string("selected"));
        let show_computer = cc.storage.and_then(|s| s.get_string("show_computer")).is_some_and(|v| v == "1");
        let mut app = Self {
            unlock: (!unlocked).then(|| Unlock { pass: String::new(), confirm: String::new(), remember: false, error: None, job: None }),
            engine,
            rt,
            threads,
            sink,
            view: View::Chat,
            bots: vec![],
            selected,
            drafts: HashMap::new(),
            show_computer,
            toasts: vec![],
            login_requests: vec![],
            last_poll: Instant::now() - Duration::from_secs(10),
            last_tick: Instant::now(),
            chat: Default::default(),
            computer: Default::default(),
            screens: Default::default(),
        };
        app.reload_bots();
        app
    }

    fn reload_bots(&mut self) {
        self.bots = self.engine.db.bots().unwrap_or_default();
        if self.selected.as_ref().is_none_or(|id| !self.bots.iter().any(|b| &b.id == id)) {
            self.selected = self.bots.first().map(|b| b.id.clone());
        }
    }

    fn bot(&self) -> Option<&Bot> {
        let id = self.selected.as_ref()?;
        self.bots.iter().find(|b| &b.id == id)
    }

    pub fn toast(&mut self, text: impl Into<String>) {
        self.toasts.push(Toast { text: text.into(), bad: false, at: Instant::now() });
    }

    pub fn toast_err(&mut self, text: impl Into<String>) {
        self.toasts.push(Toast { text: text.into(), bad: true, at: Instant::now() });
    }

    pub fn open_chat(&mut self, bot_id: &str) {
        self.selected = Some(bot_id.to_string());
        self.view = View::Chat;
    }

    /// Housekeeping that runs every frame but does real work rarely.
    fn background(&mut self, ctx: &egui::Context) {
        if self.last_poll.elapsed() > Duration::from_millis(900) {
            self.last_poll = Instant::now();
            let before = self.login_requests.len();
            self.login_requests = self.engine.approvals.pending();
            if self.login_requests.len() > before {
                ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
            }
        }
        if self.last_tick.elapsed() > Duration::from_secs(30) && self.unlock.is_none() {
            self.last_tick = Instant::now();
            let n = self.engine.tick_routines(&self.rt, self.sink.clone());
            if n > 0 {
                self.toast(format!("{n} scheduled routine{} started.", if n == 1 { "" } else { "s" }));
            }
        }
        ctx.request_repaint_after(Duration::from_secs(1));
    }

    // --- unlock -----------------------------------------------------------------

    fn unlock_screen(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let first_run = !self.engine.keys.exists() && !self.engine.logins.exists();
        let Some(u) = self.unlock.as_mut() else { return };
        if let Some(r) = poll(&mut u.job) {
            match r {
                Ok(()) => {
                    self.unlock = None;
                    return;
                }
                Err(e) => u.error = Some(e),
            }
        }
        let mut skip = false;
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(BG)).show(ui, |ui| {
            ui.add_space((ui.available_height() * 0.22).max(24.0));
            ui.vertical_centered(|ui| {
                theme::mark(ui, 64.0);
                ui.add_space(10.0);
                ui.label(RichText::new("MyBot").size(30.0).strong());
                ui.label(muted(if first_run { "Choose a passphrase. It encrypts your API keys and saved logins on this computer." } else { "Enter your passphrase to unlock your keys and saved logins." }));
                ui.add_space(18.0);
                ui.allocate_ui(Vec2::new(360.0, 260.0), |ui| {
                    card().show(ui, |ui| {
                        ui.set_width(330.0);
                        let busy = u.job.is_some();
                        let r = ui.add(input(&mut u.pass).password(true).hint_text("Passphrase").desired_width(f32::INFINITY));
                        if !busy && u.pass.is_empty() && u.error.is_none() {
                            r.request_focus();
                        }
                        if first_run {
                            ui.add(input(&mut u.confirm).password(true).hint_text("Type it again").desired_width(f32::INFINITY));
                        }
                        if mybot_vault::keychain::available() {
                            ui.checkbox(&mut u.remember, "Remember on this computer (system keychain)");
                        }
                        if let Some(e) = &u.error {
                            ui.label(RichText::new(e).color(BAD));
                        }
                        ui.add_space(4.0);
                        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let go = ui.add_enabled(!busy, theme::accent_button(if busy { "Unlocking…" } else if first_run { "Create and unlock" } else { "Unlock" }).min_size(Vec2::new(ui.available_width(), 34.0))).clicked();
                        if (go || enter) && !busy {
                            if first_run && u.pass != u.confirm {
                                u.error = Some("The two passphrases differ.".into());
                            } else if first_run && u.pass.chars().count() < 8 {
                                u.error = Some("Use at least 8 characters.".into());
                            } else {
                                u.error = None;
                                let (engine, pass, remember) = (self.engine.clone(), std::mem::take(&mut u.pass), u.remember);
                                u.confirm.clear();
                                u.job = Some(Job::blocking(&self.rt, &ctx, move || {
                                    engine.unlock(&pass)?;
                                    if remember {
                                        mybot_vault::keychain::remember(&pass).map_err(|e| format!("Unlocked, but the keychain refused: {e}"))?;
                                    }
                                    Ok(())
                                }));
                            }
                        }
                    });
                });
                ui.add_space(8.0);
                ui.label(small("Keys set in the environment (ANTHROPIC_API_KEY, …) work without unlocking."));
                if ui.add(egui::Button::new(small("Continue without unlocking")).frame(false)).clicked() {
                    skip = true;
                }
            });
        });
        if skip {
            self.unlock = None;
        }
    }

    // --- left rail -------------------------------------------------------------------

    fn rail(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("rail")
            .exact_size(256.0)
            .resizable(false)
            .frame(egui::Frame::NONE.fill(RAIL).inner_margin(Margin { left: 12, right: 12, top: 14, bottom: 12 }).stroke(Stroke::new(1.0, LINE)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    theme::mark(ui, 26.0);
                    ui.label(RichText::new("MyBot").size(18.0).strong());
                    ui.label(small("2.0"));
                });
                ui.add_space(12.0);
                if ui.add(theme::accent_button("+  New teammate").min_size(Vec2::new(ui.available_width(), 34.0))).clicked() {
                    self.chat.open_new_bot(&self.engine);
                }
                ui.add_space(14.0);
                ui.label(small("TEAMMATES"));
                ui.add_space(2.0);

                let nav_h = 4.0 * 34.0 + 40.0;
                egui::ScrollArea::vertical().max_height((ui.available_height() - nav_h).max(80.0)).auto_shrink([false, true]).show(ui, |ui| {
                    if self.bots.is_empty() {
                        ui.label(muted("No teammates yet."));
                    }
                    let mut pick = None;
                    for b in &self.bots {
                        let (working, waiting) = self.threads.with(&b.id, |t| (t.working, t.waiting.is_some()));
                        let running = self.engine.is_running(&b.id);
                        let dot = if waiting { Some(WARN) } else if running || working { Some(GOOD) } else { None };
                        let sub = if waiting { "Needs you".to_string() } else if running { "Working…".to_string() } else { format!("{} · {}", mybot_core::providers::family_label(&b.provider), b.model) };
                        let sel = self.view == View::Chat && self.selected.as_deref() == Some(b.id.as_str());
                        if roster_row(ui, &b.name, &sub, dot, sel, waiting).clicked() {
                            pick = Some(b.id.clone());
                        }
                    }
                    if let Some(id) = pick {
                        self.open_chat(&id);
                    }
                });

                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    ui.horizontal(|ui| {
                        let unlocked = self.engine.is_unlocked();
                        ui.label(small(if unlocked { "🔓 Unlocked" } else { "🔒 Locked" }));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if unlocked {
                                if ui.add(egui::Button::new(small("Lock")).frame(false)).clicked() {
                                    self.engine.lock();
                                    self.unlock = Some(Unlock { pass: String::new(), confirm: String::new(), remember: false, error: None, job: None });
                                }
                            } else if ui.add(egui::Button::new(small("Unlock")).frame(false)).clicked() {
                                self.unlock = Some(Unlock { pass: String::new(), confirm: String::new(), remember: false, error: None, job: None });
                            }
                        });
                    });
                    ui.add_space(6.0);
                    let nav = [(View::Settings, "⚙  Settings"), (View::Logins, "🔑  Logins"), (View::Routines, "⏰  Routines"), (View::Skills, "✨  Skills")];
                    for (v, label) in nav {
                        let badge = match v {
                            View::Logins if !self.login_requests.is_empty() => Some(self.login_requests.len()),
                            _ => None,
                        };
                        if nav_row(ui, label, self.view == v, badge).clicked() {
                            self.view = v;
                        }
                    }
                });
            });
    }

    // --- overlays --------------------------------------------------------------------

    /// Sign-in requests, top right, above everything.
    fn approvals(&mut self, ctx: &egui::Context) {
        if self.login_requests.is_empty() {
            return;
        }
        let mut decided: Option<(String, Result<LoginRequest, String>)> = None;
        egui::Area::new(Id::new("approvals")).anchor(Align2::RIGHT_TOP, Vec2::new(-16.0, 14.0)).order(egui::Order::Foreground).show(ctx, |ui| {
            ui.set_width(360.0);
            for r in self.login_requests.clone() {
                egui::Frame::NONE.fill(PANEL).stroke(Stroke::new(1.0, ACCENT)).corner_radius(12).inner_margin(Margin::same(14)).shadow(egui::Shadow { offset: [0, 6], blur: 24, spread: 0, color: Color32::from_black_alpha(140) }).show(ui, |ui| {
                    ui.set_width(332.0);
                    ui.horizontal(|ui| {
                        avatar(ui, &r.bot, 26.0, None);
                        ui.label(RichText::new(format!("{} wants to sign in", r.bot)).strong());
                    });
                    let site = mybot_catalog::site_for_origin(&r.origin).map(|s| s.name.clone()).unwrap_or_else(|| r.origin.clone());
                    ui.label(RichText::new(format!("to {site} as {}", r.username)).color(TEXT2));
                    ui.label(small(&r.origin));
                    ui.label(small("Your password is typed into the page by MyBot. The bot never sees it."));
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui.add(theme::accent_button("Allow once")).clicked() {
                            decided = Some(("Allowed once.".into(), self.engine.approvals.allow(&r.id, false)));
                        }
                        if ui.add(theme::ghost_button("Always for this site")).clicked() {
                            decided = Some(("Allowed — this site won't ask again.".into(), self.engine.approvals.allow(&r.id, true)));
                        }
                        if ui.add(theme::danger_button("Deny")).clicked() {
                            decided = Some(("Denied.".into(), self.engine.approvals.deny(&r.id)));
                        }
                    });
                });
                ui.add_space(8.0);
            }
        });
        if let Some((msg, r)) = decided {
            match r {
                Ok(_) => self.toast(msg),
                Err(e) => self.toast_err(e),
            }
            self.login_requests = self.engine.approvals.pending();
        }
    }

    fn toasts(&mut self, ctx: &egui::Context) {
        self.toasts.retain(|t| t.at.elapsed() < Duration::from_secs(if t.bad { 8 } else { 4 }));
        if self.toasts.is_empty() {
            return;
        }
        egui::Area::new(Id::new("toasts")).anchor(Align2::CENTER_BOTTOM, Vec2::new(0.0, -96.0)).order(egui::Order::Tooltip).interactable(false).show(ctx, |ui| {
            for t in &self.toasts {
                egui::Frame::NONE.fill(RAISED).stroke(Stroke::new(1.0, if t.bad { BAD } else { LINE })).corner_radius(10).inner_margin(Margin::symmetric(14, 9)).show(ui, |ui| {
                    ui.label(RichText::new(&t.text).color(if t.bad { BAD } else { TEXT }));
                });
                ui.add_space(6.0);
            }
        });
        ctx.request_repaint_after(Duration::from_millis(500));
    }
}

fn roster_row(ui: &mut egui::Ui, name: &str, sub: &str, dot: Option<Color32>, selected: bool, attention: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 50.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 10, RAISED);
    } else if resp.hovered() {
        p.rect_filled(rect, 10, Color32::from_rgb(28, 28, 33));
    }
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(Vec2::new(8.0, 7.0))).layout(Layout::left_to_right(Align::Center)));
    avatar(&mut child, name, 34.0, dot);
    child.add_space(4.0);
    child.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        ui.label(RichText::new(name).strong().color(TEXT));
        ui.label(RichText::new(sub).size(12.0).color(if attention { WARN } else { MUTED }));
    });
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn nav_row(ui: &mut egui::Ui, label: &str, selected: bool, badge: Option<usize>) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 8, RAISED);
    } else if resp.hovered() {
        p.rect_filled(rect, 8, Color32::from_rgb(28, 28, 33));
    }
    p.text(rect.left_center() + Vec2::new(10.0, 0.0), Align2::LEFT_CENTER, label, egui::FontId::proportional(14.0), if selected { TEXT } else { TEXT2 });
    if let Some(n) = badge {
        let c = rect.right_center() - Vec2::new(16.0, 0.0);
        p.circle_filled(c, 9.0, ACCENT);
        p.text(c, Align2::CENTER_CENTER, n.to_string(), egui::FontId::proportional(11.0), ACCENT_INK);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.unlock.is_some() {
            self.unlock_screen(ui);
            return;
        }
        self.background(&ctx);
        self.rail(ui);
        match self.view {
            View::Chat => {
                if self.show_computer && self.bot().is_some() {
                    self.computer_panel(ui);
                }
                self.chat_view(ui);
            }
            View::Skills => self.skills_view(ui),
            View::Routines => self.routines_view(ui),
            View::Logins => self.logins_view(ui),
            View::Settings => self.settings_view(ui),
        }
        self.chat_modals(&ctx);
        self.screen_modals(&ctx);
        self.teach_modal(&ctx);
        self.approvals(&ctx);
        self.toasts(&ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Some(s) = &self.selected {
            storage.set_string("selected", s.clone());
        }
        storage.set_string("show_computer", if self.show_computer { "1" } else { "0" }.into());
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Leave nothing half-done: stop runs so their rows are closed out.
        for r in self.engine.runs.lock().unwrap().values() {
            r.cancel.cancel();
        }
    }
}

/// The window icon: MyBot's hexagon, drawn here (no image files).
fn icon() -> egui::IconData {
    let n = 64usize;
    let mut rgba = vec![0u8; n * n * 4];
    let c = (n as f32 - 1.0) / 2.0;
    let inside = |x: f32, y: f32, r: f32| -> bool {
        // Pointy-top hexagon: |y| ≤ r and the two slanted edges.
        let (dx, dy) = ((x - c).abs(), (y - c).abs());
        dx <= r * 0.866 && dy <= r - dx * 0.577
    };
    for y in 0..n {
        for x in 0..n {
            let (fx, fy) = (x as f32, y as f32);
            let ring = inside(fx, fy, 30.0) && !inside(fx, fy, 25.0);
            let core = inside(fx, fy, 13.0);
            if ring || core {
                let i = (y * n + x) * 4;
                rgba[i..i + 4].copy_from_slice(&[245, 166, 35, 255]);
            }
        }
    }
    egui::IconData { rgba, width: n as u32, height: n as u32 }
}

pub fn run(engine: Arc<Engine>, rt: tokio::runtime::Handle) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("MyBot")
            .with_app_id("mybot")
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([900.0, 560.0])
            .with_icon(icon()),
        ..Default::default()
    };
    eframe::run_native("MyBot", options, Box::new(move |cc| Ok(Box::new(App::new(cc, engine, rt)))))
}
