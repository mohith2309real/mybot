/*
 * The agent log.
 *
 * This is the screen a person actually reads, usually while it is still moving,
 * so the rendering job is to make four different things distinguishable at a
 * glance while scrolling fast:
 *
 *   what the bot was asked   — a quoted brief, once, at the top
 *   what the model said      — prose, in the one cool hue in the palette
 *   what the bot did         — mono, "→ tool  args"
 *   what came back           — mono, dimmed, "← tool  result", clamped
 *
 * Everything below builds nodes with textContent. Tool results are page text
 * scraped off the live internet; treating them as markup would be a hole.
 */

import { clock, el } from './dom.js';

const BLOCK_LABEL = {
  done: 'Result',
  error: 'Error',
  refusal: 'Refused',
  handoff: 'Handoff',
};

const MAX_VALUE = 200;

function truncate(value, limit) {
  const s = String(value);
  return s.length > limit ? `${s.slice(0, limit)}…` : s;
}

/** `{"url":"https://x"}` reads worse than `url: https://x` at a glance. */
function formatArgs(raw) {
  const trimmed = raw.trim();
  if (!trimmed) return '';
  try {
    const parsed = JSON.parse(trimmed);
    if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
      const parts = Object.entries(parsed).map(([key, value]) => {
        const text = typeof value === 'string' ? value : JSON.stringify(value);
        return `${key}: ${truncate(text, MAX_VALUE)}`;
      });
      if (parts.length) return parts.join('   ');
      return '';
    }
  } catch {
    /* not JSON — show it as it came */
  }
  return truncate(trimmed, MAX_VALUE * 2);
}

function toolLine(text, arrow) {
  // The loop emits "name {json}" for calls and "name -> result" for results.
  const separator = text.indexOf(' -> ');
  let name;
  let rest;
  if (separator !== -1) {
    name = text.slice(0, separator).trim();
    rest = text.slice(separator + 4);
  } else {
    const space = text.indexOf(' ');
    name = space === -1 ? text : text.slice(0, space);
    rest = space === -1 ? '' : text.slice(space + 1);
  }
  return { arrow, name, rest };
}

function bodyForCall(text) {
  const { name, rest } = toolLine(text, '→');
  return el('div', { class: 'log-body' },
    el('span', { class: 'log-arrow', 'aria-hidden': 'true', text: '→' }),
    el('span', { class: 'log-tool', text: name }),
    rest ? '  ' : null,
    rest ? el('span', { class: 'log-args', text: formatArgs(rest) }) : null,
  );
}

function bodyForResult(text) {
  const { name, rest } = toolLine(text, '←');
  const long = rest.length > 220 || rest.split('\n').length > 3;
  const content = el('span', { class: long ? 'log-clamp' : null, text: rest });

  const body = el('div', { class: 'log-body' },
    el('span', { class: 'log-arrow', 'aria-hidden': 'true', text: '←' }),
    el('span', { class: 'log-tool', text: name }),
    rest ? '  ' : null,
    rest ? content : null,
  );

  if (long) {
    let open = false;
    const toggle = el('button', {
      type: 'button',
      class: 'log-more',
      text: 'show all',
      onclick: () => {
        open = !open;
        content.classList.toggle('log-clamp', !open);
        toggle.textContent = open ? 'show less' : 'show all';
      },
    });
    body.appendChild(toggle);
  }
  return body;
}

function bodyForBlock(kind, text) {
  return el('div', { class: 'log-body' },
    el('div', { class: 'log-block' },
      el('span', { class: 'log-block-label', text: BLOCK_LABEL[kind] ?? kind }),
      text,
    ),
  );
}

export function renderEntry(entry) {
  const kind = entry.kind ?? 'note';
  const text = entry.text ?? '';

  let body;
  if (kind === 'tool_call') body = bodyForCall(text);
  else if (kind === 'tool_result' || kind === 'tool_error') body = bodyForResult(text);
  else if (BLOCK_LABEL[kind]) body = bodyForBlock(kind, text);
  else body = el('div', { class: 'log-body', text });

  return el('li', { class: 'log-row', dataset: { kind } },
    el('span', { class: 'log-time', text: clock(entry.at) }),
    body,
  );
}

/**
 * Owns the scrolling list.
 *
 * Sticks to the bottom while the reader is at the bottom, and stops the moment
 * they scroll up — a log that yanks you back to the tail while you are reading
 * something is worse than no auto-scroll at all.
 */
export function createLogView(list, jumpButton) {
  let stick = true;
  let lastTime = '';

  const atBottom = () => list.scrollHeight - list.scrollTop - list.clientHeight < 64;

  const setStick = (value) => {
    stick = value;
    jumpButton.hidden = value;
  };

  const toBottom = () => {
    list.scrollTop = list.scrollHeight;
    setStick(true);
  };

  list.addEventListener('scroll', () => {
    const bottom = atBottom();
    if (bottom !== stick) setStick(bottom);
  }, { passive: true });

  jumpButton.addEventListener('click', toBottom);

  return {
    reset() {
      list.replaceChildren();
      lastTime = '';
      setStick(true);
    },

    append(entry, { animate = true } = {}) {
      const row = renderEntry(entry);
      const time = clock(entry.at);
      // Only mark the row where the clock actually moved. A column of identical
      // timestamps is noise; one per second is a rhythm.
      if (time !== lastTime) {
        row.dataset.tick = '1';
        lastTime = time;
      }
      if (animate) row.classList.add('is-new');
      list.appendChild(row);
      if (stick) list.scrollTop = list.scrollHeight;
      return row;
    },

    isEmpty() {
      return list.childElementCount === 0;
    },

    toBottom,
  };
}
