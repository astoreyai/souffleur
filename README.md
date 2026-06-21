# Souffleur

[![ci](https://github.com/astoreyai/souffleur/actions/workflows/ci.yml/badge.svg)](https://github.com/astoreyai/souffleur/actions/workflows/ci.yml)

> *A souffleur is the theatre prompter who, unseen from the audience, whispers an actor's next line.*

Open-source, **local-first, privacy-preserving real-time conversation coach**. It captures live meeting audio, transcribes it on-device, and surfaces short AI-generated prompts (facts to recall, questions to ask, objection handling, next-line cues) on a surface that is **off the shared screen**: your phone, smart glasses, or an un-shared monitor, with a best-effort desktop overlay as a secondary option.

- **License:** AGPL-3.0 (see `LICENSE`), which prevents closed-SaaS forks of the project.
- **Inference:** local-only by default (audio never leaves the machine); cloud STT/LLM is opt-in, BYO-key, and consent-gated.
- **Core stack:** Rust workspace + Tauri overlay; polyglot surfaces behind the Coach Protocol seam.

> **Status:** Phases 0 through 4 implemented and verified on real audio, a real whisper model, a real local LLM, and real devices (see `docs/plan/PHASE0_RESULTS.md` ... `PHASE4_RESULTS.md`). Local suggestions run on Ollama by default; the cloud tier (Gemini verified, Anthropic/OpenAI BYO-key) is opt-in and consent-gated. Glasses support targets the XREAL Air as a plain external display (no SDK).
> **Privacy/git:** local-only repository (no remote). Do not push without an explicit decision. No real recordings, models, or secrets are committed (see `.gitignore`).

## Quickstart

### Prerequisites

- **Rust** (stable). The MSRV is declared as 1.80 in `Cargo.toml`.
- **Native build deps** for the engine:
  - `cmake`, `clang`, `libclang-dev` (whisper-rs builds whisper.cpp and generates bindings)
  - `pkg-config`, `libasound2-dev` (cpal links ALSA on Linux)
  - On Debian/Ubuntu: `sudo apt-get install build-essential cmake clang libclang-dev pkg-config libasound2-dev`
- **PulseAudio** (`parec`) to capture the far side of a call (the "them" channel) from the system-audio monitor. Linux today; the macOS/Windows loopback path is on the roadmap.
- **Ollama** with a local model for suggestions, e.g. `ollama pull qwen3:8b` (GPU recommended; runs on CPU too).

### Fetch the real assets (gitignored)

```bash
./scripts/fetch-assets.sh    # ggml-base.en.bin (~142 MB) + jfk.wav sample
```

### Build and test

```bash
cargo build
cargo test --workspace        # offline + device-free; no model or network needed
```

The CI gate is fmt, clippy `-D warnings`, the full test suite, and an MSRV check. Run it
locally with `scripts/ci.sh`:

```bash
./scripts/ci.sh                          # fmt + clippy + test, same checks as CI
git config core.hooksPath .githooks      # optional: run it automatically on every push
```

A GitHub Actions workflow (`.github/workflows/ci.yml`) runs the same gate on every push and PR.

### Run the core

The core binds `127.0.0.1:8123` by default and speaks the Coach Protocol (NDJSON over WebSocket).

```bash
# Live: mic (you) + system-audio monitor (them), local Ollama suggestions
cargo run --bin souffleur-core -- --mode duplex

# Offline smoke test against the bundled real sample
cargo run --bin souffleur-core -- --mode wav --wav assets/jfk.wav
```

### Open a coaching surface

The surface is deliberately on a different device or display from the shared screen.

```bash
./scripts/serve-phone.sh           # phone PWA; open the printed URL on your phone
./scripts/launch-xreal.sh          # XREAL Air or any external 1080p monitor (kiosk window)
cd surfaces/overlay/src-tauri && cargo run   # desktop overlay (Tauri; needs Tauri's webkit deps)
```

### Reach the core from another device (phone over a private network)

The core refuses a non-loopback bind unless you opt in AND set a shared secret, so the transcript never lands on the open LAN by accident:

```bash
cargo run --bin souffleur-core -- --bind <tailscale-ip>:8123 --listen-lan --token <secret>
# then open the surface with the token:
#   http://<host>:8080/?ws=ws://<host>:8123/?token=<secret>
```

Prefer a Tailscale/VPN address over `0.0.0.0`.

### Cloud suggestions (opt-in, consent-gated, BYO-key)

Local Ollama is the default and keeps audio on the machine. To use a cloud LLM you must pass `--allow-cloud` AND a consenting surface must disclose it; otherwise the core refuses to transmit.

```bash
GEMINI_API_KEY=... cargo run --bin souffleur-core -- --suggest-backend gemini --allow-cloud
# backends: local (default) | gemini | claude | openai
```

## Layout

```
crates/
  souffleur-protocol/   Coach Protocol v0 types (the core<->surface NDJSON event/control contract)
  souffleur-engine/     audio capture, streaming STT (whisper), suggestion engine
                        bins: souffleur-core (daemon), latency-harness, ws-tap
surfaces/
  phone/                off-device phone PWA
  overlay/              Tauri v2 desktop overlay (frameless, click-through, honest capture badge)
  xreal/                display surface for XREAL Air / any external 1080p monitor
scripts/                fetch-assets.sh, serve-phone.sh, launch-xreal.sh
docs/
  research/             01 commercial · 02 open-source · 03 audio/STT/overlay · 04 smart glasses (all cited)
  plan/                 PLAN.md, COACH_PROTOCOL.md, PHASE0..4_RESULTS.md
models/, assets/        gitignored real model + sample audio (populated by fetch-assets.sh)
LICENSE                 AGPL-3.0
```

## Why this exists (intended use)

Real-time retrieval-not-recall support for:
- **Accessibility / memory accommodation:** surfacing names, facts, and threads you would otherwise lose under pressure.
- **Meetings:** live notes, action-item capture, "what was the number we agreed on?"
- **Sales / customer calls:** objection handling, product facts, next-best-question.
- **Public speaking / teleprompting:** discreet cue cards on glasses.

This is a dual-use category. The plan treats **consent, recording law, and venue policy as first-class design constraints** (see `docs/plan/PLAN.md §7`). Default mode is **disclosed + on-device**. It is **not** built to defeat exam proctoring or interview-honesty policies; the optional desktop overlay's screen-capture exclusion exists for legitimate privacy of your own coaching notes during a presentation you are giving, and the plan documents that boundary honestly (including that it is Windows-only and broken on macOS 15+).

## Key design decision

Both "must not be visible on screen-share" and "smart glasses as an extension" are satisfied by the **same** move: put the coaching surface **off the shared machine**. Same-machine screen-capture exclusion is a losing, platform-dependent arms race (works on Windows, broken on macOS 15+, none on Linux). See `docs/plan/PLAN.md §0`.
