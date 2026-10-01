#!/usr/bin/env bash
# Create and run one bot's desktop: a Linux user, an X display, XFCE, and a
# noVNC endpoint so a human can watch and take over.
#
# Called by agentd.py, not by hand. Each bot gets:
#   user     bot-<name>          own home, own files, own permissions
#   display  :<n>                own X server, so desktops are truly separate
#   vnc      5900+<n> -> 6080+<n> own live view
#
# "Hopping" between desktops is just viewing a different port — the containers
# and users are unchanged by who is looking.
set -euo pipefail

CMD="${1:-}"
NAME="${2:-}"
DISPLAY_NUM="${3:-1}"

USER_NAME="bot-${NAME}"
HOME_DIR="/home/${USER_NAME}"
VNC_PORT=$((5900 + DISPLAY_NUM))
WEB_PORT=$((6080 + DISPLAY_NUM))
GEOM="${SCREEN_GEOMETRY:-1440x900x24}"

log() { echo "[desktopctl] $*"; }

ensure_user() {
  if ! id "${USER_NAME}" >/dev/null 2>&1; then
    log "creating user ${USER_NAME}"
    # -G bots is what lets the daemon act as this user without being root.
    sudo useradd --create-home --shell /bin/bash -G bots "${USER_NAME}"
    # Every bot can reach the shared drop-box, but each keeps its own home.
    sudo chown "${USER_NAME}:${USER_NAME}" "${HOME_DIR}"
    sudo mkdir -p "${HOME_DIR}/Desktop" "${HOME_DIR}/chrome-profile"
    sudo chown -R "${USER_NAME}:${USER_NAME}" "${HOME_DIR}"
  fi
}

start() {
  ensure_user

  # A socket file is not a running X server. `docker start` on an existing
  # container brings back the entrypoint services but NOT the per-bot desktop,
  # while /tmp survives — so the stale /tmp/.X11-unix/XN left behind made this
  # report "already up", the desktop was never rebuilt, and Chromium launched
  # against a dead display and went straight to <defunct>. Ask the server
  # itself, and sweep the corpse if it does not answer.
  if xdpyinfo -display ":${DISPLAY_NUM}" >/dev/null 2>&1; then
    log "display :${DISPLAY_NUM} already up"
    echo "{\"user\":\"${USER_NAME}\",\"display\":${DISPLAY_NUM},\"webPort\":${WEB_PORT},\"reused\":true}"
    return 0
  fi

  if [ -e "/tmp/.X11-unix/X${DISPLAY_NUM}" ]; then
    log "stale display :${DISPLAY_NUM} (no X server behind it) — clearing"
    sudo rm -f "/tmp/.X11-unix/X${DISPLAY_NUM}" "/tmp/.X${DISPLAY_NUM}-lock"
  fi

  log "Xvfb :${DISPLAY_NUM} (${GEOM})"
  Xvfb ":${DISPLAY_NUM}" -screen 0 "${GEOM}" -nolisten tcp >/tmp/xvfb-${DISPLAY_NUM}.log 2>&1 &

  for _ in $(seq 1 60); do
    [ -e "/tmp/.X11-unix/X${DISPLAY_NUM}" ] && break
    sleep 0.2
  done
  [ -e "/tmp/.X11-unix/X${DISPLAY_NUM}" ] || { log "Xvfb failed"; cat /tmp/xvfb-${DISPLAY_NUM}.log; exit 1; }

  # XFCE runs AS the bot's user, so what the human sees over VNC is genuinely
  # that bot's session — its files, its browser profile, its permissions.
  log "xfce4 as ${USER_NAME} on :${DISPLAY_NUM}"
  sudo -u "${USER_NAME}" env \
    DISPLAY=":${DISPLAY_NUM}" HOME="${HOME_DIR}" USER="${USER_NAME}" \
    dbus-launch --exit-with-session xfce4-session \
    >"/tmp/xfce-${DISPLAY_NUM}.log" 2>&1 &

  log "x11vnc + noVNC on ${WEB_PORT}"
  x11vnc -display ":${DISPLAY_NUM}" -forever -shared -nopw -quiet \
         -rfbport "${VNC_PORT}" -localhost >/dev/null 2>&1 &

  for _ in $(seq 1 60); do
    (echo >"/dev/tcp/127.0.0.1/${VNC_PORT}") >/dev/null 2>&1 && break
    sleep 0.2
  done

  websockify --web=/usr/share/novnc "0.0.0.0:${WEB_PORT}" "127.0.0.1:${VNC_PORT}" \
    >/dev/null 2>&1 &

  echo "{\"user\":\"${USER_NAME}\",\"display\":${DISPLAY_NUM},\"webPort\":${WEB_PORT},\"reused\":false}"
}

browser() {
  # Chromium inside this bot's own session, with its own profile. CDP is bound
  # to loopback by Chromium regardless of flags, hence the relay.
  local cdp=$((9222 + DISPLAY_NUM))
  local relay=$((9322 + DISPLAY_NUM))

  # A profile volume outlives the container; Chromium stamps the creating
  # container's hostname into SingletonLock and refuses to start (exit 21) when
  # it no longer matches. We are the only user of this profile, so clearing a
  # stale lock is always safe -- and required, or persistence breaks on recreate.
  sudo -u "${USER_NAME}" rm -f "${HOME_DIR}"/chrome-profile/Singleton{Lock,Socket,Cookie} 2>/dev/null || true

  sudo -u "${USER_NAME}" env DISPLAY=":${DISPLAY_NUM}" HOME="${HOME_DIR}" \
    chromium \
      --remote-debugging-port="${cdp}" \
      --remote-allow-origins='*' \
      --user-data-dir="${HOME_DIR}/chrome-profile" \
      --window-size=1440,860 --window-position=0,0 \
      --no-first-run --no-default-browser-check \
      --disable-features=Translate,AcceptCHFrame \
      --disable-background-networking --disable-dev-shm-usage \
      --disable-gpu --no-sandbox \
      about:blank >/dev/null 2>&1 &

  CDP_UPSTREAM="${cdp}" CDP_LISTEN="${relay}" python3 /usr/local/bin/cdp-relay.py >/dev/null 2>&1 &
  echo "{\"cdpPort\":${relay}}"
}

stop() {
  pkill -f "Xvfb :${DISPLAY_NUM}" 2>/dev/null || true
  pkill -f "rfbport ${VNC_PORT}" 2>/dev/null || true
  pkill -f "0.0.0.0:${WEB_PORT}" 2>/dev/null || true
  echo "{\"stopped\":${DISPLAY_NUM}}"
}

case "${CMD}" in
  start)   start ;;
  browser) browser ;;
  stop)    stop ;;
  *) echo "usage: desktopctl.sh {start|browser|stop} <name> <display-num>" >&2; exit 2 ;;
esac
