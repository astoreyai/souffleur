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
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

const DEFAULT_EMBED_MODEL: &str = "nomic-embed-text";
const MAX_CHUNK_CHARS: usize = 900; // a passage-sized chunk
const MIN_CHUNK_CHARS: usize = 40; // drop trivial fragments

/// The Ollama base URL, honoring `$OLLAMA_URL` so embeddings hit the same server
/// as the [`crate::suggest::OllamaBackend`] by default.
pub fn default_ollama_url() -> String {
    std::env::var("OLLAMA_URL").unwrap_or_else(|_| "http://localhost:11434".into())
}

// ----------------------------------------------------------------------------
// Persistent embedding cache: so an unchanged corpus is not re-embedded on every
// launch. Keyed per source file by (mtime, size, model); a file whose stamp is
// unchanged reuses its stored embeddings, a changed/new file is re-embedded, and
// a deleted file's rows are pruned. Bumped on any change to chunking.
// ----------------------------------------------------------------------------

const CACHE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct CachedChunk {
    text: String,
    embedding: Vec<f32>,
}

#[derive(Serialize, Deserialize)]
struct CacheEntry {
    version: u32,
    mtime_ns: u128,
    size: u64,
    model: String,
    chunks: Vec<CachedChunk>,
}

type Cache = HashMap<String, CacheEntry>;

/// Stable 64-bit FNV-1a hash (for naming the per-corpus cache file).
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Per-corpus cache file under `$XDG_CACHE_HOME`/`~/.cache`/`$TMPDIR`, named by a
/// stable hash of the corpus directory's absolute path.
fn cache_path_for(dir: &Path) -> PathBuf {
    let abs = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let mut base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    base.push("souffleur");
    base.push(format!(
        "corpus-{:016x}.bin",
        fnv1a(abs.to_string_lossy().as_bytes())
    ));
    base
}

fn load_cache(path: &Path) -> Cache {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| bincode::deserialize(&bytes).ok())
        .unwrap_or_default()
}

fn save_cache(path: &Path, cache: &Cache) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let bytes = bincode::serialize(cache).context("serialize embedding cache")?;
    std::fs::write(path, bytes).with_context(|| format!("write cache {}", path.display()))?;
    Ok(())
}

fn file_stamp(path: &Path) -> Result<(u128, u64)> {
    let meta = std::fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    Ok((mtime_ns, meta.len()))
}

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
    /// chunk and embed each, using `$OLLAMA_URL` and the default embed model.
    pub fn ingest(dir: &Path) -> Result<Self> {
        Self::ingest_with(dir, &default_ollama_url(), DEFAULT_EMBED_MODEL)
    }

    /// Like [`ingest`](Self::ingest) but with a caller-chosen embedding model
    /// (URL still from `$OLLAMA_URL`).
    pub fn ingest_model(dir: &Path, model: &str) -> Result<Self> {
        Self::ingest_with(dir, &default_ollama_url(), model)
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
        let cache_path = cache_path_for(dir);
        let mut cache = load_cache(&cache_path);
        let mut chunks = Vec::new();
        let (mut reused, mut embedded) = (0usize, 0usize);

        for f in &files {
            let key = f.to_string_lossy().into_owned();
            let source = f
                .strip_prefix(dir)
                .unwrap_or(f)
                .to_string_lossy()
                .into_owned();
            let (mtime_ns, size) = file_stamp(f)?;

            // Cache hit: an unchanged file (same stamp, model, cache version) reuses
            // its stored embeddings instead of calling the embedder again.
            if let Some(entry) = cache.get(&key) {
                if entry.version == CACHE_VERSION
                    && entry.size == size
                    && entry.mtime_ns == mtime_ns
                    && entry.model == model
                {
                    for cc in &entry.chunks {
                        chunks.push(Chunk {
                            text: cc.text.clone(),
                            source: source.clone(),
                            embedding: cc.embedding.clone(),
                        });
                    }
                    reused += entry.chunks.len();
                    continue;
                }
            }

            // Cache miss: (re)chunk + (re)embed and record the result for next time.
            let body =
                std::fs::read_to_string(f).with_context(|| format!("read {}", f.display()))?;
            let mut cached = Vec::new();
            for c in chunk_text(&body) {
                let embedding =
                    embed(&c, url, model).with_context(|| format!("embed chunk from {source}"))?;
                cached.push(CachedChunk {
                    text: c.clone(),
                    embedding: embedding.clone(),
                });
                chunks.push(Chunk {
                    text: c,
                    source: source.clone(),
                    embedding,
                });
                embedded += 1;
            }
            cache.insert(
                key,
                CacheEntry {
                    version: CACHE_VERSION,
                    mtime_ns,
                    size,
                    model: model.to_string(),
                    chunks: cached,
                },
            );
        }

        // Prune cache rows for files no longer present under dir.
        let present: HashSet<String> = files
            .iter()
            .map(|f| f.to_string_lossy().into_owned())
            .collect();
        cache.retain(|k, _| present.contains(k));

        // A cache write failure is non-fatal — ingestion already succeeded.
        if let Err(e) = save_cache(&cache_path, &cache) {
            eprintln!("[corpus] warning: could not write embedding cache: {e:#}");
        }
        eprintln!("[corpus] embeddings: {reused} reused from cache, {embedded} freshly embedded");

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
    fn cache_entry_bincode_roundtrips() {
        let mut map: Cache = HashMap::new();
        map.insert(
            "a.md".into(),
            CacheEntry {
                version: CACHE_VERSION,
                mtime_ns: 123,
                size: 45,
                model: "nomic-embed-text".into(),
                chunks: vec![CachedChunk {
                    text: "hi".into(),
                    embedding: vec![0.1, 0.2, 0.3],
                }],
            },
        );
        let bytes = bincode::serialize(&map).unwrap();
        let back: Cache = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back["a.md"].chunks[0].embedding, vec![0.1, 0.2, 0.3]);
        assert_eq!(back["a.md"].size, 45);
        assert_eq!(back["a.md"].version, CACHE_VERSION);
    }

    #[test]
    fn fnv1a_is_stable_and_distinct() {
        assert_eq!(fnv1a(b"/home/aaron/thesis"), fnv1a(b"/home/aaron/thesis"));
        assert_ne!(fnv1a(b"/a"), fnv1a(b"/b"));
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
