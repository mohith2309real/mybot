//! `mybot2 <command>`: everything the window does, from a terminal.

use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};
use mybot_actions::{skills_store, teach};
use mybot_core::agent::{AgentEvent, RunStatus};
use mybot_core::approvals::ApprovalEvent;
use mybot_core::cron::Cron;
use mybot_core::db::Bot;
use mybot_core::providers::PROVIDERS;
use mybot_vault::logins::NewLogin;

use crate::engine::{CONVERSATION, Engine};

#[derive(Parser)]
#[command(name = "mybot2", version, about = "MyBot 2.0 — AI teammates with their own computers. Run with no command to open the app.")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Cmd>,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Open the desktop app (the default).
    App,
    /// API keys for Claude, OpenAI and Gemini.
    Keys {
        #[command(subcommand)]
        cmd: KeysCmd,
    },
    /// Your teammates.
    Bots {
        #[command(subcommand)]
        cmd: BotsCmd,
    },
    /// Give a teammate a task and watch it work.
    Run {
        bot: String,
        instruction: Vec<String>,
        /// Run a skill (built-in, yours, or imported).
        #[arg(long)]
        skill: Option<String>,
        /// Skill inputs, `name=value`.
        #[arg(long = "arg", value_name = "NAME=VALUE")]
        args: Vec<String>,
    },
    /// Recent tasks.
    Tasks {
        #[arg(long)]
        bot: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Everything a task did.
    Log { task: String },
    /// Hand a paused task (in another window) back to its bot.
    Resume { task: String },
    /// Stop a task.
    Cancel { task: String },
    /// Saved logins (passwords never shown to the AI).
    Logins {
        #[command(subcommand)]
        cmd: LoginsCmd,
    },
    /// Built-in, your own, and imported (Agent Skills / SKILL.md) skills.
    Skills {
        #[command(subcommand)]
        cmd: SkillsCmd,
    },
    /// Teach a task by doing it once in a teammate's browser.
    Teach { bot: String, label: Vec<String> },
    /// Skills on a schedule.
    Routines {
        #[command(subcommand)]
        cmd: RoutinesCmd,
    },
    /// Search the 200+ actions teammates can take.
    Actions { query: Vec<String> },
    /// Search the 300 known sites and their sign-in pages.
    Sites { query: Vec<String> },
    /// The models a provider offers right now.
    Models { provider: String },
    /// The agent computers (Docker).
    Computer {
        #[command(subcommand)]
        cmd: ComputerCmd,
    },
}

#[derive(Subcommand)]
pub enum KeysCmd {
    /// Save a key (asked for without echo).
    Set { provider: String },
    List,
    Rm { provider: String },
}

#[derive(Subcommand)]
pub enum BotsCmd {
    List,
    Add {
        name: String,
        #[arg(long, default_value = "anthropic")]
        provider: String,
        #[arg(long)]
        model: Option<String>,
        /// ask | auto | bypass
        #[arg(long, default_value = "ask")]
        mode: String,
        /// low | medium | high | xhigh | max
        #[arg(long, default_value = "high")]
        effort: String,
        #[arg(long)]
        persona: Option<String>,
    },
    Rm { name: String },
}

#[derive(Subcommand)]
pub enum LoginsCmd {
    /// Save a login; the password is asked for without echo.
    Add {
        /// The sign-in page (or a known site: gmail, github, …).
        site: String,
        username: String,
        #[arg(long)]
        label: Option<String>,
    },
    List,
    Rm { selector: String },
    /// Let teammates use a login without asking each time (on|off).
    Always { selector: String, state: String },
    /// Sign-in requests waiting for you.
    Pending,
    Allow {
        id: String,
        #[arg(long)]
        always: bool,
    },
    Deny { id: String },
    History,
}

#[derive(Subcommand)]
pub enum SkillsCmd {
    List {
        #[arg(long)]
        query: Option<String>,
        #[arg(long)]
        category: Option<String>,
    },
    Show { name: String },
    /// Import an Agent Skills folder, .zip, or GitHub folder URL. Starts switched off.
    Import { source: String },
    /// What the import scan found, and the files.
    Review { name: String },
    Enable { name: String },
    Disable { name: String },
    Rm { name: String },
    /// Write any skill out as an Agent Skills folder (SKILL.md).
    Export {
        name: String,
        #[arg(long)]
        to: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum RoutinesCmd {
    List,
    Add {
        skill: String,
        /// Cron: "min hour day month weekday", e.g. "0 9 * * 1-5".
        #[arg(long)]
        cron: String,
        #[arg(long)]
        bot: Option<String>,
        /// JSON object of skill inputs.
        #[arg(long)]
        args: Option<String>,
    },
    Rm { id: String },
    On { id: String },
    Off { id: String },
    Run { id: String },
    /// Keep running, firing routines when they come due.
    Watch,
}

#[derive(Subcommand)]
pub enum ComputerCmd {
    Status,
    /// Sign every teammate out of every site.
    ResetProfiles,
    /// Stop the computer (`--remove` deletes it).
    Down {
        #[arg(long)]
        remove: bool,
    },
}

fn prompt(q: &str) -> String {
    print!("{q}");
    let _ = std::io::stdout().flush();
    let mut s = String::new();
    let _ = std::io::stdin().lock().read_line(&mut s);
    s.trim().to_string()
}

/// Ask for a secret without echo; read a line from stdin when it's piped.
fn secret(q: &str) -> anyhow::Result<String> {
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() {
        return Ok(rpassword::prompt_password(q)?);
    }
    let mut s = String::new();
    std::io::stdin().lock().read_line(&mut s)?;
    Ok(s.trim_end_matches(['\r', '\n']).to_string())
}

/// Unlock from the environment, the keychain, or by asking.
fn unlock(engine: &Engine) -> anyhow::Result<()> {
    if engine.auto_unlock() {
        return Ok(());
    }
    let first = !engine.keys.exists() && !engine.logins.exists();
    let pass = secret(if first { "Choose a passphrase (encrypts keys and logins): " } else { "MyBot passphrase: " })?;
    if first && std::io::IsTerminal::is_terminal(&std::io::stdin()) && secret("Again: ")? != pass {
        anyhow::bail!("the two passphrases differ");
    }
    engine.unlock(&pass).map_err(anyhow::Error::msg)
}

fn find_bot(engine: &Engine, name: &str) -> anyhow::Result<Bot> {
    engine.db.bot_by_name(name)?.or(engine.db.bot(name)?).ok_or_else(|| anyhow::anyhow!("no teammate called \"{name}\" — see `mybot2 bots list`"))
}

pub fn main(cli: Cli) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let engine = Engine::open()?;
    let Some(cmd) = cli.command else { unreachable!("no command opens the app") };
    match cmd {
        Cmd::App => unreachable!(),
        Cmd::Keys { cmd } => {
            unlock(&engine)?;
            match cmd {
                KeysCmd::Set { provider } => {
                    anyhow::ensure!(PROVIDERS.contains(&provider.as_str()), "provider is one of: {}", PROVIDERS.join(", "));
                    let key = secret(&format!("{provider} API key: "))?;
                    engine.set_key(&provider, &key).map_err(anyhow::Error::msg)?;
                    println!("Saved.");
                }
                KeysCmd::List => {
                    for p in PROVIDERS {
                        println!("{:<10} {}", p, engine.key_source(p).unwrap_or("—"));
                    }
                }
                KeysCmd::Rm { provider } => {
                    engine.remove_key(&provider).map_err(anyhow::Error::msg)?;
                    println!("Removed.");
                }
            }
        }
        Cmd::Bots { cmd } => match cmd {
            BotsCmd::List => {
                for b in engine.db.bots()? {
                    println!("{:<18} {:<10} {:<22} {:<7} {}", b.name, b.provider, b.model, b.mode, b.effort);
                }
            }
            BotsCmd::Add { name, provider, model, mode, effort, persona } => {
                anyhow::ensure!(PROVIDERS.contains(&provider.as_str()), "provider is one of: {}", PROVIDERS.join(", "));
                anyhow::ensure!(mybot_core::policy::Mode::parse(&mode).is_some(), "mode is ask, auto or bypass");
                anyhow::ensure!(mybot_core::model::Effort::parse(&effort).is_some(), "effort is low, medium, high, xhigh or max");
                let model = model.unwrap_or_else(|| mybot_core::providers::default_model(&provider).into());
                let mut b = engine.db.create_bot(&name, &provider, &model, persona.as_deref())?;
                b.mode = mode;
                b.effort = effort;
                engine.db.update_bot(&b)?;
                println!("Created {} ({} · {}).", b.name, b.provider, b.model);
            }
            BotsCmd::Rm { name } => {
                let b = find_bot(&engine, &name)?;
                engine.db.delete_bot(&b.id)?;
                println!("Deleted {}.", b.name);
            }
        },
        Cmd::Run { bot, instruction, skill, args } => {
            unlock(&engine).ok();
            let bot = find_bot(&engine, &bot)?;
            let args = parse_args(&args)?;
            let skill = skill.map(|s| engine.skill_run(&s, &args)).transpose().map_err(anyhow::Error::msg)?;
            let text = instruction.join(" ");
            let instruction = match (&skill, text.trim().is_empty()) {
                (Some(s), true) => format!("Run the skill \"{}\".", s.name),
                (Some(s), false) => format!("Run the skill \"{}\". {text}", s.name),
                (None, true) => anyhow::bail!("say what to do"),
                (None, false) => text,
            };
            rt.block_on(run_in_terminal(engine, bot, instruction, skill))?;
        }
        Cmd::Tasks { bot, limit } => {
            let tasks = match bot {
                Some(b) => engine.db.tasks_for(&find_bot(&engine, &b)?.id, limit)?,
                None => engine.db.recent_tasks(limit)?,
            };
            for t in tasks {
                println!("{}  {:<9} {}  {}", &t.id[..8], t.status, t.created_at, mybot_core::agent::preview(&t.instruction));
            }
        }
        Cmd::Log { task } => {
            let t = find_task(&engine, &task)?;
            println!("{} — {}\n", t.status, t.instruction);
            for e in engine.db.log(&t.id)? {
                println!("[{}] {:<11} {}", e.at, e.kind, e.text);
            }
        }
        Cmd::Resume { task } => {
            let t = find_task(&engine, &task)?;
            anyhow::ensure!(t.status == "paused", "that task isn't paused (it's {})", t.status);
            engine.db.set_task_status(&t.id, "running")?;
            println!("Handed back.");
        }
        Cmd::Cancel { task } => {
            let t = find_task(&engine, &task)?;
            engine.db.set_task_status(&t.id, "cancelled")?;
            println!("Cancelled.");
        }
        Cmd::Logins { cmd } => logins(&engine, cmd)?,
        Cmd::Skills { cmd } => skills(&engine, &rt, cmd)?,
        Cmd::Teach { bot, label } => {
            unlock(&engine).ok();
            let bot = find_bot(&engine, &bot)?;
            rt.block_on(teach_in_terminal(engine, bot, label.join(" ")))?;
        }
        Cmd::Routines { cmd } => routines(&engine, &rt, cmd)?,
        Cmd::Actions { query } => {
            let q = query.join(" ");
            let list = if q.is_empty() { mybot_actions::all().iter().collect() } else { mybot_actions::search(&q, None, 40) };
            for a in &list {
                println!("{:<12} {}", a.category, a.signature());
            }
            println!("\n{} of {} actions", list.len(), mybot_actions::all().len());
        }
        Cmd::Sites { query } => {
            let q = query.join(" ");
            let list = if q.is_empty() { mybot_catalog::sites().iter().collect() } else { mybot_catalog::search_sites(&q, 40) };
            for s in &list {
                println!("{:<14} {:<26} {}{}", s.category, s.name, s.login, if s.two_step { "  (2-step)" } else { "" });
            }
            println!("\n{} of {} sites", list.len(), mybot_catalog::sites().len());
        }
        Cmd::Models { provider } => {
            unlock(&engine).ok();
            let p = engine.provider(&provider)?;
            for m in rt.block_on(p.list_models())? {
                println!("{m}");
            }
        }
        Cmd::Computer { cmd } => match cmd {
            ComputerCmd::Status => {
                let s = rt.block_on(mybot_computer::container::status());
                println!("docker: {}\nimage:  {}", if s.docker { "running" } else { "not found" }, if s.image { mybot_computer::container::IMAGE } else { "not built yet" });
                for (c, st) in s.conversations {
                    println!("computer {c}: {st:?}");
                }
            }
            ComputerCmd::ResetProfiles => {
                rt.block_on(mybot_computer::container::reset_profiles(CONVERSATION));
                println!("Every teammate is signed out of every site.");
            }
            ComputerCmd::Down { remove } => {
                rt.block_on(mybot_computer::container::down(CONVERSATION, remove));
                println!("Stopped.");
            }
        },
    }
    Ok(())
}

fn parse_args(args: &[String]) -> anyhow::Result<BTreeMap<String, String>> {
    args.iter()
        .map(|a| a.split_once('=').map(|(k, v)| (k.trim().to_string(), v.to_string())).ok_or_else(|| anyhow::anyhow!("--arg takes name=value, got \"{a}\"")))
        .collect()
}

fn find_task(engine: &Engine, prefix: &str) -> anyhow::Result<mybot_core::db::Task> {
    if let Some(t) = engine.db.task(prefix)? {
        return Ok(t);
    }
    let hits: Vec<_> = engine.db.recent_tasks(500)?.into_iter().filter(|t| t.id.starts_with(prefix)).collect();
    match hits.len() {
        1 => Ok(hits.into_iter().next().unwrap()),
        0 => anyhow::bail!("no task {prefix}"),
        _ => anyhow::bail!("{prefix} matches several tasks; give more of the id"),
    }
}

/// Stream a run; answer sign-in requests and pauses at the prompt.
async fn run_in_terminal(engine: Arc<Engine>, bot: Bot, instruction: String, skill: Option<crate::engine::SkillRun>) -> anyhow::Result<()> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
    let sink: crate::gui::Sink = Arc::new(move |_: &str, e: AgentEvent| {
        let _ = tx.send(e);
    });
    let rt = tokio::runtime::Handle::current();
    let task = engine.start_run(&rt, bot.clone(), instruction, skill, sink).map_err(anyhow::Error::msg)?;
    let mut approvals = engine.approvals.subscribe();
    println!("{} is on it (task {}). Ctrl+C stops it.\n", bot.name, &task.id[..8]);
    let stop = engine.clone();
    let bot_id = bot.id.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            stop.stop(&bot_id);
        }
    });
    let mut in_text = false;
    loop {
        tokio::select! {
            Some(e) = rx.recv() => match e {
                AgentEvent::TextDelta(t) => {
                    in_text = true;
                    print!("{t}");
                    let _ = std::io::stdout().flush();
                }
                AgentEvent::ThinkingDelta(_) | AgentEvent::Thinking(_) | AgentEvent::Started { .. } => {}
                AgentEvent::Assistant(a) => {
                    if !in_text {
                        println!("{a}");
                    } else {
                        println!();
                    }
                    in_text = false;
                }
                AgentEvent::ToolCall { name, input } => {
                    if in_text {
                        println!();
                        in_text = false;
                    }
                    println!("  → {} {}", crate::gui::state::verb(&name), crate::gui::state::compact(&input));
                }
                AgentEvent::ToolResult { is_error: true, content, .. } => println!("    ✕ {}", mybot_core::agent::preview(&content)),
                AgentEvent::ToolResult { .. } => {}
                AgentEvent::Paused(p) => {
                    println!("\n⏸  {}\n   {}", if p.confirm { "Needs your OK:" } else { "Needs you:" }, p.reason);
                    let (eng, task_id) = (engine.clone(), task.id.clone());
                    let confirm = p.confirm;
                    tokio::task::spawn_blocking(move || {
                        let a = if confirm { prompt("   Approve? [y/N] ") } else { prompt("   Do it in the live view (open the app), then press Enter — or type s to skip: ") };
                        let skipped = if confirm { !a.eq_ignore_ascii_case("y") } else { a.eq_ignore_ascii_case("s") };
                        eng.handoffs.resume(&task_id, skipped);
                    });
                }
                AgentEvent::Resumed { skipped } => println!("   {}\n", if skipped { "Skipped." } else { "Continuing." }),
                AgentEvent::Finished { status, summary } => {
                    if in_text {
                        println!();
                    }
                    let mark = match status { RunStatus::Done => "✓", RunStatus::Cancelled => "■", _ => "✕" };
                    println!("\n{mark} {summary}");
                    break;
                }
            },
            Ok(ApprovalEvent::Requested(r)) = approvals.recv() => {
                println!("\n🔐 {} wants to sign in to {} as {}.", r.bot, r.origin, r.username);
                let eng = engine.clone();
                tokio::task::spawn_blocking(move || {
                    let a = prompt("   [a]llow once · [A]lways for this site · [d]eny: ");
                    let res = match a.as_str() {
                        "a" | "y" => eng.approvals.allow(&r.id, false),
                        "A" => eng.approvals.allow(&r.id, true),
                        _ => eng.approvals.deny(&r.id),
                    };
                    if let Err(e) = res {
                        println!("   {e}");
                    }
                });
            }
        }
    }
    Ok(())
}

async fn teach_in_terminal(engine: Arc<Engine>, bot: Bot, label: String) -> anyhow::Result<()> {
    let session = engine.session(&bot.name);
    println!("Starting {}'s computer…", bot.name);
    let (_, desktop, browser) = session.ensure().await.map_err(anyhow::Error::msg)?;
    let rec = teach::Recorder::start(browser, &label).await.map_err(anyhow::Error::msg)?;
    println!("Recording. Do the task in the bot's browser: {}\nPasswords and codes are blanked inside the page. Press Enter when you're done (10 minutes at most).", desktop.web_url());
    let enter = tokio::task::spawn_blocking(|| prompt(""));
    tokio::select! {
        _ = enter => {}
        _ = tokio::time::sleep(teach::MAX_RECORDING) => println!("Ten minutes — stopping."),
    }
    let stored = rec.stop(&engine.db).await.map_err(anyhow::Error::msg)?;
    let actions = teach::actions_of(&stored);
    println!("\nRecorded {} actions:\n{}", actions.len(), teach::format_actions(&actions, 60));
    anyhow::ensure!(!actions.is_empty(), "nothing was recorded");
    println!("\nDrafting a skill…");
    let provider = engine.provider(&bot.provider)?;
    let draft = teach::compile(provider.as_ref(), Some(bot.model.clone()), &stored.name, &actions).await.map_err(anyhow::Error::msg)?;
    println!("\n{}\n", teach::render_draft(&draft));
    let a = tokio::task::spawn_blocking(|| prompt("Save this skill? [Y/n] ")).await?;
    if a.eq_ignore_ascii_case("n") {
        println!("Not saved. The recording is kept ({}).", &stored.id[..8]);
    } else {
        let row = teach::save(&engine.db, Some(&stored.id), &draft).map_err(anyhow::Error::msg)?;
        println!("Saved as “{}”. Run it with: mybot2 run {} --skill {}", row.name, bot.name, row.name);
    }
    Ok(())
}

fn logins(engine: &Engine, cmd: LoginsCmd) -> anyhow::Result<()> {
    match cmd {
        LoginsCmd::Pending => {
            for r in engine.approvals.pending() {
                println!("{}  {} → {} as {}  ({})", &r.id[..8], r.bot, r.origin, r.username, r.created_at);
            }
            return Ok(());
        }
        LoginsCmd::History => {
            for r in engine.approvals.recent(30) {
                println!("{:<8} {}  {} → {} as {}", r.status, r.created_at, r.bot, r.origin, r.username);
            }
            return Ok(());
        }
        LoginsCmd::Allow { id, always } => {
            let id = full_request_id(engine, &id)?;
            engine.approvals.allow(&id, always).map_err(anyhow::Error::msg)?;
            println!("Allowed.");
            return Ok(());
        }
        LoginsCmd::Deny { id } => {
            let id = full_request_id(engine, &id)?;
            engine.approvals.deny(&id).map_err(anyhow::Error::msg)?;
            println!("Denied.");
            return Ok(());
        }
        _ => {}
    }
    unlock(engine)?;
    match cmd {
        LoginsCmd::Add { site, username, label } => {
            let url = if site.contains("://") { site.clone() } else { mybot_catalog::suggest_login_url(&site).map(String::from).unwrap_or(format!("https://{site}")) };
            let password = zeroize::Zeroizing::new(secret(&format!("Password for {username} at {url}: "))?);
            let s = engine.logins.add(NewLogin { url: &url, username: &username, password: &password, label: label.as_deref() })?;
            println!("Saved {} for {}. Teammates will ask you each time they use it.", s.username, s.origin);
        }
        LoginsCmd::List => {
            for l in engine.logins.list()? {
                println!("{}  {:<36} {:<28} {}{}", &l.id[..8], l.origin, l.username, l.label.unwrap_or_default(), if l.always_allow { "  (doesn't ask)" } else { "" });
            }
        }
        LoginsCmd::Rm { selector } => {
            let gone = engine.logins.remove(&selector)?;
            println!("Removed {}.", gone.len());
        }
        LoginsCmd::Always { selector, state } => {
            let on = matches!(state.as_str(), "on" | "yes" | "true");
            let changed = engine.logins.set_always_allow(&selector, on)?;
            println!("{} login(s) now {}.", changed.len(), if on { "used without asking" } else { "ask every time" });
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn full_request_id(engine: &Engine, prefix: &str) -> anyhow::Result<String> {
    let hits: Vec<_> = engine.approvals.pending().into_iter().filter(|r| r.id.starts_with(prefix)).collect();
    match hits.as_slice() {
        [one] => Ok(one.id.clone()),
        [] => anyhow::bail!("no pending request {prefix}"),
        _ => anyhow::bail!("{prefix} matches several requests"),
    }
}

fn skills(engine: &Engine, rt: &tokio::runtime::Runtime, cmd: SkillsCmd) -> anyhow::Result<()> {
    let print_review = |r: &skills_store::Review| {
        println!("{} — {}", r.row.name, r.doc.description);
        if let Some(l) = &r.doc.license {
            println!("license: {l}");
        }
        println!("{}", if r.row.enabled { "status: on" } else { "status: off (enable after reading this)" });
        if !r.unchanged {
            println!("!! files changed since import — re-import to review them");
        }
        println!("\nfindings ({}):", r.findings.len());
        for f in &r.findings {
            println!("  [{:?}] {}:{} {}", f.severity, f.file, f.line, f.message);
        }
        println!("\nfiles:");
        for (p, n) in &r.files {
            println!("  {p} ({n} bytes)");
        }
    };
    match cmd {
        SkillsCmd::List { query, category } => {
            let q = query.unwrap_or_default();
            let list = if q.is_empty() { mybot_catalog::skills().iter().collect() } else { mybot_catalog::search_skills(&q, 300) };
            let mut n = 0;
            for s in list.into_iter().filter(|s| category.as_ref().is_none_or(|c| &s.category == c)) {
                println!("{:<14} {:<34} {}", s.category, s.name, s.description);
                n += 1;
            }
            for s in engine.db.skills()? {
                if q.is_empty() || s.name.contains(&q) || s.description.contains(&q) {
                    println!("{:<14} {:<34} {}{}", s.source, s.name, s.description, if s.source == "imported" && !s.enabled { " (off)" } else { "" });
                    n += 1;
                }
            }
            println!("\n{n} skills");
        }
        SkillsCmd::Show { name } => {
            let s = engine.skill_run(&name, &BTreeMap::new()).map_err(anyhow::Error::msg)?;
            println!("{}", s.text);
        }
        SkillsCmd::Import { source } => {
            let r = if source.starts_with("http://") || source.starts_with("https://") {
                rt.block_on(skills_store::import_url(&engine.db, &source))
            } else {
                skills_store::import_path(&engine.db, std::path::Path::new(&source))
            }
            .map_err(anyhow::Error::msg)?;
            print_review(&r);
            println!("\nImported, switched off. Turn it on with: mybot2 skills enable {}", r.row.name);
        }
        SkillsCmd::Review { name } => print_review(&skills_store::review(&engine.db, &name).map_err(anyhow::Error::msg)?),
        SkillsCmd::Enable { name } => {
            skills_store::set_enabled(&engine.db, &name, true).map_err(anyhow::Error::msg)?;
            println!("On.");
        }
        SkillsCmd::Disable { name } => {
            skills_store::set_enabled(&engine.db, &name, false).map_err(anyhow::Error::msg)?;
            println!("Off.");
        }
        SkillsCmd::Rm { name } => {
            match engine.db.skill(&name)? {
                Some(s) if s.source == "imported" => skills_store::remove(&engine.db, &name).map_err(anyhow::Error::msg)?,
                Some(s) => engine.db.delete_skill(&s.id)?,
                None => anyhow::bail!("no skill \"{name}\" of yours (built-ins can't be removed)"),
            }
            println!("Removed.");
        }
        SkillsCmd::Export { name, to } => {
            let dest = to.map(std::path::PathBuf::from).unwrap_or_else(|| mybot_vault::home().join("exports"));
            let p = skills_store::export(&engine.db, &name, &dest).map_err(anyhow::Error::msg)?;
            println!("{}", p.display());
        }
    }
    Ok(())
}

fn routines(engine: &Arc<Engine>, rt: &tokio::runtime::Runtime, cmd: RoutinesCmd) -> anyhow::Result<()> {
    let printer: crate::gui::Sink = Arc::new(|bot: &str, e: AgentEvent| {
        if let AgentEvent::Finished { status, summary } = e {
            println!("[{bot}] {status:?}: {}", mybot_core::agent::preview(&summary));
        }
    });
    match cmd {
        RoutinesCmd::List => {
            for r in engine.db.routines()? {
                let when = r.schedule.as_deref().map(|s| Cron::parse(s).map(|c| c.describe()).unwrap_or_else(|_| s.into())).unwrap_or_default();
                println!("{}  {:<4} {:<30} {}", &r.id[..8], if r.enabled { "on" } else { "off" }, r.skill, when);
            }
        }
        RoutinesCmd::Add { skill, cron, bot, args } => {
            let c = Cron::parse(&cron).map_err(anyhow::Error::msg)?;
            anyhow::ensure!(mybot_catalog::skill(&skill).is_some() || engine.db.skill(&skill)?.is_some(), "no skill \"{skill}\"");
            if let Some(a) = &args {
                serde_json::from_str::<BTreeMap<String, String>>(a).map_err(|_| anyhow::anyhow!("--args must be a JSON object of strings"))?;
            }
            let bot_id = bot.map(|b| find_bot(engine, &b).map(|b| b.id)).transpose()?;
            let r = engine.db.add_routine(&skill, bot_id.as_deref(), Some(&cron), None, args.as_deref())?;
            println!("Added {} — {}.", &r.id[..8], c.describe());
        }
        RoutinesCmd::Rm { id } => {
            engine.db.delete_routine(&full_routine_id(engine, &id)?)?;
            println!("Removed.");
        }
        RoutinesCmd::On { id } => {
            engine.db.set_routine_enabled(&full_routine_id(engine, &id)?, true)?;
            println!("On.");
        }
        RoutinesCmd::Off { id } => {
            engine.db.set_routine_enabled(&full_routine_id(engine, &id)?, false)?;
            println!("Off.");
        }
        RoutinesCmd::Run { id } => {
            unlock(engine).ok();
            let r = engine.db.routine(&full_routine_id(engine, &id)?)?.ok_or_else(|| anyhow::anyhow!("gone"))?;
            let bot = match &r.bot_id {
                Some(b) => engine.db.bot(b)?,
                None => engine.db.bots()?.into_iter().next(),
            }
            .ok_or_else(|| anyhow::anyhow!("no teammate to run it"))?;
            let args = r.args.as_deref().and_then(|a| serde_json::from_str(a).ok()).unwrap_or_default();
            let skill = engine.skill_run(&r.skill, &args).map_err(anyhow::Error::msg)?;
            let _ = engine.db.record_routine_run(&r.id, None, "run now");
            rt.block_on(run_in_terminal(engine.clone(), bot, format!("Run the skill \"{}\" (run now).", skill.name), Some(skill)))?;
        }
        RoutinesCmd::Watch => {
            unlock(engine).ok();
            println!("Watching routines. Ctrl+C to stop.");
            rt.block_on(async {
                loop {
                    let n = engine.tick_routines(&tokio::runtime::Handle::current(), printer.clone());
                    if n > 0 {
                        println!("{} started {n} routine(s)", chrono::Local::now().format("%H:%M"));
                    }
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(30)) => {}
                        _ = tokio::signal::ctrl_c() => break,
                    }
                }
            });
        }
    }
    Ok(())
}

fn full_routine_id(engine: &Engine, prefix: &str) -> anyhow::Result<String> {
    let hits: Vec<_> = engine.db.routines()?.into_iter().filter(|r| r.id.starts_with(prefix)).collect();
    match hits.as_slice() {
        [one] => Ok(one.id.clone()),
        [] => anyhow::bail!("no routine {prefix}"),
        _ => anyhow::bail!("{prefix} matches several routines"),
    }
}
