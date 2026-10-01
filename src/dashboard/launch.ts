#!/usr/bin/env node
/**
 * Entry point for the macOS app (and `mybot app`).
 *
 * Starts the console, writes its port where the launcher can find it, and — if
 * asked — opens it to the LAN behind a pairing token so a phone or another
 * machine can connect.
 *
 * The port file exists so a second launch attaches to the running console
 * instead of racing it for a port; clicking the Dock icon twice should not give
 * you two consoles fighting over one container.
 */
import { mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { homedir, networkInterfaces } from 'node:os';
import { join } from 'node:path';
import { start } from './server.ts';

const HOME = process.env.MYBOT_HOME ?? join(homedir(), '.mybot');
const PORT_FILE = join(HOME, 'app.port');

function lanAddress(): string {
  for (const addrs of Object.values(networkInterfaces())) {
    for (const a of addrs ?? []) {
      if (a.family === 'IPv4' && !a.internal) return a.address;
    }
  }
  return '127.0.0.1';
}

export interface LaunchOptions {
  lan?: boolean;
  port?: number;
  passphrase?: string;
}

export async function launch(opts: LaunchOptions = {}) {
  mkdirSync(HOME, { recursive: true, mode: 0o700 });

  const handle = await start({
    port: opts.port,
    lan: opts.lan,
    passphrase: opts.passphrase,
  });

  writeFileSync(PORT_FILE, String(handle.port), { mode: 0o600 });

  const local = `http://127.0.0.1:${handle.port}`;
  console.log(`\n  MyBot console  ${local}`);

  if (handle.token) {
    const url = `http://${lanAddress()}:${handle.port}/?t=${handle.token}`;
    console.log(`\n  Phone / other machines on this wifi:`);
    console.log(`    ${url}`);
    console.log(`\n  That link contains the pairing token. Anyone who has it can run`);
    console.log(`  shell commands in your container and drive its logged-in browser,`);
    console.log(`  so treat it like a password. Restart without --lan to close it off.`);
  } else {
    console.log(`  Loopback only. Add --lan to let your phone connect.`);
  }
  console.log();

  const shutdown = () => {
    rmSync(PORT_FILE, { force: true });
    void handle.close().then(() => process.exit(0));
  };
  process.on('SIGINT', shutdown);
  process.on('SIGTERM', shutdown);

  return handle;
}

// Run directly (the app launcher does exactly this).
if (process.argv[1] && import.meta.url.endsWith(process.argv[1].split('/').pop() ?? '')) {
  const argv = process.argv.slice(2);
  await launch({
    lan: argv.includes('--lan'),
    port: argv.includes('--port') ? Number(argv[argv.indexOf('--port') + 1]) : undefined,
  });
}
