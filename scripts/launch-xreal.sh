#!/usr/bin/env bash
# Launch the Souffleur cue surface on an XREAL Air (or any external 1080p
# display) as a borderless full-screen browser window.
#
# The XREAL Air is NOT a BLE/SDK device — over USB-C it is an ordinary
# DisplayPort monitor that the OS sees as a second 1920x1080 output. So this
# launcher is just: serve surfaces/xreal locally, find the external 1080p
# output, and place a chromeless browser window on it pointed at the Coach
# Protocol WebSocket. No glasses SDK is involved.
#
# Linux/X11 only (uses xrandr to find the output geometry).
#   macOS:   open the surface in Chrome with --app and drag the window onto the
#            glasses display, or use System Settings > Displays to make the
#            XREAL the only/extended screen, then Cmd-Ctrl-F to full-screen:
#              /Applications/Google\ Chrome.app/Contents/MacOS/Google\ Chrome \
#                --app=http://127.0.0.1:8084/
#   Windows: chrome.exe --app=http://127.0.0.1:8084/ then Win+Shift+Arrow to
#            push the window onto the XREAL display and F11 to full-screen.
#
# Override the auto-detected output by name:  SOUFFLEUR_XREAL_OUTPUT=DP-3 ./launch-xreal.sh
# Point at a non-default core:                ./launch-xreal.sh --ws ws://127.0.0.1:8123
set -euo pipefail

HTTP_PORT="${SOUFFLEUR_XREAL_PORT:-8084}"
CORE_HOST="127.0.0.1"
CORE_PORT="8123"
WS_URL=""
while [ $# -gt 0 ]; do
  case "$1" in
    --ws) WS_URL="$2"; shift 2 ;;
    --port) HTTP_PORT="$2"; shift 2 ;;
    -h|--help) sed -n '2,30p' "$0"; exit 0 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

here="$(cd "$(dirname "$0")" && pwd)"
surface_dir="$here/../surfaces/xreal"
[ -f "$surface_dir/index.html" ] || { echo "FATAL: $surface_dir/index.html not found" >&2; exit 1; }

command -v xrandr >/dev/null || { echo "FATAL: xrandr not found (Linux/X11 only — see header for macOS/Windows)" >&2; exit 1; }

# --- find the external 1080p output (non-primary, 1920x1080) ----------------
want_name="${SOUFFLEUR_XREAL_OUTPUT:-}"
sel_name="" ; sel_x="" ; sel_y="" ; sel_w="" ; sel_h=""
while read -r _idx flagged geom name; do
  case "$flagged" in *'*'*) primary=1 ;; *) primary=0 ;; esac
  w="${geom%%/*}"                 # 1920/600x1080/340+2560+180 -> 1920
  rest="${geom#*x}"               # -> 1080/340+2560+180
  h="${rest%%/*}"                 # -> 1080
  tmp="${geom#*+}"                # -> 2560+180
  x="${tmp%%+*}"                  # -> 2560
  y="${tmp#*+}"                   # -> 180
  if [ -n "$want_name" ]; then
    [ "$name" = "$want_name" ] && { sel_name="$name"; sel_x="$x"; sel_y="$y"; sel_w="$w"; sel_h="$h"; break; }
    continue
  fi
  # auto: first non-primary output at native 1080p (the XREAL Air's panel res)
  if [ "$primary" = "0" ] && [ "$w" = "1920" ] && [ "$h" = "1080" ]; then
    sel_name="$name"; sel_x="$x"; sel_y="$y"; sel_w="$w"; sel_h="$h"; break
  fi
done < <(xrandr --listmonitors | tail -n +2)

if [ -z "$sel_name" ]; then
  echo "FATAL: no external 1080p display found." >&2
  echo "Plug in the XREAL Air (it appears as a second 1920x1080 monitor) and retry." >&2
  echo "If your external output is not 1920x1080, force it with SOUFFLEUR_XREAL_OUTPUT=<name>." >&2
  echo "" >&2
  echo "Current monitors:" >&2
  xrandr --listmonitors >&2
  exit 1
fi
echo "External display: $sel_name  ${sel_w}x${sel_h} at +${sel_x}+${sel_y}"

# --- pick a browser ---------------------------------------------------------
BROWSER=""
for b in chromium chromium-browser google-chrome google-chrome-stable brave-browser; do
  command -v "$b" >/dev/null && { BROWSER="$b"; break; }
done
[ -n "$BROWSER" ] || { echo "FATAL: no Chromium/Chrome found (need chromium or google-chrome)" >&2; exit 1; }

# --- core reachability note (don't hard-fail; the surface auto-reconnects) ---
if ! (exec 3<>"/dev/tcp/${CORE_HOST}/${CORE_PORT}") 2>/dev/null; then
  echo "NOTE: core not reachable on ${CORE_HOST}:${CORE_PORT} yet — start it with:" >&2
  echo "  cargo run --bin souffleur-core" >&2
  echo "The surface will keep retrying until it connects." >&2
fi

# --- serve the surface locally ----------------------------------------------
PROFILE_DIR="$(mktemp -d)"
python3 -m http.server "$HTTP_PORT" --bind 127.0.0.1 --directory "$surface_dir" >/dev/null 2>&1 &
HTTP_PID=$!
cleanup() { kill "$HTTP_PID" 2>/dev/null || true; rm -rf "$PROFILE_DIR" 2>/dev/null || true; }
trap cleanup EXIT
sleep 0.4
kill -0 "$HTTP_PID" 2>/dev/null || { echo "FATAL: failed to serve surface on port $HTTP_PORT" >&2; exit 1; }

URL="http://127.0.0.1:${HTTP_PORT}/"
[ -n "$WS_URL" ] && URL="${URL}?ws=$(python3 -c 'import urllib.parse,sys;print(urllib.parse.quote(sys.argv[1],safe=""))' "$WS_URL")"
echo "Serving surface at $URL  ->  $BROWSER on $sel_name"

# Borderless app window sized to the external panel and placed at its offset.
# Exact size + position fills the XREAL output without depending on which
# monitor 'fullscreen' lands on; the page paints true black to the edges.
exec "$BROWSER" \
  --app="$URL" \
  --user-data-dir="$PROFILE_DIR" \
  --no-first-run --no-default-browser-check \
  --window-position="${sel_x},${sel_y}" \
  --window-size="${sel_w},${sel_h}" \
  --start-fullscreen
