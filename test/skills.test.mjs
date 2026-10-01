/**
 * Skills + routines. No API keys, no network, no container — pure logic.
 *
 * Cron assertions are in LOCAL time on purpose. An earlier version of this file
 * compared against toISOString() and "failed" every case by exactly the machine's
 * UTC offset; the parser fires on local wall-clock, which is what a human writing
 * "0 9 * * 1" means.
 */
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import assert from 'node:assert';

const HOME = mkdtempSync(join(tmpdir(), 'mybot-skills-'));
process.env.MYBOT_HOME = HOME;

const { parseCron, nextFireTime, dryRun } = await import('../src/routines/index.ts');
const { addSkill, parseSafetyRules, renderSkillPrompt, resolveSkill } = await import('../src/skills/index.ts');
const db = await import('../src/db/index.ts');

const pad = (n) => String(n).padStart(2, '0');
const local = (d) =>
  `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;

// --- cron ------------------------------------------------------------------
const cron = [
  ['*/15 * * * *', '2026-08-13T10:07:00', '2026-08-13 10:15'],
  ['*/15 * * * *', '2026-08-13T23:52:00', '2026-08-14 00:00'],
  ['0 9 * * 1', '2026-08-13T12:00:00', '2026-08-17 09:00'],
  ['0 9 * * 1', '2026-08-17T09:00:00', '2026-08-24 09:00'], // on the slot -> next week
  ['30 3 1 * *', '2026-08-13T00:00:00', '2026-09-01 03:30'], // month rollover
  ['30 3 1 * *', '2026-12-15T00:00:00', '2027-01-01 03:30'], // year rollover
  ['0 0 * * 0', '2026-08-13T00:00:00', '2026-08-16 00:00'],
  ['0 0 29 2 *', '2026-03-01T00:00:00', '2028-02-29 00:00'], // skips non-leap years
  ['0 0 1 * 1', '2026-08-13T00:00:00', '2026-08-17 00:00'], // Vixie OR: Monday beats the 1st
];
for (const [expr, from, want] of cron) {
  assert.equal(local(nextFireTime(expr, new Date(from))), want, `${expr} from ${from}`);
}
console.log(`cron next-fire (${cron.length} cases incl. leap/rollover/OR-semantics): OK`);

for (const bad of ['* * * *', '70 * * * *', '* * * 13 *', '*/0 * * * *', '5-1 * * * *', '', 'a * * * *']) {
  assert.throws(() => parseCron(bad), `should reject ${JSON.stringify(bad)}`);
}
console.log('cron rejects malformed expressions: OK');

// --- skills ----------------------------------------------------------------
db.createBot({ name: 'scout', provider: 'anthropic', model: 'claude-opus-5' });

const skill = addSkill({
  name: 'weekly-competitors',
  instructions: 'Check competitor pricing and write /workspace/pricing-{{week}}.md',
  safetyRules: ['always-confirm: posting anything publicly', 'Never sign in to a competitor site'],
});

const rules = parseSafetyRules(skill);
assert.deepEqual(rules.alwaysConfirm, ['posting anything publicly']);
assert.deepEqual(rules.other, ['Never sign in to a competitor site']);
console.log('always-confirm rules parsed out of safety rules: OK');

const prompt = renderSkillPrompt(skill, { week: '33' });
assert.ok(prompt.includes('pricing-33.md'), 'placeholder substituted');
assert.ok(!prompt.includes('{{week}}'), 'no placeholder left behind');
assert.ok(/always-confirm/i.test(prompt), 'always-confirm surfaced to the model');
assert.ok(prompt.includes('Never sign in'), 'plain safety rules surfaced');
console.log('rendered prompt substitutes placeholders and carries safety rules: OK');

assert.throws(() => addSkill({ name: 'weekly-competitors', instructions: 'dupe' }), /exist|unique|taken/i);
assert.throws(() => addSkill({ name: '', instructions: 'x' }), /name/i);
assert.throws(() => addSkill({ name: 'blank', instructions: '   ' }), /instruction/i);
console.log('skill validation (duplicate name, empty name, empty body): OK');

// Local-only by design — the schema is the last line of defence against a
// ClawHub-style supply-chain problem, so assert it directly.
assert.throws(
  () => db.openDb().prepare("INSERT INTO skills (id,name,instructions,source) VALUES ('x','y','z','remote')").run(),
  /CHECK|constraint/i,
);
console.log('skills cannot be sourced remotely (schema CHECK): OK');

// --- routines --------------------------------------------------------------
const routine = db.createRoutine({ skillId: skill.id, schedule: '0 9 * * 1' });
const report = dryRun(routine);
assert.equal(report.kind, 'schedule');
assert.equal(report.enabled, true);
assert.equal(report.bot?.name, 'scout', 'falls back to the only bot when bot_id is null');
assert.deepEqual(report.problems, []);
assert.ok(report.instruction?.includes('competitor'), 'dry run shows what would run');
assert.deepEqual(report.alwaysConfirm, ['posting anything publicly']);
assert.equal(db.listTasks().length, 0, 'dry run must not create a task');
assert.equal(db.listRoutineRuns(routine.id).length, 0, 'dry run must not log a firing');
console.log('routine dry run reports without executing: OK');

assert.throws(
  () => db.createRoutine({ skillId: skill.id, schedule: '0 9 * * 1', trigger: 'webhook' }),
  /exactly one/i,
);
assert.throws(() => db.createRoutine({ skillId: skill.id }), /exactly one/i);
console.log('routine requires exactly one of schedule / trigger: OK');

rmSync(HOME, { recursive: true, force: true });
console.log('\nALL SKILLS + ROUTINES TESTS PASSED');
