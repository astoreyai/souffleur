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

/// A previous run's segment, recorded in ABSOLUTE audio time so that agreement
/// survives the front-trim that happens on every commit.
type PrevSeg = (String, u64); // (trimmed text, absolute end-time ms)

/// The committer's decision for one transcription run. Pure + unit-tested.
#[derive(Debug, PartialEq)]
struct Commit {
    /// Contiguous stable prefix text to emit as `transcript.final` ("" = none).
    text: String,
    /// End time (ms, relative to the current buffer start) of that prefix — the
    /// amount of audio to trim off the front. 0 when nothing commits.
    t1_ms: u64,
    /// The still-moving tail to emit as `transcript.partial`.
    pending: String,
}

/// Decide what to commit from one transcription. Frame-invariant: a segment is
/// "stable" only if it sits behind the `hold` lag (`t1_ms <= cut_ms`) AND the
/// same text appeared at ~the same ABSOLUTE time in the previous run
/// (LocalAgreement-2). Only the CONTIGUOUS stable prefix is committed — the first
/// non-stable speech segment ends the prefix, so a flickering middle segment can
/// never be skipped over (which would drop its audio on the trim).
fn decide_commit(
    segments: &[Segment],
    prev_abs: &[PrevSeg],
    base_ms: u64,
    cut_ms: u64,
    tol_ms: u64,
) -> Commit {
    let agreed = |text: &str, abs_t1: u64| -> bool {
        prev_abs
            .iter()
            .any(|(pt, pt1)| pt.as_str() == text && pt1.abs_diff(abs_t1) <= tol_ms)
    };
    let mut text = String::new();
    let mut t1_ms = 0u64;
    let mut pending = String::new();
    let mut in_prefix = true;
    for seg in segments {
        let t = seg.text.trim();
        if is_nonspeech(t) {
            // silence/marker segments don't break a contiguous prefix
            continue;
        }
        let rel_t1 = seg.t1_ms.max(0) as u64;
        let stable = in_prefix && rel_t1 <= cut_ms && agreed(t, base_ms + rel_t1);
        if stable {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(t);
            t1_ms = rel_t1;
        } else {
            in_prefix = false; // first unstable speech segment ends the prefix
            if !pending.is_empty() {
                pending.push(' ');
            }
            pending.push_str(t);
        }
    }
    Commit {
        text,
        t1_ms,
        pending: pending.trim().to_string(),
    }
}

/// Per-channel streaming transcriber. One instance per speaker channel.
pub struct StreamingStt {
    stt: Arc<Stt>,
    speaker: Speaker,
    cfg: StreamConfig,
    buf: Vec<f32>,
    /// Absolute audio time (ms) of `buf[0]` — advances by the committed amount on
    /// every trim, so agreement timestamps stay in one frame across commits.
    base_ms: u64,
    new_since_infer: usize,
    prev_abs: Vec<PrevSeg>,
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
            prev_abs: Vec::new(),
            last_partial: String::new(),
            utt: 0,
        }
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
        let commit = decide_commit(
            &tr.segments,
            &self.prev_abs,
            self.base_ms,
            cut,
            self.cfg.agree_tol_ms,
        );

        // Record THIS run for the next agreement check, in absolute time (using the
        // pre-trim base_ms, since this transcription was of the pre-trim buffer).
        self.prev_abs = tr
            .segments
            .iter()
            .filter_map(|s| {
                let t = s.text.trim();
                if is_nonspeech(t) {
                    None
                } else {
                    Some((t.to_string(), self.base_ms + s.t1_ms.max(0) as u64))
                }
            })
            .collect();

        if !commit.text.is_empty() {
            let id = self.next_utt_id();
            out.push(Event::TranscriptFinal {
                version: PROTOCOL_VERSION,
                t: session_ms,
                utterance_id: id,
                speaker: self.speaker.clone(),
                text: commit.text,
                stt_latency_ms: Some(stt_ms),
            });
            // Trim the committed audio off the front and advance absolute time.
            let cut_samples = ((commit.t1_ms as usize * SR) / 1000).min(self.buf.len());
            self.buf.drain(..cut_samples);
            self.base_ms += commit.t1_ms;
            self.last_partial.clear();
        }

        if !commit.pending.is_empty() && commit.pending != self.last_partial {
            out.push(Event::TranscriptPartial {
                version: PROTOCOL_VERSION,
                t: session_ms,
                utterance_id: format!("{}-p{}", self.speaker, self.utt + 1),
                speaker: self.speaker.clone(),
                text: commit.pending.clone(),
            });
            self.last_partial = commit.pending;
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
        self.base_ms += ms_of(self.buf.len());
        self.buf.clear();
        self.prev_abs.clear();
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

    fn seg(text: &str, t0: i64, t1: i64) -> Segment {
        Segment {
            text: text.into(),
            t0_ms: t0,
            t1_ms: t1,
        }
    }

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

    #[test]
    fn commits_contiguous_stable_prefix_only() {
        // prev run saw hello@1000 and world@2000 (absolute). This run: hello, world
        // both behind the 2500ms cut and agreed; "there"@3000 is past the cut.
        let prev = vec![("hello".to_string(), 1000), ("world".to_string(), 2000)];
        let segs = [
            seg("hello", 0, 1000),
            seg("world", 1000, 2000),
            seg("there", 2000, 3000),
        ];
        let c = decide_commit(&segs, &prev, 0, 2500, 500);
        assert_eq!(c.text, "hello world");
        assert_eq!(c.t1_ms, 2000);
        assert_eq!(c.pending, "there");
    }

    #[test]
    fn flickering_middle_segment_does_not_drop_later_words() {
        // S1 stable, S2 NOT in prev (flickering), S3 in prev. The prefix must STOP
        // at S2 — S3 must NOT be committed (which would trim away S2's audio).
        let prev = vec![("one".to_string(), 1000), ("three".to_string(), 3000)];
        let segs = [
            seg("one", 0, 1000),
            seg("two", 1000, 2000),
            seg("three", 2000, 3000),
        ];
        let c = decide_commit(&segs, &prev, 0, 5000, 500);
        assert_eq!(c.text, "one");
        assert_eq!(c.t1_ms, 1000);
        assert_eq!(c.pending, "two three");
    }

    #[test]
    fn agreement_is_frame_invariant_across_a_trim() {
        // Run 1 committed up to 2000ms, so base_ms advanced to 2000 and the buffer
        // was trimmed. "there" was at absolute 3000; in the trimmed frame it is at
        // t1=1000. With base_ms=2000 its absolute time is 3000 again, matching prev.
        let prev = vec![("there".to_string(), 3000)];
        let segs = [seg("there", 0, 1000)];
        let c = decide_commit(&segs, &prev, 2000, 5000, 500);
        assert_eq!(c.text, "there", "frame-shifted segment must still agree");
        // Without the absolute-time fix, rel t1=1000 vs prev 3000 would fail to agree.
    }

    #[test]
    fn nothing_agreed_commits_nothing() {
        let segs = [seg("hello", 0, 1000)];
        let c = decide_commit(&segs, &[], 0, 5000, 500);
        assert_eq!(c.text, "");
        assert_eq!(c.t1_ms, 0);
        assert_eq!(c.pending, "hello");
    }

    #[test]
    fn nonspeech_marker_does_not_break_the_prefix() {
        let prev = vec![("hi".to_string(), 1000), ("bye".to_string(), 3000)];
        let segs = [
            seg("hi", 0, 1000),
            seg("[BLANK_AUDIO]", 1000, 2000),
            seg("bye", 2000, 3000),
        ];
        let c = decide_commit(&segs, &prev, 0, 5000, 500);
        assert_eq!(c.text, "hi bye");
        assert_eq!(c.t1_ms, 3000);
        assert_eq!(c.pending, "");
    }
}
