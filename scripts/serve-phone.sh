#!/usr/bin/env bash
# Serve the phone surface over HTTP so a phone (or browser) can load it.
# The page connects to the Coach Protocol WebSocket; pass ?ws=ws://HOST:8123 to
# point it at a non-localhost core.
#
# PRIVACY: the core binds 127.0.0.1 by default. To reach it from a phone, run the
# core on a NON-loopback address with the opt-in flag + a shared secret:
#   cargo run --bin souffleur-core -- --bind <tailscale-ip>:8123 --listen-lan --token <secret>
# then open the surface with the token:
#   http://<host>:8080/?ws=ws://<host>:8123/?token=<secret>
# Prefer a Tailscale/VPN address over 0.0.0.0 so the transcript stays off the open LAN.
set -euo pipefail
cd "$(dirname "$0")/../surfaces/phone"
PORT="${1:-8080}"
echo "Serving phone surface on http://0.0.0.0:${PORT}/"
echo "  open ?ws=ws://<core-host>:8123/?token=<secret>  (see header for the secure core flags)"
exec python3 -m http.server "$PORT" --bind 0.0.0.0
