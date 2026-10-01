#!/usr/bin/env python3
"""In-container handler.

This is the process the host talks to. It owns three things:

  1. Desktops. Bots do not get a desktop at build time -- the set of bots is not
     known then. A desktop is created on demand: a Linux user, an X display,
     XFCE, and a noVNC port.

  2. The file drop. When a bot asks for something off the human's machine, the
     host asks the human, and on approval pushes the file HERE rather than
     bind-mounting the host filesystem. A mount would give the container standing
     access to a whole directory tree for the life of the container; a push gives
     it exactly the file that was approved, once. That difference is the entire
     security story of host access, so it is deliberate.

  3. Staying alive. The container's lifetime is this daemon's lifetime.

Python stdlib only -- nothing to install, works offline.
"""
import base64
import json
import os
import re
import socket
import subprocess
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(os.environ.get("AGENTD_PORT", "7000"))
DESKTOPCTL = "/usr/local/bin/desktopctl.sh"
INBOX = "/shared/inbox"

# name -> {"user", "display", "webPort", "cdpPort"}
DESKTOPS: dict[str, dict] = {}
NEXT_DISPLAY = [1]
LOCK = threading.Lock()

SAFE_NAME = re.compile(r"^[a-z0-9][a-z0-9_-]{0,30}$")


def run(args: list[str]) -> tuple[int, str, str]:
    p = subprocess.run(args, capture_output=True, text=True)
    return p.returncode, p.stdout.strip(), p.stderr.strip()


def display_alive(display: int) -> bool:
    """Is an X server actually answering on :N?"""
    return run(["xdpyinfo", "-display", f":{display}"])[0] == 0


def cdp_alive(port: int) -> bool:
    """Is Chromium actually accepting DevTools connections on this port?"""
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=1):
            return True
    except OSError:
        return False


def start_desktop(name: str) -> dict:
    """
    Bring up a bot's desktop, rebuilding it if the cached one is dead.

    The registry is in-memory state about processes that can die without
    telling us — a `docker start` on an existing container restores this daemon
    but not the desktops, and a crashed Xvfb leaves its socket behind. Trusting
    the cache meant handing back a display number with nothing behind it, and
    the failure surfaced far away as "Chromium did not expose CDP".
    """
    with LOCK:
        cached = DESKTOPS.get(name)
        if cached and display_alive(cached["display"]):
            return cached
        if cached:
            DESKTOPS.pop(name, None)
            display = cached["display"]
        else:
            display = NEXT_DISPLAY[0]
            NEXT_DISPLAY[0] += 1

    code, out, err = run([DESKTOPCTL, "start", name, str(display)])
    if code != 0:
        raise RuntimeError(f"desktopctl start failed: {err or out}")

    info = json.loads(out.splitlines()[-1])
    with LOCK:
        DESKTOPS[name] = info
    return info


def start_browser(name: str) -> dict:
    info = start_desktop(name)
    # Same reasoning as the desktop: a cached port number proves nothing about
    # whether Chromium is still alive behind it. It had gone <defunct> here and
    # this function kept reporting the port anyway.
    if info.get("cdpPort") and cdp_alive(info["cdpPort"]):
        return info
    info.pop("cdpPort", None)
    code, out, err = run([DESKTOPCTL, "browser", name, str(info["display"])])
    if code != 0:
        raise RuntimeError(f"desktopctl browser failed: {err or out}")
    info.update(json.loads(out.splitlines()[-1]))
    with LOCK:
        DESKTOPS[name] = info
    return info


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_args):  # quiet; the host keeps the real log
        pass

    def _send(self, code: int, payload: dict):
        body = json.dumps(payload).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _body(self) -> dict:
        n = int(self.headers.get("content-length") or 0)
        if not n:
            return {}
        try:
            return json.loads(self.rfile.read(n))
        except json.JSONDecodeError:
            return {}

    def do_GET(self):
        if self.path == "/health":
            return self._send(200, {"ok": True, "desktops": len(DESKTOPS)})
        if self.path == "/desktops":
            return self._send(200, {"desktops": DESKTOPS})
        return self._send(404, {"error": "not found"})

    def do_POST(self):
        body = self._body()

        if self.path == "/desktops":
            name = str(body.get("name", "")).lower()
            if not SAFE_NAME.match(name):
                # The name becomes a Unix username and a shell argument.
                return self._send(400, {"error": "name must be [a-z0-9_-], max 31 chars"})
            try:
                return self._send(200, start_desktop(name))
            except Exception as exc:
                return self._send(500, {"error": str(exc)})

        if self.path == "/browser":
            name = str(body.get("name", "")).lower()
            if not SAFE_NAME.match(name):
                return self._send(400, {"error": "bad name"})
            try:
                return self._send(200, start_browser(name))
            except Exception as exc:
                return self._send(500, {"error": str(exc)})

        if self.path == "/files":
            # {name, path, contentBase64} -- one approved file, pushed in.
            name = str(body.get("name", "")).lower()
            rel = str(body.get("path", ""))
            data = body.get("contentBase64")
            if not SAFE_NAME.match(name) or not rel or data is None:
                return self._send(400, {"error": "name, path and contentBase64 are required"})

            # basename only: the host chose what to share, the container does not
            # get to pick where it lands.
            safe = os.path.basename(rel)
            if not safe or safe in (".", ".."):
                return self._send(400, {"error": "bad filename"})

            dest_dir = os.path.join(INBOX, name)
            os.makedirs(dest_dir, exist_ok=True)
            dest = os.path.join(dest_dir, safe)
            with open(dest, "wb") as fh:
                fh.write(base64.b64decode(data))
            os.chmod(dest, 0o644)
            return self._send(200, {"path": dest, "bytes": os.path.getsize(dest)})

        return self._send(404, {"error": "not found"})


def main():
    os.makedirs(INBOX, exist_ok=True)
    print(f"[agentd] listening on :{PORT}", flush=True)
    ThreadingHTTPServer(("0.0.0.0", PORT), Handler).serve_forever()


if __name__ == "__main__":
    main()
