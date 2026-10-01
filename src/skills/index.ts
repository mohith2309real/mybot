import {
  createSkill,
  createTask,
  deleteSkill,
  getSkill,
  getSkillByName,
  listSkills,
  updateSkill,
  type Bot,
  type Skill,
} from '../db/index.ts';
import { runTask, type RunResult } from '../agent/loop.ts';
import type { ModelProvider } from '../providers/types.ts';

/**
 * Skills — a named, reusable set of instructions a bot can invoke.
 *
 * A skill is natural language, not code: the decision rules and boundaries a bot
 * should have in front of it while it works. Rendering one produces the
 * instruction text handed to the agent loop.
 *
 * Skills only ever come from two places — hand-written here, or compiled from a
 * teach-a-task recording — and the `skills.source` CHECK constraint enforces it.
 * There is deliberately no registry, no URL import, nothing fetched from the
 * internet: on 1 Feb 2026 Koi Security found 341 of 2,857 skills in OpenClaw's
 * ClawHub registry were malicious (824 by 16 Feb), most shipping the AMOS
 * infostealer. A skill is instructions a model will follow with a real browser
 * and a real shell, so importing one is equivalent to running a stranger's code.
 */

export type { Skill } from '../db/index.ts';
export { listSkills } from '../db/index.ts';

/** Values substituted into a skill's `{{placeholders}}`, or a single free-text input. */
export type SkillArgs = string | Record<string, unknown>;

export interface SafetyRules {
  /** Rules tagged `always-confirm`: the bot must stop and get a human before acting. */
  alwaysConfirm: string[];
  other: string[];
}

/** A user-facing problem with a skill (bad name, duplicate, missing instructions). */
export class SkillError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'SkillError';
  }
}

// --- CRUD with validation ---------------------------------------------------

export interface SkillInput {
  name: string;
  instructions: string;
  source?: 'manual' | 'taught';
  /** One rule per line, or a list. `always-confirm` marks a rule as human-gated. */
  safetyRules?: string | string[] | null;
}

export function addSkill(input: SkillInput): Skill {
  const name = (input.name ?? '').trim();
  const instructions = (input.instructions ?? '').trim();

  if (!name) throw new SkillError('A skill needs a name.');
  if (/\s{2,}|[\n\r]/.test(name)) throw new SkillError('A skill name must be a single line.');
  if (!instructions) {
    throw new SkillError(`Skill "${name}" has no instructions — the instructions are the skill.`);
  }
  if (getSkillByName(name)) throw new SkillError(duplicateMessage(name));

  try {
    return createSkill({
      name,
      instructions,
      source: input.source ?? 'manual',
      safetyRules: serializeSafetyRules(input.safetyRules),
    });
  } catch (err) {
    // The pre-check above loses a race in theory (and always, if two processes
    // write at once). Translate the raw constraint text either way.
    if (String(err).includes('UNIQUE')) throw new SkillError(duplicateMessage(name));
    throw err;
  }
}

/**
 * Edit instructions and/or safety rules. Renaming is not supported: routines and
 * recordings point at a skill by id, but humans refer to it by name, so a rename
 * would silently change what every `mybot routines` line means.
 */
export function editSkill(
  idOrName: string,
  patch: { instructions?: string; safetyRules?: string | string[] | null },
): Skill {
  const skill = resolveSkill(idOrName);

  if (patch.instructions !== undefined && !patch.instructions.trim()) {
    throw new SkillError(`Skill "${skill.name}" cannot be left with empty instructions.`);
  }

  updateSkill(skill.id, {
    ...(patch.instructions === undefined ? {} : { instructions: patch.instructions.trim() }),
    ...(patch.safetyRules === undefined
      ? {}
      : { safetyRules: serializeSafetyRules(patch.safetyRules) ?? null }),
  });

  return getSkill(skill.id)!;
}

export function removeSkill(idOrName: string): Skill {
  const skill = resolveSkill(idOrName);
  deleteSkill(skill.id);
  return skill;
}

/** Look a skill up by id or by name. Returns undefined rather than throwing. */
export function findSkill(idOrName: string): Skill | undefined {
  const key = (idOrName ?? '').trim();
  if (!key) return undefined;
  return getSkill(key) ?? getSkillByName(key);
}

export function resolveSkill(idOrName: string): Skill {
  const skill = findSkill(idOrName);
  if (skill) return skill;
  const known = listSkills().map((s) => s.name);
  throw new SkillError(
    `No skill "${idOrName}".` + (known.length ? ` Known skills: ${known.join(', ')}` : ' None defined yet.'),
  );
}

function duplicateMessage(name: string): string {
  return `A skill named "${name}" already exists. Edit that one, or pick another name.`;
}

// --- safety rules -----------------------------------------------------------

export const ALWAYS_CONFIRM = 'always-confirm';

/**
 * Ways a rule can carry the tag: `always-confirm: <rule>`, `[always-confirm] <rule>`,
 * `<rule> (always-confirm)`, or a rule that simply starts "always confirm …".
 * The last form keeps its full wording — stripping the verb out of a sentence
 * would leave a fragment like "before sending mail".
 */
const TAG_SUFFIX = /\s*[([]\s*always[-_ ]?confirm\s*[)\]]\s*$/i;
const TAG_PREFIX = /^\[?\s*always[-_ ]?confirm\s*\]?\s*[:–—-]\s*/i;
const TAG_BRACKET = /^\[\s*always[-_ ]?confirm\s*\]\s*/i;
const TAG_PHRASE = /^always[-_ ]?confirm\b/i;

export function parseSafetyRules(input: Skill | string | null | undefined): SafetyRules {
  const raw = typeof input === 'string' || input == null ? input : input.safety_rules;
  const out: SafetyRules = { alwaysConfirm: [], other: [] };
  if (!raw || !raw.trim()) return out;

  for (const line of splitRules(raw)) {
    const rule = classifyRule(line.text);
    if (!rule) continue;
    if (line.alwaysConfirm || rule.alwaysConfirm) out.alwaysConfirm.push(rule.text);
    else out.other.push(rule.text);
  }
  return out;
}

/**
 * Rules are stored as one per line, but teach-a-task compiles rules structurally,
 * so a JSON array is accepted too rather than forcing that side to flatten first.
 */
function splitRules(raw: string): { text: string; alwaysConfirm: boolean }[] {
  const trimmed = raw.trim();
  if (trimmed.startsWith('[')) {
    try {
      const parsed = JSON.parse(trimmed) as unknown[];
      return parsed.map((entry) => {
        if (typeof entry === 'string') return { text: entry, alwaysConfirm: false };
        const obj = (entry ?? {}) as Record<string, unknown>;
        return {
          text: String(obj.rule ?? obj.text ?? ''),
          alwaysConfirm: Boolean(obj.alwaysConfirm ?? obj.always_confirm),
        };
      });
    } catch {
      // Not JSON after all — fall through to line splitting.
    }
  }
  return trimmed.split(/\r?\n/).map((text) => ({ text, alwaysConfirm: false }));
}

function classifyRule(raw: string): { text: string; alwaysConfirm: boolean } | undefined {
  const line = raw.replace(/^\s*[-*•]\s*/, '').trim();
  if (!line) return undefined;

  const suffix = TAG_SUFFIX.exec(line);
  if (suffix) return tagged(line.slice(0, suffix.index), line);

  const bracket = TAG_BRACKET.exec(line);
  if (bracket) return tagged(line.slice(bracket[0].length), line);

  const prefix = TAG_PREFIX.exec(line);
  if (prefix) return tagged(line.slice(prefix[0].length), line);

  if (TAG_PHRASE.test(line)) return { text: line, alwaysConfirm: true };
  return { text: line, alwaysConfirm: false };
}

function tagged(stripped: string, whole: string): { text: string; alwaysConfirm: boolean } {
  const text = stripped.trim();
  return { text: text || whole.trim(), alwaysConfirm: true };
}

/** Back to the stored representation: one rule per line. */
export function serializeSafetyRules(rules?: string | string[] | SafetyRules | null): string | undefined {
  if (rules == null) return undefined;
  if (typeof rules === 'string') return rules.trim() || undefined;

  const list = Array.isArray(rules)
    ? rules
    : [...rules.alwaysConfirm.map((r) => `${ALWAYS_CONFIRM}: ${r}`), ...rules.other];

  const lines = list.map((r) => r.trim()).filter(Boolean);
  return lines.length ? lines.join('\n') : undefined;
}

// --- rendering --------------------------------------------------------------

export interface RenderOptions {
  /** Why this run is happening (e.g. which routine fired). Shown to the bot. */
  context?: string;
}

/**
 * Turn a skill into the instruction text for a run.
 *
 * Safety rules are rendered last and clearly demarcated so they read as the final
 * word rather than as more of the procedure, and `always-confirm` rules get their
 * own block with an explicit stop instruction. The live-view pause layer enforces
 * these for real; this is what makes the model aware it should be escalating in
 * the first place.
 */
export function renderSkillPrompt(skill: Skill, args?: SkillArgs, opts: RenderOptions = {}): string {
  const rules = parseSafetyRules(skill);
  const filled = applyArgs(skill.instructions, args);
  const sections: string[] = [`# Skill: ${skill.name}`, filled.text.trim()];

  if (opts.context) sections.push(`## Why you are running this\n${opts.context}`);

  if (filled.input !== undefined) sections.push(`## Input for this run\n${filled.input}`);
  if (filled.extras.length) {
    sections.push(`## Inputs\n${filled.extras.map(([k, v]) => `- ${k}: ${v}`).join('\n')}`);
  }

  if (rules.alwaysConfirm.length) {
    sections.push(
      `## Stop and confirm — ${ALWAYS_CONFIRM}\n` +
        'Before doing any of the following, stop. Say exactly what you are about to\n' +
        'do and wait for a human to approve it. Do not do it on your own authority and\n' +
        'do not look for a way around it. If you have no way to ask, end the run with\n' +
        'task_done explaining what needs approval and why you stopped there.\n' +
        bullets(rules.alwaysConfirm),
    );
  }

  if (rules.other.length) {
    sections.push(`## Safety rules for this skill\nThese override the procedure above.\n${bullets(rules.other)}`);
  }

  return sections.join('\n\n');
}

function bullets(items: string[]): string {
  return items.map((i) => `- ${i}`).join('\n');
}

/**
 * Substitute `{{name}}` placeholders. An unknown placeholder is left visible
 * rather than blanked, so a missing argument shows up in the log instead of
 * quietly changing what the skill says.
 */
function applyArgs(
  instructions: string,
  args?: SkillArgs,
): { text: string; extras: [string, string][]; input?: string } {
  if (args === undefined) return { text: instructions, extras: [] };

  const record: Record<string, unknown> = typeof args === 'string' ? { input: args } : args;
  const used = new Set<string>();

  const text = instructions.replace(/\{\{\s*([\w.-]+)\s*\}\}/g, (whole, key: string) => {
    if (!(key in record)) return whole;
    used.add(key);
    return stringifyArg(record[key]);
  });

  if (typeof args === 'string') {
    return used.has('input') ? { text, extras: [] } : { text, extras: [], input: args };
  }

  const extras = Object.entries(record)
    .filter(([k]) => !used.has(k))
    .map(([k, v]): [string, string] => [k, stringifyArg(v)]);

  return { text, extras };
}

function stringifyArg(value: unknown): string {
  return typeof value === 'string' ? value : JSON.stringify(value ?? null);
}

// --- running ----------------------------------------------------------------

export interface RunSkillOptions {
  skill: Skill;
  bot: Bot;
  provider: ModelProvider;
  args?: SkillArgs;
  /** Defaults to the bot's own model. */
  model?: string;
  maxIterations?: number;
  context?: string;
  onEvent?: (kind: string, text: string) => void;
}

export interface SkillRunResult extends RunResult {
  taskId: string;
  /** Exactly what was sent — a skill run is only auditable if this is kept. */
  instruction: string;
}

export async function runSkill(opts: RunSkillOptions): Promise<SkillRunResult> {
  const instruction = renderSkillPrompt(opts.skill, opts.args, { context: opts.context });
  const task = createTask(opts.bot.id, instruction);

  const result = await runTask({
    provider: opts.provider,
    model: opts.model ?? opts.bot.model,
    instruction,
    taskId: task.id,
    maxIterations: opts.maxIterations,
    onEvent: opts.onEvent,
  });

  return { ...result, taskId: task.id, instruction };
}
