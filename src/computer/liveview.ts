import { EventEmitter } from 'node:events';
import { randomUUID } from 'node:crypto';
import * as browser from './browser.ts';
import type { Snapshot } from './browser.ts';
import { activeViewUrl } from './container.ts';
import { getTask, listPausedTasks, resumeTask, type Task } from '../db/index.ts';

/**
 * Security boundaries and the human takeover protocol.
 *
 * When the bot reaches something only a human should do — a CAPTCHA, a 2FA
 * prompt, a password field, a payment page, or anything a skill tagged
 * `always-confirm` — the run stops and the human is handed the *same* browser
 * over noVNC. They complete that one step and hand control back.
 *
 * The rule that shapes this whole file: while paused the bot genuinely waits.
 * It does not poll the page, retry the sensitive action, or look for another
 * route around the human. The only thing polled here is the task row, i.e. we
 * are watching for the *human's* decision, not for the obstacle to disappear.
 */

// ---------------------------------------------------------------------------
// Boundaries
// ---------------------------------------------------------------------------

export type BoundaryKind = 'captcha' | 'two_factor' | 'password' | 'payment' | 'always_confirm';

export interface Boundary {
  kind: BoundaryKind;
  /** One line addressed to the human: what they are being asked to do. */
  reason: string;
  /** What actually matched. Goes in the task log, so never the typed text. */
  evidence: string;
  /** Where it was found. */
  url: string;
  /**
   * 'element' — the thing the bot was about to touch tripped it
   * 'page'    — the page itself did, with no action needed to see it
   * 'model'   — the bot escalated on its own judgement (request_human)
   */
  source: 'element' | 'page' | 'model';
}

/**
 * Element-label patterns. These run against a snapshot line such as
 * `[7] input:password "Password"`, which is all the model itself sees, so a
 * field the bot could identify as a credential is a field we can too.
 *
 * Ordered checks: password, then one-time codes, then card fields. "security
 * code" is deliberately in the OTP set — misfiling a CVV as a 2FA code changes
 * only the wording of the prompt, and both stop the bot dead either way.
 */
const EL_PASSWORD = /input:password|\bpass(word|phrase)\b|\bpwd\b/i;
const EL_OTP =
  /\botp\b|one[\s-]?time|two[\s-]?factor|\b2fa\b|\bmfa\b|verification code|security code|auth(entication)? code|sms code|passcode|confirmation code|backup code|recovery code/i;
const EL_CARD =
  /card ?number|credit card|debit card|\bcvv\b|\bcvc\b|expiry|expiration|exp date|\biban\b|routing number|sort code|cardholder|card holder|\bupi\b/i;

/**
 * Buttons that spend money or commit an order. Clicking one is irreversible in
 * a way navigation is not, so these pause even on a page the human already
 * took over — the takeover was consent to *be there*, not to buy.
 */
const EL_COMMIT =
  /place (the )?order|pay now|complete (the )?(purchase|order)|confirm (and pay|payment|purchase|order)|buy now|submit payment|authorize payment|start (free )?trial|subscribe now/i;

/** Distinctive enough in a path or query string to stand on its own. */
const URL_PAYMENT =
  /checkout|\/(payments?|billing|purchase|subscribe|order[-_]?(confirm|review|summary))(\/|\?|$)|[?&](step|stage)=(payment|checkout)/i;

// ---------------------------------------------------------------------------
// always-confirm rules (owned by the skills layer, injected here)
// ---------------------------------------------------------------------------

export interface AlwaysConfirmRule {
  /** Shown to the human, e.g. "posts to the company account". */
  label: string;
  /** RegExp, or a plain string treated as a case-insensitive substring. */
  match: RegExp | string;
  /**
   * 'element' checks only what the bot is about to click/type into, 'page'
   * only the URL/title/text, 'any' (default) both.
   */
  scope?: 'element' | 'page' | 'any';
}

let alwaysConfirm: AlwaysConfirmRule[] = [];

/** Replace the rule set — called when a skill with `always-confirm` tags loads. */
export function setAlwaysConfirmRules(rules: AlwaysConfirmRule[]): void {
  alwaysConfirm = [...rules];
}

export function addAlwaysConfirmRule(rule: AlwaysConfirmRule): void {
  alwaysConfirm.push(rule);
}

export function alwaysConfirmRules(): readonly AlwaysConfirmRule[] {
  return alwaysConfirm;
}

export function clearAlwaysConfirmRules(): void {
  alwaysConfirm = [];
}

function ruleMatches(rule: AlwaysConfirmRule, haystack: string): boolean {
  if (!haystack) return false;
  return typeof rule.match === 'string'
    ? haystack.toLowerCase().includes(rule.match.toLowerCase())
    : rule.match.test(haystack);
}

// ---------------------------------------------------------------------------
// Acknowledgements
// ---------------------------------------------------------------------------

/**
 * Page-level boundaries the human has already taken over once.
 *
 * Without this, a resumed run re-detects the same checkout page on its very
 * next click and pauses forever. Acknowledging is not "routing around the
 * human" — they explicitly handed control back on this page — so it is scoped
 * to the exact kind + path, and never applies to element-level detections or
 * to the commit buttons above.
 */
const acknowledged = new Set<string>();

function ackKey(b: Boundary): string {
  let path = b.url;
  try {
    const u = new URL(b.url);
    path = `${u.origin}${u.pathname}`;
  } catch {
    /* non-URL (about:blank, chrome://) — key on the raw string */
  }
  // Kind + path only: a CAPTCHA iframe src carries a per-load nonce, so keying
  // on the evidence would miss its own acknowledgement and pause forever.
  return `${b.kind}|${path}`;
}

export function acknowledge(b: Boundary): void {
  if (b.source !== 'page') return;
  acknowledged.add(ackKey(b));
}

/** Called at the start of every run: a new task gets no inherited consent. */
export function resetAcknowledged(): void {
  acknowledged.clear();
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

export interface DetectInput {
  /** What the bot is about to do. Element checks only run for type/click. */
  action?: 'type' | 'click' | 'navigate' | 'read';
  /** Ref the action targets, resolved against `snapshot`. */
  ref?: number;
  /** Reuse a snapshot the caller already took, rather than re-walking the DOM. */
  snapshot?: Snapshot;
}

interface PageSignals {
  url: string;
  title: string;
  captcha: string[];
  otpFields: string[];
  cardFields: string[];
  text: string;
}

/**
 * Classify what the bot is about to touch, and the page it is on.
 *
 * Note what is deliberately *not* here: a page merely containing a password
 * field is not a boundary. Login forms are everywhere and the bot has to be
 * able to read one to notice it is logged out. The boundary is typing into the
 * field, which is caught element-side.
 */
export async function detectBoundary(input: DetectInput = {}): Promise<Boundary | undefined> {
  const action = input.action ?? 'read';

  let snap = input.snapshot;
  if (!snap && input.ref !== undefined) snap = await browser.snapshot().catch(() => undefined);
  const url = snap?.url ?? (await currentUrl());

  if (input.ref !== undefined && (action === 'type' || action === 'click')) {
    const line = elementLine(snap, input.ref);
    const el = fromElement(line, url, action);
    if (el) return el;
  }

  const signals = await pageSignals();
  if (!signals) return undefined;

  const page = fromPage(signals);
  if (page && !acknowledged.has(ackKey(page))) return page;
  return undefined;
}

/** The snapshot line for a ref, e.g. `[7] input:password "Password"`. */
export function elementLine(snap: Snapshot | undefined, ref: number): string {
  if (!snap) return '';
  return (snap.elements.split('\n').find((l) => l.trim().startsWith(`[${ref}]`)) ?? '').trim();
}

function fromElement(line: string, url: string, action: 'type' | 'click'): Boundary | undefined {
  if (!line) return undefined;

  for (const rule of alwaysConfirm) {
    if (rule.scope !== 'page' && ruleMatches(rule, line)) {
      return {
        kind: 'always_confirm',
        reason: `A skill marked this always-confirm: ${rule.label}.`,
        evidence: line,
        url,
        source: 'element',
      };
    }
  }

  if (action === 'click') {
    // Typing into a field labelled "Pay now" is not a thing; only the click is.
    if (EL_COMMIT.test(line)) {
      return {
        kind: 'payment',
        reason: 'This button completes a purchase or commits an order.',
        evidence: line,
        url,
        source: 'element',
      };
    }
    return undefined;
  }

  if (EL_PASSWORD.test(line)) {
    return {
      kind: 'password',
      reason: 'This is a password field. A human signs in; the bot continues afterwards.',
      evidence: line,
      url,
      source: 'element',
    };
  }
  if (EL_OTP.test(line)) {
    return {
      kind: 'two_factor',
      reason: 'This is a one-time / 2FA code field. Only the human has the code.',
      evidence: line,
      url,
      source: 'element',
    };
  }
  if (EL_CARD.test(line)) {
    return {
      kind: 'payment',
      reason: 'This is a payment card field.',
      evidence: line,
      url,
      source: 'element',
    };
  }
  return undefined;
}

function fromPage(s: PageSignals): Boundary | undefined {
  const at = { url: s.url, source: 'page' as const };

  if (s.captcha.length) {
    return {
      ...at,
      kind: 'captcha',
      reason: 'A CAPTCHA is on screen. Solve it in the live view, then resume.',
      evidence: s.captcha.join('; ').slice(0, 200),
    };
  }

  if (s.otpFields.length) {
    return {
      ...at,
      kind: 'two_factor',
      reason: 'This page is asking for a one-time / 2FA code.',
      evidence: s.otpFields.join('; ').slice(0, 200),
    };
  }

  if (s.cardFields.length) {
    return {
      ...at,
      kind: 'payment',
      reason: 'This page is asking for payment card details.',
      evidence: s.cardFields.join('; ').slice(0, 200),
    };
  }

  // URL alone is enough for checkout: the cost of a needless 15-minute wait is
  // recoverable, the cost of an unattended purchase is not.
  const path = pathOf(s.url);
  if (URL_PAYMENT.test(path)) {
    return { ...at, kind: 'payment', reason: 'This is a checkout or billing page.', evidence: path };
  }

  const haystack = `${s.url}\n${s.title}\n${s.text}`;
  for (const rule of alwaysConfirm) {
    if (rule.scope !== 'element' && ruleMatches(rule, haystack)) {
      return {
        ...at,
        kind: 'always_confirm',
        reason: `A skill marked this always-confirm: ${rule.label}.`,
        evidence: rule.label,
      };
    }
  }

  return undefined;
}

function pathOf(url: string): string {
  try {
    const u = new URL(url);
    return `${u.pathname}${u.search}`;
  } catch {
    return url;
  }
}

async function currentUrl(): Promise<string> {
  try {
    return (await browser.activePage()).url();
  } catch {
    return '';
  }
}

/**
 * Everything that has to be read from the live DOM rather than from a snapshot:
 * iframe sources, input autocomplete hints, widget containers. `snapshot()`
 * flattens elements to a label, which is enough for credential fields but not
 * for a CAPTCHA — those are cross-origin iframes with no label at all.
 */
async function pageSignals(): Promise<PageSignals | undefined> {
  let page;
  try {
    page = await browser.activePage();
  } catch {
    return undefined;
  }

  try {
    return await page.evaluate(() => {
      const OTP =
        /\botp\b|one[\s-]?time|two[\s-]?factor|\b2fa\b|\bmfa\b|verification code|security code|auth(entication)? code|sms code|passcode|confirmation code/i;
      const CARD =
        /card ?number|cardnumber|credit card|debit card|\bcvv\b|\bcvc\b|\bcc[-_]?(num|number|exp|csc)|expiry|expiration|exp[-_ ]?(date|month|year)|\biban\b|routing ?number|sort ?code|cardholder/i;
      const CAPTCHA_SRC = /recaptcha|hcaptcha|turnstile|arkoselabs|funcaptcha|geetest|captcha/i;
      const CAPTCHA_TEXT =
        /verify (that )?you (are|'re) (a )?human|i'm not a robot|complete the security check|unusual traffic from your computer|checking your browser before/i;

      const seen = (el: Element): boolean => {
        const r = el.getBoundingClientRect();
        if (r.width < 4 || r.height < 4) return false;
        const s = getComputedStyle(el);
        return s.visibility !== 'hidden' && s.display !== 'none' && Number(s.opacity) > 0.05;
      };

      const labelOf = (el: Element): string => {
        const e = el as HTMLInputElement;
        const parts = [
          e.getAttribute('type') ?? '',
          e.getAttribute('name') ?? '',
          e.getAttribute('id') ?? '',
          e.getAttribute('placeholder') ?? '',
          e.getAttribute('aria-label') ?? '',
          e.getAttribute('autocomplete') ?? '',
        ];
        const id = e.getAttribute('id');
        if (id) {
          const lab = document.querySelector(`label[for="${CSS.escape(id)}"]`) as HTMLElement | null;
          if (lab?.innerText) parts.push(lab.innerText);
        }
        const wrap = el.closest('label') as HTMLElement | null;
        if (wrap?.innerText) parts.push(wrap.innerText);
        return parts.join(' ').replace(/\s+/g, ' ').trim().slice(0, 120);
      };

      const captcha: string[] = [];

      for (const f of Array.from(document.querySelectorAll('iframe'))) {
        const src = f.getAttribute('src') ?? '';
        if (!CAPTCHA_SRC.test(src)) continue;
        // reCAPTCHA v3 and "invisible" v2 load an iframe on pages with no
        // challenge at all, and the score badge is a visible 256x60 box. Both
        // would pause every third site on the web, so they are filtered out
        // and only a real, human-sized widget counts.
        if (/size=invisible/i.test(src)) continue;
        if (f.closest('.grecaptcha-badge')) continue;
        const r = f.getBoundingClientRect();
        if (!seen(f) || r.width < 200 || r.height < 60) continue;
        captcha.push(`captcha iframe ${src.slice(0, 120)}`);
      }

      for (const el of Array.from(
        document.querySelectorAll('.g-recaptcha, .h-captcha, .cf-turnstile, [data-sitekey], #challenge-form'),
      )) {
        if (!seen(el)) continue;
        if (el.getAttribute('data-size') === 'invisible') continue;
        captcha.push(`captcha widget <${el.tagName.toLowerCase()} class="${el.className}">`.slice(0, 140));
      }

      const bodyText = (document.body?.innerText ?? '').replace(/\s+/g, ' ').trim();
      if (CAPTCHA_TEXT.test(bodyText)) captcha.push('captcha wording on the page');

      const otpFields: string[] = [];
      const cardFields: string[] = [];
      for (const el of Array.from(document.querySelectorAll('input, textarea'))) {
        const e = el as HTMLInputElement;
        if (e.type === 'hidden' || !seen(el)) continue;
        const label = labelOf(el);
        const auto = (e.getAttribute('autocomplete') ?? '').toLowerCase();
        if (auto.includes('one-time-code') || OTP.test(label)) otpFields.push(label);
        else if (auto.startsWith('cc-') || CARD.test(label)) cardFields.push(label);
      }

      return {
        url: location.href,
        title: document.title,
        captcha: captcha.slice(0, 5),
        otpFields: otpFields.slice(0, 5),
        cardFields: cardFields.slice(0, 5),
        text: bodyText.slice(0, 4000),
      };
    });
  } catch {
    // Mid-navigation the context is torn down. Element-level checks still ran,
    // and the next tool call re-checks, so a failed read is not fatal.
    return undefined;
  }
}

// ---------------------------------------------------------------------------
// The live view
// ---------------------------------------------------------------------------

/**
 * The URL the human opens to take over. noVNC is served by the container that
 * is already running the browser, so this is a view of the *same* session —
 * cookies the human earns by signing in are the ones the bot keeps using.
 */
export function liveViewUrl(): string {
  const base = process.env.MYBOT_VIEW_URL || activeViewUrl() || 'http://127.0.0.1:6081/vnc.html';
  if (base.includes('?')) return base;
  // Without autoconnect the human lands on a connect dialog; without scaling
  // the 1280x800 display is cropped to the window and the button they need is
  // often the one off-screen.
  return `${base}?autoconnect=1&resize=scale`;
}

// ---------------------------------------------------------------------------
// Pause / resume
// ---------------------------------------------------------------------------

export const DEFAULT_PAUSE_TIMEOUT_MS = 15 * 60_000;
export const DEFAULT_POLL_MS = 1000;

/**
 * `skipped` is not a flavour of `resumed`. A human who declines the step has
 * told the bot something — that the step will never happen — and a bot that
 * treats it as "carry on" just re-reads the page, sees the same login form,
 * and asks again. That is an infinite ask, and it is what happens when the two
 * hand-back buttons both mean "continue".
 */
export type ResumeOutcome = 'resumed' | 'skipped' | 'timeout' | 'cancelled';

export interface PendingPause {
  pauseId: string;
  taskId?: string;
  kind: BoundaryKind | 'unknown';
  reason: string;
  viewUrl: string;
  /** ISO timestamp. */
  since: string;
  /** True when a run in *this* process is blocked on it. */
  waiting: boolean;
}

/**
 * `pause` / `resolved` for anyone rendering the queue; `resume` is the input
 * channel — emitting it (via `resume()`) releases a waiting run immediately
 * instead of on the next poll tick, which is how the dashboard will answer
 * without going through the database.
 */
export const events = new EventEmitter();

const waiting = new Map<string, PendingPause>();

export interface AwaitResumeOptions {
  /** Default 15 min. Timeout means "the human never came", never "carry on". */
  timeoutMs?: number;
  pollMs?: number;
  boundary?: Boundary;
  /** Heartbeat on every poll tick, for a countdown in the CLI. */
  onWaiting?: (elapsedMs: number, remainingMs: number) => void;
  signal?: AbortSignal;
}

/**
 * Block until a human resumes the task, or the timeout elapses.
 *
 * Two ways in, because the CLI and the dashboard are different processes: the
 * task row is polled once a second (cross-process, survives a restart of
 * whatever is watching), and the `resume` event resolves in-process instantly.
 * Neither path touches the browser — the obstacle is not what we are waiting on.
 */
export async function awaitResume(
  taskId: string | undefined,
  opts: AwaitResumeOptions = {},
): Promise<ResumeOutcome> {
  const timeoutMs = opts.timeoutMs ?? DEFAULT_PAUSE_TIMEOUT_MS;
  const pollMs = opts.pollMs ?? DEFAULT_POLL_MS;
  const started = Date.now();

  const pending: PendingPause = {
    pauseId: randomUUID(),
    taskId,
    kind: opts.boundary?.kind ?? 'unknown',
    reason: opts.boundary?.reason ?? 'Waiting for a human.',
    viewUrl: liveViewUrl(),
    since: new Date(started).toISOString(),
    waiting: true,
  };
  waiting.set(pending.pauseId, pending);
  events.emit('pause', pending);

  try {
    const outcome = await new Promise<ResumeOutcome>((resolve) => {
      let settled = false;

      const finish = (result: ResumeOutcome) => {
        if (settled) return;
        settled = true;
        clearInterval(poll);
        clearTimeout(timer);
        events.off('resume', onResume);
        opts.signal?.removeEventListener('abort', onAbort);
        resolve(result);
      };

      const onResume = (id: string, skipped = false) => {
        if (id === pending.pauseId || (taskId !== undefined && id === taskId)) {
          finish(skipped ? 'skipped' : 'resumed');
        }
      };
      const onAbort = () => finish('cancelled');

      const poll = setInterval(() => {
        const elapsed = Date.now() - started;
        opts.onWaiting?.(elapsed, Math.max(0, timeoutMs - elapsed));
        if (taskId === undefined) return;
        try {
          const row = getTask(taskId);
          if (!row || row.status === 'paused') return;
          // Anything other than a return to 'running' means the human dealt
          // with the task some other way (cancelled it, marked it done).
          finish(row.status === 'running' ? 'resumed' : 'cancelled');
        } catch {
          // A transient SQLITE_BUSY must not be read as a decision.
        }
      }, pollMs);

      const timer = setTimeout(() => finish('timeout'), timeoutMs);

      events.on('resume', onResume);
      opts.signal?.addEventListener('abort', onAbort, { once: true });
    });

    events.emit('resolved', { ...pending, waiting: false }, outcome);
    return outcome;
  } finally {
    waiting.delete(pending.pauseId);
  }
}

/**
 * Hand control back. Accepts a task id or a pause id, so a pause on an ad-hoc
 * run with no database row can still be released.
 */
export function resume(id: string, opts: { skipped?: boolean } = {}): boolean {
  let known = false;
  try {
    if (getTask(id)) {
      resumeTask(id);
      known = true;
    }
  } catch {
    /* no database yet — in-process waiters below still apply */
  }
  for (const p of waiting.values()) {
    if (p.pauseId === id || p.taskId === id) known = true;
  }
  // The skip travels on the event, not the row: a waiter in another process
  // falls back to the row's status and reads it as a plain resume, which is the
  // safe direction to be wrong in — it asks again rather than silently dropping
  // a step the human never actually declined.
  events.emit('resume', id, opts.skipped === true);
  return known;
}

/**
 * Everything waiting on a human: paused task rows (durable, visible to any
 * process) merged with pauses in this process that have no task row.
 */
export function listPending(): PendingPause[] {
  const out: PendingPause[] = [];
  const seenTasks = new Set<string>();

  let rows: Task[] = [];
  try {
    rows = listPausedTasks();
  } catch {
    /* database not initialised — fall back to in-process state only */
  }

  for (const row of rows) {
    // paused_reason / paused_at arrive via migration, after the Task interface.
    const r = row as Task & { paused_reason?: string | null; paused_at?: string | null };
    const local = [...waiting.values()].find((p) => p.taskId === row.id);
    const parsed = splitReason(r.paused_reason ?? '');
    seenTasks.add(row.id);
    out.push({
      pauseId: local?.pauseId ?? row.id,
      taskId: row.id,
      kind: local?.kind ?? parsed.kind,
      reason: parsed.reason || local?.reason || 'Waiting for a human.',
      viewUrl: liveViewUrl(),
      since: r.paused_at ?? local?.since ?? '',
      waiting: !!local,
    });
  }

  for (const p of waiting.values()) {
    if (p.taskId && seenTasks.has(p.taskId)) continue;
    out.push(p);
  }
  return out;
}

/**
 * `paused_reason` is stored as "<kind>: <reason>" by the agent loop; recover the
 * kind so a listing can render it, without making the column a second schema.
 */
function splitReason(stored: string): { kind: BoundaryKind | 'unknown'; reason: string } {
  const m = stored.match(/^(captcha|two_factor|password|payment|always_confirm): ([\s\S]*)$/);
  return m ? { kind: m[1] as BoundaryKind, reason: m[2]! } : { kind: 'unknown', reason: stored };
}
