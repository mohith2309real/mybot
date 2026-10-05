//! Everything the window and the command line share: storage, secrets,
//! providers, starting runs, and the routine scheduler.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mybot_actions::toolbox::{ComputerSession, Progress, Toolbox};
use mybot_computer::boundary::Boundaries;
use mybot_computer::loginfill::LoginFiller;
use mybot_core::agent::{self, AgentEvent, RunCtx, RunOptions, RunResult};
use mybot_core::approvals::Approvals;
use mybot_core::cancel::Cancel;
use mybot_core::db::{Bot, Db, Task};
use mybot_core::handoff::Handoffs;
use mybot_core::model::Effort;
use mybot_core::policy::{self, Capability, Mode};
use mybot_core::providers::{self, Provider, ProviderConfig, ProviderError};
use mybot_vault::keys::{KeyStore, env_var_for};
use mybot_vault::logins::LoginStore;
use zeroize::Zeroizing;

pub const CONVERSATION: &str = "default";

pub struct RunHandle {
    pub task_id: String,
    pub cancel: Cancel,
}

pub struct Engine {
    pub db: Db,
    pub keys: KeyStore,
    pub logins: Arc<LoginStore>,
    pub approvals: Approvals,
    pub handoffs: Handoffs,
    pub filler: Arc<LoginFiller>,
    passphrase: Mutex<Option<Zeroizing<String>>>,
    /// One computer session per bot, kept so the live view and later runs reuse it.
    sessions: Mutex<HashMap<String, Arc<ComputerSession>>>,
    pub runs: Mutex<HashMap<String, RunHandle>>,
    pub computer_log: Arc<Mutex<Vec<String>>>,
}

#[derive(Clone)]
pub struct SkillRun {
    pub name: String,
    /// The rendered instructions + rules.
    pub text: String,
    /// `always-confirm` rule texts.
    pub confirm: Vec<String>,
}

impl Engine {
    pub fn open() -> anyhow::Result<Arc<Self>> {
        let db = Db::open_default()?;
        db.sweep_orphans()?;
        Ok(Self::with_db(db))
    }

    pub fn with_db(db: Db) -> Arc<Self> {
        let logins = Arc::new(LoginStore::default());
        let approvals = Approvals::new(db.clone(), logins.clone());
        let filler = Arc::new(LoginFiller::new(logins.clone(), approvals.clone()));
        Arc::new(Self {
            handoffs: Handoffs::new(Some(db.clone())),
            db,
            keys: KeyStore::default(),
            logins,
            approvals,
            filler,
            passphrase: Mutex::new(None),
            sessions: Mutex::new(HashMap::new()),
            runs: Mutex::new(HashMap::new()),
            computer_log: Arc::new(Mutex::new(Vec::new())),
        })
    }

    // --- secrets ---------------------------------------------------------------

    pub fn needs_unlock(&self) -> bool {
        self.passphrase.lock().unwrap().is_none() && (self.keys.exists() || self.logins.exists())
    }

    pub fn is_unlocked(&self) -> bool {
        self.passphrase.lock().unwrap().is_some()
    }

    /// Check a passphrase against whichever store exists, then hold it.
    pub fn unlock(&self, pass: &str) -> Result<(), String> {
        if pass.is_empty() {
            return Err("Enter your passphrase.".into());
        }
        if self.logins.exists() {
            self.logins.unlock(pass).map_err(|e| e.to_string())?;
        } else if self.keys.exists() {
            self.keys.read(pass).map_err(|e| e.to_string())?;
            self.logins.unlock(pass).map_err(|e| e.to_string())?;
        } else {
            // First run: create the key vault so the passphrase is fixed.
            self.keys.write(pass, &Default::default()).map_err(|e| e.to_string())?;
            self.logins.unlock(pass).map_err(|e| e.to_string())?;
        }
        *self.passphrase.lock().unwrap() = Some(Zeroizing::new(pass.to_string()));
        Ok(())
    }

    pub fn lock(&self) {
        *self.passphrase.lock().unwrap() = None;
        self.logins.lock();
    }

    /// Try the environment, then the OS keychain.
    pub fn auto_unlock(&self) -> bool {
        if self.is_unlocked() {
            return true;
        }
        for candidate in [mybot_vault::env_passphrase(), mybot_vault::keychain::recall()].into_iter().flatten() {
            if self.unlock(&candidate).is_ok() {
                return true;
            }
        }
        false
    }

    pub fn passphrase(&self) -> Option<String> {
        self.passphrase.lock().unwrap().as_ref().map(|p| p.to_string())
    }

    pub fn set_key(&self, provider: &str, key: &str) -> Result<(), String> {
        let pass = self.passphrase().ok_or("Unlock MyBot first.")?;
        self.keys.set(&pass, provider, key.trim()).map_err(|e| e.to_string())
    }

    pub fn remove_key(&self, provider: &str) -> Result<(), String> {
        let pass = self.passphrase().ok_or("Unlock MyBot first.")?;
        self.keys.remove(&pass, provider).map(|_| ()).map_err(|e| e.to_string())
    }

    /// Where a provider's key comes from: "env", "saved", or none.
    pub fn key_source(&self, provider: &str) -> Option<&'static str> {
        if std::env::var(env_var_for(provider)).is_ok_and(|v| !v.is_empty()) {
            return Some("environment");
        }
        let pass = self.passphrase()?;
        self.keys.read(&pass).ok()?.get(provider).map(|_| "saved")
    }

    // --- providers ---------------------------------------------------------------

    pub fn base_url(&self, provider: &str) -> Option<String> {
        self.db.setting(&format!("{provider}.base_url")).ok().flatten().filter(|s| !s.trim().is_empty())
    }

    pub fn provider(&self, name: &str) -> Result<Arc<dyn Provider>, ProviderError> {
        let base_url = self.base_url(name);
        let key = self.keys.resolve(name, self.passphrase().as_deref()).map(|k| k.to_string());
        // An OpenAI-compatible local proxy (ChatGPT sign-in) needs no key.
        let key = match (key, &base_url) {
            (Some(k), _) => k,
            (None, Some(u)) if u.starts_with("http://127.0.0.1") || u.starts_with("http://localhost") => String::new(),
            (None, _) => return Err(ProviderError::MissingKey { provider: name.into(), env_var: env_var_for(name).into() }),
        };
        Ok(Arc::from(providers::make(name, ProviderConfig { api_key: key, base_url })?))
    }

    // --- computer ------------------------------------------------------------------

    pub fn session(&self, bot: &str) -> Arc<ComputerSession> {
        let mut s = self.sessions.lock().unwrap();
        s.entry(bot.to_string())
            .or_insert_with(|| {
                let log = self.computer_log.clone();
                Arc::new(ComputerSession::new(CONVERSATION, bot, Arc::new(move |l: String| {
                    let mut v = log.lock().unwrap();
                    v.push(l);
                    let n = v.len();
                    if n > 400 {
                        v.drain(..n - 400);
                    }
                })))
            })
            .clone()
    }

    // --- runs ------------------------------------------------------------------------

    pub fn is_running(&self, bot_id: &str) -> bool {
        self.runs.lock().unwrap().contains_key(bot_id)
    }

    pub fn stop(&self, bot_id: &str) {
        if let Some(r) = self.runs.lock().unwrap().get(bot_id) {
            r.cancel.cancel();
        }
    }

    /// Start a run in the background. `on_event` receives everything the run
    /// reports; it is called from the runtime, not the UI thread.
    pub fn start_run(
        self: &Arc<Self>,
        rt: &tokio::runtime::Handle,
        bot: Bot,
        instruction: String,
        skill: Option<SkillRun>,
        on_event: Arc<dyn Fn(&str, AgentEvent) + Send + Sync>,
    ) -> Result<Task, String> {
        if self.is_running(&bot.id) {
            return Err(format!("{} is already working on something. Stop it first, or wait.", bot.name));
        }
        let task = self.db.create_task(&bot.id, &instruction).map_err(|e| e.to_string())?;
        let cancel = Cancel::new();
        self.runs.lock().unwrap().insert(bot.id.clone(), RunHandle { task_id: task.id.clone(), cancel: cancel.clone() });

        let me = self.clone();
        let task_id = task.id.clone();
        rt.spawn(async move {
            let result = me.run_inner(&bot, &task_id, &instruction, skill, cancel, on_event.clone()).await;
            if let Err(e) = result {
                let _ = me.db.append_log(&task_id, "error", &e);
                let _ = me.db.finish_task(&task_id, "failed", &e, 0, 0);
                on_event(&bot.id, AgentEvent::Finished { status: agent::RunStatus::Error, summary: e });
            }
            me.runs.lock().unwrap().remove(&bot.id);
        });
        Ok(task)
    }

    async fn run_inner(
        self: &Arc<Self>,
        bot: &Bot,
        task_id: &str,
        instruction: &str,
        skill: Option<SkillRun>,
        cancel: Cancel,
        on_event: Arc<dyn Fn(&str, AgentEvent) + Send + Sync>,
    ) -> Result<RunResult, String> {
        let provider = self.provider(&bot.provider).map_err(|e| e.to_string())?;
        let mut pol = policy::Context {
            mode: Mode::parse(&bot.mode).unwrap_or_default(),
            grants: policy::grants_from_instruction(instruction),
            always_confirm: Default::default(),
        };
        if let Some(sk) = &skill {
            pol.always_confirm = capabilities_in(&sk.confirm);
        }
        let boundaries = Arc::new(Boundaries::default());
        if let Some(sk) = &skill {
            boundaries.set_rules(sk.confirm.iter().filter(|r| r.split_whitespace().count() <= 6).map(|r| mybot_computer::boundary::ConfirmRule::phrase(r, r)).collect());
        }
        let bot_id = bot.id.clone();
        let progress_sink = on_event.clone();
        let progress_bot = bot_id.clone();
        let progress: Progress = Arc::new(move |t: String| progress_sink(&progress_bot, AgentEvent::Assistant(format!("▸ {t}"))));
        let toolbox = Toolbox {
            session: self.session(&bot.name),
            boundaries,
            filler: self.filler.clone(),
            policy: pol,
            db: Some(self.db.clone()),
            progress,
        };
        let system = toolbox.system_prompt(bot.system.as_deref(), skill.as_ref().map(|s| s.text.as_str()));
        let mut prior = self.db.tasks_for(&bot.id, 11).map_err(|e| e.to_string())?;
        prior.retain(|t| t.id != task_id);
        let history = agent::history_from_tasks(&prior[prior.len().saturating_sub(10)..]);

        let opts = RunOptions {
            provider,
            model: Some(bot.model.clone()),
            effort: Effort::parse(&bot.effort),
            system,
            history,
            instruction: instruction.to_string(),
            max_iterations: 40,
            pause_timeout: Duration::from_secs(15 * 60),
            ctx: RunCtx::new(bot.name.clone(), Some(task_id.to_string()), instruction, cancel),
            db: Some(self.db.clone()),
        };
        let sink = move |e: AgentEvent| on_event(&bot_id, e);
        Ok(agent::run_task(opts, &toolbox, &self.handoffs, &sink).await)
    }

    // --- skills -------------------------------------------------------------------------

    /// Find any skill by name and render it with inputs.
    pub fn skill_run(&self, name: &str, args: &BTreeMap<String, String>) -> Result<SkillRun, String> {
        if let Some(s) = mybot_catalog::skill(name) {
            let (confirm, _) = mybot_catalog::safety_rules(&s.safety);
            return Ok(SkillRun { name: s.name.clone(), text: mybot_catalog::render_skill(s, args), confirm });
        }
        let row = self.db.skill(name).map_err(|e| e.to_string())?.ok_or_else(|| format!("No skill named \"{name}\"."))?;
        if row.source == "imported" {
            let (row, pkg) = mybot_actions::skills_store::package_for_use(&self.db, name)?;
            let files: Vec<(String, usize)> = pkg.files.iter().map(|f| (f.path.clone(), f.bytes.len())).collect();
            let mut text = mybot_actions::skills_store::render(&row, &files);
            if !args.is_empty() {
                text.push_str("\n\nInputs from the human:");
                for (k, v) in args {
                    text.push_str(&format!("\n- {k}: {v}"));
                }
            }
            return Ok(SkillRun { name: row.name, text, confirm: vec![] });
        }
        let rules: Vec<String> = row.safety_rules.lines().map(String::from).collect();
        let (confirm, other) = mybot_catalog::safety_rules(&rules);
        let mut text = format!("Skill: {}\n\n{}", row.name, mybot_catalog::render_instructions(&row.instructions, args));
        if !confirm.is_empty() || !other.is_empty() {
            text.push_str("\n\nSafety rules for this skill:");
            for r in &other {
                text.push_str(&format!("\n- {r}"));
            }
            for r in &confirm {
                text.push_str(&format!("\n- Always stop and get a human to confirm before: {r}"));
            }
        }
        Ok(SkillRun { name: row.name, text, confirm })
    }

    // --- routines -------------------------------------------------------------------------

    /// Fire scheduled routines that came due. Returns how many started.
    pub fn tick_routines(self: &Arc<Self>, rt: &tokio::runtime::Handle, on_event: Arc<dyn Fn(&str, AgentEvent) + Send + Sync>) -> usize {
        let now = chrono::Local::now();
        let mut started = 0;
        for r in self.db.routines().unwrap_or_default().into_iter().filter(|r| r.enabled) {
            let Some(expr) = &r.schedule else { continue };
            let Ok(cron) = mybot_core::cron::Cron::parse(expr) else { continue };
            let last = r.last_run_at.as_deref().or(Some(r.created_at.as_str())).and_then(parse_db_time).unwrap_or(now);
            if cron.due_between(last, now) && self.fire_routine(rt, &r, "schedule", on_event.clone()).is_ok() {
                started += 1;
            }
        }
        started
    }

    pub fn fire_routine(
        self: &Arc<Self>,
        rt: &tokio::runtime::Handle,
        r: &mybot_core::db::Routine,
        reason: &str,
        on_event: Arc<dyn Fn(&str, AgentEvent) + Send + Sync>,
    ) -> Result<Task, String> {
        let bot = match &r.bot_id {
            Some(id) => self.db.bot(id).ok().flatten(),
            None => self.db.bots().ok().and_then(|b| b.into_iter().next()),
        }
        .ok_or("This routine has no bot to run it.")?;
        let args: BTreeMap<String, String> = r
            .args
            .as_deref()
            .and_then(|a| serde_json::from_str(a).ok())
            .unwrap_or_default();
        let skill = self.skill_run(&r.skill, &args)?;
        let instruction = format!("Run the skill \"{}\" ({reason}).", skill.name);
        let result = self.start_run(rt, bot, instruction, Some(skill), on_event);
        let _ = self.db.record_routine_run(&r.id, result.as_ref().ok().map(|t| t.id.as_str()), reason);
        result
    }
}

/// SQLite "YYYY-MM-DD HH:MM:SS" (UTC) → local time.
pub fn parse_db_time(s: &str) -> Option<chrono::DateTime<chrono::Local>> {
    let naive = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok()?;
    Some(chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(naive, chrono::Utc).with_timezone(&chrono::Local))
}

/// Which policy capabilities a skill's always-confirm rule texts are about.
pub fn capabilities_in(rules: &[String]) -> std::collections::BTreeSet<Capability> {
    let mut out = std::collections::BTreeSet::new();
    for r in rules {
        let t = r.to_lowercase();
        let has = |ws: &[&str]| ws.iter().any(|w| t.contains(w));
        if has(&["send", "email", "message", "reply", "invitation", "unsubscribe", "proposal", "rsvp"]) {
            out.insert(Capability::SendMessage);
        }
        if has(&["publish", "post", "schedul", "comment", "react"]) {
            out.insert(Capability::Publish);
        }
        if has(&["buy", "purchase", "checkout", "order", "book", "reserv", "enrol", "register", "domain", "cart"]) {
            out.insert(Capability::Purchase);
        }
        if has(&["pay", "transfer", "money", "trade", "donat", "redeem", "invoice"]) {
            out.insert(Capability::TransferFunds);
        }
        if has(&["delet", "remov", "cancel"]) {
            out.insert(Capability::DeleteData);
        }
        if has(&["permission", "access", "setting", "profile"]) {
            out.insert(Capability::ChangePermissions);
        }
        if has(&["deploy", "production", "merg", "tag", "release", "push"]) {
            out.insert(Capability::ProductionChange);
        }
        if has(&["terms", "contract", "accept", "sign"]) {
            out.insert(Capability::AcceptTerms);
        }
    }
    out
}

/// Models offered for a new bot when the live list cannot be fetched.
pub fn fallback_models(provider: &str) -> Vec<&'static str> {
    match provider {
        "anthropic" => vec!["claude-opus-5-5", "claude-sonnet-5-5", "claude-fable-5-1", "claude-haiku-4-5"],
        "openai" => vec!["gpt-5.6", "gpt-5.6-terra", "gpt-5.6-luna"],
        "gemini" => vec!["gemini-3.6-flash", "gemini-3-flash-preview", "gemini-2.5-flash"],
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirm_rules_map_to_capabilities() {
        let caps = capabilities_in(&["sending the email".into(), "checkout or payment".into()]);
        assert!(caps.contains(&Capability::SendMessage) && caps.contains(&Capability::Purchase) && caps.contains(&Capability::TransferFunds));
        assert!(capabilities_in(&[]).is_empty());
    }

    #[test]
    fn db_time() {
        assert!(parse_db_time("2026-10-05 13:00:00").is_some());
        assert!(parse_db_time("nonsense").is_none());
    }
}
