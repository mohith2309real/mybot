import { createServer, type IncomingMessage, type Server, type ServerResponse } from 'node:http';
import { randomBytes, timingSafeEqual } from 'node:crypto';
import { networkInterfaces } from 'node:os';
import { createReadStream } from 'node:fs';
import { stat } from 'node:fs/promises';
import { dirname, extname, join, normalize, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  createBot,
  createSkill,
  createTask,
  deleteBot,
  deleteSkill,
  appendTaskLog,
  getBot,
  getRoutine,
  getSkill,
  getTask,
  listBots,
  listPausedTasks,
  listRoutineRuns,
  listRoutines,
  listSkills,
  listTasks,
  readTaskLog,
  resumeTask,
  setRoutineEnabled,
  setTaskStatus,
  updateSkill,
  type Bot,
  type LogEntry,
  type Task,
} from '../db/index.ts';
import { isProviderName, getProvider } from '../providers/index.ts';
import * as computer from '../computer/container.ts';
import * as liveview from '../computer/liveview.ts';
import { runTask } from '../agent/loop.ts';
import * as remote from './remote.ts';
import { bridge } from './wsproxy.ts';

/**
 * The dashboard: a localhost operations console for the account's computer.
 *
 * Node stdlib only. The client is plain HTML/CSS/JS served out of ./public, so
 * there is no build step and nothing to fetch from a CDN — a console that stops
 * working when the wifi drops is not a console.
 *
 * Bound to 127.0.0.1 on purpose. This surface can run shell commands inside the
 * container and drive a logged-in browser; it is not something to put on a LAN.
 */

const HERE = dirname(fileURLToPath(import.meta.url));
const PUBLIC_DIR = join(HERE, 'public');

export const DEFAULT_PORT = 7717;

/** Loopback unless `lan` was requested; set in start(). */
let bindHost = '127.0.0.1';
/** Set only when bound beyond loopback. */
let pairingToken: string | undefined;

/** First non-internal IPv4 address, for the URL a phone should open. */
function lanAddress(): string {
  for (const addrs of Object.values(networkInterfaces())) {
    for (const a of addrs ?? []) {
      if (a.family === 'IPv4' && !a.internal) return a.address;
    }
  }
  return '127.0.0.1';
}

/**
 * Gate every request when bound beyond loopback.
 *
 * The token may arrive as `?t=` (the link you open on a phone) or as a bearer
 * header (what the page uses afterwards). Compared with `timingSafeEqual` — a
 * naive `===` leaks the token a character at a time to anyone who can time
 * responses, which on a shared wifi is everyone.
 */
function authorised(req: IncomingMessage, url: URL, requireToken: boolean): boolean {
  // Loopback is gated by the OS, not by a token — the desktop app opens the
  // console with no credentials and must keep working when a phone pairs.
  if (!requireToken || !pairingToken) return true;

  const supplied =
    url.searchParams.get('t') ??
    (req.headers.authorization ?? '').replace(/^Bearer\s+/i, '') ??
    '';

  const a = Buffer.from(String(supplied));
  const b = Buffer.from(pairingToken);
  return a.length === b.length && timingSafeEqual(a, b);
}

export interface StartOptions {
  /** First port to try. Falls forward if it is taken. */
  port?: number;
  /**
   * Vault passphrase, if the CLI already prompted for one. The browser never
   * asks for it: a passphrase typed into a web form is a passphrase in a form.
   */
  passphrase?: string;
  /**
   * Bind 0.0.0.0 so a phone on the same wifi can reach this.
   *
   * Off by default, and gated behind a pairing token when on. This dashboard
   * can run shell commands in a container and drive a logged-in browser, so an
   * unauthenticated port on a shared network hands all of that to anyone else
   * on the wifi. The token is the difference between "my phone" and "the cafe".
   */
  lan?: boolean;
  /** Pairing token. Generated when `lan` is set and none was supplied. */
  token?: string;
}

export interface DashboardHandle {
  url: string;
  port: number;
  host: string;
  /** Set only when bound beyond loopback. */
  token?: string;
  server: Server;
  close(): Promise<void>;
}

// --- live event bus --------------------------------------------------------

interface StreamClient {
  res: ServerResponse;
  /** Set when the client only wants events for one task. */
  taskId?: string;
}

const clients = new Set<StreamClient>();

/** Events a running task produces, kept in memory so late subscribers catch up. */
const activeRuns = new Map<string, { botId: string; startedAt: number; abort: AbortController }>();

function sseWrite(res: ServerResponse, event: string, data: unknown): void {
  // No trailing newline games: SSE frames end with a blank line, and node's
  // http writes straight to the socket when nothing is compressing it.
  res.write(`event: ${event}\ndata: ${JSON.stringify(data)}\n\n`);
}

function publish(event: string, data: unknown, taskId?: string): void {
  for (const client of clients) {
    if (client.taskId && client.taskId !== taskId) continue;
    try {
      sseWrite(client.res, event, data);
    } catch {
      clients.delete(client);
    }
  }
}

// --- task execution --------------------------------------------------------

let vaultPassphrase: string | undefined;

function botView(bot: Bot): Record<string, unknown> {
  const tasks = listTasks(bot.id);
  const current = tasks.find((t) => t.status === 'running' || t.status === 'paused');
  const last = tasks[0];
  return {
    ...bot,
    taskCount: tasks.length,
    currentTask: current ? taskView(current) : null,
    lastTask: last ? taskView(last) : null,
  };
}

function taskView(task: Task): Record<string, unknown> {
  return {
    id: task.id,
    bot_id: task.bot_id,
    instruction: task.instruction,
    status: task.status,
    started_at: task.started_at,
    completed_at: task.completed_at,
    paused_reason: (task as unknown as { paused_reason?: string | null }).paused_reason ?? null,
    paused_at: (task as unknown as { paused_at?: string | null }).paused_at ?? null,
  };
}

function pushTask(id: string): void {
  const t = getTask(id);
  if (t) publish('task', taskView(t), id);
}

/**
 * Kick off an agentic run and fan its events out over SSE.
 *
 * Deliberately not awaited by the request handler — the POST returns the task id
 * immediately and the browser follows the stream, so a twenty-minute run does
 * not sit on an open request.
 */
/**
 * Recent turns with this bot, replayed as conversation context.
 *
 * Only the human's instruction and the bot's own answer are carried forward —
 * not the tool ledger. Replaying every page_read would blow the context window
 * on a couple of research runs, and the answer is what a follow-up refers to.
 */
function historyFor(botId: string, limit = 6): { role: 'user' | 'assistant'; content: string }[] {
  const turns: { role: 'user' | 'assistant'; content: string }[] = [];

  for (const task of listTasks(botId).slice(0, limit).reverse()) {
    turns.push({ role: 'user', content: task.instruction });
    const log = readTaskLog(task.id);
    const answer = [...log].reverse().find((e) => e.kind === 'done' || e.kind === 'assistant');
    if (answer?.text) turns.push({ role: 'assistant', content: answer.text });
  }

  return turns;
}

function launchTask(bot: Bot, instruction: string): Task {
  const history = historyFor(bot.id);
  const task = createTask(bot.id, instruction);
  const abort = new AbortController();
  activeRuns.set(task.id, { botId: bot.id, startedAt: Date.now(), abort });
  publish('task', taskView(task), task.id);

  const emit = (kind: string, text: string) => {
    publish('log', { taskId: task.id, kind, text, at: new Date().toISOString() }, task.id);
  };

  // The persona is prepended to the instruction rather than replacing the loop's
  // system prompt: the loop's boundaries (no passwords, no purchases) are not a
  // bot's to override.
  const briefed = bot.system ? `${bot.system.trim()}\n\n---\n\nTask: ${instruction}` : instruction;

  (async () => {
    try {
      // Fast when warm; on a cold start this is where the wait happens, and the
      // thread says so rather than sitting silent.
      const st = await computer.status();
      if (st.container !== 'running' || !st.viewUrl) {
        emit('tool_result', 'Starting the computer…');
        await computer.up();
      }

      const provider = getProvider(bot.provider, vaultPassphrase);
      const result = await runTask({
        provider,
        model: bot.model,
        instruction: briefed,
        history,
        signal: abort.signal,
        taskId: task.id,
        onEvent: emit,
      });
      // The row is the durable record and nothing downstream re-derives it, so
      // a run that ends any way other than 'done' must still land a final
      // status here — otherwise it stays 'running' for ever and the roster
      // shows a bot working on nothing.
      if (getTask(task.id)?.status === 'running') {
        setTaskStatus(task.id, result.status === 'done' ? 'done' : 'failed');
      }
      publish('result', { taskId: task.id, ...result }, task.id);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      // Persist it, not just stream it. Anything that fails BEFORE the agent
      // loop starts — a missing key, a container that will not come up — never
      // reached appendTaskLog, so the task showed as "failed" with an entirely
      // empty thread and the reason existed only for whoever happened to be
      // watching the live stream at that second.
      appendTaskLog(task.id, 'error', message);
      emit('error', message);
      setTaskStatus(task.id, 'failed');
      publish('result', { taskId: task.id, status: 'error', summary: message }, task.id);
    } finally {
      activeRuns.delete(task.id);
      pushTask(task.id);
      publish('bots', listBots().map(botView));
    }
  })();

  return task;
}

// --- routine event passthrough --------------------------------------------

type FireEvent = (name: string, payload?: unknown) => unknown;
let fireEventFn: FireEvent | null | undefined;

/**
 * The routine engine lands in a sibling module owned by another part of the
 * build. Resolve it lazily through a computed specifier so a missing file is a
 * runtime 503 here rather than a hard import failure of the whole dashboard.
 */
async function loadFireEvent(): Promise<FireEvent | null> {
  if (fireEventFn !== undefined) return fireEventFn;
  try {
    const spec = ['..', 'routines', 'index.ts'].join('/');
    const mod: Record<string, unknown> = await import(spec);
    const fn = mod.fireEvent;
    fireEventFn = typeof fn === 'function' ? (fn as FireEvent) : null;
  } catch {
    fireEventFn = null;
  }
  return fireEventFn;
}

// --- http plumbing ---------------------------------------------------------

function sendJson(res: ServerResponse, status: number, body: unknown): void {
  const payload = JSON.stringify(body);
  res.writeHead(status, {
    'content-type': 'application/json; charset=utf-8',
    'content-length': Buffer.byteLength(payload),
    'cache-control': 'no-store',
  });
  res.end(payload);
}

const MAX_BODY = 1_000_000;

function readBody(req: IncomingMessage): Promise<unknown> {
  return new Promise((resolve, reject) => {
    let size = 0;
    const chunks: Buffer[] = [];
    req.on('data', (chunk: Buffer) => {
      size += chunk.length;
      if (size > MAX_BODY) {
        reject(new Error('Request body too large.'));
        req.destroy();
        return;
      }
      chunks.push(chunk);
    });
    req.on('end', () => {
      const raw = Buffer.concat(chunks).toString('utf8');
      if (!raw.trim()) return resolve({});
      try {
        resolve(JSON.parse(raw));
      } catch {
        reject(new Error('Body was not valid JSON.'));
      }
    });
    req.on('error', reject);
  });
}

const MIME: Record<string, string> = {
  '.html': 'text/html; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.woff2': 'font/woff2',
  '.png': 'image/png',
  '.ico': 'image/x-icon',
};

/**
 * Serve one file from ./public.
 *
 * Traversal is rejected by resolving the normalised path and checking it is
 * still inside PUBLIC_DIR — before anything touches the filesystem, so a crafted
 * path never becomes a stat() on /etc.
 */
async function serveStatic(pathname: string, res: ServerResponse): Promise<void> {
  const decoded = (() => {
    try {
      return decodeURIComponent(pathname);
    } catch {
      return '';
    }
  })();

  if (!decoded || decoded.includes('\0')) return notFound(res);

  const rel = decoded === '/' ? 'index.html' : decoded.replace(/^\/+/, '');
  const target = normalize(join(PUBLIC_DIR, rel));
  if (target !== PUBLIC_DIR && !target.startsWith(PUBLIC_DIR + sep)) return notFound(res);

  try {
    const info = await stat(target);
    if (!info.isFile()) return notFound(res);
    res.writeHead(200, {
      'content-type': MIME[extname(target)] ?? 'application/octet-stream',
      'content-length': info.size,
      // no-store, not no-cache. `no-cache` still permits a cached copy to be
      // reused when the response carries no validator, which it does not here —
      // so fixes to app.js kept not reaching an already-open window, and the
      // symptom looked like the bug had not been fixed at all.
      'cache-control': 'no-store, must-revalidate',
    });
    createReadStream(target).pipe(res);
  } catch {
    notFound(res);
  }
}

function notFound(res: ServerResponse): void {
  res.writeHead(404, { 'content-type': 'text/plain; charset=utf-8' });
  res.end('Not found');
}

function str(v: unknown): string {
  return typeof v === 'string' ? v.trim() : '';
}

// --- routes ----------------------------------------------------------------

async function handleApi(
  req: IncomingMessage,
  res: ServerResponse,
  url: URL,
): Promise<boolean> {
  const method = req.method ?? 'GET';
  const path = url.pathname;
  const seg = path.split('/').filter(Boolean); // ['api', 'tasks', ':id', ...]

  // GET /api/state — everything the console needs to paint its first frame.
  if (method === 'GET' && path === '/api/state') {
    const status = await computer.status();
    sendJson(res, 200, {
      bots: listBots().map(botView),
      paused: listPausedTasks().map(taskView),
      computer: status,
      running: [...activeRuns.keys()],
      counts: {
        tasks: listTasks().length,
        skills: listSkills().length,
        routines: listRoutines().length,
      },
    });
    return true;
  }

  // --- bots ----------------------------------------------------------------

  if (method === 'GET' && path === '/api/bots') {
    sendJson(res, 200, listBots().map(botView));
    return true;
  }

  if (method === 'POST' && path === '/api/bots') {
    const body = (await readBody(req)) as Record<string, unknown>;
    const name = str(body.name);
    const provider = str(body.provider);
    const model = str(body.model);
    const system = str(body.system);
    if (!name) return bad(res, 'A bot needs a name.'), true;
    if (!isProviderName(provider)) return bad(res, `Unknown provider "${provider}".`), true;
    if (!model) return bad(res, 'A bot needs a model.'), true;
    try {
      const bot = createBot({ name, provider, model, system: system || undefined });
      publish('bots', listBots().map(botView));
      sendJson(res, 201, botView(bot));
    } catch (err) {
      bad(res, err instanceof Error ? err.message : String(err));
    }
    return true;
  }

  if (method === 'DELETE' && seg[0] === 'api' && seg[1] === 'bots' && seg[2]) {
    deleteBot(seg[2]);
    publish('bots', listBots().map(botView));
    sendJson(res, 200, { ok: true });
    return true;
  }

  // --- tasks ---------------------------------------------------------------

  if (method === 'GET' && path === '/api/tasks') {
    const botId = url.searchParams.get('bot') ?? undefined;
    const rows = listTasks(botId || undefined).map((t) => {
      const bot = getBot(t.bot_id);
      return { ...taskView(t), botName: bot?.name ?? '(deleted bot)' };
    });
    sendJson(res, 200, rows);
    return true;
  }

  if (method === 'POST' && path === '/api/tasks') {
    const body = (await readBody(req)) as Record<string, unknown>;
    const botId = str(body.botId);
    const instruction = str(body.instruction);
    const bot = botId ? getBot(botId) : undefined;
    if (!bot) return bad(res, 'Pick a bot to run this.'), true;
    if (!instruction) return bad(res, 'Describe what the bot should do.'), true;

    // The computer is started by the run itself rather than being a
    // precondition. Refusing the message and telling someone to go press a
    // different button first is a worse product than just doing it — and the
    // start is idempotent and fast once the container is warm.
    const task = launchTask(bot, instruction);
    sendJson(res, 202, taskView(task));
    return true;
  }

  if (method === 'GET' && seg[0] === 'api' && seg[1] === 'tasks' && seg[2] && !seg[3]) {
    const task = getTask(seg[2]);
    if (!task) return notFound(res), true;
    const bot = getBot(task.bot_id);
    sendJson(res, 200, {
      ...taskView(task),
      botName: bot?.name ?? '(deleted bot)',
      log: readTaskLog(task.id),
    });
    return true;
  }

  // Stop a run in flight. The loop checks the signal between iterations, so a
  // tool call already in progress finishes rather than being torn out mid-write.
  /*
   * Your Mac's screen. Not the bots' — theirs live in containers, and nothing
   * in the agent's tool surface reaches this. Reachable from the phone link
   * because it goes through this same authorised console.
   */
  if (seg[1] === 'remote') {
    if (method === 'GET') {
      const st = await remote.status();
      sendJson(res, 200, { ...st, secret: st.enabled ? await remote.viewerSecret() : undefined });
      return true;
    }
    if (method === 'POST') {
      const result = await remote.enable();
      sendJson(res, result.ok ? 200 : 409, {
        ...result.status,
        error: result.error,
        secret: result.ok ? await remote.viewerSecret() : undefined,
      });
      return true;
    }
    if (method === 'DELETE') {
      const result = await remote.disable();
      sendJson(res, 200, { ...result.status, error: result.error });
      return true;
    }
  }

  // Pairing is loopback-only on purpose: a paired phone must not be able to
  // pair another device, or un-pair itself into a permanent hole.
  if (seg[1] === 'pair') {
    if (method === 'GET') {
      sendJson(res, 200, pairStatus());
      return true;
    }
    if (method === 'POST') {
      const result = await pair();
      sendJson(res, result.error ? 409 : 200, result);
      return true;
    }
    if (method === 'DELETE') {
      await unpair();
      sendJson(res, 200, { paired: false });
      return true;
    }
  }

  if (method === 'POST' && seg[1] === 'tasks' && seg[2] && seg[3] === 'stop') {
    const run = activeRuns.get(seg[2]);
    if (run) {
      run.abort.abort();
      sendJson(res, 202, { stopping: true });
      return true;
    }
    // No live run, but the row can still be sitting in 'running' or 'paused'
    // from a console that was killed. Those are exactly the rows a person most
    // wants to clear, and refusing them left a bot stuck "Needs you" for ever
    // with no button anywhere that could release it.
    const orphan = getTask(seg[2]);
    if (!orphan || (orphan.status !== 'running' && orphan.status !== 'paused')) {
      sendJson(res, 409, { error: 'That task is not running.' });
      return true;
    }
    setTaskStatus(orphan.id, 'failed');
    appendTaskLog(orphan.id, 'error', 'Stopped — nothing was still running this task.');
    pushTask(orphan.id);
    publish('paused', listPausedTasks().map(taskView));
    publish('bots', listBots().map(botView));
    sendJson(res, 200, taskView(getTask(orphan.id)!));
    return true;
  }

  // Live model list for the provider, so creating a bot is a choice rather than
  // a spelling test. Model ids churn faster than any table worth hardcoding.
  if (method === 'GET' && seg[1] === 'models' && seg[2]) {
    if (!isProviderName(seg[2])) {
      sendJson(res, 400, { error: `Unknown provider ${seg[2]}` });
      return true;
    }
    try {
      const models = await getProvider(seg[2], vaultPassphrase).listModels();
      sendJson(res, 200, { models });
    } catch (err) {
      // No key configured is the normal case, not an error worth a 500.
      sendJson(res, 200, { models: [], error: err instanceof Error ? err.message : String(err) });
    }
    return true;
  }

  // Files the bot produced. Saving a report into /workspace is useless if the
  // only way to read it is `docker exec`.
  if (method === 'GET' && seg[1] === 'files') {
    const wanted = url.searchParams.get('path');
    try {
      if (!wanted) {
        // Addressed by conversation rather than the ambient session: the
        // dashboard can be asked for files before anything has opened one.
        const ls = await computer.exec(
          computer.DEFAULT_CONVERSATION,
          'find /workspace -maxdepth 3 -type f -not -path "*/.*" -printf "%s\\t%p\\n" 2>/dev/null | head -100',
        );
        const files = ls.stdout
          .split('\n')
          .filter(Boolean)
          .map((line) => {
            const [size, path] = line.split('\t');
            return { path, bytes: Number(size) || 0 };
          });
        sendJson(res, 200, { files });
        return true;
      }

      // The path comes from the browser; keep it inside /workspace.
      if (wanted.includes('..') || !wanted.startsWith('/workspace/')) {
        sendJson(res, 400, { error: 'Only /workspace files can be read.' });
        return true;
      }
      const out = await computer.exec(computer.DEFAULT_CONVERSATION, `cat ${JSON.stringify(wanted)}`);
      if (out.code !== 0) {
        sendJson(res, 404, { error: out.stderr.trim() || 'not found' });
        return true;
      }
      sendJson(res, 200, { path: wanted, content: out.stdout });
    } catch (err) {
      sendJson(res, 500, { error: err instanceof Error ? err.message : String(err) });
    }
    return true;
  }

  if (method === 'POST' && seg[1] === 'tasks' && seg[2] && seg[3] === 'resume') {
    const task = getTask(seg[2]);
    if (!task) return notFound(res), true;
    if (task.status !== 'paused') return bad(res, 'That task is not paused.'), true;
    const body = (await readBody(req).catch(() => ({}))) as Record<string, unknown>;
    const skipped = body.skipped === true;
    // Going through liveview rather than the row directly: the waiting run is
    // in this process listening for the event, and it is the only path that
    // can carry "the human declined" as distinct from "the human did it".
    liveview.resume(task.id, { skipped });
    publish('log', {
      taskId: task.id,
      kind: 'handoff',
      text: skipped
        ? 'Human skipped the blocked step. The bot continues without it.'
        : 'Human completed the blocked step. Control handed back to the bot.',
      at: new Date().toISOString(),
    }, task.id);
    pushTask(task.id);
    publish('paused', listPausedTasks().map(taskView));
    sendJson(res, 200, taskView(getTask(task.id)!));
    return true;
  }

  // --- live streams --------------------------------------------------------

  if (method === 'GET' && path === '/api/stream') {
    openStream(req, res);
    return true;
  }

  if (method === 'GET' && seg[1] === 'tasks' && seg[2] && seg[3] === 'stream') {
    const task = getTask(seg[2]);
    if (!task) return notFound(res), true;
    openStream(req, res, task.id);
    // Replay what already happened, then keep the socket open for the rest. A
    // reload mid-run therefore rejoins the same log rather than starting blank.
    for (const entry of readTaskLog(task.id) as LogEntry[]) {
      sseWrite(res, 'log', { taskId: task.id, ...entry });
    }
    sseWrite(res, 'replayed', { taskId: task.id });
    sseWrite(res, 'task', taskView(task));
    return true;
  }

  // --- the computer --------------------------------------------------------

  if (method === 'GET' && path === '/api/computer') {
    sendJson(res, 200, await computer.status());
    return true;
  }

  if (method === 'POST' && path === '/api/computer/up') {
    sendJson(res, 202, { starting: true });
    computer
      .up((line) => publish('computer_log', { line }))
      .then(async () => publish('computer', await computer.status()))
      .catch(async (err: unknown) => {
        publish('computer_log', { line: err instanceof Error ? err.message : String(err), error: true });
        publish('computer', await computer.status());
      });
    return true;
  }

  // --- skills --------------------------------------------------------------

  if (method === 'GET' && path === '/api/skills') {
    sendJson(res, 200, listSkills());
    return true;
  }

  if (method === 'POST' && path === '/api/skills') {
    const body = (await readBody(req)) as Record<string, unknown>;
    const name = str(body.name);
    const instructions = str(body.instructions);
    const safetyRules = str(body.safetyRules);
    if (!name) return bad(res, 'A skill needs a name.'), true;
    if (!instructions) return bad(res, 'A skill needs instructions.'), true;
    try {
      // No `source: 'remote'` exists, and no import-from-URL will be added:
      // skills are written here or taught here, never fetched.
      sendJson(res, 201, createSkill({
        name,
        instructions,
        source: 'manual',
        safetyRules: safetyRules || undefined,
      }));
    } catch (err) {
      bad(res, err instanceof Error ? err.message : String(err));
    }
    return true;
  }

  if (seg[1] === 'skills' && seg[2]) {
    const skill = getSkill(seg[2]);
    if (!skill) return notFound(res), true;
    if (method === 'GET') {
      sendJson(res, 200, skill);
      return true;
    }
    if (method === 'PUT') {
      const body = (await readBody(req)) as Record<string, unknown>;
      updateSkill(skill.id, {
        instructions: str(body.instructions) || skill.instructions,
        safetyRules: str(body.safetyRules) || null,
      });
      sendJson(res, 200, getSkill(skill.id));
      return true;
    }
    if (method === 'DELETE') {
      deleteSkill(skill.id);
      sendJson(res, 200, { ok: true });
      return true;
    }
  }

  // --- routines ------------------------------------------------------------

  if (method === 'GET' && path === '/api/routines') {
    const rows = listRoutines().map((r) => ({
      ...r,
      skillName: getSkill(r.skill_id)?.name ?? '(deleted skill)',
      botName: r.bot_id ? (getBot(r.bot_id)?.name ?? '(deleted bot)') : null,
      runs: listRoutineRuns(r.id, 5),
    }));
    sendJson(res, 200, rows);
    return true;
  }

  if (method === 'POST' && seg[1] === 'routines' && seg[2] && seg[3] === 'enabled') {
    const routine = getRoutine(seg[2]);
    if (!routine) return notFound(res), true;
    const body = (await readBody(req)) as Record<string, unknown>;
    setRoutineEnabled(routine.id, Boolean(body.enabled));
    sendJson(res, 200, getRoutine(routine.id));
    return true;
  }

  // --- webhook: routine event triggers -------------------------------------

  if (method === 'POST' && seg[1] === 'events' && seg[2]) {
    const name = decodeURIComponent(seg[2]);
    const payload = await readBody(req).catch(() => ({}));
    const fire = await loadFireEvent();
    if (!fire) {
      sendJson(res, 503, {
        error: 'The routine engine is not available, so this event went nowhere.',
        event: name,
      });
      return true;
    }
    try {
      const fired = await fire(name, payload);
      sendJson(res, 202, { event: name, fired: fired ?? null });
    } catch (err) {
      sendJson(res, 500, { error: err instanceof Error ? err.message : String(err), event: name });
    }
    return true;
  }

  return false;
}

function bad(res: ServerResponse, message: string, status = 400): void {
  sendJson(res, status, { error: message });
}

function openStream(req: IncomingMessage, res: ServerResponse, taskId?: string): StreamClient {
  res.writeHead(200, {
    'content-type': 'text/event-stream; charset=utf-8',
    'cache-control': 'no-cache, no-transform',
    connection: 'keep-alive',
    // Belt and braces for anyone who ever puts a proxy in front of this.
    'x-accel-buffering': 'no',
  });
  // Defeat any downstream buffer that waits for a fuller first packet.
  res.write(': open\n\n');

  const client: StreamClient = { res, taskId };
  clients.add(client);

  const keepAlive = setInterval(() => {
    try {
      res.write(': ping\n\n');
    } catch {
      clearInterval(keepAlive);
      clients.delete(client);
    }
  }, 20_000);

  const close = () => {
    clearInterval(keepAlive);
    clients.delete(client);
  };
  req.on('close', close);
  res.on('close', close);
  return client;
}

// --- server ----------------------------------------------------------------

function listen(server: Server, port: number, attemptsLeft: number): Promise<number> {
  return new Promise((resolve, reject) => {
    const onError = (err: NodeJS.ErrnoException) => {
      server.removeListener('listening', onListening);
      if (err.code === 'EADDRINUSE' && attemptsLeft > 0) {
        resolve(listen(server, port + 1, attemptsLeft - 1));
      } else {
        reject(err);
      }
    };
    const onListening = () => {
      server.removeListener('error', onError);
      resolve(port);
    };
    server.once('error', onError);
    server.once('listening', onListening);
    server.listen(port, bindHost);
  });
}

export async function start(opts: StartOptions = {}): Promise<DashboardHandle> {
  vaultPassphrase = opts.passphrase;

  // Loopback unless explicitly opened up, and never open without a token.
  bindHost = opts.lan ? '0.0.0.0' : '127.0.0.1';
  pairingToken = opts.lan ? (opts.token ?? randomBytes(16).toString('hex')) : undefined;

  const server = createServer(makeHandler(Boolean(opts.lan)));
  attachRemoteSocket(server, Boolean(opts.lan));

  // A live view plus a long agent run means sockets that idle for minutes.
  server.keepAliveTimeout = 0;
  server.headersTimeout = 0;
  server.requestTimeout = 0;

  return await finishStart(server, opts);
}

// --- phone pairing ---------------------------------------------------------

/**
 * Letting a phone in, on demand.
 *
 * Off until asked for, because this console runs shell commands in a container
 * and drives a logged-in browser — an open port on cafe wifi hands all of that
 * to whoever else is on it. When it is on, the LAN listener binds the specific
 * interface address rather than 0.0.0.0: the loopback listener already holds
 * this port, and two wildcard binds on one port is an EADDRINUSE.
 *
 * The token is only demanded of the LAN listener. The desktop app talks to
 * loopback with no credentials and must keep working while a phone is paired.
 */
let lanServer: Server | undefined;
let consolePort = DEFAULT_PORT;

function pairStatus(): { paired: boolean; url?: string; address?: string } {
  if (!lanServer || !pairingToken) return { paired: false };
  const address = lanAddress();
  return {
    paired: true,
    address,
    url: `http://${address}:${consolePort}/?t=${pairingToken}`,
  };
}

async function pair(): Promise<{ paired: boolean; url?: string; address?: string; error?: string }> {
  if (lanServer) return pairStatus();

  const address = lanAddress();
  if (address === '127.0.0.1') {
    return { paired: false, error: 'This Mac is not on a network, so there is nothing for a phone to reach.' };
  }

  pairingToken = randomBytes(16).toString('hex');
  const server = createServer(makeHandler(true));
  attachRemoteSocket(server, true);
  server.keepAliveTimeout = 0;
  server.headersTimeout = 0;
  server.requestTimeout = 0;

  try {
    await new Promise<void>((resolve, reject) => {
      server.once('error', reject);
      server.listen(consolePort, address, () => {
        server.removeListener('error', reject);
        resolve();
      });
    });
  } catch (err) {
    pairingToken = undefined;
    return { paired: false, error: err instanceof Error ? err.message : String(err) };
  }

  lanServer = server;
  return pairStatus();
}

async function unpair(): Promise<void> {
  const server = lanServer;
  lanServer = undefined;
  pairingToken = undefined;
  if (!server) return;
  await new Promise<void>((resolve) => server.close(() => resolve()));
}

/**
 * The screen-sharing socket.
 *
 * Authorised exactly like every other request on this listener, which is what
 * lets the phone link reach it without a second credential. A failed check
 * closes the socket rather than answering, because a half-open upgrade is
 * indistinguishable to a client from a server that is merely slow.
 */
function attachRemoteSocket(server: Server, requireToken: boolean): void {
  server.on('upgrade', (req, socket, _head) => {
    const url = new URL(req.url ?? '/', `http://${bindHost}`);
    if (url.pathname !== '/api/remote/socket' || !authorised(req, url, requireToken)) {
      socket.destroy();
      return;
    }
    bridge(req, socket, { port: remote.VNC_PORT });
  });
}

/** One request handler, served on loopback and (when paired) on the LAN. */
function makeHandler(requireToken: boolean) {
  return (req: IncomingMessage, res: ServerResponse): void => {
    const url = new URL(req.url ?? '/', `http://${bindHost}`);

    if (!authorised(req, url, requireToken)) {
      res.writeHead(401, { 'content-type': 'application/json' });
      res.end(JSON.stringify({ error: 'pairing token required — open the link shown in the app' }));
      return;
    }

    if (url.pathname.startsWith('/api/')) {
      handleApi(req, res, url)
        .then((handled) => {
          if (!handled && !res.headersSent) notFound(res);
        })
        .catch((err: unknown) => {
          if (res.headersSent) return;
          sendJson(res, 500, { error: err instanceof Error ? err.message : String(err) });
        });
      return;
    }

    if (req.method !== 'GET' && req.method !== 'HEAD') {
      res.writeHead(405, { allow: 'GET, HEAD' });
      res.end();
      return;
    }

    void serveStatic(url.pathname, res);
  };
}

async function finishStart(server: Server, opts: StartOptions): Promise<DashboardHandle> {
  // A run only exists in this process's memory. If the console was killed
  // mid-task the row stayed `running` for ever, so the roster showed a bot
  // "Working…" hours later with nothing behind it and no way to clear it.
  // `paused` is swept for the same reason: the only thing that resumes a paused
  // task is the loop sitting in awaitResume, and that loop died with the last
  // process. Left alone the row shows "Needs you" for ever and the hand-back
  // buttons flip it to running with nothing behind them.
  for (const task of listTasks()) {
    if ((task.status !== 'running' && task.status !== 'paused') || activeRuns.has(task.id)) continue;
    setTaskStatus(task.id, 'failed');
    appendTaskLog(task.id, 'error', 'Interrupted — the console stopped while this was running.');
  }

  const port = await listen(server, opts.port ?? DEFAULT_PORT, 20);

  // Keep the roster and the computer pane honest without the client polling.
  const heartbeat = setInterval(() => {
    if (!clients.size) return;
    void computer.status().then((s) => publish('computer', s));
    publish('paused', listPausedTasks().map(taskView));
  }, 5000);
  heartbeat.unref();

  consolePort = port;

  return {
    url: `http://${bindHost === '0.0.0.0' ? lanAddress() : '127.0.0.1'}:${port}${pairingToken ? `/?t=${pairingToken}` : ''}`,
    port,
    host: bindHost,
    token: pairingToken,
    server,
    close: () =>
      new Promise<void>((resolve) => {
        clearInterval(heartbeat);
        for (const c of clients) c.res.end();
        clients.clear();
        void unpair();
        server.close(() => resolve());
      }),
  };
}
