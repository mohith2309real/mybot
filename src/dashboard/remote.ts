import { execFile } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { connect } from 'node:net';
import { chmod, mkdir, readFile, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';
import { promisify } from 'node:util';

const run = promisify(execFile);

/**
 * Your Mac's own screen, reachable from your phone.
 *
 * This is deliberately NOT part of the agent's world. Bots get containers with
 * their own Linux desktops; the whole point of that design is that a bot never
 * touches the host. Nothing here is exposed as a tool, and no code path from
 * the agent loop reaches this module — it is reachable only from the console
 * UI, which is to say from you.
 *
 * macOS has a VNC server built in (Screen Sharing). Turning it on is a system
 * security setting, so it is gated by an admin prompt that macOS presents
 * itself — the password is typed into the OS dialog, never into this app, and
 * never passes through the console.
 */

export const VNC_PORT = 5900;

const KICKSTART =
  '/System/Library/CoreServices/RemoteManagement/ARDAgent.app/Contents/Resources/kickstart';

/** Where the generated screen-sharing password lives, 0600, outside the repo. */
const SECRET_PATH = join(homedir(), '.mybot', 'remote.key');

export interface RemoteStatus {
  /** Screen Sharing is running and accepting connections. */
  enabled: boolean;
  /** A password has been generated, so the viewer can authenticate. */
  configured: boolean;
  platform: string;
  supported: boolean;
}

/** Is anything listening on the VNC port? */
function portOpen(port: number, timeoutMs = 1200): Promise<boolean> {
  return new Promise((resolve) => {
    const socket = connect({ port, host: '127.0.0.1' });
    const done = (open: boolean) => {
      socket.destroy();
      resolve(open);
    };
    socket.setTimeout(timeoutMs);
    socket.once('connect', () => done(true));
    socket.once('timeout', () => done(false));
    socket.once('error', () => done(false));
  });
}

async function readSecret(): Promise<string | undefined> {
  try {
    const raw = (await readFile(SECRET_PATH, 'utf8')).trim();
    return raw || undefined;
  } catch {
    return undefined;
  }
}

export async function status(): Promise<RemoteStatus> {
  const supported = process.platform === 'darwin';
  return {
    supported,
    platform: process.platform,
    enabled: supported ? await portOpen(VNC_PORT) : false,
    configured: Boolean(await readSecret()),
  };
}

/**
 * The password the viewer authenticates with.
 *
 * Generated, not chosen: this is never typed by a human, so it may as well be
 * random. Capped at 8 characters because that is all the VNC authentication
 * scheme actually uses — a longer one would be silently truncated by the
 * server, and then the viewer's copy would not match.
 */
async function ensureSecret(): Promise<string> {
  const existing = await readSecret();
  if (existing) return existing;

  const alphabet = 'abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789';
  const bytes = randomBytes(8);
  const secret = Array.from(bytes, (b) => alphabet[b % alphabet.length]).join('');

  await mkdir(dirname(SECRET_PATH), { recursive: true });
  await writeFile(SECRET_PATH, secret, { mode: 0o600 });
  await chmod(SECRET_PATH, 0o600);
  return secret;
}

/** Shell-quote for the string macOS hands to `do shell script`. */
function q(value: string): string {
  return `'${value.replace(/'/g, `'\\''`)}'`;
}

export interface EnableResult {
  ok: boolean;
  status: RemoteStatus;
  error?: string;
}

/**
 * Turn Screen Sharing on.
 *
 * `with administrator privileges` makes macOS raise its own authentication
 * dialog on the Mac. That prompt is the point: enabling a remote-control
 * service is exactly the kind of change that should require someone to be
 * physically at the machine, and routing it through the OS means this app never
 * sees, stores, or transmits the password that approves it.
 */
export async function enable(): Promise<EnableResult> {
  if (process.platform !== 'darwin') {
    return { ok: false, status: await status(), error: 'Screen Sharing is a macOS feature.' };
  }

  const secret = await ensureSecret();

  // `-setvnclegacy` is what makes the built-in server speak the standard VNC
  // password handshake. Without it macOS offers only its own authentication
  // types, which no standard viewer — including noVNC — can complete.
  const script =
    `do shell script ${q(
      `${KICKSTART} -activate -configure -access -on -restart -agent -privs -all ` +
        `-clientopts -setvnclegacy -vnclegacy yes -setvncpw -vncpw ${secret}`,
    )} with administrator privileges`;

  try {
    await run('osascript', ['-e', script], { timeout: 120_000 });
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    // -128 is AppleScript for "the human closed the dialog".
    const cancelled = /-128|User canceled/i.test(message);
    return {
      ok: false,
      status: await status(),
      error: cancelled
        ? 'Cancelled at the macOS password prompt, so nothing changed.'
        : message,
    };
  }

  // The agent restarts asynchronously; the port is not open the instant
  // kickstart returns.
  for (let i = 0; i < 20; i++) {
    if (await portOpen(VNC_PORT)) break;
    await new Promise((r) => setTimeout(r, 500));
  }

  const now = await status();
  return {
    ok: now.enabled,
    status: now,
    error: now.enabled ? undefined : 'Screen Sharing did not come up. Check System Settings › General › Sharing.',
  };
}

export async function disable(): Promise<EnableResult> {
  if (process.platform !== 'darwin') {
    return { ok: false, status: await status(), error: 'Screen Sharing is a macOS feature.' };
  }

  const script = `do shell script ${q(`${KICKSTART} -deactivate -configure -access -off`)} with administrator privileges`;
  try {
    await run('osascript', ['-e', script], { timeout: 120_000 });
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    const cancelled = /-128|User canceled/i.test(message);
    return { ok: false, status: await status(), error: cancelled ? 'Cancelled, so nothing changed.' : message };
  }

  return { ok: true, status: await status() };
}

/**
 * The credential the viewer needs.
 *
 * Only ever returned to an already-authorised console request — the same
 * request that could drive the screen anyway, so handing it over adds no reach
 * that the caller did not already have.
 */
export async function viewerSecret(): Promise<string | undefined> {
  return readSecret();
}
