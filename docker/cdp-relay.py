#!/usr/bin/env python3
"""Relay a published port to Chromium's loopback-only DevTools port.

Chromium ignores --remote-debugging-address and binds DevTools to 127.0.0.1
whatever you ask for, so a published Docker port reaches nothing. This forwards
0.0.0.0:$CDP_LISTEN -> 127.0.0.1:$CDP_UPSTREAM.

Two things make this less trivial than a byte pump:

1. DevTools rejects any request whose Host header is not loopback, and clients
   send the address they dialled. So Host has to be rewritten to the upstream.

2. The rewrite has to happen on EVERY request, not just the first. Playwright
   fetches /json/version and then sends the WebSocket upgrade down the SAME
   keep-alive connection. Rewriting only the first request leaves the upgrade
   carrying the original Host, DevTools refuses it, and connectOverCDP hangs
   until it times out -- while curl works perfectly, because curl closes the
   connection after one request and never reuses it.

Once the connection has been upgraded to a WebSocket, rewriting stops: from
that point the bytes are frames, not headers, and a blind search-and-replace
could corrupt payload data.
"""
import os
import socket
import threading

UPSTREAM = int(os.environ.get("CDP_UPSTREAM", "9222"))
LISTEN = int(os.environ.get("CDP_LISTEN", "9223"))
HOST_VALUE = f"127.0.0.1:{UPSTREAM}".encode()


class Conn:
    """Per-connection state.

    `origin_host` is whatever the client dialled (e.g. 127.0.0.1:52351, the
    Docker-assigned host port). We need it because Chromium advertises its
    websocket URL using its OWN address -- ws://127.0.0.1:9223 -- which is
    meaningless outside the container. Playwright reads that URL from
    /json/version and connects to it directly, so unless the port is swapped
    back to the one the caller can actually reach, it dials a dead port and
    hangs until timeout.
    """

    def __init__(self):
        self.upgraded = False
        self.origin_host = b""


def rewrite_host(chunk: bytes, state: "Conn") -> bytes:
    """Point the Host header at loopback, remembering what the client dialled."""
    head, sep, rest = chunk.partition(b"\r\n\r\n")
    if not sep:
        return chunk

    lines = []
    for line in head.split(b"\r\n"):
        if line[:5].lower() == b"host:":
            state.origin_host = line[5:].strip()
            lines.append(b"Host: " + HOST_VALUE)
        else:
            lines.append(line)
    return b"\r\n".join(lines) + sep + rest


def client_to_upstream(client: socket.socket, upstream: socket.socket, state: Conn):
    try:
        while True:
            chunk = client.recv(65536)
            if not chunk:
                break
            # Only touch bytes while this is still an HTTP conversation.
            if not state.upgraded and chunk[:4] in (b"GET ", b"POST", b"PUT ", b"HEAD"):
                chunk = rewrite_host(chunk, state)
            upstream.sendall(chunk)
    except OSError:
        pass
    finally:
        shutdown(client, upstream)


def upstream_to_client(upstream: socket.socket, client: socket.socket, state: Conn):
    try:
        while True:
            chunk = upstream.recv(65536)
            if not chunk:
                break
            # Responses are forwarded byte-for-byte. An earlier version rewrote
            # Chromium's self-reported ws:// address in the body, which desynced
            # Content-Length the moment the two ports had different digit counts
            # (":9223" is 14 bytes, ":52475" is 15). The port swap belongs on the
            # client, where it costs nothing -- see src/computer/browser.ts.
            if not state.upgraded and b"101 Switching Protocols" in chunk[:64]:
                state.upgraded = True
            client.sendall(chunk)
    except OSError:
        pass
    finally:
        shutdown(upstream, client)


def shutdown(*socks: socket.socket):
    for s in socks:
        try:
            s.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass


def serve(client: socket.socket):
    try:
        upstream = socket.create_connection(("127.0.0.1", UPSTREAM))
    except OSError:
        client.close()
        return

    state = Conn()
    threading.Thread(target=client_to_upstream, args=(client, upstream, state), daemon=True).start()
    upstream_to_client(upstream, client, state)
    client.close()
    upstream.close()


def main():
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("0.0.0.0", LISTEN))
    srv.listen(64)
    print(f"[cdp-relay] {LISTEN} -> 127.0.0.1:{UPSTREAM}", flush=True)
    while True:
        conn, _ = srv.accept()
        threading.Thread(target=serve, args=(conn,), daemon=True).start()


if __name__ == "__main__":
    main()
