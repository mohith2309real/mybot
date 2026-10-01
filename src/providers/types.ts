/**
 * Neutral model-layer types.
 *
 * Every provider adapter converts these to/from its own wire format, so the rest
 * of MyBot (the agent loop, the container harness, skills, routines) never sees
 * a provider-specific shape.
 */

export type ProviderName = 'anthropic' | 'openai' | 'gemini';

export type Role = 'system' | 'user' | 'assistant';

export interface TextPart {
  type: 'text';
  text: string;
}

/** A tool the model wants us to run. Emitted by the model, never by us. */
export interface ToolCallPart {
  type: 'tool_call';
  id: string;
  name: string;
  input: Record<string, unknown>;
}

/** The result of running a tool. Sent back to the model on the next turn. */
export interface ToolResultPart {
  type: 'tool_result';
  toolCallId: string;
  content: string;
  isError?: boolean;
}

export type ContentPart = TextPart | ToolCallPart | ToolResultPart;

export interface Message {
  role: Role;
  /** A bare string is shorthand for a single TextPart. */
  content: string | ContentPart[];
}

export interface ToolDefinition {
  name: string;
  description: string;
  /** JSON Schema object. Must be `{type:"object", properties:{...}}`. */
  parameters: Record<string, unknown>;
}

export type StopReason =
  | 'end_turn'
  | 'tool_use'
  | 'max_tokens'
  | 'refusal'
  | 'error';

export interface Usage {
  inputTokens: number;
  outputTokens: number;
}

export type StreamChunk =
  /** Incremental user-visible text. */
  | { type: 'text_delta'; text: string }
  /** Incremental reasoning summary, where the provider exposes one. */
  | { type: 'thinking_delta'; text: string }
  /** A complete tool call. Emitted once the arguments have fully arrived. */
  | { type: 'tool_call'; id: string; name: string; input: Record<string, unknown> }
  /** Terminal success. `message` is the assistant turn to append to history. */
  | {
      type: 'done';
      stopReason: StopReason;
      usage: Usage;
      message: Message;
      /** Which model actually served the response (may differ under fallback). */
      model: string;
    }
  /** Terminal failure. The generator stops after this. */
  | { type: 'error'; error: string };

export interface ChatOptions {
  model?: string;
  system?: string;
  tools?: ToolDefinition[];
  maxTokens?: number;
  signal?: AbortSignal;
}

export interface ModelProvider {
  readonly name: ProviderName;
  /** Known-good model ids. Use `listModels()` for the live set. */
  readonly models: string[];
  readonly defaultModel: string;

  chat(messages: Message[], opts?: ChatOptions): AsyncGenerator<StreamChunk>;

  /**
   * Live model list straight from the provider. Model ids churn faster than any
   * hardcoded table, so prefer this over `models` when showing choices to a user.
   */
  listModels(): Promise<string[]>;
}

// ---------------------------------------------------------------------------
// Shared helpers for provider adapters
// ---------------------------------------------------------------------------

export function toParts(content: Message['content']): ContentPart[] {
  return typeof content === 'string' ? [{ type: 'text', text: content }] : content;
}

/** Concatenate all leading `system` messages; providers take system out-of-band. */
export function splitSystem(
  messages: Message[],
  explicitSystem?: string,
): { system: string | undefined; rest: Message[] } {
  const systems: string[] = [];
  const rest: Message[] = [];
  for (const m of messages) {
    if (m.role === 'system') {
      for (const p of toParts(m.content)) if (p.type === 'text') systems.push(p.text);
    } else {
      rest.push(m);
    }
  }
  if (explicitSystem) systems.unshift(explicitSystem);
  return { system: systems.length ? systems.join('\n\n') : undefined, rest };
}

/**
 * Gemini keys tool results by function *name*, not by call id, so adapters need
 * to resolve an id back to the name that produced it.
 */
export function toolNameForCallId(messages: Message[], callId: string): string {
  for (const m of messages) {
    for (const p of toParts(m.content)) {
      if (p.type === 'tool_call' && p.id === callId) return p.name;
    }
  }
  return 'unknown_tool';
}

export function textOf(message: Message): string {
  return toParts(message.content)
    .filter((p): p is TextPart => p.type === 'text')
    .map((p) => p.text)
    .join('');
}
