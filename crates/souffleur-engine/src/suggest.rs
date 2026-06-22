//! Local + cloud coaching-suggestion engine.
//!
//! Given the recent confirmed transcript, a [`SuggestBackend`] returns up to a
//! couple of short coaching cues as JSON, which the engine turns into Coach
//! Protocol `prompt` events.
//!
//! - **Local (default):** Ollama on the machine — the transcript never leaves it.
//! - **Cloud (opt-in, BYO-key, consent-gated):** Gemini / Claude / OpenAI. These
//!   send the transcript off-device, so the daemon gates them behind
//!   `--allow-cloud` AND a disclosed-consent flag (see [`SuggestionEngine::suggest_gated`]).
//!
//! No mock backend exists (no-stub rule): a cloud backend with no API key fails
//! closed at construction.

use crate::corpus::Corpus;
use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use souffleur_protocol::{Event, PromptKind, PROTOCOL_VERSION};
use std::time::{Duration, Instant};

/// Shared tuning, independent of which backend is selected.
#[derive(Debug, Clone)]
pub struct SuggestConfig {
    pub max_turns: usize,
    pub temperature: f32,
    pub num_predict: i32,
    pub timeout: Duration,
}

impl Default for SuggestConfig {
    fn default() -> Self {
        Self {
            max_turns: 8,
            temperature: 0.4,
            num_predict: 256,
            timeout: Duration::from_secs(20),
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
facts not supported by the transcript. If a RELEVANT MATERIAL block from the \
user's own documents precedes the transcript, prefer grounding facts in it and \
cite the source in brackets, e.g. text \"Per [thesis.tex]: N=412\".";

fn user_prompt(transcript: &str) -> String {
    format!("Transcript so far:\n{transcript}\n\nCoach ME now. Respond with only the JSON object.")
}

// ----------------------------------------------------------------------------
// Backend trait + implementations
// ----------------------------------------------------------------------------

/// A pluggable completion backend. `complete` returns the model's raw JSON text
/// (a `{"prompts":[...]}` object) plus the round-trip latency in ms.
pub trait SuggestBackend: Send {
    fn name(&self) -> &str;
    /// True if this backend sends the transcript off the machine.
    fn is_cloud(&self) -> bool;
    /// Validate the backend is reachable / configured (called once at startup).
    fn check(&self) -> Result<()>;
    /// Optional warmup (load the model). Default: no-op.
    fn warmup(&self) -> Result<u64> {
        Ok(0)
    }
    fn complete(
        &self,
        system: &str,
        transcript: &str,
        cfg: &SuggestConfig,
    ) -> Result<(String, u64)>;
}

/// Local Ollama backend (`/api/chat`, JSON mode). On-device; not cloud.
pub struct OllamaBackend {
    url: String,
    model: String,
}

impl OllamaBackend {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            url: std::env::var("OLLAMA_URL").unwrap_or_else(|_| "http://localhost:11434".into()),
            model: model.into(),
        }
    }
}

impl SuggestBackend for OllamaBackend {
    fn name(&self) -> &str {
        "ollama"
    }
    fn is_cloud(&self) -> bool {
        false
    }
    fn check(&self) -> Result<()> {
        let resp = ureq::get(&format!("{}/api/tags", self.url))
            .timeout(Duration::from_secs(3))
            .call()
            .context("ollama /api/tags (is `ollama serve` running?)")?;
        let body: Value = resp.into_json().context("parse /api/tags")?;
        let have = body["models"]
            .as_array()
            .map(|a| {
                a.iter()
                    .any(|m| m["name"].as_str() == Some(self.model.as_str()))
            })
            .unwrap_or(false);
        if !have {
            bail!(
                "ollama model {:?} not pulled (try `ollama pull {}`)",
                self.model,
                self.model
            );
        }
        Ok(())
    }
    fn warmup(&self) -> Result<u64> {
        let req = json!({
            "model": self.model, "stream": false, "think": false,
            "options": { "num_predict": 1 },
            "messages": [{ "role": "user", "content": "ok" }]
        });
        let started = Instant::now();
        ureq::post(&format!("{}/api/chat", self.url))
            .timeout(Duration::from_secs(20))
            .send_json(req)
            .context("ollama warmup")?;
        Ok(started.elapsed().as_millis() as u64)
    }
    fn complete(
        &self,
        system: &str,
        transcript: &str,
        cfg: &SuggestConfig,
    ) -> Result<(String, u64)> {
        let req = json!({
            "model": self.model, "stream": false, "think": false, "format": "json",
            "options": { "temperature": cfg.temperature, "num_predict": cfg.num_predict },
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user_prompt(transcript) }
            ]
        });
        let started = Instant::now();
        let resp = ureq::post(&format!("{}/api/chat", self.url))
            .timeout(cfg.timeout)
            .send_json(req)
            .context("ollama /api/chat")?;
        let v: Value = resp.into_json().context("parse chat response")?;
        let content = v["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        Ok((content, started.elapsed().as_millis() as u64))
    }
}

/// Google Gemini backend (`generateContent`, responseMimeType=application/json).
pub struct GeminiBackend {
    model: String,
    key: String,
}

impl GeminiBackend {
    pub fn from_env(model: impl Into<String>) -> Result<Self> {
        let key = std::env::var("GEMINI_API_KEY")
            .ok()
            .filter(|k| !k.is_empty())
            .ok_or_else(|| anyhow!("GEMINI_API_KEY not set"))?;
        Ok(Self {
            model: model.into(),
            key,
        })
    }
}

impl SuggestBackend for GeminiBackend {
    fn name(&self) -> &str {
        "gemini"
    }
    fn is_cloud(&self) -> bool {
        true
    }
    fn check(&self) -> Result<()> {
        ureq::get("https://generativelanguage.googleapis.com/v1beta/models")
            .set("x-goog-api-key", &self.key)
            .timeout(Duration::from_secs(5))
            .call()
            .context("gemini models list (is GEMINI_API_KEY valid?)")?;
        Ok(())
    }
    fn complete(
        &self,
        system: &str,
        transcript: &str,
        cfg: &SuggestConfig,
    ) -> Result<(String, u64)> {
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
            self.model
        );
        let req = json!({
            "system_instruction": { "parts": [{ "text": system }] },
            "contents": [{ "role": "user", "parts": [{ "text": user_prompt(transcript) }] }],
            "generationConfig": {
                "responseMimeType": "application/json",
                "temperature": cfg.temperature,
                "maxOutputTokens": cfg.num_predict.max(64),
                // Gemini 2.5 models think by default and would spend the whole
                // output budget on thoughts; disable it for this short JSON task.
                "thinkingConfig": { "thinkingBudget": 0 }
            }
        });
        let started = Instant::now();
        let resp = ureq::post(&url)
            .set("x-goog-api-key", &self.key)
            .timeout(cfg.timeout)
            .send_json(req)
            .context("gemini generateContent")?;
        let v: Value = resp.into_json().context("parse gemini response")?;
        let content = v["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        Ok((content, started.elapsed().as_millis() as u64))
    }
}

/// Anthropic Claude backend (`/v1/messages`). BYO-key (`ANTHROPIC_API_KEY`).
pub struct AnthropicBackend {
    model: String,
    key: String,
}

impl AnthropicBackend {
    pub fn from_env(model: impl Into<String>) -> Result<Self> {
        let key = std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .filter(|k| !k.is_empty())
            .ok_or_else(|| anyhow!("ANTHROPIC_API_KEY not set"))?;
        Ok(Self {
            model: model.into(),
            key,
        })
    }
}

impl SuggestBackend for AnthropicBackend {
    fn name(&self) -> &str {
        "anthropic"
    }
    fn is_cloud(&self) -> bool {
        true
    }
    fn check(&self) -> Result<()> {
        ureq::get("https://api.anthropic.com/v1/models")
            .set("x-api-key", &self.key)
            .set("anthropic-version", "2023-06-01")
            .timeout(Duration::from_secs(5))
            .call()
            .context("anthropic /v1/models (is ANTHROPIC_API_KEY valid?)")?;
        Ok(())
    }
    fn complete(
        &self,
        system: &str,
        transcript: &str,
        cfg: &SuggestConfig,
    ) -> Result<(String, u64)> {
        let req = json!({
            "model": self.model,
            "max_tokens": cfg.num_predict.max(256),
            "system": system,
            "messages": [{ "role": "user", "content": user_prompt(transcript) }]
        });
        let started = Instant::now();
        let resp = ureq::post("https://api.anthropic.com/v1/messages")
            .set("x-api-key", &self.key)
            .set("anthropic-version", "2023-06-01")
            .set("content-type", "application/json")
            .timeout(cfg.timeout)
            .send_json(req)
            .context("anthropic /v1/messages")?;
        let v: Value = resp.into_json().context("parse anthropic response")?;
        // content is a list of blocks; take the first text block.
        let content = v["content"]
            .as_array()
            .and_then(|blocks| blocks.iter().find_map(|b| b["text"].as_str()))
            .unwrap_or_default()
            .to_string();
        Ok((content, started.elapsed().as_millis() as u64))
    }
}

/// OpenAI (or OpenAI-compatible) backend (`/v1/chat/completions`, json_object).
pub struct OpenAiBackend {
    model: String,
    key: String,
    base: String,
}

impl OpenAiBackend {
    pub fn from_env(model: impl Into<String>) -> Result<Self> {
        let key = std::env::var("OPENAI_API_KEY")
            .ok()
            .filter(|k| !k.is_empty())
            .ok_or_else(|| anyhow!("OPENAI_API_KEY not set"))?;
        let base =
            std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "https://api.openai.com/v1".into());
        Ok(Self {
            model: model.into(),
            key,
            base,
        })
    }
}

impl SuggestBackend for OpenAiBackend {
    fn name(&self) -> &str {
        "openai"
    }
    fn is_cloud(&self) -> bool {
        true
    }
    fn check(&self) -> Result<()> {
        ureq::get(&format!("{}/models", self.base))
            .set("authorization", &format!("Bearer {}", self.key))
            .timeout(Duration::from_secs(5))
            .call()
            .context("openai /models (is OPENAI_API_KEY valid?)")?;
        Ok(())
    }
    fn complete(
        &self,
        system: &str,
        transcript: &str,
        cfg: &SuggestConfig,
    ) -> Result<(String, u64)> {
        let req = json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user_prompt(transcript) }
            ],
            "response_format": { "type": "json_object" },
            "temperature": cfg.temperature,
            "max_tokens": cfg.num_predict.max(256)
        });
        let started = Instant::now();
        let resp = ureq::post(&format!("{}/chat/completions", self.base))
            .set("authorization", &format!("Bearer {}", self.key))
            .timeout(cfg.timeout)
            .send_json(req)
            .context("openai /chat/completions")?;
        let v: Value = resp.into_json().context("parse openai response")?;
        let content = v["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        Ok((content, started.elapsed().as_millis() as u64))
    }
}

/// Construct a backend by name. Cloud backends read their key from the
/// environment and fail closed if it's missing. `model` overrides the per-backend
/// default.
pub fn make_backend(kind: &str, model: Option<String>) -> Result<Box<dyn SuggestBackend>> {
    Ok(match kind {
        "local" | "ollama" => Box::new(OllamaBackend::new(
            model.unwrap_or_else(|| "qwen3:8b".into()),
        )),
        "gemini" => Box::new(GeminiBackend::from_env(
            model.unwrap_or_else(|| "gemini-2.5-flash".into()),
        )?),
        "claude" | "anthropic" => Box::new(AnthropicBackend::from_env(
            model.unwrap_or_else(|| "claude-opus-4-8".into()),
        )?),
        // Convenience alias: Claude Haiku via the Anthropic API (fast, per-token
        // billed, BYO ANTHROPIC_API_KEY). The cheapest Claude tier for cue work.
        "haiku" => Box::new(AnthropicBackend::from_env(
            model.unwrap_or_else(|| "claude-haiku-4-5".into()),
        )?),
        "openai" => Box::new(OpenAiBackend::from_env(
            model.unwrap_or_else(|| "gpt-4o-mini".into()),
        )?),
        other => bail!("unknown suggest backend: {other} (local|gemini|claude|haiku|openai)"),
    })
}

/// True if a backend name sends data off-device (so it needs --allow-cloud).
pub fn backend_is_cloud(kind: &str) -> bool {
    !matches!(kind, "local" | "ollama")
}

// ----------------------------------------------------------------------------
// Engine
// ----------------------------------------------------------------------------

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

/// Pull the FIRST balanced JSON object out of a string (models sometimes wrap
/// the JSON in prose or emit trailing junk after it). Walks brace depth,
/// respecting string literals and escapes, so a `}` inside a string value or a
/// stray brace in trailing prose does not corrupt the result.
fn extract_json_object(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    let start = s.find('{')?; // '{' is ASCII, so this is a char boundary
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escaped = false;
    for i in start..bytes.len() {
        let c = bytes[i];
        if in_str {
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == b'"' {
                in_str = false;
            }
        } else {
            match c {
                b'"' => in_str = true,
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(&s[start..=i]);
                    }
                }
                _ => {}
            }
        }
    }
    None
}

/// Parse model JSON output into `prompt` events, allocating ids from `pid`.
fn parse_prompts(content: &str, pid: &mut u32, session_ms: u64) -> Result<Vec<Event>> {
    let list = serde_json::from_str::<PromptList>(content)
        .or_else(|_| {
            extract_json_object(content)
                .ok_or_else(|| anyhow!("no JSON object in model output"))
                .and_then(|j| serde_json::from_str::<PromptList>(j).map_err(|e| e.into()))
        })
        .with_context(|| format!("parse prompts from: {content:?}"))?;

    let mut out = Vec::new();
    for item in list.prompts {
        let text = item.text.trim().to_string();
        if text.is_empty() {
            continue;
        }
        *pid += 1;
        out.push(Event::Prompt {
            version: PROTOCOL_VERSION,
            t: session_ms,
            prompt_id: format!("p{pid}"),
            kind: parse_kind(&item.kind),
            text,
            ttl_ms: 12_000,
            priority: item.priority.unwrap_or(3).clamp(1, 5),
            source_utterance: None,
        });
    }
    Ok(out)
}

/// Wraps a backend with rolling transcript context and a single prompt-id counter
/// (so ids never collide regardless of backend).
pub struct SuggestionEngine {
    backend: Box<dyn SuggestBackend>,
    cfg: SuggestConfig,
    context: Vec<(String, String)>,
    pid: u32,
    corpus: Option<Corpus>,
    retrieve_k: usize,
}

impl SuggestionEngine {
    pub fn new(backend: Box<dyn SuggestBackend>, cfg: SuggestConfig) -> Self {
        Self {
            backend,
            cfg,
            context: Vec::new(),
            pid: 0,
            corpus: None,
            retrieve_k: 3,
        }
    }

    /// Attach a retrieval corpus; cues are then grounded in the top-k chunks most
    /// similar to the recent transcript (RAG).
    pub fn set_corpus(&mut self, corpus: Corpus) {
        self.corpus = Some(corpus);
    }

    /// Number of corpus chunks to retrieve per suggestion (default 3). A no-op
    /// `0` is clamped to 1 so an attached corpus always contributes.
    pub fn set_retrieve_k(&mut self, k: usize) {
        self.retrieve_k = k.max(1);
    }

    pub fn backend_name(&self) -> &str {
        self.backend.name()
    }
    pub fn is_cloud(&self) -> bool {
        self.backend.is_cloud()
    }
    pub fn check(&self) -> Result<()> {
        self.backend.check()
    }
    pub fn warmup(&self) -> Result<u64> {
        self.backend.warmup()
    }

    pub fn push_turn(&mut self, speaker: &str, text: &str) {
        self.context
            .push((speaker.to_uppercase(), text.to_string()));
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

    /// Run the backend on the current context and return any `prompt` events.
    /// When a corpus is attached, the recent transcript retrieves the top-k most
    /// similar chunks, which are prepended as a RELEVANT MATERIAL block so the
    /// backend can ground cues in the user's documents. Retrieval failure (e.g.
    /// the embedder is down) degrades gracefully to transcript-only.
    pub fn suggest(&mut self, session_ms: u64) -> Result<(Vec<Event>, u64)> {
        if self.context.is_empty() {
            return Ok((Vec::new(), 0));
        }
        let transcript = self.transcript();
        let user_content = match &self.corpus {
            Some(c) => match c.context_block(&transcript, self.retrieve_k) {
                Some(block) => format!("{block}\nCONVERSATION:\n{transcript}"),
                None => transcript,
            },
            None => transcript,
        };
        let (content, latency) = self
            .backend
            .complete(SYSTEM_PROMPT, &user_content, &self.cfg)?;
        let evs = parse_prompts(&content, &mut self.pid, session_ms)?;
        Ok((evs, latency))
    }

    /// Like [`suggest`](Self::suggest), but a CLOUD backend refuses to transmit
    /// the transcript off-device unless consent has been disclosed. This is the
    /// enforced privacy chokepoint for the cloud tier.
    pub fn suggest_gated(
        &mut self,
        session_ms: u64,
        consent_disclosed: bool,
    ) -> Result<(Vec<Event>, u64)> {
        if self.is_cloud() && !consent_disclosed {
            return Ok((Vec::new(), 0));
        }
        self.suggest(session_ms)
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
    fn extracts_balanced_object_ignoring_trailing_braces() {
        let s = "{\"prompts\":[{\"text\":\"hi\"}]} note: {x}";
        assert_eq!(
            extract_json_object(s),
            Some("{\"prompts\":[{\"text\":\"hi\"}]}")
        );
    }

    #[test]
    fn brace_inside_string_does_not_close_object() {
        let s = "{\"text\":\"close } brace\"} tail";
        assert_eq!(extract_json_object(s), Some("{\"text\":\"close } brace\"}"));
    }

    #[test]
    fn no_object_returns_none() {
        assert_eq!(extract_json_object("no json here"), None);
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
    fn parse_prompts_allocates_ids_and_clamps_priority() {
        let mut pid = 0;
        let evs = parse_prompts(
            "{\"prompts\":[{\"kind\":\"cue\",\"text\":\"go\",\"priority\":9},{\"text\":\"\"}]}",
            &mut pid,
            5,
        )
        .unwrap();
        assert_eq!(evs.len(), 1); // empty-text prompt dropped
        match &evs[0] {
            Event::Prompt {
                prompt_id,
                priority,
                ..
            } => {
                assert_eq!(prompt_id, "p1");
                assert_eq!(*priority, 5); // clamped from 9
            }
            _ => panic!("wrong event"),
        }
    }

    /// A backend that panics if `complete` is called — proves the consent gate
    /// stops a cloud call before any transcript leaves the machine.
    struct PanicCloud;
    impl SuggestBackend for PanicCloud {
        fn name(&self) -> &str {
            "panic-cloud"
        }
        fn is_cloud(&self) -> bool {
            true
        }
        fn check(&self) -> Result<()> {
            Ok(())
        }
        fn complete(&self, _: &str, _: &str, _: &SuggestConfig) -> Result<(String, u64)> {
            panic!("cloud backend must not be called without consent");
        }
    }

    #[test]
    fn cloud_backend_refuses_to_transmit_without_consent() {
        let mut e = SuggestionEngine::new(Box::new(PanicCloud), SuggestConfig::default());
        e.push_turn("them", "our budget is tight");
        let (evs, lat) = e.suggest_gated(1, false).unwrap(); // must NOT call complete()
        assert!(evs.is_empty());
        assert_eq!(lat, 0);
    }

    #[test]
    fn context_truncates() {
        let mut e = SuggestionEngine::new(
            Box::new(OllamaBackend::new("m")),
            SuggestConfig {
                max_turns: 2,
                ..Default::default()
            },
        );
        e.push_turn("me", "a");
        e.push_turn("them", "b");
        e.push_turn("me", "c");
        assert_eq!(e.context.len(), 2);
        assert_eq!(e.context[0].1, "b");
    }
}
