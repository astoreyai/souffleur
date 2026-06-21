# Live test on Even Realities G1/G2 (MentraOS)

What's already verified on the dev box: the app **type-checks against the real
`@mentra/sdk`**, the bridge logic passes **12 unit tests**, and the **AppServer
boots** (`🎯 App server running at http://localhost:7010`). What only you can do
(needs the glasses + your MentraOS account) is the on-lens display and measuring
the real chars-per-line / latency.

## One-time setup

1. **Pair the G1/G2** to the **MentraOS phone app** (the glasses tether to your phone over BLE; the phone talks to MentraOS Cloud).
2. **Register the app** at <https://console.mentra.glass>:
   - Package name (e.g. `space.souffleur.coach`) → this is `MENTRAOS_PACKAGE`.
   - Generate an **API key** → `MENTRAOS_API_KEY`.
   - Set the app's **public server URL** to wherever this server is reachable (next step).
3. **Expose this server publicly** so MentraOS Cloud can reach its webhook (port 7010):
   - cloudflared: `cloudflared tunnel --url http://localhost:7010` → use the printed `https://…trycloudflare.com` as the console URL, or
   - Tailscale Funnel: `tailscale funnel 7010` → use the funnel URL.

## Run (two processes on this host)

```bash
# 1) the coach core (both sides + suggestions), localhost is fine — the glasses
#    app connects to it on the same host:
cargo run --release --bin souffleur-core -- --mode duplex --wait-surface

# 2) the MentraOS glasses app:
cd surfaces/glasses/mentraos && npm run build
MENTRAOS_PACKAGE="space.souffleur.coach" \
MENTRAOS_API_KEY="<from console>" \
SOUFFLEUR_WS="ws://127.0.0.1:8123" \
GLASSES_CHARS_PER_LINE=32 GLASSES_LINES=4 \
npm start
```

3. On the **phone**, open the MentraOS app and **start the Souffleur app**. A
   session opens → the overlay text appears on the **G1/G2 lens**: coaching cues
   as reference cards, the other party's transcript as a text wall, coalesced to
   the ~300 ms display limit.

## Tune to the lens (the measurement step)

`GLASSES_CHARS_PER_LINE` (default 32) and `GLASSES_LINES` (default 4) are
**placeholders** — the G1/G2 panel is 640×200 mono-green and the real glanceable
char/line budget is undocumented. Read a few real cues on the lens, then set
these env vars so cues fit without truncation or overflow. Note the value you
land on; that's the on-hardware measurement Phase 3 needs.

## Privacy / consent

The core is `--bind 127.0.0.1` by default (transcript stays on the host). Only the
MentraOS **webhook** is exposed publicly (it carries no transcript — that flows
core→app over localhost). Disclose the assistant to the room per the consent
guidance in `docs/plan/PLAN.md §7`.
