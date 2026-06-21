# Souffleur — Architecture & Implementation Plan

Date: 2026-06-21. Project name: **Souffleur**.
Source research: `docs/research/01..04`. This plan synthesizes those four cited landscape reports.

**Settled decisions (2026-06-21):** License **AGPL-3.0** · Inference **local-only default, cloud opt-in (BYO-key, consent-gated)** · Core **Rust/Tauri from day one** · Name **Souffleur**. See §10 for the log.

---

## 0. The one insight that drives the whole design

Two of your requirements — **"must not be visible on screen-share"** and **"smart glasses as an extension"** — converge on a *single* architectural decision:

> **Put the coaching surface OFF the shared machine.**

This matters because the popular alternative — an OS "screen-capture-excluded" overlay window on the *same* machine — is a **losing, platform-dependent arms race** (research `02 §3`, `03 §4`, both **CONFIRMED**):

| Stealth approach | Windows | macOS 15+ (current) | Linux/Wayland | vs full-screen capture | vs 2nd camera / capture card | vs proctoring |
|---|---|---|---|---|---|---|
| Same-machine window exclusion (`setContentProtection`) | ✅ works (build 19041+) | ❌ **broken** (SCK ignores it) | ❌ no API | ❌ loses | ❌ loses | ❌ loses |
| **Off-device surface** (phone / glasses / un-shared 2nd monitor) | ✅ | ✅ | ✅ | ✅ | ❌ (webcam can still see you glance) | partial |

WhisperCoach.io itself already made this call: its "stealth" is a **two-device setup** (coaching on your phone), not a capture-exclusion hack, and it never claims "undetectable" (research `01 §1`, **CONFIRMED**). The off-device surface is the robust path **and** it *is* the glasses requirement. So we make it primary, and treat the same-machine overlay as a **best-effort secondary** shipped with an honest per-OS disclosure rather than a marketing "100% invisible" claim.

**Design consequence:** the system is a **local "coach core" that emits a stream of short prompts**, plus **interchangeable thin "surfaces"** that subscribe to that stream. The seam between them is a stable protocol. This one boundary is what lets meetings + phone + glasses + overlay all coexist and lets us swap STT/LLM backends without touching surfaces.

---

## 1. Goals & non-goals

**Goals**
- Capture **both sides** of a live meeting's audio locally.
- Produce **near-real-time** coaching prompts (target **< 2 s** speech → on-surface prompt; stretch < 1 s on the local Moonshine path).
- Deliver prompts to a surface that is **not on the shared screen** (phone PWA / smart glasses / un-shared monitor), with a same-machine overlay as a labeled best-effort extra.
- **Local-first / on-device by default** (no audio leaves the machine) — the privacy posture that structurally avoids the wiretapping/BIPA exposure crushing the cloud incumbents (research `01 §4`).
- Work with **Zoom / Teams / Meet / Webex / in-person** (capture is app-agnostic system-audio, not a meeting bot).
- **Smart glasses as a first-class surface** (MentraOS primary; Brilliant Labs Frame fully-open reference).

**Non-goals (explicit boundaries)**
- Not built to defeat **exam proctoring** or **interview-honesty policies**. The same-machine overlay's capture-exclusion exists for legitimate privacy of *your own notes during a presentation you're giving*, and ships documented as such (the plan does not chase the Proctorio/Honorlock arms race — research `01 §3`).
- No covert recording of others without disclosure. Default mode is **disclosed + on-device**; a consent banner/notice helper is built in (§7).
- Not a meeting *notetaker* product (Otter/Granola already own that). Live coaching is the point; durable transcript is a byproduct.

**Intended legitimate use** (drives the framing): accessibility / **memory-accommodation** (retrieval-not-recall support), sales-call objection handling, public-speaking cue cards, language-learner support, sanctioned interview *practice*.

---

## 2. System architecture

```
                         ┌─────────────────────────────────────────────┐
                         │                COACH CORE (local)            │
   meeting / room        │                                              │
   audio  ──mic────────► │  Capture     ─► STT (streaming) ─► Diarize   │
          ──loopback───► │  (both sides)     confirmed/provisional      │
                         │       │                                │     │
                         │       └─────────► durable transcript    ▼     │
                         │                    + RAG context  ►  Suggestion│
                         │                    (resume / product   engine │
                         │                     facts / history)   (LLM)  │
                         └───────────────────────┬──────────────────────┘
                                                 │  Coach Protocol
                                   (JSON events over localhost/LAN WebSocket; BLE for glasses)
                ┌──────────────────────┬─────────┴────────┬───────────────────────┐
                ▼                      ▼                  ▼                       ▼
        Phone PWA surface      Smart-glasses surface   Desktop overlay      (future) Watch /
        (2nd device — the      (MentraOS app /         (Tauri, Windows      earbud TTS surface
         robust "invisible")    Frame Python)           real-invisible;
                                                        macOS/Linux labeled)
```

**The seam — "Coach Protocol":** a tiny JSON-over-WebSocket event stream the core publishes and any surface consumes. This is the single most important engineering decision; it makes surfaces polyglot (glasses = TypeScript, phone = web, overlay = Rust/Tauri) and backends swappable. Draft event shapes:
- `transcript.partial` `{speaker, text, t}` — provisional (greyed on surfaces)
- `transcript.final` `{speaker, text, t}` — confirmed prefix; coaching fires off this
- `prompt` `{kind: "fact"|"question"|"objection"|"cue"|"recover", text, ttl, priority}`
- `state` `{capturing, latency_ms, model, consent_disclosed}`

---

## 3. Recommended technology stack

Each pick leads with the recommendation, then the alternative it beats and why (grounded in research, not preference).

### 3.1 Audio capture — **OS system-audio loopback + mic, native per-OS**
- macOS: **Core Audio process taps** (`AudioHardwareCreateProcessTap`, macOS 14.4+) — lighter, non-re-prompting permission than screen-recording TCC (research `03 §1`, **CONFIRMED**; used by Hyprnote/Screenpipe/Natively). Fallback ScreenCaptureKit audio.
- Windows: **WASAPI loopback** + mic.
- Linux: **PipeWire/PulseAudio monitor source** + mic.
- **Free diarization:** mic channel = "me", loopback channel = "them" — zero added latency (research `03 §2`).
- Beats: meeting-bot capture (Vexa-style) — robust and no-install, but **visible in the participant list** and a per-meeting cost (research `02 §4`); keep as an optional Phase-4 backend, not the default. Beats browser `getDisplayMedia` — can't see the Zoom/Teams *desktop* app.

### 3.2 Streaming STT — **Moonshine (local) for the live coaching prefix; whisper.cpp for the durable transcript**
- **Moonshine** (MIT) is the only local engine with **truly sub-300 ms stable output** (34–107 ms TTFT; Medium 6.65% WER beats Whisper-large-v3's 7.44% at 245M params) — research `03 §2`, **CONFIRMED**. This is what makes "near-real-time" real.
- whisper.cpp / WhisperLive carry a **~2–3.5 s LocalAgreement stable-text floor** — fine for the durable transcript, too slow as the *sole* live path. **Critical UI rule:** render confirmed (solid) vs provisional (greyed) text; fire coaching only off the **confirmed prefix**.
- Avoid as live engines: **Parakeet/Canary headline models are offline/batch** (throughput, not latency); **Canary-1b is CC-BY-NC = commercial blocker** (research `03 §2`). NVIDIA's real streaming answer is FastConformer-streaming (CC-BY-4.0) if a non-Moonshine local path is wanted.
- Cloud option (Phase 4, BYO-key): **AssemblyAI Universal-Streaming** ($0.15/hr, **immutable transcripts** → no flicker logic) is architecturally cleanest; Deepgram Nova-3 (~$0.29/hr) for built-in diarization.

### 3.3 Suggestion engine (LLM) — **local llama.cpp / Ollama, GBNF-constrained, default; cloud BYO-key optional**
- Local small model (Llama-3.x-8B / Qwen-2.5-7B) with **GBNF grammar-constrained decoding** (Hyprnote pattern, research `02 §2`) so a small model reliably emits structured `prompt` events. Keeps audio + transcript on-device.
- Cloud (Claude / GPT / Gemini, BYO-key) as an opt-in quality/latency tier — but it sends transcript off-device, so it's gated behind explicit consent UI.
- Latency techniques (research `03 §3`): rolling last-N-turns window; generate off the confirmed prefix; stream tokens to the surface; cache stable context (resume / product facts / prior turns) so each call is a short delta; debounce to ≤ ~1 suggestion / few-hundred-ms.

### 3.4 Surfaces
- **Phone PWA (the robust "invisible")** — a tiny web app the phone browser opens on the LAN; mirrors WhisperCoach's two-device model, the only durably screen-share-safe path on every OS.
- **Smart glasses** — **MentraOS** TypeScript cloud app (MIT; one app → Even Realities G1/G2, Vuzix Z100, Mentra Mach1; gives a `TRANSCRIPTION` stream in and `showTextWall()` out; 300 ms display floor → coalesce to ≤3 updates/s) as primary; **Brilliant Labs Frame** Python/BLE (`display.show_text`, PC-direct, no cloud) as the fully-open reference (research `04 §2–3`). Design for **1–5 short glanceable lines**, self-paginated.
- **Desktop overlay** — Tauri window with `contentProtected:true`. Real invisibility on **Windows (build 19041+)**; on **macOS 15+ it does NOT hide** and on **Linux there's no API** — the UI states this honestly per-OS (research `02 §3`).

### 3.5 Core language — **Rust / Tauri from day one** (settled decision)
- Goes straight to the right distribution target — the strongest references all chose it: Hyprnote/Anarlog, Pluely, Screenpipe, Meetily, Vibe are all **Tauri/Rust** (research `02 §1–2`). One toolchain covers the cross-platform audio core (cpal / native CoreAudio taps / WASAPI / PipeWire), the overlay surface (`contentProtected`), and packaging.
- **Accepted trade-off:** no throwaway Python prototype, so the latency/UX loop is validated *in* the production stack rather than ahead of it. **Mitigation — make the latency harness the very first Phase-0 deliverable** so the viability question (can the local Moonshine path hit < 1.3 s on real hardware?) is answered before the rest of the core is built on top.
- Crate/tooling starting points: `cpal` (cross-platform audio I/O), `tauri` v2 (shell + overlay + `contentProtected`), `tokio` + `tokio-tungstenite` (Coach Protocol WebSocket), a Rust binding to whisper.cpp/Moonshine (FFI or `whisper-rs`) for STT, and `llama.cpp`/Ollama over HTTP for the suggestion engine. Glasses (MentraOS) and phone (PWA) surfaces stay TypeScript/web behind the protocol seam — unaffected by the core language.

---

## 4. Latency budget (the "near-real-time" requirement)

| Stage | Local (Moonshine) | Local (whisper.cpp) | Cloud |
|---|---|---|---|
| Capture | 20–50 ms | 20–50 ms | 20–50 ms |
| STT (stable) | 100–300 ms | **2000–3500 ms** | ~300 ms + RTT |
| Suggestion LLM (first token) | 300–900 ms | 300–900 ms | 300–800 ms + RTT |
| Render to surface | 16–50 ms (overlay/phone); ≤300 ms (glasses floor) | same | same |
| **Speech → prompt** | **~0.5–1.3 s** ✅ | ~2.5–4.5 s ⚠️ | ~0.8–1.8 s |

Target **< 2 s**; the **Moonshine local path is the only way to hit < 1.3 s fully on-device.** Ship a **reproducible latency harness** (speech→prompt) as a first-class artifact — no commercial competitor publishes one (research `01 §5`), so it's a credibility wedge.

---

## 5. Phased roadmap

- **Phase 0 — Seam + capture spike (Rust).** Write the Coach Protocol spec. Rust core: capture both sides (mic + loopback, via `cpal`/native taps) on the primary dev OS, emit `transcript.partial/final` over the WebSocket. **Build the latency harness first** and confirm the local Moonshine path hits its target on real hardware. *Exit:* both-sides transcript visible with a measured speech→text latency number from the harness.
- **Phase 1 — MVP coach (local, phone surface).** ✅ **Spine done** (see `PHASE1_RESULTS.md`): overlapping streaming windows (fixes chunk-boundary errors), system-audio loopback "them" via parec + duplex both-sides, local suggestion engine (Ollama qwen3:8b, ~0.35 s on the RTX 3090 GPU vs ~10 s CPU), and the off-device phone PWA (Playwright-verified). *Exit met* for the transcript+suggestion+surface spine; ~2–2.5 s speech→cue. Carry-forward: Moonshine for sub-300 ms stable STT, smarter suggestion triggering, real-speech both-sides on a real call.
- **Phase 2 — Desktop overlay surface.** Tauri overlay with `contentProtected` + honest per-OS invisibility disclosure (Windows real; macOS/Linux labeled). *Exit:* overlay works on a Windows screen-share with documented limits.
- **Phase 3 — Smart glasses.** MentraOS TS surface consuming the protocol (or its `TRANSCRIPTION` stream); Frame Python reference. *Exit:* live prompts on real glasses; measure on-hardware chars/line + end-to-end latency (the two undocumented unknowns, research `04 §4`).
- **Phase 4 — Cloud tier + meeting-bot backend (both opt-in, BYO-key).** AssemblyAI/Deepgram STT + Claude/GPT/Gemini suggestions behind consent gating; optional Vexa-style bot for no-install web meetings. *Exit:* a max-real-time/quality mode and a zero-install mode, both clearly consent-gated.
- **Phase 5 — Polish.** Acoustic diarization for multiple remote speakers (Streaming Sortformer CC-BY-4.0 / diart MIT); RAG over user docs (resume / product facts); published latency benchmarks; consent tooling; cross-platform packaging.

---

## 6. Reference implementations to study (don't reinvent)

From research `02 §2`: **Natively** (most-engineered covert clone; local ONNX Whisper; test suite) · **Pluely** (cleanest Tauri/`contentProtected` reference) · **Hyprnote/Anarlog** (local-first audio engine + GBNF-constrained local LLM) · **cheating-daddy** (canonical macOS `SystemAudioDump` audio recipe) · **Aura-AI** (explicit raw Win32 `WDA_EXCLUDEFROMCAPTURE`) · **whisper.cpp / RealtimeSTT / Moonshine / WhisperLiveKit** (STT) · **MentraOS / Frame SDK** (glasses). **License caution:** Glass/cheating-daddy/Pluely are **GPL-3.0**, Natively/CodeInterviewAssist **AGPL-3.0** — *study, don't copy* unless we adopt a compatible license (§10 / open decision).

---

## 7. Ethics, consent & legal posture (first-class, not a footnote)

- **Default = disclosed + on-device.** Audio never leaves the machine in the default config → structurally sidesteps the CIPA "capability test" sinking Otter/Fireflies (research `01 §4`, **CONFIRMED** litigation).
- **Consent helper built in:** one-click "announce assistant" / join-time notice text, and a jurisdiction note for the **11 all-party-consent states** (CA/CT/FL/IL/MD/MA/MI/MT/NH/PA/WA) and BIPA voiceprint risk.
- **Honest invisibility labeling:** the overlay never claims "100% undetectable"; it shows a per-OS truth table. This is itself a differentiator — the research's #1 identified gap is that *"nobody solved macOS 15+ invisibility and most don't admit it's broken"* (research `02 §5`).
- **Documented non-use for proctored exams / honesty-policy interviews.** README + first-run notice.

> Note for the maintainer: this is a dual-use category. The plan deliberately positions toward the *disclosed, accessibility-first, on-device* market that the stealth framing poisons (research `01 §5`), and away from the cheating-tool arms race that got Interview Coder's author expelled. That positioning is a product decision worth confirming before public release.

---

## 8. Key risks & open unknowns

1. **macOS same-machine invisibility is unsolvable on 15+** (Apple: "no public APIs for preventing screen capture"). *Mitigation:* off-device surface is primary; overlay labeled. **CONFIRMED.**
2. **Glasses chars/line + true end-to-end latency are undocumented on every device** — must be measured on real hardware in Phase 3 (research `04 §4`).
3. **Local latency depends on Moonshine** delivering its published numbers on the user's hardware — verify on the latency harness in Phase 0/1 before committing the UX (claim is vendor-published; **INFERRED** for our hardware).
4. **Legal positioning** of a dual-use tool — confirm the disclosed/accessibility framing before any public repo.
5. **License contamination** if we fork GPL/AGPL references — keep clean-room or adopt a compatible license (§10).

---

## 9. Immediate next actions (Phase 0)

1. Confirm the four open decisions in §10.
2. Write `docs/plan/COACH_PROTOCOL.md` (event schema).
3. Stand up the Python core skeleton + both-sides capture on the dev OS + latency harness.
4. Choose and pin the license.

---

## 10. Decisions log (settled 2026-06-21)

1. **License → AGPL-3.0.** Prevents a closed-SaaS fork (the exact move Cluely/Interview Coder pulled), fits the anti-paywalled-stealth ethos, compatible with studying the AGPL references. (`LICENSE` added.) *Considered:* Apache-2.0 (max adoption but permits closed forks), GPL-3.0 (copyleft without the network clause).
2. **Default inference → local-only, cloud opt-in (BYO-key, consent-gated).** The on-device privacy posture is the product's whole legal/ethical edge. *Considered:* cloud-default (best latency/quality, but reintroduces the wiretapping/BIPA exposure).
3. **Core stack → Rust/Tauri from day one.** Straight to the distribution target the best references all use; accepted trade-off is validating latency/UX in the production stack, mitigated by building the latency harness first (see §3.5). *Considered:* Python MVP → Rust port (faster to validate, but throwaway).
4. **Name → Souffleur.** The theatre prompter who whispers an actor's lines — on-theme, avoids the WhisperCoach trademark and the OpenAI-Whisper collision. Working directory renamed `whispercoach` → `souffleur`. *Considered:* Sotto, Cue/Cuepilot, Aside, Murmur.
