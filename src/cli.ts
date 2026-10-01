#!/usr/bin/env node
import { createInterface } from 'node:readline/promises';
import { readFile, rm } from 'node:fs/promises';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { stdin, stdout } from 'node:process';
import {
  MissingKeyError,
  PROVIDER_NAMES,
  envVarFor,
  getProvider,
  isProviderName,
  needsKey,
  type Message,
  type ProviderName,
} from './providers/index.ts';
import {
  OAUTH_DEFAULT_URL,
  configPath,
  probeOAuth,
  providerConfig,
  setProviderConfig,
} from './secrets/config.ts';
import { readVault, setKey, removeKey, vaultExists, vaultPath, writeVault } from './secrets/vault.ts';
import {
  appendTaskLog,
  createBot,
  createTask,
  dbPath,
  getBotByName,
  listBots,
  listTasks,
  readTaskLog,
  setTaskStatus,
} from './db/index.ts';
import * as computer from './computer/container.ts';
import * as browser from './computer/browser.ts';
import { runTask } from './agent/loop.ts';
import {
  addSkill,
  editSkill,
  listSkills,
  parseSafetyRules,
  removeSkill,
  renderSkillPrompt,
  resolveSkill,
  runSkill,
} from './skills/index.ts';
import { dryRun, fireEvent, nextDueAt, routineSkill, startScheduler } from './routines/index.ts';
import {
  createRoutine,
  deleteRoutine,
  listRoutineRuns,
  listRoutines,
  setRoutineEnabled,
  type Routine,
} from './db/index.ts';

const USAGE = `mybot

  keys set <provider>            Store an API key (encrypted, local)
  keys list                      Show which providers have a key
  keys rm <provider>             Remove a stored key

  models <provider>              List models live from the provider API

  auth oauth [url]               Use your ChatGPT account via openai-oauth (no API key)
  auth apikey                    Switch OpenAI back to an API key
  auth status                    Show auth mode per provider

  bots add <name> <provider> [model]
  bots list

  computer up                    Start the account's computer (builds image on first run)
  computer status                Docker / image / container / browser state
  computer down [--rm]           Stop it (--rm also deletes the container)
  computer reset-profile         Drop ALL logged-in browser sessions
  computer sh "<command>"        Run a shell command inside it

  chat <bot-name> "<message>"    One turn, no tools
  run <bot-name> "<task>"        Agentic run on the computer (browser + shell)

  skills list | show <name> | add <name> | edit <name> | rm <name>
  skills run <name> [bot] [args] Run a skill now

  routines list                  Schedules and event triggers
  routines add <skill> --cron "<expr>" | --on <event> [--bot <name>]
  routines dryrun <skill>        What would run, and when it next fires
  routines on|off|rm <skill>
  routines runs <skill>          Firing history
  routines fire <event>          Trigger an event routine by hand
  routines serve                 Run the scheduler in the foreground

  tasks [--verbose]              Recent task history

providers: ${PROVIDER_NAMES.join(', ')}
`;

async function prompt(question: string, hidden = false): Promise<string> {
  const rl = createInterface({ input: stdin, output: stdout, terminal: true });
  if (!hidden) {
    const answer = await rl.question(question);
    rl.close();
    return answer.trim();
  }

  // Suppress echo so pasted keys and passphrases never hit the scrollback.
  stdout.write(question);
  const original = (rl as any).output.write.bind((rl as any).output);
  (rl as any).output.write = (chunk: string) => {
    if (chunk.includes('\n')) original(chunk);
  };
  const answer = await rl.question('');
  (rl as any).output.write = original;
  stdout.write('\n');
  rl.close();
  return answer.trim();
}

async function getPassphrase(forWrite: boolean): Promise<string> {
  const fromEnv = process.env.MYBOT_PASSPHRASE;
  if (fromEnv) return fromEnv;
  if (forWrite && !vaultExists()) {
    const p = await prompt('Create a vault passphrase: ', true);
    if (!p) throw new Error('Passphrase cannot be empty.');
    const again = await prompt('Confirm passphrase: ', true);
    if (p !== again) throw new Error('Passphrases did not match.');
    writeVault(p, {});
    console.log(`Vault created at ${vaultPath()} (mode 0600)`);
    return p;
  }
  return prompt('Vault passphrase: ', true);
}

/**
 * Only unlock the vault when the provider actually needs a key. An oauth-mode
 * provider has no secret to fetch, so prompting would be pure friction.
 */
async function passphraseFor(provider: ProviderName): Promise<string | undefined> {
  if (!needsKey(provider)) return undefined;
  if (process.env[envVarFor(provider)]) return undefined;
  if (!vaultExists()) return undefined;
  return getPassphrase(false);
}

function requireProvider(v: string | undefined): ProviderName {
  if (!v || !isProviderName(v)) {
    throw new Error(`Unknown provider "${v ?? ''}". Expected one of: ${PROVIDER_NAMES.join(', ')}`);
  }
  return v;
}

async function cmdKeys(args: string[]): Promise<void> {
  const sub = args[0];

  if (sub === 'set') {
    const provider = requireProvider(args[1]);
    const pass = await getPassphrase(true);
    const key = await prompt(`Paste ${provider} API key (input hidden): `, true);
    if (!key) throw new Error('No key entered.');
    setKey(pass, provider, key);
    console.log(`Stored ${provider} key — encrypted at ${vaultPath()}`);
    return;
  }

  if (sub === 'rm') {
    const provider = requireProvider(args[1]);
    removeKey(await getPassphrase(false), provider);
    console.log(`Removed ${provider} key.`);
    return;
  }

  if (sub === 'list' || sub === undefined) {
    const stored = vaultExists() ? readVault(await getPassphrase(false)) : {};
    for (const p of PROVIDER_NAMES) {
      if (!needsKey(p)) {
        console.log(`  ${p.padEnd(10)} oauth (no key needed)`);
        continue;
      }
      const env = process.env[envVarFor(p)] ? 'env' : null;
      const vault = stored[p] ? 'vault' : null;
      const where = [env, vault].filter(Boolean).join(' + ') || '—';
      console.log(`  ${p.padEnd(10)} ${where}`);
    }
    return;
  }

  throw new Error(`Unknown "keys" subcommand: ${sub}`);
}

async function cmdModels(args: string[]): Promise<void> {
  const provider = requireProvider(args[0]);
  const models = await getProvider(provider, await passphraseFor(provider)).listModels();
  for (const m of models) console.log(`  ${m}`);
  console.log(`\n${models.length} models (live from ${provider}).`);
}

async function cmdBots(args: string[]): Promise<void> {
  const sub = args[0];

  if (sub === 'add') {
    const name = args[1];
    if (!name) throw new Error('Usage: mybot bots add <name> <provider> [model]');
    const provider = requireProvider(args[2]);
    const model = args[3] ?? getProviderDefaults()[provider];
    const bot = createBot({ name, provider, model });
    console.log(`Created bot "${bot.name}" (${bot.provider}/${bot.model})`);
    console.log(`  id: ${bot.id}`);
    return;
  }

  if (sub === 'list' || sub === undefined) {
    const bots = listBots();
    if (!bots.length) {
      console.log('No bots yet.  mybot bots add <name> <provider>');
      return;
    }
    for (const b of bots) console.log(`  ${b.name.padEnd(16)} ${b.provider}/${b.model}`);
    return;
  }

  throw new Error(`Unknown "bots" subcommand: ${sub}`);
}

function getProviderDefaults(): Record<ProviderName, string> {
  return { anthropic: 'claude-opus-5', openai: 'gpt-5.6', gemini: 'gemini-3.6-flash' };
}

async function cmdChat(args: string[]): Promise<void> {
  let provider: ProviderName;
  let model: string | undefined;
  let system: string | undefined;
  let botId: string | undefined;
  let text: string;

  if (args[0] === '--provider') {
    provider = requireProvider(args[1]);
    text = args.slice(2).join(' ');
  } else {
    const bot = getBotByName(args[0] ?? '');
    if (!bot) throw new Error(`No bot named "${args[0] ?? ''}". See: mybot bots list`);
    provider = bot.provider;
    model = bot.model;
    system = bot.system ?? undefined;
    botId = bot.id;
    text = args.slice(1).join(' ');
  }

  if (!text) throw new Error('Nothing to send. Usage: mybot chat <bot-name> "<message>"');

  const p = getProvider(provider, await passphraseFor(provider));

  const task = botId ? createTask(botId, text) : undefined;
  if (task) {
    setTaskStatus(task.id, 'running');
    appendTaskLog(task.id, 'user', text);
  }

  const messages: Message[] = [{ role: 'user', content: text }];
  let thinkingOpen = false;
  let answer = '';

  for await (const chunk of p.chat(messages, { model, system })) {
    switch (chunk.type) {
      case 'thinking_delta':
        if (!thinkingOpen) {
          stdout.write('\x1b[2m[thinking] ');
          thinkingOpen = true;
        }
        stdout.write(chunk.text);
        break;

      case 'text_delta':
        if (thinkingOpen) {
          stdout.write('\x1b[0m\n\n');
          thinkingOpen = false;
        }
        stdout.write(chunk.text);
        answer += chunk.text;
        break;

      case 'tool_call':
        stdout.write(`\n[tool_call] ${chunk.name}(${JSON.stringify(chunk.input)})\n`);
        break;

      case 'done': {
        if (thinkingOpen) stdout.write('\x1b[0m');
        const { inputTokens: i, outputTokens: o } = chunk.usage;
        stdout.write(`\n\n\x1b[2m— ${chunk.model} · ${chunk.stopReason} · ${i} in / ${o} out\x1b[0m\n`);
        if (chunk.stopReason === 'refusal') {
          stdout.write('\x1b[33mThe model declined this request.\x1b[0m\n');
        }
        if (task) {
          appendTaskLog(task.id, 'assistant', answer);
          setTaskStatus(task.id, 'done');
        }
        return;
      }

      case 'error':
        if (task) {
          appendTaskLog(task.id, 'error', chunk.error);
          setTaskStatus(task.id, 'failed');
        }
        throw new Error(chunk.error);
    }
  }
}

async function cmdAuth(args: string[]): Promise<void> {
  const sub = args[0] ?? 'status';

  if (sub === 'oauth') {
    const baseURL = args[1] ?? OAUTH_DEFAULT_URL;
    const probe = await probeOAuth(baseURL);

    if (!probe.up) {
      throw new Error(
        `No openai-oauth proxy at ${baseURL} (${probe.reason}).\n` +
          `Start it first:  npx openai-oauth@latest --detach`,
      );
    }

    setProviderConfig('openai', { mode: 'oauth', baseURL });
    stdout.write(`OpenAI provider now uses your ChatGPT account via ${baseURL}\n\n`);
    stdout.write(`Models available to this account:\n`);
    for (const m of probe.models) stdout.write(`  ${m}\n`);

    const chat = probe.models.filter((m) => !m.includes('image'));
    if (chat.length) {
      stdout.write(`\nMake a bot:  mybot bots add scout openai ${chat[0]}\n`);
    }
    stdout.write(`\nNote: this is subscription auth, not an API key — it draws on your\n`);
    stdout.write(`ChatGPT plan's limits. Switch back any time with:  mybot auth apikey\n`);
    return;
  }

  if (sub === 'apikey') {
    setProviderConfig('openai', { mode: 'api_key' });
    stdout.write('OpenAI provider now uses an API key. Set one with:  mybot keys set openai\n');
    return;
  }

  if (sub === 'status') {
    for (const p of PROVIDER_NAMES) {
      const pc = providerConfig(p);
      if (pc.mode === 'oauth') {
        const probe = await probeOAuth(pc.baseURL ?? OAUTH_DEFAULT_URL);
        stdout.write(
          `  ${p.padEnd(10)} oauth   ${pc.baseURL ?? OAUTH_DEFAULT_URL}  ` +
            `${probe.up ? `\x1b[32mup\x1b[0m (${probe.models.length} models)` : `\x1b[31mdown\x1b[0m — ${probe.reason}`}\n`,
        );
      } else {
        stdout.write(`  ${p.padEnd(10)} api_key\n`);
      }
    }
    stdout.write(`\nconfig: ${configPath()}\n`);
    return;
  }

  throw new Error(`Unknown "auth" subcommand: ${sub}`);
}

async function cmdComputer(args: string[]): Promise<void> {
  const sub = args[0] ?? 'status';

  if (sub === 'up') {
    stdout.write('Starting the computer…\n');
    await computer.up((line) => stdout.write(`  ${line}\n`));
    const s = await computer.status();
    stdout.write(`\nReady.\n  browser  ${s.browser ?? 'starting'}\n  cdp      ${s.cdpUrl}\n  view     ${s.viewUrl}  (step 3)\n`);
    return;
  }

  if (sub === 'down') {
    await computer.down(computer.DEFAULT_CONVERSATION, args.includes('--rm'));
    stdout.write(args.includes('--rm') ? 'Stopped and removed.\n' : 'Stopped. Sessions and /workspace are preserved.\n');
    return;
  }

  if (sub === 'reset-profile') {
    await computer.resetProfile();
    stdout.write('Browser profile wiped — every logged-in session is gone. /workspace kept.\n');
    return;
  }

  if (sub === 'sh') {
    const cmd = args.slice(1).join(' ');
    if (!cmd) throw new Error('Usage: mybot computer sh "<command>"');
    const r = await computer.exec(cmd);
    if (r.stdout) stdout.write(r.stdout);
    if (r.stderr) stdout.write(r.stderr);
    process.exitCode = r.code;
    return;
  }

  if (sub === 'status') {
    const s = await computer.status();
    stdout.write(
      `  docker     ${s.docker ? 'up' : 'DOWN — start Docker Desktop'}\n` +
        `  image      ${s.image ? computer.IMAGE : 'not built (mybot computer up)'}\n` +
        `  container  ${s.container}\n` +
        `  browser    ${s.browser ?? '—'}\n` +
        `  cdp        ${s.cdpUrl}\n` +
        `  view       ${s.viewUrl}  (wired up in step 3)\n`,
    );
    return;
  }

  throw new Error(`Unknown "computer" subcommand: ${sub}`);
}

async function cmdRun(args: string[]): Promise<void> {
  const bot = getBotByName(args[0] ?? '');
  if (!bot) throw new Error(`No bot named "${args[0] ?? ''}". See: mybot bots list`);
  const instruction = args.slice(1).join(' ');
  if (!instruction) throw new Error('Usage: mybot run <bot-name> "<task>"');

  const provider = getProvider(bot.provider, await passphraseFor(bot.provider));

  stdout.write('Starting the computer…\n');
  await computer.up((line) => stdout.write(`  ${line}\n`));

  const task = createTask(bot.id, instruction);
  stdout.write(`\n\x1b[1m${bot.name}\x1b[0m — ${instruction}\n\n`);

  const result = await runTask({
    provider,
    model: bot.model,
    instruction,
    taskId: task.id,
    onEvent: (kind, text) => {
      const flat = text.replace(/\s+/g, ' ').trim();
      const short = flat.length > 150 ? `${flat.slice(0, 150)}…` : flat;
      if (kind === 'assistant') stdout.write(`\x1b[2m${text.trim()}\x1b[0m\n\n`);
      else if (kind === 'tool_call') stdout.write(`  \x1b[36m→\x1b[0m ${short}\n`);
      else if (kind === 'tool_result') stdout.write(`  \x1b[2m  ${short}\x1b[0m\n`);
      else if (kind === 'tool_error') stdout.write(`  \x1b[33m  ${short}\x1b[0m\n`);
      else if (kind === 'error' || kind === 'refusal') stdout.write(`  \x1b[31m${short}\x1b[0m\n`);
    },
  });

  await browser.disconnect();

  const { inputTokens: i, outputTokens: o } = result.usage;
  stdout.write(`\n\x1b[1m${result.status}\x1b[0m after ${result.iterations} iteration(s) · ${i} in / ${o} out\n\n`);
  stdout.write(`${result.summary}\n`);
}

// --- skills ----------------------------------------------------------------

async function cmdSkills(args: string[]): Promise<void> {
  const sub = args[0] ?? 'list';

  if (sub === 'list') {
    const skills = listSkills();
    if (!skills.length) {
      stdout.write('No skills yet.  mybot skills add "<name>"\n');
      return;
    }
    for (const s of skills) {
      const rules = parseSafetyRules(s);
      const confirm = rules.alwaysConfirm.length ? `  \x1b[33m${rules.alwaysConfirm.length} always-confirm\x1b[0m` : '';
      stdout.write(`  ${s.name.padEnd(24)} \x1b[2m${s.source}\x1b[0m${confirm}\n`);
    }
    return;
  }

  if (sub === 'add') {
    const name = args[1];
    if (!name) throw new Error('Usage: mybot skills add "<name>"');
    stdout.write('Instructions (natural language). End with a line containing only "." :\n');
    const instructions = await readBlock();
    if (!instructions.trim()) throw new Error('A skill needs instructions.');
    stdout.write('Safety rules, one per line. Prefix with "always-confirm:" to force a human check.\nEnd with "." :\n');
    const rules = (await readBlock()).split('\n').map((l) => l.trim()).filter(Boolean);
    const skill = addSkill({ name, instructions, safetyRules: rules });
    stdout.write(`\nSaved skill "${skill.name}" (${skill.source}).\n`);
    return;
  }

  if (sub === 'show') {
    const skill = resolveSkill(args[1] ?? '');
    stdout.write(`\x1b[1m${skill.name}\x1b[0m  (${skill.source})\n\n${renderSkillPrompt(skill)}\n`);
    return;
  }

  if (sub === 'edit') {
    const skill = resolveSkill(args[1] ?? '');
    stdout.write(`Current instructions:\n${skill.instructions}\n\nNew instructions. End with "." :\n`);
    const instructions = await readBlock();
    if (!instructions.trim()) throw new Error('Instructions cannot be empty.');
    editSkill(skill.id, { instructions });
    stdout.write(`Updated "${skill.name}".\n`);
    return;
  }

  if (sub === 'rm') {
    const skill = removeSkill(args[1] ?? '');
    stdout.write(`Removed "${skill.name}".\n`);
    return;
  }

  if (sub === 'run') {
    const skill = resolveSkill(args[1] ?? '');
    const botName = args[2];
    const bot = botName ? getBotByName(botName) : listBots()[0];
    if (!bot) throw new Error('No bot to run this with.  mybot bots add <name> <provider>');

    const provider = getProvider(bot.provider, await passphraseFor(bot.provider));
    await computer.up((line) => stdout.write(`  ${line}\n`));

    const result = await runSkill({
      skill,
      bot,
      provider,
      args: args.slice(3).join(' ') || undefined,
      onEvent: streamEvent,
    });
    stdout.write(`\n\x1b[1m${result.status}\x1b[0m — ${result.summary}\n`);
    await browser.disconnect();
    return;
  }

  throw new Error(`Unknown "skills" subcommand: ${sub}`);
}

/** Read a multi-line block from stdin, terminated by a lone "." */
async function readBlock(): Promise<string> {
  const rl = createInterface({ input: stdin, output: stdout, terminal: true });
  const lines: string[] = [];
  for await (const line of rl) {
    if (line.trim() === '.') break;
    lines.push(line);
  }
  rl.close();
  return lines.join('\n');
}

// --- routines --------------------------------------------------------------

async function cmdRoutines(args: string[]): Promise<void> {
  const sub = args[0] ?? 'list';

  if (sub === 'list') {
    const routines = listRoutines();
    if (!routines.length) {
      stdout.write('No routines yet.  mybot routines add <skill> --cron "<expr>" | --on <event>\n');
      return;
    }
    for (const r of routines) {
      const skill = routineSkill(r);
      const when = r.schedule ? `cron ${r.schedule}` : `on ${r.trigger}`;
      const next = r.schedule ? `  next ${nextDueAt(r).toLocaleString()}` : '';
      const state = r.enabled ? '' : '  \x1b[2m(disabled)\x1b[0m';
      stdout.write(`  ${skill.name.padEnd(24)} ${when.padEnd(24)}${next}${state}\n`);
    }
    return;
  }

  if (sub === 'add') {
    const skill = resolveSkill(args[1] ?? '');
    const cronIdx = args.indexOf('--cron');
    const onIdx = args.indexOf('--on');
    const botIdx = args.indexOf('--bot');
    const bot = botIdx > -1 ? getBotByName(args[botIdx + 1] ?? '') : undefined;

    const routine = createRoutine({
      skillId: skill.id,
      botId: bot?.id,
      schedule: cronIdx > -1 ? args[cronIdx + 1] : undefined,
      trigger: onIdx > -1 ? args[onIdx + 1] : undefined,
    });

    const report = dryRun(routine);
    stdout.write(`Created routine for "${skill.name}".\n`);
    if (report.nextFireAt) stdout.write(`  next fire: ${report.nextFireAt.toLocaleString()}\n`);
    for (const p of report.problems) stdout.write(`  \x1b[33m${p}\x1b[0m\n`);
    stdout.write(`\nTest it before trusting the schedule:  mybot routines dryrun ${skill.name}\n`);
    return;
  }

  if (sub === 'dryrun') {
    const skill = resolveSkill(args[1] ?? '');
    const routine = listRoutines().find((r: Routine) => r.skill_id === skill.id);
    if (!routine) throw new Error(`No routine bound to "${skill.name}".`);
    const report = dryRun(routine);
    stdout.write(
      `  skill    ${report.skill?.name ?? '—'}\n` +
        `  bot      ${report.bot?.name ?? '—'}\n` +
        `  kind     ${report.kind}\n` +
        `  enabled  ${report.enabled}\n` +
        `  next     ${report.nextFireAt?.toLocaleString() ?? '—'}\n`,
    );
    if (report.alwaysConfirm.length) {
      stdout.write(`  \x1b[33malways-confirm:\x1b[0m\n`);
      for (const r of report.alwaysConfirm) stdout.write(`    ${r}\n`);
    }
    for (const p of report.problems) stdout.write(`  \x1b[31m${p}\x1b[0m\n`);
    stdout.write(`\nWould run:\n${report.instruction ?? '(nothing — fix the problems above)'}\n`);
    return;
  }

  if (sub === 'on' || sub === 'off') {
    const skill = resolveSkill(args[1] ?? '');
    const routine = listRoutines().find((r: Routine) => r.skill_id === skill.id);
    if (!routine) throw new Error(`No routine bound to "${skill.name}".`);
    setRoutineEnabled(routine.id, sub === 'on');
    stdout.write(`Routine for "${skill.name}" ${sub === 'on' ? 'enabled' : 'disabled'}.\n`);
    return;
  }

  if (sub === 'rm') {
    const skill = resolveSkill(args[1] ?? '');
    const routine = listRoutines().find((r: Routine) => r.skill_id === skill.id);
    if (!routine) throw new Error(`No routine bound to "${skill.name}".`);
    deleteRoutine(routine.id);
    stdout.write(`Removed routine for "${skill.name}".\n`);
    return;
  }

  if (sub === 'runs') {
    const skill = resolveSkill(args[1] ?? '');
    const routine = listRoutines().find((r: Routine) => r.skill_id === skill.id);
    if (!routine) throw new Error(`No routine bound to "${skill.name}".`);
    const runs = listRoutineRuns(routine.id);
    if (!runs.length) {
      stdout.write('Never fired yet.\n');
      return;
    }
    for (const r of runs) {
      stdout.write(`  ${r.fired_at}  ${r.error ? `\x1b[31m${r.error}\x1b[0m` : 'ok'}\n`);
    }
    return;
  }

  if (sub === 'fire') {
    const name = args[1];
    if (!name) throw new Error('Usage: mybot routines fire <event-name>');
    await computer.up((line) => stdout.write(`  ${line}\n`));
    const outcomes = await fireEvent(name, undefined, { onEvent: streamEvent });
    if (!outcomes.length) stdout.write(`No enabled routine listens for "${name}".\n`);
    for (const o of outcomes) stdout.write(`  ${o.ok ? 'ok' : 'FAILED'}  ${o.skillName} — ${o.summary}\n`);
    await browser.disconnect();
    return;
  }

  if (sub === 'serve') {
    await computer.up((line) => stdout.write(`  ${line}\n`));
    const handle = startScheduler({
      onEvent: streamEvent,
      onFire: (o) => stdout.write(`\n\x1b[1m${o.skillName}\x1b[0m — ${o.ok ? 'ok' : 'FAILED'}: ${o.summary}\n`),
      onError: (err) => stdout.write(`\x1b[31mscheduler: ${err instanceof Error ? err.message : String(err)}\x1b[0m\n`),
    });
    stdout.write('Scheduler running. Ctrl-C to stop.\n');
    for (const t of listRoutines().filter((r: Routine) => r.enabled && r.schedule)) {
      stdout.write(`  ${routineSkill(t).name} → ${nextDueAt(t).toLocaleString()}\n`);
    }
    await new Promise<void>((resolve) => {
      const stop = () => {
        handle.stop();
        stdout.write('\nScheduler stopped.\n');
        resolve();
      };
      process.on('SIGINT', stop);
      process.on('SIGTERM', stop);
    });
    return;
  }

  throw new Error(`Unknown "routines" subcommand: ${sub}`);
}

/** Shared console renderer for agent-loop events. */
function streamEvent(kind: string, text: string): void {
  const flat = text.replace(/\s+/g, ' ').trim();
  const short = flat.length > 150 ? `${flat.slice(0, 150)}…` : flat;
  if (kind === 'assistant') stdout.write(`\x1b[2m${text.trim()}\x1b[0m\n\n`);
  else if (kind === 'tool_call') stdout.write(`  \x1b[36m→\x1b[0m ${short}\n`);
  else if (kind === 'tool_result') stdout.write(`  \x1b[2m  ${short}\x1b[0m\n`);
  else if (kind === 'tool_error') stdout.write(`  \x1b[33m  ${short}\x1b[0m\n`);
  else if (kind === 'error' || kind === 'refusal') stdout.write(`  \x1b[31m${short}\x1b[0m\n`);
  else if (kind === 'paused') stdout.write(`\n  \x1b[33m⏸ ${short}\x1b[0m\n`);
}

async function cmdTasks(): Promise<void> {
  const tasks = listTasks();
  if (!tasks.length) {
    console.log('No tasks yet.');
    return;
  }
  for (const t of tasks.slice(0, 20)) {
    const preview = t.instruction.replace(/\s+/g, ' ').slice(0, 56);
    console.log(`  ${t.status.padEnd(10)} ${preview}${t.instruction.length > 56 ? '…' : ''}`);
    if (process.argv.includes('--verbose')) {
      for (const e of readTaskLog(t.id)) {
        console.log(`      ${e.kind}: ${e.text.replace(/\s+/g, ' ').slice(0, 100)}`);
      }
    }
  }
}

/**
 * Stop the background console.
 *
 * The desktop app leaves the console running when its window closes, because a
 * paired phone and any scheduled routine depend on it. That makes an explicit
 * way to stop it a requirement, not a nicety.
 */
async function cmdStop(): Promise<void> {
  const pidFile = join(homedir(), '.mybot', 'console.pid');
  const portFile = join(homedir(), '.mybot', 'app.port');

  let pid: number | undefined;
  try {
    pid = Number.parseInt(await readFile(pidFile, 'utf8'), 10);
  } catch {
    /* no pid file: fall through to the "not running" message */
  }

  if (!pid || Number.isNaN(pid)) {
    console.log('No console is running.');
    return;
  }

  try {
    process.kill(pid, 'SIGTERM');
  } catch {
    console.log('No console is running (stale pid file).');
    await rm(pidFile, { force: true });
    return;
  }

  await rm(pidFile, { force: true });
  await rm(portFile, { force: true });
  console.log(`Stopped the console (pid ${pid}).`);
}

async function main(): Promise<void> {
  const [cmd, ...rest] = process.argv.slice(2);

  switch (cmd) {
    case 'keys':
      return cmdKeys(rest);
    case 'models':
      return cmdModels(rest);
    case 'bots':
      return cmdBots(rest);
    case 'chat':
      return cmdChat(rest);
    case 'auth':
      return cmdAuth(rest);
    case 'computer':
      return cmdComputer(rest);
    case 'run':
      return cmdRun(rest);
    case 'skills':
      return cmdSkills(rest);
    case 'routines':
      return cmdRoutines(rest);
    case 'tasks':
      return cmdTasks();
    case 'stop':
      return cmdStop();
    case 'where':
      console.log(`vault: ${vaultPath()}\ndb:    ${dbPath()}`);
      return;
    default:
      stdout.write(USAGE);
      if (cmd && cmd !== '--help' && cmd !== '-h') process.exitCode = 1;
  }
}

main().catch((err) => {
  if (err instanceof MissingKeyError) console.error(`\n${err.message}\n`);
  else console.error(`\nError: ${err instanceof Error ? err.message : String(err)}\n`);
  process.exitCode = 1;
});
