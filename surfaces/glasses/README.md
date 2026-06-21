# Souffleur — smart-glasses surfaces

Both surfaces are thin: they connect to the Souffleur core's Coach Protocol
WebSocket and display prompts + transcript on the lens. The core does all
capture / STT / suggestion (see `../../docs/plan/COACH_PROTOCOL.md`).

## `mentraos/` — MentraOS surface (primary)

A `@mentra/sdk` `AppServer` that pushes the stream to MentraOS-supported glasses
(Even Realities G1/G2, Vuzix Z100, Mentra Mach1) via `session.layouts.*`. Phone-
as-hub, MentraOS Cloud relays to the glasses over BLE.

```bash
cd mentraos && npm install
npm run typecheck   # type-checks against the real @mentra/sdk
npm test            # bridge unit tests (12)
# To run live you need: glasses + MentraOS phone app + a registered app at
# https://console.mentra.glass (MENTRAOS_PACKAGE + MENTRAOS_API_KEY) + a public
# URL so MentraOS Cloud can reach this server.
MENTRAOS_PACKAGE=... MENTRAOS_API_KEY=... SOUFFLEUR_WS=ws://127.0.0.1:8123 npm start
```

## `frame/` — Brilliant Labs Frame surface (fully-open, PC-direct)

A `frame-sdk` script that drives the Frame lens directly over BLE — no phone, no
cloud. The fully-open reference.

```bash
cd frame && python3 -m venv .venv && .venv/bin/pip install -r requirements.txt
.venv/bin/pytest            # bridge unit tests
# To run live you need: Brilliant Labs Frame glasses + BLE on this machine.
SOUFFLEUR_WS=ws://127.0.0.1:8123 .venv/bin/python frame_bridge.py
```

## What is verified vs hardware-blocked

The **bridge logic** (Coach Protocol -> lens text: pagination, prompt-over-
transcript priority, rate coalescing) is unit-tested and the MentraOS app
type-checks against the real SDK. The **on-glasses display, the chars-per-line /
lines geometry, and end-to-end latency** can only be verified on the physical
glasses (none on this dev box) — see `../../docs/plan/PHASE3_RESULTS.md`. The
`DEFAULT_DISPLAY` / `FRAME_CHARS_PER_LINE` values are placeholders to measure and
override on real hardware (`docs/research/04 §4`).
