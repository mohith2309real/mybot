/**
 * The console's saved-login endpoints, and the cross-site guard in front of
 * every console request. Real server on a free loopback port, throwaway home.
 */
import { mkdtempSync } from 'node:fs';
import { request } from 'node:http';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import assert from 'node:assert';

const HOME = mkdtempSync(join(tmpdir(), 'mybot-console-'));
process.env.MYBOT_HOME = HOME;
process.env.MYBOT_DB = join(HOME, 'mybot.db');
delete process.env.MYBOT_PASSPHRASE;

const SECRET = 'console-pw-Xy7#unique';

const server = await import('../src/dashboard/server.ts');
const logins = await import('../src/secrets/logins.ts');

function call(port, method, path, { body, headers = {} } = {}) {
  return new Promise((resolve, reject) => {
    const payload = body === undefined ? undefined : JSON.stringify(body);
    const req = request(
      {
        host: '127.0.0.1',
        port,
        method,
        path,
        headers: {
          host: `127.0.0.1:${port}`,
          ...(payload ? { 'content-type': 'application/json', 'content-length': Buffer.byteLength(payload) } : {}),
          ...headers,
        },
      },
      (res) => {
        let text = '';
        res.on('data', (c) => (text += c));
        res.on('end', () => resolve({ status: res.statusCode, text, json: text ? JSON.parse(text) : null }));
      },
    );
    req.on('error', reject);
    if (payload) req.write(payload);
    req.end();
  });
}

// --- locked: no passphrase given to the console ------------------------------

let handle = await server.start({ port: 47000 + Math.floor(Math.random() * 1000) });
let port = handle.port;

let r = await call(port, 'GET', '/api/logins');
assert.equal(r.status, 200);
assert.equal(r.json.locked, true);
r = await call(port, 'POST', '/api/logins', { body: { url: 'https://github.com', username: 'a', password: SECRET } });
assert.equal(r.status, 409);
console.log('locked console: lists nothing, adds nothing: OK');
await handle.close();
logins.lock();

// --- unlocked ----------------------------------------------------------------

handle = await server.start({ port: port + 1, passphrase: 'console-pass' });
port = handle.port;

// Cross-site: a page on another origin cannot change anything...
r = await call(port, 'POST', '/api/logins', {
  body: { url: 'https://github.com', username: 'attacker', password: 'x' },
  headers: { origin: 'https://evil.example' },
});
assert.equal(r.status, 403);
r = await call(port, 'POST', '/api/tasks', {
  body: { botId: 'x', instruction: 'do something' },
  headers: { origin: 'https://evil.example', 'content-type': 'text/plain' },
});
assert.equal(r.status, 403);
// ...and a DNS-rebinding page (our port, its own name) cannot even read.
r = await call(port, 'GET', '/api/logins', { headers: { host: `evil.example:${port}` } });
assert.equal(r.status, 403);
// The console's own pages, and local tools with no Origin, are fine.
r = await call(port, 'GET', '/api/logins', { headers: { origin: `http://127.0.0.1:${port}` } });
assert.equal(r.status, 200);
r = await call(port, 'GET', '/api/logins', { headers: { host: `localhost:${port}` } });
assert.equal(r.status, 200);
console.log('cross-site writes and DNS rebinding refused; own pages allowed: OK');

r = await call(port, 'POST', '/api/logins', {
  body: { url: 'https://github.com/login', username: 'alice@example.com', password: SECRET, label: 'Work' },
  headers: { origin: `http://127.0.0.1:${port}` },
});
assert.equal(r.status, 201, r.text);
assert.ok(!r.text.includes(SECRET));
const id = r.json.id;

r = await call(port, 'GET', '/api/logins');
assert.equal(r.json.locked, false);
assert.equal(r.json.logins.length, 1);
assert.equal(r.json.logins[0].origin, 'https://github.com');
assert.ok(!r.text.includes(SECRET), 'password in a console response!');
console.log('add + list through the console, never returning a password: OK');

r = await call(port, 'POST', '/api/logins', { body: { url: 'http://github.com', username: 'a', password: 'b' } });
assert.equal(r.status, 400);
assert.match(r.json.error, /https/);
console.log('http site refused at save time: OK');

// Approvals: a request shows up, and allowing it from the console releases it.
const login = logins.listLogins().find((l) => l.id === id);
const waiting = logins.requestApproval({ bot: 'scout', login }, { waitMs: 5000 });
await new Promise((res) => setTimeout(res, 50));
r = await call(port, 'GET', '/api/login-requests');
assert.equal(r.json.pending.length, 1);
const reqId = r.json.pending[0].id;
r = await call(port, 'POST', `/api/login-requests/${reqId}/allow`, { body: { always: true } });
assert.equal(r.status, 200, r.text);
assert.equal((await waiting).allowed, true);
r = await call(port, 'GET', '/api/logins');
assert.equal(r.json.logins[0].alwaysAllow, true);
r = await call(port, 'POST', `/api/login-requests/${reqId}/deny`, { body: {} });
assert.equal(r.status, 409);
console.log('approve from the console (always) releases the waiting fill: OK');

r = await call(port, 'POST', `/api/logins/${id}/always`, { body: { always: false } });
assert.equal(r.status, 200);
r = await call(port, 'DELETE', `/api/logins/${id}`);
assert.equal(r.status, 200);
r = await call(port, 'GET', '/api/logins');
assert.equal(r.json.logins.length, 0);
console.log('toggle and remove: OK');

await handle.close();
console.log('\nALL CONSOLE LOGINS TESTS PASSED');
