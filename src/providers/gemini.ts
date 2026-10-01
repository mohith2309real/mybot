import { GoogleGenAI } from '@google/genai';
import {
  type ChatOptions,
  type ContentPart,
  type Message,
  type ModelProvider,
  type StopReason,
  type StreamChunk,
  splitSystem,
  toParts,
  toolNameForCallId,
} from './types.ts';

/**
 * Google Gemini adapter, on the unified @google/genai SDK
 * (google-generativeai is retired — do not reintroduce it).
 *
 * Gemini quirks handled here:
 *  - roles are `user` / `model`, not user/assistant.
 *  - a functionResponse is keyed by function *name*, not by call id, so the
 *    original call has to be looked up to recover the name.
 *  - functionCall ids are frequently absent; a synthetic one is minted so the
 *    rest of MyBot can key tool results consistently.
 */
export class GeminiProvider implements ModelProvider {
  readonly name = 'gemini' as const;
  readonly defaultModel = 'gemini-3.6-flash';
  readonly models = ['gemini-3.6-flash', 'gemini-3-flash-preview', 'gemini-2.5-flash'];

  #client: GoogleGenAI;

  /** `baseURL` exists for tests and self-hosted gateways/proxies. */
  constructor(apiKey: string, opts: { baseURL?: string } = {}) {
    this.#client = new GoogleGenAI({
      apiKey,
      ...(opts.baseURL ? { httpOptions: { baseUrl: opts.baseURL } } : {}),
    });
  }

  async listModels(): Promise<string[]> {
    const out: string[] = [];
    const pager = await this.#client.models.list();
    for await (const m of pager) {
      if (m.name) out.push(m.name.replace(/^models\//, ''));
    }
    return out.sort();
  }

  async *chat(messages: Message[], opts: ChatOptions = {}): AsyncGenerator<StreamChunk> {
    const { system, rest } = splitSystem(messages, opts.system);

    try {
      const stream = await this.#client.models.generateContentStream({
        model: opts.model ?? this.defaultModel,
        contents: rest.map((m) => toGeminiContent(m, messages)),
        config: {
          ...(system ? { systemInstruction: system } : {}),
          ...(opts.maxTokens ? { maxOutputTokens: opts.maxTokens } : {}),
          ...(opts.tools?.length
            ? {
                tools: [
                  {
                    functionDeclarations: opts.tools.map((t) => ({
                      name: t.name,
                      description: t.description,
                      parametersJsonSchema: t.parameters,
                    })),
                  },
                ],
              }
            : {}),
          ...(opts.signal ? { abortSignal: opts.signal } : {}),
        },
      } as any);

      const parts: ContentPart[] = [];
      let usage = { inputTokens: 0, outputTokens: 0 };
      let stopReason: StopReason = 'end_turn';
      let servedModel = opts.model ?? this.defaultModel;
      let callSeq = 0;

      for await (const chunk of stream) {
        // Read text out of the parts array rather than the `.text` accessor:
        // that accessor logs a warning to stderr whenever a chunk also carries
        // a functionCall, which is every tool-calling turn.
        const text = (chunk.candidates?.[0]?.content?.parts ?? [])
          .map((p) => p.text)
          .filter((t): t is string => typeof t === 'string' && t.length > 0)
          .join('');
        if (text) {
          parts.push({ type: 'text', text });
          yield { type: 'text_delta', text };
        }

        for (const fc of chunk.functionCalls ?? []) {
          const id = fc.id ?? `gemini_call_${++callSeq}`;
          const input = (fc.args ?? {}) as Record<string, unknown>;
          const name = fc.name ?? 'unknown_tool';
          parts.push({ type: 'tool_call', id, name, input });
          stopReason = 'tool_use';
          yield { type: 'tool_call', id, name, input };
        }

        if (chunk.usageMetadata) {
          usage = {
            inputTokens: chunk.usageMetadata.promptTokenCount ?? 0,
            outputTokens: chunk.usageMetadata.candidatesTokenCount ?? 0,
          };
        }
        if (chunk.modelVersion) servedModel = chunk.modelVersion;

        const finish = chunk.candidates?.[0]?.finishReason;
        if (finish === 'MAX_TOKENS') stopReason = 'max_tokens';
        else if (finish === 'SAFETY' || finish === 'PROHIBITED_CONTENT') stopReason = 'refusal';
      }

      yield {
        type: 'done',
        stopReason,
        usage,
        message: { role: 'assistant', content: coalesceText(parts) },
        model: servedModel,
      };
    } catch (err) {
      yield { type: 'error', error: describe(err) };
    }
  }
}

function toGeminiContent(m: Message, all: Message[]): Record<string, unknown> {
  const parts: Record<string, unknown>[] = [];

  for (const p of toParts(m.content)) {
    switch (p.type) {
      case 'text':
        parts.push({ text: p.text });
        break;
      case 'tool_call':
        parts.push({ functionCall: { name: p.name, args: p.input } });
        break;
      case 'tool_result':
        parts.push({
          functionResponse: {
            name: toolNameForCallId(all, p.toolCallId),
            response: p.isError ? { error: p.content } : { result: p.content },
          },
        });
        break;
    }
  }

  return { role: m.role === 'assistant' ? 'model' : 'user', parts };
}

/** Streaming produces one text part per chunk; merge them for clean history. */
function coalesceText(parts: ContentPart[]): ContentPart[] {
  const out: ContentPart[] = [];
  for (const p of parts) {
    const prev = out[out.length - 1];
    if (p.type === 'text' && prev?.type === 'text') prev.text += p.text;
    else out.push(p);
  }
  return out;
}

function describe(err: unknown): string {
  return err instanceof Error ? `gemini: ${err.message}` : String(err);
}
