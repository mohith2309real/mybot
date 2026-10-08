//! The desktop window: a roster of teammates on the left, the conversation in
//! the middle, the agent's computer on the right — with sign-in approvals,
//! skills, routines, logins and settings one click away. Pure Rust (egui), no
//! web view anywhere.

mod chat;
mod computer;
mod screens;
mod snapshot;
pub mod state;
pub mod theme;

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{Align, Align2, Color32, Id, Layout, Margin, Pos2, RichText, Sense, Stroke, Vec2};
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
    /// Per-teammate preview line and time for the roster, refreshed every
    /// couple of seconds rather than queried every frame.
    roster: HashMap<String, RosterMeta>,
    last_roster: Instant,
    chat: chat::ChatState,
    computer: computer::ComputerPanel,
    screens: screens::Screens,
    snapshot: Option<snapshot::Snapshot>,
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
            roster: HashMap::new(),
            last_roster: Instant::now() - Duration::from_secs(10),
            chat: Default::default(),
            computer: Default::default(),
            screens: Default::default(),
            snapshot: snapshot::Snapshot::from_env(),
        };
        app.reload_bots();
        app.stage_snapshot();
        app
    }

    /// Put the window in the state a snapshot asked for (see `snapshot.rs`).
    fn stage_snapshot(&mut self) {
        let Some(snap) = &self.snapshot else { return };
        let (view, bot) = (snap.view.clone(), snap.bot.clone());
        if let Some(name) = bot
            && let Some(b) = self.bots.iter().find(|b| b.name.eq_ignore_ascii_case(&name))
        {
            self.selected = Some(b.id.clone());
        }
        self.show_computer = matches!(view.as_str(), "computer" | "needs-you");
        match view.as_str() {
            "skills" => self.view = View::Skills,
            "imported" => {
                self.view = View::Skills;
                self.screens.show_imported();
            }
            "routines" => self.view = View::Routines,
            "logins" => self.view = View::Logins,
            "settings" => self.view = View::Settings,
            "new-teammate" => self.chat.open_new_bot(&self.engine),
            "needs-you" => {
                if let Some(id) = self.selected.clone() {
                    self.threads.load(&self.engine.db, &id);
                    let pause = mybot_core::agent::Pause {
                        kind: "two_factor".into(),
                        reason: "The bank wants a 6-digit code from your phone. Type it into the page, then hand back.".into(),
                        url: "https://online.examplebank.com/verify".into(),
                        page_level: true,
                        confirm: false,
                    };
                    self.threads.with(&id, |t| {
                        t.items.push(state::Item::You("Download last month's bank statement".into()));
                        t.items.push(state::Item::Step { name: "page_navigate".into(), input: "url: https://online.examplebank.com".into(), result: Some(("Opened".into(), false)) });
                        t.items.push(state::Item::Step { name: "fill_login".into(), input: "{\"username_ref\":4}".into(), result: Some(("Signed in".into(), false)) });
                        t.items.push(state::Item::Paused { pause: pause.clone(), task_id: Some("snapshot".into()) });
                        t.waiting = Some(("snapshot".into(), pause));
                    });
                }
            }
            _ => {}
        }
    }

    fn reload_bots(&mut self) {
        self.bots = self.engine.db.bots().unwrap_or_default();
        if self.selected.as_ref().is_none_or(|id| !self.bots.iter().any(|b| &b.id == id)) {
            self.selected = self.bots.first().map(|b| b.id.clone());
        }
        self.refresh_roster();
    }

    /// The preview under each name: what was last said, and when.
    fn refresh_roster(&mut self) {
        self.last_roster = Instant::now();
        self.roster = self
            .bots
            .iter()
            .map(|b| {
                let last = self.engine.db.tasks_for(&b.id, 1).ok().and_then(|mut v| v.pop());
                let meta = match last {
                    Some(t) => {
                        let said = t.summary.as_deref().filter(|s| !s.trim().is_empty() && t.status == "done");
                        let preview = match said {
                            Some(s) => one_line(s),
                            None => format!("You: {}", one_line(&t.instruction)),
                        };
                        RosterMeta { preview, when: when_label(t.completed_at.as_deref().unwrap_or(&t.created_at)) }
                    }
                    None => RosterMeta { preview: format!("{} · {}", mybot_core::providers::family_label(&b.provider), b.model), when: String::new() },
                };
                (b.id.clone(), meta)
            })
            .collect();
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
        if self.last_roster.elapsed() > Duration::from_secs(2) {
            self.refresh_roster();
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
                theme::app_tile(ui, 76.0);
                ui.add_space(12.0);
                ui.label(RichText::new("MyBot").font(semibold(28.0)).color(TEXT));
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

    /// The roster. As in the reference, the colour in the window lives here:
    /// each teammate's tile, a role chip, the time, and what was last said.
    fn rail(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("rail")
            .exact_size(276.0)
            .resizable(false)
            .frame(egui::Frame::NONE.fill(RAIL).inner_margin(Margin { left: 10, right: 10, top: 14, bottom: 10 }).stroke(Stroke::new(1.0, LINE_SOFT)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    theme::mark(ui, 24.0);
                    ui.add_space(2.0);
                    ui.label(RichText::new("MyBot").font(semibold(16.5)).color(TEXT));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(2.0);
                        if round_icon_button(ui, Glyph::Plus).on_hover_text("New teammate").clicked() {
                            self.chat.open_new_bot(&self.engine);
                        }
                    });
                });
                ui.add_space(10.0);

                let foot_h = 4.0 * 34.0 + 54.0;
                egui::ScrollArea::vertical().max_height((ui.available_height() - foot_h).max(80.0)).auto_shrink([false, true]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    if self.bots.is_empty() {
                        ui.add_space(8.0);
                        ui.label(muted("No teammates yet."));
                        if ui.add(egui::Button::new(RichText::new("Create one").color(ACCENT)).frame(false)).clicked() {
                            self.chat.open_new_bot(&self.engine);
                        }
                    }
                    let mut pick = None;
                    for b in &self.bots {
                        let (working, waiting) = self.threads.with(&b.id, |t| (t.working, t.waiting.is_some()));
                        let running = self.engine.is_running(&b.id);
                        let meta = self.roster.get(&b.id);
                        let (preview, tone) = if waiting {
                            ("Needs you".to_string(), Tone::Attention)
                        } else if running || working {
                            ("Working…".to_string(), Tone::Live)
                        } else {
                            (meta.map(|m| m.preview.clone()).unwrap_or_default(), Tone::Quiet)
                        };
                        let row = RosterRow {
                            name: &b.name,
                            role: mybot_core::providers::family_label(&b.provider),
                            when: meta.map(|m| m.when.as_str()).unwrap_or(""),
                            preview: &preview,
                            tone,
                            selected: self.view == View::Chat && self.selected.as_deref() == Some(b.id.as_str()),
                        };
                        if roster_row(ui, &row).clicked() {
                            pick = Some(b.id.clone());
                        }
                    }
                    if let Some(id) = pick {
                        self.open_chat(&id);
                    }
                });

                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let unlocked = self.engine.is_unlocked();
                    let mut toggle = false;
                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        let (dot, text) = if unlocked { (GOOD, "Keys unlocked") } else { (FAINT, "Keys locked") };
                        ui.label(RichText::new("●").size(9.0).color(dot));
                        ui.label(small(text));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.add_space(6.0);
                            if ui.add(link_button(if unlocked { "Lock" } else { "Unlock" })).clicked() {
                                toggle = true;
                            }
                        });
                    });
                    if toggle {
                        if unlocked {
                            self.engine.lock();
                        }
                        self.unlock = Some(Unlock { pass: String::new(), confirm: String::new(), remember: false, error: None, job: None });
                    }
                    ui.add_space(4.0);
                    let r = ui.available_rect_before_wrap();
                    ui.painter().hline(r.x_range(), r.bottom() - 1.0, Stroke::new(1.0, LINE_SOFT));
                    ui.add_space(8.0);
                    let nav = [(View::Settings, "⚙", "Settings"), (View::Logins, "🔑", "Logins"), (View::Routines, "⏰", "Routines"), (View::Skills, "✨", "Skills")];
                    for (v, icon, label) in nav {
                        let badge = match v {
                            View::Logins if !self.login_requests.is_empty() => Some(self.login_requests.len()),
                            _ => None,
                        };
                        if nav_row(ui, icon, label, self.view == v, badge).clicked() {
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

#[derive(Clone, Copy, PartialEq)]
enum Tone {
    Quiet,
    Live,
    Attention,
}

struct RosterRow<'a> {
    name: &'a str,
    role: &'a str,
    when: &'a str,
    preview: &'a str,
    tone: Tone,
    selected: bool,
}

struct RosterMeta {
    preview: String,
    when: String,
}

/// One teammate in the roster: tile · name · chip · time, then a preview line.
fn roster_row(ui: &mut egui::Ui, row: &RosterRow) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 60.0), Sense::click());
    let p = ui.painter().clone();
    let bg = if row.selected { RAISED } else if resp.hovered() { Color32::from_rgb(22, 21, 19) } else { RAIL };
    if bg != RAIL {
        p.rect_filled(rect, 12, bg);
    }
    let dot = match row.tone {
        Tone::Attention => Some(ACCENT),
        Tone::Live => Some(GOOD),
        Tone::Quiet => None,
    };
    let tile = egui::Rect::from_min_size(rect.left_top() + Vec2::new(10.0, 11.0), Vec2::splat(38.0));
    theme::paint_avatar(&p, tile, row.name, dot, bg);

    let x0 = tile.right() + 12.0;
    let right = rect.right() - 10.0;
    let top = rect.top() + 11.0;

    // Time, right-aligned on the first line.
    let when = p.layout_no_wrap(row.when.to_string(), egui::FontId::proportional(11.5), FAINT);
    let when_x = right - when.size().x;
    p.galley(Pos2::new(when_x, top + 1.0), when, FAINT);

    // Name, then the role chip, both clipped before the time.
    let name_max = (when_x - x0 - 8.0 - 56.0).max(40.0);
    let name = one_row(&p, row.name, theme::semibold(14.0), TEXT, name_max);
    let name_w = name.size().x;
    p.galley(Pos2::new(x0, top - 1.0), name, TEXT);
    if !row.role.is_empty() {
        let chip = p.layout_no_wrap(row.role.to_string(), theme::semibold(10.0), MUTED);
        let cr = egui::Rect::from_min_size(Pos2::new(x0 + name_w + 7.0, top + 1.0), chip.size() + Vec2::new(10.0, 3.0));
        if cr.right() < when_x - 6.0 {
            p.rect_filled(cr, 5, if row.selected { RAISED2 } else { RAISED });
            p.galley(cr.min + Vec2::new(5.0, 1.5), chip, MUTED);
        }
    }

    // What was last said.
    let color = match row.tone {
        Tone::Attention => ACCENT,
        Tone::Live => GOOD,
        Tone::Quiet => MUTED,
    };
    let preview = one_row(&p, row.preview, egui::FontId::proportional(12.5), color, right - x0);
    p.galley(Pos2::new(x0, top + 20.0), preview, color);

    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Lay a string out on one line, ending in "…" if it doesn't fit.
fn one_row(p: &egui::Painter, text: &str, font: egui::FontId, color: Color32, max_width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(text.to_string(), egui::TextFormat::simple(font, color));
    job.wrap = egui::text::TextWrapping { max_width, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    p.layout_job(job)
}

fn nav_row(ui: &mut egui::Ui, icon: &str, label: &str, selected: bool, badge: Option<usize>) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 10, RAISED);
    } else if resp.hovered() {
        p.rect_filled(rect, 10, Color32::from_rgb(22, 21, 19));
    }
    let ink = if selected { TEXT } else { TEXT2 };
    p.text(rect.left_center() + Vec2::new(14.0, 0.0), Align2::LEFT_CENTER, icon, egui::FontId::proportional(13.0), if selected { ACCENT } else { MUTED });
    p.text(rect.left_center() + Vec2::new(38.0, 0.0), Align2::LEFT_CENTER, label, egui::FontId::proportional(13.5), ink);
    if let Some(n) = badge {
        let c = rect.right_center() - Vec2::new(16.0, 0.0);
        p.circle_filled(c, 9.0, ACCENT);
        p.text(c, Align2::CENTER_CENTER, n.to_string(), theme::semibold(11.0), ACCENT_INK);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[derive(Clone, Copy)]
pub enum Glyph {
    Plus,
    Up,
    Stop,
}

/// A small round icon button, the glyph drawn rather than typed (font
/// coverage for arrows varies by platform).
pub fn round_icon_button(ui: &mut egui::Ui, glyph: Glyph) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
    let p = ui.painter();
    let fill = if resp.hovered() { RAISED2 } else { RAISED };
    p.circle_filled(rect.center(), 14.0, fill);
    paint_glyph(p, rect, glyph, TEXT2);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn paint_glyph(p: &egui::Painter, rect: egui::Rect, glyph: Glyph, color: Color32) {
    let c = rect.center();
    let k = rect.width() * 0.22;
    let st = Stroke::new(1.8, color);
    match glyph {
        Glyph::Plus => {
            p.line_segment([c - Vec2::new(k, 0.0), c + Vec2::new(k, 0.0)], st);
            p.line_segment([c - Vec2::new(0.0, k), c + Vec2::new(0.0, k)], st);
        }
        Glyph::Up => {
            let st = Stroke::new(2.0, color);
            p.line_segment([c + Vec2::new(0.0, k * 1.1), c - Vec2::new(0.0, k * 1.1)], st);
            p.line_segment([c - Vec2::new(0.0, k * 1.1), c + Vec2::new(-k, -k * 0.1)], st);
            p.line_segment([c - Vec2::new(0.0, k * 1.1), c + Vec2::new(k, -k * 0.1)], st);
        }
        Glyph::Stop => {
            p.rect_filled(egui::Rect::from_center_size(c, Vec2::splat(k * 1.5)), 2, color);
        }
    }
}

/// First line of a message, whitespace collapsed.
fn one_line(s: &str) -> String {
    let first = s.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let first = first.trim_start_matches(['#', '*', '-', ' ']);
    first.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// "14:05" today, "Yesterday", a weekday this week, else "3 Oct".
fn when_label(stored: &str) -> String {
    use chrono::{Datelike, Local, NaiveDateTime, TimeZone, Utc};
    let Ok(naive) = NaiveDateTime::parse_from_str(stored.get(..19).unwrap_or(stored), "%Y-%m-%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(stored.get(..19).unwrap_or(stored), "%Y-%m-%dT%H:%M:%S"))
    else {
        return String::new();
    };
    let at = Utc.from_utc_datetime(&naive).with_timezone(&Local);
    let now = Local::now();
    let days = (now.date_naive() - at.date_naive()).num_days();
    match days {
        0 => at.format("%H:%M").to_string(),
        1 => "Yesterday".into(),
        2..=6 => at.format("%a").to_string(),
        _ if at.year() == now.year() => at.format("%-d %b").to_string(),
        _ => at.format("%-d %b %Y").to_string(),
    }
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
        if let Some(s) = &mut self.snapshot {
            s.tick(&ctx);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if self.snapshot.is_some() {
            return;
        }
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

/// The window and Dock icon: the MyBot app icon, rasterised at startup.
fn icon() -> egui::IconData {
    let n = 256;
    // macOS shows this in the Dock when running unbundled; give it the same
    // margin a bundled icon has, so it isn't oversized next to its neighbours.
    let margin = if cfg!(target_os = "macos") { 0.1 } else { 0.0 };
    egui::IconData { rgba: theme::icon_rgba(n, margin), width: n as u32, height: n as u32 }
}

pub fn run(engine: Arc<Engine>, rt: tokio::runtime::Handle) -> eframe::Result {
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("MyBot")
            .with_app_id("mybot")
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([900.0, 560.0])
            .with_icon(icon()),
        ..Default::default()
    };
    // A snapshot must neither read nor overwrite the real window's saved
    // size, scroll positions and selection.
    if std::env::var_os("MYBOT_SNAPSHOT").is_some_and(|v| !v.is_empty()) {
        options.persistence_path = Some(mybot_vault::home().join("snapshot-ui.ron"));
        options.persist_window = false;
    }
    eframe::run_native("MyBot", options, Box::new(move |cc| Ok(Box::new(App::new(cc, engine, rt)))))
}
