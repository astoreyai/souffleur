# Phase 2 — Results (verified 2026-06-21)

Phase 2 plan exit (`PLAN.md §5`): *overlay works on a Windows screen-share with documented limits.* **Partially met by platform:** the overlay is built, runs, and was screenshotted on Linux; the actual screen-capture *exclusion* is a Windows-only capability and is the user's test on Windows. The overlay reports its own per-OS capability honestly rather than claiming invisibility it doesn't have — which is the plan's #1 identified market gap (`docs/research/02 §5`: "nobody solved macOS 15+ and most don't admit it's broken").

## What was built

- `surfaces/overlay/ui/index.html` — compact, corner-anchored overlay UI (Coach Protocol client): prompt chips on top, a 3-line transcript tail, transparent background, and an honest "hidden / VISIBLE on screen-share" badge.
- `surfaces/overlay/src-tauri/` — Tauri v2 app: frameless, transparent, always-on-top, skip-taskbar, click-through (`set_ignore_cursor_events`), `contentProtected: true`. A `capture_status` Rust command returns the real per-OS exclusion capability; the badge is driven by it.
- Excluded from the engine workspace (heavy webkit deps); built standalone.

## The per-OS invisibility matrix (the honest core)

`capture_status` (in `src-tauri/src/main.rs`) returns, by `cfg!(target_os)`:

| OS | `contentProtected` effect | badge | reported note |
|---|---|---|---|
| **Windows** | **real** — `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`; window absent from capture | 🟢 hidden on screen-share | excluded via WDA_EXCLUDEFROMCAPTURE |
| **macOS 15+** | **no-op** — ScreenCaptureKit ignores `NSWindowSharingNone` | 🟠 VISIBLE on screen-share | SCK ignores window exclusion on macOS 15+ |
| **Linux** | **no-op** — no per-window exclusion API on X11/Wayland | 🟠 VISIBLE on screen-share | no per-window capture-exclusion API |

This matches the confirmed research (`docs/research/02 §3`, `03 §4`). The overlay never claims "100% invisible"; it shows the truth for the OS it's running on. On startup the Rust side also logs a plain warning on any OS where it cannot hide.

## What was verified on this Linux box

- **Builds** (Tauri v2, webkit2gtk-4.1). 172 MB debug binary.
- **Runs on the real desktop (`:0`, 4480×1440):** the overlay window is frameless, transparent (the desktop wallpaper showed *through* it under the compositor), always-on-top, click-through. It connected to the daemon's WebSocket — confirmed because the daemon's `--wait-surface` only streams once a surface attaches, and the overlay attaching is what triggered the transcript + prompt.
- **Renders the UI + honest badge (screenshot):** captured on a dedicated **Xvfb** display (software framebuffer). The screenshot shows "Souffleur · ggml-base.en", the CUE prompt chip ("Acknowledge their quote and ask for their perspective."), the THEM transcript tail, and the **🟠 VISIBLE on screen-share** badge — the badge text coming from the real Rust `capture_status` command (`{os: linux, hidden: false}`), not a guess.
- **Startup capability log:** `[overlay] WARNING: this OS (linux) does NOT hide windows from screen capture … no per-window screen-capture exclusion API on X11/Wayland`.

### Screenshot-method note (honest)
On the real compositor display (`:0`), `import` (X11 root grab) could not capture the webview's **GPU-accelerated** GL surface — the grab showed only the wallpaper *through* the transparent window (proving transparency + compositing, but not the content). The content screenshot was therefore taken on a software-rendered **Xvfb** display, where the webview composites into the X pixmap. The overlay genuinely rendered and functioned on `:0` either way (it drove the daemon's prompt).

## What is NOT verified here (and is the user's test)

1. **The actual screen-capture exclusion on Windows** — that the overlay vanishes from a real Zoom / Teams / Meet / OBS screen-share. Linux has no exclusion API (no-op, honestly reported); only Windows can demonstrate the real "invisible" behavior. **This is the single claim to verify on a Windows machine.**
2. **macOS 15+ is known-broken** (research-confirmed); the overlay reports it as VISIBLE rather than pretending otherwise.

## How to run

```bash
# Terminal A — the coach core:
cargo run --release --bin souffleur-core -- --mode duplex --wait-surface
# Terminal B — the overlay (connects to ws://127.0.0.1:8123):
cd surfaces/overlay/src-tauri && cargo run
# On Windows the overlay is excluded from screen capture; on macOS 15+/Linux it
# is visible (the badge says so). Click-through by default.
```

## Carry-forward

- Verify real Windows screen-share exclusion on a Windows host (the true Phase-2 exit).
- A hotkey to toggle click-through / move the overlay (currently always click-through).
- Smart-glasses surface (MentraOS) is the next plan step (`PLAN.md §5`, Phase 3).
