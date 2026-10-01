import type { ModelProvider, ToolDefinition } from '../providers/types.ts';
import * as browser from './browser.ts';
import * as container from './container.ts';
import { exec } from './container.ts';
import * as hostaccess from './hostaccess.ts';
import * as merge from './merge.ts';
import * as liveview from './liveview.ts';
import type { Boundary, BoundaryKind } from './liveview.ts';
import { checkDestructive } from '../policy/index.ts';

/**
 * Who is running, and on whose behalf.
 *
 * Host access and merges both need to know which bot asked, which conversation
 * it belongs to, and what it was originally told to do — the arbiter judges
 * necessity against the actual task, not against the bot's own framing of it.
 */
export interface ToolContext {
  bot?: string;
  conversationId?: string;
  taskId?: string;
  instruction?: string;
  provider?: ModelProvider;
  model?: string;
  hostWaitMs?: number;
  mergeWaitMs?: number;
}

/**
 * The tool surface a bot gets on the computer.
 *
 * Elements are addressed by `ref` from the most recent page_read. Refs are
 * re-stamped on every snapshot, so a stale ref throws rather than clicking
 * whatever happens to occupy that slot now.
 */

export const TOOLS: ToolDefinition[] = [
  {
    name: 'page_read',
    description:
      'Read the current page: URL, title, visible text, and a numbered list of interactive elements. ' +
      'Call this before clicking or typing, and again after any action that changes the page — ' +
      'element refs are only valid until the next page_read.',
    parameters: { type: 'object', properties: {}, required: [] },
  },
  {
    name: 'page_navigate',
    description: 'Navigate the browser to a URL.',
    parameters: {
      type: 'object',
      properties: { url: { type: 'string', description: 'URL to open. https:// is assumed if omitted.' } },
      required: ['url'],
    },
  },
  {
    name: 'page_click',
    description: 'Click an element by its ref number from the most recent page_read.',
    parameters: {
      type: 'object',
      properties: { ref: { type: 'integer', description: 'Element ref from page_read' } },
      required: ['ref'],
    },
  },
  {
    name: 'page_type',
    description: 'Type text into an input by its ref number. Set submit=true to press Enter afterwards.',
    parameters: {
      type: 'object',
      properties: {
        ref: { type: 'integer', description: 'Element ref from page_read' },
        text: { type: 'string', description: 'Text to type' },
        submit: { type: 'boolean', description: 'Press Enter after typing' },
      },
      required: ['ref', 'text'],
    },
  },
  {
    name: 'page_scroll',
    description: 'Scroll the page up or down.',
    parameters: {
      type: 'object',
      properties: { direction: { type: 'string', enum: ['up', 'down'] } },
      required: ['direction'],
    },
  },
  {
    name: 'bash',
    description:
      'Run a bash command inside the computer, working directory /workspace. ' +
      'Use for file work, downloads, and data processing.',
    parameters: {
      type: 'object',
      properties: { command: { type: 'string', description: 'Command to run' } },
      required: ['command'],
    },
  },
  {
    name: 'request_human',
    description:
      'Hand control to a human and wait. Use when you judge a step needs a person — a sign-in you ' +
      'cannot complete, an identity check, anything you are unsure is safe to do unattended. ' +
      'The human opens a live view of this browser, does that one step, and hands control back; ' +
      'you then continue from wherever they left the page. Prefer this over giving up.',
    parameters: {
      type: 'object',
      properties: {
        reason: {
          type: 'string',
          description: 'What you need the human to do, in one line. They see this and nothing else.',
        },
      },
      required: ['reason'],
    },
  },
  {
    name: 'request_host_access',
    description:
      "Ask for one file from the human's own computer, when the task genuinely needs it and it is " +
      'not already in /workspace. Say exactly which file and why. The human decides; if they do not ' +
      'answer, another model judges whether the file is actually necessary. Credential files ' +
      '(.ssh, .aws, .env, keychains) are never shared and asking for them wastes a turn.',
    parameters: {
      type: 'object',
      properties: {
        path: { type: 'string', description: 'Path on the human\'s machine, e.g. ~/Downloads/report.pdf' },
        reason: { type: 'string', description: 'Why the task cannot proceed without this file' },
      },
      required: ['path', 'reason'],
    },
  },
  {
    name: 'merge_request',
    description:
      "Ask another conversation's computer to share with this one — its /workspace files, its " +
      'desktops, or both. The other side must agree; you cannot merge on your own authority. ' +
      'Use when work genuinely spans two conversations, e.g. you need a file another bot produced.',
    parameters: {
      type: 'object',
      properties: {
        conversation: { type: 'string', description: 'The other conversation id' },
        scope: { type: 'string', enum: ['workspace', 'desktop', 'both'] },
        reason: { type: 'string', description: 'Why the merge is needed' },
      },
      required: ['conversation', 'reason'],
    },
  },
  {
    name: 'list_desktops',
    description:
      'List the desktops on this computer — one per bot, each its own Linux user and session. ' +
      'Use before hopping to another desktop.',
    parameters: { type: 'object', properties: {}, required: [] },
  },
  {
    name: 'task_done',
    description: 'Call when the task is complete, with a summary of what you did and what you found.',
    parameters: {
      type: 'object',
      properties: { summary: { type: 'string', description: 'What you accomplished' } },
      required: ['summary'],
    },
  },
];

/** What the loop needs to hand a run over to a human. See computer/liveview.ts. */
export interface PauseRequest {
  kind: BoundaryKind;
  reason: string;
  viewUrl: string;
  /** The full detection, handed back to liveview on resume. */
  boundary: Boundary;
}

export interface ToolOutcome {
  content: string;
  isError?: boolean;
  /** Set when the model signalled completion. */
  done?: { summary: string };
  /**
   * Set when the action hit a security boundary. Not an error: the action did
   * not happen and the run stops here until a human deals with it.
   */
  pause?: PauseRequest;
}

function pauseOutcome(b: Boundary, prefix = ''): ToolOutcome {
  return {
    content: `${prefix}Paused — this needs a human (${b.kind}): ${b.reason}`,
    pause: { kind: b.kind, reason: b.reason, viewUrl: liveview.liveViewUrl(), boundary: b },
  };
}

export async function runTool(
  name: string,
  input: Record<string, unknown>,
  ctx: ToolContext = {},
): Promise<ToolOutcome> {
  try {
    switch (name) {
      case 'page_read': {
        const s = await browser.snapshot();
        return {
          content: `URL: ${s.url}\nTITLE: ${s.title}\n\nINTERACTIVE ELEMENTS:\n${s.elements}\n\nPAGE TEXT:\n${s.text}`,
        };
      }

      case 'page_navigate': {
        const r = await browser.navigate(String(input.url));
        const opened = `Opened ${r.url}\nTitle: ${r.title}\n`;
        // A checkout page or a CAPTCHA can appear with no action from the bot
        // at all, so landing on one is itself the boundary.
        const b = await liveview.detectBoundary({ action: 'navigate' });
        if (b) return pauseOutcome(b, opened);
        return { content: `${opened}Call page_read to see the page.` };
      }

      case 'page_click': {
        const ref = Number(input.ref);
        // Same fresh snapshot page_type takes: refs are re-stamped in DOM order,
        // so an unchanged page renumbers identically and we get the label of the
        // element that is actually about to be clicked.
        const snapshot = await browser.snapshot();
        const b = await liveview.detectBoundary({ action: 'click', ref, snapshot });
        if (b) return pauseOutcome(b);
        return { content: await browser.click(ref) };
      }

      case 'page_type': {
        const text = String(input.text ?? '');
        const ref = Number(input.ref);

        const snapshot = await browser.snapshot();
        const b = await liveview.detectBoundary({ action: 'type', ref, snapshot });
        if (b) return pauseOutcome(b);

        return { content: await browser.type(ref, text, Boolean(input.submit)) };
      }

      case 'page_scroll':
        return { content: await browser.scroll(input.direction === 'up' ? 'up' : 'down') };

      case 'bash': {
        const command = String(input.command ?? '');

        // Checked here, at the point of execution, rather than only in the
        // policy layer: this is the last gate before the command reaches a
        // shell, and it holds in every permission mode including bypass.
        const destructive = checkDestructive(command);
        if (destructive.destructive) {
          return {
            isError: true,
            content:
              `Refused: that is a ${destructive.label} (matched \`${destructive.matched}\`). ` +
              `No permission mode allows this, including bypass. If it is genuinely needed, ` +
              `ask the human to run it themselves and explain why.`,
          };
        }

        const r = await exec(command, { asBot: ctx.bot });
        const body = [r.stdout.trim(), r.stderr.trim()].filter(Boolean).join('\n');
        return {
          content: `exit ${r.code}\n${body || '(no output)'}`.slice(0, 8000),
          isError: r.code !== 0,
        };
      }

      case 'request_human': {
        const reason = String(input.reason ?? '').trim() || 'The bot asked for a human, without saying why.';
        let url = '';
        try {
          url = (await browser.activePage()).url();
        } catch {
          /* shell-only task — there is no page to name */
        }
        // Escalation by judgement, not by pattern: it is never suppressed by an
        // earlier acknowledgement, because the bot is asking about *this* step.
        return pauseOutcome({
          kind: 'always_confirm',
          reason,
          evidence: 'request_human',
          url,
          source: 'model',
        });
      }

      case 'task_done':
        return { content: 'Task marked complete.', done: { summary: String(input.summary ?? '') } };

      case 'request_host_access': {
        const path = String(input.path ?? '');
        const reason = String(input.reason ?? '');
        if (!path || !reason) {
          return { isError: true, content: 'request_host_access needs both a path and a reason.' };
        }

        const forbidden = hostaccess.isForbiddenPath(path);
        if (forbidden.forbidden) {
          return { isError: true, content: `${forbidden.why} Continue without it.` };
        }

        const result = await hostaccess.requestFile(
          { bot: ctx?.bot ?? 'bot', path, reason, taskId: ctx?.taskId },
          {
            waitMs: ctx?.hostWaitMs,
            provider: ctx?.provider,
            model: ctx?.model,
            taskInstruction: ctx?.instruction,
          },
        );

        switch (result.outcome) {
          case 'granted':
            return { content: `The human approved it. The file is at ${result.containerPath} (${result.bytes} bytes).` };
          case 'denied':
            return { content: `Denied: ${result.reason} Carry on without the file, or finish and say what you could not do.` };
          case 'not_needed':
            return { content: `No answer from the human, and this was judged unnecessary: ${result.reason}` };
          case 'escalated':
            return {
              content:
                `No answer from the human. This was judged genuinely necessary (${result.reason}), ` +
                `so it has been escalated. Do any other parts of the task you can, then finish and ` +
                `say what is still blocked.`,
            };
        }
        break;
      }

      case 'merge_request': {
        const other = String(input.conversation ?? '');
        const reason = String(input.reason ?? '');
        if (!other || !reason) {
          return { isError: true, content: 'merge_request needs a conversation and a reason.' };
        }

        const outcome = await merge.proposeAndWait(
          {
            from: ctx?.conversationId ?? container.DEFAULT_CONVERSATION,
            to: other,
            requestedBy: ctx?.bot ?? 'bot',
            scope: (input.scope as merge.MergeScope) ?? 'workspace',
          },
          { waitMs: ctx?.mergeWaitMs },
        );

        if (outcome.outcome === 'active') {
          return { content: `Both sides agreed. You can now reach ${other}'s ${outcome.merge.scope}.` };
        }
        if (outcome.outcome === 'refused') {
          return { content: `${other} declined the merge. Continue without it.` };
        }
        return {
          content:
            `${other} did not respond, so the decision has gone to the human. ` +
            `Do the parts of the task that do not need the merge, then report what is blocked.`,
        };
      }

      case 'list_desktops': {
        const s = container.maybeSession();
        if (!s) return { isError: true, content: 'No computer session is open.' };
        const desks = await container.listDesktops(s.computer);
        const names = Object.keys(desks);
        const shared = merge.sharedWith(s.computer.conversationId);
        return {
          content:
            `Desktops on this computer: ${names.length ? names.join(', ') : '(none yet)'}\n` +
            `You are on: ${s.desktop.bot} (user ${s.desktop.user}, display :${s.desktop.display})\n` +
            `Merged conversations you may reach: ${shared.desktops.join(', ') || '(none)'}`,
        };
      }

      default:
        return { isError: true, content: `Unknown tool: ${name}` };
    }
  } catch (err) {
    return { isError: true, content: err instanceof Error ? err.message : String(err) };
  }
}
