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
        let provider = PROVIDERS.iter().find(|p| engine.ready(p)).copied().unwrap_or("anthropic");
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
            .frame(egui::Frame::NONE.fill(BG).inner_margin(Margin::symmetric(20, 11)).stroke(Stroke::new(1.0, LINE_SOFT)))
            .show(ui, |ui| self.chat_header(ui, &bot));

        egui::Panel::bottom("composer")
            .frame(egui::Frame::NONE.fill(BG).inner_margin(Margin { left: 20, right: 20, top: 6, bottom: 16 }))
            .show(ui, |ui| {
                // Same column as the thread, so the composer sits under the words.
                let w = ui.available_width();
                let col = (w - 16.0).clamp(160.0, 800.0);
                ui.horizontal(|ui| {
                    ui.add_space(((w - col) / 2.0).max(0.0));
                    ui.vertical(|ui| {
                        ui.set_width(col);
                        self.waiting_banner(ui, &bot);
                        self.composer(ui, &bot);
                    });
                });
            });

        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(BG).inner_margin(Margin::symmetric(0, 0))).show(ui, |ui| {
            self.thread(ui, &bot);
        });
    }

    fn welcome(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(BG)).show(ui, |ui| {
            ui.add_space((ui.available_height() * 0.26).max(30.0));
            ui.vertical_centered(|ui| {
                app_tile(ui, 76.0);
                ui.add_space(14.0);
                ui.label(RichText::new("Meet a teammate").font(semibold(24.0)).color(TEXT));
                ui.add_space(2.0);
                ui.label(muted("Each teammate gets its own computer — a browser and a desktop it works in."));
                ui.label(muted("Message one the way you'd message a colleague. Take over any time."));
                ui.add_space(18.0);
                if ui.add(accent_button("New teammate").min_size(Vec2::new(180.0, 38.0))).clicked() {
                    self.chat.open_new_bot(&self.engine);
                }
                if PROVIDERS.iter().all(|p| !self.engine.ready(p)) {
                    ui.add_space(10.0);
                    if ui.add(link_button("Sign in with ChatGPT or add an API key in Settings")).clicked() {
                        self.view = super::View::Settings;
                    }
                }
            });
        });
    }

    fn chat_header(&mut self, ui: &mut egui::Ui, bot: &Bot) {
        let width = ui.available_width();
        let narrow = width < 560.0;
        let live = self.computer.view.as_ref().is_some_and(|v| v.is_connected()) && self.computer.bot.as_deref() == Some(bot.name.as_str());
        ui.horizontal(|ui| {
            avatar(ui, &bot.name, 34.0, None);
            ui.add_space(2.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 1.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&bot.name).font(semibold(15.5)).color(TEXT));
                    chip(ui, mybot_core::providers::family_label(&bot.provider));
                });
                if !narrow {
                    let mode = MODES.iter().find(|m| m.0 == bot.mode).map(|m| m.1).unwrap_or("Ask first");
                    ui.label(small(format!("{} · {} effort · {mode}", bot.model, bot.effort)));
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(ghost_button("Edit")).on_hover_text("Name, model, effort and when it asks you").clicked() {
                    self.chat.bot_form = Some(BotForm::from_bot(bot));
                }
                let label = if narrow { "Computer" } else { "Agent computer" };
                if computer_pill(ui, label, live, self.show_computer).on_hover_text("The teammate's own computer: watch it work, or take over").clicked() {
                    self.show_computer = !self.show_computer;
                }
                if self.engine.is_running(&bot.id) && width > 720.0 {
                    ui.add_space(4.0);
                    ui.label(RichText::new("Working").color(ACCENT).size(12.5));
                    ui.add(egui::Spinner::new().size(12.0).color(ACCENT));
                }
            });
        });
    }

    fn thread(&mut self, ui: &mut egui::Ui, bot: &Bot) {
        let (items, live_text, live_thinking, working, waiting) = self.threads.with(&bot.id, |t| (t.items.clone(), t.live_text.clone(), t.live_thinking.clone(), t.working, t.waiting.is_some()));
        let running = self.engine.is_running(&bot.id);
        egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(20.0);
            let w = ui.available_width();
            let col = (w - 56.0).clamp(160.0, 760.0);
            let pad = ((w - col) / 2.0).max(10.0);
            ui.horizontal(|ui| {
                ui.add_space(pad);
                ui.vertical(|ui| {
                    ui.set_width(col);
                    ui.spacing_mut().item_spacing.y = 4.0;
                    if items.is_empty() && !running {
                        self.empty_thread(ui, bot);
                    }
                    let mut prev: Option<&Item> = None;
                    for (i, item) in items.iter().enumerate() {
                        // A turn is: what you asked, what it did, what it said.
                        // The seams between those get more air than the lines inside.
                        if let Some(p) = prev
                            && std::mem::discriminant(p) != std::mem::discriminant(item)
                            && (matches!(item, Item::You(_) | Item::Bot(_)) || matches!(p, Item::You(_) | Item::Bot(_)))
                        {
                            ui.add_space(8.0);
                        }
                        self.item(ui, bot, i, item);
                        prev = Some(item);
                    }
                    if !live_thinking.trim().is_empty() {
                        let tail: String = live_thinking.chars().rev().take(240).collect::<Vec<_>>().into_iter().rev().collect();
                        ui.add_space(6.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.add(egui::Spinner::new().size(11.0).color(THINK));
                            ui.label(RichText::new(format!("Thinking… {}", tail.replace('\n', " "))).italics().color(THINK).size(12.5));
                        });
                    }
                    if !live_text.is_empty() {
                        ui.add_space(8.0);
                        bot_bubble(ui, &live_text);
                    } else if (working || running) && !waiting && live_thinking.trim().is_empty() {
                        ui.add_space(6.0);
                        working_dots(ui);
                    }
                    ui.add_space(24.0);
                });
            });
        });
    }

    fn empty_thread(&mut self, ui: &mut egui::Ui, bot: &Bot) {
        ui.add_space(48.0);
        ui.vertical_centered(|ui| {
            avatar(ui, &bot.name, 60.0, None);
            ui.add_space(10.0);
            ui.label(RichText::new(format!("What should {} do?", bot.name)).font(semibold(20.0)).color(TEXT));
            ui.label(muted("It works in its own browser. Open the agent computer to watch, or take over."));
        });
        ui.add_space(20.0);
        let ideas = [
            "Find three well-reviewed Thai restaurants near me open tonight and compare them",
            "Check my GitHub notifications and summarise what needs me",
            "Research the best 27\" monitors under $400 and make a comparison table",
            "Sign in to my bank and download last month's statement",
        ];
        for idea in ideas {
            let r = ui.add(
                egui::Button::new(RichText::new(idea).color(TEXT2).size(13.0))
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0, LINE_SOFT))
                    .corner_radius(12)
                    .min_size(Vec2::new(ui.available_width(), 40.0)),
            );
            if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.drafts.insert(bot.id.clone(), idea.to_string());
            }
        }
    }

    fn item(&mut self, ui: &mut egui::Ui, _bot: &Bot, i: usize, item: &Item) {
        match item {
            Item::You(text) => {
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    egui::Frame::NONE
                        .fill(ACCENT)
                        .corner_radius(egui::CornerRadius { nw: 16, ne: 16, sw: 16, se: 5 })
                        .inner_margin(Margin::symmetric(14, 9))
                        .show(ui, |ui| {
                            ui.set_max_width(ui.available_width() * 0.78);
                            ui.add(egui::Label::new(RichText::new(text).color(ACCENT_INK)).wrap().selectable(true));
                        });
                });
            }
            Item::Bot(text) => bot_bubble(ui, text),
            Item::Thinking(text) => {
                let words = text.split_whitespace().count();
                egui::CollapsingHeader::new(RichText::new(format!("Thought for {words} word{}", if words == 1 { "" } else { "s" })).color(THINK).size(12.5)).id_salt(("think", i)).default_open(false).show(ui, |ui| {
                    egui::Frame::NONE.stroke(Stroke::new(1.0, LINE_SOFT)).corner_radius(10).inner_margin(Margin::same(10)).show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(text).color(MUTED).size(12.5)).wrap().selectable(true));
                    });
                });
            }
            Item::Step { name, input, result } => step_row(ui, i, name, input, result.as_ref()),
            Item::Paused { pause, .. } => {
                notice(ui, if pause.confirm { "Asked you to confirm" } else { "Stopped for you" }, ACCENT, |ui| {
                    ui.add(egui::Label::new(RichText::new(&pause.reason).color(TEXT)).wrap());
                    if !pause.url.is_empty() {
                        ui.label(small(&pause.url));
                    }
                });
            }
            Item::Note(text) => {
                ui.add_space(2.0);
                ui.vertical_centered(|ui| ui.label(RichText::new(text).color(FAINT).size(12.0)));
                ui.add_space(2.0);
            }
            Item::Done(text) => {
                let plain = text.trim().is_empty() || text.trim().eq_ignore_ascii_case("done");
                if !plain {
                    bot_bubble(ui, text);
                }
                ui.horizontal(|ui| {
                    ui.add_space(4.0);
                    ui.label(RichText::new("✓  Done").color(GOOD).size(12.5));
                });
            }
            Item::Problem(text) => {
                notice(ui, "Stopped", BAD, |ui| {
                    ui.add(egui::Label::new(RichText::new(text).color(TEXT)).wrap().selectable(true));
                    if text.contains("API key") && ui.add(link_button("Open Settings → Keys")).clicked() {
                        self.view = super::View::Settings;
                    }
                });
            }
        }
    }

    /// The teammate is waiting on you. The same three answers the agent
    /// computer offers, so you can answer without opening it.
    fn waiting_banner(&mut self, ui: &mut egui::Ui, bot: &Bot) {
        let Some((thread_task, pause)) = self.threads.with(&bot.id, |t| t.waiting.clone()) else { return };
        let task = self.engine.runs.lock().unwrap().get(&bot.id).map(|r| r.task_id.clone()).unwrap_or(thread_task);
        egui::Frame::NONE.fill(ACCENT_WASH).stroke(Stroke::new(1.0, ACCENT_EDGE)).corner_radius(16).inner_margin(Margin::symmetric(14, 12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                avatar(ui, &bot.name, 22.0, None);
                ui.label(RichText::new(if pause.confirm { format!("{} needs your OK", bot.name) } else { format!("{} needs you", bot.name) }).font(semibold(14.0)).color(ACCENT));
            });
            ui.add(egui::Label::new(RichText::new(&pause.reason).color(TEXT)).wrap());
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if pause.confirm {
                    if ui.add(accent_button("Approve")).clicked() {
                        self.engine.handoffs.resume(&task, false);
                    }
                    if ui.add(ghost_button("Decline")).clicked() {
                        self.engine.handoffs.resume(&task, true);
                    }
                } else {
                    if ui.add(light_button("I'm done, continue")).on_hover_text("Hand control back to the teammate").clicked() {
                        self.computer.takeover = false;
                        self.engine.handoffs.resume(&task, false);
                    }
                    if !self.show_computer && ui.add(ghost_button("Take over")).on_hover_text("Open the agent computer and use its browser yourself").clicked() {
                        self.show_computer = true;
                        self.computer.takeover = true;
                    }
                    if ui.add(link_button("Skip this step")).on_hover_text("Carry on without this step").clicked() {
                        self.engine.handoffs.resume(&task, true);
                    }
                }
            });
        });
        ui.add_space(10.0);
    }

    fn composer(&mut self, ui: &mut egui::Ui, bot: &Bot) {
        let running = self.engine.is_running(&bot.id);
        let draft = self.drafts.entry(bot.id.clone()).or_default();
        let id = Id::new(("composer", &bot.id));
        let focused = ui.memory(|m| m.has_focus(id));
        let mut send = focused && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
        let can = !draft.trim().is_empty() || self.chat.skill.is_some();

        let edge = if focused { ACCENT_EDGE } else { LINE };
        egui::Frame::NONE.fill(PANEL).stroke(Stroke::new(1.0, edge)).corner_radius(20).inner_margin(Margin::symmetric(8, 7)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if let Some(s) = self.chat.skill.clone() {
                ui.horizontal(|ui| {
                    ui.add_space(4.0);
                    let r = ui.add(egui::Button::new(RichText::new(format!("✨ {s}   ✕")).size(12.5).color(ACCENT)).fill(ACCENT_WASH).stroke(Stroke::new(1.0, ACCENT_EDGE)).corner_radius(255));
                    if r.on_hover_text("Don't use this skill").clicked() {
                        self.chat.skill = None;
                    }
                });
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let plus = super::round_icon_button(ui, super::Glyph::Plus).on_hover_text("Use a skill");
                if plus.clicked() {
                    self.chat.skill_picker = !self.chat.skill_picker;
                }
                let hint = match &self.chat.skill {
                    Some(s) => format!("Anything to add for \"{s}\"? (optional)"),
                    None => format!("Message {}", bot.name),
                };
                let w = (ui.available_width() - 44.0).max(60.0);
                ui.add(
                    egui::TextEdit::multiline(draft)
                        .id(id)
                        .hint_text(RichText::new(hint).color(FAINT))
                        .desired_rows(1)
                        .desired_width(w)
                        .frame(egui::Frame::NONE)
                        .margin(Margin::symmetric(4, 5))
                        .font(egui::TextStyle::Body),
                );
                if running {
                    if send_circle(ui, super::Glyph::Stop, true, BAD).on_hover_text("Stop").clicked() {
                        self.engine.stop(&bot.id);
                    }
                } else if send_circle(ui, super::Glyph::Up, can, ACCENT).on_hover_text("Send · Enter   (Shift+Enter for a new line)").clicked() && can {
                    send = true;
                }
            });
        });

        if self.chat.skill_picker {
            ui.add_space(6.0);
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
        let r = egui::Modal::new(Id::new("bot_form")).frame(modal_frame()).show(ctx, |ui| {
            ui.set_width(480.0);
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.horizontal(|ui| {
                let shown = if f.name.trim().is_empty() { "?" } else { f.name.trim() };
                avatar(ui, shown, 40.0, None);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(heading(if editing { "Edit teammate" } else { "New teammate" }));
                    ui.label(small("A persona over its own computer."));
                });
            });
            ui.add_space(10.0);

            field(ui, "Name");
            ui.add(input(&mut f.name).hint_text("e.g. Researcher, Ops, Shopper").desired_width(f32::INFINITY));

            field(ui, "Provider");
            let options: Vec<(String, String)> = PROVIDERS
                .iter()
                .map(|p| {
                    let label = mybot_core::providers::family_label(p);
                    let note = if !self.engine.ready(p) { " · no key" } else if *p == "openai" && self.engine.gpt_auth() == crate::engine::GptAuth::ChatGpt { " · ChatGPT" } else { "" };
                    (p.to_string(), format!("{label}{note}"))
                })
                .collect();
            let before = f.provider.clone();
            segmented(ui, &options, &mut f.provider);
            if f.provider != before {
                f.model = mybot_core::providers::default_model(&f.provider).into();
            }

            field(ui, "Model");
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("model").selected_text(&f.model).width(ui.available_width() - 30.0).show_ui(ui, |ui| {
                    for m in &f.models {
                        ui.selectable_value(&mut f.model, m.clone(), m);
                    }
                });
                if f.models_job.is_some() {
                    ui.add(egui::Spinner::new().size(12.0).color(ACCENT));
                }
            });

            field(ui, "Effort");
            let efforts: Vec<(String, String)> = EFFORTS.iter().map(|e| (e.to_string(), e.to_string())).collect();
            segmented(ui, &efforts, &mut f.effort);

            field(ui, "When it asks you");
            let modes: Vec<(String, String)> = MODES.iter().map(|m| (m.0.to_string(), m.1.to_string())).collect();
            segmented(ui, &modes, &mut f.mode);
            if let Some(m) = MODES.iter().find(|m| m.0 == f.mode) {
                ui.add(egui::Label::new(small(m.2)).wrap());
            }

            field(ui, "Persona  (optional)");
            ui.add(egui::TextEdit::multiline(&mut f.system).hint_text("How this teammate works, what it focuses on…").desired_rows(3).desired_width(f32::INFINITY).margin(Margin::symmetric(10, 8)));

            if let Some(e) = &f.error {
                ui.label(RichText::new(e).color(BAD));
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if let Some(id) = &f.id
                    && ui.add(danger_button("Delete")).clicked()
                {
                    delete = Some(id.clone());
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(accent_button(if editing { "Save" } else { "Create" }).min_size(Vec2::new(92.0, 32.0))).clicked() {
                        save = true;
                    }
                    if ui.add(ghost_button("Cancel")).clicked() {
                        close = true;
                    }
                });
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

fn bot_bubble(ui: &mut egui::Ui, text: &str) {
    egui::Frame::NONE
        .fill(PANEL)
        .stroke(Stroke::new(1.0, LINE_SOFT))
        .corner_radius(egui::CornerRadius { nw: 16, ne: 16, sw: 5, se: 16 })
        .inner_margin(Margin::symmetric(15, 11))
        .show(ui, |ui| {
            ui.set_max_width(ui.available_width() * 0.92);
            markdownish(ui, text);
        });
}

/// A boxed notice in the thread: a pause, or a run that stopped.
fn notice(ui: &mut egui::Ui, title: &str, tone: egui::Color32, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::NONE.fill(PANEL).stroke(Stroke::new(1.0, tone.gamma_multiply(0.45))).corner_radius(14).inner_margin(Margin::symmetric(14, 11)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title).font(semibold(13.5)).color(tone));
        body(ui);
    });
}

/// One action the teammate took: a dot, the verb, the target, on one tight
/// line. Click it for the raw arguments and result — the only thing that
/// helps when a long unattended run goes sideways.
fn step_row(ui: &mut egui::Ui, i: usize, name: &str, input: &str, result: Option<&(String, bool)>) {
    let (dot, ink) = match result {
        None => (ACCENT, MUTED),
        Some((_, false)) => (FAINT, TEXT2),
        Some((_, true)) => (BAD, BAD),
    };
    let id = ui.id().with(("step", i));
    let open = ui.data(|d| d.get_temp::<bool>(id).unwrap_or(false));
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), egui::Sense::click());
    let p = ui.painter();
    if resp.hovered() || open {
        p.rect_filled(rect.expand2(Vec2::new(0.0, 1.0)), 7, PANEL);
    }
    p.circle_filled(rect.left_center() + Vec2::new(10.0, 0.0), 2.6, dot);
    let verb_g = p.layout_no_wrap(verb(name).to_string(), egui::FontId::proportional(13.0), ink);
    let x = rect.left() + 20.0;
    let verb_w = verb_g.size().x;
    p.galley(egui::pos2(x, rect.center().y - verb_g.size().y / 2.0), verb_g, ink);
    let target = step_target(input);
    if !target.is_empty() {
        let mut job = egui::text::LayoutJob::single_section(target, egui::TextFormat::simple(egui::FontId::proportional(13.0), FAINT));
        job.wrap = egui::text::TextWrapping { max_width: (rect.right() - x - verb_w - 18.0).max(20.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
        let g = p.layout_job(job);
        p.galley(egui::pos2(x + verb_w + 9.0, rect.center().y - g.size().y / 2.0), g, FAINT);
    }
    if resp.clicked() {
        ui.data_mut(|d| d.insert_temp(id, !open));
    }
    let _ = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if open {
        egui::Frame::NONE.fill(PANEL).corner_radius(10).inner_margin(Margin::same(10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(small(format!("{name}  {input}")));
            match result {
                Some((r, err)) => {
                    let shown: String = r.chars().take(4000).collect();
                    ui.add(egui::Label::new(RichText::new(shown).monospace().size(11.5).color(if *err { BAD } else { TEXT2 })).wrap().selectable(true));
                }
                None => {
                    ui.label(muted("Running…"));
                }
            }
        });
    }
}

/// `url: https://www.example.com/x` → `example.com/x`; raw JSON → nothing.
fn step_target(input: &str) -> String {
    let t = input.trim();
    if t.starts_with('{') || t.starts_with('[') {
        return String::new();
    }
    let v = match t.split_once(": ") {
        Some((k, v)) if k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') => v,
        _ => t,
    };
    let v = v.trim_start_matches("https://").trim_start_matches("http://").trim_start_matches("www.");
    v.trim_end_matches('/').to_string()
}

/// "Working…" as three pulsing dots, so a long run doesn't look hung.
fn working_dots(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(54.0, 26.0), egui::Sense::hover());
    let t = ui.input(|i| i.time) as f32;
    ui.painter().rect_filled(rect, 13, PANEL);
    for k in 0..3 {
        let phase = (t * 4.0 - k as f32 * 0.7).sin() * 0.5 + 0.5;
        let c = rect.left_center() + Vec2::new(15.0 + k as f32 * 12.0, 0.0);
        ui.painter().circle_filled(c, 3.0, MUTED.gamma_multiply(0.45 + 0.55 * phase));
    }
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
}

/// The header's toggle for the agent computer: a pill with a live dot.
fn computer_pill(ui: &mut egui::Ui, label: &str, live: bool, open: bool) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(13.0), TEXT2);
    let size = Vec2::new(galley.size().x + 36.0, 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let p = ui.painter();
    let fill = if open { RAISED2 } else if resp.hovered() { RAISED } else { PANEL };
    p.rect_filled(rect, 255, fill);
    p.rect_stroke(rect, 255, Stroke::new(1.0, if open { ACCENT_EDGE } else { LINE }), egui::StrokeKind::Inside);
    p.circle_filled(rect.left_center() + Vec2::new(15.0, 0.0), 3.5, if live { GOOD } else { FAINT });
    p.galley(rect.left_center() + Vec2::new(25.0, -galley.size().y / 2.0), galley, if open { TEXT } else { TEXT2 });
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The round send (or stop) button at the end of the composer.
fn send_circle(ui: &mut egui::Ui, glyph: super::Glyph, enabled: bool, tone: egui::Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(34.0), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    let p = ui.painter();
    let stop = matches!(glyph, super::Glyph::Stop);
    let (fill, ink) = match (enabled, stop) {
        (true, false) => (if resp.hovered() { tone.gamma_multiply(1.1) } else { tone }, ACCENT_INK),
        (true, true) => (if resp.hovered() { RAISED2 } else { RAISED }, tone),
        (false, _) => (RAISED, FAINT),
    };
    p.circle_filled(rect.center(), 17.0, fill);
    super::paint_glyph(p, rect, glyph, ink);
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
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
                ui.label(RichText::new("•").color(ACCENT_EDGE.gamma_multiply(2.0)));
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

/// The look shared by every dialog.
pub fn modal_frame() -> egui::Frame {
    egui::Frame::NONE.fill(PANEL).stroke(Stroke::new(1.0, LINE)).corner_radius(18).inner_margin(Margin::same(22))
}

/// A small label above a form field.
fn field(ui: &mut egui::Ui, label: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(label).font(semibold(12.0)).color(MUTED));
}

/// A row of options in a track, one selected — the segmented control.
pub fn segmented(ui: &mut egui::Ui, options: &[(String, String)], value: &mut String) {
    egui::Frame::NONE.fill(RAISED).corner_radius(11).inner_margin(Margin::same(3)).show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.horizontal(|ui| {
            for (key, label) in options {
                let on = value == key;
                let r = ui.add(
                    egui::Button::new(RichText::new(label).size(13.0).color(if on { TEXT } else { MUTED }))
                        .fill(if on { RAISED2 } else { egui::Color32::TRANSPARENT })
                        .stroke(Stroke::NONE)
                        .corner_radius(8)
                        .min_size(Vec2::new(0.0, 26.0)),
                );
                if r.clicked() {
                    *value = key.clone();
                }
            }
        });
    });
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
