import { randomUUID } from 'node:crypto';
import { EventEmitter } from 'node:events';
import { openDb } from '../db/index.ts';
import { containerName, exec, run } from './container.ts';

/**
 * Merging two conversations' computers.
 *
 * Per-conversation containers buy real isolation and cost automatic sharing:
 * signing into Gmail in one conversation does nothing for another. Merge is how
 * sharing comes back, and it is deliberately not something one side can do
 * alone — a bot that could pull another conversation's `/workspace` on its own
 * authority would make the isolation cosmetic.
 *
 * So: BOTH sides must grant. A non-response is not a grant. When the other side
 * never answers and the requesting bot still believes it needs the merge, the
 * decision goes to the human rather than defaulting either way.
 *
 * Scope is explicit:
 *   workspace  share files
 *   desktop    let each side view/drive the other's desktops
 *   both       both of the above
 */

export type MergeScope = 'workspace' | 'desktop' | 'both';
export type Consent = 'granted' | 'denied' | 'pending';
export type MergeStatus = 'pending' | 'active' | 'refused' | 'human_review' | 'revoked';

export interface Merge {
  id: string;
  from_conv: string;
  to_conv: string;
  scope: MergeScope;
  requested_by: string;
  from_consent: Consent;
  to_consent: Consent;
  status: MergeStatus;
  created_at: string;
  decided_at: string | null;
}

export const events = new EventEmitter();

export const DEFAULT_CONSENT_WAIT_MS = 3 * 60 * 1000;
const POLL_MS = 1000;

export function propose(input: {
  from: string;
  to: string;
  requestedBy: string;
  scope?: MergeScope;
}): Merge {
  if (input.from === input.to) throw new Error('A conversation cannot merge with itself.');
  const id = randomUUID();
  openDb()
    .prepare(
      `INSERT INTO merges (id, from_conv, to_conv, scope, requested_by, from_consent, to_consent)
       VALUES (?, ?, ?, ?, ?, 'granted', 'pending')`,
    )
    .run(id, input.from, input.to, input.scope ?? 'workspace', input.requestedBy);
  const m = get(id)!;
  events.emit('proposed', m);
  return m;
}

export function get(id: string): Merge | undefined {
  return openDb().prepare('SELECT * FROM merges WHERE id = ?').get(id) as unknown as Merge | undefined;
}

export function listPending(): Merge[] {
  return openDb()
    .prepare("SELECT * FROM merges WHERE status IN ('pending','human_review') ORDER BY created_at")
    .all() as unknown as Merge[];
}

export function listActive(): Merge[] {
  return openDb().prepare("SELECT * FROM merges WHERE status='active'").all() as unknown as Merge[];
}

/** The other side answers. Only this can move `to_consent` off pending. */
export function respond(id: string, consent: 'granted' | 'denied'): Merge {
  const m = get(id);
  if (!m) throw new Error(`No merge ${id}`);
  if (m.status !== 'pending' && m.status !== 'human_review') {
    throw new Error(`Merge ${id} is already ${m.status}.`);
  }

  const status: MergeStatus = consent === 'granted' ? 'active' : 'refused';
  openDb()
    .prepare("UPDATE merges SET to_consent=?, status=?, decided_at=datetime('now') WHERE id=?")
    .run(consent, status, id);

  const updated = get(id)!;
  events.emit(consent === 'granted' ? 'granted' : 'refused', updated);
  return updated;
}

/** Nobody answered and the requester still wants it — hand the call to a human. */
export function escalate(id: string): Merge {
  openDb().prepare("UPDATE merges SET status='human_review' WHERE id=?").run(id);
  const m = get(id)!;
  events.emit('escalated', m);
  return m;
}

export function revoke(id: string): void {
  openDb().prepare("UPDATE merges SET status='revoked', decided_at=datetime('now') WHERE id=?").run(id);
  events.emit('revoked', get(id));
}

export type MergeOutcome =
  | { outcome: 'active'; merge: Merge }
  | { outcome: 'refused'; merge: Merge }
  | { outcome: 'human_review'; merge: Merge };

/**
 * Propose and wait for the other side.
 *
 * On timeout this escalates to a human rather than resolving either way. That
 * is the point: silence must not read as yes (the isolation would be theatre)
 * and must not read as no either (a bot that flagged a genuine need deserves a
 * human decision, not a silent refusal).
 */
export async function proposeAndWait(
  input: { from: string; to: string; requestedBy: string; scope?: MergeScope },
  opts: { waitMs?: number; onWaiting?: (elapsedMs: number) => void } = {},
): Promise<MergeOutcome> {
  const m = propose(input);
  const waitMs = opts.waitMs ?? DEFAULT_CONSENT_WAIT_MS;
  const deadline = Date.now() + waitMs;

  while (Date.now() < deadline) {
    const now = get(m.id)!;
    if (now.status === 'active') return { outcome: 'active', merge: now };
    if (now.status === 'refused') return { outcome: 'refused', merge: now };
    opts.onWaiting?.(waitMs - (deadline - Date.now()));
    await new Promise((r) => setTimeout(r, POLL_MS));
  }

  return { outcome: 'human_review', merge: escalate(m.id) };
}

// --- applying an active merge ---------------------------------------------

export interface SharedView {
  /** Conversations whose /workspace this conversation may read. */
  workspaces: string[];
  /** Conversations whose desktops this conversation may view or drive. */
  desktops: string[];
}

/** What a conversation is currently allowed to reach, given active merges. */
export function sharedWith(conversationId: string): SharedView {
  const view: SharedView = { workspaces: [], desktops: [] };
  for (const m of listActive()) {
    const other =
      m.from_conv === conversationId ? m.to_conv : m.to_conv === conversationId ? m.from_conv : undefined;
    if (!other) continue;
    if (m.scope === 'workspace' || m.scope === 'both') view.workspaces.push(other);
    if (m.scope === 'desktop' || m.scope === 'both') view.desktops.push(other);
  }
  return view;
}

/**
 * Copy files between two merged conversations' workspaces.
 *
 * Docker cannot add a volume to a running container, and recreating both
 * containers mid-conversation would kill whatever the bots were doing. Copying
 * through the host is the option that does not interrupt anyone — and unlike a
 * live mount it means a later revoke actually removes future access rather than
 * leaving a mount in place.
 */
export async function syncWorkspace(fromConv: string, toConv: string, subpath = '.'): Promise<{ copied: boolean; detail: string }> {
  const active = listActive().some(
    (m) =>
      (m.scope === 'workspace' || m.scope === 'both') &&
      ((m.from_conv === fromConv && m.to_conv === toConv) || (m.from_conv === toConv && m.to_conv === fromConv)),
  );
  if (!active) {
    return { copied: false, detail: `No active workspace merge between ${fromConv} and ${toConv}.` };
  }

  // Stage through a temp DIRECTORY, not a temp file. `docker cp src dest` where
  // dest does not exist renames the file to dest — so copying out to
  // /tmp/mybot-merge-ab12 and back in produced a file called "mybot-merge-ab12"
  // on the far side, and reading it by its real name found nothing.
  const stage = `/tmp/mybot-merge-${randomUUID().slice(0, 8)}`;
  const mk = await run('mkdir', ['-p', stage]);
  if (mk.code !== 0) return { copied: false, detail: `could not stage: ${mk.stderr.trim()}` };

  const src = `${containerName(fromConv)}:/workspace/${subpath}`;
  const dst = `${containerName(toConv)}:/workspace/`;

  try {
    const pull = await run('docker', ['cp', src, `${stage}/`]);
    if (pull.code !== 0) return { copied: false, detail: `copy out failed: ${pull.stderr.trim()}` };

    // The trailing /. copies the directory's CONTENTS, so names are preserved
    // and nothing is nested under an extra folder on arrival.
    const push = await run('docker', ['cp', `${stage}/.`, dst]);
    if (push.code !== 0) return { copied: false, detail: `copy in failed: ${push.stderr.trim()}` };

    return { copied: true, detail: `Copied ${subpath} from ${fromConv} to ${toConv}.` };
  } finally {
    await run('rm', ['-rf', stage]);
  }
}

/** A merged conversation's files, listed without copying anything. */
export async function peekWorkspace(conversationId: string, subpath = '.'): Promise<string> {
  const r = await exec(conversationId, `ls -la ${JSON.stringify(`/workspace/${subpath}`)}`);
  return r.code === 0 ? r.stdout.trim() : r.stderr.trim();
}
