import type { BrowserContext, Frame } from 'playwright-core';
import { activePage, connect } from '../computer/browser.ts';
import * as db from '../db/index.ts';
import { getProvider } from '../providers/index.ts';
import type { ModelProvider, ProviderName, ToolDefinition } from '../providers/types.ts';

/**
 * Teach-a-Task: the human demonstrates a workflow in the bot's own browser and a
 * model compiles the recording into a Skill.
 *
 * The demonstration happens over noVNC on the *same* Chromium the bot drives, so
 * a listener injected into every document sees the real interactions. Grok Bot's
 * equivalent records one browser workflow and caps it at ten minutes; MyBot
 * matches that, and enforces the cap here rather than trusting a caller to stop.
 *
 * Secrets never leave the page. Classification and redaction happen *inside* the
 * browser, so the raw contents of a password / OTP / card field are never handed
 * to Node at all; a second pass in Node re-checks every action before it reaches
 * SQLite or a model. Over-redaction is the deliberate failure mode.
 */

/** xAI's documented limit for one teach-a-task recording. */
export const MAX_RECORDING_MS = 10 * 60 * 1000;

/** Name of the page->Node channel and of the recorder's window state object. */
const BINDING = '__mybotTeachEmit';
const STATE_KEY = '__mybotTeach';

/**
 * Field hints that mean "whatever is typed here is a secret". Shared verbatim
 * with the in-page recorder so there is exactly one definition of sensitive,
 * and applied again in Node as defence in depth.
 */
const SENSITIVE_HINT_PATTERN =
  'pass(word|code|phrase)|passwd|\\bpwd\\b|\\botp\\b|one[-_ ]?time|2fa|\\bmfa\\b|two[-_ ]?factor|' +
  'auth(entication)?[-_ ]?code|verif(y|ication)[-_ ]?code|security[-_ ]?code|confirm(ation)?[-_ ]?code|' +
  'sms[-_ ]?code|\\bcvv\\b|\\bcvc\\b|\\bcsc\\b|card[-_ ]?number|cardnum|credit[-_ ]?card|\\bpin\\b|' +
  'secret|\\btoken\\b|api[-_ ]?key|private[-_ ]?key|seed[-_ ]?phrase|mnemonic|\\bssn\\b|' +
  'social[-_ ]?security|routing[-_ ]?number|account[-_ ]?number|\\biban\\b|passport|security[-_ ]?answer';

const SENSITIVE_HINT_RE = new RegExp(SENSITIVE_HINT_PATTERN, 'i');

/**
 * Query-string keys that carry credentials.
 *
 * This matters more than it looks. A form with `method="get"` puts every field
 * value into the URL, so a login the human demonstrates ends up as
 * `?user=…&pw=…&cc=…` — and the recorder stores URLs verbatim. An earlier
 * version listed only long spellings (`password`, `pwd`, `otp`) and happily
 * recorded `pw=hunter2` and `cc=4111…` in the clear.
 *
 * Two sources now: the same hint pattern used for field labels, plus the short
 * abbreviations that appear as form field names but never as human labels.
 * Over-matching here costs a `REDACTED` in a demo; under-matching writes a
 * password to disk.
 */
const SENSITIVE_PARAM_ALIASES =
  'pw|pass|passwd|pwd|cc|ccnum|ccnumber|card|cardno|cvv|cvc|csc|pin|ssn|otp|totp|mfa|' +
  'code|token|secret|key|sig|signature|session|auth|credential|state|nonce';

const SENSITIVE_PARAM_RE = new RegExp(
  `(^|[_-])(${SENSITIVE_PARAM_ALIASES})([_-]|$)|${SENSITIVE_HINT_PATTERN}`,
  'i',
);

export interface ActionTarget {
  /** Human-readable: aria-label, associated <label>, placeholder, name, text. */
  description: string;
  tag: string;
  type?: string;
  /** Best-effort stable selector. The compiled skill is prose, not a replay. */
  selector?: string;
}

export type ActionKind =
  | 'navigate'
  | 'click'
  | 'input'
  | 'select'
  | 'submit'
  | 'key'
  | 'scroll';

export interface RecordedAction {
  /** `<documentId>:<sequence>` — unique per document, used to de-duplicate. */
  id: string;
  at: string;
  kind: ActionKind;
  url: string;
  target?: ActionTarget;
  /** Typed/selected text. Already a `<redacted:…>` marker when sensitive. */
  value?: string;
  redacted?: boolean;
  detail?: string;
}

export interface TeachStatus {
  id: string;
  label: string;
  status: 'recording' | 'stopped' | 'compiled';
  actionCount: number;
  startedAt: string;
  stoppedAt: string | null;
  skillId: string | null;
  /** Milliseconds left before the hard cap; null once stopped. */
  msRemaining: number | null;
  /** True when this process owns the live recorder (vs. reading a past run). */
  live: boolean;
}

export interface SkillDraft {
  name: string;
  /** When a bot should reach for this skill. */
  trigger: string;
  steps: string[];
  safetyRules: string[];
  notes?: string;
}

// ---------------------------------------------------------------------------
// The injected recorder (runs in Chromium, not in Node)
// ---------------------------------------------------------------------------

interface RecorderConfig {
  binding: string;
  stateKey: string;
  sensitivePattern: string;
}

/**
 * Installed via `addInitScript` (so it survives navigation and covers frames)
 * and via `evaluate` (so the document that is already open is covered too).
 * Idempotent: a second install just re-arms the existing state.
 */
function installRecorder(cfg: RecorderConfig): void {
  const w = window as unknown as Record<string, any>;
  const already = w[cfg.stateKey];
  if (already && already.installed) {
    already.on = true;
    return;
  }

  const state = {
    installed: true,
    on: true,
    queue: [] as Record<string, unknown>[],
    seq: 0,
    docId: Math.random().toString(36).slice(2, 10),
  };
  w[cfg.stateKey] = state;

  const sensitive = new RegExp(cfg.sensitivePattern, 'i');
  const lastValue = new WeakMap<Element, string>();
  let pendingEl: Element | null = null;
  let pendingTimer: any = 0;
  let lastScrollAt = 0;
  let lastScrollY = 0;

  const clean = (s: string | null): string =>
    (s || '').slice(0, 400).replace(/\s+/g, ' ').trim().slice(0, 120);

  const emit = (a: Record<string, unknown>): void => {
    if (!state.on) return;
    a.id = `${state.docId}:${++state.seq}`;
    a.at = new Date().toISOString();
    a.url = location.href;
    state.queue.push(a);
    // Bound the buffer: a runaway page must not grow memory without limit.
    if (state.queue.length > 500) state.queue.splice(0, state.queue.length - 500);

    const send = w[cfg.binding];
    if (typeof send !== 'function') return;
    try {
      // The queue is the source of truth; entries are dropped only once Node
      // has acknowledged them, so a binding installed late loses nothing.
      Promise.resolve(send(a))
        .then(() => {
          const i = state.queue.indexOf(a);
          if (i >= 0) state.queue.splice(i, 1);
        })
        .catch(() => {});
    } catch {
      /* page navigated mid-send; the poller will pick it up */
    }
  };

  const labelOf = (el: Element): string => {
    const e = el as HTMLElement & {
      labels?: ArrayLike<HTMLLabelElement>;
      value?: string;
      name?: string;
      type?: string;
    };
    const fromLabel = e.labels && e.labels.length ? (e.labels[0] as HTMLElement).innerText : '';
    const ref = el.getAttribute('aria-labelledby');
    const referenced = ref ? document.getElementById(ref.split(' ')[0]) : null;
    const tag = el.tagName.toLowerCase();
    const type = (e.type || '').toLowerCase();
    // A button's `value` is its caption; every other control's `value` is user
    // data and must never be used as a description.
    const caption =
      tag === 'button' || (tag === 'input' && ['submit', 'button', 'reset'].includes(type))
        ? e.value || ''
        : '';
    return (
      clean(el.getAttribute('aria-label')) ||
      clean(fromLabel) ||
      clean(referenced ? referenced.innerText : '') ||
      clean(el.getAttribute('placeholder')) ||
      clean(el.getAttribute('title')) ||
      clean(el.getAttribute('alt')) ||
      clean((el as HTMLElement).innerText) ||
      clean(caption) ||
      clean(el.getAttribute('name')) ||
      ''
    );
  };

  const selectorOf = (el: Element): string => {
    const esc = (s: string) => s.replace(/["\\]/g, '\\$&');
    const tag = el.tagName.toLowerCase();
    const id = el.getAttribute('id');
    // Framework-generated ids (React's `:r3:`, hash suffixes) are not stable.
    if (id && !/\d{4,}|:r[0-9a-z]+:/i.test(id)) return `#${id}`;
    const testId = el.getAttribute('data-testid') || el.getAttribute('data-test-id');
    if (testId) return `[data-testid="${esc(testId)}"]`;
    const name = el.getAttribute('name');
    if (name) return `${tag}[name="${esc(name)}"]`;
    const aria = el.getAttribute('aria-label');
    if (aria) return `${tag}[aria-label="${esc(aria)}"]`;

    const parts: string[] = [];
    let node: Element | null = el;
    for (let depth = 0; node && depth < 4; depth++) {
      const current: Element = node;
      let part = current.tagName.toLowerCase();
      const parent = current.parentElement;
      if (parent) {
        const sibs = Array.prototype.filter.call(
          parent.children,
          (c: Element) => c.tagName === current.tagName,
        ) as Element[];
        if (sibs.length > 1) part += `:nth-of-type(${sibs.indexOf(current) + 1})`;
      }
      parts.unshift(part);
      node = parent;
    }
    return parts.join(' > ');
  };

  /** '' means "safe to record verbatim"; anything else is a redaction reason. */
  const sensitivityOf = (el: Element): string => {
    const e = el as HTMLInputElement;
    const type = (e.type || '').toLowerCase();
    if (type === 'password') return 'password';

    const auto = (el.getAttribute('autocomplete') || '').toLowerCase();
    if (auto.includes('password')) return 'password';
    if (auto.includes('one-time-code')) return 'one-time-code';
    if (auto.includes('cc-number')) return 'card-number';
    if (auto.includes('cc-csc')) return 'card-security-code';
    if (auto.startsWith('cc-')) return 'card-detail';

    const hint = [
      el.getAttribute('name'),
      el.getAttribute('id'),
      el.getAttribute('placeholder'),
      el.getAttribute('aria-label'),
      el.getAttribute('title'),
      el.getAttribute('data-testid'),
      labelOf(el),
    ].join(' ');
    if (sensitive.test(hint)) return 'sensitive';

    // A short numeric field is an OTP / CVV far more often than it is data
    // worth keeping, so it is treated as suspect.
    const inputMode = (el.getAttribute('inputmode') || '').toLowerCase();
    const maxLen = typeof e.maxLength === 'number' && e.maxLength > 0 ? e.maxLength : 0;
    if ((inputMode === 'numeric' || type === 'tel' || type === 'number') && maxLen > 0 && maxLen <= 8) {
      return 'short-numeric';
    }
    return '';
  };

  const describe = (el: Element): Record<string, unknown> => {
    const e = el as HTMLInputElement;
    const t = { description: labelOf(el), tag: el.tagName.toLowerCase(), selector: selectorOf(el) } as Record<
      string,
      unknown
    >;
    if (e.type) t.type = String(e.type).toLowerCase();
    return t;
  };

  const readValue = (el: Element): { value: string; redacted: boolean; reason: string } => {
    const reason = sensitivityOf(el);
    if (reason) return { value: `<redacted:${reason}>`, redacted: true, reason };

    const e = el as HTMLInputElement;
    const raw =
      e.value !== undefined && e.value !== null
        ? String(e.value)
        : ((el as HTMLElement).innerText || '');
    // Anything shaped like a card / account number goes regardless of labelling.
    if (/^\d{12,}$/.test(raw.replace(/[\s-]/g, ''))) {
      return { value: '<redacted:long-number>', redacted: true, reason: 'long-number' };
    }
    return { value: raw.slice(0, 200), redacted: false, reason: '' };
  };

  const isTextEntry = (el: Element): boolean => {
    const tag = el.tagName.toLowerCase();
    if (tag === 'textarea' || tag === 'select') return true;
    if (tag === 'input') {
      const t = ((el as HTMLInputElement).type || 'text').toLowerCase();
      return !['submit', 'button', 'reset', 'file', 'image'].includes(t);
    }
    return (el as HTMLElement).isContentEditable === true;
  };

  const flushPending = (): void => {
    if (pendingTimer) {
      clearTimeout(pendingTimer);
      pendingTimer = 0;
    }
    const el = pendingEl;
    pendingEl = null;
    if (!el || !el.isConnected) return;

    const tag = el.tagName.toLowerCase();
    const { value, redacted, reason } = readValue(el);
    if (!value) return;
    if (lastValue.get(el) === value) return;
    lastValue.set(el, value);

    const action: Record<string, unknown> = {
      kind: tag === 'select' ? 'select' : 'input',
      target: describe(el),
      value,
    };
    if (redacted) {
      action.redacted = true;
      action.detail = `redacted (${reason})`;
    }
    emit(action);
  };

  const noteInput = (el: Element): void => {
    if (pendingEl && pendingEl !== el) flushPending();
    pendingEl = el;
    if (pendingTimer) clearTimeout(pendingTimer);
    // A pause in typing is the cheapest signal that a field is "done" without
    // recording one action per keystroke.
    pendingTimer = setTimeout(flushPending, 900);
  };

  const CLICKABLE =
    'a,button,input,select,textarea,label,summary,option,[role=button],[role=link],[role=tab],[role=menuitem],[role=option],[role=checkbox],[contenteditable=true]';

  document.addEventListener(
    'click',
    (ev) => {
      const raw = ev.target as Element | null;
      if (!raw || !raw.tagName) return;
      const el = (raw.closest && raw.closest(CLICKABLE)) || raw;
      // Typed text belongs before the click that acts on it.
      flushPending();
      const action: Record<string, unknown> = { kind: 'click', target: describe(el) };
      const href = el.getAttribute && el.getAttribute('href');
      if (href) action.detail = `href ${href.slice(0, 200)}`;
      emit(action);
    },
    true,
  );

  document.addEventListener(
    'input',
    (ev) => {
      const el = ev.target as Element | null;
      if (el && el.tagName && isTextEntry(el)) noteInput(el);
    },
    true,
  );

  document.addEventListener(
    'change',
    (ev) => {
      const el = ev.target as Element | null;
      if (!el || !el.tagName || !isTextEntry(el)) return;
      pendingEl = el;
      flushPending();
    },
    true,
  );

  document.addEventListener(
    'focusout',
    (ev) => {
      if (ev.target === pendingEl) flushPending();
    },
    true,
  );

  document.addEventListener(
    'submit',
    (ev) => {
      flushPending();
      const form = ev.target as Element | null;
      emit({ kind: 'submit', target: form ? describe(form) : undefined });
    },
    true,
  );

  document.addEventListener(
    'keydown',
    (ev) => {
      // Only named keys are recorded, never characters: logging keystrokes
      // would reconstruct a password one character at a time.
      const key = (ev as KeyboardEvent).key;
      if (!['Enter', 'Escape', 'Tab'].includes(key)) return;
      if (key === 'Enter') flushPending();
      const el = (ev.target as Element | null) || null;
      emit({ kind: 'key', detail: key, target: el && el.tagName ? describe(el) : undefined });
    },
    true,
  );

  window.addEventListener(
    'scroll',
    () => {
      const now = Date.now();
      if (now - lastScrollAt < 1000) return;
      lastScrollAt = now;
      const y = window.scrollY;
      if (Math.abs(y - lastScrollY) < 40) return;
      const direction = y > lastScrollY ? 'down' : 'up';
      lastScrollY = y;
      emit({ kind: 'scroll', detail: `${direction} to ${Math.round(y)}px` });
    },
    true,
  );

  if (window.top === window) {
    const navigated = (detail: string) =>
      emit({
        kind: 'navigate',
        detail,
        target: { description: clean(document.title) || location.href, tag: 'document' },
      });

    if (document.readyState === 'loading') {
      // Wait for the title so the log names the page rather than just its URL.
      document.addEventListener('DOMContentLoaded', () => navigated('load'), { once: true });
    } else {
      navigated('load');
    }

    // SPA route changes never fire a load event, and a workflow that stays in
    // one document would otherwise look like it never moved.
    const wrap = (name: 'pushState' | 'replaceState') => {
      const orig = (history as any)[name];
      (history as any)[name] = function (this: History, ...args: unknown[]) {
        const out = orig.apply(this, args);
        setTimeout(() => navigated('in-page'), 0);
        return out;
      };
    };
    wrap('pushState');
    wrap('replaceState');
    window.addEventListener('popstate', () => setTimeout(() => navigated('back/forward'), 0));
  }
}

// ---------------------------------------------------------------------------
// Live session state
// ---------------------------------------------------------------------------

interface LiveSession {
  id: string;
  startedAtMs: number;
  actions: RecordedAction[];
  seen: Set<string>;
  capTimer: NodeJS.Timeout;
  pollTimer: NodeJS.Timeout;
  onAutoStop?: (status: TeachStatus) => void;
}

let live: LiveSession | undefined;
/** The context the recorder is already armed on, so re-arming is a no-op. */
let armedContext: BrowserContext | undefined;

/** SQLite writes `YYYY-MM-DD HH:MM:SS` in UTC; `new Date()` would read it local. */
function parseSqlUtc(value: string): number {
  const iso = /Z|[+-]\d{2}:?\d{2}$/.test(value) ? value : `${value.replace(' ', 'T')}Z`;
  const ms = Date.parse(iso);
  return Number.isNaN(ms) ? Date.now() : ms;
}

async function attachRecorder(): Promise<BrowserContext> {
  const ctx = await connect();

  // Arming is once per context: playwright-core can neither un-expose a binding
  // nor remove an init script, so re-arming would stack duplicates on every
  // recording. The recorder itself decides whether events still count.
  if (armedContext !== ctx) {
    await ctx
      .exposeBinding(BINDING, (_source, payload: unknown) => {
        ingest([payload]);
        return true;
      })
      .catch((err: unknown) => {
        if (!/already registered/i.test(String(err))) throw err;
      });
    await ctx.addInitScript(installRecorder, {
      binding: BINDING,
      stateKey: STATE_KEY,
      sensitivePattern: SENSITIVE_HINT_PATTERN,
    });
    armedContext = ctx;
  }

  // addInitScript only affects documents created from now on, so every document
  // that is already open needs an explicit install (and a previous recording
  // will have switched them off).
  await activePage();
  for (const frame of openFrames(ctx)) {
    await frame
      .evaluate(installRecorder, {
        binding: BINDING,
        stateKey: STATE_KEY,
        sensitivePattern: SENSITIVE_HINT_PATTERN,
      })
      .catch(() => {
        /* chrome:// pages and mid-navigation contexts cannot be recorded */
      });
  }
  return ctx;
}

function openFrames(ctx: BrowserContext): Frame[] {
  const frames: Frame[] = [];
  for (const page of ctx.pages()) {
    if (!page.isClosed()) frames.push(...page.frames());
  }
  return frames;
}

/** Flip every already-installed recorder off without waiting for a reload. */
async function disarmRecorders(): Promise<void> {
  const ctx = await connect().catch(() => undefined);
  if (!ctx) return;
  for (const frame of openFrames(ctx)) {
    await frame
      .evaluate((key) => {
        const s = (window as unknown as Record<string, any>)[key];
        if (s) s.on = false;
      }, STATE_KEY)
      .catch(() => {});
  }
}

/**
 * Pull anything the binding could not deliver — a document that loaded before
 * the binding was live, or a frame whose send raced a navigation.
 */
async function drain(): Promise<void> {
  const ctx = await connect().catch(() => undefined);
  if (!ctx || !live) return;

  for (const frame of openFrames(ctx)) {
    const batch = await frame
      .evaluate((key) => {
        const s = (window as unknown as Record<string, any>)[key];
        if (!s || !s.queue || !s.queue.length) return [];
        const out = s.queue.slice();
        s.queue.length = 0;
        return out;
      }, STATE_KEY)
      .catch(() => [] as unknown[]);
    if (Array.isArray(batch) && batch.length) ingest(batch);
  }
}

function ingest(batch: unknown[]): void {
  if (!live) return;
  let added = false;
  for (const raw of batch) {
    const action = sanitize(raw);
    if (!action || live.seen.has(action.id)) continue;
    live.seen.add(action.id);
    live.actions.push(action);
    added = true;
  }
  if (!added) return;
  // Array#sort is stable, so equal timestamps keep arrival order.
  live.actions.sort((a, b) => a.at.localeCompare(b.at));
  db.saveRecordingActions(live.id, live.actions);
}

// ---------------------------------------------------------------------------
// Node-side redaction (second pass)
// ---------------------------------------------------------------------------

function sanitize(raw: unknown): RecordedAction | undefined {
  if (!raw || typeof raw !== 'object') return undefined;
  const r = raw as Record<string, unknown>;
  if (typeof r.id !== 'string' || typeof r.kind !== 'string') return undefined;

  const t = (r.target ?? undefined) as Record<string, unknown> | undefined;
  const target: ActionTarget | undefined = t
    ? {
        description: str(t.description).slice(0, 120),
        tag: str(t.tag).slice(0, 20) || 'unknown',
        ...(t.type ? { type: str(t.type).slice(0, 20) } : {}),
        ...(t.selector ? { selector: str(t.selector).slice(0, 200) } : {}),
      }
    : undefined;

  const action: RecordedAction = {
    id: r.id,
    at: typeof r.at === 'string' ? r.at : new Date().toISOString(),
    kind: r.kind as ActionKind,
    url: scrubUrl(str(r.url)),
    ...(target ? { target } : {}),
    ...(r.detail ? { detail: str(r.detail).slice(0, 200) } : {}),
  };

  if (r.value !== undefined) {
    const redaction = redactValue(str(r.value), target, r.redacted === true);
    action.value = redaction.value;
    if (redaction.redacted) {
      action.redacted = true;
      if (!action.detail) action.detail = 'redacted';
    }
  }
  return action;
}

/**
 * Re-decides redaction from the metadata alone. The page already redacted, but
 * a value that reaches here in the clear when the target looks sensitive means
 * the in-page classifier was bypassed — so it is redacted again, not trusted.
 */
function redactValue(
  value: string,
  target: ActionTarget | undefined,
  alreadyRedacted: boolean,
): { value: string; redacted: boolean } {
  if (alreadyRedacted) {
    return value.startsWith('<redacted:')
      ? { value, redacted: true }
      : { value: '<redacted:sensitive>', redacted: true };
  }
  if (/^<redacted:[a-z-]+>$/.test(value)) return { value, redacted: true };

  const hint = [target?.description, target?.type, target?.selector].filter(Boolean).join(' ');
  if (target?.type === 'password' || SENSITIVE_HINT_RE.test(hint)) {
    return { value: '<redacted:sensitive>', redacted: true };
  }
  if (/^\d{12,}$/.test(value.replace(/[\s-]/g, ''))) {
    return { value: '<redacted:long-number>', redacted: true };
  }
  // High-entropy-looking blobs are far more likely to be a token than prose.
  if (/^[A-Za-z0-9_-]{24,}$/.test(value) && /\d/.test(value) && /[A-Za-z]/.test(value)) {
    return { value: '<redacted:secret-like>', redacted: true };
  }
  return { value: value.slice(0, 200), redacted: false };
}

/** Redirect URLs routinely carry the credential in the query string. */
export function scrubUrl(raw: string): string {
  if (!raw) return '';
  if (/^data:/i.test(raw)) return raw.length > 200 ? `${raw.slice(0, 200)}…` : raw;
  try {
    const url = new URL(raw);
    let touched = false;
    for (const key of Array.from(url.searchParams.keys())) {
      if (SENSITIVE_PARAM_RE.test(key)) {
        url.searchParams.set(key, 'REDACTED');
        touched = true;
      }
    }
    if (url.hash && /(access_token|id_token|[?&#]code=)/i.test(url.hash)) {
      url.hash = '#REDACTED';
      touched = true;
    }
    return (touched ? url.toString() : raw).slice(0, 500);
  } catch {
    return raw.slice(0, 500);
  }
}

function str(v: unknown): string {
  return typeof v === 'string' ? v : v === undefined || v === null ? '' : String(v);
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

export interface StartOptions {
  /** Fired when the ten-minute cap stops the recording on its own. */
  onAutoStop?: (status: TeachStatus) => void;
}

/**
 * Begin recording the human's demonstration in the container's browser.
 *
 * One recording at a time, because one browser is being demonstrated in and
 * two overlapping logs of the same clicks would compile into nonsense.
 */
export async function startRecording(label: string, opts: StartOptions = {}): Promise<TeachStatus> {
  const name = label.trim();
  if (!name) throw new Error('A recording needs a label — it becomes the draft skill name.');

  const existing = await enforceCap(db.activeRecording());
  if (existing && existing.status === 'recording') {
    throw new Error(
      `A recording is already running ("${existing.label}", ${existing.id}). Stop it before starting another.`,
    );
  }

  const recording = db.createRecording(name);
  try {
    await attachRecorder();
  } catch (err) {
    // Never leave a row claiming to be recording when nothing is attached.
    db.stopRecording(recording.id);
    throw new Error(
      `Could not attach the recorder to the container's browser (${(err as Error).message}). Is the computer up?`,
    );
  }

  live = {
    id: recording.id,
    startedAtMs: Date.now(),
    actions: [],
    seen: new Set<string>(),
    onAutoStop: opts.onAutoStop,
    capTimer: setTimeout(() => {
      void stopRecording(recording.id).then((status) => opts.onAutoStop?.(status)).catch(() => {});
    }, MAX_RECORDING_MS),
    pollTimer: setInterval(() => void drain().catch(() => {}), 2000),
  };

  return statusOf(db.getRecording(recording.id)!);
}

/**
 * Stop recording and persist the final action log. Safe to call on a recording
 * this process does not own (e.g. one orphaned by a crashed CLI).
 */
export async function stopRecording(id?: string): Promise<TeachStatus> {
  const target = id ?? live?.id ?? db.activeRecording()?.id;
  if (!target) throw new Error('No recording to stop.');

  const recording = db.getRecording(target);
  if (!recording) throw new Error(`No recording ${target}`);

  if (live && live.id === target) {
    clearTimeout(live.capTimer);
    clearInterval(live.pollTimer);
    await drain().catch(() => {});
    db.saveRecordingActions(live.id, live.actions);
    live = undefined;
    await disarmRecorders().catch(() => {});
  }

  if (recording.status === 'recording') db.stopRecording(target);
  return statusOf(db.getRecording(target)!);
}

/** The recorded actions, redacted, oldest first. */
export async function getActions(id?: string): Promise<RecordedAction[]> {
  const recording = await requireRecording(id);
  if (live && live.id === recording.id) {
    await drain().catch(() => {});
    return live.actions.slice();
  }
  return parseActions(recording.actions);
}

export async function teachStatus(id?: string): Promise<TeachStatus> {
  const recording = await requireRecording(id);
  if (live && live.id === recording.id) await drain().catch(() => {});
  return statusOf(db.getRecording(recording.id)!);
}

export function listTeachRecordings(): TeachStatus[] {
  return db.listRecordings().map(statusOf);
}

function statusOf(recording: db.Recording): TeachStatus {
  const isLive = live?.id === recording.id;
  const startedMs = isLive ? live!.startedAtMs : parseSqlUtc(recording.started_at);
  return {
    id: recording.id,
    label: recording.label,
    status: recording.status,
    actionCount: isLive ? live!.actions.length : parseActions(recording.actions).length,
    startedAt: recording.started_at,
    stoppedAt: recording.stopped_at,
    skillId: recording.skill_id,
    msRemaining:
      recording.status === 'recording' ? Math.max(0, startedMs + MAX_RECORDING_MS - Date.now()) : null,
    live: isLive,
  };
}

function parseActions(json: string): RecordedAction[] {
  try {
    const parsed = JSON.parse(json || '[]');
    return Array.isArray(parsed) ? (parsed as RecordedAction[]) : [];
  } catch {
    return [];
  }
}

async function requireRecording(id?: string): Promise<db.Recording> {
  // Falling back to the most recent recording matters for the normal flow:
  // start → stop → getActions → compile. After the stop there is no *running*
  // recording, so resolving only against the active one made the very next call
  // throw, and the caller had to have kept the id from three steps earlier.
  const target = id ?? live?.id ?? db.activeRecording()?.id ?? db.listRecordings()[0]?.id;
  if (!target) throw new Error('No recording specified and none exists yet.');
  const recording = db.getRecording(target);
  if (!recording) throw new Error(`No recording ${target}`);
  return (await enforceCap(recording))!;
}

/**
 * The cap is also enforced lazily on every read: the timer only exists inside
 * the process that started the recording, and that process may be long gone.
 */
async function enforceCap(recording: db.Recording | undefined): Promise<db.Recording | undefined> {
  if (!recording || recording.status !== 'recording') return recording;
  const startedMs = live?.id === recording.id ? live.startedAtMs : parseSqlUtc(recording.started_at);
  if (Date.now() - startedMs < MAX_RECORDING_MS) return recording;
  await stopRecording(recording.id).catch(() => {
    db.stopRecording(recording.id);
  });
  return db.getRecording(recording.id);
}

// ---------------------------------------------------------------------------
// Compilation
// ---------------------------------------------------------------------------

const COMPILE_SYSTEM = `You turn a recorded browser demonstration into a reusable Skill for an AI teammate that drives a real browser.

The log is what a human actually did, in order. Write the skill so a competent agent could repeat the workflow on a different day with different data — describe intent ("open the orders page", "search for the customer"), not coordinates or replay steps. Selectors are context, not instructions.

Rules:
- Values shown as <redacted:...> were secrets (passwords, one-time codes, card details). You do not know them and must never guess. Any step that needed one must instead tell the bot to stop and hand control to the human.
- Note which concrete values in the demonstration are inputs that will change each run.
- Ignore incidental noise: stray scrolls, mis-clicks the human corrected, focus changes that led nowhere.
- Safety boundaries must reflect what this workflow actually touches: anything irreversible or public (sending, posting, purchasing, deleting, accepting terms), any authentication step, and any point where the bot should confirm with a human first.

Reply with JSON only, no prose, in this shape:
{
  "name": "short-kebab-or-title-case name",
  "trigger": "one sentence: when a bot should use this skill",
  "steps": ["step 1", "step 2", "..."],
  "safety_rules": ["rule 1", "rule 2"],
  "notes": "optional caveats, or omit"
}`;

const EMIT_TOOL: ToolDefinition = {
  name: 'emit_skill',
  description: 'Return the compiled skill draft.',
  parameters: {
    type: 'object',
    properties: {
      name: { type: 'string', description: 'Short skill name.' },
      trigger: { type: 'string', description: 'When a bot should use this skill.' },
      steps: { type: 'array', items: { type: 'string' }, description: 'Ordered steps.' },
      safety_rules: {
        type: 'array',
        items: { type: 'string' },
        description: 'Boundaries the bot must respect while running this skill.',
      },
      notes: { type: 'string', description: 'Optional caveats.' },
    },
    required: ['name', 'trigger', 'steps', 'safety_rules'],
  },
};

export interface CompileOptions {
  /** A provider name (resolved through the vault) or an already-built provider. */
  provider: ProviderName | ModelProvider;
  model?: string;
  passphrase?: string;
  /** Cap on actions sent to the model. Default 400. */
  maxActions?: number;
}

/**
 * Compile a recording into a draft skill.
 *
 * Deliberately does NOT persist: the human reviews and edits the draft first,
 * then calls `saveCompiledSkill`. A model's reading of a demonstration is a
 * proposal, not a decision.
 */
export async function compileRecording(id: string, opts: CompileOptions): Promise<SkillDraft> {
  const recording = await requireRecording(id);
  // Compiling a live recording would race the drain, so close it first.
  if (recording.status === 'recording') await stopRecording(recording.id);

  const actions = await getActions(recording.id);
  if (!actions.length) {
    throw new Error(
      `Recording "${recording.label}" captured no actions. Was the demonstration done in the container's browser (noVNC on http://127.0.0.1:6080/vnc.html)?`,
    );
  }

  const provider =
    typeof opts.provider === 'string' ? getProvider(opts.provider, opts.passphrase) : opts.provider;

  const prompt =
    `Demonstration label: ${recording.label}\n` +
    `Actions recorded: ${actions.length}\n` +
    `Duration: ${describeDuration(actions)}\n\n` +
    `Action log:\n${formatActions(actions, opts.maxActions ?? 400)}`;

  let text = '';
  let structured: Record<string, unknown> | undefined;

  for await (const chunk of provider.chat([{ role: 'user', content: prompt }], {
    model: opts.model,
    system: COMPILE_SYSTEM,
    tools: [EMIT_TOOL],
    maxTokens: 4000,
  })) {
    if (chunk.type === 'text_delta') text += chunk.text;
    else if (chunk.type === 'tool_call' && chunk.name === EMIT_TOOL.name) structured = chunk.input;
    else if (chunk.type === 'error') throw new Error(`Compilation failed: ${chunk.error}`);
  }

  const raw = structured ?? extractJson(text);
  if (!raw) {
    throw new Error(
      `The model did not return a usable skill draft. It said:\n${text.trim().slice(0, 500) || '(nothing)'}`,
    );
  }
  return normalizeDraft(raw, recording.label);
}

/** Persist a reviewed draft as a `taught` skill and link it to its recording. */
export function saveCompiledSkill(recordingId: string, draft: SkillDraft): db.Skill {
  const recording = db.getRecording(recordingId);
  if (!recording) throw new Error(`No recording ${recordingId}`);

  const clean = normalizeDraft(draft as unknown as Record<string, unknown>, recording.label);
  const skill = db.createSkill({
    name: uniqueSkillName(clean.name),
    instructions: skillInstructions(clean),
    source: 'taught',
    safetyRules: clean.safetyRules.join('\n'),
  });
  db.attachRecordingSkill(recordingId, skill.id);
  return skill;
}

/** skills.name is UNIQUE; suffix rather than lose a draft the human just reviewed. */
function uniqueSkillName(name: string): string {
  if (!db.getSkillByName(name)) return name;
  for (let n = 2; n < 100; n++) {
    const candidate = `${name}-${n}`;
    if (!db.getSkillByName(candidate)) return candidate;
  }
  throw new Error(`Too many skills already named "${name}".`);
}

/** The body stored in `skills.instructions`; safety rules live in their own column. */
export function skillInstructions(draft: SkillDraft): string {
  const lines = [`Use this when: ${draft.trigger}`, '', 'Steps:'];
  draft.steps.forEach((step, i) => lines.push(`${i + 1}. ${step}`));
  if (draft.notes?.trim()) lines.push('', `Notes: ${draft.notes.trim()}`);
  return lines.join('\n');
}

/** Full draft as review text, for the CLI/dashboard to show before saving. */
export function renderDraft(draft: SkillDraft): string {
  const lines = [`Name: ${draft.name}`, '', skillInstructions(draft), '', 'Safety boundaries:'];
  for (const rule of draft.safetyRules) lines.push(`- ${rule}`);
  return lines.join('\n');
}

function normalizeDraft(raw: Record<string, unknown>, fallbackName: string): SkillDraft {
  const steps = toStringList(raw.steps ?? (raw as Record<string, unknown>).step_sequence);
  if (!steps.length) {
    throw new Error(
      `The model's draft has no steps. Raw output:\n${JSON.stringify(raw).slice(0, 500)}`,
    );
  }
  const safetyRules = toStringList(raw.safety_rules ?? raw.safetyRules ?? raw.safety);
  const trigger = str(raw.trigger ?? raw.when ?? raw.description).trim();
  return {
    name: (str(raw.name).trim() || fallbackName).slice(0, 80),
    trigger: trigger || `Repeat the "${fallbackName}" workflow.`,
    steps,
    safetyRules: safetyRules.length
      ? safetyRules
      : ['Stop and hand control to a human before any irreversible or public action.'],
    ...(str(raw.notes).trim() ? { notes: str(raw.notes).trim() } : {}),
  };
}

/** Models return steps as strings, or as objects with the text on some key. */
function toStringList(value: unknown): string[] {
  if (typeof value === 'string') {
    return value
      .split('\n')
      .map((l) => l.replace(/^\s*(?:[-*]|\d+[.)])\s*/, '').trim())
      .filter(Boolean);
  }
  if (!Array.isArray(value)) return [];
  const out: string[] = [];
  for (const item of value) {
    if (typeof item === 'string') {
      const s = item.trim();
      if (s) out.push(s);
    } else if (item && typeof item === 'object') {
      const o = item as Record<string, unknown>;
      const s = str(o.step ?? o.text ?? o.description ?? o.instruction ?? o.action ?? o.rule).trim();
      if (s) out.push(s);
    }
  }
  return out;
}

function extractJson(text: string): Record<string, unknown> | undefined {
  const fenced = text.match(/```(?:json)?\s*([\s\S]*?)```/i);
  for (const candidate of [fenced?.[1], text]) {
    if (!candidate) continue;
    const start = candidate.indexOf('{');
    const end = candidate.lastIndexOf('}');
    if (start < 0 || end <= start) continue;
    try {
      const parsed = JSON.parse(candidate.slice(start, end + 1));
      if (parsed && typeof parsed === 'object') return parsed as Record<string, unknown>;
    } catch {
      /* try the next candidate */
    }
  }
  return undefined;
}

function describeDuration(actions: RecordedAction[]): string {
  if (actions.length < 2) return 'under a second';
  const ms = Date.parse(actions[actions.length - 1].at) - Date.parse(actions[0].at);
  const secs = Math.max(0, Math.round(ms / 1000));
  return secs < 60 ? `${secs}s` : `${Math.floor(secs / 60)}m ${secs % 60}s`;
}

export function formatActions(actions: RecordedAction[], max = 400): string {
  const shown = actions.slice(0, max);
  const t0 = shown.length ? Date.parse(shown[0].at) : 0;
  let lastUrl = '';

  const lines = shown.map((a, i) => {
    const secs = Math.max(0, Math.round((Date.parse(a.at) - t0) / 1000));
    const clock = `${String(Math.floor(secs / 60)).padStart(2, '0')}:${String(secs % 60).padStart(2, '0')}`;
    const parts = [`#${String(i + 1).padStart(3)}`, `[${clock}]`, a.kind.padEnd(8)];
    if (a.target?.description) parts.push(`"${a.target.description}"`);
    if (a.target?.tag) parts.push(`<${a.target.tag}${a.target.type ? `:${a.target.type}` : ''}>`);
    if (a.value !== undefined) parts.push(`= ${a.value}`);
    if (a.detail) parts.push(`(${a.detail})`);
    if (a.target?.selector) parts.push(`{${a.target.selector}}`);
    // Repeating the URL on every line is mostly wasted tokens; showing it only
    // when it changes makes the moments the workflow moved pages obvious.
    if (a.url && a.url !== lastUrl) {
      parts.push(`@ ${a.url.length > 120 ? `${a.url.slice(0, 120)}…` : a.url}`);
      lastUrl = a.url;
    }
    return parts.join(' ');
  });

  if (actions.length > shown.length) {
    lines.push(`… ${actions.length - shown.length} further actions omitted`);
  }
  return lines.join('\n');
}
