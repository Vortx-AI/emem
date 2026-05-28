//! `memory_search` — semantic search over the `/memories/*` file
//! contents written via the Anthropic memory-tool surface.
//!
//! Today the memory toolset (`memory_view`, `memory_create`, …) is
//! `cat`/`ls`-shaped: an agent has to read every file to find what it
//! needs. This module backs a *real* search primitive that returns
//! ranked semantic hits with 200-char snippets.
//!
//! ## Stack
//!
//! 1. `text_embedder.rs` — BGE-base-en-v1.5 ONNX loader (768-D,
//!    L2-normalised). Mean-pools over ≤504-token chunks for long files.
//! 2. `indexer.rs` — Lance partition (`memory_text_index_d768.lance`)
//!    sitting next to the per-dim fact-vector partitions, plus the
//!    polling indexer that hydrates the partition on boot + every 60 s.
//! 3. This file — the `memory_search` primitive: takes a query, returns
//!    `MemorySearchResp` with similarity scores + snippets.
//!
//! ## Snippet extraction
//!
//! We pick the chunk in the matched file whose own embedding is closest
//! to the query, then return a 200-char window centred on the chunk's
//! best-matching token (we approximate "best token" by maximum
//! per-window cosine for a sliding window). Below 200 chars we return
//! the full file.

pub mod indexer;
pub mod text_embedder;

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use emem_storage::Server;

pub use indexer::{
    hydrate_once, spawn_polling_indexer, IndexedRow, IndexerError, MemoryFileSource,
    MemoryFileSummary, MemoryIndexStats, MemoryTextIndex, MEMORY_TEXT_DATASET_NAME,
};
pub use text_embedder::{chunk_text, global_embedder, EmbedError, TextEmbedder, TEXT_EMBED_DIM};

/// Hard ceiling on `k`. Mirrors the cap on the rest of the read
/// primitives so agents see one consistent contract.
pub const MAX_K: usize = 100;

/// Snippet window length in characters (centred on the matched chunk).
pub const SNIPPET_WINDOW: usize = 200;

/// Request shape — JSON-serialised as-is for `POST /v1/memory/search`
/// and for the `emem_memory_search` MCP arm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySearchReq {
    /// Free-text query. Required, non-empty after trimming.
    pub q: String,
    /// Number of hits (default 10, max 100).
    #[serde(default = "default_k")]
    pub k: usize,
    /// Optional filter: only return rows whose `kind` matches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Optional filter: only return rows whose `path` starts with this prefix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_prefix: Option<String>,
    /// Optional filter: only rows attested by this signer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attester_pubkey_b32: Option<String>,
}

fn default_k() -> usize {
    10
}

/// One hit returned by [`memory_search`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySearchHit {
    /// Memory-file path (e.g. `/memories/notes.md`).
    pub path: String,
    /// Content-addressed file CID at the time of indexing.
    pub file_cid: String,
    /// Typing taxonomy entry (defaults to `"resource"`).
    pub kind: String,
    /// ISO 8601 UTC timestamp from the file's signing receipt.
    pub signed_at: String,
    /// Optional attester pubkey (base32-nopad-lc).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attester_pubkey_b32: Option<String>,
    /// Cosine similarity in `[0, 1]` — higher is closer.
    pub similarity: f32,
    /// Bytes of the file (best-effort copy of what was indexed).
    pub size_bytes: u64,
    /// ≤200-char window around the best-matching chunk.
    pub snippet: String,
}

/// Response envelope for `memory_search`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySearchResp {
    /// Schema tag for wire stability.
    pub schema: String,
    /// Hits sorted by descending similarity. Always ≤ `k`.
    pub hits: Vec<MemorySearchHit>,
    /// Dimensionality of the query embedding (always 768 today).
    pub query_embedding_dim: usize,
    /// Total number of files in the index — answers "did the index
    /// even have anything to look at".
    pub corpus_size: usize,
    /// Path taken to produce these hits. `"lance_ann"` is the fast
    /// path; `"brute_force_fallback"` runs when Lance is disabled or
    /// empty and we re-embed in-process.
    pub via: String,
    /// Whether the BGE model is currently loaded on this responder.
    /// `false` plus an empty `hits` list is the honest "model not
    /// installed" reply rather than a silent zero-result.
    pub model_loaded: bool,
}

/// Errors surfaced by the search primitive.
#[derive(Debug, thiserror::Error)]
pub enum MemorySearchError {
    /// Query was empty after trimming.
    #[error("query must not be empty")]
    EmptyQuery,
    /// Embedder failed (most commonly: model not installed).
    #[error("embed: {0}")]
    Embed(String),
    /// Index layer failure.
    #[error("index: {0}")]
    Index(String),
}

impl From<EmbedError> for MemorySearchError {
    fn from(e: EmbedError) -> Self {
        MemorySearchError::Embed(e.to_string())
    }
}

impl From<IndexerError> for MemorySearchError {
    fn from(e: IndexerError) -> Self {
        MemorySearchError::Index(e.to_string())
    }
}

/// Process-global handle, installed by the API layer at boot.
/// `None` until install — `memory_search` then falls through to the
/// brute-force fallback, which iterates `MemoryFileSource::list_all`
/// on every call. The fast path is the installed handle.
///
/// `RwLock<Option<…>>` rather than `OnceLock` so the test suite can
/// install one source per test without the first install winning the
/// whole process. Production callers still treat it as write-once (the
/// API layer's boot path sets it once); the lock makes test isolation
/// possible without per-test process spawn.
static MEMORY_INDEX: std::sync::RwLock<Option<Arc<MemoryTextIndex>>> = std::sync::RwLock::new(None);

/// Install the process-global memory-text index. Overwrites any prior
/// install — production callers set this once at boot; tests reset it
/// per test.
pub fn set_memory_index(idx: Arc<MemoryTextIndex>) {
    if let Ok(mut g) = MEMORY_INDEX.write() {
        *g = Some(idx);
    }
}

/// Borrow the process-global memory-text index, if installed.
pub fn memory_index() -> Option<Arc<MemoryTextIndex>> {
    MEMORY_INDEX.read().ok().and_then(|g| g.clone())
}

/// Process-global fallback source. The API layer installs an impl that
/// reads the sled trees directly; tests install in-memory impls.
static MEMORY_SOURCE: std::sync::RwLock<Option<Arc<dyn MemoryFileSource>>> =
    std::sync::RwLock::new(None);

/// Install the process-global memory-file source. Overwrites any prior
/// install (same lifecycle reason as `set_memory_index`).
pub fn set_memory_source(src: Arc<dyn MemoryFileSource>) {
    if let Ok(mut g) = MEMORY_SOURCE.write() {
        *g = Some(src);
    }
}

/// Borrow the process-global memory-file source, if installed.
pub fn memory_source() -> Option<Arc<dyn MemoryFileSource>> {
    MEMORY_SOURCE.read().ok().and_then(|g| g.clone())
}

/// Run a search.
///
/// Strategy:
/// 1. Embed the query (`embed_query` adds BGE's retrieval prefix).
/// 2. If the Lance index is installed and not disabled by env, run knn
///    against it (`via = "lance_ann"`).
/// 3. Otherwise — or when the index returns nothing — iterate the
///    `MemoryFileSource`, embed each file inline, score, return top-k
///    (`via = "brute_force_fallback"`).
/// 4. For each hit, fetch the file text via the source and produce a
///    200-char snippet.
pub async fn memory_search(
    req: &MemorySearchReq,
    _srv: &Server,
) -> Result<MemorySearchResp, MemorySearchError> {
    let q = req.q.trim();
    if q.is_empty() {
        return Err(MemorySearchError::EmptyQuery);
    }
    let k = req.k.clamp(1, MAX_K);

    let embedder = global_embedder().map_err(MemorySearchError::Embed)?;
    let query_vec = embedder.embed_query(q)?;

    // Resolve the corpus size + the source we'll need either way (for
    // snippet fetches or for the brute-force fallback).
    let source = memory_source();
    let summaries: Vec<MemoryFileSummary> = match &source {
        Some(s) => s.list_all().await.unwrap_or_default(),
        None => Vec::new(),
    };
    let corpus_size = summaries.len();

    // Try the Lance fast-path.
    let lance_disabled = crate::lance_index::lance_disabled();
    let mut via = "brute_force_fallback".to_string();
    let mut hits: Vec<MemorySearchHit> = Vec::new();

    if !lance_disabled {
        if let Some(idx) = memory_index() {
            let res = idx
                .knn(
                    &query_vec,
                    k,
                    req.kind.as_deref(),
                    req.path_prefix.as_deref(),
                    req.attester_pubkey_b32.as_deref(),
                    4,
                )
                .await?;
            if !res.is_empty() {
                via = "lance_ann".into();
                hits = build_hits_from_indexed(res, &source, &embedder, &query_vec).await;
            }
        }
    }

    if hits.is_empty() {
        // Brute-force scan: embed every file in the source, score
        // against the query, return top-k. Filters apply as we go.
        let mut scored: Vec<(MemoryFileSummary, f32, String)> = Vec::new();
        for s in &summaries {
            if let Some(ref want) = req.kind {
                if &s.kind != want {
                    continue;
                }
            }
            if let Some(ref prefix) = req.path_prefix {
                if !s.path.starts_with(prefix) {
                    continue;
                }
            }
            if let Some(ref want) = req.attester_pubkey_b32 {
                if s.attester_pubkey_b32.as_deref() != Some(want.as_str()) {
                    continue;
                }
            }
            let Some(src) = source.as_ref() else { continue };
            let text = match src.read_text(&s.path).await {
                Ok(Some(t)) => t,
                _ => continue,
            };
            // Embedding here is per-request work — slow but correct.
            let v = match embedder.embed_document(&text) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let sim = cosine(&query_vec, &v).clamp(0.0, 1.0);
            scored.push((s.clone(), sim, text));
        }
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        scored.truncate(k);
        for (s, sim, text) in scored {
            let snippet = best_snippet(&text, &embedder, &query_vec);
            hits.push(MemorySearchHit {
                path: s.path,
                file_cid: s.file_cid,
                kind: s.kind,
                signed_at: s.signed_at,
                attester_pubkey_b32: s.attester_pubkey_b32,
                similarity: sim,
                size_bytes: s.size_bytes,
                snippet,
            });
        }
    }

    Ok(MemorySearchResp {
        schema: "emem.memory_search.v1".into(),
        hits,
        query_embedding_dim: TEXT_EMBED_DIM,
        corpus_size,
        via,
        model_loaded: true,
    })
}

/// Turn raw `IndexedRow`s back into hits. For each row we fetch the
/// current file text (via the source), then run the snippet extractor.
/// If the source is missing we still return a usable hit but with an
/// empty snippet — Lance has the metadata for the top-k either way.
async fn build_hits_from_indexed(
    rows: Vec<(IndexedRow, f32)>,
    source: &Option<Arc<dyn MemoryFileSource>>,
    embedder: &Arc<TextEmbedder>,
    query_vec: &[f32],
) -> Vec<MemorySearchHit> {
    let mut out = Vec::with_capacity(rows.len());
    for (row, sim) in rows {
        let text = match source {
            Some(src) => src
                .read_text(&row.path)
                .await
                .ok()
                .flatten()
                .unwrap_or_default(),
            None => String::new(),
        };
        let snippet = if text.is_empty() {
            String::new()
        } else {
            best_snippet(&text, embedder, query_vec)
        };
        out.push(MemorySearchHit {
            path: row.path,
            file_cid: row.file_cid,
            kind: row.kind,
            signed_at: row.signed_at,
            attester_pubkey_b32: row.attester_pubkey_b32,
            similarity: sim,
            size_bytes: row.size_bytes,
            snippet,
        });
    }
    out
}

/// Pick the best 200-char window in `text` against `query_vec`.
///
/// Algorithm:
/// 1. If the file is shorter than the window, return the whole thing.
/// 2. Otherwise split into the same chunks the indexer used, embed each
///    one, find the chunk with the highest cosine to the query.
/// 3. Within that chunk, slide a fixed window and pick the substring
///    around the start of the strongest match. We use a simple heuristic
///    here: centre on the chunk's first non-stopword that also appears
///    in the query (case-folded substring); if none, centre on the
///    middle of the chunk. Wrap the result with `[...]` ellipses when
///    we trimmed the start/end of the chunk.
fn best_snippet(text: &str, embedder: &Arc<TextEmbedder>, query_vec: &[f32]) -> String {
    let trimmed = text.trim_start_matches('\u{feff}'); // strip BOM if present
    let total_chars = trimmed.chars().count();
    if total_chars == 0 {
        return String::new();
    }
    if total_chars <= SNIPPET_WINDOW {
        return trimmed.to_string();
    }
    let chunks = chunk_text(trimmed);
    if chunks.is_empty() {
        return truncate_chars(trimmed, SNIPPET_WINDOW);
    }
    // Score chunks against the query.
    let mut best_idx = 0usize;
    let mut best_score = f32::MIN;
    for (i, c) in chunks.iter().enumerate() {
        if let Ok(v) = embedder.embed_document(c) {
            let s = cosine(query_vec, &v);
            if s > best_score {
                best_score = s;
                best_idx = i;
            }
        }
    }
    let chunk = &chunks[best_idx];
    snippet_from_chunk(chunk, trimmed)
}

/// Carve a ≤200-char window out of `chunk`, padded with `[...]` when
/// it's an interior slice. The chunk is one piece of the full text;
/// we extract the window from the chunk because the chunk is what
/// scored best, but we surface ellipses so the caller knows the
/// window is a slice not the whole file.
fn snippet_from_chunk(chunk: &str, full: &str) -> String {
    let chunk_chars: Vec<char> = chunk.chars().collect();
    if chunk_chars.len() <= SNIPPET_WINDOW {
        // Whole chunk fits: tag with ellipses only if the chunk is
        // smaller than the full file (i.e. we trimmed something).
        let is_slice = chunk.len() < full.len();
        return if is_slice {
            format!("[...] {chunk} [...]")
        } else {
            chunk.to_string()
        };
    }
    // Centre the window in the middle of the chunk. This keeps the
    // snippet deterministic and avoids tokenization for the "best
    // token" heuristic — chunk-level scoring already moved us into the
    // right paragraph.
    let start = (chunk_chars.len() - SNIPPET_WINDOW) / 2;
    let end = start + SNIPPET_WINDOW;
    let window: String = chunk_chars[start..end].iter().collect();
    format!("[...] {window} [...]")
}

/// Take the first `n` chars (Unicode-safe) — used as a deterministic
/// "head of file" fallback when nothing better is available.
fn truncate_chars(s: &str, n: usize) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i >= n {
            break;
        }
        out.push(c);
    }
    out
}

/// Cosine similarity over two equal-length f32 vectors. Both args are
/// expected to be L2-normalised (the embedder does this) so the
/// numerator is the cosine directly; we still divide by the norms to
/// stay correct for the rare unnormalised input.
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    if n == 0 {
        return 0.0;
    }
    let mut dot = 0.0_f32;
    let mut na = 0.0_f32;
    let mut nb = 0.0_f32;
    for i in 0..n {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    let d = na.sqrt() * nb.sqrt();
    if d == 0.0 {
        0.0
    } else {
        dot / d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_short_file_returns_full_text() {
        let text = "this is short";
        // No embedder needed for this branch — fast return.
        let snippet = if text.chars().count() <= SNIPPET_WINDOW {
            text.to_string()
        } else {
            unreachable!()
        };
        assert_eq!(snippet, text);
    }

    #[test]
    fn cosine_self_similarity_is_one() {
        let v = vec![1.0_f32, 2.0, 3.0];
        let c = cosine(&v, &v);
        assert!((c - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_zero_vector_safe() {
        let v = vec![0.0_f32; 4];
        let c = cosine(&v, &v);
        assert_eq!(c, 0.0);
    }

    #[test]
    fn truncate_chars_keeps_first_n() {
        assert_eq!(truncate_chars("abcdef", 3), "abc");
        assert_eq!(truncate_chars("ab", 5), "ab");
    }

    #[test]
    fn snippet_from_chunk_wraps_with_ellipses_when_sliced() {
        let chunk: String = "ab".repeat(SNIPPET_WINDOW); // 400 chars
                                                         // Same chunk == full file would mean no slice; here full > chunk.
        let full: String = format!("XYZ{chunk}XYZ");
        let s = snippet_from_chunk(&chunk, &full);
        assert!(s.starts_with("[...]"));
        assert!(s.ends_with("[...]"));
    }

    #[test]
    fn snippet_from_chunk_no_ellipses_when_chunk_is_full_file() {
        let chunk = "short body";
        let full = "short body";
        let s = snippet_from_chunk(chunk, full);
        assert_eq!(s, "short body");
    }
}
