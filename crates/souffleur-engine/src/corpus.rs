//! Self-contained retrieval corpus for grounding coaching cues in the user's own
//! documents (dissertation chapters, interview prep, notes). Chunks are embedded
//! locally via Ollama `nomic-embed-text` (768-dim, on-device, zero cost); at
//! suggestion time the recent transcript retrieves the top-k most similar chunks,
//! which are injected into the suggestion prompt.
//!
//! Privacy: the corpus stays on the machine. It only leaves if a CLOUD suggestion
//! backend is used AND consent has been disclosed — the retrieved text rides the
//! same `suggest_gated` chokepoint as the transcript.

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";
const DEFAULT_EMBED_MODEL: &str = "nomic-embed-text";
const MAX_CHUNK_CHARS: usize = 900; // a passage-sized chunk
const MIN_CHUNK_CHARS: usize = 40; // drop trivial fragments

/// One embedded passage from the corpus.
pub struct Chunk {
    pub text: String,
    pub source: String,
    embedding: Vec<f32>,
}

/// A local, embedded document corpus.
pub struct Corpus {
    chunks: Vec<Chunk>,
    url: String,
    model: String,
}

/// Cosine similarity of two equal-length vectors. Returns 0.0 on a length
/// mismatch or a zero-norm vector (so a degenerate embedding never ranks high).
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// Embed one text via Ollama `/api/embeddings`.
pub fn embed(text: &str, url: &str, model: &str) -> Result<Vec<f32>> {
    let req = json!({ "model": model, "prompt": text });
    let resp = ureq::post(&format!("{url}/api/embeddings"))
        .timeout(Duration::from_secs(30))
        .send_json(req)
        .context("ollama /api/embeddings (is `ollama serve` running?)")?;
    let v: Value = resp.into_json().context("parse embeddings response")?;
    let emb: Vec<f32> = v["embedding"]
        .as_array()
        .ok_or_else(|| anyhow!("no embedding in response (is {model:?} pulled?)"))?
        .iter()
        .map(|x| x.as_f64().unwrap_or(0.0) as f32)
        .collect();
    if emb.is_empty() {
        return Err(anyhow!("empty embedding for model {model:?}"));
    }
    Ok(emb)
}

/// Split a document into passage-sized chunks on blank-line (paragraph)
/// boundaries, accumulating paragraphs up to `MAX_CHUNK_CHARS`.
pub fn chunk_text(text: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut cur = String::new();
    for para in text.split("\n\n") {
        let para = para.trim();
        if para.is_empty() {
            continue;
        }
        if !cur.is_empty() && cur.len() + para.len() + 1 > MAX_CHUNK_CHARS {
            chunks.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push('\n');
        }
        cur.push_str(para);
        // a single very long paragraph becomes its own (over-long) chunk
        if cur.len() >= MAX_CHUNK_CHARS {
            chunks.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        chunks.push(cur);
    }
    chunks.retain(|c| c.trim().chars().count() >= MIN_CHUNK_CHARS);
    chunks
}

impl Corpus {
    /// Ingest every `.md`/`.txt`/`.text`/`.tex` file under `dir` (recursively),
    /// chunk and embed each, using the default local Ollama endpoint.
    pub fn ingest(dir: &Path) -> Result<Self> {
        Self::ingest_with(dir, DEFAULT_OLLAMA_URL, DEFAULT_EMBED_MODEL)
    }

    /// Ingest against an explicit embedding endpoint/model. Errors if no readable
    /// text is found or embedding fails, so a `--corpus` request never silently
    /// runs un-grounded.
    pub fn ingest_with(dir: &Path, url: &str, model: &str) -> Result<Self> {
        let mut files = Vec::new();
        collect_text_files(dir, &mut files)?;
        files.sort();
        if files.is_empty() {
            return Err(anyhow!(
                "no .md/.txt/.text/.tex files found under {}",
                dir.display()
            ));
        }
        let mut chunks = Vec::new();
        for f in &files {
            let body =
                std::fs::read_to_string(f).with_context(|| format!("read {}", f.display()))?;
            let source = f
                .strip_prefix(dir)
                .unwrap_or(f)
                .to_string_lossy()
                .into_owned();
            for c in chunk_text(&body) {
                let embedding =
                    embed(&c, url, model).with_context(|| format!("embed chunk from {source}"))?;
                chunks.push(Chunk {
                    text: c,
                    source: source.clone(),
                    embedding,
                });
            }
        }
        if chunks.is_empty() {
            return Err(anyhow!("corpus produced no usable chunks"));
        }
        Ok(Self {
            chunks,
            url: url.to_string(),
            model: model.to_string(),
        })
    }

    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// Number of distinct source files represented.
    pub fn sources(&self) -> usize {
        self.chunks
            .iter()
            .map(|c| c.source.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    }

    /// Retrieve the `k` chunks most similar to `query_emb` (descending score).
    pub fn retrieve(&self, query_emb: &[f32], k: usize) -> Vec<&Chunk> {
        let mut scored: Vec<(f32, &Chunk)> = self
            .chunks
            .iter()
            .map(|c| (cosine(query_emb, &c.embedding), c))
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.into_iter().take(k).map(|(_, c)| c).collect()
    }

    /// Embed `query` and retrieve the top-k chunks (one local Ollama call).
    pub fn retrieve_text(&self, query: &str, k: usize) -> Result<Vec<&Chunk>> {
        let q = embed(query, &self.url, &self.model)?;
        Ok(self.retrieve(&q, k))
    }

    /// Format the top-k retrieved chunks as a labelled prompt block, or `None`
    /// if retrieval fails or returns nothing (caller falls back to transcript-only).
    pub fn context_block(&self, query: &str, k: usize) -> Option<String> {
        let hits = self.retrieve_text(query, k).ok()?;
        if hits.is_empty() {
            return None;
        }
        let mut s = String::from("RELEVANT MATERIAL FROM YOUR DOCUMENTS:\n");
        for c in hits {
            s.push_str(&format!("- [{}] {}\n", c.source, c.text.replace('\n', " ")));
        }
        Some(s)
    }
}

fn collect_text_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("read dir {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            collect_text_files(&path, out)?;
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if matches!(ext, "md" | "txt" | "text" | "tex") {
                out.push(path);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_basics() {
        assert!((cosine(&[1.0, 0.0, 0.0], &[1.0, 0.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6); // orthogonal
        assert!((cosine(&[1.0, 1.0], &[2.0, 2.0]) - 1.0).abs() < 1e-6); // same direction
        assert_eq!(cosine(&[1.0, 0.0], &[1.0]), 0.0); // length mismatch
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 1.0]), 0.0); // zero norm
    }

    #[test]
    fn chunking_splits_and_drops_fragments() {
        let doc = format!(
            "{}\n\n{}\n\nx",
            "A".repeat(600),
            "B".repeat(600) // forces a new chunk (600+600+1 > 900)
        );
        let chunks = chunk_text(&doc);
        assert_eq!(chunks.len(), 2); // two big paras; trailing "x" dropped (< MIN)
        assert!(chunks[0].starts_with('A'));
        assert!(chunks[1].starts_with('B'));
    }

    #[test]
    fn retrieve_ranks_by_cosine() {
        // Hand-built embeddings exercise the ranking logic without a network call.
        let corpus = Corpus {
            url: String::new(),
            model: String::new(),
            chunks: vec![
                Chunk {
                    text: "near".into(),
                    source: "a.md".into(),
                    embedding: vec![1.0, 0.1, 0.0],
                },
                Chunk {
                    text: "far".into(),
                    source: "b.md".into(),
                    embedding: vec![0.0, 0.0, 1.0],
                },
                Chunk {
                    text: "mid".into(),
                    source: "c.md".into(),
                    embedding: vec![0.7, 0.7, 0.0],
                },
            ],
        };
        let hits = corpus.retrieve(&[1.0, 0.0, 0.0], 2);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].text, "near"); // closest to the query direction
        assert_eq!(hits[1].text, "mid");
    }
}
