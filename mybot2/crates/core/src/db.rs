//! Local storage: SQLite at `~/.mybot/mybot2.db` (or `$MYBOT2_DB`).
//!
//! A separate file from 1.x's `mybot.db` — the schemas differ — while the
//! encrypted vault files are shared (see `mybot-vault`).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

const SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS bots (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL UNIQUE,
  provider    TEXT NOT NULL,
  model       TEXT NOT NULL,
  system      TEXT,
  effort      TEXT NOT NULL DEFAULT 'high',
  mode        TEXT NOT NULL DEFAULT 'ask' CHECK (mode IN ('ask','auto','bypass')),
  created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS tasks (
  id             TEXT PRIMARY KEY,
  bot_id         TEXT NOT NULL REFERENCES bots(id) ON DELETE CASCADE,
  instruction    TEXT NOT NULL,
  status         TEXT NOT NULL DEFAULT 'queued'
                 CHECK (status IN ('queued','running','paused','done','failed','cancelled')),
  summary        TEXT,
  paused_reason  TEXT,
  input_tokens   INTEGER NOT NULL DEFAULT 0,
  output_tokens  INTEGER NOT NULL DEFAULT 0,
  created_at     TEXT NOT NULL DEFAULT (datetime('now')),
  started_at     TEXT,
  completed_at   TEXT
);

CREATE TABLE IF NOT EXISTS task_log (
  id       INTEGER PRIMARY KEY AUTOINCREMENT,
  task_id  TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  kind     TEXT NOT NULL,
  text     TEXT NOT NULL
);

-- Skills you wrote or taught. The built-in library ships in the binary and is
-- never stored here; nothing is ever fetched from the internet.
-- Skills you wrote, taught, or imported (Agent Skills / SKILL.md folders).
-- Imported skills start disabled: a human reads the review and turns them on.
-- `digest` is the sha256 of every file at review time; a skill whose files
-- changed afterwards is refused until reviewed again.
CREATE TABLE IF NOT EXISTS skills (
  id            TEXT PRIMARY KEY,
  name          TEXT NOT NULL UNIQUE,
  source        TEXT NOT NULL CHECK (source IN ('manual','taught','imported')),
  category      TEXT NOT NULL DEFAULT 'Custom',
  description   TEXT NOT NULL DEFAULT '',
  instructions  TEXT NOT NULL,
  safety_rules  TEXT NOT NULL DEFAULT '',
  enabled       INTEGER NOT NULL DEFAULT 1,
  origin        TEXT,
  dir           TEXT,
  digest        TEXT,
  findings      TEXT NOT NULL DEFAULT '[]',
  license       TEXT,
  allowed_tools TEXT,
  created_at    TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS routines (
  id           TEXT PRIMARY KEY,
  skill        TEXT NOT NULL,
  bot_id       TEXT REFERENCES bots(id) ON DELETE SET NULL,
  schedule     TEXT,
  trigger      TEXT,
  args         TEXT,
  enabled      INTEGER NOT NULL DEFAULT 1,
  last_run_at  TEXT,
  created_at   TEXT NOT NULL DEFAULT (datetime('now')),
  CHECK ((schedule IS NULL) <> (trigger IS NULL))
);

CREATE TABLE IF NOT EXISTS routine_runs (
  id          TEXT PRIMARY KEY,
  routine_id  TEXT NOT NULL REFERENCES routines(id) ON DELETE CASCADE,
  task_id     TEXT REFERENCES tasks(id) ON DELETE SET NULL,
  fired_at    TEXT NOT NULL DEFAULT (datetime('now')),
  reason      TEXT NOT NULL
);

-- Every use of a saved login and what you said. Site and username only:
-- the password never touches this database.
CREATE TABLE IF NOT EXISTS login_requests (
  id          TEXT PRIMARY KEY,
  task_id     TEXT,
  bot         TEXT NOT NULL,
  origin      TEXT NOT NULL,
  username    TEXT NOT NULL,
  status      TEXT NOT NULL DEFAULT 'pending'
              CHECK (status IN ('pending','allowed','denied','expired')),
  always      INTEGER NOT NULL DEFAULT 0,
  decided_by  TEXT CHECK (decided_by IN ('human','rule','timeout') OR decided_by IS NULL),
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  decided_at  TEXT
);

-- Teach-a-task recordings: what you did, step by step, before it becomes a skill.
CREATE TABLE IF NOT EXISTS recordings (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL,
  steps       TEXT NOT NULL DEFAULT '[]',
  skill_id    TEXT,
  created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS settings (
  key    TEXT PRIMARY KEY,
  value  TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_tasks_bot ON tasks(bot_id);
CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
CREATE INDEX IF NOT EXISTS idx_log_task ON task_log(task_id);
CREATE INDEX IF NOT EXISTS idx_login_requests_status ON login_requests(status);
"#;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bot {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub model: String,
    pub system: Option<String>,
    pub effort: String,
    pub mode: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub bot_id: String,
    pub instruction: String,
    pub status: String,
    pub summary: Option<String>,
    pub paused_reason: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogEntry {
    pub id: i64,
    pub task_id: String,
    pub at: String,
    pub kind: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillRow {
    pub id: String,
    pub name: String,
    pub source: String,
    pub category: String,
    pub description: String,
    pub instructions: String,
    pub safety_rules: String,
    pub enabled: bool,
    /// Imported skills: where from (path or URL).
    pub origin: Option<String>,
    /// Imported skills: the copied folder under ~/.mybot/skills.
    pub dir: Option<String>,
    /// Imported skills: sha256 of the files as reviewed.
    pub digest: Option<String>,
    /// Imported skills: review findings (JSON).
    pub findings: String,
    pub license: Option<String>,
    pub allowed_tools: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Everything recorded about an imported skill.
pub struct ImportedSkill<'a> {
    pub name: &'a str,
    pub description: &'a str,
    pub body: &'a str,
    pub origin: &'a str,
    pub dir: &'a str,
    pub digest: &'a str,
    pub findings_json: &'a str,
    pub license: Option<&'a str>,
    pub allowed_tools: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Routine {
    pub id: String,
    pub skill: String,
    pub bot_id: Option<String>,
    pub schedule: Option<String>,
    pub trigger: Option<String>,
    pub args: Option<String>,
    pub enabled: bool,
    pub last_run_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoginRequest {
    pub id: String,
    pub task_id: Option<String>,
    pub bot: String,
    pub origin: String,
    pub username: String,
    pub status: String,
    pub always: bool,
    pub decided_by: Option<String>,
    pub created_at: String,
    pub decided_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recording {
    pub id: String,
    pub name: String,
    pub steps: String,
    pub skill_id: Option<String>,
    pub created_at: String,
}

pub type DbResult<T> = Result<T, rusqlite::Error>;

/// Cheap to clone; every clone shares one connection.
#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
    path: PathBuf,
}

pub fn default_path() -> PathBuf {
    std::env::var_os("MYBOT2_DB").map(PathBuf::from).unwrap_or_else(|| mybot_vault::home().join("mybot2.db"))
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

impl Db {
    pub fn open_default() -> DbResult<Self> {
        Self::open(&default_path())
    }

    pub fn open(path: &Path) -> DbResult<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn: Arc::new(Mutex::new(conn)), path: path.to_path_buf() })
    }

    pub fn in_memory() -> DbResult<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn: Arc::new(Mutex::new(conn)), path: PathBuf::from(":memory:") })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn c(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    // --- bots ----------------------------------------------------------------

    pub fn create_bot(&self, name: &str, provider: &str, model: &str, system: Option<&str>) -> DbResult<Bot> {
        let id = new_id();
        self.c().execute(
            "INSERT INTO bots (id, name, provider, model, system) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, name.trim(), provider, model, system.filter(|s| !s.trim().is_empty())],
        )?;
        Ok(self.bot(&id)?.expect("just inserted"))
    }

    fn bot_row(r: &Row) -> rusqlite::Result<Bot> {
        Ok(Bot {
            id: r.get("id")?,
            name: r.get("name")?,
            provider: r.get("provider")?,
            model: r.get("model")?,
            system: r.get("system")?,
            effort: r.get("effort")?,
            mode: r.get("mode")?,
            created_at: r.get("created_at")?,
        })
    }

    pub fn bot(&self, id: &str) -> DbResult<Option<Bot>> {
        self.c().query_row("SELECT * FROM bots WHERE id = ?1", [id], Self::bot_row).optional()
    }

    pub fn bot_by_name(&self, name: &str) -> DbResult<Option<Bot>> {
        self.c().query_row("SELECT * FROM bots WHERE name = ?1 COLLATE NOCASE", [name], Self::bot_row).optional()
    }

    pub fn bots(&self) -> DbResult<Vec<Bot>> {
        let c = self.c();
        let mut st = c.prepare("SELECT * FROM bots ORDER BY created_at, name")?;
        st.query_map([], Self::bot_row)?.collect()
    }

    pub fn update_bot(&self, bot: &Bot) -> DbResult<()> {
        self.c().execute(
            "UPDATE bots SET name=?2, provider=?3, model=?4, system=?5, effort=?6, mode=?7 WHERE id=?1",
            params![bot.id, bot.name, bot.provider, bot.model, bot.system, bot.effort, bot.mode],
        )?;
        Ok(())
    }

    pub fn delete_bot(&self, id: &str) -> DbResult<()> {
        self.c().execute("DELETE FROM bots WHERE id = ?1", [id])?;
        Ok(())
    }

    // --- tasks ---------------------------------------------------------------

    fn task_row(r: &Row) -> rusqlite::Result<Task> {
        Ok(Task {
            id: r.get("id")?,
            bot_id: r.get("bot_id")?,
            instruction: r.get("instruction")?,
            status: r.get("status")?,
            summary: r.get("summary")?,
            paused_reason: r.get("paused_reason")?,
            input_tokens: r.get("input_tokens")?,
            output_tokens: r.get("output_tokens")?,
            created_at: r.get("created_at")?,
            started_at: r.get("started_at")?,
            completed_at: r.get("completed_at")?,
        })
    }

    pub fn create_task(&self, bot_id: &str, instruction: &str) -> DbResult<Task> {
        let id = new_id();
        self.c().execute("INSERT INTO tasks (id, bot_id, instruction) VALUES (?1, ?2, ?3)", params![id, bot_id, instruction])?;
        Ok(self.task(&id)?.expect("just inserted"))
    }

    pub fn task(&self, id: &str) -> DbResult<Option<Task>> {
        self.c().query_row("SELECT * FROM tasks WHERE id = ?1", [id], Self::task_row).optional()
    }

    pub fn tasks_for(&self, bot_id: &str, limit: usize) -> DbResult<Vec<Task>> {
        let c = self.c();
        let mut st = c.prepare("SELECT * FROM tasks WHERE bot_id = ?1 ORDER BY created_at DESC, rowid DESC LIMIT ?2")?;
        let mut v: Vec<Task> = st.query_map(params![bot_id, limit as i64], Self::task_row)?.collect::<DbResult<_>>()?;
        v.reverse();
        Ok(v)
    }

    pub fn tasks_with_status(&self, status: &str) -> DbResult<Vec<Task>> {
        let c = self.c();
        let mut st = c.prepare("SELECT * FROM tasks WHERE status = ?1 ORDER BY created_at")?;
        st.query_map([status], Self::task_row)?.collect()
    }

    pub fn recent_tasks(&self, limit: usize) -> DbResult<Vec<Task>> {
        let c = self.c();
        let mut st = c.prepare("SELECT * FROM tasks ORDER BY created_at DESC, rowid DESC LIMIT ?1")?;
        st.query_map([limit as i64], Self::task_row)?.collect()
    }

    pub fn set_task_status(&self, id: &str, status: &str) -> DbResult<()> {
        let c = self.c();
        match status {
            "running" => c.execute(
                "UPDATE tasks SET status=?2, paused_reason=NULL, started_at=COALESCE(started_at, datetime('now')) WHERE id=?1",
                params![id, status],
            )?,
            "done" | "failed" | "cancelled" => c.execute(
                "UPDATE tasks SET status=?2, paused_reason=NULL, completed_at=datetime('now') WHERE id=?1",
                params![id, status],
            )?,
            _ => c.execute("UPDATE tasks SET status=?2 WHERE id=?1", params![id, status])?,
        };
        Ok(())
    }

    pub fn pause_task(&self, id: &str, reason: &str) -> DbResult<()> {
        self.c().execute("UPDATE tasks SET status='paused', paused_reason=?2 WHERE id=?1", params![id, reason])?;
        Ok(())
    }

    pub fn finish_task(&self, id: &str, status: &str, summary: &str, input: u64, output: u64) -> DbResult<()> {
        self.set_task_status(id, status)?;
        self.c().execute(
            "UPDATE tasks SET summary=?2, input_tokens=?3, output_tokens=?4 WHERE id=?1",
            params![id, summary, input as i64, output as i64],
        )?;
        Ok(())
    }

    pub fn append_log(&self, task_id: &str, kind: &str, text: &str) -> DbResult<()> {
        self.c().execute("INSERT INTO task_log (task_id, kind, text) VALUES (?1, ?2, ?3)", params![task_id, kind, text])?;
        Ok(())
    }

    pub fn log(&self, task_id: &str) -> DbResult<Vec<LogEntry>> {
        let c = self.c();
        let mut st = c.prepare("SELECT * FROM task_log WHERE task_id = ?1 ORDER BY id")?;
        st.query_map([task_id], |r| {
            Ok(LogEntry { id: r.get("id")?, task_id: r.get("task_id")?, at: r.get("at")?, kind: r.get("kind")?, text: r.get("text")? })
        })?
        .collect()
    }

    /// A console that died mid-run leaves rows "running" or "paused" with
    /// nothing behind them. Called at startup.
    pub fn sweep_orphans(&self) -> DbResult<usize> {
        let n = self.c().execute(
            "UPDATE tasks SET status='failed', completed_at=datetime('now') WHERE status IN ('running','paused','queued')",
            [],
        )?;
        Ok(n)
    }

    // --- skills ----------------------------------------------------------------

    fn skill_row(r: &Row) -> rusqlite::Result<SkillRow> {
        Ok(SkillRow {
            id: r.get("id")?,
            name: r.get("name")?,
            source: r.get("source")?,
            category: r.get("category")?,
            description: r.get("description")?,
            instructions: r.get("instructions")?,
            safety_rules: r.get("safety_rules")?,
            enabled: r.get::<_, i64>("enabled")? != 0,
            origin: r.get("origin")?,
            dir: r.get("dir")?,
            digest: r.get("digest")?,
            findings: r.get("findings")?,
            license: r.get("license")?,
            allowed_tools: r.get("allowed_tools")?,
            created_at: r.get("created_at")?,
            updated_at: r.get("updated_at")?,
        })
    }

    /// Record an imported skill — disabled until a human enables it.
    pub fn add_imported_skill(&self, s: &ImportedSkill<'_>) -> DbResult<SkillRow> {
        let id = new_id();
        self.c().execute(
            "INSERT INTO skills (id, name, source, category, description, instructions, enabled, origin, dir, digest, findings, license, allowed_tools) \
             VALUES (?1, ?2, 'imported', 'Imported', ?3, ?4, 0, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![id, s.name, s.description, s.body, s.origin, s.dir, s.digest, s.findings_json, s.license, s.allowed_tools],
        )?;
        Ok(self.skill(&id)?.expect("just inserted"))
    }

    pub fn set_skill_enabled(&self, id: &str, enabled: bool) -> DbResult<()> {
        self.c().execute("UPDATE skills SET enabled=?2, updated_at=datetime('now') WHERE id=?1", params![id, enabled as i64])?;
        Ok(())
    }

    pub fn add_skill(
        &self,
        name: &str,
        source: &str,
        category: &str,
        description: &str,
        instructions: &str,
        safety_rules: &str,
    ) -> DbResult<SkillRow> {
        let id = new_id();
        self.c().execute(
            "INSERT INTO skills (id, name, source, category, description, instructions, safety_rules) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![id, name.trim(), source, category, description, instructions, safety_rules],
        )?;
        Ok(self.skill(&id)?.expect("just inserted"))
    }

    pub fn skill(&self, id_or_name: &str) -> DbResult<Option<SkillRow>> {
        self.c()
            .query_row("SELECT * FROM skills WHERE id = ?1 OR name = ?1 COLLATE NOCASE", [id_or_name], Self::skill_row)
            .optional()
    }

    pub fn skills(&self) -> DbResult<Vec<SkillRow>> {
        let c = self.c();
        let mut st = c.prepare("SELECT * FROM skills ORDER BY source, name")?;
        st.query_map([], Self::skill_row)?.collect()
    }

    pub fn update_skill(&self, id: &str, instructions: &str, safety_rules: &str, description: &str) -> DbResult<()> {
        self.c().execute(
            "UPDATE skills SET instructions=?2, safety_rules=?3, description=?4, updated_at=datetime('now') WHERE id=?1",
            params![id, instructions, safety_rules, description],
        )?;
        Ok(())
    }

    pub fn delete_skill(&self, id: &str) -> DbResult<()> {
        self.c().execute("DELETE FROM skills WHERE id = ?1", [id])?;
        Ok(())
    }

    // --- routines ---------------------------------------------------------------

    fn routine_row(r: &Row) -> rusqlite::Result<Routine> {
        Ok(Routine {
            id: r.get("id")?,
            skill: r.get("skill")?,
            bot_id: r.get("bot_id")?,
            schedule: r.get("schedule")?,
            trigger: r.get("trigger")?,
            args: r.get("args")?,
            enabled: r.get::<_, i64>("enabled")? != 0,
            last_run_at: r.get("last_run_at")?,
            created_at: r.get("created_at")?,
        })
    }

    pub fn add_routine(
        &self,
        skill: &str,
        bot_id: Option<&str>,
        schedule: Option<&str>,
        trigger: Option<&str>,
        args: Option<&str>,
    ) -> DbResult<Routine> {
        let id = new_id();
        self.c().execute(
            "INSERT INTO routines (id, skill, bot_id, schedule, trigger, args) VALUES (?1,?2,?3,?4,?5,?6)",
            params![id, skill, bot_id, schedule, trigger, args],
        )?;
        Ok(self.routine(&id)?.expect("just inserted"))
    }

    pub fn routine(&self, id: &str) -> DbResult<Option<Routine>> {
        self.c().query_row("SELECT * FROM routines WHERE id = ?1", [id], Self::routine_row).optional()
    }

    pub fn routines(&self) -> DbResult<Vec<Routine>> {
        let c = self.c();
        let mut st = c.prepare("SELECT * FROM routines ORDER BY created_at")?;
        st.query_map([], Self::routine_row)?.collect()
    }

    pub fn set_routine_enabled(&self, id: &str, enabled: bool) -> DbResult<()> {
        self.c().execute("UPDATE routines SET enabled=?2 WHERE id=?1", params![id, enabled as i64])?;
        Ok(())
    }

    pub fn delete_routine(&self, id: &str) -> DbResult<()> {
        self.c().execute("DELETE FROM routines WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn record_routine_run(&self, routine_id: &str, task_id: Option<&str>, reason: &str) -> DbResult<()> {
        let c = self.c();
        c.execute(
            "INSERT INTO routine_runs (id, routine_id, task_id, reason) VALUES (?1,?2,?3,?4)",
            params![new_id(), routine_id, task_id, reason],
        )?;
        c.execute("UPDATE routines SET last_run_at=datetime('now') WHERE id=?1", [routine_id])?;
        Ok(())
    }

    pub fn routine_runs(&self, routine_id: &str) -> DbResult<Vec<(String, Option<String>, String)>> {
        let c = self.c();
        let mut st = c.prepare("SELECT fired_at, task_id, reason FROM routine_runs WHERE routine_id=?1 ORDER BY fired_at DESC LIMIT 50")?;
        st.query_map([routine_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect()
    }

    // --- login requests -----------------------------------------------------------

    fn login_row(r: &Row) -> rusqlite::Result<LoginRequest> {
        Ok(LoginRequest {
            id: r.get("id")?,
            task_id: r.get("task_id")?,
            bot: r.get("bot")?,
            origin: r.get("origin")?,
            username: r.get("username")?,
            status: r.get("status")?,
            always: r.get::<_, i64>("always")? != 0,
            decided_by: r.get("decided_by")?,
            created_at: r.get("created_at")?,
            decided_at: r.get("decided_at")?,
        })
    }

    pub fn insert_login_request(&self, task_id: Option<&str>, bot: &str, origin: &str, username: &str) -> DbResult<String> {
        let id = new_id();
        self.c().execute(
            "INSERT INTO login_requests (id, task_id, bot, origin, username) VALUES (?1,?2,?3,?4,?5)",
            params![id, task_id, bot, origin, username],
        )?;
        Ok(id)
    }

    pub fn login_request(&self, id: &str) -> DbResult<Option<LoginRequest>> {
        self.c().query_row("SELECT * FROM login_requests WHERE id = ?1", [id], Self::login_row).optional()
    }

    /// Settle a pending request. Returns false if it was already settled.
    pub fn settle_login_request(&self, id: &str, status: &str, by: &str, always: bool) -> DbResult<bool> {
        let n = self.c().execute(
            "UPDATE login_requests SET status=?2, decided_by=?3, always=?4, decided_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') \
             WHERE id=?1 AND status='pending'",
            params![id, status, by, always as i64],
        )?;
        Ok(n > 0)
    }

    pub fn pending_login_requests(&self) -> DbResult<Vec<LoginRequest>> {
        let c = self.c();
        let mut st = c.prepare("SELECT * FROM login_requests WHERE status='pending' ORDER BY created_at, rowid")?;
        st.query_map([], Self::login_row)?.collect()
    }

    pub fn recent_login_requests(&self, limit: usize) -> DbResult<Vec<LoginRequest>> {
        let c = self.c();
        let mut st = c.prepare("SELECT * FROM login_requests ORDER BY created_at DESC, rowid DESC LIMIT ?1")?;
        st.query_map([limit as i64], Self::login_row)?.collect()
    }

    // --- recordings ------------------------------------------------------------

    pub fn add_recording(&self, name: &str, steps_json: &str) -> DbResult<Recording> {
        let id = new_id();
        self.c().execute("INSERT INTO recordings (id, name, steps) VALUES (?1,?2,?3)", params![id, name, steps_json])?;
        Ok(self.recording(&id)?.expect("just inserted"))
    }

    pub fn recording(&self, id: &str) -> DbResult<Option<Recording>> {
        self.c()
            .query_row("SELECT * FROM recordings WHERE id=?1", [id], |r| {
                Ok(Recording { id: r.get("id")?, name: r.get("name")?, steps: r.get("steps")?, skill_id: r.get("skill_id")?, created_at: r.get("created_at")? })
            })
            .optional()
    }

    pub fn set_recording_skill(&self, id: &str, skill_id: &str) -> DbResult<()> {
        self.c().execute("UPDATE recordings SET skill_id=?2 WHERE id=?1", params![id, skill_id])?;
        Ok(())
    }

    // --- settings ------------------------------------------------------------------

    pub fn setting(&self, key: &str) -> DbResult<Option<String>> {
        self.c().query_row("SELECT value FROM settings WHERE key=?1", [key], |r| r.get(0)).optional()
    }

    pub fn set_setting(&self, key: &str, value: &str) -> DbResult<()> {
        self.c().execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle() {
        let db = Db::in_memory().unwrap();
        let bot = db.create_bot("scout", "anthropic", "claude-opus-5-5", None).unwrap();
        assert!(db.create_bot("Scout", "openai", "x", None).is_err() || db.bots().unwrap().len() == 2);
        let t = db.create_task(&bot.id, "find things").unwrap();
        db.set_task_status(&t.id, "running").unwrap();
        db.append_log(&t.id, "user", "find things").unwrap();
        db.pause_task(&t.id, "captcha: solve it").unwrap();
        assert_eq!(db.tasks_with_status("paused").unwrap().len(), 1);
        db.finish_task(&t.id, "done", "found 3", 10, 20).unwrap();
        let t = db.task(&t.id).unwrap().unwrap();
        assert_eq!((t.status.as_str(), t.summary.as_deref(), t.output_tokens), ("done", Some("found 3"), 20));
        assert!(t.started_at.is_some() && t.completed_at.is_some() && t.paused_reason.is_none());
        assert_eq!(db.log(&t.id).unwrap().len(), 1);
        assert_eq!(db.bot_by_name("SCOUT").unwrap().unwrap().id, bot.id);
    }

    #[test]
    fn routines_need_exactly_one_trigger() {
        let db = Db::in_memory().unwrap();
        assert!(db.add_routine("x", None, Some("0 9 * * 1"), None, None).is_ok());
        assert!(db.add_routine("x", None, None, Some("email"), None).is_ok());
        assert!(db.add_routine("x", None, None, None, None).is_err());
        assert!(db.add_routine("x", None, Some("0 9 * * *"), Some("e"), None).is_err());
    }

    #[test]
    fn login_requests_settle_once() {
        let db = Db::in_memory().unwrap();
        let id = db.insert_login_request(None, "scout", "https://github.com", "alice").unwrap();
        assert_eq!(db.pending_login_requests().unwrap().len(), 1);
        assert!(db.settle_login_request(&id, "allowed", "human", true).unwrap());
        assert!(!db.settle_login_request(&id, "denied", "human", false).unwrap());
        let r = db.login_request(&id).unwrap().unwrap();
        assert_eq!((r.status.as_str(), r.always), ("allowed", true));
    }
}
