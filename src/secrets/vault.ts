import { createCipheriv, createDecipheriv, randomBytes, scryptSync } from 'node:crypto';
import { chmodSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';
import type { ProviderName } from '../providers/types.ts';

/**
 * Local encrypted API-key store.
 *
 * AES-256-GCM under a scrypt-derived key. The passphrase is never written to
 * disk; it comes from MYBOT_PASSPHRASE or an interactive prompt. Keys never
 * leave this machine and are never logged.
 *
 * Deliberately NOT a secrets manager — this is one developer's laptop. The threat
 * model is "someone reads ~/.mybot/keys.enc", not "someone has root and a debugger".
 */

const DIR = process.env.MYBOT_HOME ?? join(homedir(), '.mybot');
const VAULT_PATH = join(DIR, 'keys.enc');

const SCRYPT_N = 2 ** 15;
const KEY_LEN = 32;
const SALT_LEN = 16;
const IV_LEN = 12;

type KeyMap = Partial<Record<ProviderName, string>>;

export interface VaultFile {
  v: 1;
  salt: string;
  iv: string;
  tag: string;
  data: string;
}

function deriveKey(passphrase: string, salt: Buffer): Buffer {
  return scryptSync(passphrase, salt, KEY_LEN, { N: SCRYPT_N, r: 8, p: 1, maxmem: 64 * 1024 * 1024 });
}

export function vaultExists(): boolean {
  return existsSync(VAULT_PATH);
}

export function vaultPath(): string {
  return VAULT_PATH;
}

export function readVault(passphrase: string): KeyMap {
  if (!vaultExists()) return {};
  try {
    return openSealed<KeyMap>(passphrase, JSON.parse(readFileSync(VAULT_PATH, 'utf8')));
  } catch {
    // GCM auth failure means wrong passphrase or a tampered file. Both are
    // "you can't read this", and distinguishing them leaks nothing useful.
    throw new Error('Could not decrypt vault — wrong passphrase, or the file was modified.');
  }
}

export function writeVault(passphrase: string, keys: KeyMap): void {
  writeSealed(VAULT_PATH, passphrase, keys);
}

// --- sealing, shared with the saved-logins store -----------------------------

/**
 * Encrypt any JSON value under the passphrase. A fresh salt and IV every time,
 * so rewriting the same content never produces the same file.
 */
export function seal(passphrase: string, value: unknown): VaultFile {
  const salt = randomBytes(SALT_LEN);
  const iv = randomBytes(IV_LEN);
  const cipher = createCipheriv('aes-256-gcm', deriveKey(passphrase, salt), iv);
  const data = Buffer.concat([cipher.update(JSON.stringify(value), 'utf8'), cipher.final()]);
  return {
    v: 1,
    salt: salt.toString('base64'),
    iv: iv.toString('base64'),
    tag: cipher.getAuthTag().toString('base64'),
    data: data.toString('base64'),
  };
}

/** Throws on a wrong passphrase or a tampered file; callers word the error. */
export function openSealed<T>(passphrase: string, file: VaultFile): T {
  const salt = Buffer.from(file.salt, 'base64');
  const decipher = createDecipheriv('aes-256-gcm', deriveKey(passphrase, salt), Buffer.from(file.iv, 'base64'));
  decipher.setAuthTag(Buffer.from(file.tag, 'base64'));
  const plain = Buffer.concat([decipher.update(Buffer.from(file.data, 'base64')), decipher.final()]);
  return JSON.parse(plain.toString('utf8')) as T;
}

/** Seal and write with mode 0600, creating ~/.mybot at 0700 if needed. */
export function writeSealed(path: string, passphrase: string, value: unknown): void {
  mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  writeFileSync(path, JSON.stringify(seal(passphrase, value)), { mode: 0o600 });
  chmodSync(path, 0o600);
}

export function setKey(passphrase: string, provider: ProviderName, apiKey: string): void {
  const keys = readVault(passphrase);
  keys[provider] = apiKey;
  writeVault(passphrase, keys);
}

export function removeKey(passphrase: string, provider: ProviderName): void {
  const keys = readVault(passphrase);
  delete keys[provider];
  writeVault(passphrase, keys);
}

/**
 * Resolve a provider key. Environment variables win over the vault, so CI and
 * one-off shells work without a passphrase at all.
 */
export function resolveKey(provider: ProviderName, passphrase?: string): string | undefined {
  const fromEnv = {
    anthropic: process.env.ANTHROPIC_API_KEY,
    openai: process.env.OPENAI_API_KEY,
    gemini: process.env.GEMINI_API_KEY ?? process.env.GOOGLE_API_KEY,
  }[provider];
  if (fromEnv) return fromEnv;

  if (!passphrase || !vaultExists()) return undefined;
  return readVault(passphrase)[provider];
}
