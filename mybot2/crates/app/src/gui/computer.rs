//! The agent computer: the bot's desktop drawn natively from VNC, a takeover
//! switch that hands the human the mouse and keyboard, and "teach a task".

use std::time::{Duration, Instant};

use egui::{Align, Color32, Event, Id, Layout, Margin, RichText, Sense, Stroke, Vec2};
use mybot_actions::teach::{self, Recorder, SkillDraft};
use mybot_computer::container::Desktop;
use mybot_computer::rfb::{Input, LiveView, char_keysym, named_keysym};
use mybot_core::db::{Bot, Recording};

use super::theme::*;
use super::{App, Job, poll};

type Connected = Result<Option<(Desktop, LiveView)>, String>;

#[derive(Default)]
pub struct ComputerPanel {
    /// The bot (by name) the panel is showing.
    pub bot: Option<String>,
    pub view: Option<LiveView>,
    pub desktop: Option<Desktop>,
    /// `Ok(None)`: the computer isn't running yet.
    connecting: Option<Job<Connected>>,
    starting: bool,
    pub error: Option<String>,
    texture: Option<egui::TextureHandle>,
    generation: u64,
    pub takeover: bool,
    buttons: u8,
    last_pos: Option<(u16, u16)>,
    last_check: Option<Instant>,
    // teach a task
    recorder: Option<Recorder>,
    teach_label: String,
    teach_start: Option<Job<Result<Recorder, String>>>,
    teach_stop: Option<Job<Result<Recording, String>>>,
    compile: Option<Job<Result<SkillDraft, String>>>,
    last_recording: Option<Recording>,
    draft: Option<DraftForm>,
}

struct DraftForm {
    recording_id: String,
    name: String,
    trigger: String,
    steps: String,
    rules: String,
    notes: String,
}

impl DraftForm {
    fn new(recording_id: String, d: SkillDraft) -> Self {
        Self { recording_id, name: d.name, trigger: d.trigger, steps: d.steps.join("\n"), rules: d.safety_rules.join("\n"), notes: d.notes.unwrap_or_default() }
    }

    fn draft(&self) -> SkillDraft {
        let lines = |s: &str| s.lines().map(|l| l.trim().trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ')').trim().to_string()).filter(|l| !l.is_empty()).collect::<Vec<_>>();
        SkillDraft {
            name: mybot_catalog::agent_skill::slug(&self.name),
            trigger: self.trigger.trim().to_string(),
            steps: lines(&self.steps),
            safety_rules: self.rules.lines().map(|l| l.trim().trim_start_matches("- ").to_string()).filter(|l| !l.is_empty()).collect(),
            notes: Some(self.notes.trim().to_string()).filter(|n| !n.is_empty()),
        }
    }
}

impl App {
    pub(super) fn computer_panel(&mut self, ui: &mut egui::Ui) {
        let Some(bot) = self.bot().cloned() else { return };
        let ctx = ui.ctx().clone();
        self.computer_sync(&ctx, &bot);

        // Keep at least ~360 px for the conversation beside the 256 px roster.
        let room = (ctx.content_rect().width() - 256.0 - 360.0).max(320.0);
        egui::Panel::right("computer")
            .default_size(560.0_f32.min(room))
            .size_range(320.0_f32.min(room)..=room.min(1100.0))
            .resizable(true)
            .frame(egui::Frame::NONE.fill(RAIL).inner_margin(Margin::same(14)).stroke(Stroke::new(1.0, LINE_SOFT)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Agent computer").font(semibold(15.0)).color(TEXT));
                    let (dot, text) = match (&self.computer.view, self.computer.connecting.is_some() || self.computer.starting) {
                        (Some(v), _) if v.is_connected() => (GOOD, "Live"),
                        (_, true) => (ACCENT, "Connecting…"),
                        _ => (FAINT, "Off"),
                    };
                    ui.label(RichText::new("●").size(9.0).color(dot));
                    ui.label(small(text));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if let Some(d) = &self.computer.desktop
                            && ui.add(link_button("↗")).on_hover_text("Open the same desktop in a browser tab").clicked()
                        {
                            ctx.open_url(egui::OpenUrl::new_tab(d.web_url()));
                        }
                        let live = self.computer.view.as_ref().is_some_and(|v| v.is_connected());
                        if live {
                            let t = if self.computer.takeover { accent_button("Hand back") } else { ghost_button("Take over") };
                            if ui.add(t).on_hover_text("Use the teammate's mouse and keyboard yourself").clicked() {
                                self.computer.takeover = !self.computer.takeover;
                            }
                        }
                    });
                });
                ui.add_space(8.0);
                self.handoff_bar(ui, &bot);
                self.recording_bar(ui, &bot);
                self.screen(ui, &bot);
                ui.add_space(10.0);
                self.teach_section(ui, &bot);
                ui.add_space(6.0);
                egui::CollapsingHeader::new(small("Activity")).id_salt("computer_log").default_open(false).show(ui, |ui| {
                    let lines = self.engine.computer_log.lock().unwrap().clone();
                    egui::ScrollArea::vertical().max_height(140.0).stick_to_bottom(true).show(ui, |ui| {
                        if lines.is_empty() {
                            ui.label(small("Nothing yet."));
                        }
                        for l in lines.iter().rev().take(80).rev() {
                            ui.label(RichText::new(l).monospace().size(11.5).color(MUTED));
                        }
                    });
                });
            });
    }

    /// The step the teammate is waiting on, inside the computer — the reference's
    /// pattern: the instruction stays on screen while you do it, and handing
    /// back is one click from the thing you just finished.
    fn handoff_bar(&mut self, ui: &mut egui::Ui, bot: &Bot) {
        let Some((thread_task, pause)) = self.threads.with(&bot.id, |t| t.waiting.clone()) else { return };
        if pause.confirm {
            return; // an approval, answered in the thread
        }
        let task = self.engine.runs.lock().unwrap().get(&bot.id).map(|r| r.task_id.clone()).unwrap_or(thread_task);
        egui::Frame::NONE.fill(ACCENT_WASH).stroke(Stroke::new(1.0, ACCENT_EDGE)).corner_radius(12).inner_margin(Margin::symmetric(12, 9)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(&pause.reason).color(TEXT).size(13.0)).wrap());
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(light_button("I'm done, continue")).clicked() {
                        self.computer.takeover = false;
                        self.engine.handoffs.resume(&task, false);
                    }
                    if ui.add(link_button("Skip this step")).clicked() {
                        self.engine.handoffs.resume(&task, true);
                    }
                });
            });
        });
        ui.add_space(8.0);
    }

    /// While teaching, say so above the screen (the screen also gets a red frame).
    fn recording_bar(&mut self, ui: &mut egui::Ui, bot: &Bot) {
        let Some(rec) = &self.computer.recorder else { return };
        let el = rec.elapsed().as_secs();
        let blink = (ui.input(|i| i.time) * 2.0) as i64 % 2 == 0;
        egui::Frame::NONE.fill(Color32::from_rgb(38, 16, 18)).stroke(Stroke::new(1.0, Color32::from_rgb(110, 36, 42))).corner_radius(12).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("●").size(11.0).color(if blink { BAD } else { Color32::from_rgb(120, 40, 46) }));
                ui.label(RichText::new(format!("{} is watching and learning", bot.name)).color(TEXT).size(13.0));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(small(format!("{:02}:{:02} · {} actions", el / 60, el % 60, rec.count())));
                });
            });
        });
        ui.add_space(8.0);
    }

    /// Follow the selected bot; connect when its computer is up.
    fn computer_sync(&mut self, ctx: &egui::Context, bot: &Bot) {
        let p = &mut self.computer;
        if p.bot.as_deref() != Some(bot.name.as_str()) {
            p.bot = Some(bot.name.clone());
            p.view = None;
            p.desktop = None;
            p.texture = None;
            p.generation = 0;
            p.takeover = false;
            p.error = None;
            p.connecting = None;
            p.last_check = None;
        }
        if let Some(r) = poll(&mut p.connecting) {
            p.starting = false;
            match r {
                Ok(Some((d, v))) => {
                    p.desktop = Some(d);
                    p.view = Some(v);
                    p.error = None;
                }
                Ok(None) => {}
                Err(e) => p.error = Some(e),
            }
        }
        let dropped = p.view.as_ref().is_some_and(|v| !v.is_connected());
        if dropped {
            p.error = p.view.as_ref().and_then(|v| v.error.try_lock().ok().and_then(|e| e.clone())).or(Some("The live view disconnected.".into()));
            p.view = None;
            p.takeover = false;
        }
        // Look again every few seconds: a run may have started the computer.
        let due = p.last_check.is_none_or(|t| t.elapsed() > Duration::from_secs(4));
        if p.view.is_none() && p.connecting.is_none() && due && p.error.is_none() {
            p.last_check = Some(Instant::now());
            let session = self.engine.session(&bot.name);
            p.connecting = Some(Job::spawn(&self.rt, ctx, async move {
                let Some((_, desktop)) = session.current().await else { return Ok(None) };
                let view = LiveView::connect_ws(&desktop.vnc_ws_url(), None).await?;
                Ok(Some((desktop, view)))
            }));
        }
        if p.view.is_some() {
            ctx.request_repaint_after(Duration::from_millis(40));
        }
    }

    fn start_computer(&mut self, ctx: &egui::Context, bot: &Bot) {
        let session = self.engine.session(&bot.name);
        self.computer.error = None;
        self.computer.starting = true;
        self.computer.connecting = Some(Job::spawn(&self.rt, ctx, async move {
            let (_, desktop, _) = session.ensure().await?;
            let view = LiveView::connect_ws(&desktop.vnc_ws_url(), None).await?;
            Ok(Some((desktop, view)))
        }));
    }

    fn screen(&mut self, ui: &mut egui::Ui, bot: &Bot) {
        let ctx = ui.ctx().clone();
        let Some(view) = self.computer.view.clone() else {
            // The frame's 1 px stroke sits outside its content: leave room for it,
            // or a resizable panel grows by 2 px every frame.
            let w = ui.available_width() - 2.0;
            egui::Frame::NONE.fill(Color32::from_rgb(9, 8, 7)).corner_radius(14).stroke(Stroke::new(1.0, LINE_SOFT)).show(ui, |ui| {
                ui.set_min_size(Vec2::new(w, w * 0.6));
                ui.vertical_centered(|ui| {
                    ui.add_space(w * 0.16);
                    if self.computer.connecting.is_some() || self.computer.starting {
                        ui.add(egui::Spinner::new().size(22.0).color(ACCENT));
                        ui.label(muted(if self.computer.starting { "Starting the computer… the first start builds its image and can take a few minutes." } else { "Looking for the computer…" }));
                    } else {
                        avatar(ui, &bot.name, 40.0, None);
                        ui.add_space(4.0);
                        ui.label(RichText::new(format!("{}'s computer is off", bot.name)).font(semibold(14.5)).color(TEXT2));
                        ui.label(small("It starts by itself when a task needs it."));
                        if let Some(e) = &self.computer.error {
                            ui.add_space(4.0);
                            ui.label(RichText::new(e).color(BAD).size(12.5));
                        }
                        ui.add_space(6.0);
                        if ui.add(accent_button("Start computer")).clicked() {
                            self.start_computer(&ctx, bot);
                        }
                    }
                });
            });
            return;
        };

        // Upload the framebuffer when it changed.
        if let Ok(fb) = view.fb.try_lock()
            && fb.width > 0 && fb.generation != self.computer.generation {
                let img = egui::ColorImage::from_rgba_unmultiplied([fb.width as usize, fb.height as usize], &fb.rgba);
                match &mut self.computer.texture {
                    Some(t) if t.size() == [fb.width as usize, fb.height as usize] => t.set(img, egui::TextureOptions::LINEAR),
                    _ => self.computer.texture = Some(ctx.load_texture("desktop", img, egui::TextureOptions::LINEAR)),
                }
                self.computer.generation = fb.generation;
            }
        let Some(tex) = self.computer.texture.clone() else {
            ui.add(egui::Spinner::new());
            return;
        };
        let [tw, th] = tex.size();
        let w = ui.available_width();
        let h = (w * th as f32 / tw as f32).min(ui.available_height() - 180.0).max(120.0);
        let size = Vec2::new(h * tw as f32 / th as f32, h);
        let takeover = self.computer.takeover;
        let resp = ui.add(egui::Image::from_texture(egui::load::SizedTexture::new(tex.id(), size)).corner_radius(10).sense(if takeover { Sense::click_and_drag() } else { Sense::hover() }));
        let recording = self.computer.recorder.is_some();
        let (frame_w, frame_color) = if recording { (2.5, BAD) } else if takeover { (2.0, ACCENT) } else { (1.0, LINE) };
        ui.painter().rect_stroke(resp.rect, 10, Stroke::new(frame_w, frame_color), egui::StrokeKind::Outside);

        if !takeover {
            if resp.hovered() {
                let r = egui::Rect::from_min_size(resp.rect.left_bottom() - Vec2::new(-8.0, 30.0), Vec2::new(250.0, 22.0));
                ui.painter().rect_filled(r, 6, Color32::from_black_alpha(170));
                ui.painter().text(r.left_center() + Vec2::new(8.0, 0.0), egui::Align2::LEFT_CENTER, "View only — Take over to use it", egui::FontId::proportional(12.0), TEXT2);
            }
            return;
        }

        // --- takeover: pointer, wheel and keys go to the bot's desktop ---
        if resp.clicked() || resp.drag_started() {
            resp.request_focus();
        }
        if resp.has_focus() {
            ui.memory_mut(|m| m.set_focus_lock_filter(resp.id, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true }));
        }
        let to_fb = |p: egui::Pos2| -> (u16, u16) {
            let x = ((p.x - resp.rect.min.x) / resp.rect.width() * tw as f32).clamp(0.0, tw as f32 - 1.0);
            let y = ((p.y - resp.rect.min.y) / resp.rect.height() * th as f32).clamp(0.0, th as f32 - 1.0);
            (x as u16, y as u16)
        };
        if let Some(pos) = resp.hover_pos().or(if resp.dragged() { resp.interact_pointer_pos() } else { None }) {
            let buttons = ui.input(|i| (i.pointer.primary_down() as u8) | ((i.pointer.middle_down() as u8) << 1) | ((i.pointer.secondary_down() as u8) << 2));
            let (x, y) = to_fb(pos);
            if self.computer.last_pos != Some((x, y)) || self.computer.buttons != buttons {
                view.send(Input::Pointer { x, y, buttons });
                self.computer.last_pos = Some((x, y));
                self.computer.buttons = buttons;
            }
            let wheel: f32 = ui.input(|i| i.events.iter().filter_map(|e| if let Event::MouseWheel { delta, .. } = e { Some(delta.y) } else { None }).sum());
            if wheel.abs() > 0.0 {
                let bit = if wheel > 0.0 { 1 << 3 } else { 1 << 4 };
                for _ in 0..(wheel.abs().ceil() as usize).clamp(1, 5) {
                    view.send(Input::Pointer { x, y, buttons: buttons | bit });
                    view.send(Input::Pointer { x, y, buttons });
                }
            }
        }
        if resp.has_focus() {
            let events = ui.input(|i| i.events.clone());
            for e in events {
                match e {
                    Event::Text(t) => {
                        for c in t.chars() {
                            let k = char_keysym(c);
                            view.send(Input::Key { keysym: k, down: true });
                            view.send(Input::Key { keysym: k, down: false });
                        }
                    }
                    Event::Paste(t) => view.type_text(&t),
                    Event::Copy => combo(&view, 'c'),
                    Event::Cut => combo(&view, 'x'),
                    Event::Key { key, pressed, modifiers, .. } => {
                        let name = key.name();
                        if let Some(k) = named_keysym(name) {
                            if name != "Space" {
                                view.send(Input::Key { keysym: k, down: pressed });
                            }
                        } else if pressed && (modifiers.ctrl || modifiers.command) && !matches!(key, egui::Key::C | egui::Key::X | egui::Key::V)
                            && let Some(c) = name.chars().next().filter(|_| name.chars().count() == 1) {
                                combo(&view, c.to_ascii_lowercase());
                            }
                    }
                    _ => {}
                }
            }
        }
    }

    fn teach_section(&mut self, ui: &mut egui::Ui, bot: &Bot) {
        let ctx = ui.ctx().clone();
        let p = &mut self.computer;
        if let Some(r) = poll(&mut p.teach_start) {
            match r {
                Ok(rec) => {
                    p.recorder = Some(rec);
                    p.takeover = true;
                }
                Err(e) => self.toasts.push(super::Toast { text: e, bad: true, at: Instant::now() }),
            }
        }
        if let Some(r) = poll(&mut p.teach_stop) {
            match r {
                Ok(rec) => {
                    let n = teach::actions_of(&rec).len();
                    p.last_recording = Some(rec);
                    if n == 0 {
                        self.toasts.push(super::Toast { text: "Nothing was recorded. Do the task in the bot's browser while recording.".into(), bad: true, at: Instant::now() });
                    } else {
                        start_compile(&self.engine, &self.rt, &ctx, p, bot);
                    }
                }
                Err(e) => self.toasts.push(super::Toast { text: e, bad: true, at: Instant::now() }),
            }
        }
        if let Some(r) = poll(&mut p.compile) {
            match r {
                Ok(d) => p.draft = Some(DraftForm::new(p.last_recording.as_ref().map(|r| r.id.clone()).unwrap_or_default(), d)),
                Err(e) => self.toasts.push(super::Toast { text: format!("Couldn't draft the skill: {e}"), bad: true, at: Instant::now() }),
            }
        }
        if p.recorder.as_ref().is_some_and(|r| r.expired()) {
            let rec = p.recorder.take().unwrap();
            let db = self.engine.db.clone();
            p.teach_stop = Some(Job::spawn(&self.rt, &ctx, async move { rec.stop(&db).await }));
        }

        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            let p = &mut self.computer;
            if let Some(rec) = &p.recorder {
                let el = rec.elapsed().as_secs();
                ui.horizontal(|ui| {
                    let blink = (ctx.input(|i| i.time) * 2.0) as i64 % 2 == 0;
                    ui.label(RichText::new("•").size(22.0).color(if blink { BAD } else { Color32::from_rgb(120, 40, 40) }));
                    ui.label(RichText::new(format!("Recording “{}”", rec.label)).strong());
                    ui.label(small(format!("{:02}:{:02} / 10:00 · {} actions", el / 60, el % 60, rec.count())));
                });
                for a in rec.actions().iter().rev().take(5).rev() {
                    ui.label(RichText::new(a.line()).monospace().size(11.5).color(MUTED));
                }
                ui.label(small("Passwords, codes and card numbers are blanked inside the page — they're never recorded."));
                ui.horizontal(|ui| {
                    if ui.add(accent_button("Stop and draft skill")).clicked() {
                        let rec = p.recorder.take().unwrap();
                        let db = self.engine.db.clone();
                        p.teach_stop = Some(Job::spawn(&self.rt, &ctx, async move { rec.stop(&db).await }));
                        p.takeover = false;
                    }
                });
                ctx.request_repaint_after(Duration::from_millis(500));
            } else if p.teach_stop.is_some() || p.compile.is_some() {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(14.0).color(ACCENT));
                    ui.label(muted(if p.compile.is_some() { format!("{} is writing the skill from your demonstration…", bot.name) } else { "Saving the recording…".into() }));
                });
            } else {
                ui.label(RichText::new("Teach a task").strong());
                ui.label(small(format!("Do it once yourself in {}'s browser; it becomes a skill the bot can repeat.", bot.name)));
                let live = p.view.as_ref().is_some_and(|v| v.is_connected());
                ui.horizontal(|ui| {
                    ui.add(input(&mut p.teach_label).hint_text("What are you showing? e.g. download my invoice").desired_width((ui.available_width() - 130.0).max(120.0)));
                    let ok = live && !p.teach_label.trim().is_empty() && p.teach_start.is_none();
                    if ui.add_enabled(ok, accent_button("Record")).on_disabled_hover_text(if live { "Name the task first" } else { "Start the computer first" }).clicked() {
                        let session = self.engine.session(&bot.name);
                        let label = p.teach_label.trim().to_string();
                        p.teach_start = Some(Job::spawn(&self.rt, &ctx, async move {
                            let browser = session.browser().await?;
                            Recorder::start(browser, &label).await
                        }));
                    }
                });
                if let Some(rec) = p.last_recording.clone()
                    && p.draft.is_none() && rec.skill_id.is_none() && ui.add(egui::Button::new(small(format!("Draft “{}” again", rec.name))).frame(false)).clicked() {
                        start_compile(&self.engine, &self.rt, &ctx, p, bot);
                    }
            }
        });
    }

    /// Review the drafted skill before it's saved.
    pub(super) fn teach_modal(&mut self, ctx: &egui::Context) {
        let Some(f) = self.computer.draft.as_mut() else { return };
        let mut save = false;
        let mut close = false;
        let r = egui::Modal::new(Id::new("teach_draft")).show(ctx, |ui| {
            ui.set_width(600.0);
            ui.label(heading("Review the new skill"));
            ui.label(muted("Drafted from your demonstration. Edit anything, then save."));
            ui.add_space(6.0);
            egui::Grid::new("draft_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label(muted("Name"));
                ui.add(input(&mut f.name).desired_width(460.0));
                ui.end_row();
                ui.label(muted("Use when"));
                ui.add(input(&mut f.trigger).desired_width(460.0));
                ui.end_row();
                ui.label(muted("Steps"));
                ui.add(egui::TextEdit::multiline(&mut f.steps).desired_rows(8).desired_width(460.0).hint_text("One step per line"));
                ui.end_row();
                ui.label(muted("Safety rules"));
                ui.add(egui::TextEdit::multiline(&mut f.rules).desired_rows(3).desired_width(460.0).hint_text("always-confirm: sending the email"));
                ui.end_row();
                ui.label(muted("Notes"));
                ui.add(egui::TextEdit::multiline(&mut f.notes).desired_rows(2).desired_width(460.0));
                ui.end_row();
            });
            ui.label(small("“always-confirm: …” rules make the bot stop and ask you before that action, every time."));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.add(accent_button("Save skill")).clicked() {
                    save = true;
                }
                if ui.add(ghost_button("Discard")).clicked() {
                    close = true;
                }
            });
        });
        if save {
            let d = f.draft();
            let rec = Some(f.recording_id.clone()).filter(|s| !s.is_empty());
            if d.steps.is_empty() {
                self.toast_err("A skill needs at least one step.");
                return;
            }
            match teach::save(&self.engine.db, rec.as_deref(), &d) {
                Ok(row) => {
                    self.computer.draft = None;
                    if let Some(r) = &mut self.computer.last_recording {
                        r.skill_id = Some(row.id.clone());
                    }
                    self.computer.teach_label.clear();
                    self.toast(format!("Saved the skill “{}”. Find it under Skills → Yours.", row.name));
                }
                Err(e) => self.toast_err(e),
            }
        } else if close || r.should_close() {
            self.computer.draft = None;
        }
    }
}

fn start_compile(engine: &std::sync::Arc<crate::engine::Engine>, rt: &tokio::runtime::Handle, ctx: &egui::Context, p: &mut ComputerPanel, bot: &Bot) {
    let Some(rec) = p.last_recording.clone() else { return };
    let engine = engine.clone();
    let (provider, model) = (bot.provider.clone(), bot.model.clone());
    p.compile = Some(Job::spawn(rt, ctx, async move {
        let prov = engine.provider(&provider).map_err(|e| e.to_string())?;
        teach::compile(prov.as_ref(), Some(model), &rec.name, &teach::actions_of(&rec)).await
    }));
}

/// Ctrl+<letter> on the remote desktop.
fn combo(view: &LiveView, c: char) {
    let ctrl = named_keysym("Control").unwrap();
    view.send(Input::Key { keysym: ctrl, down: true });
    view.send(Input::Key { keysym: char_keysym(c), down: true });
    view.send(Input::Key { keysym: char_keysym(c), down: false });
    view.send(Input::Key { keysym: ctrl, down: false });
}
