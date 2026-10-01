import OpenAI from 'openai';
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
 * OpenAI adapter, built on the Responses API (not chat.completions).
 *
 * Responses streams *semantic* typed events rather than opaque delta chunks:
 * `response.output_text.delta` for text, `response.output_item.done` carrying a
 * `function_call` item for tool calls. Function-call arguments arrive as a JSON
 * string, so they're parsed here rather than downstream.
 */
export class OpenAIProvider implements ModelProvider {
  readonly name = 'openai' as const;
  readonly defaultModel = 'gpt-5.6';
  readonly models = ['gpt-5.6', 'gpt-5.6-sol', 'gpt-5.6-terra', 'gpt-5.6-luna'];

  #client: OpenAI;

  /** `baseURL` exists for tests and self-hosted gateways/proxies. */
  constructor(apiKey: string, opts: { baseURL?: string } = {}) {
    this.#client = new OpenAI({ apiKey, ...(opts.baseURL ? { baseURL: opts.baseURL } : {}) });
  }

  async listModels(): Promise<string[]> {
    const page = await this.#client.models.list();
    return page.data.map((m) => m.id).sort();
  }

  async *chat(messages: Message[], opts: ChatOptions = {}): AsyncGenerator<StreamChunk> {
    const { system, rest } = splitSystem(messages, opts.system);
    const client = this.#client;

    try {
      const stream = await client.responses.create(
        {
          model: opts.model ?? this.defaultModel,
          ...(system ? { instructions: system } : {}),
          ...(opts.maxTokens ? { max_output_tokens: opts.maxTokens } : {}),
          ...(opts.tools?.length
            ? {
                tools: opts.tools.map((t) => ({
                  type: 'function' as const,
                  name: t.name,
                  description: t.description,
                  parameters: t.parameters,
                  strict: false,
                })),
              }
            : {}),
          input: rest.flatMap(toResponsesItems),
          stream: true,
        } as unknown as Parameters<typeof client.responses.create>[0],
        { signal: opts.signal },
      );

      const parts: ContentPart[] = [];
      let usage = { inputTokens: 0, outputTokens: 0 };
      let stopReason: StopReason = 'end_turn';
      let servedModel = opts.model ?? this.defaultModel;

      for await (const event of stream as AsyncIterable<Record<string, any>>) {
        switch (event.type) {
          case 'response.output_text.delta':
            yield { type: 'text_delta', text: event.delta as string };
            break;

          case 'response.reasoning_summary_text.delta':
            yield { type: 'thinking_delta', text: event.delta as string };
            break;

          case 'response.output_item.done': {
            const item = event.item;
            if (item?.type === 'function_call') {
              const input = safeParse(item.arguments);
              // call_id is what a function_call_output must reference later —
              // item.id is a different identifier and will not round-trip.
              const id = item.call_id ?? item.id;
              parts.push({ type: 'tool_call', id, name: item.name, input });
              stopReason = 'tool_use';
              yield { type: 'tool_call', id, name: item.name, input };
            } else if (item?.type === 'message') {
              const text = (item.content ?? [])
                .filter((c: any) => c.type === 'output_text')
                .map((c: any) => c.text)
                .join('');
              if (text) parts.push({ type: 'text', text });
            }
            break;
          }

          case 'response.completed':
          case 'response.incomplete': {
            const r = event.response ?? {};
            servedModel = r.model ?? servedModel;
            usage = {
              inputTokens: r.usage?.input_tokens ?? 0,
              outputTokens: r.usage?.output_tokens ?? 0,
            };
            if (r.incomplete_details?.reason === 'max_output_tokens') {
              stopReason = 'max_tokens';
            }
            if (r.status === 'incomplete' && r.incomplete_details?.reason === 'content_filter') {
              stopReason = 'refusal';
            }
            break;
          }

          case 'response.failed':
          case 'error':
            yield {
              type: 'error',
              error: event.response?.error?.message ?? event.message ?? 'openai stream failed',
            };
            return;
        }
      }

      yield {
        type: 'done',
        stopReason,
        usage,
        message: { role: 'assistant', content: parts },
        model: servedModel,
      };
    } catch (err) {
      yield { type: 'error', error: describe(err) };
    }
  }
}

/** One neutral Message can expand to several Responses input items. */
function toResponsesItems(m: Message): Record<string, unknown>[] {
  const items: Record<string, unknown>[] = [];
  const textParts: string[] = [];

  for (const p of toParts(m.content)) {
    switch (p.type) {
      case 'text':
        textParts.push(p.text);
        break;
      case 'tool_call':
        items.push({
          type: 'function_call',
          call_id: p.id,
          name: p.name,
          arguments: JSON.stringify(p.input),
        });
        break;
      case 'tool_result':
        items.push({
          type: 'function_call_output',
          call_id: p.toolCallId,
          output: p.content,
        });
        break;
    }
  }

  if (textParts.length) {
    const text = textParts.join('');
    items.unshift(
      m.role === 'assistant'
        ? { role: 'assistant', content: [{ type: 'output_text', text }] }
        : { role: 'user', content: [{ type: 'input_text', text }] },
    );
  }

  return items;
}

function safeParse(raw: unknown): Record<string, unknown> {
  if (typeof raw !== 'string') return (raw as Record<string, unknown>) ?? {};
  try {
    return JSON.parse(raw || '{}');
  } catch {
    return { _unparsed: raw };
  }
}

function describe(err: unknown): string {
  if (err instanceof OpenAI.APIError) return `openai ${err.status}: ${err.message}`;
  return err instanceof Error ? err.message : String(err);
}
