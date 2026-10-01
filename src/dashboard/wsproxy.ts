import { createHash } from 'node:crypto';
import { connect, type Socket } from 'node:net';
import type { IncomingMessage } from 'node:http';
import type { Duplex } from 'node:stream';

/**
 * A WebSocket-to-TCP bridge, in stdlib.
 *
 * noVNC speaks RFB over a WebSocket; macOS Screen Sharing speaks RFB over a
 * plain TCP socket on 5900. Something has to sit in the middle, and the usual
 * answer is websockify — a Python daemon, or the `ws` package. Neither earns
 * its keep here: the framing this needs is a few hundred lines, and a second
 * process is a second thing to supervise, to fail on a cold start, and to leave
 * a port open after the console exits.
 *
 * Scope is deliberately narrow. This bridges one local TCP port to a socket the
 * console has already authorised; it is not a general WebSocket server.
 */

const GUID = '258EAFA5-E914-47DA-95CA-C5AB0DC85B11';

/** RFC 6455 opcodes, only the ones a binary bridge can legally see. */
const OP_CONTINUATION = 0x0;
const OP_TEXT = 0x1;
const OP_BINARY = 0x2;
const OP_CLOSE = 0x8;
const OP_PING = 0x9;
const OP_PONG = 0xa;

/**
 * Frame a payload for the client.
 *
 * Server-to-client frames are never masked, which is the whole of the
 * difference from the parsing side.
 */
function encodeFrame(payload: Buffer, opcode = OP_BINARY): Buffer {
  const len = payload.length;
  let header: Buffer;

  if (len < 126) {
    header = Buffer.alloc(2);
    header[1] = len;
  } else if (len < 65536) {
    header = Buffer.alloc(4);
    header[1] = 126;
    header.writeUInt16BE(len, 2);
  } else {
    header = Buffer.alloc(10);
    header[1] = 127;
    // A 64-bit length, written as two 32-bit halves: RFB never sends 4GB in one
    // frame, but writing only the low half would silently truncate if it did.
    header.writeUInt32BE(Math.floor(len / 2 ** 32), 2);
    header.writeUInt32BE(len >>> 0, 6);
  }

  header[0] = 0x80 | opcode; // FIN + opcode
  return Buffer.concat([header, payload]);
}

interface ParsedFrame {
  opcode: number;
  payload: Buffer;
  /** Bytes consumed from the front of the buffer. */
  size: number;
}

/**
 * Pull one frame off the front of `buf`, or return null if it is incomplete.
 *
 * TCP gives no message boundaries, so a frame can arrive in pieces and several
 * frames can arrive in one read. Treating a short read as a malformed frame is
 * the classic bug here — it shows up as a viewer that works on loopback and
 * corrupts over wifi, where the packets actually fragment.
 */
function decodeFrame(buf: Buffer): ParsedFrame | null {
  if (buf.length < 2) return null;

  const masked = (buf[1]! & 0x80) !== 0;
  let len = buf[1]! & 0x7f;
  let offset = 2;

  if (len === 126) {
    if (buf.length < offset + 2) return null;
    len = buf.readUInt16BE(offset);
    offset += 2;
  } else if (len === 127) {
    if (buf.length < offset + 8) return null;
    const high = buf.readUInt32BE(offset);
    const low = buf.readUInt32BE(offset + 4);
    len = high * 2 ** 32 + low;
    offset += 8;
  }

  let mask: Buffer | undefined;
  if (masked) {
    if (buf.length < offset + 4) return null;
    mask = buf.subarray(offset, offset + 4);
    offset += 4;
  }

  if (buf.length < offset + len) return null;

  const payload = Buffer.from(buf.subarray(offset, offset + len));
  if (mask) {
    for (let i = 0; i < payload.length; i++) payload[i] = payload[i]! ^ mask[i % 4]!;
  }

  return { opcode: buf[0]! & 0x0f, payload, size: offset + len };
}

export interface BridgeOptions {
  /** TCP port on the loopback interface to bridge to. */
  port: number;
  host?: string;
}

/**
 * Complete the WebSocket handshake on an upgrade, then pump bytes both ways.
 *
 * The caller is responsible for authorising the request first — by the time
 * this runs, the socket is trusted.
 */
export function bridge(req: IncomingMessage, socket: Duplex, opts: BridgeOptions): void {
  const key = req.headers['sec-websocket-key'];
  if (typeof key !== 'string') {
    socket.end('HTTP/1.1 400 Bad Request\r\n\r\n');
    return;
  }

  const accept = createHash('sha1').update(key + GUID).digest('base64');
  const headers = [
    'HTTP/1.1 101 Switching Protocols',
    'Upgrade: websocket',
    'Connection: Upgrade',
    `Sec-WebSocket-Accept: ${accept}`,
  ];

  // Older noVNC builds offer a subprotocol and refuse a server that ignores it.
  const offered = String(req.headers['sec-websocket-protocol'] ?? '')
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean);
  if (offered.includes('binary')) headers.push('Sec-WebSocket-Protocol: binary');

  socket.write(`${headers.join('\r\n')}\r\n\r\n`);

  const upstream: Socket = connect({ port: opts.port, host: opts.host ?? '127.0.0.1' });
  let inbound = Buffer.alloc(0);
  let closed = false;

  const shutdown = (): void => {
    if (closed) return;
    closed = true;
    upstream.destroy();
    socket.destroy();
  };

  upstream.on('connect', () => {
    upstream.setNoDelay(true);
  });
  upstream.on('data', (chunk: Buffer) => {
    if (!socket.writableEnded) socket.write(encodeFrame(chunk));
  });
  upstream.on('error', shutdown);
  upstream.on('close', () => {
    if (!socket.writableEnded) socket.write(encodeFrame(Buffer.alloc(0), OP_CLOSE));
    shutdown();
  });

  socket.on('data', (chunk: Buffer) => {
    inbound = inbound.length ? Buffer.concat([inbound, chunk]) : Buffer.from(chunk);

    for (;;) {
      const frame = decodeFrame(inbound);
      if (!frame) break;
      inbound = Buffer.from(inbound.subarray(frame.size));

      switch (frame.opcode) {
        case OP_BINARY:
        case OP_TEXT:
        case OP_CONTINUATION:
          upstream.write(frame.payload);
          break;
        case OP_PING:
          socket.write(encodeFrame(frame.payload, OP_PONG));
          break;
        case OP_PONG:
          break;
        case OP_CLOSE:
          shutdown();
          return;
        default:
          shutdown();
          return;
      }
    }
  });

  socket.on('error', shutdown);
  socket.on('close', shutdown);
}
