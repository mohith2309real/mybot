//! The conversation: header, thread, the "needs you" banner, the composer,
//! and the new-teammate / edit-teammate dialogs.

use std::collections::BTreeMap;

use egui::{Align, Id, Key, Layout, Margin, Modifiers, RichText, Stroke, Vec2};
use mybot_core::db::Bot;
use mybot_core::providers::PROVIDERS;

use super::state::{Item, verb};
use super::theme::*;
use super::{App, Job, poll};
use crate::engine::{Engine, SkillRun, fallback_models};

#[derive(Default)]
pub struct ChatState {
    pub bot_form: Option<BotForm>,
    pub confirm_delete: Option<String>,
    /// A skill picked in the composer to run with the next message.
    pub skill: Option<String>,
    pub skill_query: String,
    pub skill_picker: bool,
}

pub struct BotForm {
    /// None: a new teammate.
    pub id: Option<String>,
    pub name: String,
    pub provider: String,
    pub model: String,
    pub system: String,
    pub mode: String,
    pub effort: String,
    pub models: Vec<String>,
    pub models_job: Option<Job<Result<Vec<String>, String>>>,
    pub models_for: String,
    pub error: Option<String>,
}

impl BotForm {
    fn from_bot(b: &Bot) -> Self {
        Self {
            id: Some(b.id.clone()),
            name: b.name.clone(),
            provider: b.provider.clone(),
            model: b.model.clone(),
            system: b.system.clone().unwrap_or_default(),
            mode: b.mode.clone(),
            effort: b.effort.clone(),
            models: vec![],
            models_job: None,
            models_for: String::new(),
            error: None,
        }
    }
}

impl ChatState {
    pub fn open_new_bot(&mut self, engine: &Engine) {
        // Default to a provider that can actually run.
        let provider = PROVIDERS.iter().find(|p| engine.key_source(p).is_some() || engine.base_url(p).is_some()).copied().unwrap_or("anthropic");
        self.bot_form = Some(BotForm {
            id: None,
            name: String::new(),
            provider: provider.into(),
            model: mybot_core::providers::default_model(provider).into(),
            system: String::new(),
            mode: "ask".into(),
            effort: "high".into(),
            models: vec![],
            models_job: None,
            models_for: String::new(),
            error: None,
        });
    }
}

const MODES: [(&str, &str, &str); 3] = [
    ("ask", "Ask first", "Stops before anything consequential your request didn't ask for (sending, posting, buying, paying, deleting, changing access, accepting terms), and checks in when unsure."),
    ("auto", "Auto", "Gets on with routine steps without checking in. Consequential steps your request didn't ask for still wait for you."),
    ("bypass", "Bypass", "Doesn't stop for consequential steps either. CAPTCHAs, 2-step codes, card fields, checkout and destructive commands still stop for you."),
];

const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

impl App {
    pub(super) fn chat_view(&mut self, ui: &mut egui::Ui) {
        let Some(bot) = self.bot().cloned() else {
            self.welcome(ui);
            return;
        };
        if !self.threads.with(&bot.id, |t| t.loaded) {
            self.threads.load(&self.engine.db, &bot.id);
        }

        egui::Panel::top("chat_header")
            .frame(egui::Frame::NONE.fill(BG).inner_margin(Margin::symmetric(20, 12)).stroke(Stroke::new(1.0, LINE)))
            .show(ui, |ui| self.chat_header(ui, &bot));

        egui::Panel::bottom("composer")
            .frame(egui::Frame::NONE.fill(BG).inner_margin(Margin { left: 20, right: 20, top: 8, bottom: 16 }))
            .show(ui, |ui| {
                self.waiting_banner(ui, &bot);
                self.composer(ui, &bot);
            });

        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(BG).inner_margin(Margin::symmetric(0, 0))).show(ui, |ui| {
            self.thread(ui, &bot);
        });
    }

    fn welcome(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(BG)).show(ui, |ui| {
            ui.add_space(ui.available_height() * 0.25);
            ui.vertical_centered(|ui| {
                mark(ui, 56.0);
                ui.add_space(8.0);
                ui.label(heading("Meet your AI teammates"));
                ui.label(muted("Each teammate has its own computer: a browser and a desktop it works in while you watch."));
                ui.label(muted("Give it a task, take over any time, and it asks before anything that matters."));
                ui.add_space(14.0);
                if ui.add(accent_button("+  Create your first teammate").min_size(Vec2::new(260.0, 38.0))).clicked() {
                    self.chat.open_new_bot(&self.engine);
                }
                if PROVIDERS.iter().all(|p| self.engine.key_source(p).is_none()) {
                    ui.add_space(8.0);
                    if ui.link("Add an API key in Settings first").clicked() {
                        self.view = super::View::Settings;
                    }
                }
            });
        });
    }

    fn chat_header(&mut self, ui: &mut egui::Ui, bot: &Bot) {
        let width = ui.available_width();
        let narrow = width < 540.0;
        ui.horizontal(|ui| {
            avatar(ui, &bot.name, 36.0, None);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(RichText::new(&bot.name).size(16.5).strong());
                if !narrow {
                    let mode = MODES.iter().find(|m| m.0 == bot.mode).map(|m| m.1).unwrap_or("Ask first");
                    ui.label(small(format!("{} · {} · {mode}", bot.model, bot.effort)));
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(ghost_button("Edit")).clicked() {
                    self.chat.bot_form = Some(BotForm::from_bot(bot));
                }
                let label = match (self.show_computer, narrow) {
                    (true, false) => "Hide computer",
                    (true, true) => "Hide",
                    (false, false) => "🖥  Agent computer",
                    (false, true) => "🖥",
                };
                let b = if self.show_computer { ghost_button(label) } else { accent_button(label) };
                if ui.add(b).on_hover_text("The bot's own computer: watch it, or take over").clicked() {
                    self.show_computer = !self.show_computer;
                }
                if self.engine.is_running(&bot.id) && width > 700.0 {
                    ui.add(egui::Spinner::new().size(14.0).color(ACCENT));
                    ui.label(RichText::new("Working").color(ACCENT));
                }
            });
        });
    }

    fn thread(&mut self, ui: &mut egui::Ui, bot: &Bot) {
        let (items, live_text, live_thinking, working, waiting) = self.threads.with(&bot.id, |t| (t.items.clone(), t.live_text.clone(), t.live_thinking.clone(), t.working, t.waiting.is_some()));
        let running = self.engine.is_running(&bot.id);
        egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(16.0);
            let w = ui.available_width();
            let col = (w - 48.0).clamp(160.0, 820.0);
            let pad = ((w - col) / 2.0).max(8.0);
            ui.horizontal(|ui| {
                ui.add_space(pad);
                ui.vertical(|ui| {
                    ui.set_width(col);
                    if items.is_empty() && !running {
                        self.empty_thread(ui, bot);
                    }
                    for (i, item) in items.iter().enumerate() {
                        self.item(ui, bot, i, item);
                    }
                    if !live_thinking.trim().is_empty() {
                        let tail: String = live_thinking.chars().rev().take(280).collect::<Vec<_>>().into_iter().rev().collect();
                        ui.horizontal_wrapped(|ui| {
                            ui.add(egui::Spinner::new().size(12.0).color(THINK));
                            ui.label(RichText::new(format!("Thinking… {}", tail.replace('\n', " "))).italics().color(THINK).size(13.0));
                        });
                    }
                    if !live_text.is_empty() {
                        bot_bubble(ui, &bot.name, &live_text);
                    } else if (working || running) && !waiting && live_thinking.trim().is_empty() {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new().size(14.0).color(ACCENT));
                            ui.label(muted("Working…"));
                        });
                    }
                    ui.add_space(20.0);
                });
            });
        });
    }

    fn empty_thread(&mut self, ui: &mut egui::Ui, bot: &Bot) {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            avatar(ui, &bot.name, 56.0, None);
            ui.add_space(6.0);
            ui.label(heading(format!("What should {} do?", bot.name)));
            ui.label(muted("It works in its own browser. Open the agent computer to watch, or take over."));
        });
        ui.add_space(16.0);
        let ideas = [
            "Find three well-reviewed Thai restaurants near me open tonight and compare them",
            "Check my GitHub notifications and summarise what needs me",
            "Research the best 27\" monitors under $400 and make a comparison table",
            "Sign in to my bank and download last month's statement",
        ];
        ui.horizontal_wrapped(|ui| {
            for idea in ideas {
                if ui.add(egui::Button::new(RichText::new(idea).color(TEXT2).size(13.0)).fill(PANEL).stroke(Stroke::new(1.0, LINE)).corner_radius(10)).clicked() {
                    self.drafts.insert(bot.id.clone(), idea.to_string());
                }
            }
        });
    }

    fn item(&mut self, ui: &mut egui::Ui, bot: &Bot, i: usize, item: &Item) {
        match item {
            Item::You(text) => {
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    egui::Frame::NONE.fill(RAISED).corner_radius(14).inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
                        ui.set_max_width(ui.available_width() * 0.75);
                        ui.add(egui::Label::new(RichText::new(text).color(TEXT)).wrap().selectable(true));
                    });
                });
                ui.add_space(6.0);
            }
            Item::Bot(text) => bot_bubble(ui, &bot.name, text),
            Item::Thinking(text) => {
                egui::CollapsingHeader::new(RichText::new("Thought").color(THINK).size(13.0)).id_salt(("think", i)).default_open(false).show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(text).color(MUTED).size(13.0)).wrap().selectable(true));
                });
            }
            Item::Step { name, input, result } => {
                let (icon, color) = match result {
                    None => ("○", MUTED),
                    Some((_, false)) => ("✔", GOOD),
                    Some((_, true)) => ("✖", BAD),
                };
                let mut job = egui::text::LayoutJob::default();
                let font = egui::FontId::proportional(13.0);
                job.append(&format!("{icon}  "), 0.0, egui::TextFormat::simple(font.clone(), color));
                job.append(&format!("{}  ", verb(name)), 0.0, egui::TextFormat::simple(font.clone(), if result.is_some() { TEXT2 } else { MUTED }));
                // One line, cut short: a long URL must not widen the column.
                let fit = ((ui.available_width() - 60.0 - 7.0 * verb(name).len() as f32) / 6.6).max(12.0) as usize;
                let shown = if input.chars().count() > fit { format!("{}…", input.chars().take(fit.saturating_sub(1)).collect::<String>()) } else { input.clone() };
                job.append(&shown, 0.0, egui::TextFormat::simple(font, MUTED));
                egui::CollapsingHeader::new(job).id_salt(("step", i)).default_open(false).icon(|_, _, _| {}).show(ui, |ui| {
                    egui::Frame::NONE.fill(PANEL).corner_radius(8).inner_margin(Margin::same(10)).show(ui, |ui| {
                        ui.label(small(format!("tool: {name}")));
                        match result {
                            Some((r, err)) => {
                                let shown: String = r.chars().take(4000).collect();
                                ui.add(egui::Label::new(RichText::new(shown).monospace().size(12.0).color(if *err { BAD } else { TEXT2 })).wrap().selectable(true));
                            }
                            None => {
                                ui.label(muted("Running…"));
                            }
                        }
                    });
                });
            }
            Item::Paused { pause, .. } => {
                egui::Frame::NONE.fill(PANEL).stroke(Stroke::new(1.0, WARN)).corner_radius(12).inner_margin(Margin::same(12)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let title = if pause.confirm { "Asked you to confirm" } else { "Stopped for you" };
                    ui.label(RichText::new(title).color(WARN).strong());
                    ui.add(egui::Label::new(RichText::new(&pause.reason).color(TEXT)).wrap());
                    if !pause.url.is_empty() {
                        ui.label(small(&pause.url));
                    }
                });
                ui.add_space(6.0);
            }
            Item::Note(text) => {
                ui.vertical_centered(|ui| ui.label(small(text)));
            }
            Item::Done(text) if text.trim().is_empty() || text.trim().eq_ignore_ascii_case("done") => {
                ui.label(RichText::new("✔ Done").color(GOOD).size(13.0));
                ui.add_space(6.0);
            }
            Item::Done(text) => {
                egui::Frame::NONE.fill(PANEL).stroke(Stroke::new(1.0, GOOD.gamma_multiply(0.5))).corner_radius(12).inner_margin(Margin::same(12)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("✔ Done").color(GOOD).strong());
                    ui.add(egui::Label::new(RichText::new(text).color(TEXT)).wrap().selectable(true));
                });
                ui.add_space(6.0);
            }
            Item::Problem(text) => {
                egui::Frame::NONE.fill(PANEL).stroke(Stroke::new(1.0, BAD.gamma_multiply(0.6))).corner_radius(12).inner_margin(Margin::same(12)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Stopped").color(BAD).strong());
                    ui.add(egui::Label::new(RichText::new(text).color(TEXT)).wrap().selectable(true));
                    if text.contains("API key") && ui.link("Open Settings → Keys").clicked() {
                        self.view = super::View::Settings;
                    }
                });
                ui.add_space(6.0);
            }
        }
    }

    /// The bot is waiting on the human: take over / done / skip, or approve / decline.
    fn waiting_banner(&mut self, ui: &mut egui::Ui, bot: &Bot) {
        let Some((thread_task, pause)) = self.threads.with(&bot.id, |t| t.waiting.clone()) else { return };
        let task = self.engine.runs.lock().unwrap().get(&bot.id).map(|r| r.task_id.clone()).unwrap_or(thread_task);
        egui::Frame::NONE.fill(ACCENT_WASH).stroke(Stroke::new(1.0, ACCENT)).corner_radius(12).inner_margin(Margin::same(12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(if pause.confirm { format!("{} needs your OK", bot.name) } else { format!("{} needs you", bot.name) }).color(ACCENT).strong());
            ui.add(egui::Label::new(RichText::new(&pause.reason).color(TEXT)).wrap());
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if pause.confirm {
                    if ui.add(accent_button("Approve")).clicked() {
                        self.engine.handoffs.resume(&task, false);
                    }
                    if ui.add(danger_button("Decline")).clicked() {
                        self.engine.handoffs.resume(&task, true);
                    }
                } else {
                    if ui.add(ghost_button("Take over")).on_hover_text("Use the bot's browser yourself in the agent computer").clicked() {
                        self.show_computer = true;
                        self.computer.takeover = true;
                    }
                    if ui.add(accent_button("I'm done")).on_hover_text("Hand control back to the bot").clicked() {
                        self.computer.takeover = false;
                        self.engine.handoffs.resume(&task, false);
                    }
                    if ui.add(ghost_button("Skip")).on_hover_text("Carry on without this step").clicked() {
                        self.engine.handoffs.resume(&task, true);
                    }
                }
            });
        });
        ui.add_space(8.0);
    }

    fn composer(&mut self, ui: &mut egui::Ui, bot: &Bot) {
        let running = self.engine.is_running(&bot.id);
        let draft = self.drafts.entry(bot.id.clone()).or_default();
        let id = Id::new(("composer", &bot.id));
        let focused = ui.memory(|m| m.has_focus(id));
        let mut send = focused && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));

        card().inner_margin(Margin { left: 14, right: 10, top: 10, bottom: 8 }).corner_radius(16).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let hint = match &self.chat.skill {
                Some(s) => format!("Anything to add for \"{s}\"? (optional)"),
                None => format!("Ask {} to do something…", bot.name),
            };
            ui.add(egui::TextEdit::multiline(draft).id(id).hint_text(hint).desired_rows(2).desired_width(f32::INFINITY).frame(egui::Frame::NONE).font(egui::TextStyle::Body));
            ui.horizontal(|ui| {
                let label = match &self.chat.skill {
                    Some(s) => format!("✨ {s}  ✖"),
                    None => "✨ Skill".into(),
                };
                let r = ui.add(egui::Button::new(RichText::new(label).size(13.0).color(if self.chat.skill.is_some() { ACCENT } else { TEXT2 })).fill(RAISED).corner_radius(8));
                if r.clicked() {
                    if self.chat.skill.is_some() {
                        self.chat.skill = None;
                    } else {
                        self.chat.skill_picker = !self.chat.skill_picker;
                    }
                }
                if ui.available_width() > 360.0 {
                    ui.label(small("Enter to send · Shift+Enter for a new line"));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if running {
                        if ui.add(danger_button("■  Stop")).clicked() {
                            self.engine.stop(&bot.id);
                        }
                    } else {
                        let can = !draft.trim().is_empty() || self.chat.skill.is_some();
                        if ui.add_enabled(can, accent_button("Send")).clicked() {
                            send = true;
                        }
                    }
                });
            });
        });

        if self.chat.skill_picker {
            self.skill_picker(ui);
        }

        if send && !running {
            let text = self.drafts.get(&bot.id).cloned().unwrap_or_default().trim().to_string();
            if text.is_empty() && self.chat.skill.is_none() {
                return;
            }
            let skill = match self.chat.skill.take() {
                Some(name) => match self.engine.skill_run(&name, &BTreeMap::new()) {
                    Ok(s) => Some(s),
                    Err(e) => {
                        self.toast_err(e);
                        return;
                    }
                },
                None => None,
            };
            self.send(bot.clone(), text, skill);
            self.drafts.insert(bot.id.clone(), String::new());
        }
    }

    pub(super) fn send(&mut self, bot: Bot, text: String, skill: Option<SkillRun>) {
        let instruction = match (&skill, text.is_empty()) {
            (Some(s), true) => format!("Run the skill \"{}\".", s.name),
            (Some(s), false) => format!("Run the skill \"{}\". {text}", s.name),
            (None, _) => text,
        };
        self.threads.with(&bot.id, |t| {
            t.items.push(Item::You(instruction.clone()));
            t.working = true;
        });
        match self.engine.start_run(&self.rt, bot.clone(), instruction, skill, self.sink.clone()) {
            Ok(task) => self.threads.with(&bot.id, |t| t.task_id = Some(task.id)),
            Err(e) => {
                self.threads.with(&bot.id, |t| {
                    t.working = false;
                    t.items.push(Item::Problem(e.clone()));
                });
            }
        }
    }

    fn skill_picker(&mut self, ui: &mut egui::Ui) {
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add(input(&mut self.chat.skill_query).hint_text("Search 245 built-in skills and yours…").desired_width((ui.available_width() - 70.0).max(120.0)));
                if ui.add(ghost_button("Close")).clicked() {
                    self.chat.skill_picker = false;
                }
            });
            egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                let q = self.chat.skill_query.trim().to_string();
                let mut picked = None;
                let mine: Vec<_> = self.engine.db.skills().unwrap_or_default().into_iter().filter(|s| s.source != "imported" || s.enabled).filter(|s| q.is_empty() || s.name.to_lowercase().contains(&q.to_lowercase()) || s.description.to_lowercase().contains(&q.to_lowercase())).take(20).collect();
                for s in &mine {
                    if skill_row(ui, &s.name, &s.description, &s.source).clicked() {
                        picked = Some(s.name.clone());
                    }
                }
                let builtins = if q.is_empty() { mybot_catalog::skills().iter().take(30).collect::<Vec<_>>() } else { mybot_catalog::search_skills(&q, 30) };
                for s in builtins {
                    if skill_row(ui, &s.name, &s.description, &s.category).clicked() {
                        picked = Some(s.name.clone());
                    }
                }
                if let Some(p) = picked {
                    self.chat.skill = Some(p);
                    self.chat.skill_picker = false;
                }
            });
        });
    }

    // --- dialogs -----------------------------------------------------------------------

    pub(super) fn chat_modals(&mut self, ctx: &egui::Context) {
        self.bot_form_modal(ctx);
        if let Some(id) = self.chat.confirm_delete.clone() {
            let name = self.bots.iter().find(|b| b.id == id).map(|b| b.name.clone()).unwrap_or_default();
            let r = egui::Modal::new(Id::new("del_bot")).show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(heading(format!("Delete {name}?")));
                ui.label(muted("Its conversation history goes too. Saved logins and skills stay."));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add(danger_button("Delete")).clicked() {
                        self.engine.stop(&id);
                        match self.engine.db.delete_bot(&id) {
                            Ok(()) => {
                                self.threads.0.lock().unwrap().remove(&id);
                                self.chat.confirm_delete = None;
                                self.chat.bot_form = None;
                                self.reload_bots();
                            }
                            Err(e) => self.toast_err(e.to_string()),
                        }
                    }
                    if ui.add(ghost_button("Cancel")).clicked() {
                        self.chat.confirm_delete = None;
                    }
                });
            });
            if r.should_close() {
                self.chat.confirm_delete = None;
            }
        }
    }

    fn bot_form_modal(&mut self, ctx: &egui::Context) {
        let Some(f) = self.chat.bot_form.as_mut() else { return };
        // Live model list for the chosen provider.
        if f.models_for != f.provider {
            f.models_for = f.provider.clone();
            f.models = fallback_models(&f.provider).into_iter().map(String::from).collect();
            let engine = self.engine.clone();
            let p = f.provider.clone();
            f.models_job = Some(Job::spawn(&self.rt, ctx, async move {
                let provider = engine.provider(&p).map_err(|e| e.to_string())?;
                provider.list_models().await.map_err(|e| e.to_string())
            }));
        }
        if let Some(Ok(mut live)) = poll(&mut f.models_job) {
            live.sort();
            live.dedup();
            if !live.is_empty() {
                f.models = live;
            }
        }
        let mut save = false;
        let mut close = false;
        let mut delete = None;
        let editing = f.id.is_some();
        let r = egui::Modal::new(Id::new("bot_form")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.label(heading(if editing { "Edit teammate" } else { "New teammate" }));
            ui.add_space(6.0);
            egui::Grid::new("bot_grid").num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
                ui.label(muted("Name"));
                ui.add(input(&mut f.name).hint_text("e.g. Researcher, Ops, Shopper").desired_width(360.0));
                ui.end_row();

                ui.label(muted("Provider"));
                ui.horizontal(|ui| {
                    for p in PROVIDERS {
                        let ready = self.engine.key_source(p).is_some() || self.engine.base_url(p).is_some();
                        let text = format!("{}{}", mybot_core::providers::family_label(p), if ready { "" } else { " (no key)" });
                        if ui.selectable_label(f.provider == p, text).clicked() && f.provider != p {
                            f.provider = p.into();
                            f.model = mybot_core::providers::default_model(p).into();
                        }
                    }
                });
                ui.end_row();

                ui.label(muted("Model"));
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("model").selected_text(&f.model).width(280.0).show_ui(ui, |ui| {
                        for m in &f.models {
                            ui.selectable_value(&mut f.model, m.clone(), m);
                        }
                    });
                    if f.models_job.is_some() {
                        ui.add(egui::Spinner::new().size(12.0));
                    }
                });
                ui.end_row();

                ui.label(muted("Effort"));
                ui.horizontal(|ui| {
                    for e in EFFORTS {
                        ui.selectable_value(&mut f.effort, e.to_string(), e);
                    }
                });
                ui.end_row();

                ui.label(muted("Asks before"));
                ui.vertical(|ui| {
                    for (key, label, help) in MODES {
                        ui.radio_value(&mut f.mode, key.to_string(), label).on_hover_text(help);
                    }
                    if let Some(m) = MODES.iter().find(|m| m.0 == f.mode) {
                        ui.label(small(m.2));
                    }
                });
                ui.end_row();

                ui.label(muted("Persona"));
                ui.add(egui::TextEdit::multiline(&mut f.system).hint_text("Optional: how this teammate should work, what it focuses on…").desired_rows(3).desired_width(360.0));
                ui.end_row();
            });
            if let Some(e) = &f.error {
                ui.label(RichText::new(e).color(BAD));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.add(accent_button(if editing { "Save" } else { "Create" })).clicked() {
                    save = true;
                }
                if ui.add(ghost_button("Cancel")).clicked() {
                    close = true;
                }
                if let Some(id) = &f.id {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(danger_button("Delete teammate")).clicked() {
                            delete = Some(id.clone());
                        }
                    });
                }
            });
        });
        if r.should_close() {
            close = true;
        }
        if let Some(id) = delete {
            self.chat.confirm_delete = Some(id);
            return;
        }
        if save {
            let f = self.chat.bot_form.as_mut().unwrap();
            let name = f.name.trim().to_string();
            if name.is_empty() {
                f.error = Some("Give your teammate a name.".into());
                return;
            }
            if self.bots.iter().any(|b| b.name.eq_ignore_ascii_case(&name) && Some(&b.id) != f.id.as_ref()) {
                f.error = Some(format!("There's already a teammate called {name}."));
                return;
            }
            let system = Some(f.system.trim()).filter(|s| !s.is_empty());
            let result = match &f.id {
                Some(id) => self.engine.db.bot(id).ok().flatten().ok_or("That teammate is gone.".to_string()).and_then(|mut b| {
                    b.name = name.clone();
                    b.provider = f.provider.clone();
                    b.model = f.model.clone();
                    b.system = system.map(String::from);
                    b.mode = f.mode.clone();
                    b.effort = f.effort.clone();
                    self.engine.db.update_bot(&b).map_err(|e| e.to_string()).map(|_| b)
                }),
                None => self.engine.db.create_bot(&name, &f.provider, &f.model, system).map_err(|e| e.to_string()).and_then(|mut b| {
                    b.mode = f.mode.clone();
                    b.effort = f.effort.clone();
                    self.engine.db.update_bot(&b).map_err(|e| e.to_string()).map(|_| b)
                }),
            };
            match result {
                Ok(b) => {
                    self.chat.bot_form = None;
                    self.reload_bots();
                    self.open_chat(&b.id);
                    self.toast(if editing { format!("Saved {}.", b.name) } else { format!("{} is ready.", b.name) });
                }
                Err(e) => self.chat.bot_form.as_mut().unwrap().error = Some(e),
            }
        } else if close {
            self.chat.bot_form = None;
        }
    }
}

fn bot_bubble(ui: &mut egui::Ui, name: &str, text: &str) {
    ui.horizontal_top(|ui| {
        avatar(ui, name, 26.0, None);
        ui.vertical(|ui| {
            ui.set_max_width(ui.available_width());
            markdownish(ui, text);
        });
    });
    ui.add_space(8.0);
}

/// Enough Markdown for chat: paragraphs, bullets, headings, **bold**, and code blocks.
fn markdownish(ui: &mut egui::Ui, text: &str) {
    let mut in_code = false;
    let mut code = String::new();
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            if in_code {
                egui::Frame::NONE.fill(egui::Color32::from_rgb(10, 10, 12)).corner_radius(8).inner_margin(Margin::same(10)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(egui::Label::new(RichText::new(code.trim_end()).monospace().size(12.5).color(TEXT2)).selectable(true));
                });
                code.clear();
            }
            in_code = !in_code;
            continue;
        }
        if in_code {
            code.push_str(line);
            code.push('\n');
            continue;
        }
        let t = line.trim_end();
        if t.is_empty() {
            ui.add_space(4.0);
        } else if let Some(h) = t.strip_prefix("### ").or(t.strip_prefix("## ")).or(t.strip_prefix("# ")) {
            ui.label(RichText::new(h).strong().size(15.5));
        } else if let Some(b) = t.trim_start().strip_prefix("- ").or(t.trim_start().strip_prefix("* ")) {
            ui.horizontal_top(|ui| {
                ui.label(RichText::new("•").color(ACCENT));
                inline(ui, b);
            });
        } else {
            inline(ui, t);
        }
    }
    if in_code && !code.is_empty() {
        ui.add(egui::Label::new(RichText::new(code.trim_end()).monospace().size(12.5)).selectable(true));
    }
}

/// One line of chat Markdown: **bold** and `code`, laid out as one wrapping job.
fn inline(ui: &mut egui::Ui, t: &str) {
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = ui.available_width();
    let body = egui::FontId::proportional(14.5);
    for (ci, chunk) in t.split('`').enumerate() {
        if ci % 2 == 1 {
            let mut f = egui::TextFormat::simple(egui::FontId::monospace(13.0), ACCENT);
            f.background = RAISED;
            job.append(chunk, 0.0, f);
            continue;
        }
        for (bi, part) in chunk.split("**").enumerate() {
            let color = if bi % 2 == 1 { egui::Color32::WHITE } else { TEXT };
            job.append(part, 0.0, egui::TextFormat::simple(body.clone(), color));
        }
    }
    ui.add(egui::Label::new(job).wrap().selectable(true));
}

fn skill_row(ui: &mut egui::Ui, name: &str, desc: &str, tag: &str) -> egui::Response {
    let r = ui.add(egui::Button::new(RichText::new(format!("{name}  ·  {tag}")).color(TEXT)).fill(egui::Color32::TRANSPARENT).frame(false));
    ui.label(small(desc.chars().take(120).collect::<String>()));
    ui.add_space(2.0);
    r
}

/// Re-exported for the Skills screen's "Run with…" dialog.
pub fn start_skill(app: &mut App, bot: Bot, name: &str, args: BTreeMap<String, String>, extra: String) {
    match app.engine.skill_run(name, &args) {
        Ok(s) => {
            app.open_chat(&bot.id);
            app.send(bot, extra, Some(s));
        }
        Err(e) => app.toast_err(e),
    }
}
