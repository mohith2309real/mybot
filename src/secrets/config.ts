import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';
import type { ProviderName } from '../providers/types.ts';

/**
 * Non-secret settings, plaintext at ~/.mybot/config.json.
 *
 * Kept separate from the vault on purpose: the vault exists to hold secrets and
 * costs a passphrase to open. Auth *mode* and a localhost URL are not secrets,
 * and requiring a passphrase to read them would mean prompting on every run.
 */

const DIR = process.env.MYBOT_HOME ?? join(homedir(), '.mybot');
const CONFIG_PATH = join(DIR, 'config.json');

/** Where `npx openai-oauth` listens by default. */
export const OAUTH_DEFAULT_URL = 'http://127.0.0.1:10531/v1';

export type AuthMode = 'api_key' | 'oauth';

export interface ProviderConfig {
  mode: AuthMode;
  /** Only meaningful for oauth / self-hosted gateways. */
  baseURL?: string;
}

export interface Config {
  providers: Partial<Record<ProviderName, ProviderConfig>>;
}

export function configPath(): string {
  return CONFIG_PATH;
}

export function readConfig(): Config {
  if (!existsSync(CONFIG_PATH)) return { providers: {} };
  try {
    const parsed = JSON.parse(readFileSync(CONFIG_PATH, 'utf8')) as Config;
    return { providers: parsed.providers ?? {} };
  } catch {
    return { providers: {} };
  }
}

export function writeConfig(cfg: Config): void {
  mkdirSync(dirname(CONFIG_PATH), { recursive: true, mode: 0o700 });
  writeFileSync(CONFIG_PATH, `${JSON.stringify(cfg, null, 2)}\n`, { mode: 0o600 });
}

export function providerConfig(provider: ProviderName): ProviderConfig {
  return readConfig().providers[provider] ?? { mode: 'api_key' };
}

export function setProviderConfig(provider: ProviderName, pc: ProviderConfig): void {
  const cfg = readConfig();
  cfg.providers[provider] = pc;
  writeConfig(cfg);
}

/** Is the openai-oauth proxy actually up, and what can this account use? */
export async function probeOAuth(baseURL = OAUTH_DEFAULT_URL): Promise<
  { up: true; models: string[] } | { up: false; reason: string }
> {
  try {
    const res = await fetch(`${baseURL.replace(/\/$/, '')}/models`, { signal: AbortSignal.timeout(4000) });
    if (!res.ok) return { up: false, reason: `HTTP ${res.status}` };
    const body = (await res.json()) as { data?: { id: string }[] };
    return { up: true, models: (body.data ?? []).map((m) => m.id) };
  } catch (err) {
    const cause = (err as { cause?: { code?: string } }).cause?.code;
    return { up: false, reason: cause ?? (err as Error).message };
  }
}
