# Phase 3 — Results (verified 2026-06-21)

Phase 3 plan exit (`PLAN.md §5`): *the coach surface runs on the XREAL display path.* **Met in software, on the real daemon.** The XREAL Air is a USB-C DisplayPort monitor, not a BLE/SDK device, so there is no glasses SDK: the surface is a plain Coach Protocol WebSocket client and the launcher places a chromeless full-screen window on the external 1080p output. The previous SDK-based Phase 3 (MentraOS + Frame) was deleted (`surfaces/glasses/` removed, commit 1039087); this doc replaces the SDK results.

## What was added

- `surfaces/xreal/index.html` — display-tuned Coach Protocol client. True-black background (minimizes light leak through the birdbath lens), one dominant centered cue line (clamps to 76 px), a kind label in the cue's accent colour, a single dim transcript tail line, and a corner connection dot. No chrome, no scrolling, no consent footer (that lives on the controlling phone/overlay surface). It inherits the corrected `handle()` logic from the phone client (the `state.capturing`, `error.message`, and prompt-kind fixes from Group B), so it does not fork a stale copy.
- `scripts/launch-xreal.sh` — one command: detect the external 1080p output via `xrandr --listmonitors`, serve the surface on loopback, and open a borderless browser window sized and positioned to fill that output. Fails loud (dumps the monitor list, exits non-zero) when no external 1080p display is present. Documents the macOS and Windows paths (drag-to-display + full-screen) in its header, since `xrandr` is Linux/X11 only.

## Why no SDK

The XREAL Air presents to the OS as a second 1920x1080 monitor over USB-C Alt-Mode DisplayPort. Nothing renders "on the glasses" through an API; the OS extends the desktop onto it. So the correct integration is exactly the phone/overlay surface pattern pointed at that monitor, not a device SDK. This is why the glasses-SDK code was deleted rather than ported.

## Measurement 1 — external 1080p detection (real `xrandr`)

The dev box has a second physical output (`HDMI-0`, 1920x1080 at `+2560+180`) alongside a 1440p primary (`DP-2`) — the same shape an XREAL Air presents. The launcher's parser, run against the live `xrandr --listmonitors`, correctly skips the primary and selects the external 1080p output and its pixel offset:
```
candidate: name=DP-2   primary=1 w=2560 h=1440 x=0    y=0
candidate: name=HDMI-0 primary=0 w=1920 h=1080 x=2560 y=180
SELECTED:  HDMI-0 1920x1080 at +2560+180
```
`bash -n` clean. The `SOUFFLEUR_XREAL_OUTPUT=<name>` env override is honored for non-1080p external panels.

## Measurement 2 — surface render, end-to-end on the real daemon

Drove the real daemon (`souffleur-core --mode wav --wav assets/jfk.wav`, real `ggml-base.en` whisper model, real Ollama `qwen3:8b` suggestion backend) into the served surface and rendered it headless at a 1920x1080 viewport (Playwright). The captured frame is committed at `docs/plan/assets/xreal_render.png`. No synthetic events.

Measured DOM/CSS facts from the live render:
- background `rgb(0, 0, 0)` (true black) ✓
- cue text `rgb(255, 255, 255)`, computed font size `76px` (the large clamp) ✓
- cue horizontal centre at x=960 in a 1920 viewport (exact centre) ✓
- connection dot lit (`state.capturing` true) ✓

Rendered content was real, not placeholder:
- cue (real Ollama classification + text): `CUE — "Acknowledge their quote and ask for their perspective."`
- transcript tail (real whisper output of jfk.wav): `THEM — "And so my fellow Americans, ask not what your country can do for you, ask what you can do for your country."`

Daemon round-trip on this run: model warm 1245 ms (one-time), 1 prompt in 360 ms after the final transcript.

## What is NOT yet hardware-observed (needs the glasses on-head)

These depend on the physical XREAL Air and a human wearing it; they are deliberately **not** measured or estimated here, because fabricating optical numbers would be dishonest. Run this checklist with the glasses plugged in:

- [ ] Legibility of the 76 px cue through the birdbath lens at the virtual focal distance.
- [ ] Whether the true-black field actually reads as transparent/non-leaking in the lens (and is invisible to others in the room).
- [ ] Comfortable cue font size and max line count for a real conversation (the clamp may want tuning per eyesight).
- [ ] That the borderless window lands fully on the XREAL output (no spill onto the primary) under the user's window manager.
- [ ] End-to-end glance latency: speaking → cue visible in the lens, on the real audio path (mic + monitor), not the wav fixture.

## Status

Software path complete and verified on the real daemon. Glasses-on-head tuning is the remaining work and is hardware-gated. 18/18 engine tests pass; `cargo clippy --workspace` clean (unchanged by this surface, which is static HTML).
