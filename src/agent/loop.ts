import type { ContentPart, Message, ModelProvider } from '../providers/types.ts';
import { TOOLS, runTool, type PauseRequest } from '../computer/tools.ts';
import {
  acknowledge,
  awaitResume,
  resetAcknowledged,
  DEFAULT_PAUSE_TIMEOUT_MS,
  type ResumeOutcome,
} from '../computer/liveview.ts';
import { appendTaskLog, pauseTask, setTaskStatus } from '../db/index.ts';

/**
 * The agent loop: model -> tool calls -> results -> model, until the bot calls
 * task_done or hits the iteration cap.
 *
 * Everything is written to the task log as it happens, so a run is auditable
 * after the fact and the step-8 dashboard has something to render.
 */

const SYSTEM = `You are a bot operating a real computer with a browser, a shell, and a filesystem.

The computer is shared with the other bots on this account: same /workspace, same
browser profile, same logged-in sessions. Anything you sign into stays signed in
for everyone. Leave the machine in a sane state.

How to work:
- Call page_read before acting on a page, and again after anything that changes it.
  Element refs are only valid until the next page_read.
- Prefer the shell for file and data work; prefer the browser for the web.
- Work in /workspace. It persists across runs.
- When you are finished, call task_done with a summary. Do not stop without it.

Boundaries — these are hard limits, not preferences:
- Never type passwords, one-time codes, 2FA codes, or card details. The harness
  stops you at those fields anyway: the run pauses, a human opens a live view of
  your browser, completes that one step, and hands control back. When that
  happens, do not retry the step they just did — call page_read and carry on.
- Call request_human whenever you judge a step needs a person. That is a normal
  move, not a failure, and it is better than guessing or giving up.
- Do not send messages, publish content, make purchases, transfer money, delete
  data, or accept legal terms. Report what you would do and let the human decide.`;

/** What the human is shown when a run stops at a boundary. */
export interface PauseNotice {
  kind: string;
  reason: string;
  viewUrl: string;
  taskId?: string;
  timeoutMs: number;
}

export interface RunOptions {
  provider: ModelProvider;
  /** Earlier turns with this bot, so follow-ups have context. */
  history?: Message[];
  /** Abort a run in flight (the Stop button). */
  signal?: AbortSignal;
  model?: string;
  instruction: string;
  taskId?: string;
  maxIterations?: number;
  onEvent?: (kind: string, text: string) => void;
  /** How long a pause waits for a human. Default 15 min. */
  pauseTimeoutMs?: number;
  /**
   * Where the pause banner goes. Unset means the loop prints it itself — a
   * live-view URL nobody sees is a run that hangs for fifteen silent minutes.
   */
  onPause?: (notice: PauseNotice) => void;
}

export interface RunResult {
  status: 'done' | 'max_iterations' | 'refused' | 'error' | 'paused_timeout' | 'cancelled';
  summary: string;
  iterations: number;
  usage: { inputTokens: number; outputTokens: number };
}

export async function runTask(opts: RunOptions): Promise<RunResult> {
  const { provider, instruction, taskId } = opts;
  const maxIterations = opts.maxIterations ?? 25;
  const pauseTimeoutMs = opts.pauseTimeoutMs ?? DEFAULT_PAUSE_TIMEOUT_MS;

  // A takeover the human granted on a previous task is not consent for this one.
  resetAcknowledged();

  const emit = (kind: string, text: string) => {
    opts.onEvent?.(kind, text);
    if (taskId) appendTaskLog(taskId, kind, text);
  };

  // Prior turns with this bot, so a follow-up like "continue" or "now do the
  // other one" has something to refer to. Without this every task started from
  // nothing and the bot answered a question it had never been asked.
  const messages: Message[] = [...(opts.history ?? []), { role: 'user', content: instruction }];
  const usage = { inputTokens: 0, outputTokens: 0 };

  if (taskId) setTaskStatus(taskId, 'running');
  emit('user', instruction);

  for (let i = 1; i <= maxIterations; i++) {
    if (opts.signal?.aborted) {
      emit('cancelled', 'Stopped by you.');
      if (taskId) setTaskStatus(taskId, 'cancelled');
      return { status: 'cancelled', summary: 'Stopped by you.', iterations: i, usage };
    }
    let assistant: Message | undefined;
    let stopReason = '';
    let text = '';
    let thinking = '';
    const calls: { id: string; name: string; input: Record<string, unknown> }[] = [];

    for await (const chunk of provider.chat(messages, {
      model: opts.model,
      system: SYSTEM,
      tools: TOOLS,
      maxTokens: 8000,
    })) {
      switch (chunk.type) {
        case 'text_delta':
          text += chunk.text;
          break;
        case 'thinking_delta':
          // Buffered rather than emitted per-token: a per-delta event would
          // write one log row per word.
          thinking += chunk.text;
          break;
        case 'tool_call':
          calls.push({ id: chunk.id, name: chunk.name, input: chunk.input });
          break;
        case 'done':
          assistant = chunk.message;
          stopReason = chunk.stopReason;
          usage.inputTokens += chunk.usage.inputTokens;
          usage.outputTokens += chunk.usage.outputTokens;
          break;
        case 'error':
          emit('error', chunk.error);
          if (taskId) setTaskStatus(taskId, 'failed');
          return { status: 'error', summary: chunk.error, iterations: i, usage };
      }
    }

    if (thinking.trim()) emit('thinking', thinking.trim());
    if (text.trim()) emit('assistant', text.trim());

    if (stopReason === 'refusal') {
      const msg = 'The model declined this task.';
      emit('refusal', msg);
      if (taskId) setTaskStatus(taskId, 'failed');
      return { status: 'refused', summary: msg, iterations: i, usage };
    }

    // No tools requested -> the bot considers itself finished, even without
    // task_done. Take the text as the answer rather than looping pointlessly.
    if (!calls.length) {
      if (taskId) setTaskStatus(taskId, 'done');
      return { status: 'done', summary: text.trim() || '(no output)', iterations: i, usage };
    }

    messages.push(assistant ?? { role: 'assistant', content: buildAssistantParts(text, calls) });

    const results: ContentPart[] = [];
    let resultsSent = false;
    for (let c = 0; c < calls.length; c++) {
      const call = calls[c]!;
      emit('tool_call', `${call.name} ${JSON.stringify(call.input)}`);
      const outcome = await runTool(call.name, call.input);

      if (outcome.pause) {
        const handover = await handOver(outcome.pause, {
          emit,
          taskId,
          timeoutMs: pauseTimeoutMs,
          onPause: opts.onPause,
        });

        results.push({ type: 'tool_result', toolCallId: call.id, content: `${outcome.content}\n\n${handover.note}` });
        // Whatever else the model queued this turn was planned against a page a
        // human has since been driving. Never run it blind — and never let a
        // batched "type password, then click Sign in" survive the handover.
        for (const skipped of calls.slice(c + 1)) {
          results.push({
            type: 'tool_result',
            toolCallId: skipped.id,
            content: 'Not run: the run paused for a human first. Call page_read, then redo this only if still needed.',
          });
        }
        messages.push({ role: 'user', content: results });
        resultsSent = true;

        // Skipping continues the run for the same reason resuming does: the
        // note in the tool result already tells the model the step is dead, and
        // the point of skipping is to get the rest of the task done anyway.
        if (handover.outcome === 'resumed' || handover.outcome === 'skipped') break;

        return {
          status: handover.outcome === 'timeout' ? 'paused_timeout' : 'cancelled',
          summary: handover.note,
          iterations: i,
          usage,
        };
      }

      emit(outcome.isError ? 'tool_error' : 'tool_result', `${call.name} -> ${preview(outcome.content)}`);

      results.push({
        type: 'tool_result',
        toolCallId: call.id,
        content: outcome.content,
        ...(outcome.isError ? { isError: true } : {}),
      });

      if (outcome.done) {
        // Still send the tool result back so the transcript stays well-formed
        // for any follow-up turn on this task.
        messages.push({ role: 'user', content: results });
        if (taskId) setTaskStatus(taskId, 'done');
        emit('done', outcome.done.summary);
        return { status: 'done', summary: outcome.done.summary, iterations: i, usage };
      }
    }

    // A resumed pause already pushed this turn's results.
    if (!resultsSent) messages.push({ role: 'user', content: results });
  }

  const msg = `Stopped after ${maxIterations} iterations without calling task_done.`;
  emit('error', msg);
  if (taskId) setTaskStatus(taskId, 'failed');
  return { status: 'max_iterations', summary: msg, iterations: maxIterations, usage };
}

interface HandOverContext {
  emit: (kind: string, text: string) => void;
  taskId?: string;
  timeoutMs: number;
  onPause?: (notice: PauseNotice) => void;
}

/**
 * Stop, show the human the browser, and wait.
 *
 * The only thing happening between the pause and the resume is a poll of this
 * task's own row — nothing touches the page. That is the point: the human is
 * driving the same browser, and a bot retrying the sensitive step underneath
 * them is exactly the failure mode this whole boundary exists to prevent.
 */
async function handOver(
  pause: PauseRequest,
  ctx: HandOverContext,
): Promise<{ outcome: ResumeOutcome; note: string }> {
  const label = `${pause.kind}: ${pause.reason}`;
  ctx.emit('pause', `${label} — live view ${pause.viewUrl}`);
  if (ctx.taskId) pauseTask(ctx.taskId, label);

  const notice: PauseNotice = {
    kind: pause.kind,
    reason: pause.reason,
    viewUrl: pause.viewUrl,
    taskId: ctx.taskId,
    timeoutMs: ctx.timeoutMs,
  };
  if (ctx.onPause) ctx.onPause(notice);
  else printPauseBanner(notice);

  const outcome = await awaitResume(ctx.taskId, { timeoutMs: ctx.timeoutMs, boundary: pause.boundary });

  if (outcome === 'resumed') {
    // The human was here; do not stop them again for the same page context.
    acknowledge(pause.boundary);
    if (ctx.taskId) setTaskStatus(ctx.taskId, 'running');
    ctx.emit('resume', `human handed control back (${pause.kind})`);
    return {
      outcome,
      note:
        'A human opened the live view and completed that step. ' +
        'Do NOT redo it and do NOT re-enter anything they entered. ' +
        'Call page_read to see where the page is now, then continue the task from there.',
    };
  }

  if (outcome === 'skipped') {
    // Acknowledged for the same reason as a completed step: whatever happens
    // next, stopping on this same boundary again helps nobody.
    acknowledge(pause.boundary);
    if (ctx.taskId) setTaskStatus(ctx.taskId, 'running');
    ctx.emit('resume', `human skipped this step (${pause.kind})`);
    return {
      outcome,
      note:
        'A human DECLINED to do that step, so it will not happen. ' +
        'Do NOT ask for it again and do NOT attempt it yourself. ' +
        'Continue with any part of the task that is still possible without it. ' +
        'If the task cannot be finished without that step, stop and say so plainly.',
    };
  }

  if (outcome === 'cancelled') {
    const note = 'The task was cancelled while it was waiting for a human.';
    ctx.emit('cancelled', note);
    return { outcome, note };
  }

  // Timeout is "nobody came", never "go ahead anyway". The row stays 'paused'
  // on purpose: it is the only record that a human still owes this task an
  // action, and marking it failed would delete that from the queue.
  const note =
    `No human took over within ${humanDuration(ctx.timeoutMs)}, so the step was not done. ` +
    `The task is still paused and waiting: ${label}`;
  ctx.emit('paused_timeout', note);
  return { outcome, note };
}

function printPauseBanner(n: PauseNotice): void {
  const rule = '─'.repeat(64);
  const lines = [
    '',
    `\x1b[33m${rule}\x1b[0m`,
    `\x1b[1m  PAUSED — a human is needed (${n.kind})\x1b[0m`,
    `  ${n.reason}`,
    '',
    `  live view    \x1b[4m${n.viewUrl}\x1b[0m`,
    n.taskId ? `  hand back    mybot resume ${n.taskId}` : '  hand back    resume the run from the dashboard',
    `  waiting up to ${humanDuration(n.timeoutMs)} — the bot is doing nothing until then`,
    `\x1b[33m${rule}\x1b[0m`,
    '',
  ];
  process.stdout.write(`${lines.join('\n')}\n`);
}

function humanDuration(ms: number): string {
  if (ms < 60_000) return `${Math.max(1, Math.round(ms / 1000))}s`;
  return `${Math.round(ms / 60_000)} minutes`;
}

function buildAssistantParts(
  text: string,
  calls: { id: string; name: string; input: Record<string, unknown> }[],
): ContentPart[] {
  const parts: ContentPart[] = [];
  if (text.trim()) parts.push({ type: 'text', text });
  for (const c of calls) parts.push({ type: 'tool_call', id: c.id, name: c.name, input: c.input });
  return parts;
}

function preview(s: string): string {
  const flat = s.replace(/\s+/g, ' ').trim();
  return flat.length > 160 ? `${flat.slice(0, 160)}…` : flat;
}
