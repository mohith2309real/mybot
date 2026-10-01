import { randomUUID } from 'node:crypto';
import { readFile, stat } from 'node:fs/promises';
import { homedir } from 'node:os';
import { isAbsolute, resolve } from 'node:path';
import { EventEmitter } from 'node:events';
import { openDb } from '../db/index.ts';
import { deliverFile, session, type Computer } from './container.ts';
import type { ModelProvider } from '../providers/types.ts';

/**
 * A bot asking for a file from the human's own machine.
 *
 * The container never gets a bind mount of a host directory. A mount grants
 * standing access to a whole tree for the container's lifetime; this grants
 * exactly the file that was approved, once, pushed in through the daemon. When
 * the bot is done the file is in its inbox and nothing else on the Mac was ever
 * reachable. That difference is the whole security story of host access.
 *
 * The timeout path is the interesting one. When a human does not answer, an
 * ARBITER model call judges whether the task genuinely needs the file. It can
 * deny, or it can escalate — it can never grant. An unanswered request is not
 * consent, however reasonable the request looks, and a model that could talk
 * itself into a grant would make the whole consent step decorative.
 */

export type RequestStatus = 'pending' | 'granted' | 'denied' | 'arbitrated' | 'expired';

export interface HostRequest {
  id: string;
  task_id: string | null;
  bot: string;
  path: string;
  reason: string;
  status: RequestStatus;
  verdict: string | null;
  decided_by: 'human' | 'arbiter' | null;
  created_at: string;
  decided_at: string | null;
}

export const events = new EventEmitter();

export const DEFAULT_WAIT_MS = 5 * 60 * 1000;
export const POLL_MS = 1000;

/**
 * Paths never shared, whatever anyone says.
 *
 * These hold credentials for other systems. A bot that talks a human into
 * "just send me your .ssh folder" has escalated out of its container entirely,
 * so this list is checked before the human is even asked — there is no prompt
 * to social-engineer.
 */
const NEVER_SHARE = [
  /(^|\/)\.ssh(\/|$)/,
  /(^|\/)\.aws(\/|$)/,
  /(^|\/)\.gnupg(\/|$)/,
  /(^|\/)\.config\/gcloud(\/|$)/,
  /(^|\/)\.kube(\/|$)/,
  /(^|\/)Library\/Keychains(\/|$)/,
  /(^|\/)\.netrc$/,
  /(^|\/)\.env(\.[a-z]+)?$/,
  /(^|\/)id_(rsa|ed25519|ecdsa)/,
  /(^|\/)\.mybot(\/|$)/, // our own vault
];

export function isForbiddenPath(path: string): { forbidden: boolean; why?: string } {
  const expanded = path.startsWith('~') ? path.replace(/^~/, homedir()) : path;
  const abs = isAbsolute(expanded) ? resolve(expanded) : resolve(homedir(), expanded);
  for (const rule of NEVER_SHARE) {
    if (rule.test(abs)) {
      return {
        forbidden: true,
        why: `${abs} holds credentials for other systems and is never shared with a bot, in any mode.`,
      };
    }
  }
  return { forbidden: false };
}

export function resolveHostPath(path: string): string {
  const expanded = path.startsWith('~') ? path.replace(/^~/, homedir()) : path;
  return isAbsolute(expanded) ? resolve(expanded) : resolve(homedir(), expanded);
}

// --- persistence -----------------------------------------------------------

export function createRequest(input: {
  bot: string;
  path: string;
  reason: string;
  taskId?: string;
}): HostRequest {
  const id = randomUUID();
  openDb()
    .prepare('INSERT INTO host_requests (id, task_id, bot, path, reason) VALUES (?, ?, ?, ?, ?)')
    .run(id, input.taskId ?? null, input.bot, input.path, input.reason);
  const req = getRequest(id)!;
  events.emit('requested', req);
  return req;
}

export function getRequest(id: string): HostRequest | undefined {
  return openDb().prepare('SELECT * FROM host_requests WHERE id = ?').get(id) as unknown as
    | HostRequest
    | undefined;
}

export function listPending(): HostRequest[] {
  return openDb()
    .prepare("SELECT * FROM host_requests WHERE status='pending' ORDER BY created_at")
    .all() as unknown as HostRequest[];
}

function settle(id: string, status: RequestStatus, by: 'human' | 'arbiter', verdict: string): void {
  openDb()
    .prepare("UPDATE host_requests SET status=?, decided_by=?, verdict=?, decided_at=datetime('now') WHERE id=?")
    .run(status, by, verdict, id);
  const req = getRequest(id);
  if (req) events.emit('settled', req);
}

/** Human says yes. The only path that can ever grant. */
export function grant(id: string, verdict = 'approved by human'): void {
  settle(id, 'granted', 'human', verdict);
}

export function deny(id: string, verdict = 'denied by human'): void {
  settle(id, 'denied', 'human', verdict);
}

// --- the arbiter -----------------------------------------------------------

export interface ArbiterOptions {
  provider: ModelProvider;
  model?: string;
  /** What the bot was asked to do, so necessity can be judged against it. */
  taskInstruction?: string;
}

const ARBITER_SYSTEM = `You judge whether an AI agent genuinely needs a file from its user's
personal computer in order to finish the task it was given.

The user did not answer the request in time. You are deciding what happens in
their absence.

You have exactly two options:

  NOT_NEEDED  The task can be completed, or usefully progressed, without this
              file. Choose this whenever the agent has a reasonable alternative,
              when the request is broader than the task requires, or when you
              are unsure.

  ESCALATE    The task genuinely cannot proceed without this specific file, and
              it is proportionate to what was asked.

You cannot approve access. Silence is not consent, so ESCALATE means "ask the
human again and wait", never "go ahead".

Reply with exactly one line:
VERDICT: NOT_NEEDED <one sentence on what the agent should do instead>
or
VERDICT: ESCALATE <one sentence on why nothing else will do>`;

export interface ArbiterVerdict {
  outcome: 'not_needed' | 'escalate';
  rationale: string;
}

/** Ask a model whether an unanswered request is genuinely necessary. */
export async function arbitrate(req: HostRequest, opts: ArbiterOptions): Promise<ArbiterVerdict> {
  const prompt =
    `Task the agent was given: ${opts.taskInstruction ?? '(not recorded)'}\n\n` +
    `Agent: ${req.bot}\n` +
    `File requested: ${req.path}\n` +
    `Agent's stated reason: ${req.reason}\n\n` +
    `The human did not respond. Judge necessity.`;

  let text = '';
  for await (const chunk of opts.provider.chat([{ role: 'user', content: prompt }], {
    model: opts.model,
    system: ARBITER_SYSTEM,
    maxTokens: 500,
  })) {
    if (chunk.type === 'text_delta') text += chunk.text;
    if (chunk.type === 'error') {
      // A broken arbiter must not become an accidental grant.
      return { outcome: 'not_needed', rationale: `arbiter unavailable (${chunk.error}); proceeding without the file` };
    }
  }

  const escalate = /VERDICT:\s*ESCALATE/i.test(text);
  const rationale = text.replace(/VERDICT:\s*(NOT_NEEDED|ESCALATE)\s*/i, '').trim() || text.trim();
  return { outcome: escalate ? 'escalate' : 'not_needed', rationale: rationale.slice(0, 400) };
}

// --- the full flow ---------------------------------------------------------

export interface AwaitOptions extends Partial<ArbiterOptions> {
  waitMs?: number;
  onWaiting?: (elapsedMs: number) => void;
}

export type AccessOutcome =
  | { outcome: 'granted'; containerPath: string; bytes: number }
  | { outcome: 'denied'; reason: string }
  | { outcome: 'not_needed'; reason: string }
  | { outcome: 'escalated'; reason: string };

/**
 * Ask for a host file and see the request through: wait for the human, fall
 * back to the arbiter on timeout, and on approval push the file in.
 */
export async function requestFile(
  input: { bot: string; path: string; reason: string; taskId?: string },
  opts: AwaitOptions = {},
): Promise<AccessOutcome> {
  const forbidden = isForbiddenPath(input.path);
  if (forbidden.forbidden) {
    return { outcome: 'denied', reason: forbidden.why! };
  }

  const req = createRequest(input);
  const waitMs = opts.waitMs ?? DEFAULT_WAIT_MS;
  const deadline = Date.now() + waitMs;

  while (Date.now() < deadline) {
    const now = getRequest(req.id);
    if (!now) return { outcome: 'denied', reason: 'request disappeared' };

    if (now.status === 'granted') return deliver(now);
    if (now.status === 'denied') return { outcome: 'denied', reason: now.verdict ?? 'denied by human' };

    opts.onWaiting?.(waitMs - (deadline - Date.now()));
    await new Promise((r) => setTimeout(r, POLL_MS));
  }

  // Nobody answered. Ask the arbiter whether this was even necessary.
  if (!opts.provider) {
    settle(req.id, 'expired', 'arbiter', 'no human response and no arbiter configured');
    return { outcome: 'not_needed', reason: 'No response, and no arbiter available. Continue without the file.' };
  }

  const verdict = await arbitrate(req, {
    provider: opts.provider,
    model: opts.model,
    taskInstruction: opts.taskInstruction,
  });
  settle(req.id, 'arbitrated', 'arbiter', `${verdict.outcome}: ${verdict.rationale}`);

  return verdict.outcome === 'escalate'
    ? { outcome: 'escalated', reason: verdict.rationale }
    : { outcome: 'not_needed', reason: verdict.rationale };
}

async function deliver(req: HostRequest): Promise<AccessOutcome> {
  const abs = resolveHostPath(req.path);

  // Re-check at delivery time: the path is re-read from the row, and a row can
  // be edited between request and grant.
  const forbidden = isForbiddenPath(abs);
  if (forbidden.forbidden) return { outcome: 'denied', reason: forbidden.why! };

  const info = await stat(abs).catch(() => undefined);
  if (!info?.isFile()) return { outcome: 'denied', reason: `${abs} is not a readable file` };
  if (info.size > 25 * 1024 * 1024) {
    return { outcome: 'denied', reason: `${abs} is ${(info.size / 1e6).toFixed(1)}MB; 25MB is the limit` };
  }

  const computer: Computer = session().computer;
  const body = await readFile(abs);
  const placed = await deliverFile(computer, req.bot, abs.split('/').pop() ?? 'file', body);
  return { outcome: 'granted', containerPath: placed.path, bytes: placed.bytes };
}
