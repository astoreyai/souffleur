//! Streaming transcription with overlapping windows and fixed-lag commit.
//!
//! Phase 0 cut the audio into non-overlapping windows, which sliced words at
//! window boundaries ("what you are coming to" instead of "what your country").
//! Here we keep an uncommitted rolling buffer and re-transcribe it as new audio
//! arrives, so every word is decoded with the future context around it. A
//! segment is committed (emitted as `transcript.final`) only once it sits behind
//! a `hold` lag from the live edge AND the same text appeared in the previous
//! run over the same time region (LocalAgreement-2). Committed audio is trimmed
//! off the front using whisper's segment timestamps; the still-moving tail is
//! emitted as `transcript.partial`.

use crate::stt::{is_nonspeech, Segment, Stt};
use souffleur_protocol::{Event, Speaker, PROTOCOL_VERSION};
use std::sync::Arc;
use std::time::Instant;

/// Tuning for the streaming committer. Defaults target near-real-time stability.
#[derive(Debug, Clone)]
pub struct StreamConfig {
    /// Re-transcribe once this much new audio has arrived since the last run.
    pub step_ms: u64,
    /// Don't commit anything newer than this from the live edge (stability lag).
    pub hold_ms: u64,
    /// Force-commit and reset if the uncommitted buffer grows past this.
    pub max_buf_ms: u64,
    /// Don't run inference until the buffer holds at least this much audio.
    pub min_infer_ms: u64,
    /// Time tolerance when matching a segment against the previous run (LA-2).
    pub agree_tol_ms: u64,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            step_ms: 700,
            hold_ms: 1500,
            max_buf_ms: 14_000,
            min_infer_ms: 1000,
            agree_tol_ms: 500,
        }
    }
}

const SR: usize = 16_000;

fn ms_of(samples: usize) -> u64 {
    (samples as u64 * 1000) / SR as u64
}

/// Per-channel streaming transcriber. One instance per speaker channel.
pub struct StreamingStt {
    stt: Arc<Stt>,
    speaker: Speaker,
    cfg: StreamConfig,
    buf: Vec<f32>,
    base_ms: u64,
    new_since_infer: usize,
    prev_segments: Vec<Segment>,
    last_partial: String,
    utt: u32,
}

impl StreamingStt {
    pub fn new(stt: Arc<Stt>, speaker: Speaker, cfg: StreamConfig) -> Self {
        Self {
            stt,
            speaker,
            cfg,
            buf: Vec::new(),
            base_ms: 0,
            new_since_infer: 0,
            prev_segments: Vec::new(),
            last_partial: String::new(),
            utt: 0,
        }
    }

    /// True if `text` appeared at ~`t1` in the previous transcription (LA-2).
    fn agreed_in_prev(&self, text: &str, t1: u64) -> bool {
        let want = text.trim();
        self.prev_segments.iter().any(|s| {
            s.text.trim() == want && (s.t1_ms as u64).abs_diff(t1) <= self.cfg.agree_tol_ms
        })
    }

    fn next_utt_id(&mut self) -> String {
        self.utt += 1;
        format!("{}-{}", self.speaker, self.utt)
    }

    /// Feed newly captured 16 kHz mono audio. Returns any Coach Protocol events
    /// to publish (zero or more), stamping each with `session_ms`.
    pub fn push(&mut self, samples: &[f32], session_ms: u64) -> Vec<Event> {
        self.buf.extend_from_slice(samples);
        self.new_since_infer += samples.len();
        let mut out = Vec::new();

        let dur = ms_of(self.buf.len());
        if dur < self.cfg.min_infer_ms || ms_of(self.new_since_infer) < self.cfg.step_ms {
            return out;
        }
        self.new_since_infer = 0;

        let started = Instant::now();
        let tr = match self.stt.transcribe(&self.buf) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("[stream:{}] transcribe error: {e:#}", self.speaker);
                return out;
            }
        };
        let stt_ms = started.elapsed().as_millis() as u64;

        let cut = dur.saturating_sub(self.cfg.hold_ms);
        let mut commit_text = String::new();
        let mut commit_t1 = 0u64;
        let mut pending = String::new();

        for seg in &tr.segments {
            let t = seg.text.trim();
            if is_nonspeech(t) {
                continue;
            }
            let stable = (seg.t1_ms as u64) <= cut && self.agreed_in_prev(t, seg.t1_ms as u64);
            if stable {
                if !commit_text.is_empty() {
                    commit_text.push(' ');
                }
                commit_text.push_str(t);
                commit_t1 = seg.t1_ms as u64;
            } else {
                if !pending.is_empty() {
                    pending.push(' ');
                }
                pending.push_str(t);
            }
        }
        self.prev_segments = tr.segments;

        if !commit_text.is_empty() {
            let id = self.next_utt_id();
            out.push(Event::TranscriptFinal {
                version: PROTOCOL_VERSION,
                t: session_ms,
                utterance_id: id,
                speaker: self.speaker.clone(),
                text: commit_text,
                stt_latency_ms: Some(stt_ms),
            });
            // Trim the committed audio off the front.
            let cut_samples = (commit_t1 as usize * SR) / 1000;
            let cut_samples = cut_samples.min(self.buf.len());
            self.buf.drain(..cut_samples);
            self.base_ms += commit_t1;
            self.last_partial.clear();
        }

        let pending = pending.trim().to_string();
        if !pending.is_empty() && pending != self.last_partial {
            out.push(Event::TranscriptPartial {
                version: PROTOCOL_VERSION,
                t: session_ms,
                utterance_id: format!("{}-p{}", self.speaker, self.utt + 1),
                speaker: self.speaker.clone(),
                text: pending.clone(),
            });
            self.last_partial = pending;
        }

        if ms_of(self.buf.len()) > self.cfg.max_buf_ms {
            out.extend(self.flush(session_ms));
        }
        out
    }

    /// Commit everything still buffered (end of stream, or a long-silence boundary).
    pub fn flush(&mut self, session_ms: u64) -> Vec<Event> {
        let mut out = Vec::new();
        if self.buf.is_empty() {
            return out;
        }
        if let Ok(tr) = self.stt.transcribe(&self.buf) {
            let text: Vec<&str> = tr
                .segments
                .iter()
                .map(|s| s.text.trim())
                .filter(|t| !is_nonspeech(t))
                .collect();
            if !text.is_empty() {
                let id = self.next_utt_id();
                out.push(Event::TranscriptFinal {
                    version: PROTOCOL_VERSION,
                    t: session_ms,
                    utterance_id: id,
                    speaker: self.speaker.clone(),
                    text: text.join(" "),
                    stt_latency_ms: None,
                });
            }
        }
        self.buf.clear();
        self.prev_segments.clear();
        self.last_partial.clear();
        out
    }

    pub fn speaker(&self) -> &Speaker {
        &self.speaker
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ms_conversion() {
        assert_eq!(ms_of(16_000), 1000);
        assert_eq!(ms_of(8_000), 500);
    }

    #[test]
    fn config_defaults_sane() {
        let c = StreamConfig::default();
        assert!(c.hold_ms < c.max_buf_ms);
        assert!(c.min_infer_ms >= c.step_ms);
    }
}
