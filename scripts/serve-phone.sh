#!/usr/bin/env bash
# Serve the phone surface over HTTP so a phone (or browser) can load it.
# The page connects to the Coach Protocol WebSocket; pass ?ws=ws://HOST:8123 to
# point it at a non-localhost core (run the core with --bind 0.0.0.0:8123 for that).
set -euo pipefail
cd "$(dirname "$0")/../surfaces/phone"
PORT="${1:-8080}"
echo "Serving phone surface on http://0.0.0.0:${PORT}/  (open ?ws=ws://<core-host>:8123)"
exec python3 -m http.server "$PORT" --bind 0.0.0.0
