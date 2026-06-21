# Souffleur

> *A souffleur is the theatre prompter who, unseen from the audience, whispers an actor's next line.*

Open-source, **local-first, privacy-preserving real-time conversation coach**: captures live meeting audio, transcribes it on-device, and surfaces short AI-generated prompts (facts to recall, questions to ask, objection handling, next-line cues) on a surface that is **off the shared screen** — your phone, smart glasses, or an un-shared monitor — with a best-effort desktop overlay as a secondary option.

- **License:** AGPL-3.0 (see `LICENSE`) — prevents closed-SaaS forks of the project.
- **Inference:** local-only by default (audio never leaves the machine); cloud STT/LLM is opt-in, BYO-key, and consent-gated.
- **Core stack:** Rust / Tauri from day one (cross-platform desktop core + overlay); polyglot surfaces behind the Coach Protocol seam.

> **Status:** Planning complete; implementation not started. This repo holds research + the architecture plan. No application code yet.
> **Privacy/git:** local-only repository (no remote). Do not push without an explicit decision. No real recordings, models, or secrets are committed (see `.gitignore`).

## Why this exists (intended use)

Real-time retrieval-not-recall support for:
- **Accessibility / memory accommodation** — surfacing names, facts, and threads you'd otherwise lose under pressure.
- **Meetings** — live notes, action-item capture, "what was the number we agreed on?"
- **Sales / customer calls** — objection handling, product facts, next-best-question.
- **Public speaking / teleprompting** — discreet cue cards on glasses.

This is a dual-use category. The plan treats **consent, recording law, and venue policy as first-class design constraints** (see `docs/plan/PLAN.md §7`). Default mode is **disclosed + on-device**. It is **not** built to defeat exam proctoring or interview-honesty policies; the optional desktop overlay's screen-capture exclusion exists for legitimate privacy of your own coaching notes during a presentation you are giving, and the plan documents that boundary honestly (including that it is Windows-only and broken on macOS 15+).

## Layout

```
docs/
  research/   01 commercial · 02 open-source · 03 audio/STT/overlay · 04 smart glasses  (all cited)
  plan/       PLAN.md  — architecture, latency budget, phased roadmap, decisions log
LICENSE       AGPL-3.0
```

## Key design decision

Both "must not be visible on screen-share" and "smart glasses as an extension" are satisfied by the **same** move: put the coaching surface **off the shared machine**. Same-machine screen-capture exclusion is a losing, platform-dependent arms race (works on Windows, broken on macOS 15+, none on Linux). See `docs/plan/PLAN.md §0`.
