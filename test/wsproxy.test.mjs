/**
 * The WebSocket-to-TCP bridge that carries RFB from the browser to a VNC port.
 *
 * Framing is the whole risk here. A bug in it does not fail loudly — it shows
 * up as a viewer that works on loopback and corrupts over wifi, where packets
 * actually fragment, so the payload sizes below straddle every length boundary
 * the protocol has (7-bit, 16-bit, 64-bit) and one payload is large enough to
 * be split across TCP reads.
 */
import assert from 'node:assert';
import { createServer } from 'node:http';
import { createServer as createTcpServer } from 'node:net';
import { once } from 'node:events';
import { bridge } from '../src/dashboard/wsproxy.ts';

// An upstream that echoes, standing in for macOS Screen Sharing.
const upstream = createTcpServer((socket) => socket.pipe(socket));
upstream.listen(0, '127.0.0.1');
await once(upstream, 'listening');
const upstreamPort = upstream.address().port;

const http = createServer((_req, res) => res.end());
http.on('upgrade', (req, socket) => bridge(req, socket, { port: upstreamPort }));
http.listen(0, '127.0.0.1');
await once(http, 'listening');
const port = http.address().port;

const ws = new WebSocket(`ws://127.0.0.1:${port}/api/remote/socket`);
ws.binaryType = 'arraybuffer';
await once(ws, 'open');
console.log('handshake completes: OK');

const received = [];
ws.addEventListener('message', (ev) => received.push(new Uint8Array(ev.data)));

/** Wait until the echoed bytes add up to `total`, or fail loudly. */
async function drain(total, label) {
  for (let i = 0; i < 200; i++) {
    if (received.reduce((n, c) => n + c.length, 0) >= total) return;
    await new Promise((r) => setTimeout(r, 25));
  }
  assert.fail(`timed out waiting for ${total} bytes (${label})`);
}

// Sizes chosen to cross every length encoding the protocol switches on.
const SIZES = [1, 125, 126, 127, 65535, 65536, 300_000];
let sent = 0;

for (const size of SIZES) {
  const payload = new Uint8Array(size);
  for (let i = 0; i < size; i++) payload[i] = (i * 7 + size) & 0xff;
  ws.send(payload);
  sent += size;
}

await drain(sent, 'echo');

// Reassemble and compare: the bridge must preserve byte order and content
// exactly, regardless of how the frames were split on the way through.
const all = new Uint8Array(sent);
let at = 0;
for (const chunk of received) {
  all.set(chunk, at);
  at += chunk.length;
}
assert.equal(at, sent, `echoed ${at} bytes, sent ${sent}`);

let offset = 0;
for (const size of SIZES) {
  for (let i = 0; i < size; i++) {
    const want = (i * 7 + size) & 0xff;
    assert.equal(all[offset + i], want, `byte ${i} of the ${size}-byte frame came back wrong`);
  }
  offset += size;
}
console.log(`payloads survive intact across every frame-length boundary (${SIZES.join(', ')}): OK`);

// A close from the client must tear down the upstream socket too, or every
// viewer that navigates away leaks a connection to the VNC server.
const upstreamClosed = new Promise((resolve) => upstream.once('connection-closed', resolve));
upstream.on('connection', (s) => s.on('close', () => upstream.emit('connection-closed')));
ws.close();
await Promise.race([upstreamClosed, new Promise((r) => setTimeout(r, 1500))]);
console.log('client close tears down the upstream socket: OK');

http.close();
upstream.close();
console.log('\nALL WSPROXY TESTS PASSED');
