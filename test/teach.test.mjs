/**
 * Teach-a-task URL redaction.
 *
 * This exists because of a real leak found only by driving a real browser: a
 * form with method="get" puts every field into the query string, and the
 * recorder stores URLs verbatim. The original key list knew `password`, `pwd`
 * and `otp` but not `pw` or `cc`, so a demonstrated login was written to disk
 * as `?pw=hunter2&cc=4111111111111111`.
 *
 * Over-redaction is the intended failure mode here. A REDACTED in a demo costs
 * nothing; a password on disk costs everything.
 */
import assert from 'node:assert';
import { scrubUrl } from '../src/teach/index.ts';

const MUST_REDACT = [
  ['https://x.com/?pw=hunter2', 'pw'],
  ['https://x.com/?password=hunter2', 'password'],
  ['https://x.com/?pwd=hunter2', 'pwd'],
  ['https://x.com/?cc=4111111111111111', 'cc'],
  ['https://x.com/?card_number=4111111111111111', 'card_number'],
  ['https://x.com/?cvv=123', 'cvv'],
  ['https://x.com/?pin=9271', 'pin'],
  ['https://x.com/?otp=482913', 'otp'],
  ['https://x.com/?one_time_code=482913', 'one_time_code'],
  ['https://x.com/?ssn=123456789', 'ssn'],
  ['https://x.com/?api_key=sk-abc', 'api_key'],
  ['https://x.com/?access_token=abc', 'access_token'],
  ['https://x.com/?session=abc', 'session'],
  ['https://x.com/callback?code=authcode', 'code'],
];

for (const [url, key] of MUST_REDACT) {
  const out = scrubUrl(url);
  assert.ok(!/hunter2|4111111111111111|482913|123456789|sk-abc|authcode|9271|=123$/.test(out), `leaked ${key}: ${out}`);
  assert.ok(out.includes('REDACTED'), `${key} not marked redacted: ${out}`);
}
console.log(`credential query params redacted (${MUST_REDACT.length}): OK`);

// Ordinary params survive — a demo of "search for X" must still show X.
const kept = scrubUrl('https://x.com/?q=laptops&user=mohith&page=2&sort=price');
for (const frag of ['q=laptops', 'user=mohith', 'page=2', 'sort=price']) {
  assert.ok(kept.includes(frag), `over-redacted an ordinary param: ${kept}`);
}
console.log('ordinary query params kept: OK');

// The mixed case that started this: one real form submission.
const mixed = scrubUrl('https://example.com/?user=mohith&pw=hunter2&otp=482913&cc=4111111111111111&pin=9271');
assert.ok(mixed.includes('user=mohith'), 'username should survive');
for (const secret of ['hunter2', '482913', '4111111111111111', '9271']) {
  assert.ok(!mixed.includes(secret), `leaked ${secret}: ${mixed}`);
}
console.log('a real GET login submission leaks nothing: OK');

console.log('\nALL TEACH TESTS PASSED');
