# Technical Building Blocks: Audio Capture · Streaming STT · LLM Suggestion · Invisible Overlay — Research

Compiled 2026-06-21. **[C]** confirmed-from-docs, **[I]** inferred. Latency figures from vendor marketing are flagged as unverified.

## 1. Audio capture (both sides of a meeting)

**macOS:**
- **Core Audio process taps** (`AudioHardwareCreateProcessTap`, macOS 14.4+) — capture a specific process's audio; **lighter, non-re-prompting permission** than screen recording. Used by Hyprnote, Screenpipe, Natively. **Recommended default.** [C]
- **ScreenCaptureKit audio** (macOS 13+; in-stream mic macOS 15+) — robust but pulls the screen-recording TCC permission. [C]
- **BlackHole / aggregate devices** — virtual driver; requires user install; **BlackHole is GPL-3.0 / paid-for-commercial** (licensing caveat). [C]
- Channel separation: mic = "me", process-tap/loopback = "them" → **free diarization, zero added latency**. [C]

**Windows:** **WASAPI loopback** captures the system render endpoint (everything played). Per-process loopback documented from **build 20348**. No special permission for basic loopback. [C]

**Linux:** PulseAudio/PipeWire **monitor sources** expose loopback of any sink. [C]

**Browser:** `getDisplayMedia({audio:true})` (tab/system audio, user picks, gesture-gated) or extension `chrome.tabCapture`. Limited to the browser tab; Zoom/Teams desktop apps invisible to it. [C]

**Meeting-bot (Recall.ai-style):** a bot joins as a participant → no client install, all platforms, clean per-speaker streams; but **visible in participant list** (consent surface) and a recurring cost. [C]

## 2. Streaming / near-real-time STT

**Local (licenses precise):**
| Engine | License | Latency / note |
|---|---|---|
| **whisper.cpp** | MIT | sliding-window streaming; **stable text ~2–3.5 s** (LocalAgreement floor), not 0.2 s |
| **faster-whisper** (CTranslate2) | MIT | engine layer; **no native streaming** (wrappers add it) |
| **WhisperLive / WhisperLiveKit** | MIT / Apache-2.0 | streaming + diarization (Sortformer/pyannote) |
| **RealtimeSTT** (KoljaB) | MIT | full mic→VAD→incremental→callback loop |
| **Moonshine** | MIT | **34 ms (Tiny) / 107 ms (Medium) TTFT** on MacBook; Medium **6.65% WER beats Whisper-large-v3 7.44%** at 245M params — strongest local low-latency, only truly sub-300 ms stable path |
| **NVIDIA Parakeet-TDT** | CC-BY-4.0 | **offline/batch** (RTFx is throughput, NOT latency) — right for *post-meeting* transcript only |
| **NVIDIA Canary-1b** | **CC-BY-NC** | **non-commercial — product blocker**; Canary-flash/qwen are CC-BY-4.0 |
| **NVIDIA FastConformer streaming** (`stt_en_fastconformer_hybrid_large_streaming_multi`) | CC-BY-4.0 | NVIDIA's actual *streaming* answer: 0/80/480/1040 ms modes |
| **Vosk** | Apache-2.0 | lightweight, lower accuracy |

**Critical correction:** UFAL `whisper_streaming` LocalAgreement has a **~1.9 s algorithmic floor / ~3.3 s real average** → whisper.cpp/WhisperLive give *stable* text in ~2–3.5 s, not sub-second. **Moonshine is the only local path with truly sub-300 ms stable output.** UI implication: render **confirmed (solid) vs provisional (greyed) text**; fire coaching logic only off the **confirmed prefix**.

**Cloud streaming (exact pricing):**
- **AssemblyAI Universal-Streaming** — **$0.15/hr** ($0.0025/min), **immutable transcripts** (no flicker/revision logic) → architecturally cleanest for a coach. [C]
- **Deepgram Nova-3** — **$0.0048/min ≈ $0.29/hr**; diarization +$0.0020/min. [C]
- **OpenAI Realtime / gpt-4o-transcribe**, **Google/Azure streaming** — **publish no streaming latency number** (treat ~300 ms claims as marketing). [C/I]

**Diarization:** channel separation (mic vs loopback) is free for "me vs them." Splitting *individual remote* speakers needs acoustic diarization on the loopback channel only — **Streaming Sortformer** (CC-BY-4.0, 0.32 s, DER 4.88% CH109) or **diart** (MIT, CPU-capable, 500 ms–5 s).

## 3. Low-latency LLM suggestion layer

- **Local:** llama.cpp / Ollama with small models (e.g. Llama-3.x-8B, Qwen-2.5-7B). **GBNF grammar-constrained** decoding (Hyprnote pattern) makes a small local model reliably emit structured prompts. Best privacy; ~hundreds-of-ms to low-seconds on a decent GPU.
- **Cloud:** Claude / GPT / Gemini streaming — higher quality, lower local compute; adds network RTT + sends transcript off-device (consent/privacy cost).
- **Latency techniques:** incremental transcript windowing (feed only the rolling last N turns); fire generation off the **confirmed** STT prefix (not provisional); stream tokens to the display; cache stable context (resume, product facts, prior turns) so each call is a short delta; debounce so you generate at most ~1 suggestion / few-hundred-ms.

## 4. Screen-share-invisible overlay (cross-platform)

- **Windows:** `SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE=0x11)` — **works** (build 19041+); window absent from capture (DWM-level). [C]
- **macOS:** `NSWindow.sharingType=.none` — **broken on macOS 15+** (ScreenCaptureKit ignores it; Apple: "no public APIs for preventing screen capture"). Works only ≤macOS 14 against legacy capturers. [C]
- **Electron `setContentProtection(true)` / Tauri `contentProtected:true`** map to the two above (so inherit Windows-yes / macOS15+-no). [C]
- **Defeated by:** OS full-screen recording on macOS 15+, second camera at the screen, hardware capture cards / HDMI splitters, some proctoring stacks. [C]

## Recommended stacks

**Local-first privacy MVP (default):**
- Capture: macOS Core Audio process taps + mic / Windows WASAPI loopback + mic (Rust cpal or native).
- STT: **Moonshine** (sub-300 ms stable) for the live coaching prefix; whisper.cpp for the durable transcript.
- Diarization: channel separation (free) + optional diart on loopback.
- LLM: local llama.cpp/Ollama, GBNF-constrained short prompts.
- Latency budget (approx): capture ~20–50 ms → Moonshine STT ~100–300 ms → local LLM ~300–900 ms → render ~16–50 ms = **~0.5–1.3 s speech→prompt** (Moonshine path). With whisper.cpp the STT term becomes ~2–3.5 s.

**Max real-time cloud stack:**
- Capture: same local capture.
- STT: **AssemblyAI Universal-Streaming** (immutable, $0.15/hr) or Deepgram Nova-3.
- LLM: Claude / GPT / Gemini streaming.
- Latency budget: capture ~20–50 ms → cloud STT ~300 ms + RTT → cloud LLM first-token ~300–800 ms + RTT → render = **~0.8–1.8 s**, higher quality, transcript leaves device.

**Most-likely-wrong claims to re-verify:** cross-vendor latency figures from aggregators/marketing; exact macOS 15 point release behavior; Windows 20348 per-process loopback; OpenAI/Google unpublished streaming latency.
