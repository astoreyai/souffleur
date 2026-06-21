//! Speech-to-text via `whisper-rs` (whisper.cpp bindings).
//!
//! Phase 0 uses whole-buffer transcription with per-segment timings. The live
//! daemon (Stage B) calls [`Stt::transcribe`] on rolling windows; a true
//! LocalAgreement sliding window is a Phase 1 refinement.

use anyhow::{Context, Result};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// Whisper emits bracketed/parenthesized non-speech markers such as
/// `[BLANK_AUDIO]`, `[ Silence ]`, or `(wind blowing)` when there is no speech.
/// Surfaces must not show these as utterances.
pub fn is_nonspeech(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return true;
    }
    (t.starts_with('[') && t.ends_with(']')) || (t.starts_with('(') && t.ends_with(')'))
}

/// A transcribed segment with whisper's internal timestamps (ms from buffer start).
#[derive(Debug, Clone)]
pub struct Segment {
    pub text: String,
    pub t0_ms: i64,
    pub t1_ms: i64,
}

/// Result of transcribing one audio buffer.
#[derive(Debug, Clone)]
pub struct Transcription {
    pub segments: Vec<Segment>,
    pub text: String,
}

/// A loaded whisper model. Cheap to clone-call: each [`transcribe`](Self::transcribe)
/// creates a fresh decode state so the model can be shared across worker threads.
pub struct Stt {
    ctx: WhisperContext,
    model_label: String,
    n_threads: i32,
}

impl Stt {
    /// Load a ggml/gguf whisper model from disk.
    pub fn load(model_path: &str, n_threads: i32) -> Result<Self> {
        // Route whisper.cpp/GGML's chatty C-level logging into the `log` facade.
        // With no log backend feature enabled this silences it (idempotent).
        whisper_rs::install_logging_hooks();
        let ctx = WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
            .with_context(|| format!("loading whisper model {model_path}"))?;
        let model_label = std::path::Path::new(model_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("whisper")
            .to_string();
        Ok(Self {
            ctx,
            model_label,
            n_threads,
        })
    }

    pub fn model_label(&self) -> &str {
        &self.model_label
    }

    /// Transcribe 16 kHz mono f32 PCM. Returns segments with timings and the
    /// concatenated text.
    pub fn transcribe(&self, audio_16k_mono: &[f32]) -> Result<Transcription> {
        let mut state = self.ctx.create_state().context("create whisper state")?;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_n_threads(self.n_threads);
        params.set_translate(false);
        params.set_language(Some("en"));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        state
            .full(params, audio_16k_mono)
            .context("whisper full inference")?;

        let n = state.full_n_segments().context("full_n_segments")?;
        let mut segments = Vec::with_capacity(n as usize);
        let mut text = String::new();
        for i in 0..n {
            let seg = state
                .full_get_segment_text(i)
                .context("full_get_segment_text")?;
            let trimmed = seg.trim();
            // whisper timestamps are in centiseconds (10 ms units).
            let t0 = state
                .full_get_segment_t0(i)
                .context("full_get_segment_t0")?
                * 10;
            let t1 = state
                .full_get_segment_t1(i)
                .context("full_get_segment_t1")?
                * 10;
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(trimmed);
            segments.push(Segment {
                text: trimmed.to_string(),
                t0_ms: t0,
                t1_ms: t1,
            });
        }
        Ok(Transcription { segments, text })
    }
}
