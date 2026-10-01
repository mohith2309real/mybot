import Anthropic from '@anthropic-ai/sdk';
import {
  type ChatOptions,
  type ContentPart,
  type Message,
  type ModelProvider,
  type StopReason,
  type StreamChunk,
  splitSystem,
  toParts,
} from './types.ts';

/**
 * Anthropic adapter.
 *
 * Notes that bit during implementation, so they don't get "fixed" back later:
 *  - `budget_tokens` is a 400 on Opus 5. Thinking is adaptive-only, and it is ON
 *    by default, so `max_tokens` caps thinking + text together.
 *  - `display: "summarized"` is opt-in; the default emits empty thinking blocks,
 *    which would make the Agent Computer log look like a long stall.
 *  - `temperature` / `top_p` / `top_k` are all 400s. Steer with the prompt.
 *  - A safety refusal is HTTP 200 with `stop_reason: "refusal"`, not an exception.
 *    We opt into server-side fallbacks so a decline gets re-served rather than
 *    dead-ending an autonomous run.
 */
export class AnthropicProvider implements ModelProvider {
  readonly name = 'anthropic' as const;
  readonly defaultModel = 'claude-opus-5';
  readonly models = [
    'claude-opus-5',
    'claude-sonnet-5',
    'claude-opus-4-8',
    'claude-haiku-4-5',
  ];

  #client: Anthropic;

  /** `baseURL` exists for tests and self-hosted gateways/proxies. */
  constructor(apiKey: string, opts: { baseURL?: string } = {}) {
    this.#client = new Anthropic({ apiKey, ...(opts.baseURL ? { baseURL: opts.baseURL } : {}) });
  }

  async listModels(): Promise<string[]> {
    const out: string[] = [];
    for await (const m of this.#client.models.list()) out.push(m.id);
    return out;
  }

  async *chat(messages: Message[], opts: ChatOptions = {}): AsyncGenerator<StreamChunk> {
    const { system, rest } = splitSystem(messages, opts.system);
    const client = this.#client;

    const stream = client.beta.messages.stream({
      model: opts.model ?? this.defaultModel,
      max_tokens: opts.maxTokens ?? 16000,
      thinking: { type: 'adaptive', display: 'summarized' },
      ...(system ? { system } : {}),
      ...(opts.tools?.length
        ? {
            tools: opts.tools.map((t) => ({
              name: t.name,
              description: t.description,
              input_schema: t.parameters as Anthropic.Tool.InputSchema,
            })),
          }
        : {}),
      messages: rest.map(toAnthropicMessage),
      // A policy decline mid-run would otherwise strand an autonomous bot.
      // "default" routes by refusal category, so there's no fallback model list
      // to maintain.
      betas: ['server-side-fallback-2026-07-01'],
      fallbacks: 'default',
    } as Parameters<typeof client.beta.messages.stream>[0], {
      signal: opts.signal,
    });

    try {
      for await (const event of stream) {
        if (event.type === 'content_block_delta') {
          if (event.delta.type === 'text_delta') {
            yield { type: 'text_delta', text: event.delta.text };
          } else if (event.delta.type === 'thinking_delta') {
            yield { type: 'thinking_delta', text: event.delta.thinking };
          }
        }
      }

      const final = await stream.finalMessage();
      const parts: ContentPart[] = [];

      for (const block of final.content) {
        if (block.type === 'text') {
          parts.push({ type: 'text', text: block.text });
        } else if (block.type === 'tool_use') {
          const input = (block.input ?? {}) as Record<string, unknown>;
          parts.push({ type: 'tool_call', id: block.id, name: block.name, input });
          yield { type: 'tool_call', id: block.id, name: block.name, input };
        }
      }

      yield {
        type: 'done',
        stopReason: mapStop(final.stop_reason),
        usage: {
          inputTokens: final.usage.input_tokens ?? 0,
          outputTokens: final.usage.output_tokens ?? 0,
        },
        message: { role: 'assistant', content: parts },
        model: final.model,
      };
    } catch (err) {
      yield { type: 'error', error: describe(err) };
    }
  }
}

function toAnthropicMessage(m: Message): Anthropic.MessageParam {
  const parts = toParts(m.content);

  // Tool results must be their own user turn in the Anthropic wire format.
  const blocks = parts.map((p): Anthropic.ContentBlockParam => {
    switch (p.type) {
      case 'text':
        return { type: 'text', text: p.text };
      case 'tool_call':
        return { type: 'tool_use', id: p.id, name: p.name, input: p.input };
      case 'tool_result':
        return {
          type: 'tool_result',
          tool_use_id: p.toolCallId,
          content: p.content,
          ...(p.isError ? { is_error: true } : {}),
        };
    }
  });

  return { role: m.role === 'assistant' ? 'assistant' : 'user', content: blocks };
}

function mapStop(reason: string | null): StopReason {
  switch (reason) {
    case 'tool_use':
      return 'tool_use';
    case 'max_tokens':
      return 'max_tokens';
    case 'refusal':
      return 'refusal';
    default:
      return 'end_turn';
  }
}

function describe(err: unknown): string {
  if (err instanceof Anthropic.APIError) return `anthropic ${err.status}: ${err.message}`;
  return err instanceof Error ? err.message : String(err);
}
