/** Vault + SQLite tests. Uses a throwaway MYBOT_HOME; touches nothing real. */
import { mkdtempSync, readFileSync, statSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import assert from 'node:assert';

const HOME = mkdtempSync(join(tmpdir(), 'mybot-test-'));
process.env.MYBOT_HOME = HOME;

const { writeVault, readVault, setKey, resolveKey, vaultExists } = await import('../src/secrets/vault.ts');
const db = await import('../src/db/index.ts');

// --- vault -----------------------------------------------------------------
writeVault('pw1', {});
assert.ok(vaultExists());
setKey('pw1', 'anthropic', 'sk-ant-FAKE-abc123');
setKey('pw1', 'openai', 'sk-FAKE-openai');
assert.equal(readVault('pw1').anthropic, 'sk-ant-FAKE-abc123');
assert.equal(readVault('pw1').openai, 'sk-FAKE-openai');
console.log('vault roundtrip: OK');

assert.throws(() => readVault('wrong'), /Could not decrypt/);
console.log('wrong passphrase fails closed: OK');

assert.ok(!readFileSync(join(HOME, 'keys.enc'), 'utf8').includes('sk-ant-FAKE'), 'plaintext leaked!');
assert.equal(statSync(join(HOME, 'keys.enc')).mode & 0o777, 0o600);
console.log('ciphertext-only on disk, mode 0600: OK');

process.env.ANTHROPIC_API_KEY = 'sk-ant-FROM-ENV';
assert.equal(resolveKey('anthropic', 'pw1'), 'sk-ant-FROM-ENV');
delete process.env.ANTHROPIC_API_KEY;
assert.equal(resolveKey('anthropic', 'pw1'), 'sk-ant-FAKE-abc123');
console.log('env beats vault: OK');

// --- db --------------------------------------------------------------------
const bot = db.createBot({ name: 'researcher', provider: 'anthropic', model: 'claude-opus-5' });
assert.equal(db.listBots().length, 1);

const t = db.createTask(bot.id, 'Find the top 3 competitors');
db.setTaskStatus(t.id, 'running');
db.appendTaskLog(t.id, 'user', 'Find the top 3 competitors');
db.appendTaskLog(t.id, 'assistant', 'Working on it');
db.setTaskStatus(t.id, 'paused'); // the human-in-the-loop boundary state
db.setTaskStatus(t.id, 'done');
assert.equal(db.readTaskLog(t.id).length, 2);
const [row] = db.listTasks(bot.id);
assert.equal(row.status, 'done');
assert.ok(row.started_at && row.completed_at);
console.log('task lifecycle + log: OK');

db.saveWorkspaceSession(Buffer.from('cookie-jar-v1'));
db.saveWorkspaceSession(Buffer.from('cookie-jar-v2'));
const loaded = db.loadWorkspaceSession();
assert.ok(Buffer.isBuffer(loaded), 'node:sqlite returns Uint8Array — must convert');
assert.equal(loaded.toString(), 'cookie-jar-v2');
assert.equal(db.openDb().prepare('SELECT COUNT(*) c FROM workspace_sessions').get().c, 1);
console.log('shared workspace session (single row, real Buffer): OK');

const h = db.openDb();
h.prepare("INSERT INTO skills (id,name,instructions,source) VALUES ('s1','Weekly','x','manual')").run();
h.prepare("INSERT INTO routines (id,skill_id,schedule) VALUES ('r1','s1','0 9 * * 1')").run();
assert.throws(
  () => h.prepare("INSERT INTO routines (id,skill_id,schedule,trigger) VALUES ('r2','s1','0 9 * * 1','hook')").run(),
  /CHECK|constraint/i,
);
assert.throws(
  () => h.prepare("INSERT INTO skills (id,name,instructions,source) VALUES ('s2','R','x','remote')").run(),
  /CHECK|constraint/i,
  'no remote skill source by design — see ClawHub',
);
console.log('schema constraints (routine XOR, no remote skills): OK');

rmSync(HOME, { recursive: true, force: true });
console.log('\nALL LOCAL TESTS PASSED');
