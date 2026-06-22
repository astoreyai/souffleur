//! Coach Protocol v0 — the stable seam between the Souffleur core and any surface
//! (phone PWA, smart glasses, desktop overlay).
//!
//! Transport is newline-delimited JSON (NDJSON) over a WebSocket; these are the
//! frame types. See `docs/plan/COACH_PROTOCOL.md` for the normative spec.

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Current protocol major version. Bumped on any breaking change.
pub const PROTOCOL_VERSION: u8 = 0;

/// Who spoke a piece of transcript. In Phase 0 this is derived for free from
/// channel separation: the mic channel is [`Speaker::Me`], the system-audio
/// loopback channel is [`Speaker::Them`]. Acoustic diarization (Phase 5) later
/// splits individual remote speakers into [`Speaker::ThemN`].
///
/// Serializes to the wire string form: `"me"`, `"them"`, `"them:2"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Speaker {
    Me,
    Them,
    /// n-th distinct remote speaker, once acoustic diarization is enabled.
    ThemN(u16),
}

impl std::fmt::Display for Speaker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Speaker::Me => write!(f, "me"),
            Speaker::Them => write!(f, "them"),
            Speaker::ThemN(n) => write!(f, "them:{n}"),
        }
    }
}

impl std::str::FromStr for Speaker {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "me" => Ok(Speaker::Me),
            "them" => Ok(Speaker::Them),
            other => other
                .strip_prefix("them:")
                .and_then(|n| n.parse::<u16>().ok())
                .map(Speaker::ThemN)
                .ok_or_else(|| format!("invalid speaker: {other:?}")),
        }
    }
}

impl Serialize for Speaker {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Speaker {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = Speaker;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a speaker string: \"me\", \"them\", or \"them:<n>\"")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Speaker, E> {
                v.parse().map_err(E::custom)
            }
        }
        d.deserialize_str(V)
    }
}

/// The kind of coaching cue a `prompt` event carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptKind {
    Fact,
    Question,
    Objection,
    Cue,
    Recover,
    Note,
}

/// A core -> surface event. Serialized as `{ "v":0, "type":"...", "t":<ms>, ... }`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// Provisional, still-being-revised text. Render greyed; never trigger
    /// suggestions off this.
    #[serde(rename = "transcript.partial")]
    TranscriptPartial {
        #[serde(rename = "v", default = "default_version")]
        version: u8,
        t: u64,
        utterance_id: String,
        speaker: Speaker,
        text: String,
    },
    /// Confirmed, immutable prefix. Render solid; the suggestion engine fires
    /// only off these.
    #[serde(rename = "transcript.final")]
    TranscriptFinal {
        #[serde(rename = "v", default = "default_version")]
        version: u8,
        t: u64,
        utterance_id: String,
        speaker: Speaker,
        text: String,
        /// Measured audio-end -> text-ready latency for this segment (ms).
        #[serde(skip_serializing_if = "Option::is_none")]
        stt_latency_ms: Option<u64>,
    },
    /// A short coaching cue to display (Phase 1+).
    Prompt {
        #[serde(rename = "v", default = "default_version")]
        version: u8,
        t: u64,
        prompt_id: String,
        kind: PromptKind,
        text: String,
        ttl_ms: u64,
        priority: u8,
        #[serde(skip_serializing_if = "Option::is_none")]
        source_utterance: Option<String>,
    },
    /// Heartbeat + status.
    State {
        #[serde(rename = "v", default = "default_version")]
        version: u8,
        t: u64,
        capturing: bool,
        model: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        e2e_latency_ms: Option<u64>,
        consent_disclosed: bool,
        surfaces: u32,
    },
    /// Non-fatal core-side problem the surface should surface to the user.
    Error {
        #[serde(rename = "v", default = "default_version")]
        version: u8,
        t: u64,
        code: String,
        message: String,
        fatal: bool,
    },
    /// A retrieval corpus finished loading (in response to [`Control::SetCorpus`]
    /// or `--corpus`); cues are now grounded in it.
    CorpusLoaded {
        #[serde(rename = "v", default = "default_version")]
        version: u8,
        t: u64,
        path: String,
        chunks: u32,
        sources: u32,
    },
}

/// A surface -> core control message (optional uplink).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Control {
    SetConsent {
        disclosed: bool,
    },
    Dismiss {
        prompt_id: String,
    },
    Hint {
        text: String,
    },
    /// Load (or replace) the retrieval corpus from a directory on the host. The
    /// path is server-side: the core reads its own filesystem (local-first), so
    /// surfaces send a path, not uploaded files.
    SetCorpus {
        path: String,
    },
    Ack,
}

fn default_version() -> u8 {
    PROTOCOL_VERSION
}

impl Event {
    /// Serialize to one NDJSON line (no trailing newline).
    pub fn to_ndjson(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_roundtrips_and_tags_correctly() {
        let ev = Event::TranscriptFinal {
            version: PROTOCOL_VERSION,
            t: 2010,
            utterance_id: "u12".into(),
            speaker: Speaker::Them,
            text: "So the number we landed on was forty-two thousand.".into(),
            stt_latency_ms: Some(260),
        };
        let line = ev.to_ndjson().unwrap();
        assert!(line.contains("\"type\":\"transcript.final\""));
        assert!(line.contains("\"speaker\":\"them\""));
        assert!(line.contains("\"stt_latency_ms\":260"));
        // Round-trips back to the same variant.
        let back: Event = serde_json::from_str(&line).unwrap();
        match back {
            Event::TranscriptFinal { text, speaker, .. } => {
                assert_eq!(speaker, Speaker::Them);
                assert!(text.starts_with("So the number"));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn partial_omits_latency_field() {
        let ev = Event::TranscriptPartial {
            version: PROTOCOL_VERSION,
            t: 1840,
            utterance_id: "u12".into(),
            speaker: Speaker::Me,
            text: "so the number we landed on was".into(),
        };
        let line = ev.to_ndjson().unwrap();
        assert!(line.contains("\"type\":\"transcript.partial\""));
        assert!(!line.contains("stt_latency_ms"));
        assert!(line.contains("\"speaker\":\"me\""));
    }

    #[test]
    fn control_parses() {
        let c: Control =
            serde_json::from_str(r#"{"type":"set_consent","disclosed":true}"#).unwrap();
        assert!(matches!(c, Control::SetConsent { disclosed: true }));

        let d: Control = serde_json::from_str(r#"{"type":"dismiss","prompt_id":"p7"}"#).unwrap();
        assert!(matches!(d, Control::Dismiss { prompt_id } if prompt_id == "p7"));

        let h: Control = serde_json::from_str(r#"{"type":"hint","text":"slow down"}"#).unwrap();
        assert!(matches!(h, Control::Hint { text } if text == "slow down"));

        let a: Control = serde_json::from_str(r#"{"type":"ack"}"#).unwrap();
        assert!(matches!(a, Control::Ack));

        let sc: Control =
            serde_json::from_str(r#"{"type":"set_corpus","path":"/home/a/thesis"}"#).unwrap();
        assert!(matches!(sc, Control::SetCorpus { path } if path == "/home/a/thesis"));
    }

    #[test]
    fn speaker_them_n_roundtrips() {
        // wire string <-> ThemN, in both serde directions
        assert_eq!("them:3".parse::<Speaker>().unwrap(), Speaker::ThemN(3));
        assert_eq!(Speaker::ThemN(3).to_string(), "them:3");

        let json = serde_json::to_string(&Speaker::ThemN(2)).unwrap();
        assert_eq!(json, "\"them:2\"");
        let back: Speaker = serde_json::from_str("\"them:2\"").unwrap();
        assert_eq!(back, Speaker::ThemN(2));

        // me / them still roundtrip
        assert_eq!(
            serde_json::from_str::<Speaker>("\"me\"").unwrap(),
            Speaker::Me
        );
        assert_eq!(
            serde_json::from_str::<Speaker>("\"them\"").unwrap(),
            Speaker::Them
        );

        // garbage rejected, not silently coerced
        assert!("them:".parse::<Speaker>().is_err());
        assert!("them:notanumber".parse::<Speaker>().is_err());
        assert!("nobody".parse::<Speaker>().is_err());
    }
}
