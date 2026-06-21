//! Souffleur core engine.
//!
//! - [`stt`]: whisper.cpp speech-to-text.
//! - [`resample`]: downmix + sample-rate conversion helpers.
//! - [`audio`]: live cpal microphone capture.
//! - [`source`]: audio sources (mic / system-audio monitor / wav) as 16 kHz mono streams.
//! - [`stream`]: streaming transcription with overlapping windows + fixed-lag commit.
//! - [`suggest`]: local-LLM coaching-prompt engine (Ollama).
//!
//! The `souffleur-core` binary wires sources -> streaming STT -> suggestion engine
//! -> Coach Protocol over a WebSocket; `latency-harness` measures the STT path;
//! `ws-tap` is a minimal client surface.

pub mod audio;
pub mod resample;
pub mod source;
pub mod stream;
pub mod stt;
pub mod suggest;
