/**
 * Saved logins: the store, the origin rule, the approval flow, and — when a
 * Chromium is available — a real fill into a real page.
 *
 * Throwaway MYBOT_HOME; touches nothing real. The password used throughout is
 * distinctive so a leak anywhere (file, row, tool text, snapshot) is greppable.
 */
import { mkdtempSync, readFileSync, statSync, existsSync } from 'node:fs';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import assert from 'node:assert';

const HOME = mkdtempSync(join(tmpdir(), 'mybot-logins-'));
process.env.MYBOT_HOME = HOME;
process.env.MYBOT_DB = join(HOME, 'mybot.db');
delete process.env.MYBOT_PASSPHRASE;

const logins = await import('../src/secrets/logins.ts');
const { fillLogin, clearGrants, elementKind } = await import('../src/computer/loginfill.ts');
const db = await import('../src/db/index.ts');

const SECRET = 'hunter2-Zq9!unique-pw';
const PASS = 'vault-pass-1';

// --- origins ----------------------------------------------------------------

assert.equal(logins.normalizeOrigin('github.com'), 'https://github.com');
assert.equal(logins.normalizeOrigin('https://GitHub.com/login?return=/x'), 'https://github.com');
assert.equal(logins.normalizeOrigin('https://github.com:443/'), 'https://github.com');
assert.equal(logins.normalizeOrigin('https://app.example.com:8443/a'), 'https://app.example.com:8443');
assert.equal(logins.normalizeOrigin('http://localhost:3000/login'), 'http://localhost:3000');
assert.throws(() => logins.normalizeOrigin('http://github.com'), /only work on https/);
assert.throws(() => logins.normalizeOrigin('ftp://github.com'), /only work on https/);
assert.throws(() => logins.normalizeOrigin('javascript:alert(1)'), /only work on https|not a web address/);
assert.throws(() => logins.normalizeOrigin(''), /sign-in page/);
assert.equal(logins.pageOrigin('about:blank'), undefined);
console.log('origins: https only, exact, normalised: OK');

// --- store ------------------------------------------------------------------

assert.throws(() => logins.listLogins(), /locked/);
logins.unlock(PASS);

const gh = logins.addLogin({ url: 'https://github.com/login', username: 'alice@example.com', password: SECRET });
assert.equal(gh.origin, 'https://github.com');
assert.ok(!('password' in gh), 'summary must not carry the password');
logins.addLogin({ url: 'https://bank.example', username: 'alice', password: 'bank-pw', label: 'Bank' });

const listed = logins.listLogins();
assert.equal(listed.length, 2);
for (const l of listed) assert.ok(!('password' in l), 'list must not carry passwords');
assert.ok(!JSON.stringify(listed).includes(SECRET));

const raw = readFileSync(logins.loginsPath(), 'utf8');
assert.ok(!raw.includes(SECRET), 'plaintext password on disk!');
assert.ok(!raw.includes('alice@example.com'), 'plaintext username on disk');
assert.ok(!raw.includes('github.com'), 'plaintext site on disk');
assert.equal(statSync(logins.loginsPath()).mode & 0o777, 0o600);
console.log('store: encrypted at rest (site, user, password), mode 0600, summaries only: OK');

assert.throws(() => logins.listLogins('wrong-pass'), /Could not unlock/);
console.log('wrong passphrase fails closed: OK');

// Re-adding the same account replaces the password, keeps the id.
const again = logins.addLogin({ url: 'github.com', username: 'alice@example.com', password: SECRET });
assert.equal(again.id, gh.id);
assert.equal(logins.listLogins().length, 2);
assert.equal(logins.revealForFill(gh.id, 'https://github.com').password, SECRET);
assert.throws(() => logins.revealForFill(gh.id, 'https://github.com.evil.example'), /does not belong/);
console.log('re-add replaces; reveal is bound to the origin: OK');

// --- fill_login, with a fake browser -----------------------------------------

function fakePage(url, elements) {
  return { url, title: 'Sign in', text: '', elements };
}
const LOGIN_FORM = '  [1] input:email "Email"\n  [2] input:password "Password"\n  [3] button "Sign in"';

function harness({ url = 'https://github.com/login', elements = LOGIN_FORM, answer = 'allow' } = {}) {
  const calls = { fill: [], approve: 0 };
  const deps = {
    snapshot: async () => fakePage(url, elements),
    fill: async (opts) => {
      calls.fill.push(opts);
      return { submitted: Boolean(opts.submit) };
    },
    approve: async (input, opts) => {
      calls.approve++;
      if (answer === 'real') return logins.requestApproval(input, opts);
      return answer === 'allow'
        ? { allowed: true, by: 'human', requestId: 'x' }
        : { allowed: false, by: answer === 'timeout' ? 'timeout' : 'human', requestId: 'x' };
    },
  };
  return { calls, deps };
}

assert.equal(elementKind(fakePage('', LOGIN_FORM), 2), 'input:password');
assert.equal(elementKind(fakePage('', '  [4] input:text "input:password"'), 4), 'input:text');

{
  // Approved: fills the right account, tool text never has the password.
  clearGrants();
  const { calls, deps } = harness();
  const r = await fillLogin({ passwordRef: 2, usernameRef: 1, submit: true }, { bot: 'scout', taskId: 't1' }, deps);
  assert.equal(r.kind, 'filled');
  assert.equal(calls.fill.length, 1);
  assert.equal(calls.fill[0].password, SECRET);
  assert.equal(calls.fill[0].username, 'alice@example.com');
  assert.equal(calls.fill[0].origin, 'https://github.com');
  assert.ok(!r.text.includes(SECRET));
  console.log('approved fill: right site, right account, no password in tool text: OK');
}

{
  // A lookalike site gets nothing — not even a prompt.
  clearGrants();
  const { calls, deps } = harness({ url: 'https://github.com.login-check.example/login' });
  const r = await fillLogin({ passwordRef: 2, usernameRef: 1 }, { bot: 'scout' }, deps);
  assert.equal(r.kind, 'handoff');
  assert.equal(calls.approve, 0);
  assert.equal(calls.fill.length, 0);
  const sub = harness({ url: 'https://gist.github.com/login' });
  assert.equal((await fillLogin({ passwordRef: 2 }, {}, sub.deps)).kind, 'handoff');
  assert.equal(sub.calls.fill.length, 0);
  console.log('lookalike and subdomain pages: no prompt, no fill, hand to human: OK');
}

{
  // Plain http never fills.
  const { calls, deps } = harness({ url: 'http://github.com/login' });
  const r = await fillLogin({ passwordRef: 2 }, {}, deps);
  assert.equal(r.kind, 'error');
  assert.equal(calls.fill.length + calls.approve, 0);
  console.log('http page: refused before asking: OK');
}

{
  // A text field whose *label* says password is not a password field.
  const { calls, deps } = harness({ elements: '  [1] input:email "Email"\n  [2] input:text "input:password Password"' });
  const r = await fillLogin({ passwordRef: 2 }, {}, deps);
  assert.equal(r.kind, 'error');
  assert.equal(calls.fill.length + calls.approve, 0);
  const bad = harness({ elements: '  [1] textarea "Email"\n  [2] input:password "Password"' });
  assert.equal((await fillLogin({ passwordRef: 2, usernameRef: 1 }, {}, bad.deps)).kind, 'error');
  console.log('field kinds parsed, not pattern-matched: OK');
}

{
  // Said no: nothing fills, and the bot is told not to ask again.
  clearGrants();
  const { calls, deps } = harness({ answer: 'deny' });
  const r = await fillLogin({ passwordRef: 2, usernameRef: 1 }, { bot: 'scout', taskId: 't2' }, deps);
  assert.equal(r.kind, 'denied');
  assert.equal(calls.fill.length, 0);
  assert.match(r.text, /Do not call fill_login again/);
  console.log('denied: nothing filled: OK');
}

{
  // Multiple accounts on one site: the bot must say which.
  clearGrants();
  logins.addLogin({ url: 'https://github.com', username: 'alice-work', password: 'work-pw' });
  const { calls, deps } = harness();
  const r = await fillLogin({ passwordRef: 2 }, { taskId: 't3' }, deps);
  assert.equal(r.kind, 'error');
  assert.match(r.text, /alice-work/);
  assert.equal(calls.fill.length, 0);
  const r2 = await fillLogin({ passwordRef: 2, account: 'alice-work' }, { taskId: 't3' }, deps);
  assert.equal(r2.kind, 'filled');
  assert.equal(calls.fill[0].password, 'work-pw');
  logins.removeLogin('https://github.com alice-work');
  console.log('several accounts: must pick one; picks the right one: OK');
}

{
  // Two-step sign-in: one approval covers the username page and the password page.
  clearGrants();
  const { calls, deps } = harness();
  assert.equal((await fillLogin({ usernameRef: 1 }, { taskId: 't4' }, deps)).kind, 'filled');
  assert.equal((await fillLogin({ passwordRef: 2 }, { taskId: 't4' }, deps)).kind, 'filled');
  assert.equal(calls.approve, 1);
  // ...but not across tasks.
  await fillLogin({ passwordRef: 2 }, { taskId: 't5' }, deps);
  assert.equal(calls.approve, 2);
  console.log('two-step sign-in asks once per task: OK');
}

// --- the real approval flow ---------------------------------------------------

{
  // Silence is a no.
  clearGrants();
  const bot = db.createBot({ name: 'scout', provider: 'anthropic', model: 'x' });
  const task = db.createTask(bot.id, 'sign in to github');
  const { calls, deps } = harness({ answer: 'real' });
  const r = await fillLogin({ passwordRef: 2 }, { bot: 'scout', taskId: task.id, approvalWaitMs: 150 }, deps);
  assert.equal(r.kind, 'denied');
  assert.match(r.text, /did not answer/);
  assert.equal(calls.fill.length, 0);
  const [row] = logins.recentRequests(1);
  assert.equal(row.status, 'expired');
  assert.equal(row.decided_by, 'timeout');
  console.log('unanswered request expires; nothing fills: OK');

  // An answer from "another process" (straight to the row) is picked up by the poll.
  clearGrants();
  const pending = fillLogin({ passwordRef: 2 }, { bot: 'scout', taskId: task.id, approvalWaitMs: 5000 }, deps);
  await new Promise((r) => setTimeout(r, 60));
  const [req] = logins.pendingRequests();
  assert.ok(req, 'request row exists while waiting');
  assert.equal(req.origin, 'https://github.com');
  db.openDb()
    .prepare("UPDATE login_requests SET status='allowed', decided_by='human', decided_at=datetime('now') WHERE id=?")
    .run(req.id);
  assert.equal((await pending).kind, 'filled');
  console.log('answer from another process is honoured: OK');

  // In-process answer, with "always": the next request needs no one.
  clearGrants();
  const asked = new Promise((resolve) => logins.events.once('requested', resolve));
  const p2 = fillLogin({ passwordRef: 2 }, { bot: 'scout', taskId: 'other-task', approvalWaitMs: 5000 }, deps);
  logins.allow((await asked).id, { always: true });
  assert.equal((await p2).kind, 'filled');
  assert.equal(logins.listLogins().find((l) => l.origin === 'https://github.com').alwaysAllow, true);

  clearGrants();
  let prompted = false;
  const onReq = () => (prompted = true);
  logins.events.on('requested', onReq);
  assert.equal((await fillLogin({ passwordRef: 2 }, { bot: 'scout', taskId: 'third' }, deps)).kind, 'filled');
  logins.events.off('requested', onReq);
  assert.equal(prompted, false);
  assert.equal(logins.recentRequests(1)[0].decided_by, 'rule');
  console.log('"always for this site" fills without asking and is still audited: OK');

  // A Stop while waiting withdraws the request.
  logins.setAlwaysAllow('https://github.com', false);
  clearGrants();
  const ac = new AbortController();
  const p3 = fillLogin({ passwordRef: 2 }, { taskId: 'stopped', signal: ac.signal, approvalWaitMs: 5000 }, deps);
  await new Promise((r) => setTimeout(r, 30));
  ac.abort();
  const r3 = await p3;
  assert.equal(r3.kind, 'denied');
  assert.match(r3.text, /stopped/);
  console.log('Stop withdraws a pending sign-in: OK');

  const everything = JSON.stringify(db.openDb().prepare('SELECT * FROM login_requests').all());
  assert.ok(!everything.includes(SECRET), 'password in the database!');
  assert.ok(!readFileSync(process.env.MYBOT_DB).includes(Buffer.from(SECRET)), 'password in the db file!');
  console.log('no password anywhere in the database: OK');
}

{
  // Locked store: hands over instead of failing the run.
  logins.lock();
  clearGrants();
  const { calls, deps } = harness();
  const r = await fillLogin({ passwordRef: 2 }, {}, deps);
  assert.equal(r.kind, 'handoff');
  assert.equal(calls.fill.length + calls.approve, 0);
  logins.unlock(PASS);
  console.log('locked store: hand to human, ask nothing: OK');
}

// --- a real browser -----------------------------------------------------------

const { chromium } = await import('playwright-core');
const candidates = [process.env.MYBOT_TEST_CHROMIUM, chromium.executablePath(), '/opt/pw-browsers/chromium'];
const exe = candidates.find((p) => p && existsSync(p));

if (!exe) {
  console.log('real-browser fill: SKIPPED (no Chromium found)');
} else {
  const port = await new Promise((resolve) => {
    const s = createServer();
    s.listen(0, '127.0.0.1', () => {
      const p = s.address().port;
      s.close(() => resolve(p));
    });
  });
  const launched = await chromium.launch({
    executablePath: exe,
    headless: true,
    args: [`--remote-debugging-port=${port}`],
  });
  try {
    const ctx = await launched.newContext();
    // Pages served by route interception: real https origins, no network.
    const pages = {
      'https://bank.example/login': `<!doctype html><title>Bank</title>
        <form onsubmit="event.preventDefault(); document.body.dataset.sent='1'">
          <input type="email" id="u">
          <input type="password" id="p">
          <input type="text" id="spoof" placeholder="Password">
          <button>Sign in</button>
        </form>`,
    };
    await ctx.route('https://**/*', (route) => {
      const body = pages[route.request().url().split('?')[0]];
      return body ? route.fulfill({ status: 200, contentType: 'text/html', body }) : route.abort();
    });
    const page = await ctx.newPage();
    await page.goto('https://bank.example/login');

    process.env.MYBOT_CDP_PORT = String(port);
    const browser = await import('../src/computer/browser.ts');

    const snap = await browser.snapshot();
    const pwRef = Number(/\[(\d+)\] input:password/.exec(snap.elements)[1]);
    const userRef = Number(/\[(\d+)\] input:email/.exec(snap.elements)[1]);
    const spoofRef = Number(/\[(\d+)\] input:text "Password"/.exec(snap.elements)[1]);

    // Wrong origin: refused against the live page, not the snapshot.
    await assert.rejects(
      browser.fillCredential({ origin: 'https://github.com', passwordRef: pwRef, username: 'a', password: SECRET }),
      (e) => e.name === 'FillError' && !e.message.includes(SECRET),
    );
    // A text input labelled "Password" is refused.
    await assert.rejects(
      browser.fillCredential({ origin: 'https://bank.example', passwordRef: spoofRef, username: 'a', password: SECRET }),
      /not a password field/,
    );
    assert.equal(await page.inputValue('#spoof'), '');
    console.log('real browser: wrong origin and spoofed field refused: OK');

    // Through the whole tool path, with the real browser underneath.
    clearGrants();
    // An email address: the field is type=email, and the browser's own
    // validation (rightly) blocks submitting "alice" from it.
    logins.removeLogin('https://bank.example');
    logins.addLogin({ url: 'https://bank.example', username: 'alice@bank.example', password: SECRET });
    const r = await fillLogin(
      { passwordRef: pwRef, usernameRef: userRef, submit: true },
      { bot: 'scout', taskId: 'browser' },
      {
        snapshot: () => browser.snapshot(),
        fill: browser.fillCredential,
        approve: async () => ({ allowed: true, by: 'human', requestId: 'b' }),
      },
    );
    assert.equal(r.kind, 'filled', r.text);
    assert.equal(await page.inputValue('#p'), SECRET);
    assert.equal(await page.inputValue('#u'), 'alice@bank.example');
    assert.equal(await page.evaluate(() => document.body.dataset.sent), '1');
    console.log('real browser: approved login filled and submitted: OK');

    // The next page_read must not show the password — this field has no name,
    // no placeholder and no label, the case that used to fall through to .value.
    const after = await browser.snapshot();
    assert.ok(!after.elements.includes(SECRET), `password leaked into page_read:\n${after.elements}`);
    assert.ok(!after.text.includes(SECRET));
    console.log('real browser: filled password never appears in page_read: OK');

    await browser.disconnect();
  } finally {
    await launched.close();
  }
}

console.log('\nALL LOGINS TESTS PASSED');
