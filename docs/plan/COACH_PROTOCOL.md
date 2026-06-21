# Coach Protocol v0

The seam between the **core** (capture → STT → diarize → suggestions) and any **surface** (phone PWA, smart glasses, desktop overlay). One stable contract; everything on either side is swappable.

- **Transport:** newline-delimited JSON (NDJSON) frames over a WebSocket. Core is the server; surfaces connect as clients. Default bind `127.0.0.1:8123` (localhost-only; never 0.0.0.0 without an explicit opt-in flag, because the stream carries live transcript).
- **Direction:** mostly core → surface (events). Surfaces may send a small set of control messages back (e.g. `ack`, `set_consent`). A surface that only renders can ignore the uplink entirely.
- **Versioning:** every frame carries `v` (protocol major). v0 is pre-stable; breaking changes bump `v`. Unknown event `type`s must be ignored by a surface, not error.
- **Time:** `t` is milliseconds since capture-session start (monotonic), not wall-clock — keeps surfaces clock-skew-free and makes latency math trivial.

## Core → surface events

Every frame: `{ "v": 0, "type": <string>, "t": <int ms>, ... }`.

### `transcript.partial`
Provisional text for an utterance still being revised. Surfaces render it **greyed/italic**. Replaces the previous partial for the same `utterance_id`.
```json
{ "v":0, "type":"transcript.partial", "t":1840, "utterance_id":"u12", "speaker":"them", "text":"so the number we landed on was" }
```

### `transcript.final`
Confirmed, immutable prefix. Surfaces render it **solid**. **The suggestion engine fires only off `final` events** (never off partials) — this is the rule that keeps coaching stable despite STT revision (see `docs/research/03 §2`).
```json
{ "v":0, "type":"transcript.final", "t":2010, "utterance_id":"u12", "speaker":"them", "text":"So the number we landed on was forty-two thousand.", "stt_latency_ms":260 }
```
- `speaker`: `"me"` | `"them"` | `"them:<n>"` (n-th remote speaker once acoustic diarization is on). In Phase 0, channel separation gives `me` (mic) vs `them` (loopback) for free.
- `stt_latency_ms`: measured audio-end → text time for this segment (populated by the latency harness and the live core).

### `prompt`
A short coaching cue to display. Surfaces show 1–N depending on real estate (glasses: 1–5 short lines, self-paginated).
```json
{ "v":0, "type":"prompt", "t":2120, "prompt_id":"p7", "kind":"objection", "text":"Anchor to ROI: payback ~4 months at their volume.", "ttl_ms":12000, "priority":3, "source_utterance":"u12" }
```
- `kind`: `"fact"` | `"question"` | `"objection"` | `"cue"` | `"recover"` | `"note"`.
- `ttl_ms`: how long to keep showing it before fading. `priority`: 1(low)–5(urgent); surfaces with one slot show the highest.

### `state`
Heartbeat + status. Drives the surface's status pill and the consent indicator.
```json
{ "v":0, "type":"state", "t":3000, "capturing":true, "model":"whisper-base.en", "e2e_latency_ms":910, "consent_disclosed":false, "surfaces":1 }
```

### `error`
Non-fatal core-side problem the surface should show (device lost, model load failed, cloud key rejected).
```json
{ "v":0, "type":"error", "t":4200, "code":"audio_device_lost", "message":"capture device card2 disappeared", "fatal":false }
```

## Surface → core control (optional uplink)

```json
{ "v":0, "type":"set_consent", "disclosed":true }          // user announced the assistant
{ "v":0, "type":"dismiss", "prompt_id":"p7" }              // user dismissed a prompt
{ "v":0, "type":"hint", "text":"focus on pricing" }        // steer the suggestion engine
{ "v":0, "type":"ack" }                                     // keepalive
```

## Design rules (load-bearing)

1. **Partials are cosmetic; finals are causal.** Only `transcript.final` triggers suggestions. Prevents flicker-driven bad prompts.
2. **The core never renders.** It only emits events. All layout/pagination/length-fitting lives in the surface (a glasses surface paginates to its line count; an overlay wraps to its width).
3. **Localhost by default.** The transcript stream is sensitive; binding beyond loopback requires BOTH `--listen-lan` AND a shared secret (`--token` / `$SOUFFLEUR_TOKEN`), which the core checks at the WebSocket handshake (surfaces connect with `ws://HOST:PORT/?token=<secret>`). A bare non-loopback `--bind` is refused.
4. **Surfaces are stateless-friendly.** A surface can connect mid-session and render correctly from the next `state` + subsequent events; it must not depend on having seen the whole history.
5. **Backpressure:** the core coalesces — at most ~3 `prompt`/s and ~1 `state`/s; a glasses surface further coalesces to the device's 300 ms display floor.

## Phase 0 subset

The core emits `transcript.partial`, `transcript.final` (with real `stt_latency_ms`), `prompt` (suggestion engine), `state` (heartbeat with real `capturing` + `consent_disclosed`), and `error` (e.g. `audio_device_lost`, `suggest_unavailable`). On the uplink, the core consumes `set_consent` (drives the `consent_disclosed` reported in `state`); `dismiss`/`hint`/`ack` are accepted and logged but not yet acted on.
