import { randomUUID } from 'node:crypto';
import { EventEmitter } from 'node:events';
import { existsSync, readFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { openDb } from '../db/index.ts';
import { openSealed, writeSealed, type VaultFile } from './vault.ts';

/**
 * Saved logins: your own usernames and passwords, filled in for you after you
 * approve each use.
 *
 * Three rules shape everything in this file:
 *
 *   1. The model never sees a password. It asks for "the saved login, into
 *      these two fields"; the harness decrypts it and types it into the page
 *      itself. Nothing secret is ever in a tool result, a task log, a database
 *      row, or a request to a model provider.
 *
 *   2. A login only fills on the exact origin it was saved for. No subdomain
 *      matching, no "close enough". That is what stops a lookalike page — or a
 *      prompt-injected bot steered onto one — from receiving your password.
 *
 *   3. Every fill needs a yes from you, unless you said "always" for that one
 *      login. Silence is a no: an unanswered request expires and nothing fills.
 *
 * Stored in ~/.mybot/logins.enc with the same AES-256-GCM + scrypt sealing and
 * the same passphrase as the API-key vault.
 */

const DIR = process.env.MYBOT_HOME ?? join(homedir(), '.mybot');
const LOGINS_PATH = join(DIR, 'logins.enc');

export interface SavedLogin {
  id: string;
  /** Exact origin, e.g. https://github.com. */
  origin: string;
  username: string;
  password: string;
  label?: string;
  /** You chose "Always for this site" — fills without asking. */
  alwaysAllow: boolean;
  createdAt: string;
  lastUsedAt?: string;
}

/** Everything about a login except the password. The only shape that leaves this module. */
export type LoginSummary = Omit<SavedLogin, 'password'>;

interface LoginsFile {
  logins: SavedLogin[];
}

export function loginsPath(): string {
  return LOGINS_PATH;
}

export function loginsExist(): boolean {
  return existsSync(LOGINS_PATH);
}

function summarise(l: SavedLogin): LoginSummary {
  const { password: _password, ...rest } = l;
  return rest;
}

// ---------------------------------------------------------------------------
// Origins
// ---------------------------------------------------------------------------

const LOOPBACK = new Set(['localhost', '127.0.0.1', '[::1]']);

/**
 * Turn whatever the human typed ("github.com", "https://github.com/login?x=1")
 * into the origin a login is bound to.
 *
 * https only. Plain http is allowed for loopback alone, where nothing crosses a
 * network that could read it.
 */
export function normalizeOrigin(input: string): string {
  const raw = input.trim();
  if (!raw) throw new Error('Give the address of the sign-in page, e.g. https://github.com/login');
  const withScheme = /^[a-z][a-z0-9+.-]*:\/\//i.test(raw) ? raw : `https://${raw}`;

  let u: URL;
  try {
    u = new URL(withScheme);
  } catch {
    throw new Error(`"${input}" is not a web address.`);
  }

  if (u.protocol === 'https:') return u.origin;
  if (u.protocol === 'http:' && LOOPBACK.has(u.hostname)) return u.origin;
  throw new Error(`Saved logins only work on https pages (got ${u.protocol}//${u.host}).`);
}

/** The origin of a page the bot is on, or undefined if it is not a fillable page. */
export function pageOrigin(url: string): string | undefined {
  try {
    return normalizeOrigin(url);
  } catch {
    return undefined;
  }
}

// ---------------------------------------------------------------------------
// Unlocking
// ---------------------------------------------------------------------------

let passphrase: string | undefined;
let passphraseSource: (() => Promise<string | undefined>) | undefined;

/** Hold the passphrase in memory for this process. The console calls this once at start. */
export function unlock(pass: string | undefined): void {
  if (pass) passphrase = pass;
}

export function lock(): void {
  passphrase = undefined;
}

/**
 * Where to get the passphrase the first time a fill needs it — the CLI wires a
 * hidden terminal prompt here, so a run only asks if it actually signs in.
 */
export function setPassphraseSource(source: (() => Promise<string | undefined>) | undefined): void {
  passphraseSource = source;
}

export function isUnlocked(): boolean {
  return Boolean(passphrase ?? process.env.MYBOT_PASSPHRASE);
}

export async function ensureUnlocked(): Promise<boolean> {
  if (isUnlocked()) return true;
  if (!passphraseSource) return false;
  const p = await passphraseSource().catch(() => undefined);
  if (!p) return false;
  // Prove it before caching it: a mistyped passphrase should fail here, once,
  // not on every fill for the rest of the run.
  if (loginsExist()) readFile(p);
  passphrase = p;
  return true;
}

function currentPassphrase(explicit?: string): string {
  const p = explicit ?? passphrase ?? process.env.MYBOT_PASSPHRASE;
  if (!p) throw new Error('Saved logins are locked. Start MyBot with your vault passphrase.');
  return p;
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

function readFile(pass: string): LoginsFile {
  if (!loginsExist()) return { logins: [] };
  try {
    const sealed = JSON.parse(readFileSync(LOGINS_PATH, 'utf8')) as VaultFile;
    const parsed = openSealed<LoginsFile>(pass, sealed);
    return { logins: parsed.logins ?? [] };
  } catch {
    throw new Error('Could not unlock saved logins — wrong passphrase, or the file was modified.');
  }
}

function writeFile(pass: string, file: LoginsFile): void {
  writeSealed(LOGINS_PATH, pass, file);
}

export function listLogins(pass?: string): LoginSummary[] {
  return readFile(currentPassphrase(pass))
    .logins.map(summarise)
    .sort((a, b) => a.origin.localeCompare(b.origin) || a.username.localeCompare(b.username));
}

export interface AddLoginInput {
  url: string;
  username: string;
  password: string;
  label?: string;
}

/**
 * Save a login. Saving the same origin + username again replaces the password
 * (the usual reason to re-add one is that you changed it) and keeps the rest.
 */
export function addLogin(input: AddLoginInput, pass?: string): LoginSummary {
  const p = currentPassphrase(pass);
  const origin = normalizeOrigin(input.url);
  const username = input.username.trim();
  if (!username) throw new Error('A login needs a username or email.');
  if (!input.password) throw new Error('A login needs a password.');

  const file = readFile(p);
  const existing = file.logins.find((l) => l.origin === origin && l.username === username);
  if (existing) {
    existing.password = input.password;
    if (input.label !== undefined) existing.label = input.label.trim() || undefined;
    writeFile(p, file);
    return summarise(existing);
  }

  const login: SavedLogin = {
    id: randomUUID(),
    origin,
    username,
    password: input.password,
    label: input.label?.trim() || undefined,
    alwaysAllow: false,
    createdAt: new Date().toISOString(),
  };
  file.logins.push(login);
  writeFile(p, file);
  return summarise(login);
}

/** Remove by id, or by origin (every account on it), or by "origin username". */
export function removeLogin(selector: string, pass?: string): LoginSummary[] {
  const p = currentPassphrase(pass);
  const file = readFile(p);
  const match = matcher(selector);
  const removed = file.logins.filter(match);
  if (!removed.length) throw new Error(`No saved login matches "${selector}".`);
  file.logins = file.logins.filter((l) => !match(l));
  writeFile(p, file);
  return removed.map(summarise);
}

export function setAlwaysAllow(selector: string, always: boolean, pass?: string): LoginSummary[] {
  const p = currentPassphrase(pass);
  const file = readFile(p);
  const match = matcher(selector);
  const hit = file.logins.filter(match);
  if (!hit.length) throw new Error(`No saved login matches "${selector}".`);
  for (const l of hit) l.alwaysAllow = always;
  writeFile(p, file);
  return hit.map(summarise);
}

function matcher(selector: string): (l: SavedLogin) => boolean {
  const s = selector.trim();
  const [first, ...rest] = s.split(/\s+/);
  const user = rest.join(' ');
  let origin: string | undefined;
  try {
    origin = normalizeOrigin(first ?? '');
  } catch {
    origin = undefined;
  }
  return (l) => l.id === s || (origin !== undefined && l.origin === origin && (!user || l.username === user));
}

/** Logins for one exact origin, summaries only — what the bot may be told. */
export function loginsFor(origin: string, pass?: string): LoginSummary[] {
  return readFile(currentPassphrase(pass)).logins.filter((l) => l.origin === origin).map(summarise);
}

/**
 * The one place a password leaves the store, and only into the browser fill.
 * Never call this to build text for a model, a log or a response.
 */
export function revealForFill(id: string, origin: string, pass?: string): { username: string; password: string } {
  const p = currentPassphrase(pass);
  const file = readFile(p);
  const login = file.logins.find((l) => l.id === id);
  if (!login) throw new Error('That saved login no longer exists.');
  // Checked again here, not only by the caller: this function is the boundary.
  if (login.origin !== origin) throw new Error('Saved login does not belong to this site.');
  login.lastUsedAt = new Date().toISOString();
  writeFile(p, file);
  return { username: login.username, password: login.password };
}

// ---------------------------------------------------------------------------
// Approvals
// ---------------------------------------------------------------------------

export type LoginRequestStatus = 'pending' | 'allowed' | 'denied' | 'expired';

export interface LoginRequest {
  id: string;
  task_id: string | null;
  bot: string;
  origin: string;
  username: string;
  status: LoginRequestStatus;
  always: number;
  decided_by: 'human' | 'rule' | 'timeout' | null;
  created_at: string;
  decided_at: string | null;
}

/**
 * `requested` / `settled` for anything rendering the queue (the console, the
 * CLI prompt). Settling in-process also wakes the waiting fill immediately
 * instead of on the next poll tick.
 */
export const events = new EventEmitter();

export const DEFAULT_APPROVAL_WAIT_MS = 3 * 60_000;
const POLL_MS = 500;

export function getRequest(id: string): LoginRequest | undefined {
  return openDb().prepare('SELECT * FROM login_requests WHERE id = ?').get(id) as unknown as
    | LoginRequest
    | undefined;
}

export function pendingRequests(): LoginRequest[] {
  return openDb()
    .prepare("SELECT * FROM login_requests WHERE status='pending' ORDER BY created_at, rowid")
    .all() as unknown as LoginRequest[];
}

export function recentRequests(limit = 50): LoginRequest[] {
  return openDb()
    .prepare('SELECT * FROM login_requests ORDER BY created_at DESC, rowid DESC LIMIT ?')
    .all(limit) as unknown as LoginRequest[];
}

function settle(id: string, status: LoginRequestStatus, by: LoginRequest['decided_by'], always = false): boolean {
  const changed = openDb()
    .prepare(
      "UPDATE login_requests SET status=?, decided_by=?, always=?, decided_at=datetime('now') WHERE id=? AND status='pending'",
    )
    .run(status, by, always ? 1 : 0, id);
  const req = getRequest(id);
  if (req && Number(changed.changes) > 0) events.emit('settled', req);
  return Number(changed.changes) > 0;
}

/**
 * You said yes. With `always`, this login fills on its site from now on without
 * asking — which needs the store unlocked in whichever process you answered in.
 */
export function allow(id: string, opts: { always?: boolean; passphrase?: string } = {}): LoginRequest {
  const req = getRequest(id);
  if (!req) throw new Error(`No sign-in request ${id}.`);
  if (req.status !== 'pending') throw new Error(`That request was already ${req.status}.`);
  if (opts.always) {
    const pass = currentPassphrase(opts.passphrase);
    const file = readFile(pass);
    const login = file.logins.find((l) => l.origin === req.origin && l.username === req.username);
    if (login) {
      login.alwaysAllow = true;
      writeFile(pass, file);
    }
  }
  settle(id, 'allowed', 'human', Boolean(opts.always));
  return getRequest(id)!;
}

export function deny(id: string): LoginRequest {
  const req = getRequest(id);
  if (!req) throw new Error(`No sign-in request ${id}.`);
  if (req.status !== 'pending') throw new Error(`That request was already ${req.status}.`);
  settle(id, 'denied', 'human');
  return getRequest(id)!;
}

export interface ApprovalInput {
  bot: string;
  taskId?: string;
  login: LoginSummary;
}

export interface ApprovalOptions {
  waitMs?: number;
  signal?: AbortSignal;
}

export type ApprovalOutcome =
  | { allowed: true; by: 'human' | 'rule'; requestId: string }
  | { allowed: false; by: 'human' | 'timeout' | 'cancelled'; requestId: string };

/**
 * Ask the human, and wait for the answer.
 *
 * Every request leaves a row, including the ones an "always" rule answered, so
 * there is an audit trail of every time a saved password was used. The row
 * holds the site and the username — never the password.
 */
export async function requestApproval(input: ApprovalInput, opts: ApprovalOptions = {}): Promise<ApprovalOutcome> {
  const id = randomUUID();
  const insert = openDb().prepare(
    'INSERT INTO login_requests (id, task_id, bot, origin, username) VALUES (?, ?, ?, ?, ?)',
  );
  try {
    insert.run(id, input.taskId ?? null, input.bot, input.login.origin, input.login.username);
  } catch {
    // An ad-hoc run's id with no tasks row trips the foreign key. The audit row
    // matters more than the link, so keep it unlinked rather than not asking.
    insert.run(id, null, input.bot, input.login.origin, input.login.username);
  }

  if (input.login.alwaysAllow) {
    settle(id, 'allowed', 'rule', true);
    return { allowed: true, by: 'rule', requestId: id };
  }

  events.emit('requested', getRequest(id)!);

  const waitMs = opts.waitMs ?? DEFAULT_APPROVAL_WAIT_MS;
  const outcome = await new Promise<ApprovalOutcome>((resolve) => {
    let done = false;
    const finish = (o: ApprovalOutcome) => {
      if (done) return;
      done = true;
      clearInterval(poll);
      clearTimeout(timer);
      events.off('settled', onSettled);
      opts.signal?.removeEventListener('abort', onAbort);
      resolve(o);
    };

    const fromRow = (row: LoginRequest | undefined) => {
      if (!row || row.status === 'pending') return;
      if (row.status === 'allowed') finish({ allowed: true, by: row.decided_by === 'rule' ? 'rule' : 'human', requestId: id });
      else finish({ allowed: false, by: row.status === 'expired' ? 'timeout' : 'human', requestId: id });
    };

    const onSettled = (row: LoginRequest) => {
      if (row.id === id) fromRow(row);
    };
    const onAbort = () => {
      // Finish first: settling emits 'settled', which would otherwise resolve
      // this as a plain "the human said no".
      finish({ allowed: false, by: 'cancelled', requestId: id });
      settle(id, 'denied', 'human');
    };

    // Polled as well as evented: the answer can come from another process
    // (`mybot logins allow` in a second terminal), which only the row sees.
    const poll = setInterval(() => {
      try {
        fromRow(getRequest(id));
      } catch {
        /* a transient SQLITE_BUSY is not an answer */
      }
    }, POLL_MS);

    const timer = setTimeout(() => {
      settle(id, 'expired', 'timeout');
      finish({ allowed: false, by: 'timeout', requestId: id });
    }, waitMs);

    events.on('settled', onSettled);
    opts.signal?.addEventListener('abort', onAbort, { once: true });
  });

  // However it ended — here, in another process, or by timeout — anything still
  // showing this request (a terminal prompt) should stop asking.
  events.emit('closed', id);
  return outcome;
}
