# whispercoach (working name)

Open-source, **local-first, privacy-preserving real-time conversation coach**: captures live meeting audio, transcribes it on-device, and surfaces short AI-generated prompts (facts to recall, questions to ask, objection handling, next-line cues) in a glanceable overlay — with optional extension to a screen-share-excluded desktop window and to text-display smart glasses.

> **Status:** Planning. This repo currently holds research and the architecture plan. No application code yet.
> **Privacy/git:** local-only repository (no remote). Do not push without explicit decision. No real recordings, models, or secrets are committed (see `.gitignore`).

## Why this exists (intended use)

Real-time retrieval-not-recall support for:
- **Accessibility / memory accommodation** — surfacing names, facts, and threads you'd otherwise lose under pressure.
- **Meetings** — live notes, action-item capture, "what was the number we agreed on?"
- **Sales / customer calls** — objection handling, product facts, next-best-question.
- **Public speaking / teleprompting** — discreet cue cards on glasses.

This is a dual-use category. The plan treats **consent, recording law, and venue policy as first-class design constraints** rather than afterthoughts — see `docs/plan/` (ethics & consent section). It is not built to defeat exam proctoring or interview honesty policies; the screen-share-exclusion feature exists for legitimate privacy of your own coaching notes during a presentation, and the plan documents that boundary.

## Layout

```
docs/
  research/   — competitive + technical landscape research (cited)
  plan/       — the architecture & implementation plan
```

## Naming

`whispercoach` is a working directory name (the reference product is WhisperCoach.io). The public open-source name is TBD — candidate codenames tracked in `docs/plan/`.
