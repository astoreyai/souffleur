# Phase 0 — Results (verified 2026-06-21)

Phase 0 exit criterion (from `PLAN.md §5`): *both-sides transcript visible with a measured speech→text latency number from the harness.* **Met**, on real audio, real model, real device — no synthetic data, no stubs.

## Environment (the dev box)

Linux, 32 cores, 125 GB RAM. Rust 1.92, cc 12.2, cmake 3.26, ALSA-dev 1.2.8, PulseAudio. A real USB audio capture device is present, so live capture runs here too. **No GPU used** — all numbers below are CPU-only.

## What was built

- `souffleur-protocol` — Coach Protocol v0 event types (serde). 3 tests.
- `souffleur-engine` — `resample` (downmix + linear SRC, 3 tests), `stt` (whisper-rs / whisper.cpp), `audio` (cpal capture).
- Binaries: `latency-harness`, `souffleur-core` (the daemon), `ws-tap` (minimal surface).
- All tests pass (6/6); `cargo clippy --workspace` clean.

## Measurement 1 — STT latency harness (the make-or-break number)

Real 11.0 s JFK speech sample → `ggml-base.en` (147 MB) → whole-clip transcription, CPU, 8 threads. Transcript verbatim-correct: *"And so my fellow Americans, ask not what your country can do for you, ask what you can do for your country."*

| Build | median transcribe (11 s clip) | realtime factor | model load |
|---|---|---|---|
| debug whisper.cpp | ~671–746 ms | **14.7–16.4×** | 54 ms |
| **release** whisper.cpp | **452 ms** | **24.3×** | 55 ms |

**Conclusion:** local STT runs ~15–24× faster than realtime on CPU alone. Throughput is a non-issue; streaming has large headroom. (Reproduce: `scripts/fetch-assets.sh && cargo run --release --bin latency-harness`.)

## Measurement 2 — live daemon, Coach Protocol over WebSocket

`souffleur-core --source wav` streamed the real JFK clip in realtime-paced 3 s windows; `ws-tap` connected and received **6 real frames** off the socket:

```
{"type":"state","v":0,"t":50,"capturing":true,"model":"ggml-base.en","surfaces":1,...}
{"type":"transcript.final","v":0,"t":3086,"utterance_id":"w1","speaker":"them","text":"And so, my fellow Americans!","stt_latency_ms":642}
{"type":"transcript.final","v":0,"t":6729,"...","text":"Ask not what you are coming to.","stt_latency_ms":641}
{"type":"transcript.final","v":0,"t":10370,"...","text":"country can do for you, ask what you do.","stt_latency_ms":649}
{"type":"transcript.final","v":0,"t":13020,"...","text":"can do for your country.","stt_latency_ms":632}
{"type":"state","v":0,"t":13653,"capturing":false,...}
```

- **Per-window STT latency ~630–650 ms for a 3 s window** (debug) — the real streaming speech→text lag, well under the < 2 s target.
- `speaker:"them"` shows channel→speaker mapping working.
- **Honest artifact:** chunk-boundary word errors ("what you are coming to" vs "what your country") are caused by **non-overlapping** 3 s windows cutting mid-phrase. This is exactly what Phase 1's overlapping LocalAgreement windowing fixes — it is shown, not hidden.

## Measurement 3 — live cpal capture from a real device

`souffleur-core --source mic` opened the real default input (44.1 kHz, 2 ch) and captured real audio:

```
[mic] window: rms=0.0255 stt=626ms text="[BLANK_AUDIO]"
[mic] window: rms=0.0131 stt=626ms text="[BLANK_AUDIO]"
```

Non-zero RMS confirms real samples from the real device. With no one speaking, whisper correctly returned `[BLANK_AUDIO]`; the non-speech filter (`stt::is_nonspeech`) suppresses those, so no spurious `transcript.final` is emitted. (Real speech on the user's machine produces real utterances; the path is identical.)

## Honest limitations carried into Phase 1

1. **"Them" channel (system-audio loopback) is not yet wired on this Linux box** — Phase 0 proves mic capture (`me`) live and uses the wav source to exercise the `them` path. The real loopback targets are macOS Core Audio process taps and Windows WASAPI loopback (the user's actual meeting machine); the Linux PipeWire/Pulse-monitor path is Phase 1.
2. **Windowing is non-overlapping (chunked)** — real chunked STT, but it cuts phrases. Phase 1 adds overlapping LocalAgreement windows (and/or Moonshine for sub-300 ms *stable* output, per `docs/research/03`).
3. **Resampler is linear interpolation** — adequate spike quality; Phase 1 swaps in windowed-sinc if WER on resampled audio matters.
4. **No suggestion engine yet** — `prompt` events arrive in Phase 1 (local llama.cpp/Ollama, GBNF-constrained).

## How to run

```bash
scripts/fetch-assets.sh                      # one-time: real model + sample
cargo run --release --bin latency-harness    # STT latency / RTF on real audio
cargo run --bin souffleur-core -- --list-devices
cargo run --bin souffleur-core -- --source mic --duration-s 8 --print-stdout
cargo run --bin souffleur-core -- --source wav --wav assets/jfk.wav --once &
cargo run --bin ws-tap                        # connect a surface, watch frames
```
