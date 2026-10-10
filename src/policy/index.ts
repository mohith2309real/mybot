/**
 * What a bot is allowed to do on its own authority.
 *
 * Three modes, set per bot or per run:
 *
 *   ask     Consequential actions stop and ask. The safe default.
 *   auto    Routine work proceeds; consequential actions still ask.
 *   bypass  Consequential actions proceed without asking.
 *
 * `bypass` is deliberately NOT "anything goes". Two categories are refused in
 * every mode, including bypass:
 *
 *   1. DESTRUCTIVE commands — wiping a disk, `rm -rf /`, a fork bomb. There is
 *      no legitimate agent task that needs these, and the cost of being wrong
 *      once is unrecoverable. A permission mode is a statement about how much
 *      the human wants to be interrupted, not permission to destroy the machine.
 *
 *   2. CREDENTIAL entry — typing a password, OTP, 2FA code, or card number.
 *      This is not a permissions question at all: the bot cannot see what it is
 *      typing into well enough to be trusted with a secret, and a wrong guess
 *      costs a lockout. It pauses for a human in every mode. That check lives in
 *      `src/computer/liveview.ts`; it is named here so the two stay in sync.
 *
 * Explicit instruction is consent. If the human's own task text says to send the
 * email, the bot sends the email — stopping to ask "shall I send the email?"
 * when told to send the email is theatre, not safety. What it must NOT do is
 * infer a grant it was never given, so grants are matched against the actual
 * instruction text and are scoped to the specific capability named.
 */

export type PermissionMode = 'ask' | 'auto' | 'bypass';

export const PERMISSION_MODES: PermissionMode[] = ['ask', 'auto', 'bypass'];

export function isPermissionMode(v: string): v is PermissionMode {
  return (PERMISSION_MODES as string[]).includes(v);
}

/**
 * Actions with consequences outside the sandbox. Lifted from xAI's own
 * approvals list for Grok Bot, which is a good inventory of the things a
 * teammate should not do unasked.
 */
export type Capability =
  | 'send_message'
  | 'publish'
  | 'purchase'
  | 'transfer_funds'
  | 'delete_data'
  | 'change_permissions'
  | 'production_change'
  | 'accept_terms'
  | 'host_access';

export const CAPABILITIES: Capability[] = [
  'send_message',
  'publish',
  'purchase',
  'transfer_funds',
  'delete_data',
  'change_permissions',
  'production_change',
  'accept_terms',
  'host_access',
];

export const CAPABILITY_LABEL: Record<Capability, string> = {
  send_message: 'send a message, email, or invitation',
  publish: 'publish or post content',
  purchase: 'make a purchase',
  transfer_funds: 'move money',
  delete_data: 'delete or overwrite data',
  change_permissions: 'change permissions or access',
  production_change: 'change something in production',
  accept_terms: 'accept legal terms',
  host_access: "read or write files on the human's own machine",
};

/**
 * Phrases in the human's instruction that grant a capability.
 *
 * Deliberately narrow: these match an instruction to DO the thing, not a
 * mention of it. "Draft an email" does not grant sending; "send the email"
 * does. Under-granting costs one confirmation prompt; over-granting sends mail
 * nobody asked for.
 */
const GRANT_PATTERNS: Record<Capability, RegExp> = {
  send_message: /\b(send|reply to|respond to|answer|forward|email|dm|message|invite)\b/i,
  publish: /\b(publish|post|tweet|upload|go live|submit)\b/i,
  purchase: /\b(buy|purchase|order|check ?out|pay for|book)\b/i,
  transfer_funds: /\b(transfer|send money|pay|wire|refund|withdraw|deposit)\b/i,
  delete_data: /\b(delete|remove|clear|wipe|purge|drop)\b/i,
  change_permissions: /\b(grant|revoke|share with|add .* as|change permissions?|make .* (admin|owner))\b/i,
  production_change: /\b(deploy|ship|release|push to prod|merge)\b/i,
  accept_terms: /\b(accept|agree to|sign)\b.*\b(terms|tos|policy|agreement|contract)\b/i,
  host_access: /\b(my (mac|laptop|computer|machine|desktop|downloads|documents)|host (files?|machine))\b/i,
};

/**
 * Words that cancel a grant when they appear shortly BEFORE the verb.
 *
 * The direction matters and is easy to get wrong: "draft an email but do not
 * send it" negates *backwards* from "send". A forward-only guard reads the tail
 * ("send it"), finds nothing negative after it, and cheerfully grants the very
 * capability the sentence just withheld.
 */
const NEGATORS = /\b(do ?n[o']?t|never|without|avoid|no need to|instead of|rather than|not)\b/i;

/** How far back to look for a negator, in characters. */
const NEGATION_WINDOW = 40;

function negatedAt(text: string, index: number): boolean {
  return NEGATORS.test(text.slice(Math.max(0, index - NEGATION_WINDOW), index));
}

/**
 * A determiner immediately before the word means it is a noun, not a command.
 *
 * Several of these words are both: "draft an *email*" names a thing, "*email*
 * the report" orders an action; likewise "the *post*" versus "*post* the
 * update". Without this, "draft an email but do not send it" grants sending —
 * it matches the noun "email" before ever reaching the negated verb.
 */
const DETERMINER = /\b(an?|the|my|your|our|its|their|this|that|these|those|any|some|another)\s+$/i;

function nounAt(text: string, index: number): boolean {
  return DETERMINER.test(text.slice(Math.max(0, index - 24), index));
}

/** Which capabilities the human's own words authorise for this run. */
export function grantsFromInstruction(instruction: string): Set<Capability> {
  const granted = new Set<Capability>();

  for (const cap of CAPABILITIES) {
    // Scan every occurrence: an early negated mention must not veto a later
    // genuine instruction, and vice versa.
    const pattern = new RegExp(GRANT_PATTERNS[cap].source, 'gi');
    for (const m of instruction.matchAll(pattern)) {
      if (m.index === undefined) continue;
      if (negatedAt(instruction, m.index)) continue;
      if (nounAt(instruction, m.index)) continue;
      granted.add(cap);
      break;
    }
  }

  return granted;
}

// ---------------------------------------------------------------------------
// Destructive commands — refused in every mode
// ---------------------------------------------------------------------------

interface DestructiveRule {
  label: string;
  /** Either a pattern over the normalised command, or a predicate over it. */
  match?: RegExp;
  test?: (normalised: string) => boolean;
}

/**
 * Paths that must never be the target of a recursive delete. Matched as whole
 * arguments, so `/workspace/tmp/build` is fine while `/` and `/etc` are not.
 */
const ROOTISH = new Set([
  '/', '/*', '/.', '~', '~/', '$HOME', '${HOME}', '/*/',
  '/bin', '/boot', '/dev', '/etc', '/home', '/lib', '/opt', '/proc', '/root',
  '/sbin', '/srv', '/sys', '/usr', '/var',
  '/System', '/Users', '/Applications', '/Library', '/Volumes',
]);

/**
 * Directories where a recursive delete at ANY depth is destructive — you do not
 * legitimately `rm -rf` something under /System or /usr from an agent loop.
 *
 * Notably absent: /workspace, /tmp, and paths *inside* a home directory. Those
 * are where the bot's actual work lives, and cleaning a build directory is a
 * normal thing to ask for. The line is drawn at system directories and at whole
 * home directories, not at "anything with a leading slash".
 */
const SYSTEM_PREFIXES = [
  '/System', '/Library', '/usr', '/bin', '/sbin', '/lib', '/lib64',
  '/etc', '/boot', '/dev', '/proc', '/sys', '/Applications', '/var',
];

function isRootish(arg: string): boolean {
  const bare = arg.replace(/^['"]|['"]$/g, '').replace(/\/+$/, (m, o: number) => (o === 0 ? '/' : ''));
  if (ROOTISH.has(bare) || ROOTISH.has(`${bare}/`) || /^(\/|~)\*?$/.test(bare)) return true;

  // Anything beneath a system directory.
  if (SYSTEM_PREFIXES.some((p) => bare === p || bare.startsWith(`${p}/`))) return true;

  // A whole home directory — /Users/mohith or /home/bot — but not the work
  // inside one, so `rm -rf /home/bot/project/dist` still runs.
  if (/^\/(Users|home)\/[^/]+\/?\*?$/.test(bare)) return true;

  return false;
}

/**
 * A recursive delete aimed at a system root.
 *
 * Written as an argument scan rather than one regex because the flags and the
 * path are not reliably adjacent: `rm -rf --no-preserve-root /` is the textbook
 * disaster and an adjacency pattern misses it entirely. Any appearance of
 * `--no-preserve-root` is itself disqualifying — that flag exists for no reason
 * other than defeating rm's own guard.
 */
function isRootDelete(normalised: string): boolean {
  // Check each command in a compound line separately.
  for (const segment of normalised.split(/&&|\|\||;|\|/)) {
    const tokens = segment.trim().split(/\s+/).filter(Boolean);
    const rmAt = tokens.findIndex((t) => t === 'rm' || t.endsWith('/rm'));
    if (rmAt === -1) continue;

    const rest = tokens.slice(rmAt + 1);
    if (rest.some((t) => t === '--no-preserve-root')) return true;

    const flags = rest.filter((t) => t.startsWith('-'));
    const recursive = flags.some((f) => /^-[a-z]*r/i.test(f) || f === '--recursive');
    if (!recursive) continue;

    const targets = rest.filter((t) => !t.startsWith('-'));
    // `rm -rf` with no target at all is harmless; with a rootish one it is not.
    if (targets.some(isRootish)) return true;
  }
  return false;
}

/**
 * Each rule names a specific irreversible act. Kept as an explicit list rather
 * than a cleverness heuristic so that a human can read it and agree with it.
 *
 * These are matched against the command the model wants to run. Matching is
 * necessarily approximate — a determined model could obfuscate its way past any
 * regex — so this is a guardrail against catastrophic *mistakes*, not a sandbox
 * against a hostile model. The real containment is the container itself.
 */
const DESTRUCTIVE: DestructiveRule[] = [
  { label: 'recursive delete of a root or system path', test: isRootDelete },
  { label: 'filesystem format', match: /\bmkfs(\.[a-z0-9]+)?\b|\bmke2fs\b/ },
  { label: 'partition table edit', match: /\b(fdisk|parted|sgdisk|diskutil\s+(eraseDisk|partitionDisk))\b/ },
  { label: 'raw write to a block device', match: /\bdd\b[^|;]*\bof=\/dev\/(disk|sd|nvme|hd)/ },
  { label: 'redirect over a block device', match: />\s*\/dev\/(disk|sd|nvme|hd)[a-z0-9]/ },
  { label: 'fork bomb', match: /:\s*\(\s*\)\s*\{\s*:\s*\|\s*:\s*&\s*\}\s*;\s*:/ },
  { label: 'recursive permission change on a system path', match: /\bchmod\s+(-[a-z]*R[a-z]*\s+)[0-7]{3,4}\s+(\/|\/etc|\/usr|\/bin|\/System)(\s|$)/ },
  { label: 'recursive ownership change on a system path', match: /\bchown\s+(-[a-z]*R[a-z]*\s+)\S+\s+(\/|\/etc|\/usr|\/bin|\/System)(\s|$)/ },
  { label: 'overwrite of the whole disk with zeros/random', match: /\bdd\b[^|;]*\bif=\/dev\/(zero|u?random)\b[^|;]*\bof=\// },
  { label: 'history/credential shredding', match: /\bshred\b[^|;]*(\/dev\/|\.ssh|\.aws|keychain)/i },
  { label: 'pipe of remote content straight into a shell', match: /\b(curl|wget)\b[^|]*\|\s*(sudo\s+)?(ba|z|k|d)?sh\b/ },
  { label: 'system shutdown or reboot', match: /\b(shutdown|reboot|halt|poweroff|init\s+0)\b/ },
  { label: 'mass process kill', match: /\bkill(all)?\s+-9\s+(-1|1)\b|\bpkill\s+-9\s+-u\b/ },
  { label: 'firewall flush', match: /\biptables\s+-F\b|\bufw\s+disable\b|\bpfctl\s+-d\b/ },
  { label: 'git history destruction', match: /\bgit\s+push\b[^;|]*--force[^;|]*\b(main|master|prod)\b|\bgit\s+reset\s+--hard\b[^;|]*&&[^;|]*\bpush\b/ },
];

export interface DestructiveVerdict {
  destructive: boolean;
  label?: string;
  matched?: string;
}

/** Does this shell command do something no permission mode should authorise? */
export function checkDestructive(command: string): DestructiveVerdict {
  // Normalise whitespace so `rm   -rf    /` reads the same as `rm -rf /`.
  const normalised = command.replace(/\s+/g, ' ').trim();
  for (const rule of DESTRUCTIVE) {
    if (rule.test?.(normalised)) return { destructive: true, label: rule.label, matched: normalised.slice(0, 80) };
    const m = rule.match ? normalised.match(rule.match) : null;
    if (m) return { destructive: true, label: rule.label, matched: m[0] };
  }
  return { destructive: false };
}

// ---------------------------------------------------------------------------
// The decision
// ---------------------------------------------------------------------------

export type Decision = 'allow' | 'confirm' | 'refuse';

export interface PolicyContext {
  mode: PermissionMode;
  /** Capabilities the human's instruction explicitly authorised. */
  grants?: Set<Capability>;
  /** Capabilities a skill's safety rules force a confirmation on, always. */
  alwaysConfirm?: Set<Capability>;
}

export interface PolicyRequest {
  capability?: Capability;
  /** For shell commands, so the destructive check can run. */
  command?: string;
}

export interface PolicyVerdict {
  decision: Decision;
  reason: string;
  /** Set when the verdict is `refuse` because of the destructive list. */
  destructive?: DestructiveVerdict;
}

export function decide(req: PolicyRequest, ctx: PolicyContext): PolicyVerdict {
  if (req.command) {
    const d = checkDestructive(req.command);
    if (d.destructive) {
      return {
        decision: 'refuse',
        destructive: d,
        reason:
          `Refused: this is a ${d.label} (matched \`${d.matched}\`). ` +
          `No permission mode authorises this, including bypass. ` +
          `If you genuinely need it, ask the human to run it themselves.`,
      };
    }
  }

  const cap = req.capability;
  if (!cap) return { decision: 'allow', reason: 'routine action' };

  // A skill can pin a capability to always-confirm regardless of mode. That is
  // the point of writing it into the skill's safety rules.
  if (ctx.alwaysConfirm?.has(cap)) {
    return {
      decision: 'confirm',
      reason: `The skill's safety rules require a human to confirm before you ${CAPABILITY_LABEL[cap]}.`,
    };
  }

  if (ctx.grants?.has(cap)) {
    return {
      decision: 'allow',
      reason: `The instruction explicitly asked you to ${CAPABILITY_LABEL[cap]}.`,
    };
  }

  if (ctx.mode === 'bypass') {
    return { decision: 'allow', reason: 'bypass mode' };
  }

  return {
    decision: 'confirm',
    reason:
      `About to ${CAPABILITY_LABEL[cap]}, which was not part of the instruction. ` +
      `Say what you are about to do and wait for a human.`,
  };
}

/**
 * The chunk of system prompt describing what this run may do on its own.
 *
 * Written as a description of the current grant rather than a list of
 * prohibitions, because a bot told "never send messages" on a run whose whole
 * purpose is sending a message will either disobey or stall.
 */
export function renderPolicyPrompt(ctx: PolicyContext): string {
  const lines: string[] = ['Authority for this run:'];

  const granted = [...(ctx.grants ?? [])].filter((c) => !ctx.alwaysConfirm?.has(c));
  if (granted.length) {
    lines.push(
      `- You were explicitly asked to ${granted.map((c) => CAPABILITY_LABEL[c]).join(', ')}. ` +
        `Do it. Do not stop to ask permission for what you were just told to do.`,
    );
  }

  if (ctx.mode === 'bypass') {
    lines.push('- Bypass mode: act without asking for routine and consequential steps alike.');
  } else if (ctx.mode === 'auto') {
    lines.push('- Auto mode: do the routine work without asking.');
  }

  const gated = CAPABILITIES.filter((c) => !granted.includes(c) && c !== 'host_access');
  if (ctx.mode !== 'bypass' && gated.length) {
    lines.push(
      `- Not covered by this instruction: ${gated.map((c) => CAPABILITY_LABEL[c]).join('; ')}. ` +
        `If the task turns out to need one, say what you would do and call request_human.`,
    );
  }

  if (ctx.alwaysConfirm?.size) {
    lines.push(
      `- Always confirm first, whatever the mode: ` +
        `${[...ctx.alwaysConfirm].map((c) => CAPABILITY_LABEL[c]).join('; ')}.`,
    );
  }

  lines.push(
    '- Never typed by you in any mode: passwords, one-time codes, 2FA codes, card numbers.',
    '  Sign in with fill_login (the human approves, MyBot fills a saved login). Codes and',
    '  cards pause for a human, who completes that one field and hands control back.',
    '- Never run in any mode: commands that destroy a system or a disk. Ask the human instead.',
  );

  return lines.join('\n');
}
