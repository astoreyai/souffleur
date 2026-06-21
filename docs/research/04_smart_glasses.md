# Smart Glasses for an Open-Source Real-Time AI Meeting Coach — Mid-2026 Research

Research date: 2026-06-21. Claims flagged **[CONFIRMED-DOCS]** (vendor docs / GitHub / store), **[CONFIRMED-PRESS]** (reputable third-party), **[COMMUNITY-RE]** (community reverse-engineered), or **[INFERRED]**. Two priors flipped this cycle: Even Realities now has an official SDK ("Even Hub," G2-only, April 2026) and Meta opened a *gated* display-output SDK (May 2026).

## 1. Glasses comparison

| Platform | Display tech | Resolution / FOV / color | Text capable? | Connectivity | SDK openness + language | Price | Push live text? |
|---|---|---|---|---|---|---|---|
| **Mentra / MentraOS** (OS, not hardware) | Abstraction over many devices | depends on glasses | Yes — `showTextWall()` | Cloud app → MentraOS Cloud (WS) → phone → glasses (BLE) | **MIT, fully open**, TS cloud SDK `@mentra/sdk` | SDK free; glasses $349+ | **Yes — easiest open path** |
| **Mentra Mach1** | Vuzix micro-LED waveguide | Mono green, monocular | Yes | BLE → phone → MentraOS | Via MentraOS (MIT) | ~$349 | Yes |
| **Brilliant Labs Frame** | Micro-OLED on prism | 640×400, 20° FOV, mono, ~16 colors/frame | Yes | **BLE direct** (phone OR PC) | **Open**; Python SDK MIT; PC via Bleak (Win/Mac/Linux) | $349 | **Yes — `display.show_text()` / `scroll_text()`** |
| **Brilliant Labs Halo** | Full-color micro-OLED HUD | 0.2", res unpublished, monocular | Yes | BLE direct | Open; Python + Flutter + Web Bluetooth; host text method names not yet doc'd | $349 ($299 pre-order) | Yes (API names unconfirmed) |
| **Even Realities G1** | JBD micro-LED + diffractive waveguide | 640×200, ~25° FOV, mono green, binocular, 20 Hz | Yes (native teleprompter) | Dual BLE | **No official SDK.** Community RE: `even_glasses` (Python, GPL) + MentraOS | $599 | **Yes — via MentraOS or RE `0x4E` Send Text** |
| **Even Realities G2** | Micro-LED + waveguide | G1-class | Yes | BLE → phone | **Official "Even Hub" SDK (Apr 2026)** — high-level, store-gated; `text-heavy` template | $599 | Yes (official, gated) OR via MentraOS |
| **Vuzix Z100** | Micro-LED waveguide | 640×480, 30° FOV, mono green, monocular | Yes (teleprompter/captions) | **BLE** → phone | **Free-but-licensed** Ultralite SDK (Android+iOS); `ScrollingTextView`, `sendText()`. Also MentraOS | $499 | **Yes — `ScrollingTextView` / via MentraOS** |
| **Vuzix Blade 2** | DLP/waveguide, full color | 480×480, 20° FOV | Yes | Standalone Android 11 | Full Android SDK; enterprise-locked | $799.99 | Yes (overkill) |
| **XREAL One / One Pro** | Birdbath micro-OLED | 1080p/eye, 50–57° FOV, full color | Yes (as a monitor) | **USB-C DisplayPort** | **No SDK needed** — render any window | $399 / $599 | **Yes — zero SDK, monitor latency** |
| **Rokid Max 2** | Birdbath micro-OLED | 1080p/eye, 50° FOV, color | Yes (as monitor) | **USB-C DisplayPort** | No SDK needed | ~$429 | **Yes — zero SDK** |
| **RayNeo X3 Pro** | Full-color micro-LED waveguide | 640×480/eye, ~30° FOV | Yes (native teleprompter) | Standalone Android + BLE/WiFi | Android SDK + Unity OpenXR + ADB | $1,299 | Yes (sideload) |
| **RayNeo Air 3s** | Birdbath micro-OLED | 1080p/eye, color | Yes (as monitor) | **USB-C DisplayPort** | No SDK needed | ~$199 | **Yes — zero SDK** |
| **INMO Air3** | Full-color micro-OLED waveguide | 1080p, 36° FOV, binocular | Yes | Standalone Android 14 | Unity SDK + Play Store + ADB | $1,099 | Yes; **battery ~60–90 min** |
| **Meta Ray-Ban Display** | Reflective (Lumus) waveguide | 600×600, ~20° FOV, color, monocular | Yes | Phone app + EMG Neural Band | **Gated dev preview (May 2026)**: Swift/Kotlin + Web Apps (HTML/JS); no public publishing | $799 | Prototype yes; **distribute no** (≤100 testers) |
| **Halliday** | Glance-up micro-LED module | Mono green, tiny | Native teleprompter only | BLE + ring | **No SDK** (closed appliance) | $499 | **No** |

## 2. Recommended first target

**Target MentraOS first** (Vuzix Z100 or Even Realities G1 as reference glasses), **with a Brilliant Labs Frame Python track** for a fully-open, PC-direct, no-cloud reference.

- MentraOS is MIT, purpose-built, abstracts G1/G2/Z100/Mach1 behind one TypeScript API; native `TRANSCRIPTION` stream in + `showTextWall()` out; drives the closed G1 via the RE protocol so you don't maintain it.
- Frame is the only platform that is fully open AND drivable directly from a PC over BLE in Python (`display.show_text()` / `scroll_text()`), no cloud, $349.
- **Even Hub (G2)** = sanctioned but store-gated, hides raw BLE. **Meta Ray-Ban Display** = technically possible (Web Apps path) but no public distribution yet. **Drop Halliday** (no SDK, display too small) and all display-less devices.

## 3. Integration architecture

**Pattern A — phone-as-hub via MentraOS (primary):** glasses (mic+HUD) --BLE--> phone (MentraOS app: STT, BLE, display I/O) --WS--> MentraOS Cloud --WS--> your coach app (TS, `@mentra/sdk`, extends `AppServer`): subscribe `TRANSCRIPTION` stream → run coaching/LLM → `session.layouts.showTextWall(prompt)`. Display primitives: `showTextWall`, `showDoubleTextWall`, `showReferenceCard`, `showDashboardCard`, `clear()` (layouts **replace in place** — ideal teleprompter). **Rate limit: 1 update / 300 ms** — coalesce to ≤3 updates/sec. Query `display.maxTextLines` and paginate.

**Pattern B — PC-direct via Frame (fully open, no cloud):** PC (Python, Bleak BLE central) → Frame (BLE peripheral, Lua OS). Your STT + coaching on meeting audio, then `frame.display.show_text()` / `write_text(text,x,y)` / `scroll_text()` → `show()`. No phone, no cloud, no rate-limit middleman; you own audio capture + STT.

**Zero-SDK escape hatch:** XREAL One ($399) / Rokid Max 2 / RayNeo Air 3s ($199) appear as a DisplayPort external monitor — render an always-on-top teleprompter window, monitor-level latency. Trade-off: bulky, tethered, flat 2D screen — fine for desk/demo, wrong for discreet in-meeting use.

## 4. Realistic limitations

- **Text real estate:** discreet text glasses are mono-green, low-res, narrow-FOV (G1 640×200/25°, Z100 640×480/30°, Frame 640×400/20°). Design for ~1–5 short glanceable lines, self-paginated, never paragraphs. Exact chars/lines per screen is **undocumented on every device — must be measured on hardware**.
- **Refresh/latency:** G1 panel 20 Hz; MentraOS 300 ms display floor. No vendor publishes end-to-end ms; dominant latency is STT + LLM, not the BLE hop. **Measure on real hardware before locking UX.**
- **Battery:** HUD glasses last hours (Z100 ~2-day standby); INMO Air3 ~60–90 min screen-on (too short for long meetings); RayNeo X3 Pro weak.
- **Social acceptability:** Even Realities G1/G2 look like ordinary glasses, no camera ("Quiet Tech") — most socially invisible. Z100 (38 g) glasses-like. Birdbath glasses look like bulky sunglasses — poor for discreet use.
- **Openness risk:** G1 RE path may break on firmware update; Meta distribution-blocked + single-vendor-gated; Even Hub store-gated. **MentraOS and Frame are the only no-gatekeeper paths.**

**Bottom line:** Build the coach as a **MentraOS TypeScript cloud app** (one app → Z100/G1/G2/Mach1, transcript stream + `showTextWall` provided, MIT, no gatekeeper); keep a **Brilliant Labs Frame Python track** for a fully-open PC-direct no-cloud reference. Most-want-to-verify-on-hardware: exact chars/lines per screen and end-to-end text latency on the chosen green HUD.
