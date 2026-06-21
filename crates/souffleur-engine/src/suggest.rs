//! Local coaching-suggestion engine.
//!
//! Given the recent confirmed transcript, asks a local LLM (Ollama) for up to a
//! couple of short coaching cues and turns them into Coach Protocol `prompt`
//! events. Local-only by default — the transcript never leaves the machine. If
//! Ollama is unavailable the engine reports it; the daemon then runs transcript-
//! only rather than fabricating prompts (no stubs).

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use souffleur_protocol::{Event, PromptKind, PROTOCOL_VERSION};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct SuggestConfig {
    pub url: String,
    pub model: String,
    pub max_turns: usize,
    pub temperature: f32,
    pub num_predict: i32,
    pub timeout: Duration,
}

impl Default for SuggestConfig {
    fn default() -> Self {
        Self {
            url: "http://localhost:11434".to_string(),
            model: "qwen3:8b".to_string(),
            max_turns: 8,
            temperature: 0.4,
            num_predict: 200,
            timeout: Duration::from_secs(25),
        }
    }
}

const SYSTEM_PROMPT: &str = "\
You are a discreet live conversation coach. You see a rolling transcript of a \
real conversation. ME is the person you assist; THEM is the other party. Output \
ONLY JSON of the form {\"prompts\":[{\"kind\":\"...\",\"text\":\"...\",\"priority\":N}]}. \
kind is one of: fact, question, objection, cue, recover, note. text is at most 12 \
words, directly usable by ME in the moment (a fact to recall, a question to ask, \
an objection rebuttal, a next-line cue). priority is 1-5 (5=urgent). Suggest only \
when genuinely useful; return {\"prompts\":[]} when nothing helps. Never invent \
facts not supported by the transcript.";

pub struct SuggestionEngine {
    cfg: SuggestConfig,
    context: Vec<(String, String)>,
    pid: u32,
}

#[derive(Deserialize)]
struct ChatResp {
    message: ChatMsg,
}
#[derive(Deserialize)]
struct ChatMsg {
    content: String,
}
#[derive(Deserialize)]
struct PromptList {
    #[serde(default)]
    prompts: Vec<PromptItem>,
}
#[derive(Deserialize)]
struct PromptItem {
    kind: Option<String>,
    text: String,
    #[serde(default)]
    priority: Option<u8>,
}

fn parse_kind(s: &Option<String>) -> PromptKind {
    match s.as_deref().unwrap_or("note").to_ascii_lowercase().as_str() {
        "fact" => PromptKind::Fact,
        "question" => PromptKind::Question,
        "objection" => PromptKind::Objection,
        "cue" => PromptKind::Cue,
        "recover" => PromptKind::Recover,
        _ => PromptKind::Note,
    }
}

/// Pull the first balanced JSON object out of a string (in case the model wraps it).
fn extract_json_object(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let end = s.rfind('}')?;
    if end > start {
        Some(&s[start..=end])
    } else {
        None
    }
}

impl SuggestionEngine {
    pub fn new(cfg: SuggestConfig) -> Self {
        Self {
            cfg,
            context: Vec::new(),
            pid: 0,
        }
    }

    /// Is the Ollama server reachable and the model present?
    pub fn check(&self) -> Result<()> {
        let resp = ureq::get(&format!("{}/api/tags", self.cfg.url))
            .timeout(Duration::from_secs(3))
            .call()
            .context("ollama /api/tags (is `ollama serve` running?)")?;
        let body: serde_json::Value = resp.into_json().context("parse /api/tags")?;
        let have = body["models"]
            .as_array()
            .map(|a| {
                a.iter()
                    .any(|m| m["name"].as_str() == Some(self.cfg.model.as_str()))
            })
            .unwrap_or(false);
        if !have {
            return Err(anyhow!(
                "ollama model {:?} not pulled (try `ollama pull {}`)",
                self.cfg.model,
                self.cfg.model
            ));
        }
        Ok(())
    }

    /// Load the model into the server (and GPU) so the first real suggestion
    /// isn't a cold hit. Returns the warmup round-trip in ms.
    pub fn warmup(&self) -> Result<u64> {
        let req = serde_json::json!({
            "model": self.cfg.model,
            "stream": false,
            "think": false,
            "options": { "num_predict": 1 },
            "messages": [{ "role": "user", "content": "ok" }]
        });
        let started = Instant::now();
        ureq::post(&format!("{}/api/chat", self.cfg.url))
            .timeout(self.cfg.timeout)
            .send_json(req)
            .context("ollama warmup")?;
        Ok(started.elapsed().as_millis() as u64)
    }

    pub fn push_turn(&mut self, speaker: &str, text: &str) {
        self.context.push((speaker.to_uppercase(), text.to_string()));
        let n = self.context.len();
        if n > self.cfg.max_turns {
            self.context.drain(..n - self.cfg.max_turns);
        }
    }

    fn transcript(&self) -> String {
        self.context
            .iter()
            .map(|(s, t)| format!("{s}: {t}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Call the LLM on the current context and return any `prompt` events.
    /// Returns the LLM round-trip latency alongside the events.
    pub fn suggest(&mut self, session_ms: u64) -> Result<(Vec<Event>, u64)> {
        if self.context.is_empty() {
            return Ok((Vec::new(), 0));
        }
        let req = serde_json::json!({
            "model": self.cfg.model,
            "stream": false,
            "think": false,
            "format": "json",
            "options": { "temperature": self.cfg.temperature, "num_predict": self.cfg.num_predict },
            "messages": [
                { "role": "system", "content": SYSTEM_PROMPT },
                { "role": "user", "content": format!("Transcript so far:\n{}\n\nCoach ME now.", self.transcript()) }
            ]
        });

        let started = Instant::now();
        let resp = ureq::post(&format!("{}/api/chat", self.cfg.url))
            .timeout(self.cfg.timeout)
            .send_json(req)
            .context("ollama /api/chat")?;
        let parsed: ChatResp = resp.into_json().context("parse chat response")?;
        let latency = started.elapsed().as_millis() as u64;

        let content = parsed.message.content;
        let json = serde_json::from_str::<PromptList>(&content)
            .or_else(|_| {
                extract_json_object(&content)
                    .ok_or_else(|| anyhow!("no JSON object in model output"))
                    .and_then(|j| serde_json::from_str::<PromptList>(j).map_err(|e| e.into()))
            })
            .with_context(|| format!("parse prompts from: {content:?}"))?;

        let mut out = Vec::new();
        for item in json.prompts {
            let text = item.text.trim().to_string();
            if text.is_empty() {
                continue;
            }
            self.pid += 1;
            out.push(Event::Prompt {
                version: PROTOCOL_VERSION,
                t: session_ms,
                prompt_id: format!("p{}", self.pid),
                kind: parse_kind(&item.kind),
                text,
                ttl_ms: 12_000,
                priority: item.priority.unwrap_or(3).clamp(1, 5),
                source_utterance: None,
            });
        }
        Ok((out, latency))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_wrapped_json() {
        let s = "thinking... {\"prompts\":[]} trailing";
        assert_eq!(extract_json_object(s), Some("{\"prompts\":[]}"));
    }

    #[test]
    fn kind_mapping() {
        assert!(matches!(
            parse_kind(&Some("OBJECTION".into())),
            PromptKind::Objection
        ));
        assert!(matches!(parse_kind(&None), PromptKind::Note));
    }

    #[test]
    fn context_truncates() {
        let mut e = SuggestionEngine::new(SuggestConfig {
            max_turns: 2,
            ..Default::default()
        });
        e.push_turn("me", "a");
        e.push_turn("them", "b");
        e.push_turn("me", "c");
        assert_eq!(e.context.len(), 2);
        assert_eq!(e.context[0].1, "b");
    }
}
