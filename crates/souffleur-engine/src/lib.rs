//! Souffleur core engine.
//!
//! Phase 0 exposes the speech-to-text path ([`stt`]), small audio helpers
//! ([`resample`]), and live capture ([`audio`]). The `souffleur-core` binary
//! wires capture -> STT -> Coach Protocol over a WebSocket; `latency-harness`
//! measures the STT path on real audio; `ws-tap` is a minimal client surface.

pub mod audio;
pub mod resample;
pub mod stt;
