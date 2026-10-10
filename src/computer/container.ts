import { spawn } from 'node:child_process';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

/**
 * One container per CONVERSATION.
 *
 * This reverses the earlier account-wide design. The trade is worth stating,
 * because it is the reason `merge.ts` exists at all:
 *
 *   Shared machine  -> sign into Gmail once, every bot has it, no isolation.
 *   Per conversation -> real isolation, but each conversation starts logged out.
 *
 * Sharing is therefore no longer automatic; it is a thing two conversations opt
 * into, with both sides agreeing. See `src/computer/merge.ts`.
 *
 * Inside one container each bot gets its own Linux user, home, and XFCE desktop
 * on its own X display, created on demand by the in-container daemon
 * (docker/agentd.py). Bots hop between desktops by viewing a different port;
 * the users and sessions are unaffected by who is looking.
 *
 * Host ports are assigned by Docker (`-p 127.0.0.1::7000`) and discovered with
 * `docker port`, rather than computed from a counter. Several conversations run
 * at once and a computed port collides the moment two containers pick the same
 * arithmetic.
 */

const HERE = dirname(fileURLToPath(import.meta.url));
export const DOCKER_DIR = join(HERE, '..', '..', 'docker');

export const IMAGE = 'mybot-desktop:0.4'; // 0.4: the MyBot wallpaper on every desktop
export const AGENTD_CONTAINER_PORT = 7000;

/** Conversation id -> container name. Kept short; docker names dislike length. */
export function containerName(conversationId: string): string {
  return `mybot-conv-${conversationId.replace(/[^a-zA-Z0-9]/g, '').slice(0, 24).toLowerCase()}`;
}

export function workspaceVolume(conversationId: string): string {
  return `${containerName(conversationId)}-ws`;
}

export function homesVolume(conversationId: string): string {
  return `${containerName(conversationId)}-homes`;
}

export interface ExecResult {
  code: number;
  stdout: string;
  stderr: string;
}

export function run(cmd: string, args: string[], opts: { input?: string; cwd?: string } = {}): Promise<ExecResult> {
  return new Promise((resolve) => {
    const child = spawn(cmd, args, { cwd: opts.cwd });
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', (d) => (stdout += d));
    child.stderr.on('data', (d) => (stderr += d));
    child.on('error', (err) => resolve({ code: 127, stdout, stderr: String(err) }));
    child.on('close', (code) => resolve({ code: code ?? 0, stdout, stderr }));
    if (opts.input !== undefined) child.stdin.end(opts.input);
  });
}

const docker = (args: string[], opts?: { input?: string }) => run('docker', args, opts);

export async function dockerAvailable(): Promise<boolean> {
  return (await docker(['info', '--format', '{{.ServerVersion}}'])).code === 0;
}

export async function imageExists(): Promise<boolean> {
  return (await docker(['image', 'inspect', IMAGE])).code === 0;
}

export async function buildImage(onLine?: (line: string) => void): Promise<void> {
  await new Promise<void>((resolve, reject) => {
    const child = spawn('docker', ['build', '-t', IMAGE, '.'], { cwd: DOCKER_DIR });
    const feed = (d: Buffer) => {
      for (const line of d.toString().split('\n')) if (line.trim()) onLine?.(line);
    };
    child.stdout.on('data', feed);
    child.stderr.on('data', feed);
    child.on('close', (code) => (code === 0 ? resolve() : reject(new Error(`docker build failed (exit ${code})`))));
  });
}

export type ContainerState = 'running' | 'stopped' | 'absent';

export async function state(conversationId: string): Promise<ContainerState> {
  const r = await docker(['inspect', '-f', '{{.State.Running}}', containerName(conversationId)]);
  if (r.code !== 0) return 'absent';
  return r.stdout.trim() === 'true' ? 'running' : 'stopped';
}

/** Ask Docker which host port it bound to a container port. */
async function hostPort(name: string, containerPort: number): Promise<number | undefined> {
  const r = await docker(['port', name, String(containerPort)]);
  if (r.code !== 0) return undefined;
  const m = r.stdout.trim().split('\n')[0]?.match(/:(\d+)\s*$/);
  return m ? Number(m[1]) : undefined;
}

export interface Computer {
  conversationId: string;
  name: string;
  agentdPort: number;
}

/** Start (or attach to) the container for one conversation. */
export async function ensure(conversationId: string, onLine?: (line: string) => void): Promise<Computer> {
  if (!(await dockerAvailable())) {
    throw new Error('Docker daemon is not reachable. Start Docker Desktop, then retry.');
  }
  if (!(await imageExists())) {
    onLine?.('building the desktop image (first run — a few minutes)…');
    await buildImage(onLine);
  }

  const name = containerName(conversationId);
  const current = await state(conversationId);

  if (current === 'absent') {
    const r = await docker([
      'run', '-d',
      '--name', name,
      '--label', 'mybot=conversation',
      '--label', `mybot.conversation=${conversationId}`,
      // Ephemeral host ports: several conversations run at once.
      '-p', `127.0.0.1::${AGENTD_CONTAINER_PORT}`,
      ...desktopPortArgs(),
      '-v', `${workspaceVolume(conversationId)}:/workspace`,
      '-v', `${homesVolume(conversationId)}:/home`,
      // Chromium is a memory hog and /dev/shm defaults to 64MB under Docker.
      '--shm-size', '1g',
      IMAGE,
    ]);
    if (r.code !== 0) throw new Error(`docker run failed: ${r.stderr.trim()}`);
  } else if (current === 'stopped') {
    const r = await docker(['start', name]);
    if (r.code !== 0) throw new Error(`docker start failed: ${r.stderr.trim()}`);
  }

  const agentdPort = await waitForAgentd(name);
  return { conversationId, name, agentdPort };
}

/**
 * Publish a fixed band of desktop ports.
 *
 * Desktops are created at runtime, so their ports cannot be known at `docker
 * run` time — but ports can only be published when the container is created.
 * Publishing a band up front is what makes on-demand desktops reachable without
 * recreating the container. Eight is well past what one conversation needs.
 */
function desktopPortArgs(): string[] {
  const args: string[] = [];
  for (let d = 1; d <= 8; d++) {
    args.push('-p', `127.0.0.1::${6080 + d}`); // noVNC
    args.push('-p', `127.0.0.1::${9322 + d}`); // CDP relay
  }
  return args;
}

async function waitForAgentd(name: string, timeoutMs = 90_000): Promise<number> {
  const deadline = Date.now() + timeoutMs;
  let last = 'no attempt';
  while (Date.now() < deadline) {
    const port = await hostPort(name, AGENTD_CONTAINER_PORT);
    if (port) {
      try {
        const res = await fetch(`http://127.0.0.1:${port}/health`, { signal: AbortSignal.timeout(2000) });
        if (res.ok) return port;
        last = `HTTP ${res.status}`;
      } catch (err) {
        last = (err as { cause?: { code?: string } }).cause?.code ?? (err as Error).message;
      }
    } else {
      last = 'port not published yet';
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error(`agentd did not answer within ${timeoutMs / 1000}s (${last})`);
}

// --- desktops --------------------------------------------------------------

export interface Desktop {
  bot: string;
  user: string;
  display: number;
  /** Host URL a human opens to watch or take over. */
  viewUrl: string;
  /** Host port for CDP, once a browser has been started on this desktop. */
  cdpPort?: number;
}

/** Give this bot its own user + desktop inside the conversation's container. */
export async function desktop(computer: Computer, bot: string): Promise<Desktop> {
  const res = await fetch(`http://127.0.0.1:${computer.agentdPort}/desktops`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ name: normaliseBot(bot) }),
  });
  if (!res.ok) throw new Error(`could not create desktop: ${await res.text()}`);
  const info = (await res.json()) as { user: string; display: number; webPort: number };

  const web = await hostPort(computer.name, info.webPort);
  if (!web) throw new Error(`desktop ${info.display} is outside the published port band`);

  return {
    bot,
    user: info.user,
    display: info.display,
    viewUrl: `http://127.0.0.1:${web}/vnc.html?autoconnect=1&resize=scale&reconnect=1`,
  };
}

/** Start Chromium inside a bot's own desktop session and return its CDP port. */
export async function desktopBrowser(computer: Computer, bot: string): Promise<Desktop> {
  const d = await desktop(computer, bot);
  const res = await fetch(`http://127.0.0.1:${computer.agentdPort}/browser`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ name: normaliseBot(bot) }),
  });
  if (!res.ok) throw new Error(`could not start browser: ${await res.text()}`);
  const info = (await res.json()) as { cdpPort: number };

  const cdp = await hostPort(computer.name, info.cdpPort);
  if (!cdp) throw new Error('CDP port is outside the published band');

  // desktopctl backgrounds Chromium and returns immediately, so the port is
  // published before anything is listening on it. Without this wait the first
  // connect races the browser's startup and fails with an empty read — and
  // confusingly succeeds on the retry, because by then the browser is warm.
  await waitForCdp(cdp);

  d.cdpPort = cdp;
  return d;
}

/** Poll a CDP endpoint until DevTools answers, or give up. */
export async function waitForCdp(port: number, timeoutMs = 45_000): Promise<string> {
  const deadline = Date.now() + timeoutMs;
  let last = 'no attempt';
  while (Date.now() < deadline) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/json/version`, {
        signal: AbortSignal.timeout(2000),
      });
      if (res.ok) return ((await res.json()) as { Browser: string }).Browser;
      last = `HTTP ${res.status}`;
    } catch (err) {
      last = (err as { cause?: { code?: string } }).cause?.code ?? (err as Error).message;
    }
    await new Promise((r) => setTimeout(r, 400));
  }
  throw new Error(`Chromium did not expose CDP on :${port} within ${timeoutMs / 1000}s (${last})`);
}

export async function listDesktops(computer: Computer): Promise<Record<string, unknown>> {
  const res = await fetch(`http://127.0.0.1:${computer.agentdPort}/desktops`);
  return res.ok ? ((await res.json()) as { desktops: Record<string, unknown> }).desktops : {};
}

/** Bot names become Unix usernames, so they must survive that trip. */
export function normaliseBot(bot: string): string {
  const n = bot.toLowerCase().replace(/[^a-z0-9_-]/g, '-').replace(/^-+/, '').slice(0, 31);
  return n || 'bot';
}

// --- lifecycle -------------------------------------------------------------

export async function down(conversationId: string, remove = false): Promise<void> {
  const name = containerName(conversationId);
  await docker(['stop', name]);
  if (remove) await docker(['rm', name]);
}

/** Drop every logged-in session for one conversation, keeping /workspace. */
export async function resetProfiles(conversationId: string): Promise<void> {
  await down(conversationId, true);
  await docker(['volume', 'rm', homesVolume(conversationId)]);
}

/**
 * Run a shell command in a conversation's container.
 *
 * The conversation id is optional so the agent loop can call `exec(cmd)` and
 * hit whatever session is open, rather than threading an id through every tool.
 */
export async function exec(
  a: string,
  b?: string | { timeoutMs?: number; asBot?: string },
  c: { timeoutMs?: number; asBot?: string } = {},
): Promise<ExecResult> {
  const usingSession = typeof b !== 'string';
  const conversationId = usingSession ? session().computer.conversationId : a;
  const command = usingSession ? a : (b as string);
  const opts = usingSession ? ((b as { timeoutMs?: number; asBot?: string }) ?? {}) : c;

  const timeoutMs = opts.timeoutMs ?? 60_000;
  // Run as the bot's own user when one is named, so file ownership inside the
  // container matches whoever actually did the work.
  const args = opts.asBot
    ? ['exec', '-w', '/workspace', containerName(conversationId), 'sudo', '-u', `bot-${normaliseBot(opts.asBot)}`, 'bash', '-lc', command]
    : ['exec', '-w', '/workspace', containerName(conversationId), 'bash', '-lc', command];

  return new Promise((resolve) => {
    const child = spawn('docker', args);
    let stdout = '';
    let stderr = '';
    let done = false;
    const finish = (code: number) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      resolve({ code, stdout, stderr });
    };
    const timer = setTimeout(() => {
      child.kill('SIGKILL');
      stderr += `\n[timed out after ${timeoutMs}ms]`;
      finish(124);
    }, timeoutMs);
    child.stdout.on('data', (d) => (stdout += d));
    child.stderr.on('data', (d) => (stderr += d));
    child.on('error', (err) => { stderr += String(err); finish(127); });
    child.on('close', (code) => finish(code ?? 0));
  });
}

/** Push one approved host file into the container's inbox. */
export async function deliverFile(
  computer: Computer,
  bot: string,
  filename: string,
  contents: Buffer,
): Promise<{ path: string; bytes: number }> {
  const res = await fetch(`http://127.0.0.1:${computer.agentdPort}/files`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      name: normaliseBot(bot),
      path: filename,
      contentBase64: contents.toString('base64'),
    }),
  });
  if (!res.ok) throw new Error(`file delivery failed: ${await res.text()}`);
  return (await res.json()) as { path: string; bytes: number };
}

// --- the active session ----------------------------------------------------

/**
 * Which conversation + desktop the current process is driving.
 *
 * The browser and shell tools are called deep inside the agent loop and have no
 * business threading a conversation id through every call. A run opens a
 * session once; everything downstream asks for `activeCdpPort()` and friends.
 */
export interface Session {
  computer: Computer;
  desktop: Desktop;
}

let current: Session | undefined;

/** Conversation used when nothing more specific was chosen. */
export const DEFAULT_CONVERSATION = 'default';

export function setSession(s: Session): void {
  current = s;
}

export function session(): Session {
  if (!current) throw new Error('No computer session is open. Call open() first.');
  return current;
}

export function maybeSession(): Session | undefined {
  return current;
}

/** Start a conversation's container, give this bot its desktop, and select it. */
export async function open(
  opts: { conversationId?: string; bot?: string; onLine?: (line: string) => void } = {},
): Promise<Session> {
  const conversationId = opts.conversationId ?? DEFAULT_CONVERSATION;
  const computer = await ensure(conversationId, opts.onLine);
  const desk = await desktopBrowser(computer, opts.bot ?? 'main');
  const s: Session = { computer, desktop: desk };
  setSession(s);
  return s;
}

/** Legacy entry point: open the default conversation's computer. */
export async function up(onLine?: (line: string) => void): Promise<Session> {
  return open({ onLine });
}

export function activeCdpPort(): number {
  const d = session().desktop;
  if (!d.cdpPort) throw new Error('No browser is running on this desktop yet.');
  return d.cdpPort;
}

export function activeViewUrl(): string {
  return maybeSession()?.desktop.viewUrl ?? '';
}

export async function resetProfile(conversationId = DEFAULT_CONVERSATION): Promise<void> {
  return resetProfiles(conversationId);
}

export interface Status {
  docker: boolean;
  image: boolean;
  conversations: { conversationId: string; name: string; state: ContainerState }[];
  /** Present when a session is open, for the CLI and dashboard headers. */
  container?: ContainerState;
  browser?: string;
  cdpUrl?: string;
  viewUrl?: string;
}

export async function status(): Promise<Status> {
  const r = await docker(['ps', '-a', '--filter', 'label=mybot=conversation', '--format', '{{.Names}}\t{{.State}}\t{{.Label "mybot.conversation"}}']);
  const conversations = r.stdout
    .split('\n')
    .filter(Boolean)
    .map((line) => {
      const [name, st, conv] = line.split('\t');
      return {
        name: name ?? '',
        conversationId: conv ?? '',
        state: (st === 'running' ? 'running' : 'stopped') as ContainerState,
      };
    });

  const base: Status = { docker: await dockerAvailable(), image: await imageExists(), conversations };

  const s = maybeSession();
  if (s) {
    base.container = await state(s.computer.conversationId);
    base.viewUrl = s.desktop.viewUrl;
    if (s.desktop.cdpPort) {
      base.cdpUrl = `http://127.0.0.1:${s.desktop.cdpPort}`;
      try {
        const res = await fetch(`${base.cdpUrl}/json/version`, { signal: AbortSignal.timeout(2000) });
        if (res.ok) base.browser = ((await res.json()) as { Browser: string }).Browser;
      } catch {
        /* container up, browser still booting */
      }
    }
  }

  return base;
}
