//! The tool surface a bot gets: core page tools, bash, fill_login,
//! request_human, task_done, and `actions_search` / `action_run` over the
//! action library.
//!
//! Every path to the browser goes through the same boundary checks, every
//! shell command through the destructive-command check, and every action
//! with consequences through the run's permission policy — there is no side
//! door in the library that skips what the core tools enforce.
//!
//! The computer is started lazily: a run that only needs local actions (maths,
//! text, the skill library) works with Docker switched off.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use mybot_computer::boundary::{Action as BAction, Boundaries};
use mybot_computer::browser::Browser;
use mybot_computer::container::{self, Computer, Desktop};
use mybot_computer::loginfill::{FillInput, FillResult, LoginFiller};
use mybot_core::agent::{Pause, RunCtx, ToolHost, ToolOutcome};
use mybot_core::db::Db;
use mybot_core::model::ToolDef;
use mybot_core::policy::{self, Decision};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::{Handler, browser_actions, schema};

const MAX_RESULT: usize = 12_000;

pub type Progress = Arc<dyn Fn(String) + Send + Sync>;

/// One bot's computer: its conversation's container and its own desktop.
pub struct ComputerSession {
    pub conversation: String,
    pub bot: String,
    state: Mutex<Option<(Computer, Desktop, Arc<Browser>)>>,
    on_line: Progress,
    /// Never start a computer (tests): anything that needs one gets an error.
    offline: bool,
}

impl ComputerSession {
    pub fn new(conversation: impl Into<String>, bot: impl Into<String>, on_line: Progress) -> Self {
        Self { conversation: conversation.into(), bot: bot.into(), state: Mutex::new(None), on_line, offline: false }
    }

    /// A session that refuses to start Docker, so tests never create
    /// containers on the machine running them.
    pub fn offline(bot: impl Into<String>) -> Self {
        Self { offline: true, ..Self::new("offline", bot, Arc::new(|_| {})) }
    }

    /// Start the container and this bot's desktop if needed; reconnect the
    /// browser if its connection dropped.
    pub async fn ensure(&self) -> Result<(Computer, Desktop, Arc<Browser>), String> {
        if self.offline {
            return Err("No computer in this session.".into());
        }
        let mut st = self.state.lock().await;
        if let Some((c, d, b)) = st.as_ref()
            && b.is_alive() {
                return Ok((c.clone(), d.clone(), b.clone()));
            }
        let on_line = self.on_line.clone();
        let computer = container::ensure(&self.conversation, &move |l| on_line(l)).await.map_err(|e| e.to_string())?;
        let desktop = container::desktop(&computer, &self.bot).await.map_err(|e| e.to_string())?;
        let port = desktop.cdp_port.ok_or("no browser on this desktop")?;
        let browser = Arc::new(Browser::connect(port).await?);
        *st = Some((computer.clone(), desktop.clone(), browser.clone()));
        Ok((computer, desktop, browser))
    }

    pub async fn current(&self) -> Option<(Computer, Desktop)> {
        self.state.lock().await.as_ref().map(|(c, d, _)| (c.clone(), d.clone()))
    }

    pub async fn browser(&self) -> Result<Arc<Browser>, String> {
        Ok(self.ensure().await?.2)
    }

    pub async fn exec(&self, command: &str, timeout: Duration) -> Result<container::ExecResult, String> {
        self.ensure().await?;
        Ok(container::exec(&self.conversation, command, None, timeout).await)
    }
}

pub struct Toolbox {
    pub session: Arc<ComputerSession>,
    pub boundaries: Arc<Boundaries>,
    pub filler: Arc<LoginFiller>,
    pub policy: policy::Context,
    pub db: Option<Db>,
    pub progress: Progress,
}

fn truncate(s: String) -> String {
    if s.chars().count() <= MAX_RESULT {
        s
    } else {
        format!("{}\n…[truncated — {} characters in all]", s.chars().take(MAX_RESULT).collect::<String>(), s.chars().count())
    }
}

fn int(v: &Value, k: &str) -> Option<u32> {
    match v.get(k) {
        Some(Value::Number(n)) => n.as_u64().map(|x| x as u32),
        Some(Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    }
}

impl Toolbox {
    pub fn core_tools() -> Vec<ToolDef> {
        let t = |name: &str, description: &str, params: Value| ToolDef { name: name.into(), description: description.into(), parameters: params };
        vec![
            t("page_read", "Read the current page: URL, title, visible text, and a numbered list of interactive elements. Call this before clicking or typing, and again after anything that changes the page — element refs are only valid until the next page_read.", schema(&[])),
            t("page_navigate", "Navigate the browser to a URL. https:// is assumed if omitted.", schema(&[("url", "string", "URL to open", true)])),
            t("page_click", "Click an element by its ref from the most recent page_read.", schema(&[("ref", "integer", "Element ref", true)])),
            t("page_type", "Type text into a field by its ref. Set submit=true to press Enter afterwards. Never for passwords, codes or card numbers.", schema(&[("ref", "integer", "Element ref", true), ("text", "string", "Text to type", true), ("submit", "boolean", "Press Enter after typing", false)])),
            t("fill_login", "Sign in with a login the human saved for this exact site. Give the ref of the password field, and of the username/email field if it is on the same page (two-step sign-ins: call it on the username page with only username_ref, then on the password page with only password_ref). The human approves, then MyBot fills the fields itself — you never see the password and must never type one. If nothing is saved for this site, the human is handed the browser to sign in. One-time codes and 2FA are always the human's.",
                schema(&[("password_ref", "integer", "Ref of the password field", false), ("username_ref", "integer", "Ref of the username or email field", false), ("account", "string", "Which saved username, only if several are saved for this site", false), ("submit", "boolean", "Press Enter after filling (default true)", false)])),
            t("page_scroll", "Scroll the page up or down.", json!({"type": "object", "properties": {"direction": {"type": "string", "enum": ["up", "down"]}}, "required": ["direction"]})),
            t("bash", "Run a bash command inside the computer, working directory /workspace. For file work, downloads and data processing.", schema(&[("command", "string", "Command to run", true)])),
            t("actions_search", "Find more actions than these core tools: 200+ for files, the web, text, data, maths, dates, hashing, tabs, screenshots, notes and the skill library. Describe what you need in a few words.", schema(&[("query", "string", "What you need, e.g. 'csv to json' or 'screenshot'", true), ("category", "string", "Optional category", false)])),
            t("action_run", "Run an action found with actions_search. args must match its parameters.", json!({"type": "object", "properties": {"name": {"type": "string", "description": "Action name"}, "args": {"type": "object", "description": "Arguments"}}, "required": ["name"]})),
            t("request_human", "Hand control to the human and wait. Use when a step needs a person — a sign-in you cannot complete, an identity check, anything you are unsure is safe to do unattended. They see your reason, do that one step in a live view of this browser, and hand back. Prefer this over giving up.", schema(&[("reason", "string", "What you need the human to do, in one line", true)])),
            t("task_done", "Call when the task is complete, with a summary of what you did and what you found.", schema(&[("summary", "string", "What you accomplished", true)])),
        ]
    }

    /// The system prompt for a run: base rules, this run's authority, the
    /// bot's persona, and the skill being run if any.
    pub fn system_prompt(&self, persona: Option<&str>, skill: Option<&str>) -> String {
        let mut p = format!("{}\n\n{}", mybot_core::agent::BASE_SYSTEM, policy::render_prompt(&self.policy));
        if let Some(per) = persona.filter(|x| !x.trim().is_empty()) {
            p.push_str(&format!("\n\nYour standing brief from the human:\n{}", per.trim()));
        }
        if let Some(index) = self.db.as_ref().and_then(crate::skills_store::prompt_index) {
            p.push_str(&format!("\n\n{index}"));
        }
        if let Some(sk) = skill {
            p.push_str(&format!("\n\nYou are running this skill:\n{sk}"));
        }
        p
    }

    async fn page_pause(&self, b: &Browser) -> Option<Pause> {
        self.boundaries.detect(b, BAction::Navigate, None, None).await
    }

    async fn element_pause(&self, b: &Browser, r: u32, action: BAction) -> Result<Option<Pause>, String> {
        let snap = b.snapshot(6000).await?;
        Ok(self.boundaries.detect(b, action, Some(r), Some(&snap)).await)
    }

    async fn run_shell(&self, command: &str, timeout: Duration) -> ToolOutcome {
        if let Some(d) = policy::check_destructive(command) {
            return ToolOutcome::err(format!(
                "Refused: that is a {} (matched `{}`). No permission mode allows this, including bypass. If it is genuinely needed, ask the human to run it themselves and explain why.",
                d.label, d.matched
            ));
        }
        match self.session.exec(command, timeout).await {
            Ok(r) => {
                let body = [r.stdout.trim(), r.stderr.trim()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n");
                ToolOutcome { content: truncate(format!("exit {}\n{}", r.code, if body.is_empty() { "(no output)" } else { &body })), is_error: r.code != 0, ..Default::default() }
            }
            Err(e) => ToolOutcome::err(e),
        }
    }

    async fn core(&self, name: &str, input: &Value, ctx: &RunCtx) -> ToolOutcome {
        match name {
            "page_read" => match self.session.browser().await {
                Ok(b) => match b.snapshot(6000).await {
                    Ok(s) => ToolOutcome::ok(truncate(s.render())),
                    Err(e) => ToolOutcome::err(e),
                },
                Err(e) => ToolOutcome::err(e),
            },
            "page_navigate" => {
                let b = match self.session.browser().await {
                    Ok(b) => b,
                    Err(e) => return ToolOutcome::err(e),
                };
                match b.navigate(input["url"].as_str().unwrap_or("")).await {
                    Ok((url, title)) => {
                        let opened = format!("Opened {url}\nTitle: {title}\n");
                        // A checkout or a CAPTCHA can appear with no action at all,
                        // so landing on one is itself the boundary.
                        if let Some(p) = self.page_pause(&b).await {
                            return ToolOutcome::paused(opened, p);
                        }
                        ToolOutcome::ok(format!("{opened}Call page_read to see the page."))
                    }
                    Err(e) => ToolOutcome::err(e),
                }
            }
            "page_click" => {
                let Some(r) = int(input, "ref") else { return ToolOutcome::err("page_click needs a ref") };
                let b = match self.session.browser().await {
                    Ok(b) => b,
                    Err(e) => return ToolOutcome::err(e),
                };
                match self.element_pause(&b, r, BAction::Click).await {
                    Ok(Some(p)) => return ToolOutcome::paused("", p),
                    Err(e) => return ToolOutcome::err(e),
                    Ok(None) => {}
                }
                match b.click(r).await {
                    Ok(t) => ToolOutcome::ok(t),
                    Err(e) => ToolOutcome::err(e),
                }
            }
            "page_type" => {
                let Some(r) = int(input, "ref") else { return ToolOutcome::err("page_type needs a ref") };
                let text = input["text"].as_str().unwrap_or("");
                let b = match self.session.browser().await {
                    Ok(b) => b,
                    Err(e) => return ToolOutcome::err(e),
                };
                match self.element_pause(&b, r, BAction::Type).await {
                    Ok(Some(p)) if p.kind == "password" && self.filler.logins.exists() => ToolOutcome::err(format!(
                        "Ref {r} is a password field. Never type passwords. Call fill_login with password_ref={r} (plus username_ref if the username field is on this page)."
                    )),
                    Ok(Some(p)) => ToolOutcome::paused("", p),
                    Err(e) => ToolOutcome::err(e),
                    Ok(None) => match b.type_text(r, text, input["submit"].as_bool().unwrap_or(false)).await {
                        Ok(t) => ToolOutcome::ok(t),
                        Err(e) => ToolOutcome::err(e),
                    },
                }
            }
            "fill_login" => {
                let b = match self.session.browser().await {
                    Ok(b) => b,
                    Err(e) => return ToolOutcome::err(e),
                };
                let fill = FillInput {
                    password_ref: int(input, "password_ref"),
                    username_ref: int(input, "username_ref"),
                    account: input["account"].as_str().map(String::from),
                    // Submitting by default keeps a filled password in the page
                    // for as short a time as possible.
                    submit: input["submit"].as_bool().unwrap_or(true),
                };
                match self.filler.fill(b.as_ref(), fill, &ctx.bot, ctx.task_id.as_deref(), &ctx.cancel).await {
                    FillResult::Filled(t) | FillResult::Denied(t) => ToolOutcome::ok(t),
                    FillResult::Error(t) => ToolOutcome::err(t),
                    FillResult::Handoff { text, reason } => {
                        ToolOutcome::paused(format!("{text}\n"), Pause::new("password", reason, b.url().await, false))
                    }
                }
            }
            "page_scroll" => match self.session.browser().await {
                Ok(b) => match b.scroll(input["direction"] != "up", 600.0).await {
                    Ok(t) => ToolOutcome::ok(t),
                    Err(e) => ToolOutcome::err(e),
                },
                Err(e) => ToolOutcome::err(e),
            },
            "bash" => self.run_shell(input["command"].as_str().unwrap_or(""), Duration::from_secs(120)).await,
            "actions_search" => {
                let hits = crate::search(input["query"].as_str().unwrap_or(""), input["category"].as_str(), 12);
                if hits.is_empty() {
                    let cats: Vec<String> = crate::categories().into_iter().map(|(c, n)| format!("{c} ({n})")).collect();
                    return ToolOutcome::ok(format!("Nothing matched. Categories: {}", cats.join(", ")));
                }
                ToolOutcome::ok(format!(
                    "{}\n\nRun one with action_run {{\"name\": …, \"args\": {{…}}}}.",
                    hits.iter().map(|a| format!("- {} [{}]", a.signature(), a.kind().label())).collect::<Vec<_>>().join("\n")
                ))
            }
            "action_run" => {
                let name = input["name"].as_str().unwrap_or("");
                let args = input.get("args").cloned().unwrap_or(json!({}));
                self.action(name, &args, ctx).await
            }
            "request_human" => {
                let reason = input["reason"].as_str().map(str::trim).filter(|s| !s.is_empty()).unwrap_or("The bot asked for a human, without saying why.");
                let url = match self.session.current().await {
                    Some(_) => self.session.browser().await.ok().map(|b| async move { b.url().await }),
                    None => None,
                };
                let url = match url {
                    Some(f) => f.await,
                    None => String::new(),
                };
                ToolOutcome::paused("", Pause::new("always_confirm", reason, url, false))
            }
            "task_done" => {
                ToolOutcome { content: "Task marked complete.".into(), done: Some(input["summary"].as_str().unwrap_or("").to_string()), ..Default::default() }
            }
            other => ToolOutcome::err(format!("Unknown tool: {other}")),
        }
    }

    async fn action(&self, name: &str, args: &Value, ctx: &RunCtx) -> ToolOutcome {
        let Some(action) = crate::get(name) else {
            let close: Vec<&str> = crate::search(name, None, 5).iter().map(|a| a.name).collect();
            return ToolOutcome::err(format!("No action named \"{name}\". Closest: {}. Use actions_search.", close.join(", ")));
        };

        // Consequential actions answer to the run's policy.
        if let Some(cap) = action.capability {
            let verdict = policy::decide(Some(cap), None, &self.policy);
            match verdict.decision {
                Decision::Refuse => return ToolOutcome::err(verdict.reason),
                Decision::Confirm if !ctx.approved => {
                    return ToolOutcome::paused(
                        "",
                        Pause::confirm(format!("{} wants to run {name} {}. {}", ctx.bot, args, verdict.reason)),
                    );
                }
                _ => {}
            }
        }

        match &action.handler {
            Handler::Local(f) => match f(args) {
                Ok(t) => ToolOutcome::ok(truncate(t)),
                Err(e) => ToolOutcome::err(format!("{name}: {e}")),
            },
            Handler::Shell(f) => match f(args) {
                Ok(cmd) => self.run_shell(&cmd, Duration::from_secs(300)).await,
                Err(e) => ToolOutcome::err(format!("{name}: {e}")),
            },
            Handler::Browser => self.browser_action(name, args).await,
            Handler::Meta => self.meta(name, args, ctx).await,
        }
    }

    async fn browser_action(&self, name: &str, args: &Value) -> ToolOutcome {
        let b = match self.session.browser().await {
            Ok(b) => b,
            Err(e) => return ToolOutcome::err(e),
        };
        // Anything aimed at an element is checked like a click on it first.
        if let Some(r) = int(args, "ref") {
            match self.element_pause(&b, r, BAction::Click).await {
                Ok(Some(p)) => return ToolOutcome::paused("", p),
                Err(e) => return ToolOutcome::err(e),
                Ok(None) => {}
            }
        }
        let out = match browser_actions::run(name, &b, args).await {
            Ok(o) => o,
            Err(e) => return ToolOutcome::err(format!("{name}: {e}")),
        };
        let text = match out {
            browser_actions::Output::Text(t) => t,
            browser_actions::Output::File { default_name, bytes, path } => {
                let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
                let path = path.unwrap_or_else(|| {
                    let (dir, file) = default_name.rsplit_once('/').unwrap_or(("", &default_name));
                    let (stem, ext) = file.rsplit_once('.').unwrap_or((file, ""));
                    format!("/workspace/{dir}/{stem}-{stamp}.{ext}")
                });
                let path = if path.starts_with('/') { path } else { format!("/workspace/{path}") };
                let r = container::write_file(&self.session.conversation, &path, &bytes).await;
                if r.code != 0 {
                    return ToolOutcome::err(format!("Could not save {path}: {}", r.stderr.trim()));
                }
                format!("Saved {path} ({} bytes)", bytes.len())
            }
        };
        // Navigation-like actions can land on a boundary page.
        if matches!(name, "page_back" | "page_forward" | "page_reload" | "tab_new" | "tab_switch" | "page_go_to_ref_link")
            && let Some(p) = self.page_pause(&b).await {
                return ToolOutcome::paused(format!("{text}\n"), p);
            }
        ToolOutcome::ok(truncate(text))
    }

    async fn meta(&self, name: &str, args: &Value, ctx: &RunCtx) -> ToolOutcome {
        let s = |k: &str| args[k].as_str().unwrap_or("").to_string();
        let py = |code: String| format!("python3 - <<'PYEOF'\n{code}\nPYEOF");
        let pq = |x: &str| serde_json::to_string(x).unwrap_or_else(|_| "\"\"".into());
        match name {
            "memory_save" => {
                self.run_shell(&py(format!("import json,os\np='/workspace/.mybot/memory.json'\nos.makedirs(os.path.dirname(p),exist_ok=True)\nm=json.load(open(p)) if os.path.exists(p) else {{}}\nm[{}]={}\njson.dump(m,open(p,'w'),indent=1)\nprint('remembered')", pq(&s("key")), pq(&s("value")))), Duration::from_secs(30)).await
            }
            "memory_recall" => {
                self.run_shell(&py(format!("import json,os\np='/workspace/.mybot/memory.json'\nm=json.load(open(p)) if os.path.exists(p) else {{}}\nk={}\nprint(m.get(k,'(nothing remembered under that key)') if k else ('\\n'.join(f'{{a}}: {{b}}' for a,b in m.items()) or '(nothing remembered yet)'))", pq(&s("key")))), Duration::from_secs(30)).await
            }
            "memory_forget" => {
                self.run_shell(&py(format!("import json,os\np='/workspace/.mybot/memory.json'\nm=json.load(open(p)) if os.path.exists(p) else {{}}\nprint('forgotten' if m.pop({},None) is not None else 'nothing to forget')\njson.dump(m,open(p,'w'),indent=1)", pq(&s("key")))), Duration::from_secs(30)).await
            }
            "note_add" => {
                self.run_shell(&format!("printf -- '- %s  %s\\n' \"$(date '+%Y-%m-%d %H:%M')\" {} >> /workspace/notes.md && tail -n 3 /workspace/notes.md", crate::sh_quote(&s("text"))), Duration::from_secs(30)).await
            }
            "notes_read" => self.run_shell("cat /workspace/notes.md 2>/dev/null || echo '(no notes yet)'", Duration::from_secs(30)).await,
            "todo_add" => {
                self.run_shell(&format!("printf -- '- [ ] %s\\n' {} >> /workspace/todo.md && grep -c '' /workspace/todo.md", crate::sh_quote(&s("item"))), Duration::from_secs(30)).await
            }
            "todo_list" => self.run_shell("[ -f /workspace/todo.md ] && nl -ba /workspace/todo.md || echo '(no to-dos yet)'", Duration::from_secs(30)).await,
            "todo_done" => {
                let n = int(args, "number").unwrap_or(0);
                self.run_shell(&format!("sed -i '{n}s/^- \\[ \\]/- [x]/' /workspace/todo.md && sed -n '{n}p' /workspace/todo.md"), Duration::from_secs(30)).await
            }
            "skill_search" => {
                let q = s("query");
                let mut lines: Vec<String> = mybot_catalog::search_skills(&q, 10).iter().map(|k| format!("- {} [{}] — {}", k.name, k.category, k.description)).collect();
                if let Some(db) = &self.db {
                    // Imported skills appear only once a human enabled them and
                    // only while their files match what was reviewed.
                    let usable: Vec<String> = crate::skills_store::usable(db).into_iter().map(|r| r.id).collect();
                    for k in db.skills().unwrap_or_default().into_iter().filter(|k| k.source != "imported" || usable.contains(&k.id)) {
                        let hay = format!("{} {} {}", k.name, k.description, k.category).to_lowercase();
                        if q.to_lowercase().split_whitespace().any(|w| hay.contains(w)) {
                            lines.push(format!("- {} [{}, {}] — {}", k.name, k.category, k.source, k.description));
                        }
                    }
                }
                ToolOutcome::ok(if lines.is_empty() { "No skills matched.".into() } else { lines.join("\n") })
            }
            "skill_show" => {
                let n = s("name");
                if let Some(k) = mybot_catalog::skill(&n) {
                    return ToolOutcome::ok(mybot_catalog::render_skill(k, &Default::default()));
                }
                let Some(db) = self.db.as_ref() else { return ToolOutcome::err(format!("No skill named \"{n}\".")) };
                match db.skill(&n).ok().flatten() {
                    Some(k) if k.source == "imported" => match crate::skills_store::package_for_use(db, &n) {
                        Ok((row, pkg)) => {
                            // Its files go into this computer, where its scripts may run.
                            let mut note = String::new();
                            if pkg.files.len() > 1 {
                                match self.session.ensure().await {
                                    Ok(_) => {
                                        for f in &pkg.files {
                                            let r = container::write_file(&self.session.conversation, &format!("/workspace/.skills/{}/{}", row.name, f.path), &f.bytes).await;
                                            if r.code != 0 {
                                                note = format!("\n\n(Could not copy {} into the computer: {})", f.path, r.stderr.trim());
                                                break;
                                            }
                                        }
                                    }
                                    Err(e) => note = format!("\n\n(The computer is not running, so the skill's files are not available: {e})"),
                                }
                            }
                            let files: Vec<(String, usize)> = pkg.files.iter().map(|f| (f.path.clone(), f.bytes.len())).collect();
                            ToolOutcome::ok(truncate(format!("{}{note}", crate::skills_store::render(&row, &files))))
                        }
                        Err(e) => ToolOutcome::err(e),
                    },
                    Some(k) => ToolOutcome::ok(format!("Skill: {}\n\n{}\n\nSafety rules:\n{}", k.name, k.instructions, k.safety_rules)),
                    None => ToolOutcome::err(format!("No skill named \"{n}\". Use skill_search.")),
                }
            }
            "site_info" => match mybot_catalog::search_sites(&s("site"), 1).first() {
                Some(site) => ToolOutcome::ok(format!(
                    "{} ({})\nsite: {}\nsign in at: {}{}{}",
                    site.name,
                    site.category,
                    site.url,
                    site.login,
                    if site.two_step { "\nusername and password are on separate pages" } else { "" },
                    site.note.as_ref().map(|n| format!("\nnote: {n}")).unwrap_or_default()
                )),
                None => ToolOutcome::err("No profile for that site."),
            },
            "site_search" => {
                let q = s("query").to_lowercase();
                let hits: Vec<String> = mybot_catalog::sites()
                    .iter()
                    .filter(|x| x.category.to_lowercase().contains(&q) || x.name.to_lowercase().contains(&q) || x.key.contains(&q))
                    .take(40)
                    .map(|x| format!("- {} — {}", x.name, x.url))
                    .collect();
                ToolOutcome::ok(if hits.is_empty() { "No sites matched.".into() } else { hits.join("\n") })
            }
            "wait_seconds" => {
                let secs = int(args, "seconds").unwrap_or(5).min(120) as u64;
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(secs)) => ToolOutcome::ok(format!("waited {secs}s")),
                    _ = ctx.cancel.cancelled() => ToolOutcome::err("Stopped by you."),
                }
            }
            "task_info" => ToolOutcome::ok(format!("bot: {}\ntask: {}\ninstruction: {}", ctx.bot, ctx.task_id.as_deref().unwrap_or("(none)"), ctx.instruction)),
            "progress_note" => {
                (self.progress)(s("text"));
                ToolOutcome::ok("posted")
            }
            "list_desktops" => match self.session.ensure().await {
                Ok((c, d, _)) => ToolOutcome::ok(format!("desktops: {}\nyou are on: {} (display :{})", container::list_desktops(&c).await, d.bot, d.display)),
                Err(e) => ToolOutcome::err(e),
            },
            other => ToolOutcome::err(format!("no meta action {other}")),
        }
    }
}

#[async_trait]
impl ToolHost for Toolbox {
    fn tools(&self) -> Vec<ToolDef> {
        Self::core_tools()
    }

    async fn run(&self, name: &str, input: &Value, ctx: &RunCtx) -> ToolOutcome {
        self.core(name, input, ctx).await
    }

    fn acknowledge(&self, pause: &Pause) {
        self.boundaries.acknowledge(pause);
    }

    fn reset(&self) {
        self.boundaries.reset();
        self.filler.clear_grants();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mybot_core::approvals::Approvals;
    use mybot_core::cancel::Cancel;
    use mybot_vault::logins::LoginStore;

    fn toolbox(policy: policy::Context) -> (tempfile::TempDir, Toolbox) {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(LoginStore::at(dir.path().join("logins.enc")));
        let db = Db::in_memory().unwrap();
        let filler = Arc::new(LoginFiller::new(store.clone(), Approvals::new(db.clone(), store)));
        let session = Arc::new(ComputerSession::offline("scout"));
        (dir, Toolbox { session, boundaries: Arc::new(Boundaries::default()), filler, policy, db: Some(db), progress: Arc::new(|_| {}) })
    }

    fn ctx() -> RunCtx {
        RunCtx::new("scout", None, "do things", Cancel::new())
    }

    #[tokio::test]
    async fn local_actions_and_library_work_without_a_computer() {
        let (_d, tb) = toolbox(policy::Context::default());
        let r = tb.run("action_run", &json!({"name": "calculate", "args": {"expression": "6*7"}}), &ctx()).await;
        assert_eq!(r.content, "42");
        let r = tb.run("actions_search", &json!({"query": "csv to json"}), &ctx()).await;
        assert!(r.content.contains("csv_to_json"));
        let r = tb.run("action_run", &json!({"name": "skill_show", "args": {"name": "Inbox triage"}}), &ctx()).await;
        assert!(r.content.contains("Do not delete"));
        let r = tb.run("action_run", &json!({"name": "site_info", "args": {"site": "github"}}), &ctx()).await;
        assert!(r.content.contains("https://github.com/login"));
        let r = tb.run("action_run", &json!({"name": "nope"}), &ctx()).await;
        assert!(r.is_error);
        let r = tb.run("task_done", &json!({"summary": "ok"}), &ctx()).await;
        assert_eq!(r.done.as_deref(), Some("ok"));
    }

    #[tokio::test]
    async fn destructive_and_consequential_actions_are_stopped() {
        let (_d, tb) = toolbox(policy::Context::default());
        let r = tb.run("bash", &json!({"command": "sudo rm -rf --no-preserve-root /"}), &ctx()).await;
        assert!(r.is_error && r.content.contains("Refused"));
        // Deleting was not asked for: it needs a human's OK first.
        let r = tb.run("action_run", &json!({"name": "file_delete", "args": {"path": "old.txt"}}), &ctx()).await;
        let p = r.pause.expect("must pause for confirmation");
        assert!(p.confirm && p.reason.contains("file_delete"));
        // A root path is refused outright, approved or not.
        let mut approved = ctx();
        approved.approved = true;
        let r = tb.run("action_run", &json!({"name": "file_delete", "args": {"path": "/etc"}}), &approved).await;
        assert!(r.is_error && r.content.contains("refused"));
        // If the instruction granted deletion, no pause: it goes on to run (and,
        // with no computer in tests, says so instead of starting Docker).
        let (_d2, tb2) = toolbox(policy::Context { grants: policy::grants_from_instruction("delete the old reports"), ..Default::default() });
        let r = tb2.run("action_run", &json!({"name": "file_delete", "args": {"path": "old.txt"}}), &ctx()).await;
        assert!(r.pause.is_none());
        assert!(r.is_error && r.content.contains("No computer"), "{}", r.content);
    }

    #[test]
    fn core_tool_count_stays_small() {
        assert!(Toolbox::core_tools().len() <= 12, "the core surface should stay small; put the rest in actions");
    }
}
