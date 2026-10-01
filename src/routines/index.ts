import {
  getBot,
  getRoutine,
  getSkill,
  listBots,
  listRoutines,
  markRoutineRun,
  recordRoutineRun,
  type Bot,
  type Routine,
  type Skill,
} from '../db/index.ts';
import { getProvider } from '../providers/index.ts';
import type { ModelProvider } from '../providers/types.ts';
import { parseSafetyRules, renderSkillPrompt, runSkill, type SkillArgs } from '../skills/index.ts';

/**
 * Routines — a skill bound to a trigger, either a cron schedule or a named event.
 *
 * The scheduler is a plain in-process poll loop. It has to survive the process
 * being down (routines are checked against wall-clock time, not against a timer
 * that only exists while we run) and it has to survive a routine that throws
 * every single time, so every firing is wrapped individually and logged to
 * `routine_runs` whether it succeeded or not.
 */

export type { Routine } from '../db/index.ts';

// --- cron -------------------------------------------------------------------

/** A malformed cron expression, with the offending field named. */
export class CronError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'CronError';
  }
}

export interface CronSpec {
  expression: string;
  minute: number[];
  hour: number[];
  dayOfMonth: number[];
  month: number[];
  /** 0-6, Sunday first. A `7` in the expression is normalised to `0`. */
  dayOfWeek: number[];
  /** Whether the field narrows anything, which decides the day-of-* OR rule below. */
  dayOfMonthRestricted: boolean;
  dayOfWeekRestricted: boolean;
}

interface FieldDef {
  name: string;
  min: number;
  max: number;
}

const FIELDS: FieldDef[] = [
  { name: 'minute', min: 0, max: 59 },
  { name: 'hour', min: 0, max: 23 },
  { name: 'day-of-month', min: 1, max: 31 },
  { name: 'month', min: 1, max: 12 },
  // 7 is accepted for Sunday, as every cron implementation does.
  { name: 'day-of-week', min: 0, max: 7 },
];

const EXAMPLE = 'e.g. "0 9 * * 1" = 09:00 every Monday';

/**
 * Standard 5-field cron: minute hour day-of-month month day-of-week.
 * Supports a star, a number, `a,b,c`, `a-b`, and a step suffix (`star/n`, `a-b/n`).
 * Names (JAN, MON) are not accepted — the numeric form is unambiguous and this
 * parser stays small.
 */
export function parseCron(expr: string): CronSpec {
  const expression = String(expr ?? '').trim();
  const parts = expression.split(/\s+/).filter(Boolean);

  if (parts.length !== 5) {
    throw new CronError(
      `A cron schedule has 5 fields (minute hour day-of-month month day-of-week); ` +
        `got ${parts.length} in "${expression}" — ${EXAMPLE}.`,
    );
  }

  const [minute, hour, dayOfMonth, month, dow] = parts.map((p, i) => parseField(p, FIELDS[i]));

  // Sunday is both 0 and 7 on the wire; collapse so matching only checks 0-6.
  const dayOfWeek = [...new Set(dow.map((d) => (d === 7 ? 0 : d)))].sort((a, b) => a - b);

  return {
    expression,
    minute,
    hour,
    dayOfMonth,
    month,
    dayOfWeek,
    dayOfMonthRestricted: dayOfMonth.length < 31,
    dayOfWeekRestricted: dayOfWeek.length < 7,
  };
}

function parseField(raw: string, field: FieldDef): number[] {
  const values = new Set<number>();

  for (const term of raw.split(',')) {
    if (!term) throw new CronError(fieldError(raw, field, 'has an empty item (a stray comma?)'));

    const [rangePart, stepPart, ...rest] = term.split('/');
    if (rest.length) throw new CronError(fieldError(raw, field, 'has more than one "/" in an item'));

    let step = 1;
    if (stepPart !== undefined) {
      step = Number(stepPart);
      if (!Number.isInteger(step) || step < 1) {
        throw new CronError(fieldError(raw, field, `has a bad step "/${stepPart}" — use a whole number ≥ 1`));
      }
    }

    let from: number;
    let to: number;
    if (rangePart === '*') {
      from = field.min;
      to = field.max;
    } else if (rangePart.includes('-')) {
      const [a, b] = rangePart.split('-');
      from = toNumber(a, raw, field);
      to = toNumber(b, raw, field);
      if (from > to) throw new CronError(fieldError(raw, field, `has a backwards range "${rangePart}"`));
    } else {
      from = toNumber(rangePart, raw, field);
      to = stepPart === undefined ? from : field.max;
    }

    for (let v = from; v <= to; v += step) values.add(v);
  }

  return [...values].sort((a, b) => a - b);
}

function toNumber(text: string, raw: string, field: FieldDef): number {
  const n = Number(text);
  if (!Number.isInteger(n)) {
    throw new CronError(fieldError(raw, field, `contains "${text}", which is not a whole number`));
  }
  if (n < field.min || n > field.max) {
    throw new CronError(fieldError(raw, field, `contains ${n}, outside ${field.min}-${field.max}`));
  }
  return n;
}

function fieldError(raw: string, field: FieldDef, problem: string): string {
  return `The ${field.name} field "${raw}" ${problem}. Allowed: ${field.min}-${field.max}, ` + `"*", "a,b", "a-b", "*/n" — ${EXAMPLE}.`;
}

/** Nothing legal is further away than the next leap-year 29 February. */
const MAX_LOOKAHEAD_DAYS = 366 * 5;

/**
 * The first time the expression matches strictly after `after`, in local time.
 *
 * Walks day by day and only then scans the hour/minute lists: a sparse schedule
 * like "30 3 1 1 *" is half a million minutes out but under 400 days, so
 * minute-stepping would burn six figures of iterations for one answer.
 */
export function nextFireTime(expr: string | CronSpec, after: Date = new Date()): Date {
  const spec = typeof expr === 'string' ? parseCron(expr) : expr;

  const cursor = new Date(after.getTime());
  cursor.setSeconds(0, 0);
  cursor.setMinutes(cursor.getMinutes() + 1);

  for (let day = 0; day <= MAX_LOOKAHEAD_DAYS; day++) {
    if (matchesDate(spec, cursor)) {
      const hit = firstTimeOnDay(spec, cursor);
      if (hit) return hit;
    }
    cursor.setDate(cursor.getDate() + 1);
    cursor.setHours(0, 0, 0, 0);
  }

  throw new CronError(
    `"${spec.expression}" never fires — check the day-of-month/month pair (30 February and friends).`,
  );
}

function matchesDate(spec: CronSpec, d: Date): boolean {
  if (!spec.month.includes(d.getMonth() + 1)) return false;

  const domOk = spec.dayOfMonth.includes(d.getDate());
  const dowOk = spec.dayOfWeek.includes(d.getDay());

  // Cron's one genuine oddity: when both day fields are narrowed, they are ORed,
  // so "0 0 1 * 1" means the 1st AND every Monday, not Mondays that fall on a 1st.
  if (spec.dayOfMonthRestricted && spec.dayOfWeekRestricted) return domOk || dowOk;
  return domOk && dowOk;
}

function firstTimeOnDay(spec: CronSpec, from: Date): Date | undefined {
  for (const hour of spec.hour) {
    if (hour < from.getHours()) continue;
    const earliest = hour === from.getHours() ? from.getMinutes() : 0;
    const minute = spec.minute.find((m) => m >= earliest);
    if (minute === undefined) continue;
    return new Date(from.getFullYear(), from.getMonth(), from.getDate(), hour, minute, 0, 0);
  }
  return undefined;
}

// --- time helpers -----------------------------------------------------------

/** SQLite's `datetime('now')` shape, so our writes match the DB's own. */
function toDbTime(d: Date): string {
  return d.toISOString().slice(0, 19).replace('T', ' ');
}

function parseDbTime(s: string): Date {
  // `datetime('now')` is UTC with no zone marker; Date would read it as local
  // time and silently shift every stored timestamp by the machine's offset.
  const iso = /[zZ]|[+-]\d\d:?\d\d$/.test(s) ? s : `${s.replace(' ', 'T')}Z`;
  return new Date(iso);
}

/**
 * When a scheduled routine is next due.
 *
 * Falls back to `created_at` when it has never run, so a routine created while
 * the scheduler was down still gets its first fire computed from a real instant
 * rather than from process start.
 */
export function nextDueAt(routine: Routine): Date {
  if (!routine.schedule) {
    throw new CronError(`Routine ${routine.id} is event-triggered ("${routine.trigger}"), not scheduled.`);
  }
  if (routine.next_run_at) return parseDbTime(routine.next_run_at);
  return nextFireTime(routine.schedule, parseDbTime(routine.last_run_at ?? routine.created_at));
}

// --- resolving what a routine points at -------------------------------------

export function routineSkill(routine: Routine): Skill {
  const skill = getSkill(routine.skill_id);
  if (!skill) throw new Error(`Routine ${routine.id} points at a deleted skill (${routine.skill_id}).`);
  return skill;
}

/**
 * Which bot runs this. A routine may leave `bot_id` null, in which case the
 * account's first bot takes it — every bot shares the one computer anyway, so
 * the choice is about persona and model, not about capability.
 */
export function routineBot(routine: Routine): Bot {
  if (routine.bot_id) {
    const bot = getBot(routine.bot_id);
    if (bot) return bot;
    throw new Error(`Routine ${routine.id} is assigned to a bot that no longer exists.`);
  }
  const first = listBots()[0];
  if (!first) throw new Error('No bots exist yet — create one first (mybot bots add <name> <provider>).');
  return first;
}

// --- running ----------------------------------------------------------------

export interface RunRoutineOptions {
  args?: SkillArgs;
  /** Overrides the generated "fired by …" note handed to the bot. */
  context?: string;
  passphrase?: string;
  resolveProvider?: (bot: Bot) => ModelProvider;
  maxIterations?: number;
  onEvent?: (kind: string, text: string) => void;
  now?: Date;
}

export interface RoutineOutcome {
  routineId: string;
  skillName?: string;
  botName?: string;
  taskId?: string;
  ok: boolean;
  summary: string;
  error?: string;
  /** For scheduled routines: when it will fire next, after this run. */
  nextFireAt?: Date;
}

/**
 * Fire one routine now. Never throws — a routine that cannot even be set up
 * (deleted skill, missing API key) is a logged failure, not a reason for the
 * scheduler or a webhook handler to fall over.
 */
export async function runRoutine(routine: Routine, opts: RunRoutineOptions = {}): Promise<RoutineOutcome> {
  const now = opts.now ?? new Date();

  // Advance the schedule *before* running. A skill run can outlast the poll
  // interval, and a routine that is still working must not be started again.
  let nextFireAt: Date | undefined;
  if (routine.schedule) {
    try {
      nextFireAt = nextFireTime(routine.schedule, now);
    } catch {
      // Broken expression: the run below still happens (it was already due),
      // but there is no next slot to record. reportBrokenCron surfaces it.
    }
  }
  markRoutineRun(routine.id, nextFireAt ? toDbTime(nextFireAt) : undefined);

  const outcome: RoutineOutcome = { routineId: routine.id, ok: false, summary: '', nextFireAt };

  try {
    const skill = routineSkill(routine);
    const bot = routineBot(routine);
    outcome.skillName = skill.name;
    outcome.botName = bot.name;

    const provider = (opts.resolveProvider ?? defaultProviderResolver(opts.passphrase))(bot);

    const result = await runSkill({
      skill,
      bot,
      provider,
      args: opts.args,
      maxIterations: opts.maxIterations,
      context: opts.context ?? firingContext(routine, now),
      onEvent: opts.onEvent,
    });

    outcome.taskId = result.taskId;
    outcome.summary = result.summary;

    // Only genuine failures go in the error column. The loop's status union is
    // expected to grow (a paused, waiting-on-a-human run is not a failure), so
    // test for the known-bad values rather than for "not done".
    const failed =
      result.status === 'error' || result.status === 'refused' || result.status === 'max_iterations';
    outcome.ok = !failed;
    if (failed) outcome.error = `${result.status}: ${result.summary}`;

    recordRoutineRun(routine.id, result.taskId, outcome.error);
    return outcome;
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    outcome.error = message;
    outcome.summary = message;
    recordRoutineRun(routine.id, undefined, message);
    return outcome;
  }
}

function defaultProviderResolver(passphrase?: string): (bot: Bot) => ModelProvider {
  return (bot) => getProvider(bot.provider, passphrase);
}

function firingContext(routine: Routine, now: Date): string {
  const how = routine.schedule
    ? `the schedule "${routine.schedule}"`
    : `the event "${routine.trigger}"`;
  return (
    `This run was started automatically by a routine (${how}) at ${now.toISOString()}. ` +
    'Nobody is watching it live. If you need a decision only a human can make, stop and ' +
    'say so in your summary instead of guessing.'
  );
}

// --- event triggers ---------------------------------------------------------

/**
 * Run every enabled routine listening for `name`.
 *
 * In-process by design: the dashboard owns the HTTP surface and calls this, so
 * nothing here opens a port. Matches are case-insensitive because event names
 * arrive from webhooks and other people's spelling.
 */
export async function fireEvent(
  name: string,
  payload?: unknown,
  opts: RunRoutineOptions = {},
): Promise<RoutineOutcome[]> {
  const wanted = String(name ?? '').trim().toLowerCase();
  if (!wanted) throw new Error('fireEvent needs an event name.');

  const matches = listRoutines().filter(
    (r) => r.enabled && r.trigger && r.trigger.trim().toLowerCase() === wanted,
  );

  const outcomes: RoutineOutcome[] = [];
  for (const routine of matches) {
    // Sequential on purpose: all bots share one browser and one /workspace, so
    // two routines running at once would fight over the same tabs.
    outcomes.push(await runRoutine(routine, { ...opts, args: opts.args ?? argsFromPayload(payload) }));
  }
  return outcomes;
}

function argsFromPayload(payload: unknown): SkillArgs | undefined {
  if (payload === undefined || payload === null) return undefined;
  if (typeof payload === 'string') return payload;
  if (typeof payload === 'object' && !Array.isArray(payload)) return payload as Record<string, unknown>;
  return { payload: JSON.stringify(payload) };
}

// --- dry run ----------------------------------------------------------------

export interface DryRunReport {
  routine: Routine;
  kind: 'schedule' | 'event';
  enabled: boolean;
  skill?: Skill;
  bot?: Bot;
  /** Scheduled routines only. */
  nextFireAt?: Date;
  /** The exact prompt that would be sent, when it can be built. */
  instruction?: string;
  alwaysConfirm: string[];
  problems: string[];
}

/**
 * What this routine *would* do, without doing it. The spec's advice is to prove a
 * skill out on a one-off task before promoting it to a routine; this is the last
 * check before that — it shows the real rendered prompt, the bot that would run
 * it, and the next fire time.
 */
export function dryRun(routine: Routine | string, args?: SkillArgs): DryRunReport {
  const r = typeof routine === 'string' ? requireRoutine(routine) : routine;

  const report: DryRunReport = {
    routine: r,
    kind: r.schedule ? 'schedule' : 'event',
    enabled: Boolean(r.enabled),
    alwaysConfirm: [],
    problems: [],
  };

  if (!r.enabled) report.problems.push('Disabled — it will not fire until it is enabled.');

  try {
    report.skill = routineSkill(r);
  } catch (err) {
    report.problems.push(err instanceof Error ? err.message : String(err));
  }

  try {
    report.bot = routineBot(r);
  } catch (err) {
    report.problems.push(err instanceof Error ? err.message : String(err));
  }

  if (r.schedule) {
    try {
      report.nextFireAt = nextDueAt(r);
    } catch (err) {
      report.problems.push(err instanceof Error ? err.message : String(err));
    }
  }

  if (report.skill) {
    report.alwaysConfirm = parseSafetyRules(report.skill).alwaysConfirm;
    report.instruction = renderSkillPrompt(report.skill, args, {
      context: firingContext(r, report.nextFireAt ?? new Date()),
    });
  }

  return report;
}

export function requireRoutine(id: string): Routine {
  const r = getRoutine(id);
  if (!r) throw new Error(`No routine ${id}.`);
  return r;
}

// --- scheduler --------------------------------------------------------------

export interface SchedulerOptions extends RunRoutineOptions {
  /** How often to sweep for due routines. Default 30s. */
  intervalMs?: number;
  onFire?: (outcome: RoutineOutcome) => void;
  onError?: (error: Error, routine?: Routine) => void;
  /** Injectable clock, for tests. */
  clock?: () => Date;
}

export interface SchedulerHandle {
  stop(): void;
  /** Run one sweep by hand (the scheduler's own loop calls the same code). */
  tick(): Promise<RoutineOutcome[]>;
}

interface SchedulerState {
  active: boolean;
  intervalMs: number;
  opts: SchedulerOptions;
  clock: () => Date;
  inFlight: Set<string>;
  brokenReported: Set<string>;
  ticking: boolean;
}

let state: SchedulerState | undefined;
let timer: ReturnType<typeof setTimeout> | undefined;

export function startScheduler(opts: SchedulerOptions = {}): SchedulerHandle {
  if (state?.active) throw new Error('The scheduler is already running in this process.');

  const current: SchedulerState = {
    active: true,
    intervalMs: opts.intervalMs ?? 30_000,
    opts,
    clock: opts.clock ?? (() => new Date()),
    inFlight: new Set(),
    brokenReported: new Set(),
    ticking: false,
  };
  state = current;

  const loop = async (): Promise<void> => {
    try {
      await sweep(current);
    } catch (err) {
      // sweep() already catches per routine; this is only for a failure in the
      // sweep itself (a bad DB read). Losing the loop here would be silent death.
      current.opts.onError?.(err instanceof Error ? err : new Error(String(err)));
    }
    // Re-arm after the sweep rather than on a fixed interval, so a long run
    // cannot queue up a backlog of overlapping sweeps.
    if (current.active) timer = setTimeout(() => void loop(), current.intervalMs);
  };

  // Sweep immediately: anything that came due while the process was down should
  // not wait out a full interval first.
  timer = setTimeout(() => void loop(), 0);

  return {
    stop: stopScheduler,
    tick: () => sweep(current),
  };
}

export function stopScheduler(): void {
  if (timer) clearTimeout(timer);
  timer = undefined;
  if (state) state.active = false;
  state = undefined;
}

export function isSchedulerRunning(): boolean {
  return Boolean(state?.active);
}

async function sweep(current: SchedulerState): Promise<RoutineOutcome[]> {
  if (current.ticking) return [];
  current.ticking = true;

  const now = current.clock();
  const fired: RoutineOutcome[] = [];

  try {
    for (const routine of listRoutines()) {
      if (!routine.enabled || !routine.schedule) continue;
      if (current.inFlight.has(routine.id)) continue;

      let due: Date;
      try {
        due = nextDueAt(routine);
      } catch (err) {
        reportBrokenCron(current, routine, err);
        continue;
      }
      if (due > now) continue;

      // A routine that missed several slots while we were down fires ONCE:
      // runRoutine recomputes the next slot from `now`, not from the slot it
      // missed, so the backlog collapses instead of stampeding.
      current.inFlight.add(routine.id);
      try {
        const outcome = await runRoutine(routine, { ...current.opts, now });
        fired.push(outcome);
        current.opts.onFire?.(outcome);
        if (outcome.error) {
          current.opts.onError?.(new Error(outcome.error), routine);
        }
      } catch (err) {
        // runRoutine is meant to swallow everything; if it ever does not, one
        // bad routine still must not take the scheduler down with it.
        current.opts.onError?.(err instanceof Error ? err : new Error(String(err)), routine);
      } finally {
        current.inFlight.delete(routine.id);
      }
    }
  } finally {
    current.ticking = false;
  }

  return fired;
}

/**
 * A cron expression that will not parse is broken until someone edits it, and
 * the sweep would otherwise write the same error to routine_runs every interval.
 * Log it once per process and keep going.
 */
function reportBrokenCron(current: SchedulerState, routine: Routine, err: unknown): void {
  if (current.brokenReported.has(routine.id)) return;
  current.brokenReported.add(routine.id);

  const message = err instanceof Error ? err.message : String(err);
  recordRoutineRun(routine.id, undefined, message);
  current.opts.onError?.(err instanceof Error ? err : new Error(message), routine);
}
