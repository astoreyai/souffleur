# Phase 4 — Results (verified 2026-06-21)

Phase 4 plan exit (`PLAN.md §5`): *an optional cloud tier for stronger suggestions, opt-in and consent-gated, that never weakens the local-first default.* **Met.** Local Ollama stays the default and keeps audio on the machine; a cloud LLM is reachable only behind two locks (a startup `--allow-cloud` flag and a runtime consent disclosure), and the transcript is provably not transmitted until both are satisfied. Real backends, no stubs.

## What was added

- `SuggestBackend` trait (`suggest.rs`): `name`, `is_cloud`, `check`, `warmup`, `complete(system, transcript, cfg) -> (raw_json, latency_ms)`. One seam, four real implementations:
  - **`OllamaBackend`** (local, default) — `/api/chat`, JSON-constrained. `is_cloud() == false`.
  - **`GeminiBackend`** — `generateContent`, `x-goog-api-key`, `responseMimeType: application/json`.
  - **`AnthropicBackend`** — `/v1/messages`, `x-api-key` + `anthropic-version: 2023-06-01`.
  - **`OpenAiBackend`** — `/v1/chat/completions`, `response_format: json_object`.
- `make_backend(kind, model)` + `backend_is_cloud(kind)`; daemon flags `--suggest-backend <local|gemini|claude|openai>`, `--suggest-model`, `--allow-cloud`.
- The privacy chokepoint: `SuggestionEngine::suggest_gated(session_ms, consent_disclosed)` returns an empty result (makes no network call) when the backend is cloud and consent has not been disclosed. Enforced by a unit test (`cloud_backend_refuses_to_transmit_without_consent`) using a `PanicCloud` backend whose `complete` panics if ever called.

## The Gemini thinking-model fix (load-bearing)

`gemini-2.5-flash` is a thinking model: by default it spends the entire output budget on internal thoughts and returns `finishReason: MAX_TOKENS` with empty content, so no JSON ever arrives. The fix is `generationConfig.thinkingConfig.thinkingBudget = 0`, which disables thinking for this short JSON task. With it, responses come back `finishReason: STOP` with clean JSON. This is in `GeminiBackend::complete` and is the difference between the backend working and silently producing nothing.

## Measurement 1 — the consent gate (verified live this session)

Cloud backend refused at startup without the opt-in flag:
```
$ souffleur-core --mode wav --wav assets/jfk.wav --suggest-backend gemini
Error: --suggest-backend gemini is a CLOUD backend — it sends the live transcript off this machine.
Re-run with --allow-cloud to opt in. ...
```
With `--allow-cloud` and a surface that discloses consent, the daemon log shows the transcript is transmitted only after the disclosure, never before:
```
[suggest] backend gemini ready (warm in 0 ms)
[suggest] CLOUD backend — transcript is sent off-device ONLY after consent is disclosed
[ws] 127.0.0.1:58740 set consent disclosed=true
[suggest] 0 prompt(s) in 529 ms     <- the only backend call, AFTER consent
```
No backend call appears before the `set consent disclosed=true` line. The runtime ordering matches the unit-test guarantee.

## Measurement 2 — Gemini produces real prompts, and its empties are honest

Driving the real daemon (`--suggest-backend gemini --allow-cloud`) on the JFK fixture, the Gemini call returns a valid, parseable `{"prompts":[...]}` object. The model's output is non-deterministic (temperature 0.4), and for a single closing quote it often, correctly, decides nothing needs coaching. Five direct calls on the exact daemon transcript (`THEM: And so my fellow Americans, ...`):
```
trial 1: finish=STOP prompts=0  {"prompts":[]}
trial 2: finish=STOP prompts=0  {"prompts":[]}
trial 3: finish=STOP prompts=0  {"prompts":[]}
trial 4: finish=STOP prompts=1  {"prompts":[{"kind":"fact","text":"JFK's inaugural address.","priority":3}]}
trial 5: finish=STOP prompts=0  {"prompts":[]}
```
This is the system prompt working as designed ("return `{"prompts":[]}` when nothing helps"). When the model does suggest, the daemon renders it: an earlier end-to-end daemon run this arc produced the `fact` cue `"JFK's inaugural address."` in 671 ms. The daemon's "0 prompt(s)" is a faithful pass-through of a principled empty array, not a parse failure (a parse error propagates as an `Error`, it does not log as zero prompts).

## What is NOT key-verified (honest)

- **Anthropic and OpenAI backends** are real implementations (correct endpoints, headers, auth, JSON-mode request), but no `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` is present in this environment, so they are **not** live-verified end-to-end. They are BYO-key: set the key, pass `--suggest-backend claude|openai --allow-cloud`. The request shapes follow each provider's documented contract; first live use is the user's to confirm with a key.
- **Cloud STT (Deepgram / AssemblyAI)** is **deferred**, not built. It is a per-channel WebSocket-streaming path that bypasses the local committer entirely (a different ingest seam from the suggestion tier) and needs a provider key to verify. Documented as future work rather than stubbed.

## Status

Cloud suggestion tier complete and gated. Gemini verified end-to-end this session (gate refusal, consent-then-transmit ordering, valid parsed output, real prompt on the same path). Anthropic/OpenAI are real and BYO-key. Local Ollama remains the default and the only path that keeps audio fully on-device. 22 tests pass across the workspace; `cargo clippy --workspace --all-targets -D warnings` and `cargo fmt --check` clean.
