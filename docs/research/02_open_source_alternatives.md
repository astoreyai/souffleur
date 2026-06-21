# Open-Source "Real-Time AI Meeting Copilot / Cluely-Style Invisible Overlay" — Research

Compiled 2026-06-21. Star counts are same-day GitHub snapshots. **CONFIRMED** = read in repo/source/docs; **INFERRED** = reasoned from secondary evidence.

The space splits on two axes: (1) **purpose** — covert assistant (Cluely clones) vs meeting notetaker (Otter/Granola alternatives); (2) **audio capture** — OS system-audio loopback · mic-only · meeting-bot-joins · browser-caption-scraping · screenshot-only (no audio).

## 1. Project-by-project

### A. Cluely-style covert overlays (invisibility is the point)

| Project | Repo | ★ | License | Stack | Audio | STT | LLM | Invisibility | Status |
|---|---|---|---|---|---|---|---|---|---|
| **Glass (Pickle)** | `pickle-com/glass` | 7.5k | GPL-3.0 (after Apache violation) | Electron+Next.js+Rust AEC | system+mic, Rust echo-cancel | cloud or local | OpenAI/Gemini/Claude/Ollama BYO | `setContentProtection` all windows | Stale (2025-10) |
| **cheating-daddy** | `sohzm/cheating-daddy` | 5.4k | GPL-3.0 | Electron (JS) | macOS `SystemAudioDump` binary; Win `getDisplayMedia` loopback | model-side (Gemini Live) | Gemini Live + local BYO | `setContentProtection(true)` + skipTaskbar | Active (2026-06) |
| **Pluely** | `iamsrikanthnani/pluely` | 2.2k | GPL-3.0 | **Tauri** (Rust+React), ~10MB | system+mic | pluggable BYO (Whisper/Deepgram/Groq) | OpenAI/Claude/Gemini/Ollama BYO | `contentProtected:true` + macOS NSPanel | Active (2026-01) |
| **CodeInterviewAssist** | `j4wg/interview-coder-withoupaywall-opensource` | ~1.9k | AGPL-3.0 | Electron+React | none (screenshot) | none | OpenAI/Gemini BYO | `setContentProtection(true)` | most-forked OSS ref |
| **Natively** | `Natively-AI-assistant/...` | 1.5k | AGPL-3.0 | Electron + Rust audio (napi-rs)+SQLite | CoreAudio Tap / SCK / WASAPI | **local on-device Whisper (ONNX)** | Gemini/OpenAI/Claude/Groq/Ollama BYO | `setContentProtection` + process mask | **Active (2026-06-21)** |
| **free-cluely** | `Prat011/free-cluely` | 1.5k | Apache-2.0 | Electron | microphone | none (audio→Gemini) | Gemini 2.0 + Ollama BYO | `setContentProtection(true)` | Active |
| **Aura-AI** | `Rkcr7/Aura-AI` | 24 | MIT | **Python** (FastAPI+pywebview+ctypes), Win | system+mic | **Deepgram streaming** | Cerebras/Groq/Gemini BYO | raw `SetWindowDisplayAffinity` 0x11 | Active |
| **Clonely** | `tahahah/Clonely` | 12 | MIT | Electron + native .node | system (`desktopCapturer`) | **Deepgram nova-3** | Gemini | `WDA_EXCLUDEFROMCAPTURE` (Win-only) | Dormant |

Long-tail (≤40★): `GetEzzi/ezzi-app`, `inulute/phantom-lens` (exam), `vivekvar-dl/wingman-AI`, `Thairu-dev/StealthIt` (Python WDA 0x11), `cybertheory/ghostclaw` (Tauri), `bnovik0v/ghst` (Linux/PipeWire, honestly admits overlay visible on Wayland). **Non-functional, ignore:** `nwx77/cheap-cluely` (only translucent bg) and `programmingTomato/ScrewCodingAssessments` (only WS_EX_TOOLWINDOW) — both appear in recordings.

### B. Meeting transcribers / local notetakers (no invisibility)

| Project | Repo | ★ | License | Stack | Audio | STT | LLM | Live? |
|---|---|---|---|---|---|---|---|---|
| **Screenpipe** | `mediar-ai/screenpipe` | ~19.4k | source-available | Tauri (Rust) | system+mic loopback, event-driven screen+OCR | Whisper Large-V3-Turbo local / Deepgram | Ollama / OpenAI-compatible | record-then-recall |
| **Meetily** | `Zackriya-Solutions/meeting-minutes` | ~12.8k | MIT (open-core) | Tauri | simultaneous mic+system, **not a bot** | Whisper + Parakeet local | Ollama default + Claude/Groq/OpenAI | **live-first** |
| **Hyprnote/Anarlog** | `fastrepl/hyprnote`→`anarlog` | ~8.7k | MIT | Tauri (Rust+Swift) | macOS Core Audio process taps, Win WASAPI loopback | whisper.cpp + pluggable | **on-device llama.cpp + GBNF-constrained** | live |
| **Vibe** | `thewh1teagle/vibe` | ~6.5k | MIT | Tauri | macOS SCK, Win cpal | whisper.cpp offline | Claude/Ollama summaries | preview+batch |
| **OpenWhispr** | `openwhispr/openwhispr` | ~3.9k | MIT | Electron | mic + auto-detect Zoom/Teams/FaceTime | whisper.cpp + Parakeet local | GPT/Claude/Gemini/local BYO | both; on-device voice-fingerprint diarization |
| **Amurex** | `thepersonalaicompany/amurex` | ~2.8k | AGPL-3.0 | Chrome ext + Python | **none** — reads Meet/Teams page captions (no Zoom) | platform captions | OpenAI/Groq/Mistral/Ollama | live; stale |
| **Vexa** | `Vexa-ai/vexa` | ~2.2k | Apache-2.0 | TS+Python microservices, K8s | **MEETING BOT** — Playwright Chromium joins as attendee | WhisperLive → large-v3-turbo | transcripts-only | real-time streaming |

## 2. Best reference implementations

**Covert overlay:** **Natively** (most engineered — real test suite incl. `SetContentProtectionDedupe.test.mjs`; Rust audio abstracting CoreAudio Tap/SCK/WASAPI; local ONNX Whisper; local RAG; `electron/WindowHelper.ts` calls `setContentProtection`). **Pluely** (cleanest Tauri, `contentProtected:true`, NSPanel, pluggable BYO STT+LLM). **cheating-daddy** (canonical macOS `SystemAudioDump` recipe: CoreAudio-tap CLI, 24kHz mono, 100ms PCM chunks). **Aura-AI** (most explicit raw Win32 `SetWindowDisplayAffinity` `0x11`).

**Capture/transcription engine:** **Hyprnote/Anarlog** (local-first: lock-free ring buffers, platform-abstracted Core Audio taps/WASAPI/PulseAudio, on-device llama.cpp with **GBNF grammar-constrained** structured output). **Screenpipe** (event-driven capture cut storage ~6-7×; SQLite+FTS5 multimodal index). **Vexa** (meeting-bot reference: one Playwright bot per meeting in a container, Redis-hot/Postgres-cold, REST+WebSocket).

**Low-latency local STT to embed:** **whisper.cpp** streaming (`--step 500 --length 5000` sliding window, Silero VAD, q5 quant, MIT). **RealtimeSTT** (KoljaB) for Python mic→VAD→incremental-text→callback. **WhisperLiveKit** (`QuentinFuxa/...`, 10.5k★, Apache-2.0) for SimulStreaming/AlignAtt + streaming Sortformer diarization.

## 3. Screen-share invisibility — techniques + limits (THE critical finding)

Every project uses one of three equivalent flags; **Electron `setContentProtection(true)` maps to** Windows `SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)` + macOS `NSWindow.sharingType = NSWindowSharingNone`. Tauri `contentProtected:true` resolves to the same.

- **Windows `WDA_EXCLUDEFROMCAPTURE` (0x11) — WORKS [C].** Win10 v2004 (build 19041)+. Window "does not appear at all" in captured frames (vs `WDA_MONITOR` black box). DWM-level → covers Desktop Duplication / Windows.Graphics.Capture; honored by Zoom/Teams/Meet share, OBS display capture, Snipping Tool, PrintScreen, **Windows Recall**. NOT a security boundary (defeatable by code injection / camera). (https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity)
- **macOS `sharingType = .none` — BROKEN on macOS 15+ [C] (most important finding).** On ≤14 it excluded the window from legacy `CGWindowListCreateImage`. On **macOS 15 (Sequoia, incl. 15.4+) ScreenCaptureKit deliberately ignores it** — Quartz composites all windows before SCK reads, so the overlay **is captured anyway**. Apple DTS: "there are no public APIs for preventing screen capture." Modern Zoom/Teams/Meet/QuickTime/OBS all ride ScreenCaptureKit → **the macOS invisibility trick no longer works on current macOS.** `SCContentFilter` exclusions are the *capturer's* tool, not the overlay's. (https://developer.apple.com/forums/thread/792152)
- **Linux/Wayland:** no per-window exclude API at all (ghst README admits the overlay is always visible).

**Compatibility matrix:**

| Mechanism | Platform | Hides from OS recording? | Hides from Zoom/Teams/Meet share? |
|---|---|---|---|
| `WDA_EXCLUDEFROMCAPTURE` | Win10 2004+/11 | **Yes** | **Yes** |
| `sharingType=.none` | macOS ≤14 | Yes (legacy APIs) | Yes (if legacy capturer) |
| `sharingType=.none` | **macOS 15+** | **No** | **No** |
| Electron `setContentProtection` | Win / macOS15+ | Win Yes / macOS **No** | Win Yes / macOS **No** |
| Any | Any | **No** vs 2nd camera / capture card / HDMI splitter | **No** |

**Always defeats it:** a second camera at the screen, hardware capture cards / HDMI splitters, and (macOS 15+) any ScreenCaptureKit recorder.

## 4. Audio-capture models (robustness order)

1. **OS system-audio loopback + mic (most robust):** macOS Core Audio process taps (Hyprnote/Screenpipe/Natively) or BlackHole/aggregate+sox; Windows WASAPI loopback or Chromium `getDisplayMedia`/`desktopCapturer`; cross-platform Rust **cpal** (Vibe). Captures any app + in-person, fully private, no bot. Cost: per-OS native code + macOS TCC permission.
2. **Mic-only (simplest, weakest):** misses the other speaker unless looped through speakers.
3. **Meeting bot (Vexa):** no client install, scales, but **visible participant**, web-meetings only, GPU-heavy.
4. **Browser caption-scraping (Amurex):** zero audio plumbing, but Zoom unsupported, brittle.
5. **Screenshot-only (interview-coder family):** no STT, useless for spoken conversation.

## 5. What's MISSING (the gap a new OSS project fills)

1. **Nobody solved macOS 15+ invisibility — and most don't admit it's broken.** A genuinely valuable contribution = **rigorous, version-matrixed disclosure** of what hides where (per OS build × per capturer), not marketing copy.
2. **Covert overlays have weak STT; good STT lives in the notetakers.** No project combines a polished display surface with best-in-class local streaming STT + live diarization. (solveWatchAi gestures at it at 6★.)
3. **Fully-local end-to-end is rare.** Only Natively (local ONNX Whisper) + a few BYO-Ollama. A **100% on-device** assistant (local STT + local llama.cpp, GBNF-constrained, no audio leaving the machine) is open.
4. **License hygiene is a mess** (5+ "OSS" repos ship no license; Glass↔cheating-daddy GPL→Apache relicensing scandal). A cleanly-licensed, properly-attributed reference has value.
5. **No cross-platform parity** (Windows works; macOS broken on 15+; Wayland no mechanism). Nobody shipped a credible "this platform cannot hide, here's why" UX.
