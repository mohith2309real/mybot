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

interface VaultFile {
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

  const file: VaultFile = JSON.parse(readFileSync(VAULT_PATH, 'utf8'));
  const salt = Buffer.from(file.salt, 'base64');
  const decipher = createDecipheriv('aes-256-gcm', deriveKey(passphrase, salt), Buffer.from(file.iv, 'base64'));
  decipher.setAuthTag(Buffer.from(file.tag, 'base64'));

  try {
    const plain = Buffer.concat([decipher.update(Buffer.from(file.data, 'base64')), decipher.final()]);
    return JSON.parse(plain.toString('utf8'));
  } catch {
    // GCM auth failure means wrong passphrase or a tampered file. Both are
    // "you can't read this", and distinguishing them leaks nothing useful.
    throw new Error('Could not decrypt vault — wrong passphrase, or the file was modified.');
  }
}

export function writeVault(passphrase: string, keys: KeyMap): void {
  mkdirSync(dirname(VAULT_PATH), { recursive: true, mode: 0o700 });

  const salt = randomBytes(SALT_LEN);
  const iv = randomBytes(IV_LEN);
  const cipher = createCipheriv('aes-256-gcm', deriveKey(passphrase, salt), iv);
  const data = Buffer.concat([cipher.update(JSON.stringify(keys), 'utf8'), cipher.final()]);

  const file: VaultFile = {
    v: 1,
    salt: salt.toString('base64'),
    iv: iv.toString('base64'),
    tag: cipher.getAuthTag().toString('base64'),
    data: data.toString('base64'),
  };

  writeFileSync(VAULT_PATH, JSON.stringify(file), { mode: 0o600 });
  chmodSync(VAULT_PATH, 0o600);
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
