/*
 * MyBot — messenger client.
 *
 * One idea drives the whole file: a task is a *message you sent a colleague*,
 * and everything the bot does in service of it belongs inside that thread. So
 * there is no "run panel" and no separate log viewer — tool calls, thinking and
 * results all land in the conversation, styled so you can skim the mechanics and
 * read the prose.
 *
 * All text from the model or from pages it visited goes in via textContent.
 * Task logs contain whatever was on the web, which is to say attacker-controlled
 * input; innerHTML here would be a cross-site scripting hole with extra steps.
 */
import { api, subscribe, tokenised } from './api.js';
import { el, clear, $, initials, clock, whenText } from './dom.js';

const state = {
  bots: [],
  selected: null,
  computer: null,
  paused: [],
  /** botId -> rendered message nodes, so switching threads keeps history. */
  threads: new Map(),
  providers: ['anthropic', 'openai', 'gemini'],
};

/** The step the live stream is still waiting on a result for. */
const live = { open: null };

const app = $('.app');
const rosterList = $('#roster-list');
const messages = $('#messages');
const composer = $('#composer');
const input = $('#input');
const sendBtn = $('#send');
const emptyState = $('#empty');
const threadHead = $('#thread-head');
const stopBtn = $('#stop');
const working = $('#working');

/**
 * Reflect whether this bot is mid-run.
 *
 * Without this a long research run looks identical to a hung one: no output,
 * no spinner, no way out. Send becomes Stop while it works.
 */
function setRunning(on, taskId) {
  working.hidden = !on;
  stopBtn.hidden = !on;
  sendBtn.hidden = !!on;
  stopBtn.onclick = taskId
    ? async () => {
        stopBtn.disabled = true;
        await api.stop(taskId).catch(() => {});
        stopBtn.disabled = false;
      }
    : null;
}

function runningTaskFor(botId) {
  const bot = state.bots.find((b) => b.id === botId);
  const t = bot?.currentTask;
  return t && t.status === 'running' ? t.id : null;
}

// ── roster ──────────────────────────────────────────────────────────────

/**
 * A stable hue per bot, derived from its name.
 *
 * The reference UI gives every teammate a distinct saturated tile, and in a
 * roster of eight that colour is what you navigate by before you read a word.
 * Hashing the name means a bot keeps its colour across restarts without storing
 * anything.
 */
function hueOf(name) {
  let h = 0;
  for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) % 360;
  return h;
}

/** What this bot is wired into, shown as a chip beside its name. */
function chipFor(bot) {
  return { anthropic: 'Claude', openai: 'GPT', gemini: 'Gemini' }[bot.provider] ?? bot.provider;
}

function stateOf(bot) {
  const t = bot.currentTask ?? bot.lastTask;
  return t?.status ?? 'idle';
}

function renderRoster() {
  clear(rosterList);

  if (!state.bots.length) {
    rosterList.append(el('li', { class: 'muted', style: 'padding:10px 12px' }, 'No teammates yet.'));
    return;
  }

  for (const bot of state.bots) {
    const st = stateOf(bot);
    const task = bot.currentTask ?? bot.lastTask;

    const avatar = el('div', { class: 'avatar', 'data-state': st }, initials(bot.name));
    avatar.style.setProperty('--hue', hueOf(bot.name));

    const line = el(
      'div',
      { class: 'bot-line' },
      el('span', { class: 'bot-name' }, bot.name),
      el('span', { class: 'chip' }, chipFor(bot)),
      el('span', { class: 'bot-when' }, task ? whenText(task.started_at) : ''),
    );

    const last = el('div', { class: 'bot-last', 'data-state': st });
    last.textContent = subtitleFor(bot, st, task);

    const row = el(
      'li',
      {},
      el(
        'button',
        {
          class: 'bot',
          role: 'option',
          'aria-selected': String(state.selected === bot.id),
          onclick: () => select(bot.id),
        },
        avatar,
        el('div', {}, line, last),
      ),
    );
    rosterList.append(row);
  }
}

function subtitleFor(bot, st, task) {
  if (st === 'running') return 'Working…';
  if (st === 'paused') return 'Needs you';
  if (!task) return `${bot.provider} · ${bot.model}`;
  return task.instruction;
}

// ── thread ──────────────────────────────────────────────────────────────

function select(botId) {
  state.selected = botId;
  app.dataset.view = 'thread';

  const bot = state.bots.find((b) => b.id === botId);
  if (!bot) return;

  threadHead.hidden = false;
  composer.hidden = false;
  emptyState.hidden = true;

  const headAvatar = $('#head-avatar');
  headAvatar.textContent = initials(bot.name);
  headAvatar.dataset.state = stateOf(bot);
  headAvatar.style.setProperty('--hue', hueOf(bot.name));
  // The reference addresses the bot by name here; it makes the thread feel
  // like a conversation with someone rather than a form.
  input.placeholder = `Message ${bot.name}…`;
  $('#head-name').textContent = bot.name;
  $('#head-sub').textContent = `${bot.provider} · ${bot.model}`;

  clear(messages);
  live.open = null;
  const thread = state.threads.get(botId);
  if (thread) for (const node of thread) messages.append(node);
  else void loadHistory(bot);

  renderRoster();
  renderTakeover();
  setRunning(Boolean(runningTaskFor(botId)), runningTaskFor(botId));
  input.focus();
  scrollDown();
}

/** Replay the bot's past tasks so a thread is not blank on first open. */
async function loadHistory(bot) {
  const tasks = await api.tasks(bot.id).catch(() => []);
  const nodes = [];

  for (const task of tasks.slice(0, 8).reverse()) {
    const detail = await api.task(task.id).catch(() => null);
    const log = detail?.log ?? [];

    // The agent loop already writes the instruction into the log as a `user`
    // entry, so adding one from task.instruction as well rendered every message
    // you sent twice. Only synthesise it when the log has no user entry —
    // a task that failed before the loop started still needs to show what was
    // asked, or the thread opens on an answer to an invisible question.
    if (!log.some((e) => e.kind === 'user')) nodes.push(bubble('you', task.instruction));

    const ctx = {};
    for (const entry of log) {
      const node = entryNode(entry, ctx);
      if (node) nodes.push(node);
    }
  }

  state.threads.set(bot.id, nodes);
  if (state.selected === bot.id) {
    clear(messages);
    for (const n of nodes) messages.append(n);
    scrollDown();
  }
}

function bubble(who, text) {
  const b = el('div', { class: 'bubble' });
  b.textContent = text;
  return el('div', { class: `msg from-${who}` }, b);
}

const ACTION_ICON = {
  tool_call: '›',
  tool_result: '·',
  tool_error: '!',
  error: '!',
  refusal: '!',
  pause: '⏸',
  resume: '▸',
  done: '✓',
};

/**
 * What a tool call reads as in a sentence.
 *
 * The log stores `page_navigate {"url":"https://example.com"}` — accurate, and
 * unreadable in bulk. A thread of twenty of those is a wall of JSON where the
 * one line you care about is buried. Each entry here turns a call into the
 * phrase a person would use for it; the raw arguments stay one click away.
 */
const STEP_PHRASE = {
  page_navigate: (a) => ['Opened', prettyUrl(a.url)],
  page_read: () => ['Read the page'],
  page_click: (a) => ['Clicked', a.text || a.ref],
  page_type: (a) => ['Typed into', a.text ? `“${a.text}”` : a.ref],
  page_scroll: (a) => ['Scrolled', a.direction],
  bash: (a) => ['Ran', a.command],
  list_desktops: () => ['Looked at the desktops'],
  merge_request: (a) => ['Asked to merge with', a.bot ?? a.target],
  request_host_access: (a) => ['Asked for access to', a.path || 'your Mac'],
  request_human: () => ['Asked for you'],
  emit_skill: (a) => ['Saved a skill', a.name],
};

function prettyUrl(url) {
  try {
    const u = new URL(url);
    return u.host.replace(/^www\./, '') + (u.pathname === '/' ? '' : u.pathname);
  } catch {
    return url;
  }
}

/** `page_navigate {"url":"…"}` / `page_navigate -> Opened …` -> their parts. */
function splitTool(text) {
  const m = /^([a-z_]+)\s*(->)?\s*([\s\S]*)$/.exec(text);
  if (!m) return null;
  return { name: m[1], isResult: Boolean(m[2]), rest: m[3] };
}

function stepNode(name, rest) {
  let args = {};
  try {
    args = JSON.parse(rest);
  } catch {
    /* not every call logs parseable arguments; the raw text still shows */
  }
  const [verb, target] = (STEP_PHRASE[name] ?? ((a) => [name.replace(/_/g, ' '), summarise(a)]))(args);

  const node = el('details', { class: 'step', 'data-state': 'running' });
  const tgt = el('span', { class: 'step-target' });
  tgt.textContent = target ?? '';
  node.append(
    el('summary', {}, el('span', { class: 'step-dot' }), el('span', { class: 'step-verb' }, verb), tgt),
  );
  const detail = el('div', { class: 'step-detail' });
  if (rest.trim()) {
    const pre = el('pre', { class: 'step-args' });
    pre.textContent = rest.trim();
    detail.append(pre);
  }
  node.append(detail);
  node._tool = name;
  node._settle = (outcome, ok) => {
    node.dataset.state = ok ? 'ok' : 'bad';
    // `page_click {"ref":10}` carries no label — "Clicked 10" means nothing to
    // a reader. The result is where the element's own text shows up.
    const label = /^clicked\s+\[\d+\]\s+(.+)$/.exec(outcome.trim());
    if (label && /^\d+$/.test(tgt.textContent.trim())) tgt.textContent = label[1];
    if (!outcome.trim()) return;
    const pre = el('pre', { class: 'step-out' });
    pre.textContent = outcome.trim();
    detail.append(pre);
  };
  return node;
}

function summarise(args) {
  const v = Object.values(args)[0];
  return typeof v === 'string' ? v : '';
}

/**
 * The log stores machine-shaped text — `always_confirm: …  — live view http://…`
 * — because it has to survive a restart and be greppable. None of that belongs
 * in a sentence you read in the thread.
 */
function humanise(kind, text) {
  if (kind === 'pause') return splitPause(text.replace(/\s+—\s+live view\s+\S+$/, '')).why;
  if (kind === 'resume') {
    if (/skipped/.test(text)) return 'You skipped this step.';
    return 'You handed control back.';
  }
  return text;
}

function activity(kind, text) {
  const line = el('div', { class: 'activity', 'data-kind': kind });
  line.append(el('span', { class: 'activity-icon' }, ACTION_ICON[kind] ?? '·'));
  const body = el('span', { class: 'activity-text' });
  body.textContent = text;
  line.append(body);
  return line;
}

/**
 * One log entry -> one node, or null for the kinds not worth showing.
 *
 * `ctx` carries the step still waiting for its result, so a tool's outcome
 * folds into the row that started it instead of becoming a second line.
 */
function entryNode(entry, ctx = {}) {
  const { kind, text } = entry;
  if (!text) return null;

  if (kind === 'user') return bubble('you', text);
  if (kind === 'assistant') return bubble('bot', text);

  if (kind === 'thinking') {
    const body = el('div', { class: 'think-body' });
    body.textContent = text;
    const words = text.trim().split(/\s+/).length;
    return el(
      'details',
      { class: 'think' },
      el('summary', {}, `Thought for ${words} word${words === 1 ? '' : 's'}`),
      body,
    );
  }

  if (kind === 'tool_call' || kind === 'tool_result' || kind === 'tool_error') {
    const parts = splitTool(text);
    // task_done is bookkeeping: the `done` entry right behind it carries the
    // same summary, and showing both said everything twice.
    if (parts?.name === 'task_done') return null;

    if (parts && kind === 'tool_call') {
      ctx.open = stepNode(parts.name, parts.rest);
      return ctx.open;
    }
    if (parts && ctx.open?._tool === parts.name) {
      ctx.open._settle(parts.rest, kind !== 'tool_error');
      ctx.open = null;
      return null; // folded into the row above
    }
  }

  ctx.open = null;
  return activity(kind, humanise(kind, text));
}

function push(node) {
  if (!node || !state.selected) return;
  const thread = state.threads.get(state.selected) ?? [];
  thread.push(node);
  state.threads.set(state.selected, thread);
  messages.append(node);
  scrollDown();
}

/** Only auto-scroll when the reader is already at the bottom. */
function scrollDown(force = false) {
  const nearBottom = messages.scrollHeight - messages.scrollTop - messages.clientHeight < 130;
  if (force || nearBottom) messages.scrollTop = messages.scrollHeight;
}

// ── sending ─────────────────────────────────────────────────────────────

composer.addEventListener('submit', async (ev) => {
  ev.preventDefault();
  const text = input.value.trim();
  if (!text || !state.selected) return;

  input.value = '';
  input.style.height = 'auto';
  sendBtn.disabled = true;
  push(bubble('you', text));

  try {
    const task = await api.run(state.selected, text);
    setRunning(true, task?.id);
  } catch (err) {
    push(entryNode({ kind: 'error', text: err.message }));
    setRunning(false);
  } finally {
    sendBtn.disabled = false;
    input.focus();
  }
});

input.addEventListener('input', () => {
  input.style.height = 'auto';
  input.style.height = `${Math.min(input.scrollHeight, 168)}px`;
});

input.addEventListener('keydown', (ev) => {
  if (ev.key === 'Enter' && !ev.shiftKey) {
    ev.preventDefault();
    composer.requestSubmit();
  }
});

// ── takeover ────────────────────────────────────────────────────────────

function pausedForSelected() {
  return state.paused.find((t) => t.bot_id === state.selected);
}

/**
 * A pause is stored as `kind: reason` so the kind survives a restart. The kind
 * is a machine label though — "always_confirm: Please sign in…" is not a
 * sentence anyone should be shown. Split it: the kind becomes the headline,
 * the reason stays the bot's own words.
 */
const PAUSE_TITLE = {
  captcha: 'A CAPTCHA needs a human',
  two_factor: 'Two-factor code needed',
  password: 'A password is needed',
  payment: 'Payment details needed',
  always_confirm: 'Waiting for you',
};

function splitPause(stored) {
  const m = /^([a-z_]+):\s*([\s\S]*)$/.exec(stored ?? '');
  if (!m || !PAUSE_TITLE[m[1]]) {
    return { title: 'Waiting for you', why: stored || 'It hit a step only a person should do.' };
  }
  return { title: PAUSE_TITLE[m[1]], why: m[2] };
}

function renderTakeover() {
  const bar = $('#takeover-bar');
  const mine = pausedForSelected();
  bar.hidden = !mine;
  if (mine) {
    const { title, why } = splitPause(mine.paused_reason);
    $('#takeover-title').textContent = title;
    $('#takeover-why').textContent = why;
    $('#takeover-btn').onclick = () => openComputer();
  }
  renderHandoff();
}

/**
 * The hand-back controls, inside the computer window.
 *
 * Deliberately not in the conversation: the instruction has to stay on screen
 * while you do the step, and "I'm done" has to be one click from the thing you
 * just finished. A banner back in the thread means reading the ask, leaving it,
 * then hunting for the way to hand control back.
 */
function renderHandoff() {
  const mine = pausedForSelected();
  const bar = $('#handoff');
  bar.hidden = !mine;
  if (!mine) return;

  $('#handoff-text').textContent = splitPause(mine.paused_reason).why;

  $('#handoff-done').onclick = async () => {
    bar.hidden = true;
    await api.resume(mine.id).catch(() => {});
  };

  // Skipping is a real answer, and a DIFFERENT one from "I did it". Both
  // buttons used to hit the same endpoint, so the bot resumed, re-read the same
  // login form and asked again — an infinite ask. The flag is what tells it the
  // step is never happening.
  $('#handoff-skip').onclick = async () => {
    bar.hidden = true;
    await api.resume(mine.id, true).catch(() => {});
  };
}

// ── agent computer ──────────────────────────────────────────────────────

async function openComputer() {
  const sheet = $('#computer-sheet');
  const frame = $('#screen');
  sheet.hidden = false;

  // Before anything else. If a bot is parked waiting for a person, the ask is
  // the reason this window is open — it must not wait behind a container boot
  // that can take minutes on first run.
  renderHandoff();

  // The container is started lazily: a session only exists once something has
  // needed one. Opening this window is exactly that moment, so start it here
  // rather than showing an empty black rectangle.
  //
  // /api/computer/up returns 202 and does the work in the background, so its
  // response body says nothing about readiness — poll for the desktop URL
  // instead of assuming the reply is the new state.
  if (!state.computer?.viewUrl) {
    $('#sheet-foot').textContent =
      'Starting the computer — the first run builds the image, which takes a few minutes…';
    try {
      await api.startComputer();
    } catch (err) {
      $('#sheet-foot').textContent = `Could not start the computer: ${err.message}`;
      return;
    }

    const deadline = Date.now() + 5 * 60 * 1000;
    while (Date.now() < deadline && !state.computer?.viewUrl) {
      if (sheet.hidden) return; // they gave up and closed it
      await new Promise((r) => setTimeout(r, 1200));
      state.computer = await api.computer().catch(() => state.computer);
    }
  }

  const url = state.computer?.viewUrl;
  if (url) {
    frame.src = url;
    $('#sheet-popout').href = url;
    $('#sheet-foot').textContent = state.computer.browser
      ? `${state.computer.browser} · click into the screen to drive it yourself`
      : 'Click into the screen to drive it yourself.';
  } else {
    $('#sheet-foot').textContent = 'The computer is up, but no desktop is attached yet.';
  }

  renderHandoff();
  renderComputer();
}

function closeComputer() {
  $('#computer-sheet').hidden = true;
  $('#screen').src = 'about:blank'; // stop the VNC socket when it is not visible
}

$('#open-computer').onclick = () => void openComputer();
$('#plus-btn').onclick = () => void openComputer();
$('#sheet-close').onclick = closeComputer;

function renderComputer() {
  const dot = $('#computer-dot');
  dot.dataset.up = String(state.computer?.container === 'running');
}

// ── sheets ──────────────────────────────────────────────────────────────

function openSheet(id) { $(id).hidden = false; }
function closeSheet(id) { $(id).hidden = true; }

for (const scrim of document.querySelectorAll('.sheet-scrim')) {
  scrim.addEventListener('click', (ev) => {
    if (ev.target === scrim) {
      if (scrim.id === 'computer-sheet') closeComputer();
      else scrim.hidden = true;
    }
  });
}

document.addEventListener('keydown', (ev) => {
  if (ev.key !== 'Escape') return;
  if (!$('#computer-sheet').hidden) return closeComputer();
  for (const id of ['#bot-sheet', '#list-sheet']) if (!$(id).hidden) $(id).hidden = true;
});

// ── new teammate ────────────────────────────────────────────────────────

function openBotSheet() {
  const sel = $('#bot-provider');
  clear(sel);
  for (const p of state.providers) sel.append(el('option', { value: p }, p));
  $('#bot-error').hidden = true;
  sel.onchange = () => void loadModels(sel.value);
  void loadModels(sel.value);
  openSheet('#bot-sheet');
  $('#bot-name').focus();
}

$('#new-bot-btn').onclick = openBotSheet;
$('#empty-new').onclick = openBotSheet;
$('#bot-cancel').onclick = () => closeSheet('#bot-sheet');
$('#bot-sheet-close').onclick = () => closeSheet('#bot-sheet');

$('#bot-form').addEventListener('submit', async (ev) => {
  ev.preventDefault();
  const body = {
    name: $('#bot-name').value.trim(),
    provider: $('#bot-provider').value,
    model: $('#bot-model-select').value || undefined,
    system: $('#bot-system').value.trim() || undefined,
  };

  try {
    const bot = await api.createBot(body);
    closeSheet('#bot-sheet');
    $('#bot-form').reset();
    await refreshBots();
    select(bot.id);
  } catch (err) {
    const box = $('#bot-error');
    box.textContent = err.message;
    box.hidden = false;
  }
});

// ── skills / routines ───────────────────────────────────────────────────

/** One card in the two-column panel, matching the reference's plugin grid. */
function card(mark, title, sub, action) {
  const t = el('div', { class: 'card-title' });
  t.textContent = title;
  const s2 = el('div', { class: 'card-sub' });
  s2.textContent = sub;

  const node = el(
    'div',
    { class: 'card' },
    el('div', { class: 'card-mark' }, mark),
    el('div', { class: 'card-body' }, t, s2),
  );
  if (action) node.append(action);
  return node;
}

/**
 * Connect a phone.
 *
 * Off until asked for. This console runs shell commands in a container and
 * drives a logged-in browser, so the open port is the whole of that handed to
 * anyone else on the wifi — which is why the link carries a token and why there
 * is a way to switch it back off in one click.
 */
$('#nav-phone').onclick = async () => {
  $('#list-title').textContent = 'Connect a phone';
  const body = $('#list-body');
  clear(body);
  body.append(el('p', { class: 'muted' }, 'Checking the network…'));

  const render = (st) => {
    clear(body);

    if (!st.paired) {
      body.append(
        el(
          'p',
          { class: 'muted' },
          'Your phone opens the same console over your wifi — the same threads, the same live desktops. Off until you turn it on.',
        ),
      );
      if (st.error) body.append(el('p', { class: 'form-error' }, st.error));
      const go = el('button', { class: 'solid-btn' }, 'Turn on');
      go.onclick = async () => {
        go.disabled = true;
        go.textContent = 'Opening…';
        render(await api.pair().catch((e) => ({ paired: false, error: e.message })));
      };
      body.append(el('div', { class: 'form-actions' }, go));
      return;
    }

    body.append(el('p', { class: 'muted' }, `Open this on your phone, on the same wifi as this Mac (${st.address}):`));
    const link = el('pre', { class: 'file-body' });
    link.textContent = st.url;
    body.append(link);
    body.append(
      el(
        'p',
        { class: 'muted' },
        'The link contains the pairing key — anyone who has it has your console, so send it only to your own phone.',
      ),
    );

    const copy = el('button', { class: 'ghost-btn' }, 'Copy link');
    copy.onclick = async () => {
      await navigator.clipboard.writeText(st.url).catch(() => {});
      copy.textContent = 'Copied';
      setTimeout(() => (copy.textContent = 'Copy link'), 1400);
    };
    const off = el('button', { class: 'solid-btn' }, 'Turn off');
    off.onclick = async () => {
      off.disabled = true;
      render(await api.unpair().catch(() => ({ paired: false })));
    };
    body.append(el('div', { class: 'form-actions' }, copy, off));
  };

  openSheet('#list-sheet');
  render(await api.pairStatus().catch(() => ({ paired: false, error: 'Could not reach the console.' })));
};

/**
 * Your own Mac's screen.
 *
 * Separate from the agent computer on purpose, and separate in the backend too:
 * the bots get containers, and nothing in their tool surface can reach this.
 * The same phone link that opens the console opens this, because it rides the
 * same authorised connection rather than a second credential.
 */
let macRfb = null;

function closeMac() {
  if (macRfb) {
    try {
      macRfb.disconnect();
    } catch {
      /* already gone */
    }
    macRfb = null;
  }
  $('#mac-screen').hidden = true;
  clear($('#mac-screen'));
  closeSheet('#mac-sheet');
}

$('#mac-close').onclick = closeMac;

async function connectMac(st) {
  const setup = $('#mac-setup');
  const screen = $('#mac-screen');
  const foot = $('#mac-foot');

  setup.hidden = true;
  screen.hidden = false;
  clear(screen);
  foot.textContent = 'Connecting…';
  $('#mac-off').hidden = false;

  const { default: RFB } = await import('/vendor/novnc/core/rfb.js');
  const scheme = location.protocol === 'https:' ? 'wss' : 'ws';
  const url = `${scheme}://${location.host}${tokenised('/api/remote/socket')}`;

  macRfb = new RFB(screen, url, { credentials: { password: st.secret } });
  macRfb.scaleViewport = true;
  macRfb.clipViewport = true;
  // The whole point is driving it, so the keyboard and pointer stay live. The
  // agent computer defaults the other way.
  macRfb.viewOnly = false;

  macRfb.addEventListener('connect', () => {
    foot.textContent = 'Connected — click or tap into the screen to drive it.';
  });
  macRfb.addEventListener('disconnect', (ev) => {
    foot.textContent = ev.detail?.clean ? 'Disconnected.' : 'Connection lost.';
  });
  macRfb.addEventListener('securityfailure', () => {
    foot.textContent = 'The Mac refused the screen-sharing password. Turn sharing off and on again to reset it.';
  });
}

$('#nav-mac').onclick = async () => {
  openSheet('#mac-sheet');
  const setup = $('#mac-setup');
  const screen = $('#mac-screen');
  screen.hidden = true;
  $('#mac-off').hidden = true;
  setup.hidden = false;
  clear(setup);
  setup.append(el('p', { class: 'muted' }, 'Checking…'));

  const render = (st) => {
    clear(setup);

    if (!st.supported) {
      setup.append(el('p', { class: 'muted' }, 'Screen Sharing is a macOS feature, and this console is not running on a Mac.'));
      return;
    }

    if (st.enabled && st.secret) {
      void connectMac(st);
      return;
    }

    setup.append(
      el('p', {}, 'Turn on macOS Screen Sharing to drive this Mac from here — or from your phone, over the same link.'),
      el(
        'p',
        { class: 'muted' },
        'macOS will ask for your Mac password once, in its own dialog. MyBot never sees it. Screen Sharing opens a port on your network, so leave it off when you are not using it.',
      ),
    );
    if (st.error) setup.append(el('p', { class: 'form-error' }, st.error));

    const go = el('button', { class: 'solid-btn' }, 'Turn on Screen Sharing');
    go.onclick = async () => {
      go.disabled = true;
      go.textContent = 'Waiting for the macOS prompt…';
      render(await api.remoteEnable().catch((e) => ({ ...st, error: e.message })));
    };
    setup.append(el('div', { class: 'form-actions' }, go));
  };

  render(await api.remoteStatus().catch((e) => ({ supported: true, enabled: false, error: e.message })));
};

$('#mac-off').onclick = async () => {
  const btn = $('#mac-off');
  btn.disabled = true;
  if (macRfb) {
    try {
      macRfb.disconnect();
    } catch {
      /* already gone */
    }
    macRfb = null;
  }
  await api.remoteDisable().catch(() => {});
  btn.disabled = false;
  closeMac();
};

$('#nav-skills').onclick = async () => {
  $('#list-title').textContent = 'Skills';
  const body = $('#list-body');
  clear(body);

  const skills = await api.skills().catch(() => []);
  if (!skills.length) {
    body.append(el('p', { class: 'muted' }, 'No skills yet. Add one with:  mybot skills add "<name>"'));
    openSheet('#list-sheet');
    return;
  }

  const grid = el('div', { class: 'grid' });
  const taught = skills.filter((s3) => s3.source === 'taught');
  const manual = skills.filter((s3) => s3.source !== 'taught');

  const add = (label, list) => {
    if (!list.length) return;
    grid.append(el('div', { class: 'grid-head' }, label));
    for (const sk of list) {
      const confirms = (sk.safety_rules ?? '').split('\n').filter((r) => /always-confirm/i.test(r));
      const tag = el('span', { class: 'card-action' }, confirms.length ? `${confirms.length} gated` : 'open');
      if (confirms.length) tag.dataset.on = 'true';
      grid.append(card(initials(sk.name), sk.name, sk.instructions.replace(/\s+/g, ' '), tag));
    }
  };

  add('Written by you', manual);
  add('Taught by demonstration', taught);
  body.append(grid);
  openSheet('#list-sheet');
};

$('#nav-routines').onclick = async () => {
  $('#list-title').textContent = 'Routines';
  const body = $('#list-body');
  clear(body);

  const routines = await api.routines().catch(() => []);
  if (!routines.length) {
    body.append(el('p', { class: 'muted' }, 'No routines yet. A routine runs a skill on a schedule or an event.'));
    openSheet('#list-sheet');
    return;
  }

  const grid = el('div', { class: 'grid' });
  for (const r of routines) {
    const name = r.skill_name ?? r.skill_id;
    const toggle = el('button', { class: 'card-action' }, r.enabled ? 'On' : 'Off');
    if (r.enabled) toggle.dataset.on = 'true';
    toggle.onclick = async () => {
      const next = !r.enabled;
      await api.setRoutineEnabled(r.id, next).catch(() => {});
      r.enabled = next;
      toggle.textContent = next ? 'On' : 'Off';
      if (next) toggle.dataset.on = 'true';
      else delete toggle.dataset.on;
    };
    grid.append(card(initials(name), name, r.schedule ? `cron ${r.schedule}` : `on ${r.trigger}`, toggle));
  }

  body.append(grid);
  openSheet('#list-sheet');
};

$('#list-close').onclick = () => closeSheet('#list-sheet');


// ── notifications ───────────────────────────────────────────────────────

/**
 * Tell the human a bot is stuck waiting on them.
 *
 * A paused run blocks until someone acts, and the whole point of an autonomous
 * teammate is that you are not watching. A title marker covers the case where
 * notifications were declined.
 */
let pausedCount = 0;

function notifyPaused(task) {
  const bot = state.bots.find((b) => b.id === task.bot_id);
  const who = bot ? bot.name : 'A bot';
  const why = splitPause(task.paused_reason).why;

  pausedCount += 1;
  document.title = `(${pausedCount}) MyBot — needs you`;

  if (!('Notification' in window)) return;
  if (Notification.permission === 'granted') {
    new Notification(`${who} needs you`, { body: why, tag: task.id });
  } else if (Notification.permission !== 'denied') {
    Notification.requestPermission().then((p) => {
      if (p === 'granted') new Notification(`${who} needs you`, { body: why, tag: task.id });
    });
  }
}

function clearPausedTitle() {
  pausedCount = 0;
  document.title = 'MyBot';
}
window.addEventListener('focus', clearPausedTitle);

// ── files the bots produced ─────────────────────────────────────────────

$('#nav-files').onclick = async () => {
  $('#list-title').textContent = 'Files';
  const body = $('#list-body');
  clear(body);

  const res = await api.files().catch(() => ({ files: [] }));
  const files = res.files ?? [];

  if (!files.length) {
    body.append(
      el('p', { class: 'muted' }, 'Nothing in /workspace yet. Files a bot writes there show up here.'),
    );
    openSheet('#list-sheet');
    return;
  }

  const grid = el('div', { class: 'grid' });
  for (const f of files) {
    const name = f.path.split('/').pop();
    const open = el('button', { class: 'card-action' }, 'Open');
    open.onclick = async () => {
      const doc = await api.files(f.path).catch((e) => ({ content: `Could not read: ${e.message}` }));
      $('#list-title').textContent = name;
      clear(body);
      const pre = el('pre', { class: 'file-body' });
      pre.textContent = doc.content ?? '';
      const back = el('button', { class: 'ghost-btn', style: 'flex:none;margin-bottom:10px' }, '← All files');
      back.onclick = () => $('#nav-files').click();
      body.append(back, pre);
    };
    grid.append(card('MD', name, `${f.path} · ${(f.bytes / 1024).toFixed(1)} KB`, open));
  }
  body.append(grid);
  openSheet('#list-sheet');
};

// ── live model list ─────────────────────────────────────────────────────

/**
 * Populate the model picker from the provider itself.
 *
 * Typing a model id by hand is a spelling test with a silent failure: the wrong
 * string only shows up as a failed run much later. Asking the provider also
 * means a plan that lacks a model simply never offers it.
 */
async function loadModels(provider) {
  const select = $('#bot-model-select');
  const note = $('#bot-model-note');
  clear(select);
  select.append(el('option', { value: '' }, 'loading…'));
  note.textContent = '';

  const res = await api.models(provider).catch(() => ({ models: [], error: 'could not reach the provider' }));
  clear(select);

  if (!res.models?.length) {
    select.append(el('option', { value: '' }, 'use the default'));
    note.textContent = res.error ? `No live list (${res.error}). The provider default will be used.` : '';
    return;
  }

  // Image models cannot drive an agent loop, so they are not offered here.
  const usable = res.models.filter((m) => !/image|dall|tts|whisper|embed/i.test(m));
  for (const m of usable) select.append(el('option', { value: m }, m));
  note.textContent = `${usable.length} models on this account`;
}

// ── narrow-screen back ──────────────────────────────────────────────────

$('#back-btn').onclick = () => {
  app.dataset.view = 'list';
  state.selected = null;
  renderRoster();
};

// ── live ────────────────────────────────────────────────────────────────

async function refreshBots() {
  state.bots = await api.bots().catch(() => []);
  renderRoster();
}

subscribe('/api/stream', {
  bots: (data) => {
    state.bots = data;
    renderRoster();
    if (state.selected) {
      const bot = state.bots.find((b) => b.id === state.selected);
      if (bot) $('#head-avatar').dataset.state = stateOf(bot);
      const running = runningTaskFor(state.selected);
      setRunning(Boolean(running), running);
    }
  },
  computer: (data) => {
    state.computer = data;
    renderComputer();
  },
  paused: (data) => {
    // A pause is the one event that happens while you are looking elsewhere and
    // that blocks everything until you act, so it earns an OS notification.
    const known = new Set(state.paused.map((t) => t.id));
    for (const t of data) if (!known.has(t.id)) notifyPaused(t);
    state.paused = data;
    renderTakeover();
    if (data.length) setRunning(false);
  },
  log: (entry) => {
    const bot = state.bots.find((b) => b.currentTask?.id === entry.taskId);
    if (bot && bot.id !== state.selected) return; // arrives in that thread on open

    // The composer already rendered your message optimistically the moment you
    // hit send. The loop then writes the same text to the log and it comes back
    // down this stream, so echoing it here showed every message twice.
    if (entry.kind === 'user') return;

    push(entryNode(entry, live));
  },
  task: () => void refreshBots(),
  result: (r) => {
    setRunning(false);
    live.open = null;
    // The loop's own `done` log entry already carried this summary; publishing
    // it again from the result event repeated the last line of every run.
    if (r.summary && r.status !== 'done') push(entryNode({ kind: 'done', text: r.summary }));
    void refreshBots();
  },
});

// ── boot ────────────────────────────────────────────────────────────────

(async function boot() {
  app.dataset.view = 'list';

  // One call gets bots, paused tasks and computer status together. Fetching
  // them separately left a five-second window after load where a bot parked at
  // a login wall looked idle, because the paused list only arrived on the
  // heartbeat.
  const st = await api.state().catch(() => null);
  if (st) {
    state.bots = st.bots ?? [];
    state.paused = st.paused ?? [];
    state.computer = st.computer ?? null;
  } else {
    state.bots = await api.bots().catch(() => []);
  }

  renderRoster();
  renderComputer();

  // On a phone the roster IS the screen — auto-opening a thread means every
  // launch starts inside a conversation you did not pick, one back-tap from
  // where you actually wanted to be. On a desktop the roster stays visible
  // beside the thread, so opening one costs nothing.
  const narrow = window.matchMedia('(max-width: 760px)').matches;
  if (!state.bots.length) emptyState.hidden = false;
  else if (!narrow) select(state.bots[0].id);
})();
