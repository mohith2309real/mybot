#!/usr/bin/env bash
# Build MyBot.app — a real macOS app bundle wrapping the console.
#
# No Electron. Electron would add ~200MB and a second Chromium to a project that
# already runs one inside the container; a Chromium-based browser in --app mode
# gives a chromeless window that behaves like a native app for a few KB.
#
# The bundle owns the whole stack: it starts the dashboard, opens the window,
# and stops the server when the window closes.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:-$REPO/app/MyBot.app}"

rm -rf "$OUT"
mkdir -p "$OUT/Contents/MacOS" "$OUT/Contents/Resources"

cat > "$OUT/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>MyBot</string>
  <key>CFBundleDisplayName</key><string>MyBot</string>
  <key>CFBundleIdentifier</key><string>dev.mohith.mybot</string>
  <key>CFBundleVersion</key><string>0.3.0</string>
  <key>CFBundleShortVersionString</key><string>0.3.0</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleExecutable</key><string>MyBot</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

cat > "$OUT/Contents/MacOS/MyBot" <<LAUNCHER
#!/usr/bin/env bash
# MyBot launcher. Starts the console, opens a window, cleans up on exit.
set -uo pipefail

REPO="$REPO"
LOG="\${HOME}/.mybot/app.log"
mkdir -p "\$(dirname "\$LOG")"

# GUI apps do not inherit a login shell's PATH, so a node installed by nvm or
# Homebrew is invisible unless we go looking for it.
find_node() {
  for c in "\$(command -v node 2>/dev/null)" /opt/homebrew/bin/node /usr/local/bin/node; do
    [ -n "\$c" ] && [ -x "\$c" ] && { echo "\$c"; return; }
  done
  for v in "\$HOME"/.nvm/versions/node/*/bin/node; do
    [ -x "\$v" ] && { echo "\$v"; return; }
  done
}

NODE="\$(find_node)"
if [ -z "\$NODE" ]; then
  osascript -e 'display alert "MyBot needs Node.js" message "Install Node 22.6 or newer, then open MyBot again."'
  exit 1
fi

# Reuse an already-running console rather than starting a second one.
PORT_FILE="\${HOME}/.mybot/app.port"
if [ -f "\$PORT_FILE" ] && curl -fsS "http://127.0.0.1:\$(cat "\$PORT_FILE")/api/bots" >/dev/null 2>&1; then
  URL="http://127.0.0.1:\$(cat "\$PORT_FILE")"
else
  "\$NODE" "\$REPO/src/dashboard/launch.ts" >>"\$LOG" 2>&1 &
  SERVER_PID=\$!
  for _ in \$(seq 1 60); do
    [ -f "\$PORT_FILE" ] && curl -fsS "http://127.0.0.1:\$(cat "\$PORT_FILE")/api/bots" >/dev/null 2>&1 && break
    sleep 0.5
  done
  URL="http://127.0.0.1:\$(cat "\$PORT_FILE" 2>/dev/null || echo 7717)"
fi

open_window() {
  for app in "Helium" "Google Chrome" "Brave Browser" "Microsoft Edge"; do
    if [ -d "/Applications/\$app.app" ]; then
      open -na "/Applications/\$app.app" --args --app="\$1" \\
        --user-data-dir="\${HOME}/.mybot/window" --no-first-run >/dev/null 2>&1 && return 0
    fi
  done
  open "\$1"   # Safari has no --app mode; a normal tab is the fallback
}

open_window "\$URL"

# Hold the process so the Dock icon stays alive while the console runs.
#
# Deliberately WITHOUT killing the server on exit. The console is not the
# window: a paired phone talks to it, routines fire on a schedule, and an agent
# run can be mid-task. Tying its lifetime to a window meant closing that window
# — or quitting from the Dock — silently took all of that down, and the phone
# just saw a refused connection with nothing to explain it.
#
# Stop it deliberately instead:  mybot stop
if [ -n "\${SERVER_PID:-}" ]; then
  echo \$SERVER_PID > "\${HOME}/.mybot/console.pid"
  wait \$SERVER_PID
fi
LAUNCHER

chmod +x "$OUT/Contents/MacOS/MyBot"

# A generated icon, so the Dock shows something deliberate rather than a blank.
ICONSET="$(mktemp -d)/AppIcon.iconset"
mkdir -p "$ICONSET"
cp "$REPO/src/dashboard/public/icon.svg" "$ICONSET/icon.svg"

# sips cannot read SVG directly. rsvg-convert if present, otherwise route
# through a PDF, which macOS rasterises cleanly at any size.
render_icon() { # <size> <out.png>
  if command -v rsvg-convert >/dev/null 2>&1; then
    rsvg-convert -w "$1" -h "$1" "$ICONSET/icon.svg" -o "$2" 2>/dev/null && return 0
  fi
  if command -v qlmanage >/dev/null 2>&1; then
    qlmanage -t -s "$1" -o "$(dirname "$2")" "$ICONSET/icon.svg" >/dev/null 2>&1 \
      && mv "$(dirname "$2")/icon.svg.png" "$2" 2>/dev/null && return 0
  fi
  return 1
}

if command -v iconutil >/dev/null 2>&1; then
  ok=1
  for s in 16 32 64 128 256 512 1024; do
    render_icon "$s" "$ICONSET/icon_${s}x${s}.png" || ok=0
  done
  rm -f "$ICONSET/icon.svg"
  [ "$ok" = 1 ] && iconutil -c icns "$ICONSET" -o "$OUT/Contents/Resources/AppIcon.icns" 2>/dev/null
fi
cp "$REPO/src/dashboard/public/icon.svg" "$OUT/Contents/Resources/AppIcon.svg"

echo "Built $OUT"
echo
echo "  open \"$OUT\"          launch it"
echo "  cp -r \"$OUT\" /Applications/   keep it in the Dock"
