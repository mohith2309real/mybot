-- MyBot local state. SQLite via node:sqlite (no native dependency).
--
-- Architecture note: the workspace is per-ACCOUNT, not per-bot. This mirrors how
-- Grok Bot actually works -- every bot shares one computer, one /workspace, and
-- one browser profile. Bots are personas over a shared machine, which is also
-- why they are not a security boundary from each other.

PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS bots (
  id         TEXT PRIMARY KEY,
  name       TEXT NOT NULL UNIQUE,
  provider   TEXT NOT NULL CHECK (provider IN ('anthropic', 'openai', 'gemini')),
  model      TEXT NOT NULL,
  system     TEXT,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- One row per account. Holds the shared browser cookie/session blob so a login
-- performed by (or handed to) one bot is usable by all of them.
CREATE TABLE IF NOT EXISTS workspace_sessions (
  id           INTEGER PRIMARY KEY CHECK (id = 1),
  cookies_blob BLOB,
  updated_at   TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS skills (
  id           TEXT PRIMARY KEY,
  name         TEXT NOT NULL UNIQUE,
  instructions TEXT NOT NULL,
  -- 'manual'  : hand-written in the dashboard
  -- 'taught'  : compiled from a Teach-a-Task recording
  -- No 'remote' source exists by design: skills are never fetched from the
  -- internet. See ClawHub/ClawHavoc, Feb 2026 (341 of 2857 registry skills
  -- malicious, later 824) for why.
  source       TEXT NOT NULL CHECK (source IN ('manual', 'taught')),
  safety_rules TEXT,
  created_at   TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS routines (
  id         TEXT PRIMARY KEY,
  skill_id   TEXT NOT NULL REFERENCES skills(id) ON DELETE CASCADE,
  bot_id     TEXT REFERENCES bots(id) ON DELETE SET NULL,
  -- exactly one of schedule (cron) / trigger (event name) should be set
  schedule   TEXT,
  trigger    TEXT,
  enabled    INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  CHECK ((schedule IS NOT NULL) <> (trigger IS NOT NULL))
);

CREATE TABLE IF NOT EXISTS tasks (
  id           TEXT PRIMARY KEY,
  bot_id       TEXT NOT NULL REFERENCES bots(id) ON DELETE CASCADE,
  instruction  TEXT NOT NULL,
  -- 'paused' is the human-in-the-loop state: the bot hit a security boundary
  -- (CAPTCHA / 2FA / payment / always-confirm) and is waiting, not polling.
  status       TEXT NOT NULL DEFAULT 'queued'
               CHECK (status IN ('queued','running','paused','done','failed','cancelled')),
  log          TEXT NOT NULL DEFAULT '[]',
  started_at   TEXT,
  completed_at TEXT
);

-- Teach-a-task recordings. Raw action logs, kept separate from the compiled
-- skill so a bad compile can be re-run against the original demonstration.
CREATE TABLE IF NOT EXISTS recordings (
  id          TEXT PRIMARY KEY,
  label       TEXT NOT NULL,
  actions     TEXT NOT NULL DEFAULT '[]',
  status      TEXT NOT NULL DEFAULT 'recording'
              CHECK (status IN ('recording','stopped','compiled')),
  skill_id    TEXT REFERENCES skills(id) ON DELETE SET NULL,
  started_at  TEXT NOT NULL DEFAULT (datetime('now')),
  stopped_at  TEXT
);

-- Each firing of a routine, so a schedule that silently stops firing is visible.
CREATE TABLE IF NOT EXISTS routine_runs (
  id         TEXT PRIMARY KEY,
  routine_id TEXT NOT NULL REFERENCES routines(id) ON DELETE CASCADE,
  task_id    TEXT REFERENCES tasks(id) ON DELETE SET NULL,
  error      TEXT,
  fired_at   TEXT NOT NULL DEFAULT (datetime('now'))
);

-- A bot asking for a file off the human's own machine.
--
-- 'arbitrated' is the timeout path: when a human never answers, a separate
-- model call judges whether the task genuinely needs the file. It can only ever
-- DENY or escalate -- it cannot grant, because an unanswered request is not
-- consent no matter how reasonable the request looks.
CREATE TABLE IF NOT EXISTS host_requests (
  id          TEXT PRIMARY KEY,
  task_id     TEXT REFERENCES tasks(id) ON DELETE CASCADE,
  bot         TEXT NOT NULL,
  path        TEXT NOT NULL,
  reason      TEXT NOT NULL,
  status      TEXT NOT NULL DEFAULT 'pending'
              CHECK (status IN ('pending','granted','denied','arbitrated','expired')),
  verdict     TEXT,
  decided_by  TEXT CHECK (decided_by IN ('human','arbiter') OR decided_by IS NULL),
  created_at  TEXT NOT NULL DEFAULT (datetime('now')),
  decided_at  TEXT
);

-- Merging two conversations' computers. Requires BOTH sides to agree; a
-- non-response is not agreement, it goes to the human.
CREATE TABLE IF NOT EXISTS merges (
  id            TEXT PRIMARY KEY,
  from_conv     TEXT NOT NULL,
  to_conv       TEXT NOT NULL,
  scope         TEXT NOT NULL DEFAULT 'workspace'
                CHECK (scope IN ('workspace','desktop','both')),
  requested_by  TEXT NOT NULL,
  from_consent  TEXT NOT NULL DEFAULT 'granted'
                CHECK (from_consent IN ('granted','denied','pending')),
  to_consent    TEXT NOT NULL DEFAULT 'pending'
                CHECK (to_consent IN ('granted','denied','pending')),
  status        TEXT NOT NULL DEFAULT 'pending'
                CHECK (status IN ('pending','active','refused','human_review','revoked')),
  created_at    TEXT NOT NULL DEFAULT (datetime('now')),
  decided_at    TEXT
);

CREATE INDEX IF NOT EXISTS idx_host_requests_status ON host_requests(status);
CREATE INDEX IF NOT EXISTS idx_merges_status ON merges(status);
CREATE INDEX IF NOT EXISTS idx_tasks_bot    ON tasks(bot_id);
CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
CREATE INDEX IF NOT EXISTS idx_routines_skill ON routines(skill_id);
CREATE INDEX IF NOT EXISTS idx_routine_runs ON routine_runs(routine_id);

-- Every time a bot asked to use one of your saved logins, and what you said.
-- Holds the site and the username only: the password never touches this
-- database. 'expired' is an unanswered request, which fills nothing -- silence
-- is not a yes. decided_by='rule' is a login you marked "always for this site".
CREATE TABLE IF NOT EXISTS login_requests (
  id          TEXT PRIMARY KEY,
  task_id     TEXT REFERENCES tasks(id) ON DELETE CASCADE,
  bot         TEXT NOT NULL,
  origin      TEXT NOT NULL,
  username    TEXT NOT NULL,
  status      TEXT NOT NULL DEFAULT 'pending'
              CHECK (status IN ('pending','allowed','denied','expired')),
  always      INTEGER NOT NULL DEFAULT 0,
  decided_by  TEXT CHECK (decided_by IN ('human','rule','timeout') OR decided_by IS NULL),
  created_at  TEXT NOT NULL DEFAULT (datetime('now')),
  decided_at  TEXT
);

CREATE INDEX IF NOT EXISTS idx_login_requests_status ON login_requests(status);
