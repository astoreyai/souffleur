# Phase 3 — Results (2026-06-21)

Phase 3 plan exit (`PLAN.md §5`): *live prompts on real glasses; measure on-hardware chars/line + end-to-end latency.* **Blocked on hardware** for the live-on-lens part (no glasses on this dev box). Built and verified everything that does NOT require the physical glasses, against the **real** SDKs — and explicitly flag the on-glasses verification as the user's test, rather than faking a glasses demo (per the no-stub rule).

## Architecture

Both glasses surfaces are **thin display surfaces** (like the phone and the overlay): they connect to the Souffleur core's Coach Protocol WebSocket and push prompts + transcript to the lens. The core does all capture / STT / suggestion. No new audio/LLM logic on the glasses side.

## MentraOS surface (primary) — `surfaces/glasses/mentraos/`

Real `@mentra/sdk` **v2.1.29** (installed from npm). An `AppServer` subclass whose `onSession` opens a WS to the core and pushes events to the lens via `session.layouts.showReferenceCard(...)` / `showTextWall(...)`, coalesced to the MentraOS ~300 ms display rate limit.

**Verified here:**
- `npm run typecheck` **passes** against the real SDK types → the API usage (`AppServer`, `onSession(session, sessionId, userId)`, `session.layouts.showReferenceCard/showTextWall`, `addCleanupHandler`, `AppServerConfig{packageName, apiKey, port}`) is correct, not guessed.
- Bridge unit tests: **12/12 pass** (`src/bridge.test.ts`, `node --test`): event parsing, pagination + ellipsis, prompt→reference-card mapping, transcript→text mapping, partial/state suppression, and the `DisplayCoalescer` (300 ms rate limit + prompt-over-transcript priority).

## Frame surface (fully-open, PC-direct) — `surfaces/glasses/frame/`

Real `frame-sdk` **1.2.4** (installed from pip). An async script that drives the Frame lens directly over BLE — no phone, no cloud — via `frame.display.show_text/scroll_text/clear` (API confirmed by introspection of the installed package).

**Verified here:**
- Bridge unit tests: **10/10 pass** (`pytest`): same logic as the TS bridge, Python port.
- `frame_bridge.py` **imports cleanly** against the real `frame_sdk` + `websockets` (module-level code valid; `main()`/BLE not run).

## What is HARDWARE-BLOCKED (the user's test)

These require physical glasses and cannot be done on this box:

1. **MentraOS live display** — needs MentraOS-supported glasses (Even Realities G1/G2, Vuzix Z100, Mentra Mach1) + the MentraOS phone app + an app registered at console.mentra.glass (`MENTRAOS_PACKAGE` + `MENTRAOS_API_KEY`) + this server reachable by MentraOS Cloud (public URL / tunnel).
2. **Frame live display** — needs Brilliant Labs Frame glasses + BLE on the host.
3. **The two undocumented unknowns** (`docs/research/04 §4`): the real **chars-per-line / lines** each lens shows, and the **end-to-end latency** core→lens. The `DEFAULT_DISPLAY` (MentraOS, 32×4) and `FRAME_CHARS_PER_LINE`/`FRAME_LINES` (40×5) values are placeholders to **measure on the real glasses and override** (both are env-configurable).

## Honest status

The glasses bridges are real code against real SDKs with tested hardware-independent logic; the MentraOS app type-checks against the live API. **No part of the on-lens path is faked.** The live-on-glasses verification, and the chars/line + latency measurements, are blocked on hardware the user must supply — that is the true Phase-3 exit and the next concrete step once glasses are available.

## How to run (with hardware)

See `surfaces/glasses/README.md`. Run the core (`--mode duplex --wait-surface --bind 0.0.0.0:8123`), then the glasses surface pointed at `SOUFFLEUR_WS`.
