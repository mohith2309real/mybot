//! Skills, routines, saved logins and settings.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use egui::{Align, Color32, Id, Layout, Margin, RichText, Stroke, Vec2};
use mybot_actions::skills_store::{self, Review};
use mybot_catalog::agent_skill::Severity;
use mybot_computer::container;
use mybot_core::cron::Cron;
use mybot_core::db::Bot;
use mybot_core::providers::PROVIDERS;
use mybot_vault::logins::{LoginSummary, NewLogin};

use super::theme::*;
use super::{App, Job, poll};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SkillsTab {
    #[default]
    Library,
    Yours,
    Imported,
}

#[derive(Default)]
pub struct Screens {
    tab: SkillsTab,
    query: String,
    category: Option<String>,
    import_src: String,
    import_job: Option<Job<Result<Review, String>>>,
    review: Option<Review>,
    run: Option<RunForm>,
    edit: Option<EditForm>,
    routine: RoutineForm,
    login: LoginForm,
    logins: Option<Result<Vec<LoginSummary>, String>>,
    keys: HashMap<String, String>,
    bases: HashMap<String, String>,
    docker: Option<container::Status>,
    docker_job: Option<Job<container::Status>>,
    confirm_reset: bool,
    /// ChatGPT sign-in: the last check of the helper, and work in flight.
    gpt_probe: Option<Result<Vec<String>, String>>,
    gpt_probe_job: Option<Job<Result<Vec<String>, String>>>,
    gpt_probe_at: Option<std::time::Instant>,
    gpt_signin: Option<Job<Result<String, String>>>,
    gpt_note: Option<(String, bool)>,
}

impl Screens {
    /// Open Skills on the Imported tab (used by snapshots).
    pub(super) fn show_imported(&mut self) {
        self.tab = SkillsTab::Imported;
    }
}

struct RunForm {
    skill: String,
    inputs: Vec<String>,
    values: BTreeMap<String, String>,
    bot: Option<String>,
    extra: String,
}

struct EditForm {
    id: String,
    name: String,
    description: String,
    instructions: String,
    rules: String,
}

#[derive(Default)]
struct RoutineForm {
    skill: String,
    bot: Option<String>,
    schedule: String,
    args: String,
    error: Option<String>,
}

#[derive(Default)]
struct LoginForm {
    site: String,
    username: String,
    password: String,
    label: String,
    error: Option<String>,
}

const PRESETS: [(&str, &str); 5] = [
    ("Every morning at 8", "0 8 * * *"),
    ("Weekdays at 9", "0 9 * * 1-5"),
    ("Every hour", "0 * * * *"),
    ("Mondays at 9", "0 9 * * 1"),
    ("1st of the month", "0 9 1 * *"),
];

fn page(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(BG).inner_margin(Margin::symmetric(28, 20))).show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(980.0);
            add(ui);
            ui.add_space(30.0);
        });
    });
}

fn tab(ui: &mut egui::Ui, on: bool, label: &str) -> bool {
    let t = RichText::new(label).color(if on { TEXT } else { MUTED }).strong();
    let r = ui.add(egui::Button::new(t).fill(if on { RAISED } else { Color32::TRANSPARENT }).stroke(Stroke::NONE).corner_radius(8));
    r.clicked()
}

/// A dark tile with the item's first letter, as on the reference's cards.
fn letter_tile(ui: &mut egui::Ui, name: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(28.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 8, RAISED2);
    let letter = name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_ascii_uppercase().to_string()).unwrap_or_default();
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, letter, semibold(13.0), TEXT2);
}

fn chip(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::NONE.stroke(Stroke::new(1.0, color.gamma_multiply(0.6))).corner_radius(6).inner_margin(Margin::symmetric(6, 1)).show(ui, |ui| {
        ui.label(RichText::new(text).size(11.5).color(color));
    });
}

impl App {
    // ===================================================================== Skills

    pub(super) fn skills_view(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        // Drop a skill folder or .zip anywhere on the window to import it.
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        if let Some(path) = dropped.into_iter().next() {
            let db = self.engine.db.clone();
            self.screens.tab = SkillsTab::Imported;
            self.screens.import_job = Some(Job::blocking(&self.rt, &ctx, move || skills_store::import_path(&db, &path)));
        }
        if let Some(r) = poll(&mut self.screens.import_job) {
            match r {
                Ok(rev) => {
                    self.toast(format!("Imported “{}”. Review it, then turn it on.", rev.row.name));
                    self.screens.import_src.clear();
                    self.screens.review = Some(rev);
                }
                Err(e) => self.toast_err(format!("Import failed: {e}")),
            }
        }

        let all_mine = self.engine.db.skills().unwrap_or_default();
        let (imported, mine): (Vec<_>, Vec<_>) = all_mine.into_iter().partition(|s| s.source == "imported");

        page(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(heading("Skills"));
                ui.label(muted(format!("{} built in · {} yours · {} imported", mybot_catalog::skills().len(), mine.len(), imported.len())));
            });
            ui.label(muted("Step-by-step know-how a teammate follows. Run one from here or from the ✨ button in a chat."));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if tab(ui, self.screens.tab == SkillsTab::Library, "Library") {
                    self.screens.tab = SkillsTab::Library;
                }
                if tab(ui, self.screens.tab == SkillsTab::Yours, &format!("Yours ({})", mine.len())) {
                    self.screens.tab = SkillsTab::Yours;
                }
                if tab(ui, self.screens.tab == SkillsTab::Imported, &format!("Imported ({})", imported.len())) {
                    self.screens.tab = SkillsTab::Imported;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add(input(&mut self.screens.query).hint_text("🔍 Search skills").desired_width(260.0));
                });
            });
            ui.add_space(10.0);
            match self.screens.tab {
                SkillsTab::Library => self.library(ui),
                SkillsTab::Yours => self.yours(ui, &mine),
                SkillsTab::Imported => self.imported(ui, &imported),
            }
        });
    }

    fn library(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui.selectable_label(self.screens.category.is_none(), "All").clicked() {
                self.screens.category = None;
            }
            for (c, n) in mybot_catalog::skill_categories() {
                if ui.selectable_label(self.screens.category.as_deref() == Some(c), format!("{c} {n}")).clicked() {
                    self.screens.category = Some(c.to_string());
                }
            }
        });
        ui.add_space(8.0);
        let q = self.screens.query.trim().to_string();
        let list: Vec<&mybot_catalog::Skill> = if q.is_empty() { mybot_catalog::skills().iter().collect() } else { mybot_catalog::search_skills(&q, 300) };
        let list: Vec<_> = list.into_iter().filter(|s| self.screens.category.as_ref().is_none_or(|c| &s.category == c)).collect();
        if list.is_empty() {
            ui.label(muted("No skills match."));
        }
        let cols = ((ui.available_width() + 12.0) / 312.0).floor().max(1.0) as usize;
        let w = ((ui.available_width() - 12.0 * (cols as f32 - 1.0)) / cols as f32).max(160.0);
        for row in list.chunks(cols) {
            ui.horizontal_top(|ui| {
                for s in row {
                    ui.allocate_ui_with_layout(Vec2::new(w, 150.0), Layout::top_down(Align::Min), |ui| {
                        card().show(ui, |ui| {
                            ui.set_width(w - 30.0);
                            ui.set_min_height(118.0);
                            ui.horizontal(|ui| {
                                letter_tile(ui, &s.name);
                                ui.label(strong(&s.name));
                            });
                            ui.horizontal(|ui| {
                                chip(ui, &s.category, MUTED);
                                if s.safety.iter().any(|r| r.starts_with("always-confirm")) {
                                    chip(ui, "asks first", WARN);
                                }
                            });
                            ui.add(egui::Label::new(RichText::new(&s.description).color(TEXT2).size(13.0)).wrap());
                            ui.horizontal(|ui| {
                                if ui.add(accent_button("Run")).clicked() {
                                    self.screens.run = Some(RunForm { skill: s.name.clone(), inputs: s.inputs.clone(), values: BTreeMap::new(), bot: self.selected.clone(), extra: String::new() });
                                }
                                if ui.add(ghost_button("Schedule")).clicked() {
                                    self.screens.routine.skill = s.name.clone();
                                    self.view = super::View::Routines;
                                }
                                if ui.add(egui::Button::new(small("Export")).frame(false)).on_hover_text("Save as an Agent Skills folder (SKILL.md)").clicked() {
                                    self.export_skill(&s.name);
                                }
                            });
                        });
                    });
                }
            });
            ui.add_space(4.0);
        }
    }

    fn export_skill(&mut self, name: &str) {
        let dest = mybot_vault::home().join("exports");
        match skills_store::export(&self.engine.db, name, &dest) {
            Ok(p) => self.toast(format!("Exported to {}", p.display())),
            Err(e) => self.toast_err(e),
        }
    }

    fn yours(&mut self, ui: &mut egui::Ui, mine: &[mybot_core::db::SkillRow]) {
        ui.label(muted("Skills you wrote, and skills you taught by showing a teammate (Agent computer → Teach a task)."));
        ui.add_space(6.0);
        if ui.add(ghost_button("+ Write a skill")).clicked() {
            self.screens.edit = Some(EditForm { id: String::new(), name: String::new(), description: String::new(), instructions: String::new(), rules: String::new() });
        }
        ui.add_space(6.0);
        let q = self.screens.query.to_lowercase();
        for s in mine.iter().filter(|s| q.is_empty() || s.name.to_lowercase().contains(&q) || s.description.to_lowercase().contains(&q)) {
            card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&s.name).strong());
                    chip(ui, &s.source, if s.source == "taught" { ACCENT } else { MUTED });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(danger_button("Delete")).clicked() {
                            match self.engine.db.delete_skill(&s.id) {
                                Ok(()) => self.toast(format!("Deleted “{}”.", s.name)),
                                Err(e) => self.toast_err(e.to_string()),
                            }
                        }
                        if ui.add(egui::Button::new(small("Export")).frame(false)).clicked() {
                            self.export_skill(&s.name);
                        }
                        if ui.add(ghost_button("Edit")).clicked() {
                            self.screens.edit = Some(EditForm { id: s.id.clone(), name: s.name.clone(), description: s.description.clone(), instructions: s.instructions.clone(), rules: s.safety_rules.clone() });
                        }
                        if ui.add(ghost_button("Schedule")).clicked() {
                            self.screens.routine.skill = s.name.clone();
                            self.view = super::View::Routines;
                        }
                        if ui.add(accent_button("Run")).clicked() {
                            self.screens.run = Some(RunForm { skill: s.name.clone(), inputs: vec![], values: BTreeMap::new(), bot: self.selected.clone(), extra: String::new() });
                        }
                    });
                });
                if !s.description.is_empty() {
                    ui.label(RichText::new(&s.description).color(TEXT2).size(13.0));
                }
                let preview: String = s.instructions.lines().take(4).collect::<Vec<_>>().join("\n");
                ui.label(RichText::new(preview).color(MUTED).size(12.5));
            });
            ui.add_space(6.0);
        }
        if mine.is_empty() {
            ui.label(muted("None yet."));
        }
    }

    fn imported(&mut self, ui: &mut egui::Ui, imported: &[mybot_core::db::SkillRow]) {
        let ctx = ui.ctx().clone();
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Add a skill from anywhere").strong());
            ui.label(small("Agent Skills format (SKILL.md) — the same skills Claude uses. A folder, a .zip, or a GitHub folder link. You can also drop one on this window."));
            ui.horizontal(|ui| {
                ui.add(input(&mut self.screens.import_src).hint_text("~/skills/pdf  ·  skill.zip  ·  https://github.com/owner/repo/tree/main/skills/name").desired_width((ui.available_width() - 110.0).max(160.0)));
                let busy = self.screens.import_job.is_some();
                if ui.add_enabled(!busy && !self.screens.import_src.trim().is_empty(), accent_button(if busy { "Importing…" } else { "Import" })).clicked() {
                    let src = self.screens.import_src.trim().to_string();
                    let db = self.engine.db.clone();
                    self.screens.import_job = Some(if src.starts_with("http://") || src.starts_with("https://") {
                        Job::spawn(&self.rt, &ctx, async move { skills_store::import_url(&db, &src).await })
                    } else {
                        let path = expand_home(&src);
                        Job::blocking(&self.rt, &ctx, move || skills_store::import_path(&db, &path))
                    });
                }
            });
            ui.label(small("Imported skills start switched off. MyBot scans every file first; you decide after reading the review."));
        });
        ui.add_space(8.0);
        let q = self.screens.query.to_lowercase();
        for s in imported.iter().filter(|s| q.is_empty() || s.name.to_lowercase().contains(&q) || s.description.to_lowercase().contains(&q)) {
            card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&s.name).strong());
                    chip(ui, if s.enabled { "on" } else { "off" }, if s.enabled { GOOD } else { MUTED });
                    let worst = serde_json::from_str::<Vec<mybot_catalog::agent_skill::Finding>>(&s.findings).unwrap_or_default().iter().map(|f| f.severity).max();
                    match worst {
                        Some(Severity::High) => chip(ui, "high-risk findings", BAD),
                        Some(Severity::Medium) => chip(ui, "findings to read", WARN),
                        _ => {}
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(danger_button("Remove")).clicked() {
                            match skills_store::remove(&self.engine.db, &s.name) {
                                Ok(()) => self.toast(format!("Removed “{}”.", s.name)),
                                Err(e) => self.toast_err(e),
                            }
                        }
                        if ui.add(egui::Button::new(small("Export")).frame(false)).clicked() {
                            self.export_skill(&s.name);
                        }
                        if ui.add(ghost_button("Review")).clicked() {
                            match skills_store::review(&self.engine.db, &s.name) {
                                Ok(r) => self.screens.review = Some(r),
                                Err(e) => self.toast_err(e),
                            }
                        }
                        if s.enabled && ui.add(accent_button("Run")).clicked() {
                            self.screens.run = Some(RunForm { skill: s.name.clone(), inputs: vec![], values: BTreeMap::new(), bot: self.selected.clone(), extra: String::new() });
                        }
                    });
                });
                ui.label(RichText::new(&s.description).color(TEXT2).size(13.0));
                if let Some(o) = &s.origin {
                    ui.label(small(format!("from {o}")));
                }
            });
            ui.add_space(6.0);
        }
    }

    // ===================================================================== Routines

    pub(super) fn routines_view(&mut self, ui: &mut egui::Ui) {
        let routines = self.engine.db.routines().unwrap_or_default();
        let bots = self.bots.clone();
        page(ui, |ui| {
            ui.label(heading("Routines"));
            ui.label(muted("Run a skill on a schedule. Routines fire while MyBot is open (or under `mybot2 routines watch`)."));
            ui.add_space(10.0);
            card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("New routine").strong());
                let f = &mut self.screens.routine;
                egui::Grid::new("routine_form").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    ui.label(muted("Skill"));
                    ui.vertical(|ui| {
                        ui.add(input(&mut f.skill).hint_text("skill name").desired_width(360.0));
                        if !f.skill.trim().is_empty() && mybot_catalog::skill(f.skill.trim()).is_none() && self.engine.db.skill(f.skill.trim()).ok().flatten().is_none() {
                            ui.horizontal_wrapped(|ui| {
                                for s in mybot_catalog::search_skills(f.skill.trim(), 5) {
                                    if ui.add(egui::Button::new(small(&s.name)).fill(RAISED)).clicked() {
                                        f.skill = s.name.clone();
                                    }
                                }
                            });
                        }
                    });
                    ui.end_row();
                    ui.label(muted("Teammate"));
                    let current = f.bot.as_ref().and_then(|id| bots.iter().find(|b| &b.id == id)).map(|b| b.name.clone()).unwrap_or_else(|| "First teammate".into());
                    egui::ComboBox::from_id_salt("routine_bot").selected_text(current).width(240.0).show_ui(ui, |ui| {
                        for b in &bots {
                            ui.selectable_value(&mut f.bot, Some(b.id.clone()), &b.name);
                        }
                    });
                    ui.end_row();
                    ui.label(muted("When"));
                    ui.vertical(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            for (label, expr) in PRESETS {
                                if ui.selectable_label(f.schedule == expr, label).clicked() {
                                    f.schedule = expr.into();
                                }
                            }
                        });
                        ui.add(input(&mut f.schedule).hint_text("or a cron expression: min hour day month weekday").desired_width(360.0));
                        match Cron::parse(f.schedule.trim()) {
                            Ok(c) if !f.schedule.trim().is_empty() => {
                                let next = c.next_after(chrono::Local::now()).map(|t| t.format("%a %d %b %H:%M").to_string()).unwrap_or_default();
                                ui.label(small(format!("{} — next {next}", c.describe())));
                            }
                            Err(e) if !f.schedule.trim().is_empty() => {
                                ui.label(RichText::new(e).color(BAD).size(12.0));
                            }
                            _ => {}
                        }
                    });
                    ui.end_row();
                    ui.label(muted("Inputs"));
                    ui.add(input(&mut f.args).hint_text(r#"optional JSON, e.g. {"city": "Lisbon"}"#).desired_width(360.0));
                    ui.end_row();
                });
                if let Some(e) = &f.error {
                    ui.label(RichText::new(e).color(BAD));
                }
                if ui.add(accent_button("Add routine")).clicked() {
                    let skill = f.skill.trim().to_string();
                    let args = f.args.trim().to_string();
                    f.error = if skill.is_empty() {
                        Some("Pick a skill.".into())
                    } else if mybot_catalog::skill(&skill).is_none() && self.engine.db.skill(&skill).ok().flatten().is_none() {
                        Some(format!("No skill named “{skill}”."))
                    } else if Cron::parse(f.schedule.trim()).is_err() || f.schedule.trim().is_empty() {
                        Some("Choose when it runs.".into())
                    } else if !args.is_empty() && serde_json::from_str::<BTreeMap<String, String>>(&args).is_err() {
                        Some("Inputs must be a JSON object of text values.".into())
                    } else {
                        None
                    };
                    if f.error.is_none() {
                        match self.engine.db.add_routine(&skill, f.bot.as_deref(), Some(f.schedule.trim()), None, Some(args.as_str()).filter(|a| !a.is_empty())) {
                            Ok(_) => {
                                *f = RoutineForm::default();
                                self.toasts.push(super::Toast { text: "Routine added.".into(), bad: false, at: std::time::Instant::now() });
                            }
                            Err(e) => f.error = Some(e.to_string()),
                        }
                    }
                }
            });
            ui.add_space(12.0);
            if routines.is_empty() {
                ui.label(muted("No routines yet."));
            }
            for r in &routines {
                card().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&r.skill).strong());
                        let bot = r.bot_id.as_ref().and_then(|id| bots.iter().find(|b| &b.id == id)).map(|b| b.name.clone()).unwrap_or_else(|| "first teammate".into());
                        ui.label(muted(format!("by {bot}")));
                        chip(ui, if r.enabled { "on" } else { "paused" }, if r.enabled { GOOD } else { MUTED });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.add(danger_button("Delete")).clicked() {
                                let _ = self.engine.db.delete_routine(&r.id);
                            }
                            if ui.add(ghost_button(if r.enabled { "Pause" } else { "Resume" })).clicked() {
                                let _ = self.engine.db.set_routine_enabled(&r.id, !r.enabled);
                            }
                            if ui.add(accent_button("Run now")).clicked() {
                                match self.engine.fire_routine(&self.rt, r, "run now", self.sink.clone()) {
                                    Ok(t) => {
                                        let id = t.bot_id.clone();
                                        self.open_chat(&id);
                                    }
                                    Err(e) => self.toasts.push(super::Toast { text: e, bad: true, at: std::time::Instant::now() }),
                                }
                            }
                        });
                    });
                    let when = r.schedule.as_deref().map(|s| Cron::parse(s).map(|c| c.describe()).unwrap_or_else(|_| s.to_string())).or(r.trigger.clone()).unwrap_or_default();
                    let last = r.last_run_at.as_deref().and_then(crate::engine::parse_db_time).map(|t| format!(" · last ran {}", t.format("%a %d %b %H:%M"))).unwrap_or_default();
                    ui.label(small(format!("{when}{last}")));
                });
                ui.add_space(6.0);
            }
        });
    }

    // ===================================================================== Logins

    pub(super) fn logins_view(&mut self, ui: &mut egui::Ui) {
        let unlocked = self.engine.logins.is_unlocked();
        if self.screens.logins.is_none() && unlocked {
            self.screens.logins = Some(self.engine.logins.list().map_err(|e| e.to_string()));
        }
        let recent = self.engine.approvals.recent(12);
        page(ui, |ui| {
            ui.label(heading("Saved logins"));
            ui.label(muted("Teammates can sign in for you. Every time, you get a request to allow it — the password is typed into the page by MyBot and never shown to the AI."));
            ui.add_space(10.0);
            if !unlocked {
                card().show(ui, |ui| {
                    ui.label(muted("Unlock MyBot to see and add saved logins."));
                    if ui.add(accent_button("Unlock")).clicked() {
                        self.unlock = Some(super::Unlock { pass: String::new(), confirm: String::new(), remember: false, error: None, job: None });
                    }
                });
                return;
            }
            card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Add a login").strong());
                let f = &mut self.screens.login;
                egui::Grid::new("login_form").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    ui.label(muted("Site"));
                    ui.vertical(|ui| {
                        ui.add(input(&mut f.site).hint_text("gmail, github, https://intranet.example.com/login…").desired_width(380.0));
                        let q = f.site.trim().to_string();
                        let q = q.as_str();
                        if !q.is_empty() && !q.contains("://") {
                            ui.horizontal_wrapped(|ui| {
                                for s in mybot_catalog::search_sites(q, 6) {
                                    if ui.add(egui::Button::new(small(format!("{} · {}", s.name, s.login))).fill(RAISED)).clicked() {
                                        f.site = s.login.clone();
                                    }
                                }
                            });
                        }
                        let bound = if q.is_empty() { None } else { mybot_vault::logins::page_origin(&f.site).or_else(|| mybot_catalog::suggest_login_url(q).and_then(mybot_vault::logins::page_origin)) };
                        if let Some(o) = bound {
                            let two = mybot_catalog::site_for_origin(&o).is_some_and(|s| s.two_step);
                            ui.label(small(format!("Bound to {o}{}", if two { " · uses 2-step sign-in: you'll type the code yourself" } else { "" })));
                        }
                    });
                    ui.end_row();
                    ui.label(muted("Username"));
                    ui.add(input(&mut f.username).desired_width(380.0));
                    ui.end_row();
                    ui.label(muted("Password"));
                    ui.add(input(&mut f.password).password(true).desired_width(380.0));
                    ui.end_row();
                    ui.label(muted("Label"));
                    ui.add(input(&mut f.label).hint_text("optional, e.g. Work").desired_width(380.0));
                    ui.end_row();
                });
                if let Some(e) = &f.error {
                    ui.label(RichText::new(e).color(BAD));
                }
                if ui.add(accent_button("Save login")).clicked() {
                    let site = f.site.trim().to_string();
                    let url = if site.contains("://") { site.clone() } else { mybot_catalog::suggest_login_url(&site).map(String::from).unwrap_or(format!("https://{site}")) };
                    let label = f.label.trim().to_string();
                    let r = self.engine.logins.add(NewLogin { url: &url, username: f.username.trim(), password: &f.password, label: Some(label.as_str()).filter(|l| !l.is_empty()) });
                    match r {
                        Ok(s) => {
                            zeroize::Zeroize::zeroize(&mut f.password);
                            *f = LoginForm::default();
                            self.screens.logins = None;
                            self.toasts.push(super::Toast { text: format!("Saved {} for {}.", s.username, s.origin), bad: false, at: std::time::Instant::now() });
                        }
                        Err(e) => f.error = Some(e.to_string()),
                    }
                }
            });
            ui.add_space(12.0);
            let list = self.screens.logins.clone().unwrap_or(Ok(vec![]));
            match list {
                Err(e) => {
                    ui.label(RichText::new(e).color(BAD));
                }
                Ok(list) if list.is_empty() => {
                    ui.label(muted("No saved logins yet."));
                }
                Ok(list) => {
                    for l in &list {
                        card().show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                let site = mybot_catalog::site_for_origin(&l.origin).map(|s| s.name.clone()).unwrap_or_else(|| l.origin.clone());
                                initials_tile(ui, &site, 30.0);
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 0.0;
                                    ui.label(RichText::new(format!("{site}{}", l.label.as_deref().map(|x| format!(" · {x}")).unwrap_or_default())).strong());
                                    ui.label(small(format!("{} · {}", l.username, l.origin)));
                                });
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui.add(danger_button("Remove")).clicked() {
                                        match self.engine.logins.remove(&l.id) {
                                            Ok(_) => self.screens.logins = None,
                                            Err(e) => self.toasts.push(super::Toast { text: e.to_string(), bad: true, at: std::time::Instant::now() }),
                                        }
                                    }
                                    let mut always = l.always_allow;
                                    if ui.checkbox(&mut always, "Don't ask for this site").on_hover_text("Teammates may use this login without a request each time").changed() {
                                        let _ = self.engine.logins.set_always_allow(&l.id, always);
                                        self.screens.logins = None;
                                    }
                                });
                            });
                        });
                        ui.add_space(6.0);
                    }
                }
            }
            if !recent.is_empty() {
                ui.add_space(12.0);
                ui.label(RichText::new("Recent sign-in requests").strong());
                for r in &recent {
                    let (c, s) = match r.status.as_str() {
                        "allowed" => (GOOD, if r.decided_by.as_deref() == Some("rule") { "allowed (always)" } else { "allowed" }),
                        "denied" => (BAD, "denied"),
                        "expired" => (MUTED, "no answer"),
                        _ => (WARN, "waiting"),
                    };
                    ui.horizontal(|ui| {
                        chip(ui, s, c);
                        ui.label(small(format!("{} → {} as {} · {}", r.bot, r.origin, r.username, r.created_at)));
                    });
                }
            }
        });
    }

    // ===================================================================== Settings

    pub(super) fn settings_view(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if self.screens.docker.is_none() && self.screens.docker_job.is_none() {
            self.screens.docker_job = Some(Job::spawn(&self.rt, &ctx, container::status()));
        }
        if let Some(s) = poll(&mut self.screens.docker_job) {
            self.screens.docker = Some(s);
        }
        page(ui, |ui| {
            ui.label(heading("Settings"));
            ui.add_space(8.0);

            card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("AI providers").strong());
                ui.label(small("Keys are encrypted with your passphrase. A key in the environment (ANTHROPIC_API_KEY, OPENAI_API_KEY, GEMINI_API_KEY) wins."));
                ui.add_space(4.0);
                for p in PROVIDERS {
                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(6.0);
                    if p == "openai" {
                        self.gpt_settings(ui);
                        continue;
                    }
                    ui.horizontal(|ui| {
                        ui.label(strong(mybot_core::providers::family_label(p)));
                        match self.engine.key_source(p) {
                            Some("environment") => chip(ui, "key from the environment", GOOD),
                            Some(_) => chip(ui, "key saved", GOOD),
                            None => chip(ui, "no key", MUTED),
                        }
                    });
                    self.key_rows(ui, p);
                }
            });
            ui.add_space(10.0);

            card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Agent computers").strong());
                ui.label(small("Each conversation gets a Docker container; each teammate its own desktop and browser inside it."));
                match &self.screens.docker {
                    None => {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new().size(12.0));
                            ui.label(muted("Checking Docker…"));
                        });
                    }
                    Some(s) => {
                        ui.horizontal(|ui| {
                            chip(ui, if s.docker { "Docker running" } else { "Docker not found" }, if s.docker { GOOD } else { BAD });
                            chip(ui, if s.image { "desktop image built" } else { "image builds on first start" }, if s.image { GOOD } else { MUTED });
                            ui.label(small(format!("{} computer(s)", s.conversations.len())));
                        });
                        if !s.docker {
                            ui.label(small("Install Docker Desktop (or Docker Engine) and start it; MyBot uses it to give each teammate a safe computer."));
                        }
                    }
                }
                ui.horizontal(|ui| {
                    if ui.add(ghost_button("Check again")).clicked() {
                        self.screens.docker = None;
                    }
                    if ui.add(danger_button("Reset browser profiles")).on_hover_text("Signs every teammate out of every site").clicked() {
                        self.screens.confirm_reset = true;
                    }
                });
            });
            ui.add_space(10.0);

            card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Security").strong());
                let kc = mybot_vault::keychain::available();
                let remembered = kc && mybot_vault::keychain::recall().is_some();
                ui.horizontal(|ui| {
                    if !kc {
                        ui.label(small("No system keychain found; MyBot asks for your passphrase each time it starts."));
                    } else if remembered {
                        ui.label(small("Your passphrase is in the system keychain, so MyBot unlocks by itself."));
                        if ui.add(ghost_button("Forget it")).clicked() {
                            let _ = mybot_vault::keychain::forget();
                        }
                    } else {
                        ui.label(small("Remember the passphrase in the system keychain to unlock automatically."));
                        if let Some(p) = self.engine.passphrase()
                            && ui.add(ghost_button("Remember")).clicked()
                                && let Err(e) = mybot_vault::keychain::remember(&p) {
                                    self.toasts.push(super::Toast { text: e, bad: true, at: std::time::Instant::now() });
                                }
                    }
                });
                ui.label(small("Destructive shell commands are refused in every mode. CAPTCHAs, 2-step codes and card entry always stop for you."));
            });
            ui.add_space(10.0);

            card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("About").strong());
                self.update_controls(ui);
                ui.add_space(6.0);
                ui.label(small(format!(
                    "MyBot {} · {} actions · {} built-in skills · {} known sites",
                    env!("CARGO_PKG_VERSION"),
                    mybot_actions::all().len(),
                    mybot_catalog::skills().len(),
                    mybot_catalog::sites().len()
                )));
                ui.label(small(format!("Data: {}", mybot_vault::home().display())));
            });
        });
    }

    // ===================================================================== dialogs

    /// The API-key field (and optional custom endpoint) for one provider.
    fn key_rows(&mut self, ui: &mut egui::Ui, p: &str) {
        ui.horizontal(|ui| {
            let k = self.screens.keys.entry(p.to_string()).or_default();
            ui.add(input(k).password(true).hint_text("Paste an API key").desired_width(380.0));
            if ui.add_enabled(!k.trim().is_empty(), accent_button("Save key")).clicked() {
                match self.engine.set_key(p, k) {
                    Ok(()) => {
                        zeroize::Zeroize::zeroize(k);
                        self.toasts.push(super::Toast { text: format!("Saved the {} key.", mybot_core::providers::family_label(p)), bad: false, at: std::time::Instant::now() });
                    }
                    Err(e) => self.toasts.push(super::Toast { text: e, bad: true, at: std::time::Instant::now() }),
                }
            }
            if self.engine.key_source(p) == Some("saved")
                && ui.add(danger_button("Remove")).clicked()
                && let Err(e) = self.engine.remove_key(p)
            {
                self.toasts.push(super::Toast { text: e, bad: true, at: std::time::Instant::now() });
            }
        });
        ui.horizontal(|ui| {
            let current = self.engine.base_url(p).unwrap_or_default();
            let b = self.screens.bases.entry(p.to_string()).or_insert(current.clone());
            ui.label(small("Endpoint"));
            ui.add(input(b).hint_text("default").desired_width(320.0));
            if *b != current && ui.add(ghost_button("Apply")).clicked() {
                let _ = self.engine.db.set_setting(&format!("{p}.base_url"), b.trim());
            }
        });
    }

    /// GPT: sign in with your ChatGPT account, or use an API key.
    fn gpt_settings(&mut self, ui: &mut egui::Ui) {
        use crate::engine::{GptAuth, chatgpt};
        let ctx = ui.ctx().clone();
        let auth = self.engine.gpt_auth();

        ui.horizontal(|ui| {
            ui.label(strong("GPT"));
            match auth {
                GptAuth::ChatGpt => match &self.screens.gpt_probe {
                    Some(Ok(_)) => chip(ui, "signed in with ChatGPT", GOOD),
                    _ => chip(ui, "ChatGPT · not connected", WARN),
                },
                GptAuth::ApiKey => match self.engine.key_source("openai") {
                    Some("environment") => chip(ui, "key from the environment", GOOD),
                    Some(_) => chip(ui, "key saved", GOOD),
                    None => chip(ui, "no key", MUTED),
                },
            }
        });
        ui.add_space(2.0);

        let mut choice = match auth {
            GptAuth::ChatGpt => "chatgpt".to_string(),
            GptAuth::ApiKey => "api_key".to_string(),
        };
        let options = [("chatgpt".to_string(), "Sign in with ChatGPT".to_string()), ("api_key".to_string(), "API key".to_string())];
        super::chat::segmented(ui, &options, &mut choice);
        let picked = if choice == "chatgpt" { GptAuth::ChatGpt } else { GptAuth::ApiKey };
        if picked != auth {
            if let Err(e) = self.engine.set_gpt_auth(picked) {
                self.toasts.push(super::Toast { text: e, bad: true, at: std::time::Instant::now() });
            }
            self.screens.gpt_probe = None;
            self.screens.gpt_probe_at = None;
            self.screens.gpt_note = None;
        }
        ui.add_space(4.0);

        if picked == GptAuth::ApiKey {
            ui.label(small("Billed per use to your OpenAI account. A key in OPENAI_API_KEY wins over a saved one."));
            self.key_rows(ui, "openai");
            return;
        }

        // --- ChatGPT sign-in ---
        if let Some(r) = poll(&mut self.screens.gpt_signin) {
            self.screens.gpt_note = Some(match r {
                Ok(out) => (out, false),
                Err(e) => (e, true),
            });
            self.screens.gpt_probe_at = None; // look again now
        }
        if let Some(r) = poll(&mut self.screens.gpt_probe_job) {
            self.screens.gpt_probe = Some(r);
        }
        let due = self.screens.gpt_probe_at.is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(5));
        if due && self.screens.gpt_probe_job.is_none() && self.screens.gpt_signin.is_none() {
            self.screens.gpt_probe_at = Some(std::time::Instant::now());
            let engine = self.engine.clone();
            self.screens.gpt_probe_job = Some(Job::spawn(&self.rt, &ctx, async move {
                let p = engine.provider("openai").map_err(|e| e.to_string())?;
                p.list_models().await.map_err(|e| e.to_string())
            }));
        }
        ctx.request_repaint_after(std::time::Duration::from_secs(5));

        let busy = self.screens.gpt_signin.is_some();
        ui.horizontal(|ui| {
            if busy {
                ui.add(egui::Spinner::new().size(12.0).color(ACCENT));
                ui.label(RichText::new("Waiting for you to finish signing in in your browser…").color(TEXT2));
                return;
            }
            match &self.screens.gpt_probe {
                Some(Ok(models)) => {
                    ui.label(RichText::new("●").size(9.0).color(GOOD));
                    let shown: Vec<&str> = models.iter().map(String::as_str).filter(|m| !m.contains("image")).take(4).collect();
                    ui.label(RichText::new(format!("Connected · {}", shown.join(", "))).color(TEXT2));
                }
                Some(Err(_)) => {
                    ui.label(RichText::new("●").size(9.0).color(FAINT));
                    ui.label(RichText::new("Not connected — the sign-in helper isn't running.").color(TEXT2));
                }
                None => {
                    ui.add(egui::Spinner::new().size(12.0).color(MUTED));
                    ui.label(muted("Checking…"));
                }
            }
        });
        ui.horizontal(|ui| {
            let connected = matches!(self.screens.gpt_probe, Some(Ok(_)));
            if !connected && ui.add_enabled(!busy, accent_button("Sign in with ChatGPT")).clicked() {
                self.screens.gpt_note = None;
                self.screens.gpt_signin = Some(Job::blocking(&self.rt, &ctx, chatgpt::start));
            }
            if connected && ui.add(ghost_button("Disconnect")).on_hover_text("Stop the sign-in helper. Your ChatGPT login stays saved for next time.").clicked() {
                self.screens.gpt_signin = Some(Job::blocking(&self.rt, &ctx, chatgpt::stop));
            }
            if ui.add_enabled(!busy, ghost_button("Check again")).clicked() {
                self.screens.gpt_probe_at = None;
            }
        });
        if let Some((note, bad)) = &self.screens.gpt_note {
            egui::Frame::NONE.fill(egui::Color32::from_rgb(12, 12, 12)).corner_radius(8).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(egui::Label::new(RichText::new(note).monospace().size(11.5).color(if *bad { BAD } else { MUTED })).wrap());
            });
        }
        ui.add(egui::Label::new(small(format!(
            "Uses your ChatGPT plan — its limits, not API billing. MyBot runs {} (Apache-2.0), a helper on {} that signs you in through OpenAI in your browser and keeps the token in {}, apart from the Codex CLI's. MyBot never sees it. Needs Node.js.",
            chatgpt::PACKAGE,
            chatgpt::URL.trim_end_matches("/v1"),
            chatgpt::auth_file().display(),
        ))).wrap());
    }

    pub(super) fn screen_modals(&mut self, ctx: &egui::Context) {
        self.review_modal(ctx);
        self.run_modal(ctx);
        self.edit_modal(ctx);
        if self.screens.confirm_reset {
            let r = egui::Modal::new(Id::new("reset_profiles")).show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(heading("Reset browser profiles?"));
                ui.label(muted("Every teammate is signed out of every site. Saved logins are kept."));
                ui.horizontal(|ui| {
                    if ui.add(danger_button("Reset")).clicked() {
                        self.rt.spawn(container::reset_profiles(crate::engine::conversation()));
                        self.screens.confirm_reset = false;
                        self.toasts.push(super::Toast { text: "Browser profiles reset.".into(), bad: false, at: std::time::Instant::now() });
                    }
                    if ui.add(ghost_button("Cancel")).clicked() {
                        self.screens.confirm_reset = false;
                    }
                });
            });
            if r.should_close() {
                self.screens.confirm_reset = false;
            }
        }
    }

    fn review_modal(&mut self, ctx: &egui::Context) {
        let Some(rev) = self.screens.review.clone() else { return };
        let mut close = false;
        let r = egui::Modal::new(Id::new("skill_review")).show(ctx, |ui| {
            ui.set_width(640.0);
            ui.horizontal(|ui| {
                ui.label(heading(&rev.row.name));
                chip(ui, if rev.row.enabled { "on" } else { "off" }, if rev.row.enabled { GOOD } else { MUTED });
            });
            ui.label(RichText::new(&rev.doc.description).color(TEXT2));
            ui.horizontal_wrapped(|ui| {
                if let Some(l) = &rev.doc.license {
                    ui.label(small(format!("License: {l}")));
                }
                if let Some(c) = &rev.doc.compatibility {
                    ui.label(small(format!("· Needs: {c}")));
                }
                if let Some(t) = &rev.doc.allowed_tools {
                    ui.label(small(format!("· Tools: {t}")));
                }
            });
            if !rev.unchanged {
                ui.label(RichText::new("The files changed after import. Remove and import again to review the new version.").color(BAD));
            }
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                ui.label(RichText::new(format!("What MyBot found ({})", rev.findings.len())).strong());
                if rev.findings.is_empty() {
                    ui.label(small("Nothing worrying: no scripts that fetch and run code, no credential access, no prompt-injection phrasing."));
                }
                for f in &rev.findings {
                    let (c, s) = match f.severity {
                        Severity::High => (BAD, "high"),
                        Severity::Medium => (WARN, "medium"),
                        Severity::Info => (MUTED, "info"),
                    };
                    ui.horizontal_wrapped(|ui| {
                        chip(ui, s, c);
                        ui.label(RichText::new(format!("{}:{} — {}", f.file, f.line, f.message)).size(12.5).color(TEXT2));
                    });
                }
                ui.add_space(6.0);
                ui.label(RichText::new(format!("Files ({})", rev.files.len())).strong());
                for (p, n) in &rev.files {
                    ui.label(RichText::new(format!("{p}  ·  {n} bytes")).monospace().size(12.0).color(MUTED));
                }
                ui.add_space(6.0);
                ui.label(RichText::new("Instructions").strong());
                ui.label(RichText::new(rev.doc.body.chars().take(6000).collect::<String>()).size(12.5).color(TEXT2));
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if rev.row.enabled {
                    if ui.add(ghost_button("Turn off")).clicked() {
                        match skills_store::set_enabled(&self.engine.db, &rev.row.name, false) {
                            Ok(_) => close = true,
                            Err(e) => self.toasts.push(super::Toast { text: e, bad: true, at: std::time::Instant::now() }),
                        }
                    }
                } else if ui.add_enabled(rev.unchanged, accent_button("I've read it — turn on")).clicked() {
                    match skills_store::set_enabled(&self.engine.db, &rev.row.name, true) {
                        Ok(s) => {
                            close = true;
                            self.toasts.push(super::Toast { text: format!("“{}” is on. Teammates can use it now.", s.name), bad: false, at: std::time::Instant::now() });
                        }
                        Err(e) => self.toasts.push(super::Toast { text: e, bad: true, at: std::time::Instant::now() }),
                    }
                }
                if ui.add(ghost_button("Close")).clicked() {
                    close = true;
                }
            });
        });
        if close || r.should_close() {
            self.screens.review = None;
        }
    }

    fn run_modal(&mut self, ctx: &egui::Context) {
        let Some(f) = self.screens.run.as_mut() else { return };
        let bots = self.bots.clone();
        let mut go: Option<Bot> = None;
        let mut close = false;
        let r = egui::Modal::new(Id::new("run_skill")).show(ctx, |ui| {
            ui.set_width(480.0);
            ui.label(heading(format!("Run “{}”", f.skill)));
            if bots.is_empty() {
                ui.label(muted("Create a teammate first."));
            }
            egui::Grid::new("run_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label(muted("Teammate"));
                let current = f.bot.as_ref().and_then(|id| bots.iter().find(|b| &b.id == id)).map(|b| b.name.clone()).unwrap_or_else(|| "Choose…".into());
                egui::ComboBox::from_id_salt("run_bot").selected_text(current).width(260.0).show_ui(ui, |ui| {
                    for b in &bots {
                        ui.selectable_value(&mut f.bot, Some(b.id.clone()), &b.name);
                    }
                });
                ui.end_row();
                for name in &f.inputs {
                    ui.label(muted(name));
                    ui.add(input(f.values.entry(name.clone()).or_default()).desired_width(300.0));
                    ui.end_row();
                }
                ui.label(muted("Anything else"));
                ui.add(egui::TextEdit::multiline(&mut f.extra).desired_rows(2).desired_width(300.0).hint_text("optional"));
                ui.end_row();
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let bot = f.bot.as_ref().and_then(|id| bots.iter().find(|b| &b.id == id)).cloned();
                if ui.add_enabled(bot.is_some(), accent_button("Run")).clicked() {
                    go = bot;
                }
                if ui.add(ghost_button("Cancel")).clicked() {
                    close = true;
                }
            });
        });
        if let Some(bot) = go {
            let f = self.screens.run.take().unwrap();
            let values = f.values.into_iter().filter(|(_, v)| !v.trim().is_empty()).collect();
            if self.engine.is_running(&bot.id) {
                self.toast_err(format!("{} is busy. Stop it or wait.", bot.name));
                return;
            }
            super::chat::start_skill(self, bot, &f.skill, values, f.extra.trim().to_string());
        } else if close || r.should_close() {
            self.screens.run = None;
        }
    }

    fn edit_modal(&mut self, ctx: &egui::Context) {
        let Some(f) = self.screens.edit.as_mut() else { return };
        let mut save = false;
        let mut close = false;
        let r = egui::Modal::new(Id::new("edit_skill")).show(ctx, |ui| {
            ui.set_width(600.0);
            ui.label(heading(if f.id.is_empty() { "Write a skill" } else { "Edit skill" }));
            egui::Grid::new("edit_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label(muted("Name"));
                ui.add_enabled(f.id.is_empty(), input(&mut f.name).desired_width(460.0));
                ui.end_row();
                ui.label(muted("Description"));
                ui.add(input(&mut f.description).desired_width(460.0));
                ui.end_row();
                ui.label(muted("Instructions"));
                ui.add(egui::TextEdit::multiline(&mut f.instructions).desired_rows(10).desired_width(460.0).hint_text("Steps, in plain words. Use {{input}} for values asked each run."));
                ui.end_row();
                ui.label(muted("Safety rules"));
                ui.add(egui::TextEdit::multiline(&mut f.rules).desired_rows(3).desired_width(460.0).hint_text("One per line. always-confirm: placing the order"));
                ui.end_row();
            });
            ui.horizontal(|ui| {
                if ui.add(accent_button("Save")).clicked() {
                    save = true;
                }
                if ui.add(ghost_button("Cancel")).clicked() {
                    close = true;
                }
            });
        });
        if save {
            let f = self.screens.edit.take().unwrap();
            let r = if f.id.is_empty() {
                let name = mybot_catalog::agent_skill::slug(&f.name);
                if name.is_empty() || f.instructions.trim().is_empty() {
                    self.toast_err("A skill needs a name and instructions.");
                    self.screens.edit = Some(f);
                    return;
                }
                self.engine.db.add_skill(&name, "manual", "yours", f.description.trim(), f.instructions.trim(), f.rules.trim()).map(|_| ())
            } else {
                self.engine.db.update_skill(&f.id, f.instructions.trim(), f.rules.trim(), f.description.trim())
            };
            match r {
                Ok(()) => self.toast("Skill saved."),
                Err(e) => self.toast_err(e.to_string()),
            }
        } else if close || r.should_close() {
            self.screens.edit = None;
        }
    }
}

fn expand_home(p: &str) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => dirs_home().join(rest),
        None => PathBuf::from(p),
    }
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default()
}
