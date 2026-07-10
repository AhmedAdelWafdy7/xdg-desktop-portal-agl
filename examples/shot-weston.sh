#!/usr/bin/env bash
# Take a screenshot through the weston_capture_v1 backend, against a nested AGL weston.
# Usage: AGL=/path/to/AGL examples/shot-weston.sh [out.png]
set -euo pipefail

AGL="${AGL:-/home/wafdy/workspace-automation/AGL}"
OUT="${1:-shot.png}"
SOCK=wayland-wes
export LD_LIBRARY_PATH="$AGL/lib/aarch64-linux-gnu"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"

test -x "$AGL/bin/weston" || { echo "no weston at $AGL/bin (set AGL correctly)"; exit 1; }
echo "using $("$AGL/bin/weston" --version)"

# 1. Clean up any stale compositor squatting on the socket.
pkill -x weston 2>/dev/null || true
pkill -x agl-compositor 2>/dev/null || true
sleep 1
rm -f "$XDG_RUNTIME_DIR/$SOCK" "$XDG_RUNTIME_DIR/$SOCK.lock"

# 2. Launch AGL weston nested. --debug authorizes weston_capture_v1.
WAYLAND_DISPLAY=wayland-0 setsid "$AGL/bin/weston" \
    -B wayland-backend.so -S "$SOCK" --width=1024 --height=768 --debug \
    >/tmp/weston-shot.log 2>&1 &
disown
sleep 4
test -S "$XDG_RUNTIME_DIR/$SOCK" || { echo "weston failed:"; tail -5 /tmp/weston-shot.log; exit 1; }

# 3. Give it some content.
WAYLAND_DISPLAY="$SOCK" setsid "$AGL/bin/weston-terminal" >/dev/null 2>&1 & disown
WAYLAND_DISPLAY="$SOCK" setsid "$AGL/bin/weston-flower"   >/dev/null 2>&1 & disown
sleep 2

# 4. Capture.
WAYLAND_DISPLAY="$SOCK" cargo run --quiet --example capture -- wayland0 "$OUT"
echo "screenshot: $OUT"
echo "stop weston with: pkill -x weston"
