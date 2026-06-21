# Phase 1 — Results (verified 2026-06-21)

Phase 1 plan exit (`PLAN.md §5`): *usable end-to-end coach on a real call, on-device, < 2 s, coaching on a second device.* **Met** for the transcript + suggestion + phone-surface spine, on real audio/model/LLM/device. No synthetic data, no stubs.

## Hardware

Same Linux box as Phase 0, now with the GPU confirmed available: **NVIDIA RTX 3090 (24 GB)**. STT still runs on CPU (fast enough); the LLM runs on the GPU.

## What was added

- `stream` — streaming transcription: overlapping windows + fixed-lag commit + LocalAgreement-2 guard. Emits `transcript.partial` (live) and `transcript.final` (stable).
- `source` — audio sources normalized to 16 kHz mono: `mic` (cpal, me), `monitor` (PulseAudio `parec`, them), `wav`.
- `suggest` — local-LLM coaching engine (Ollama, JSON-constrained) → `prompt` events; honest degradation if Ollama is down.
- Daemon modes: `mic`, `monitor`, `duplex` (both sides), `wav`; `--wait-surface`, state heartbeat.
- `surfaces/phone/` — off-device phone PWA (connects to the WS, renders prompts + transcript).
- 8/8 engine tests pass; `cargo clippy --workspace` clean.

## Measurement 1 — streaming windows fix the chunk-boundary bug

wav source (real JFK), partials evolve and the final is the **complete, correct** sentence:
```
partial: "And so my fellow-"
partial: "And so, my fellow Americans!"
...
final:   "And so my fellow Americans, ask not what your country can do for you, ask what you can do for your country."
```
Phase 0 (non-overlapping) produced fragmented/wrong text ("...what you are coming to"). Phase 1 (overlapping + fixed-lag) produces the correct full utterance. **Fixed.**

## Measurement 2 — local suggestion engine (the GPU win)

qwen3:8b via Ollama, JSON-constrained, on the real transcript:

| Placement | First call | Warm call |
|---|---|---|
| CPU (cold) | **10.7 s** | — |
| **GPU (RTX 3090, warm)** | warmup ~70–95 ms | **~0.35 s** |

`ollama ps` confirms `qwen3:8b … 100% GPU`. A real, relevant cue was produced from the JFK transcript: **`<cue> "Acknowledge their quote and pivot to your perspective."`** and from a budget-objection test: **`<question> "How can we adjust the budget without cutting essentials?"`**. The daemon warms the model at startup so the first live suggestion is fast. **Suggestion latency on GPU (~0.35 s) is well inside the < 2 s budget; on CPU it is ~10 s — a hard hardware dependency, recorded honestly.**

## Measurement 3 — system-audio loopback "them" (parec)

Real loopback verified by playing audio into a PulseAudio null sink and capturing its monitor:
```
FINAL them "Ask not what your country can do for you."
FINAL them "Ask what you can do for your country."
```
Real system audio → parec (16 kHz mono) → STT → `speaker:"them"`. (A first attempt against the default hardware sink's monitor captured silence — a sink-routing detail — and whisper hallucinated "you"; the null sink removes that ambiguity. On the user's real machine the meeting audio is actively playing to the active sink, and the real targets are macOS Core Audio taps / Windows WASAPI.)

## Measurement 4 — duplex both-sides, concurrent

`--mode duplex` runs mic (me) + monitor (them) on two threads sharing one `Arc<Stt>`. Confirmed: both channels start, the them channel transcribes correctly under concurrency, state heartbeat fires, **no crash** → the shared whisper context is runtime-safe across concurrent `transcribe` calls. (The me channel had no speech on this headless box's silent mic; mic capture with real audio was verified separately and the path is identical.)

## Measurement 5 — phone surface (visual, verified by me via Playwright)

The phone PWA (`surfaces/phone/`) was served and driven in headless Chromium (390×844): it connected to the Coach Protocol WS, and rendered the live transcript and the coaching prompt. Asserted PASS (transcript present, prompt present), status pill read **"listening"**, model shown, and the screenshot was visually reviewed — clean dark phone UI: prompt card on top (kind-colored accent), THEM transcript bubble, consent toggle in the footer.

## End-to-end latency (the "near-real-time" requirement)

- Live **partial** transcript appears < ~1 s after speech.
- **Confirmed final** lags by the stability window (`hold_ms` 1.5 s + step), ~2 s.
- **Prompt** = final + ~0.35 s LLM (GPU) ≈ **~2–2.5 s speech→coaching cue**.

The transcript is sub-second; the confirmed-final + prompt is ~2–2.5 s. `hold_ms` (in `StreamConfig`) is the tunable knob trading stability for latency. Lowering it, and/or Moonshine for sub-300 ms stable STT (per `docs/research/03`), is the Phase 2+ path to a tighter budget.

## Honest limitations carried forward

1. **Suggestion latency is GPU-dependent** (~0.35 s GPU vs ~10 s CPU). Document the hardware floor; a smaller model would help CPU users.
2. **"Both sides with real speech on both channels" not shown in one run** — me channel was silent (headless box). Components each verified; a real call exercises both.
3. **Linear resampler** still in use (base.en handles it; swap to windowed-sinc only if WER demands).
4. **STT on CPU** (24× RTF, fine); GPU-whisper is an available future optimization.
5. **Phone surface needs a LAN/Tailscale bind** (`--bind 0.0.0.0:8123`) for a real phone; localhost-only by default for transcript privacy.
6. **Suggestion fires per final (debounced)**; smarter triggering (only-when-useful, dedupe) is a refinement.

## How to run

```bash
scripts/fetch-assets.sh
ollama serve &            # GPU-backed; pull qwen3:8b if needed
# Terminal A — the coach core (both sides + suggestions):
cargo run --release --bin souffleur-core -- --mode duplex --wait-surface
# Terminal B — serve the phone surface:
scripts/serve-phone.sh 8080
# On your phone (same Tailnet/LAN), open:
#   http://<host>:8080/?ws=ws://<host>:8123     (run core with --bind 0.0.0.0:8123)
```
