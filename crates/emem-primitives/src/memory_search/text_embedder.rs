//! BGE-base-en-v1.5 ONNX text embedder for the memory-file search index.
//!
//! Loads `model.onnx` + `tokenizer.json` from
//! `<EMEM_DATA>/models/bge-base-en-v1.5/` (overridable via
//! `EMEM_MEMORY_SEARCH_MODEL_DIR` or, as a compatibility fallback,
//! `EMEM_TOPIC_MODEL_DIR` — the topic-router env var, which points at
//! the same model). Output is 768-D, CLS-pooled, L2-normalised — the
//! same shape used by the topic router so an operator who has the model
//! installed for `/v1/ask` automatically has it for memory search.
//!
//! Why this isn't in `emem-api-rest::topic_router`: `emem-primitives` is
//! upstream of `emem-api-rest`, so it can't import a private helper from
//! the API crate. We duplicate the loader here. The two helpers stay
//! pin-compatible by both reading the same on-disk model directory.
//!
//! For long files we tokenize once, window the ids at the model's real
//! 512-token limit, embed each window, and mean-pool into a single 768-D
//! vector. The mean-pool path matches BGE's recommended use for
//! sentence-level retrieval and keeps the Lance schema flat (one row
//! per file, not one per chunk).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use tokenizers::Tokenizer;

/// Output dimensionality of `BAAI/bge-base-en-v1.5`. Hard-coded because
/// the Lance partition schema is `FixedSizeList<Float32, 768>` — any
/// model swap that changes the dim must update both.
pub const TEXT_EMBED_DIM: usize = 768;

/// BGE's context window, in real sub-word tokens. This is a hard limit:
/// the position embedding is a `[512, hidden]` table, so a longer sequence
/// does not degrade, it fails the graph outright with
/// `Attempting to broadcast an axis by a dimension other than 1. 512 by N`.
const MODEL_MAX_TOKENS: usize = 512;

/// Real tokens per window, leaving room for the `[CLS]` and `[SEP]` this
/// module adds around each one.
const WINDOW_TOKENS: usize = MODEL_MAX_TOKENS - 2;

/// Approximate per-chunk sub-word budget, and the assumed character cost
/// of a token. Used ONLY to cut text into snippet-sized pieces, never to
/// decide what the model can accept: see `embed_document`, which windows
/// real token ids. BGE's WordPiece averages ~4 chars/token on English,
/// which is what these encode; text that is not English prose can run far
/// denser, and that assumption is exactly what broke embedding here.
const CHUNK_BUDGET_TOKENS: usize = 504;
const CHARS_PER_TOKEN_APPROX: usize = 4;

/// Errors surfaced by the text embedder.
#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    /// Model directory not found, or missing `model.onnx` /
    /// `tokenizer.json`.
    #[error("model not present: {0}")]
    ModelMissing(String),
    /// Tokenizer load / encode failure.
    #[error("tokenizer: {0}")]
    Tokenizer(String),
    /// ORT session create / run failure.
    #[error("ort: {0}")]
    Ort(String),
    /// Output shape mismatch (e.g. the model's hidden-dim ≠ 768).
    #[error("shape: {0}")]
    Shape(String),
}

/// Holds the loaded ORT session and tokenizer. Construct once at boot;
/// the session is behind a mutex so concurrent embeds serialise (ORT
/// 2.x `Session::run` requires `&mut self`).
pub struct TextEmbedder {
    session: Arc<Mutex<ort::session::Session>>,
    tokenizer: Arc<Tokenizer>,
    /// Filesystem directory the model was loaded from. Surfaced in
    /// stats / errors so operators can confirm what's in flight.
    pub model_dir: PathBuf,
}

impl TextEmbedder {
    /// Resolve the model directory and open it. Looks at
    /// `EMEM_MEMORY_SEARCH_MODEL_DIR` first, then `EMEM_TOPIC_MODEL_DIR`
    /// (the topic-router knob — same model), then falls back to
    /// `<EMEM_DATA>/models/bge-base-en-v1.5/`.
    pub fn open_default() -> Result<Self, EmbedError> {
        let dir = resolve_model_dir()?;
        Self::open(&dir)
    }

    /// Open the embedder against an explicit model directory.
    pub fn open(model_dir: &Path) -> Result<Self, EmbedError> {
        let onnx_candidates = [
            model_dir.join("model.onnx"),
            model_dir.join("onnx").join("model.onnx"),
        ];
        let onnx_path = onnx_candidates
            .iter()
            .find(|p| p.is_file())
            .ok_or_else(|| {
                EmbedError::ModelMissing(format!(
                    "no model.onnx under {model_dir:?} (looked at {onnx_candidates:?}). \
                     Run scripts/install-topic-model.sh to populate it."
                ))
            })?
            .clone();
        let tokenizer_json = model_dir.join("tokenizer.json");
        if !tokenizer_json.is_file() {
            return Err(EmbedError::ModelMissing(format!(
                "no tokenizer.json at {tokenizer_json:?}"
            )));
        }
        let tokenizer = Tokenizer::from_file(&tokenizer_json)
            .map_err(|e| EmbedError::Tokenizer(format!("load {tokenizer_json:?}: {e}")))?;

        // `ort::init().commit()` is idempotent — the topic router may
        // have already initialised the runtime. The Result is discarded
        // because a second-init returns a benign error.
        let _ = ort::init().commit();

        let session = ort::session::Session::builder()
            .map_err(|e| EmbedError::Ort(format!("session builder: {e}")))?
            .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level3)
            .map_err(|e| EmbedError::Ort(format!("opt level: {e}")))?
            .with_intra_threads(2)
            .map_err(|e| EmbedError::Ort(format!("intra threads: {e}")))?
            .commit_from_file(&onnx_path)
            .map_err(|e| EmbedError::Ort(format!("load {onnx_path:?}: {e}")))?;

        Ok(Self {
            session: Arc::new(Mutex::new(session)),
            tokenizer: Arc::new(tokenizer),
            model_dir: model_dir.to_path_buf(),
        })
    }

    /// The `[CLS]` / `[SEP]` ids this module wraps each window in.
    ///
    /// Read from the tokenizer rather than hardcoded, so a model whose
    /// vocabulary numbers them differently is a typed error at the first
    /// embed instead of a silently wrong vector.
    fn special_ids(&self) -> Result<(u32, u32), EmbedError> {
        let cls = self
            .tokenizer
            .token_to_id("[CLS]")
            .ok_or_else(|| EmbedError::Tokenizer("tokenizer has no [CLS] token".into()))?;
        let sep = self
            .tokenizer
            .token_to_id("[SEP]")
            .ok_or_else(|| EmbedError::Tokenizer("tokenizer has no [SEP] token".into()))?;
        Ok((cls, sep))
    }

    /// Embed one window of real token ids, which must already be at most
    /// `WINDOW_TOKENS` long. `[CLS]` and `[SEP]` are added here.
    ///
    /// Takes ids rather than text because the length that matters to the
    /// model is measured in tokens, and only the tokenizer knows it. The
    /// previous version took text and trusted a character-count estimate
    /// to have kept it short enough, which is the bug this closes.
    fn embed_window(&self, window: &[u32], cls: u32, sep: u32) -> Result<Vec<f32>, EmbedError> {
        use ort::value::Tensor;

        if window.is_empty() {
            return Ok(vec![0.0_f32; TEXT_EMBED_DIM]);
        }
        if window.len() > WINDOW_TOKENS {
            return Err(EmbedError::Shape(format!(
                "window of {} tokens exceeds the {WINDOW_TOKENS}-token budget; \
                 embed_document must window before calling this",
                window.len()
            )));
        }
        let mut ids: Vec<i64> = Vec::with_capacity(window.len() + 2);
        ids.push(cls as i64);
        ids.extend(window.iter().map(|&x| x as i64));
        ids.push(sep as i64);
        let n = ids.len();
        // One segment, nothing padded: every position is real.
        let mask: Vec<i64> = vec![1; n];
        let tt: Vec<i64> = vec![0; n];

        let ids_t = Tensor::from_array(([1, n], ids))
            .map_err(|e| EmbedError::Ort(format!("ids tensor: {e}")))?;
        let mask_t = Tensor::from_array(([1, n], mask))
            .map_err(|e| EmbedError::Ort(format!("mask tensor: {e}")))?;
        let tt_t = Tensor::from_array(([1, n], tt))
            .map_err(|e| EmbedError::Ort(format!("tt tensor: {e}")))?;

        let mut guard = self
            .session
            .lock()
            .map_err(|e| EmbedError::Ort(format!("session mutex poisoned: {e}")))?;
        let outputs = guard
            .run(ort::inputs![
                "input_ids" => ids_t,
                "attention_mask" => mask_t,
                "token_type_ids" => tt_t,
            ])
            .map_err(|e| EmbedError::Ort(format!("run: {e}")))?;
        let (_name, last_hidden) = outputs
            .iter()
            .next()
            .ok_or_else(|| EmbedError::Ort("no outputs".into()))?;
        let arr = last_hidden
            .try_extract_array::<f32>()
            .map_err(|e| EmbedError::Ort(format!("extract: {e}")))?;
        // Shape is [batch=1, seq=n, hidden=dim]. CLS = token 0.
        let batch0 = arr.index_axis(ort_ndarray::Axis(0), 0);
        let cls = batch0.index_axis(ort_ndarray::Axis(0), 0);
        let v: Vec<f32> = cls.iter().copied().collect();
        if v.len() != TEXT_EMBED_DIM {
            return Err(EmbedError::Shape(format!(
                "expected {TEXT_EMBED_DIM}-D output, got {}-D",
                v.len()
            )));
        }
        Ok(l2_normalised(v))
    }

    /// Embed a full document: tokenize once, window the ids at the model's
    /// real context limit, embed each window, mean-pool to one 768-D
    /// L2-normalised vector.
    ///
    /// The document is measured in tokens rather than characters because
    /// the limit it has to respect is counted in tokens. Windowing text by
    /// a `chars / 4` estimate held for English prose and broke on
    /// everything else: a chunk of base32 CIDs, tables, code or URLs packs
    /// far more sub-words into the same 2016 characters, so the model was
    /// handed 769 and 946-token sequences against a 512 limit and threw.
    /// Every memory file dense enough to overrun failed on every pass,
    /// forever, and was never indexed at all. Notably that included every
    /// message in the agent-to-agent channel, whose whole substance is
    /// content-addressed CIDs.
    pub fn embed_document(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        let enc = self
            .tokenizer
            .encode(text, false)
            .map_err(|e| EmbedError::Tokenizer(format!("encode: {e}")))?;
        let ids = enc.get_ids();
        if ids.is_empty() {
            return Ok(vec![0.0_f32; TEXT_EMBED_DIM]);
        }
        let (cls, sep) = self.special_ids()?;
        let windows: Vec<&[u32]> = ids.chunks(WINDOW_TOKENS).collect();
        if windows.len() == 1 {
            return self.embed_window(windows[0], cls, sep);
        }
        let mut sum = vec![0.0_f32; TEXT_EMBED_DIM];
        for w in &windows {
            let v = self.embed_window(w, cls, sep)?;
            for (i, x) in v.iter().enumerate() {
                sum[i] += *x;
            }
        }
        let nf = windows.len() as f32;
        for x in &mut sum {
            *x /= nf;
        }
        Ok(l2_normalised(sum))
    }

    /// Embed a query string. Same as `embed_document` but additionally
    /// applies BGE's recommended retrieval prefix
    /// `"Represent this sentence for searching relevant passages: "`.
    /// Improves cosine separation between query and passage embeddings.
    pub fn embed_query(&self, query: &str) -> Result<Vec<f32>, EmbedError> {
        // BGE v1.5 recommends the retrieval prefix for the *query* side
        // only (the corpus side stays unprefixed). Applying it to a
        // short query string keeps the document chunker happy — 504
        // tokens is far more than a real-world query.
        let prefixed = format!(
            "Represent this sentence for searching relevant passages: {}",
            query
        );
        self.embed_document(&prefixed)
    }
}

/// Split a long string into roughly snippet-sized pieces on whitespace,
/// never breaking mid-word.
///
/// This is a TEXT splitter, used to pick which part of a file to show as a
/// search snippet. It is deliberately not what protects the model: its
/// budget is characters over an assumed 4 chars/token, an estimate that is
/// wrong by 50% or more on anything but English prose. `embed_document`
/// windows real token ids instead. Do not reintroduce a caller that trusts
/// these chunks to fit the context.
pub fn chunk_text(text: &str) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let char_budget = CHUNK_BUDGET_TOKENS.saturating_mul(CHARS_PER_TOKEN_APPROX);
    if trimmed.chars().count() <= char_budget {
        return vec![trimmed.to_string()];
    }
    let mut out = Vec::new();
    let mut current = String::new();
    let mut current_chars: usize = 0;
    for word in trimmed.split_whitespace() {
        let w_chars = word.chars().count();
        if !current.is_empty() && current_chars + 1 + w_chars > char_budget {
            out.push(std::mem::take(&mut current));
            current_chars = 0;
        }
        if !current.is_empty() {
            current.push(' ');
            current_chars += 1;
        }
        // A pathologically long single word still goes in its own chunk.
        // Harmless now: the model is protected by token windowing, not by
        // this. It was not harmless when this text went straight to the
        // encoder.
        current.push_str(word);
        current_chars += w_chars;
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// L2-normalise in place; returns the same vector for caller chaining.
fn l2_normalised(mut v: Vec<f32>) -> Vec<f32> {
    let mut s = 0.0_f32;
    for x in &v {
        s += x * x;
    }
    let n = s.sqrt();
    if n > 0.0 {
        for x in v.iter_mut() {
            *x /= n;
        }
    }
    v
}

/// Resolve the model directory from env. See `TextEmbedder::open_default`.
fn resolve_model_dir() -> Result<PathBuf, EmbedError> {
    if let Ok(d) = std::env::var("EMEM_MEMORY_SEARCH_MODEL_DIR") {
        return Ok(PathBuf::from(d));
    }
    if let Ok(d) = std::env::var("EMEM_TOPIC_MODEL_DIR") {
        return Ok(PathBuf::from(d));
    }
    if let Ok(d) = std::env::var("EMEM_DATA") {
        return Ok(PathBuf::from(d).join("models").join("bge-base-en-v1.5"));
    }
    Err(EmbedError::ModelMissing(
        "neither EMEM_MEMORY_SEARCH_MODEL_DIR, EMEM_TOPIC_MODEL_DIR nor EMEM_DATA is set".into(),
    ))
}

/// Process-global embedder. Lazy-init on first call so a server that
/// never touches memory search never pays the ONNX load cost.
/// `OnceLock<Result<…>>` so a missing-model error is returned to every
/// caller without re-trying on every request.
static EMBEDDER: OnceLock<Result<Arc<TextEmbedder>, String>> = OnceLock::new();

/// Borrow the process-global text embedder. Lazy — first call loads the
/// ONNX model (~1-2 s); subsequent calls are O(1). Returns the same
/// error string to every caller when the model is missing — the smoke
/// test surfaces it as a clear failure rather than silently degrading
/// to random vectors.
pub fn global_embedder() -> Result<Arc<TextEmbedder>, String> {
    EMBEDDER
        .get_or_init(|| {
            TextEmbedder::open_default()
                .map(Arc::new)
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map(Arc::clone)
        .map_err(|e| e.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_text_keeps_short_strings_intact() {
        let chunks = chunk_text("hello world");
        assert_eq!(chunks, vec!["hello world".to_string()]);
    }

    #[test]
    fn chunk_text_splits_long_strings() {
        let long = "word ".repeat(10_000);
        let chunks = chunk_text(&long);
        assert!(
            chunks.len() > 1,
            "expected multiple chunks, got {}",
            chunks.len()
        );
        for c in &chunks {
            // No chunk should grossly overshoot the budget.
            assert!(
                c.chars().count() <= CHUNK_BUDGET_TOKENS * CHARS_PER_TOKEN_APPROX + 8,
                "chunk too long: {} chars",
                c.chars().count()
            );
        }
    }

    #[test]
    fn chunk_text_empty_input_returns_empty() {
        assert!(chunk_text("").is_empty());
        assert!(chunk_text("   \n\t  ").is_empty());
    }

    #[test]
    fn l2_normalised_unit_vector() {
        let v = l2_normalised(vec![3.0, 4.0]);
        let mag: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((mag - 1.0).abs() < 1e-6, "mag = {mag}");
    }

    /// A memory file made of content-addressed CIDs must embed.
    ///
    /// This is the shape that broke: `chunk_text` budgets characters at an
    /// assumed 4 chars/token, so it called a 2016-character block one chunk,
    /// and the encoder turned it into 769 sub-words against a 512-token
    /// limit. ONNX refused the graph, the indexer logged
    /// "embed/index failed; file will be retried on next pass", and retried
    /// it forever. Live, 24 files were stuck in that loop, including every
    /// message in the agent-to-agent channel, whose substance is base32
    /// CIDs. `chunk_text_splits_long_strings` above stayed green throughout,
    /// because it measures the characters that were never the problem.
    ///
    /// Needs the real tokenizer to mean anything, since the whole bug is a
    /// disagreement between estimated and real token counts, so it is
    /// ignored by default rather than skipped silently on a runner with no
    /// model.
    #[test]
    #[ignore = "needs the on-disk bge-base-en-v1.5 model — run with --ignored"]
    fn a_document_of_cids_embeds_rather_than_overrunning_the_context() {
        let embedder = match TextEmbedder::open_default() {
            Ok(e) => e,
            Err(e) => panic!("model not available: {e}"),
        };
        // A real token from the channel, repeated: dense base32 with no
        // English structure for WordPiece to latch onto.
        let cid_doc = "emem:fact:defi.zb572.towe.zae65:zeal26ciu76uuksgdvacjsytza ".repeat(200);

        // First, pin the disagreement rather than assert it from memory:
        // the text splitter thinks this is few chunks, the tokenizer knows
        // better.
        let enc = embedder.tokenizer.encode(cid_doc.as_str(), false).unwrap();
        let real_tokens = enc.get_ids().len();
        for c in chunk_text(&cid_doc) {
            let n = embedder.tokenizer.encode(c.as_str(), false).unwrap();
            if n.get_ids().len() > MODEL_MAX_TOKENS {
                // This is the bug, still present in the text splitter and
                // now harmless. If this stops being true the test below is
                // no longer proving anything, so say so loudly.
                let v = embedder.embed_document(&cid_doc).expect(
                    "a CID-dense document must embed; the encoder is fed \
                     token windows, not chars/4 guesses",
                );
                assert_eq!(v.len(), TEXT_EMBED_DIM);
                return;
            }
        }
        panic!(
            "expected chunk_text to produce a chunk over {MODEL_MAX_TOKENS} real tokens \
             ({real_tokens} tokens across the document); if the splitter got token-aware, \
             rewrite this test against whatever now overruns"
        );
    }
}
