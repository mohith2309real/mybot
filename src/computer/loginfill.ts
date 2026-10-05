import type { Snapshot } from './browser.ts';
import * as logins from '../secrets/logins.ts';
import type { ApprovalOutcome, LoginSummary } from '../secrets/logins.ts';

/**
 * fill_login: sign in with a login the human saved, after the human says yes.
 *
 * The sequence, and why each step is where it is:
 *
 *   1. Read the page the bot is on. The origin comes from the browser, never
 *      from the model — a bot cannot claim it is on github.com.
 *   2. Check the refs point at the right kinds of field (password / username).
 *   3. Find a login saved for that exact origin. None → hand over to the human,
 *      exactly as a password field did before saved logins existed.
 *   4. Ask the human (or apply their "always"). No answer → nothing fills.
 *   5. Fill, re-checking origin and field types against the live DOM at the
 *      moment of filling, because the page may have moved while we waited.
 *
 * What the model gets back is a sentence about what happened. The password is
 * in this process for the length of one fill call and goes nowhere else.
 *
 * The browser and the approval are injected so all of this is testable
 * without a container; tools.ts wires in the real ones.
 */

export interface FillLoginInput {
  passwordRef?: number;
  usernameRef?: number;
  /** Which saved username, when more than one is saved for this site. */
  account?: string;
  submit?: boolean;
}

export interface FillContext {
  bot?: string;
  taskId?: string;
  signal?: AbortSignal;
  approvalWaitMs?: number;
}

export interface FillDeps {
  snapshot(): Promise<Snapshot>;
  fill(opts: {
    origin: string;
    passwordRef?: number;
    usernameRef?: number;
    username: string;
    password: string;
    submit?: boolean;
  }): Promise<{ submitted: boolean }>;
  approve(
    input: logins.ApprovalInput,
    opts: logins.ApprovalOptions,
  ): Promise<ApprovalOutcome>;
}

export type FillResult =
  /** Filled. */
  | { kind: 'filled'; text: string }
  /** The model asked for something wrong; it can correct and retry. */
  | { kind: 'error'; text: string }
  /** No saved login (or the store is locked): a human signs in instead. */
  | { kind: 'handoff'; text: string; reason: string }
  /** The human said no, or did not answer. Do not ask again. */
  | { kind: 'denied'; text: string };

/**
 * One approval covers both pages of a two-step sign-in (username, then
 * password) for the same login on the same site in the same task. Without it
 * Google and Microsoft sign-ins would ask you twice in ten seconds.
 */
const GRANT_MS = 5 * 60_000;
const grants = new Map<string, number>();

function grantKey(ctx: FillContext, origin: string, loginId: string): string {
  return `${ctx.taskId ?? '-'}|${origin}|${loginId}`;
}

export function clearGrants(): void {
  grants.clear();
}

/** `[7] input:password "Password"` → `input:password`. Parsed, not pattern-matched: a label can say anything. */
export function elementKind(snap: Snapshot, ref: number): string | undefined {
  for (const line of snap.elements.split('\n')) {
    const m = /^\s*\[(\d+)\]\s+(\S+)/.exec(line);
    if (m && Number(m[1]) === ref) return m[2];
  }
  return undefined;
}

const USERNAME_KINDS = new Set(['input:text', 'input:email', 'input:tel', 'input:search']);

export async function fillLogin(input: FillLoginInput, ctx: FillContext, deps: FillDeps): Promise<FillResult> {
  const passwordRef = input.passwordRef;
  const usernameRef = input.usernameRef;
  if (passwordRef === undefined && usernameRef === undefined) {
    return { kind: 'error', text: 'fill_login needs password_ref, username_ref, or both. Call page_read first.' };
  }

  const snap = await deps.snapshot();
  const origin = logins.pageOrigin(snap.url);
  if (!origin) {
    return {
      kind: 'error',
      text: `Saved logins are only filled on https pages. This page is ${snap.url || 'blank'}. Nothing was filled.`,
    };
  }

  if (passwordRef !== undefined && elementKind(snap, passwordRef) !== 'input:password') {
    return { kind: 'error', text: `Ref ${passwordRef} is not a password field on this page. Call page_read and check.` };
  }
  if (usernameRef !== undefined && !USERNAME_KINDS.has(elementKind(snap, usernameRef) ?? '')) {
    return {
      kind: 'error',
      text: `Ref ${usernameRef} is not a username or email field on this page. Call page_read and check.`,
    };
  }

  if (!logins.loginsExist()) {
    return {
      kind: 'handoff',
      reason: `No saved login for ${origin}. Sign in here, then hand control back.`,
      text: `No logins are saved. Handing over to the human to sign in.`,
    };
  }

  if (!(await logins.ensureUnlocked())) {
    return {
      kind: 'handoff',
      reason: `Saved logins are locked, so sign in to ${origin} yourself, then hand control back.`,
      text: 'Saved logins are locked in this session. Handing over to the human to sign in.',
    };
  }

  let candidates: LoginSummary[];
  try {
    candidates = logins.loginsFor(origin);
  } catch (err) {
    return {
      kind: 'handoff',
      reason: `Could not open saved logins (${err instanceof Error ? err.message : String(err)}). Sign in to ${origin} yourself.`,
      text: 'Saved logins could not be opened. Handing over to the human to sign in.',
    };
  }

  if (!candidates.length) {
    return {
      kind: 'handoff',
      reason: `No saved login for ${origin}. Sign in here, then hand control back.`,
      text: `No login is saved for ${origin}. Handing over to the human to sign in.`,
    };
  }

  let login: LoginSummary | undefined;
  if (input.account) {
    login = candidates.find((l) => l.username.toLowerCase() === input.account!.trim().toLowerCase());
    if (!login) {
      return {
        kind: 'error',
        text: `No saved login for "${input.account}" on ${origin}. Saved accounts here: ${candidates.map((l) => l.username).join(', ')}.`,
      };
    }
  } else if (candidates.length === 1) {
    login = candidates[0];
  } else {
    return {
      kind: 'error',
      text:
        `More than one login is saved for ${origin}: ${candidates.map((l) => l.username).join(', ')}. ` +
        'Call fill_login again with account set to the one the task needs. If the task does not say, call request_human.',
    };
  }
  login = login!;

  const key = grantKey(ctx, origin, login.id);
  const grantedUntil = grants.get(key) ?? 0;
  if (grantedUntil < Date.now()) {
    const answer = await deps.approve(
      { bot: ctx.bot ?? 'bot', taskId: ctx.taskId, login },
      { waitMs: ctx.approvalWaitMs, signal: ctx.signal },
    );
    if (!answer.allowed) {
      const why =
        answer.by === 'timeout'
          ? 'The human did not answer in time, so nothing was filled.'
          : answer.by === 'cancelled'
            ? 'The task was stopped while waiting, so nothing was filled.'
            : 'The human said no to using the saved login.';
      return {
        kind: 'denied',
        text:
          `${why} Do not call fill_login again for this site in this task, and never try to type a password yourself. ` +
          'If signing in is still required, call request_human; otherwise carry on with what you can do without it.',
      };
    }
    grants.set(key, Date.now() + GRANT_MS);
  }

  let secret: { username: string; password: string };
  try {
    secret = logins.revealForFill(login.id, origin);
  } catch (err) {
    return { kind: 'error', text: `${err instanceof Error ? err.message : String(err)} Nothing was filled.` };
  }

  try {
    const r = await deps.fill({
      origin,
      passwordRef,
      usernameRef,
      username: secret.username,
      password: secret.password,
      submit: input.submit,
    });
    const what =
      passwordRef !== undefined && usernameRef !== undefined
        ? 'username and password'
        : passwordRef !== undefined
          ? 'password'
          : 'username';
    return {
      kind: 'filled',
      text:
        `Filled the saved ${what} for ${login.username} on ${origin}${r.submitted ? ' and submitted the form' : ''}. ` +
        'Call page_read to see the result. If the site now asks for a one-time code, that is the human\'s to enter.',
    };
  } catch (err) {
    // FillError messages are fixed strings; anything else is replaced, because
    // an unknown error could have been built from the arguments.
    const safe = err instanceof Error && err.name === 'FillError' ? err.message : 'The fill failed. Nothing more was attempted.';
    return { kind: 'error', text: safe };
  } finally {
    secret.password = '';
  }
}
