import { DatabaseSync, StatementSync } from 'node:sqlite';
import { randomUUID } from 'node:crypto';
import { mkdirSync, readFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import type { ProviderName } from '../providers/types.ts';

const HERE = dirname(fileURLToPath(import.meta.url));
const DIR = process.env.MYBOT_HOME ?? join(homedir(), '.mybot');
const DB_PATH = process.env.MYBOT_DB ?? join(DIR, 'mybot.db');

export interface Bot {
  id: string;
  name: string;
  provider: ProviderName;
  model: string;
  system: string | null;
  created_at: string;
}

export type TaskStatus = 'queued' | 'running' | 'paused' | 'done' | 'failed' | 'cancelled';

export interface Task {
  id: string;
  bot_id: string;
  instruction: string;
  status: TaskStatus;
  log: string;
  started_at: string | null;
  completed_at: string | null;
}

// ⚡ Bolt: Cache compiled StatementSync objects to avoid recompilation overhead.
// Using a WeakMap tied to the specific database instance safely handles multiple
// connections preventing memory leaks and cross-connection errors.
const statementCache = new WeakMap<DatabaseSync, Map<string, StatementSync>>();

function prepare(db: DatabaseSync, sql: string): StatementSync {
  let cache = statementCache.get(db);
  if (!cache) {
    cache = new Map();
    statementCache.set(db, cache);
  }
  let stmt = cache.get(sql);
  if (!stmt) {
    stmt = db.prepare(sql);
    cache.set(sql, stmt);
  }
  return stmt;
}

let db: DatabaseSync | undefined;

export function openDb(): DatabaseSync {
  if (db) return db;
  mkdirSync(dirname(DB_PATH), { recursive: true, mode: 0o700 });
  db = new DatabaseSync(DB_PATH);
  db.exec(readFileSync(join(HERE, 'schema.sql'), 'utf8'));
  migrate(db);
  return db;
}

/**
 * `CREATE TABLE IF NOT EXISTS` never adds a column to a table that already
 * exists, so columns introduced after someone's db was created need an explicit
 * ALTER. Guarded by PRAGMA so it is safe to run on every open.
 */
function migrate(handle: DatabaseSync): void {
  const columns = (table: string): Set<string> =>
    new Set(
      (prepare(handle, `PRAGMA table_info(${table})`).all() as unknown as { name: string }[]).map(
        (c) => c.name,
      ),
    );

  const taskCols = columns('tasks');
  // Why the bot stopped and is waiting for a human (step 3 takeover).
  if (!taskCols.has('paused_reason')) handle.exec('ALTER TABLE tasks ADD COLUMN paused_reason TEXT');
  if (!taskCols.has('paused_at')) handle.exec('ALTER TABLE tasks ADD COLUMN paused_at TEXT');

  const routineCols = columns('routines');
  if (!routineCols.has('last_run_at')) handle.exec('ALTER TABLE routines ADD COLUMN last_run_at TEXT');
  if (!routineCols.has('next_run_at')) handle.exec('ALTER TABLE routines ADD COLUMN next_run_at TEXT');
}

export function dbPath(): string {
  return DB_PATH;
}

// --- bots ------------------------------------------------------------------

export function createBot(input: {
  name: string;
  provider: ProviderName;
  model: string;
  system?: string;
}): Bot {
  const id = randomUUID();
  prepare(openDb(), 'INSERT INTO bots (id, name, provider, model, system) VALUES (?, ?, ?, ?, ?)')
    .run(id, input.name, input.provider, input.model, input.system ?? null);
  return getBot(id)!;
}

export function getBot(id: string): Bot | undefined {
  return prepare(openDb(), 'SELECT * FROM bots WHERE id = ?').get(id) as Bot | undefined;
}

export function getBotByName(name: string): Bot | undefined {
  return prepare(openDb(), 'SELECT * FROM bots WHERE name = ?').get(name) as Bot | undefined;
}

export function listBots(): Bot[] {
  return prepare(openDb(), 'SELECT * FROM bots ORDER BY created_at').all() as unknown as Bot[];
}

export function deleteBot(id: string): void {
  prepare(openDb(), 'DELETE FROM bots WHERE id = ?').run(id);
}

// --- tasks -----------------------------------------------------------------

export function createTask(botId: string, instruction: string): Task {
  const id = randomUUID();
  prepare(openDb(), 'INSERT INTO tasks (id, bot_id, instruction, status) VALUES (?, ?, ?, ?)')
    .run(id, botId, instruction, 'queued');
  return getTask(id)!;
}

export function getTask(id: string): Task | undefined {
  return prepare(openDb(), 'SELECT * FROM tasks WHERE id = ?').get(id) as Task | undefined;
}

export function listTasks(botId?: string): Task[] {
  const q = botId
    ? prepare(openDb(), 'SELECT * FROM tasks WHERE bot_id = ? ORDER BY rowid DESC')
    : prepare(openDb(), 'SELECT * FROM tasks ORDER BY rowid DESC');
  return (botId ? q.all(botId) : q.all()) as unknown as Task[];
}

export function setTaskStatus(id: string, status: TaskStatus): void {
  const dbh = openDb();
  if (status === 'running') {
    prepare(dbh, "UPDATE tasks SET status = ?, started_at = COALESCE(started_at, datetime('now')) WHERE id = ?")
      .run(status, id);
  } else if (status === 'done' || status === 'failed' || status === 'cancelled') {
    prepare(dbh, "UPDATE tasks SET status = ?, completed_at = datetime('now') WHERE id = ?").run(status, id);
  } else {
    prepare(dbh, 'UPDATE tasks SET status = ? WHERE id = ?').run(status, id);
  }
}

export interface LogEntry {
  at: string;
  kind: string;
  text: string;
}

export function appendTaskLog(id: string, kind: string, text: string): void {
  const row = prepare(openDb(), 'SELECT log FROM tasks WHERE id = ?').get(id) as
    | { log: string }
    | undefined;
  if (!row) return;
  const entries: LogEntry[] = JSON.parse(row.log || '[]');
  entries.push({ at: new Date().toISOString(), kind, text });
  prepare(openDb(), 'UPDATE tasks SET log = ? WHERE id = ?').run(JSON.stringify(entries), id);
}

export function readTaskLog(id: string): LogEntry[] {
  const row = prepare(openDb(), 'SELECT log FROM tasks WHERE id = ?').get(id) as
    | { log: string }
    | undefined;
  return row ? JSON.parse(row.log || '[]') : [];
}

// --- pause / resume (step 3 human-in-the-loop) -----------------------------

export function pauseTask(id: string, reason: string): void {
  prepare(openDb(), "UPDATE tasks SET status='paused', paused_reason=?, paused_at=datetime('now') WHERE id=?")
    .run(reason, id);
}

export function resumeTask(id: string): void {
  prepare(openDb(), "UPDATE tasks SET status='running', paused_reason=NULL, paused_at=NULL WHERE id=?")
    .run(id);
}

export function listPausedTasks(): Task[] {
  return prepare(openDb(), "SELECT * FROM tasks WHERE status='paused' ORDER BY paused_at DESC")
    .all() as unknown as Task[];
}

// --- skills ----------------------------------------------------------------

export interface Skill {
  id: string;
  name: string;
  instructions: string;
  source: 'manual' | 'taught';
  safety_rules: string | null;
  created_at: string;
}

export function createSkill(input: {
  name: string;
  instructions: string;
  source?: 'manual' | 'taught';
  safetyRules?: string;
}): Skill {
  const id = randomUUID();
  prepare(openDb(), 'INSERT INTO skills (id, name, instructions, source, safety_rules) VALUES (?, ?, ?, ?, ?)')
    .run(id, input.name, input.instructions, input.source ?? 'manual', input.safetyRules ?? null);
  return getSkill(id)!;
}

export function getSkill(id: string): Skill | undefined {
  return prepare(openDb(), 'SELECT * FROM skills WHERE id = ?').get(id) as unknown as Skill | undefined;
}

export function getSkillByName(name: string): Skill | undefined {
  return prepare(openDb(), 'SELECT * FROM skills WHERE name = ?').get(name) as unknown as Skill | undefined;
}

export function listSkills(): Skill[] {
  return prepare(openDb(), 'SELECT * FROM skills ORDER BY created_at DESC').all() as unknown as Skill[];
}

export function updateSkill(
  id: string,
  patch: { instructions?: string; safetyRules?: string | null },
): void {
  const s = getSkill(id);
  if (!s) throw new Error(`No skill ${id}`);
  prepare(openDb(), 'UPDATE skills SET instructions = ?, safety_rules = ? WHERE id = ?')
    .run(
      patch.instructions ?? s.instructions,
      patch.safetyRules === undefined ? s.safety_rules : patch.safetyRules,
      id,
    );
}

export function deleteSkill(id: string): void {
  prepare(openDb(), 'DELETE FROM skills WHERE id = ?').run(id);
}

// --- routines --------------------------------------------------------------

export interface Routine {
  id: string;
  skill_id: string;
  bot_id: string | null;
  schedule: string | null;
  trigger: string | null;
  enabled: number;
  created_at: string;
  last_run_at: string | null;
  next_run_at: string | null;
}

export function createRoutine(input: {
  skillId: string;
  botId?: string;
  schedule?: string;
  trigger?: string;
}): Routine {
  // The schema enforces exactly-one-of via CHECK; fail early with a clear
  // message rather than surfacing a raw constraint error.
  if (!!input.schedule === !!input.trigger) {
    throw new Error('A routine needs exactly one of schedule (cron) or trigger (event name).');
  }
  const id = randomUUID();
  prepare(openDb(), 'INSERT INTO routines (id, skill_id, bot_id, schedule, trigger) VALUES (?, ?, ?, ?, ?)')
    .run(id, input.skillId, input.botId ?? null, input.schedule ?? null, input.trigger ?? null);
  return getRoutine(id)!;
}

export function getRoutine(id: string): Routine | undefined {
  return prepare(openDb(), 'SELECT * FROM routines WHERE id = ?').get(id) as unknown as Routine | undefined;
}

export function listRoutines(): Routine[] {
  return prepare(openDb(), 'SELECT * FROM routines ORDER BY created_at').all() as unknown as Routine[];
}

export function setRoutineEnabled(id: string, enabled: boolean): void {
  prepare(openDb(), 'UPDATE routines SET enabled = ? WHERE id = ?').run(enabled ? 1 : 0, id);
}

export function deleteRoutine(id: string): void {
  prepare(openDb(), 'DELETE FROM routines WHERE id = ?').run(id);
}

export function markRoutineRun(id: string, nextRunAt?: string): void {
  prepare(openDb(), "UPDATE routines SET last_run_at = datetime('now'), next_run_at = ? WHERE id = ?")
    .run(nextRunAt ?? null, id);
}

export function recordRoutineRun(routineId: string, taskId?: string, error?: string): void {
  prepare(openDb(), 'INSERT INTO routine_runs (id, routine_id, task_id, error) VALUES (?, ?, ?, ?)')
    .run(randomUUID(), routineId, taskId ?? null, error ?? null);
}

export interface RoutineRun {
  id: string;
  routine_id: string;
  task_id: string | null;
  error: string | null;
  fired_at: string;
}

export function listRoutineRuns(routineId: string, limit = 20): RoutineRun[] {
  return prepare(openDb(), 'SELECT * FROM routine_runs WHERE routine_id = ? ORDER BY fired_at DESC LIMIT ?')
    .all(routineId, limit) as unknown as RoutineRun[];
}

// --- teach-a-task recordings ----------------------------------------------

export interface Recording {
  id: string;
  label: string;
  actions: string;
  status: 'recording' | 'stopped' | 'compiled';
  skill_id: string | null;
  started_at: string;
  stopped_at: string | null;
}

export function createRecording(label: string): Recording {
  const id = randomUUID();
  prepare(openDb(), 'INSERT INTO recordings (id, label) VALUES (?, ?)').run(id, label);
  return getRecording(id)!;
}

export function getRecording(id: string): Recording | undefined {
  return prepare(openDb(), 'SELECT * FROM recordings WHERE id = ?').get(id) as unknown as
    | Recording
    | undefined;
}

export function activeRecording(): Recording | undefined {
  return prepare(openDb(), "SELECT * FROM recordings WHERE status='recording' ORDER BY started_at DESC LIMIT 1")
    .get() as unknown as Recording | undefined;
}

export function listRecordings(): Recording[] {
  return prepare(openDb(), 'SELECT * FROM recordings ORDER BY started_at DESC').all() as unknown as Recording[];
}

export function saveRecordingActions(id: string, actions: unknown[]): void {
  prepare(openDb(), 'UPDATE recordings SET actions = ? WHERE id = ?').run(JSON.stringify(actions), id);
}

export function stopRecording(id: string): void {
  prepare(openDb(), "UPDATE recordings SET status='stopped', stopped_at=datetime('now') WHERE id=?")
    .run(id);
}

export function attachRecordingSkill(id: string, skillId: string): void {
  prepare(openDb(), "UPDATE recordings SET status='compiled', skill_id=? WHERE id=?").run(skillId, id);
}

// --- shared workspace session ---------------------------------------------

/** Account-wide browser session blob, shared by every bot. */
export function saveWorkspaceSession(cookies: Buffer): void {
  prepare(openDb(),
      `INSERT INTO workspace_sessions (id, cookies_blob, updated_at)
       VALUES (1, ?, datetime('now'))
       ON CONFLICT(id) DO UPDATE SET cookies_blob = excluded.cookies_blob,
                                     updated_at   = excluded.updated_at`,
    )
    .run(cookies);
}

export function loadWorkspaceSession(): Buffer | undefined {
  const row = prepare(openDb(), 'SELECT cookies_blob FROM workspace_sessions WHERE id = 1').get() as
    | { cookies_blob: Uint8Array | null }
    | undefined;
  // node:sqlite hands back a Uint8Array, not a Buffer. Callers downstream (the
  // shared browser cookie jar) need real Buffer semantics, so convert here
  // rather than letting a Uint8Array masquerade as a Buffer.
  return row?.cookies_blob ? Buffer.from(row.cookies_blob) : undefined;
}
