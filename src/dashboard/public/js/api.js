/* Thin wrapper over the dashboard's own JSON API and its SSE streams. */

/*
 * The pairing token, when this page was opened from a phone link.
 *
 * The link carries `?t=…`, which authorises the page load — and nothing else.
 * Every fetch, stream and socket after that is a separate request that has to
 * carry it too, or a paired phone loads the shell and then 401s on its own
 * data. Kept in sessionStorage so a reload inside the tab still works, and
 * scrubbed out of the visible URL so the key is not sitting in the address bar
 * for anyone glancing at the screen.
 */
const TOKEN = (() => {
  const url = new URL(location.href);
  const fromLink = url.searchParams.get('t');
  if (fromLink) {
    sessionStorage.setItem('mybot.token', fromLink);
    url.searchParams.delete('t');
    history.replaceState(null, '', url.pathname + url.search + url.hash);
    return fromLink;
  }
  return sessionStorage.getItem('mybot.token') ?? '';
})();

/** Append the token to a URL that cannot carry a header (SSE, WebSocket). */
export function tokenised(path) {
  if (!TOKEN) return path;
  return path + (path.includes('?') ? '&' : '?') + 't=' + encodeURIComponent(TOKEN);
}

async function request(method, path, body) {
  const headers = {};
  if (body !== undefined) headers['content-type'] = 'application/json';
  if (TOKEN) headers.authorization = `Bearer ${TOKEN}`;

  const res = await fetch(path, {
    method,
    headers: Object.keys(headers).length ? headers : undefined,
    body: body === undefined ? undefined : JSON.stringify(body),
  });

  const text = await res.text();
  let data = null;
  if (text) {
    try {
      data = JSON.parse(text);
    } catch {
      data = { error: text };
    }
  }

  if (!res.ok) {
    const err = new Error((data && data.error) || `${res.status} ${res.statusText}`);
    err.status = res.status;
    throw err;
  }
  return data;
}

export const api = {
  state: () => request('GET', '/api/state'),

  bots: () => request('GET', '/api/bots'),
  createBot: (bot) => request('POST', '/api/bots', bot),
  deleteBot: (id) => request('DELETE', `/api/bots/${encodeURIComponent(id)}`),

  tasks: (botId) => request('GET', botId ? `/api/tasks?bot=${encodeURIComponent(botId)}` : '/api/tasks'),
  task: (id) => request('GET', `/api/tasks/${encodeURIComponent(id)}`),
  run: (botId, instruction) => request('POST', '/api/tasks', { botId, instruction }),
  remoteStatus: () => request('GET', '/api/remote'),
  remoteEnable: () => request('POST', '/api/remote'),
  remoteDisable: () => request('DELETE', '/api/remote'),
  pairStatus: () => request('GET', '/api/pair'),
  pair: () => request('POST', '/api/pair'),
  unpair: () => request('DELETE', '/api/pair'),
  resume: (id, skipped = false) =>
    request('POST', `/api/tasks/${encodeURIComponent(id)}/resume`, { skipped }),
  stop: (id) => request('POST', `/api/tasks/${encodeURIComponent(id)}/stop`),

  models: (provider) => request('GET', `/api/models/${encodeURIComponent(provider)}`),
  files: (path) =>
    request('GET', path ? `/api/files?path=${encodeURIComponent(path)}` : '/api/files'),

  computer: () => request('GET', '/api/computer'),
  startComputer: () => request('POST', '/api/computer/up'),

  skills: () => request('GET', '/api/skills'),
  createSkill: (skill) => request('POST', '/api/skills', skill),
  updateSkill: (id, patch) => request('PUT', `/api/skills/${encodeURIComponent(id)}`, patch),
  deleteSkill: (id) => request('DELETE', `/api/skills/${encodeURIComponent(id)}`),

  routines: () => request('GET', '/api/routines'),
  setRoutineEnabled: (id, enabled) =>
    request('POST', `/api/routines/${encodeURIComponent(id)}/enabled`, { enabled }),
};

/**
 * Subscribe to a server-sent event stream.
 *
 * `handlers` maps event name -> callback. EventSource reconnects on its own,
 * which is why the server replays a task's stored log on connect: a dropped
 * socket mid-run rejoins the same log instead of starting blank.
 */
export function subscribe(path, handlers) {
  const source = new EventSource(tokenised(path));
  for (const [event, handler] of Object.entries(handlers)) {
    if (event === 'error') continue;
    source.addEventListener(event, (ev) => {
      let data = null;
      try {
        data = JSON.parse(ev.data);
      } catch {
        return;
      }
      handler(data);
    });
  }
  if (handlers.error) source.addEventListener('error', handlers.error);
  return source;
}
