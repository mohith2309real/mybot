import { OAUTH_DEFAULT_URL, providerConfig } from '../secrets/config.ts';
import { resolveKey } from '../secrets/vault.ts';
import { AnthropicProvider } from './anthropic.ts';
import { GeminiProvider } from './gemini.ts';
import { OpenAIProvider } from './openai.ts';
import type { ModelProvider, ProviderName } from './types.ts';

export * from './types.ts';
export { AnthropicProvider, GeminiProvider, OpenAIProvider };

export const PROVIDER_NAMES: ProviderName[] = ['anthropic', 'openai', 'gemini'];

export function isProviderName(v: string): v is ProviderName {
  return (PROVIDER_NAMES as string[]).includes(v);
}

export class MissingKeyError extends Error {
  // Node runs .ts by stripping types only — no parameter properties, no enums.
  readonly provider: ProviderName;

  constructor(provider: ProviderName) {
    super(
      `No API key for "${provider}". Run:  mybot keys set ${provider}\n` +
        `(or export ${envVarFor(provider)} for this shell)`,
    );
    this.name = 'MissingKeyError';
    this.provider = provider;
  }
}

export function envVarFor(provider: ProviderName): string {
  return { anthropic: 'ANTHROPIC_API_KEY', openai: 'OPENAI_API_KEY', gemini: 'GEMINI_API_KEY' }[
    provider
  ];
}

/** Build a provider from an explicit key — no vault, no environment. */
export function providerWithKey(
  name: ProviderName,
  apiKey: string,
  opts: { baseURL?: string } = {},
): ModelProvider {
  switch (name) {
    case 'anthropic':
      return new AnthropicProvider(apiKey, opts);
    case 'openai':
      return new OpenAIProvider(apiKey, opts);
    case 'gemini':
      return new GeminiProvider(apiKey, opts);
  }
}

/**
 * Build a provider using the configured auth mode.
 *
 * `oauth` mode points the OpenAI adapter at a local openai-oauth proxy, which
 * speaks the same Responses API (verified: /v1/responses and /v1/models both
 * work there, tool calling included) but authenticates with a ChatGPT account
 * instead of an API key. No key is needed, so none is resolved.
 */
export function getProvider(name: ProviderName, passphrase?: string): ModelProvider {
  const pc = providerConfig(name);

  if (pc.mode === 'oauth') {
    const baseURL = pc.baseURL ?? OAUTH_DEFAULT_URL;
    // The proxy ignores the key entirely; the SDK just requires a non-empty one.
    return providerWithKey(name, 'openai-oauth-local', { baseURL });
  }

  const key = resolveKey(name, passphrase);
  if (!key) throw new MissingKeyError(name);
  return providerWithKey(name, key);
}

/** True when this provider needs the vault (and therefore a passphrase). */
export function needsKey(name: ProviderName): boolean {
  return providerConfig(name).mode !== 'oauth';
}
