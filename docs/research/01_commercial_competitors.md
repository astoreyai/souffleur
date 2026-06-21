# Commercial Landscape: Real-Time AI Coaching / Live Meeting Copilot — Mid-2026 Research

Research date: 2026-06-21. Claims tagged **[C]** confirmed-from-source or **[I]** inferred. Prices point-in-time (several vendor pages JS-gated → some figures from dated 2026 third-party reviews).

## 1. WhisperCoach.io deep-dive

- **What [C]:** "AI-Powered Interview Copilot." Upload resume + job description; connect phone to laptop; during a live video interview the AI listens and structured talking points appear **on your phone** while the laptop runs the meeting. (https://whispercoach.io/)
- **Target [C]:** Job seekers in live virtual interviews. Interviews only — not sales/meetings/exams.
- **Stealth model [C, key nuance]:** Does **not** use "invisible/undetectable/stealth/won't appear on screen share" anywhere readable. Its stealth is purely **architectural** — a **"Seamless Two-Device Setup"** ("connect laptop and phone in under 60 seconds"): coaching lives on a physically separate screen the interviewer never sees. No screen-capture-exclusion claim because coaching is never on the captured device. This is the most important differentiator from Cluely/Final Round/LockedIn: it **sidesteps the screen-share-detection arms race entirely** via a second device. Trade-off [I]: exposed to in-person proctoring or a webcam catching phone-glances.
- **Features [C]:** real-time coaching, response-recovery prompts, post-interview insights, AI follow-up email, session history.
- **Real-time [C]:** uses "real-time" language, **no latency number / SLA**.
- **Platforms [C]:** "Works with Zoom, Meet, Teams & more." Client is just a smartphone browser — **no native desktop app, no extension, no glasses/hardware**.
- **Pricing [C]:** One Shot $12 (single session) / Sprint Mode $25 (72h unlimited, "Most Popular") / Full Prep $34 (30-day unlimited). Consumption-based, not subscription.
- **One-line read:** low-price, interview-only, second-device prompter; competes on simplicity + price, not the technical overlay-invisibility claim.

## 2. Competitor matrix

### Group A — covert / "undetectable" candidate-side tools

| Product | Use case | "Real-time" (claim → observed) | Stealth claim | Pricing | Platform | Controversy |
|---|---|---|---|---|---|---|
| **WhisperCoach.io** | Interviews | "real-time," no number | No "undetectable" claim; two-device (phone) | $12 / $25 / $34 | Phone browser only | None found (small) |
| **Cluely** (Roy Lee) | "cheat on everything" → meeting assistant | reviewers: off/clunky/out-of-sync | "Completely hidden to meeting screen sharing" — gated to **$149.99/mo** tier | Free; Pro $19.99/mo; **Pro+Undetectability $149.99/mo**; Ent ~$200/mo | Desktop (Mac/Win)+iOS/Android | Columbia suspension; pivoted off "cheat"; CEO admitted faking ~$7M ARR (real ~$5.2M) Mar 2026 |
| **Final Round AI** | coding+behavioral | "instant"; **~7–8s observed** | "Undetectable on Zoom/Teams/Meet"; "Stealth Mode" — **native app only; Chrome ext NOT invisible** | Free; ~$41.67/mo annual → $149/mo | Native app (Mac/Win)+ext | Proctoring vendor **Talview** publishes a "Stop FinalRound AI" blocking page |
| **Interview Coder** (Roy Lee orig.) | live coding | "in seconds"; ~<10s | "100% Invisible to Screen-Recording"; "20+ undetectability features" | Free; **$299/mo** or **$799 lifetime** | Desktop overlay only | THE expulsion story → became Cluely |
| **LockedIn AI** | coding+behavioral+**sales+meetings** | **"116ms" claim; 4–5s observed** | "only visible to you, even when screen sharing" | ~$30–55/mo; lifetime ~$1,499 | Native overlay+Chrome+human "Duo" | Detection vendor **Sherlock** publishes detect-LockedIn guide |
| **Sensei AI** | behavioral(+coding PRO) | **"<1s" claim**; ~0.3–1.7s | "Fully undetectable" — **caught in 2/5 test screen-shares** (browser-only) | Free; PRO $89/mo or $24/mo annual | Browser/Chrome only | detectable in proctored/screen-share |
| **TechScreen.app** | technical interviews | "real-time" | "Zero Detection Incidents" | Free; $32/$47/$80 per mo | Native (Mac/Win) | TOS disclaims hidden-interview use (liability shield); note `techscreen.com` is the OPPOSITE (cheating-detection) |

### Group B — overt enterprise sales/meeting tools (visible bot, NOT stealth)

| Product | Use case | Live in-call help? | Stealth? | Pricing | Controversy |
|---|---|---|---|---|---|
| **Gong** | B2B sales | **No — post-call** | No (visible/API, consent-first) | ~$1.2K–2.4K/user/yr; median ~$54,900/yr | wiretapping-category risk |
| **Chorus** (ZoomInfo) | B2B sales | No — post-call | No (visible Notetaker bot) | ~$8K/yr/3 seats | ZoomInfo paid ~$30M to settle a wiretap suit (Community Ed.) |
| **Attention** | B2B sales | **Yes — genuine live battlecards** | No (overlay on rep's own screen) | ~$100–500/mo quote-based | $14M Series A; no privacy scandal |
| **Second Nature** | sales **training** | No — role-play simulator | N/A | ~$30–40/user/mo | $22M Series B; none |
| **Otter.ai** | meetings → sales | **Yes — 2025 Sales Agent live coaching** | No (visible participant) | Free; Pro $8.33/mo; Business $19.99 | **In re Otter.AI Privacy Litigation** (CIPA/ECPA, N.D. Cal. 2025) — the bellwether |
| **Read AI** | meetings | live analytics/self-coaching | No (visible + chat disclosure) | Free; Pro $15/mo; Ent $22.50 | **University of Washington blocked Read AI** (Jan 2025) |
| **Fireflies.ai** | meetings + sales CI | mostly post-call; 2025 Live Assist | No ("always visible in participant list") | Free; Pro $10; Business $19; Ent $39 | **Cruz v. Fireflies** (Dec 2025, Illinois BIPA) |

## 3. Stealth-technology summary

- **Core mechanism is a real OS feature [C]:** always-on-top transparent click-through window excluded from the capture buffer *before* the meeting app sees it. Windows `SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)` (flag 0x11, Win10 v2004 build 19041+, enforced at DWM level → absent from BitBlt/DXGI/Windows.Graphics.Capture). macOS `NSWindow.sharingType = .none`; ScreenCaptureKit honored per-window filters (≤macOS 14). (https://adamsvoboda.net/how-interview-cheating-tools-hide-from-zoom/)
- **Where it breaks [C]:** browser-based tools get NO protection (Sensei caught 2/5; Final Round ext visible); full-monitor/QuickTime/kernel/GPU capture bypasses per-window exclusion; nothing stops a webcam-of-screen or a proctor; proctoring (Proctorio/Honorlock) is a different threat model (screen+webcam+gaze+audio wake-words, Turnitin AI detection). Detection products now exist (Talview, Sherlock). "Undetectable" is an active arms race.
- **Hardware/glasses [C]:** none of the matrix products ship glasses. Cluely's *launch video* featured glasses but the product is desktop+mobile. Standalone AI-glasses (Halliday $489, Even Realities G2 $599 "Conversate") are separate hardware. **Original-brief "glasses integration" = category confusion** — the claim most likely to be wrong.

## 4. Ethical / legal landscape

- **Group A (covert candidate tools):** academic/employment-integrity + fraud framing. Precedent: Interview Coder → Roy Lee Columbia suspension, rescinded big-tech offers; Google leaning back to in-person interviews. Vendors hedge (TechScreen TOS vs homepage; Cluely scrubbed "cheat" branding). Counter-industry mature (Talview/Sherlock/Fabric; Fabric: AI-interview cheating doubled 15%→35% Jun→Dec 2025).
- **Group B (overt enterprise):** wiretapping / two-party-consent litigation wave. **In re Otter.AI** (ECPA+CIPA+CCDAFA, bellwether); **Cruz v. Fireflies** (BIPA). **11 all-party-consent states** (CA, CT, FL, IL, MD, MA, MI, MT, NH, PA, WA). Emerging **"capability test"**: if the vendor *can* store/train on recordings, every recording may be a wiretap; courts probing **employer vicarious liability**. "Visible bot in participant list" is **not, by itself, sufficient consent.**

## 5. Gaps an open-source alternative could fill

1. **Honest, consent-first real-time coaching** — the market splits into covert cheating tools (legal/ethical landmine) and overt-but-post-call enterprise tools; a live, *disclosed* copilot is underserved.
2. **Local / on-device inference for privacy** — every Group-B tool is under fire because audio leaves the device; on-device STT + local LLM structurally dodges the CIPA capability test.
3. **Transparent latency benchmarks** — no vendor publishes a real SLA; claims (116ms/<1s) diverge from observed (4–8s).
4. **Price disruption** — stealth is paywalled punitively ($149.99/mo Cluely, $299/mo or $799 lifetime Interview Coder, ~$1,499 LockedIn lifetime); OSS + BYO-key collapses this to inference cost.
5. **Cross-platform second-device architecture done right** — WhisperCoach's phone-as-second-screen is elegant but closed/interview-only/thin.
6. **Legitimate adjacent markets the stealth framing poisons** — accessibility (memory/processing accommodation), language learners, public-speaking/filler-word coaching, sanctioned interview practice.

**Sharpest gap:** a privacy-preserving, **on-device, disclosed** real-time coach with published latency and zero cloud-recording — dodging the Group-B wiretapping wave *and* sidestepping the Group-A detection arms race the whole commercial market is trapped between.
