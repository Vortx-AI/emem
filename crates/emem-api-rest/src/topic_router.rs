//! Transformer-routed topic dispatch for `/v1/ask` and `emem_ask`.
//!
//! Replaces the hardcoded `TOPIC_KEYWORDS` table (shipped through
//! 0.0.2 in `lib.rs`) with semantic similarity over a sentence-
//! transformer embedding. Each topic in `topics-v0.json` has a
//! `description` plus a list of `aliases[]`; we embed all of them
//! once at startup, average per-topic to get a centroid vector, then
//! at query time embed the user question and pick every topic whose
//! cosine similarity exceeds a threshold.
//!
//! Why a transformer instead of substring matching:
//!
//!   - Paraphrases route correctly. "is the lake going to overflow"
//!     and "could this place flood" land on the same topic without
//!     needing a hand-curated keyword list per topic.
//!   - Adding a new topic is a JSON edit (add to `topics-v0.json`),
//!     not a code edit (touch a 300-line `&'static str` table in
//!     `lib.rs`). The router is the kind of thing that should be
//!     data-driven, and now it is.
//!
//! Three switchable backends:
//!
//!   1. **`ort`** (default since 2026-05-04 — replacing the
//!      fastembed-rs wrapper that deadlocked on session create on
//!      this host) — direct `ort` 2.x + `tokenizers` BERT inference.
//!      Loads a `tokenizer.json` + `model.onnx` pair from
//!      `EMEM_TOPIC_MODEL_DIR` (default
//!      `<EMEM_DATA>/models/bge-base-en-v1.5/`). Default model is
//!      `BAAI/bge-base-en-v1.5` (110 M params, 768-D, MTEB ~63);
//!      pooling is CLS, output is L2-normalised. Microsoft's bundled
//!      CPU ORT (vendored via the `download-binaries` ort feature)
//!      runs the 25-topic centroid pass in ~50 ms total. GPU re-enable
//!      goes through a known-good libonnxruntime — the
//!      /opt/onnxruntime-1.22.0-cuda12 build deadlocks on session
//!      create on this host (verified with isolated /tmp/orttest).
//!
//!   2. **`model2vec`** — pure-Rust static-distillation token-lookup
//!      embedder, `minishlab/potion-base-8M`, 256-D, ~32 MB, sub-µs
//!      per question, no ONNX dep. Used as the air-gapped /
//!      no-libonnxruntime fallback.
//!
//!   3. **`keyword`** — substring search over `aliases[]` + topic
//!      key. Used in unit tests, lib-only builds, and as the last-
//!      resort fallback when both ort and model2vec fail to load.
//!
//! Configuration (env vars):
//!
//!   - `EMEM_TOPIC_BACKEND` — `ort` (default), `model2vec`,
//!     or `keyword`. The legacy alias `transformer` resolves to
//!     `model2vec`. The legacy alias `fastembed` resolves to `ort`
//!     (we silently swapped the backend behind the same name on
//!     2026-05-04).
//!   - `EMEM_TOPIC_MODEL_DIR` — where the ort backend reads
//!     `tokenizer.json` and `model.onnx` from. Default
//!     `<EMEM_DATA>/models/bge-base-en-v1.5/`.
//!   - `EMEM_TOPIC_THRESHOLD` — cosine threshold below which a topic
//!     is discarded. Default `0.35`.
//!   - `EMEM_TOPIC_USE_GPU=1` — append `CUDAExecutionProvider` to the
//!     ort session. Off by default because the CUDA-built
//!     libonnxruntime on this host hangs at session create.

use std::sync::OnceLock;

use emem_core::topics::TopicRegistry;

/// One topic match: the topic key + the cosine similarity that
/// produced it.
#[derive(Debug, Clone)]
pub struct TopicMatch {
    /// Topic key — same as `Topic.key`.
    pub key: String,
    /// Similarity score in `[-1, 1]` (cosine on L2-normalised
    /// vectors). For keyword matches, this is a synthetic score
    /// derived from substring length and position.
    pub score: f32,
    /// Which backend produced this match: `"transformer"` or
    /// `"keyword"`. Surfaced in the `/v1/ask` response so an
    /// operator can audit whether the model is actually loading.
    pub via: &'static str,
}

/// Question phrases that route to `out_of_scope`. emem only knows the
/// planet — climate, terrain, atmosphere, vegetation, oceans. These
/// patterns catch the failure modes the consumer eval surfaced where
/// the BERT cosine layer returns 5 weak (0.35-0.45) topic hits for
/// nonsense input because every English noun phrase has *some* signal.
///
/// Keep these as lowercase substrings — `is_out_of_scope` lowercases
/// the question once and runs `contains()` on each pattern. Order is
/// don't-care; matching is set-membership not priority.
///
/// Adding a pattern: only if you've seen it surface as a false-positive
/// on a real eval question. Don't pre-emptively expand — the fix
/// surface for a missed false-positive is one entry; the cost of a
/// false-negative deny is shutting out a legitimate place query.
const OUT_OF_SCOPE_PATTERNS: &[&str] = &[
    // Politics / current affairs
    "who won",
    "election",
    "elections",
    "who is the president",
    "current president",
    "current prime minister",
    "world cup",
    "super bowl",
    "olympic medal",
    // Philosophy / chitchat
    "meaning of life",
    "what's the meaning",
    "tell me a joke",
    "write a poem",
    "are you sentient",
    "are you conscious",
    // Markets / crypto
    "stock price",
    "stock market",
    "share price",
    "crypto price",
    "bitcoin price",
    "ethereum price",
    "nasdaq",
    // Personal advice unrelated to place
    "what should i eat",
    "recipe for",
    "how do i lose weight",
    // LLM trick prompts
    "ignore previous",
    "system prompt",
    "you are now",
];

/// Returns true when `q` should be treated as out-of-scope before any
/// topic routing runs. Case-insensitive substring match against
/// [`OUT_OF_SCOPE_PATTERNS`].
fn is_out_of_scope(q: &str) -> bool {
    let q_low = q.to_lowercase();
    OUT_OF_SCOPE_PATTERNS.iter().any(|p| q_low.contains(p))
}

/// Public router handle. Cheap to clone (everything inside is
/// `Arc`-shared). The first call constructs the embedder and
/// pre-computes topic centroids; subsequent calls reuse them.
#[derive(Clone)]
pub struct TopicRouter {
    inner: std::sync::Arc<TopicRouterInner>,
}

struct TopicRouterInner {
    backend: Backend,
    /// Snapshot of the topic registry at the time the router was
    /// built. Used by both backends to look up bands/algorithms.
    registry: TopicRegistry,
    /// Cosine threshold for the transformer backend. Ignored by the
    /// keyword backend.
    threshold: f32,
    /// Hard cap on the number of topics one question can match.
    max_topics: usize,
}

enum Backend {
    /// Direct `ort` 2.x + `tokenizers` BERT-style sentence embedder.
    /// Loads `tokenizer.json` + `model.onnx` from a local directory.
    /// Pooling is CLS-token (token 0 of `last_hidden_state`); output
    /// is L2-normalised before being stored as a centroid. Default
    /// model is `BAAI/bge-base-en-v1.5` (768-D). Centroid + per-
    /// query inference take ~9 ms on CPU on the bundled Microsoft
    /// ORT 1.22.
    Ort {
        session: std::sync::Arc<std::sync::Mutex<ort::session::Session>>,
        tokenizer: std::sync::Arc<tokenizers::Tokenizer>,
        centroids: Vec<Vec<f32>>,
        model_id: String,
    },
    /// Static-distillation token-lookup fallback: `model2vec` /
    /// `potion-base-8M`, 256-D, ~32 MB, sub-µs per question, pure
    /// Rust. Used when ort cannot load or when
    /// `EMEM_TOPIC_BACKEND=model2vec`.
    Transformer {
        model: std::sync::Arc<model2vec_rs::model::StaticModel>,
        centroids: Vec<Vec<f32>>,
    },
    /// Substring-search fallback: pre-lowercased aliases + key per
    /// topic. Used when both ort and model2vec load fail, or when
    /// `EMEM_TOPIC_BACKEND=keyword`.
    Keyword { aliases_per_topic: Vec<Vec<String>> },
}

/// Process-wide cached router. Lazy to defer the model download to
/// the first `/v1/ask` call rather than blocking startup.
static ROUTER: OnceLock<TopicRouter> = OnceLock::new();

impl TopicRouter {
    /// Get the process-wide router, constructing it on first call.
    pub fn global() -> &'static TopicRouter {
        ROUTER.get_or_init(TopicRouter::build)
    }

    /// Build the router: load the embedder, embed every topic
    /// description + alias, average per-topic to get a centroid.
    /// Falls back to keyword backend on any error (logged at WARN).
    fn build() -> TopicRouter {
        let registry = (*emem_core::topics::DEFAULT).clone();

        let policy = registry.routing.as_ref();
        // Last-resort fallback if neither env var nor registry supplies a
        // threshold. `topics-v0.json._threshold_learned_from` documents the
        // provenance; this const exists so the topic router still works
        // when an older registry CID is loaded that omits the field.
        const DEFAULT_TOPIC_THRESHOLD: f32 = 0.35;
        let threshold: f32 = std::env::var("EMEM_TOPIC_THRESHOLD")
            .ok()
            .and_then(|s| s.parse().ok())
            .or_else(|| policy.and_then(|p| p.threshold))
            .unwrap_or(DEFAULT_TOPIC_THRESHOLD);
        let max_topics: usize = policy.and_then(|p| p.max_topics_per_question).unwrap_or(5);

        // Default = ort (direct ort + tokenizers BERT inference).
        // Aliases:
        //   - `fastembed` resolves to `ort` (we replaced fastembed-rs
        //     with direct ort calls on 2026-05-04 because the wrapper
        //     deadlocked at session create on this host).
        //   - `transformer` resolves to `model2vec` (legacy name).
        //   - `keyword` skips both for unit tests / air-gapped builds.
        let want_backend = std::env::var("EMEM_TOPIC_BACKEND").unwrap_or_else(|_| "ort".into());
        let want_backend = match want_backend.as_str() {
            "fastembed" => "ort".to_string(),
            "transformer" => "model2vec".to_string(),
            other => other.to_string(),
        };

        if want_backend == "keyword" {
            tracing::info!(
                target: "emem::topic_router",
                "EMEM_TOPIC_BACKEND=keyword — skipping transformer load"
            );
            return TopicRouter::keyword_only(registry, threshold, max_topics);
        }

        if want_backend == "ort" {
            match TopicRouter::try_ort(&registry, threshold, max_topics) {
                Ok(r) => return r,
                Err(e) => {
                    tracing::warn!(
                        target: "emem::topic_router",
                        error = %e,
                        "topic router: ort load failed; falling back to model2vec"
                    );
                }
            }
        }

        // For the model2vec backend the model id is hardcoded —
        // `policy.transformer_model` in topics-v0.json points at the
        // fastembed default (e.g. `BAAI/bge-base-en-v1.5`), which
        // model2vec_rs can't load (it expects a static-distillation
        // tokenizer + embedding table, not an ONNX file). Use the
        // registry policy ONLY when an env override forces a
        // model2vec-compatible repo; otherwise fall back to the only
        // model2vec we ship.
        let model_id = std::env::var("EMEM_TOPIC_MODEL")
            .ok()
            .filter(|m| m.starts_with("minishlab/"))
            .unwrap_or_else(|| "minishlab/potion-base-8M".into());

        // Note: we *don't* set HF_HOME here even when EMEM_DATA is
        // configured — modifying process env from a multi-threaded
        // server is unsafe in newer Rust. Operators that want the
        // model cache under EMEM_DATA can set HF_HOME themselves at
        // service-startup time (systemd unit, container env, etc.).
        // The default cache (`~/.cache/huggingface`) is fine for
        // most deployments.

        match model2vec_rs::model::StaticModel::from_pretrained(&model_id, None, None, None) {
            Ok(model) => {
                tracing::info!(
                    target: "emem::topic_router",
                    model = %model_id,
                    threshold = threshold,
                    "topic router: model loaded; embedding topic corpus"
                );
                let centroids = compute_centroids(&model, &registry);
                TopicRouter {
                    inner: std::sync::Arc::new(TopicRouterInner {
                        backend: Backend::Transformer {
                            model: std::sync::Arc::new(model),
                            centroids,
                        },
                        registry,
                        threshold,
                        max_topics,
                    }),
                }
            }
            Err(e) => {
                tracing::warn!(
                    target: "emem::topic_router",
                    error = %e,
                    model = %model_id,
                    "topic router: model load failed; falling back to keyword backend"
                );
                TopicRouter::keyword_only(registry, threshold, max_topics)
            }
        }
    }

    /// Build the ort-backed router. Reads `tokenizer.json` and
    /// `model.onnx` from a local directory — defaults to
    /// `<EMEM_DATA>/models/bge-base-en-v1.5/` and overridable via
    /// `EMEM_TOPIC_MODEL_DIR`. The ONNX file may be at the directory
    /// root or under an `onnx/` subdirectory (matching both
    /// `BAAI/*` and `Xenova/*` mirror conventions). All operations
    /// are local-only — no hf-hub, no network. Errors propagate so
    /// the caller can drop to model2vec.
    fn try_ort(
        registry: &TopicRegistry,
        threshold: f32,
        max_topics: usize,
    ) -> Result<TopicRouter, Box<dyn std::error::Error>> {
        let model_id = registry
            .routing
            .as_ref()
            .and_then(|p| p.transformer_model.clone())
            .unwrap_or_else(|| "BAAI/bge-base-en-v1.5".into());

        // Resolve model directory: explicit env wins, else
        // `<EMEM_DATA>/models/<repo-tail>/` (e.g.
        // `/home/ubuntu/emem/var/emem/models/bge-base-en-v1.5/`).
        let model_dir: std::path::PathBuf = if let Ok(d) = std::env::var("EMEM_TOPIC_MODEL_DIR") {
            std::path::PathBuf::from(d)
        } else if let Ok(d) = std::env::var("EMEM_DATA") {
            let tail = model_id
                .rsplit('/')
                .next()
                .unwrap_or("bge-base-en-v1.5")
                .to_string();
            std::path::PathBuf::from(d).join("models").join(tail)
        } else {
            return Err(format!(
                "neither EMEM_TOPIC_MODEL_DIR nor EMEM_DATA is set — set one to point at a \
                 directory containing tokenizer.json and model.onnx (or onnx/model.onnx) for \
                 model {model_id}"
            )
            .into());
        };
        let onnx_candidates = [
            model_dir.join("model.onnx"),
            model_dir.join("onnx").join("model.onnx"),
        ];
        let onnx_path = onnx_candidates
            .iter()
            .find(|p| p.is_file())
            .ok_or_else(|| {
                format!(
                    "no ONNX model file in {model_dir:?}: expected one of {onnx_candidates:?}. \
                     Run scripts/install-topic-model.sh to populate it."
                )
            })?
            .clone();
        let tokenizer_json = model_dir.join("tokenizer.json");
        if !tokenizer_json.is_file() {
            return Err(format!(
                "no tokenizer.json at {tokenizer_json:?} — the topic router needs both \
                 model.onnx and tokenizer.json side by side under EMEM_TOPIC_MODEL_DIR"
            )
            .into());
        }

        let tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_json)
            .map_err(|e| format!("load tokenizer {tokenizer_json:?}: {e}"))?;

        let _ = ort::init().commit();

        let session = ort::session::Session::builder()?
            .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level3)?
            .with_intra_threads(2)?
            .commit_from_file(&onnx_path)?;

        let session = std::sync::Arc::new(std::sync::Mutex::new(session));
        let tokenizer = std::sync::Arc::new(tokenizer);
        let centroids = compute_centroids_ort(&session, &tokenizer, registry)?;

        tracing::info!(
            target: "emem::topic_router",
            model = %model_id,
            model_dir = %model_dir.display(),
            threshold = threshold,
            n_topics = registry.topics.len(),
            "topic router: ort loaded; topic centroids embedded"
        );

        Ok(TopicRouter {
            inner: std::sync::Arc::new(TopicRouterInner {
                backend: Backend::Ort {
                    session,
                    tokenizer,
                    centroids,
                    model_id,
                },
                registry: registry.clone(),
                threshold,
                max_topics,
            }),
        })
    }

    fn keyword_only(registry: TopicRegistry, threshold: f32, max_topics: usize) -> TopicRouter {
        let aliases_per_topic = registry
            .topics
            .iter()
            .map(|t| {
                let mut all: Vec<String> = Vec::with_capacity(t.aliases.len() + 1);
                all.push(t.key.replace('_', " ").to_lowercase());
                for a in &t.aliases {
                    all.push(a.to_lowercase());
                }
                all
            })
            .collect();
        TopicRouter {
            inner: std::sync::Arc::new(TopicRouterInner {
                backend: Backend::Keyword { aliases_per_topic },
                registry,
                threshold,
                max_topics,
            }),
        }
    }

    /// Route a free-text question to the matching topics. Returns
    /// matches sorted by descending score, capped at `max_topics`,
    /// each above the configured threshold.
    ///
    /// **Hybrid scoring** (since 2026-05-04). Even in transformer mode
    /// we run the keyword exact-match pass first as a high-precision
    /// pre-pass: if the question contains an exact alias substring
    /// (case-folded), tag those topics with `via: "keyword"` and
    /// score 1.0 so they always surface above the transformer
    /// threshold. The transformer pass then runs to add semantically
    /// related topics the keyword pass missed (paraphrases,
    /// synonyms). Final result is the keyword hits first (sorted
    /// by alias length / question length so the most specific match
    /// wins), then any transformer hits not already covered, all
    /// truncated to `max_topics`.
    ///
    /// Why this exists: `model2vec/potion-base-8M` is a ~32 MB
    /// static-lookup distillation of MiniLM. Cosine quality is good
    /// for paraphrase but degrades on questions where the topical
    /// noun is a small fraction of the embedding pool (e.g.
    /// "show me NDVI for Bengaluru" — "Bengaluru" dominates the
    /// embedding and the cosine to `vegetation_condition` falls
    /// below the 0.35 threshold). The keyword pre-pass gives us
    /// BM25-grade precision on known nouns at zero extra cost.
    ///
    /// **Out-of-scope short-circuit** (since 2026-05-08). Questions
    /// containing any of the deny-list patterns in
    /// [`OUT_OF_SCOPE_PATTERNS`] return an empty match set
    /// regardless of what the transformer scores. The 105-question
    /// consumer eval (tests/comprehensive/) caught the failure mode:
    /// the transformer happily returned 5 weak (0.35-0.45) topic
    /// hits for "who won the 2024 election" and "what is the meaning
    /// of life" because the BERT cosine geometry has *some* signal
    /// for almost any English noun phrase. The deny-list catches the
    /// obvious off-topic hits before the topic registry sees them.
    pub fn route(&self, question: &str) -> Vec<TopicMatch> {
        let q = question.trim();
        if q.is_empty() {
            return Vec::new();
        }
        // Out-of-scope short-circuit — see `OUT_OF_SCOPE_PATTERNS` doc.
        if is_out_of_scope(q) {
            return Vec::new();
        }
        // Always run the keyword exact-match pass — it's pure substring
        // search over <100 short aliases per topic and costs <1 µs.
        let keyword_hits = self.keyword_match(q);
        match &self.inner.backend {
            Backend::Ort {
                session,
                tokenizer,
                centroids,
                ..
            } => {
                let q_vec = embed_one_ort(session, tokenizer, q).unwrap_or_default();
                let mut scored: Vec<(usize, f32)> = centroids
                    .iter()
                    .enumerate()
                    .map(|(i, c)| (i, cosine(&q_vec, c)))
                    .filter(|(_, s)| *s >= self.inner.threshold)
                    .collect();
                scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                let mut transformer_hits: Vec<TopicMatch> = scored
                    .into_iter()
                    .map(|(i, score)| TopicMatch {
                        key: self.inner.registry.topics[i].key.clone(),
                        score,
                        via: "transformer",
                    })
                    .collect();
                let mut out: Vec<TopicMatch> = Vec::new();
                let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
                for h in keyword_hits.into_iter().chain(transformer_hits.drain(..)) {
                    if seen.insert(h.key.clone()) {
                        out.push(h);
                    }
                }
                out.truncate(self.inner.max_topics);
                out
            }
            Backend::Transformer { model, centroids } => {
                let q_vec = embed_one(model, q);
                let mut scored: Vec<(usize, f32)> = centroids
                    .iter()
                    .enumerate()
                    .map(|(i, c)| (i, cosine(&q_vec, c)))
                    .filter(|(_, s)| *s >= self.inner.threshold)
                    .collect();
                scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                let mut transformer_hits: Vec<TopicMatch> = scored
                    .into_iter()
                    .map(|(i, score)| TopicMatch {
                        key: self.inner.registry.topics[i].key.clone(),
                        score,
                        via: "transformer",
                    })
                    .collect();
                // Merge: keyword hits first (high precision), then
                // transformer hits not already in the keyword list.
                let mut out: Vec<TopicMatch> = Vec::new();
                let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
                for h in keyword_hits.into_iter().chain(transformer_hits.drain(..)) {
                    if seen.insert(h.key.clone()) {
                        out.push(h);
                    }
                }
                out.truncate(self.inner.max_topics);
                out
            }
            Backend::Keyword { .. } => {
                let mut hits = keyword_hits;
                hits.truncate(self.inner.max_topics);
                hits
            }
        }
    }

    /// Pure substring scoring against `aliases[]` + the topic key.
    /// Builds the alias table on the fly when the active backend is
    /// Transformer mode (where the table isn't pre-cached). Returns
    /// matches sorted by score (longest matched alias / question
    /// length) descending.
    fn keyword_match(&self, q: &str) -> Vec<TopicMatch> {
        let q_low = q.to_lowercase();
        let aliases_iter: Vec<(usize, Vec<String>)> = match &self.inner.backend {
            Backend::Keyword { aliases_per_topic } => aliases_per_topic
                .iter()
                .enumerate()
                .map(|(i, v)| (i, v.clone()))
                .collect(),
            // Ort and Transformer don't pre-cache the alias table;
            // build it on the fly from the registry. Cheap —
            // ~25 topics × ~20 aliases × `to_lowercase()`.
            Backend::Ort { .. } | Backend::Transformer { .. } => self
                .inner
                .registry
                .topics
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    let mut all: Vec<String> = Vec::with_capacity(t.aliases.len() + 1);
                    all.push(t.key.replace('_', " ").to_lowercase());
                    for a in &t.aliases {
                        all.push(a.to_lowercase());
                    }
                    (i, all)
                })
                .collect(),
        };
        let mut hits: Vec<TopicMatch> = Vec::new();
        for (i, aliases) in aliases_iter.iter() {
            let mut best: f32 = 0.0;
            for a in aliases {
                if a.is_empty() {
                    continue;
                }
                if q_low.contains(a) {
                    let s = (a.len() as f32) / (q_low.len() as f32).max(1.0);
                    if s > best {
                        best = s;
                    }
                }
            }
            if best > 0.0 {
                hits.push(TopicMatch {
                    key: self.inner.registry.topics[*i].key.clone(),
                    score: best,
                    via: "keyword",
                });
            }
        }
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits
    }

    /// Look up the canonical bands for a routed topic.
    pub fn bands_for_topic(&self, topic_key: &str) -> Vec<String> {
        self.inner
            .registry
            .lookup(topic_key)
            .map(|t| t.bands.clone())
            .unwrap_or_default()
    }

    /// Look up the algorithms for a routed topic.
    pub fn algorithms_for_topic(&self, topic_key: &str) -> Vec<String> {
        self.inner
            .registry
            .lookup(topic_key)
            .map(|t| t.algorithms.clone())
            .unwrap_or_default()
    }

    /// Snapshot of the underlying registry — useful for `/v1/topics`
    /// introspection and for debugging which topic corpus the router
    /// is using.
    pub fn registry(&self) -> &TopicRegistry {
        &self.inner.registry
    }

    /// Which backend is currently serving routes.
    pub fn backend_name(&self) -> &'static str {
        match &self.inner.backend {
            Backend::Ort { .. } => "ort",
            Backend::Transformer { .. } => "model2vec",
            Backend::Keyword { .. } => "keyword",
        }
    }

    /// The HF model id that's actually loaded. Returns `None` for the
    /// keyword-only backend (no model). Surfaced in `/v1/topics` so
    /// operators can verify which model the live router uses.
    #[allow(dead_code)]
    pub fn model_id(&self) -> Option<&str> {
        match &self.inner.backend {
            Backend::Ort { model_id, .. } => Some(model_id),
            Backend::Transformer { .. } => Some("minishlab/potion-base-8M"),
            Backend::Keyword { .. } => None,
        }
    }
}

fn embed_one(model: &model2vec_rs::model::StaticModel, text: &str) -> Vec<f32> {
    let v = model.encode(&[text.to_string()]);
    v.into_iter().next().unwrap_or_default()
}

/// Embed a single short string via ort + tokenizers. Acquires the
/// session mutex for the duration of the inference (ort sessions are
/// `Send` but their `run` takes `&mut`); a 25-topic registry serves
/// /v1/ask comfortably under this lock.
///
/// Pipeline: tokenize → run BERT-style ONNX session → CLS-pool
/// (token 0 of `last_hidden_state`) → L2 normalise.
fn embed_one_ort(
    session: &std::sync::Arc<std::sync::Mutex<ort::session::Session>>,
    tokenizer: &std::sync::Arc<tokenizers::Tokenizer>,
    text: &str,
) -> Result<Vec<f32>, String> {
    use ort::value::Tensor;

    let enc = tokenizer
        .encode(text, true)
        .map_err(|e| format!("tokenize: {e}"))?;
    let n = enc.get_ids().len();
    let ids: Vec<i64> = enc.get_ids().iter().map(|&x| x as i64).collect();
    let mask: Vec<i64> = enc.get_attention_mask().iter().map(|&x| x as i64).collect();
    let tt: Vec<i64> = enc.get_type_ids().iter().map(|&x| x as i64).collect();

    let ids_t = Tensor::from_array(([1, n], ids)).map_err(|e| format!("ids tensor: {e}"))?;
    let mask_t = Tensor::from_array(([1, n], mask)).map_err(|e| format!("mask tensor: {e}"))?;
    let tt_t = Tensor::from_array(([1, n], tt)).map_err(|e| format!("tt tensor: {e}"))?;

    let mut guard = session.lock().map_err(|e| format!("session mutex: {e}"))?;
    let outputs = guard
        .run(ort::inputs![
            "input_ids" => ids_t,
            "attention_mask" => mask_t,
            "token_type_ids" => tt_t,
        ])
        .map_err(|e| format!("ort run: {e}"))?;
    let (_name, last_hidden) = outputs.iter().next().ok_or("ort returned no outputs")?;
    let arr = last_hidden
        .try_extract_array::<f32>()
        .map_err(|e| format!("extract output: {e}"))?;
    // Shape is [batch=1, seq=n, hidden=dim]. CLS token = token 0.
    // Use index_axis (safe API) instead of the s![] macro because
    // emem-api-rest sets `#![forbid(unsafe_code)]` and s![] expands
    // to an unsafe block.
    let batch0 = arr.index_axis(ort_ndarray::Axis(0), 0);
    let cls = batch0.index_axis(ort_ndarray::Axis(0), 0);
    let mut v: Vec<f32> = cls.iter().copied().collect();
    l2_normalise(&mut v);
    Ok(v)
}

/// Per-topic centroid via ort: embed (description + aliases) for
/// every topic, average to a single vector, L2-normalise. Each
/// individual embedding is already CLS-pooled + L2-normalised, but
/// the per-topic average drifts off the unit sphere so we re-
/// normalise to keep cosine identity-comparable across topics.
fn compute_centroids_ort(
    session: &std::sync::Arc<std::sync::Mutex<ort::session::Session>>,
    tokenizer: &std::sync::Arc<tokenizers::Tokenizer>,
    registry: &TopicRegistry,
) -> Result<Vec<Vec<f32>>, String> {
    let mut centroids = Vec::with_capacity(registry.topics.len());
    for t in &registry.topics {
        let mut texts: Vec<String> = Vec::with_capacity(t.aliases.len() + 1);
        texts.push(t.description.clone());
        texts.extend(t.aliases.iter().cloned());
        let mut sum: Vec<f32> = Vec::new();
        let mut n = 0usize;
        for tx in &texts {
            let v = embed_one_ort(session, tokenizer, tx)
                .map_err(|e| format!("ort embed (topic {}, text {tx:?}): {e}", t.key))?;
            if sum.is_empty() {
                sum = v;
            } else {
                for (i, x) in v.iter().enumerate().take(sum.len()) {
                    sum[i] += x;
                }
            }
            n += 1;
        }
        if n == 0 {
            centroids.push(Vec::new());
            continue;
        }
        let nf = n as f32;
        for x in &mut sum {
            *x /= nf;
        }
        l2_normalise(&mut sum);
        centroids.push(sum);
    }
    Ok(centroids)
}

fn compute_centroids(
    model: &model2vec_rs::model::StaticModel,
    registry: &TopicRegistry,
) -> Vec<Vec<f32>> {
    // Embed each topic's full pool (description + aliases) and
    // average to a centroid. Averaging is simple and robust for
    // static embeddings — model2vec already L2-normalises per text,
    // so the averaged vector lives close to the unit sphere; we
    // re-normalise anyway to make cosine simpler downstream.
    let mut centroids = Vec::with_capacity(registry.topics.len());
    for t in &registry.topics {
        let mut texts: Vec<String> = Vec::with_capacity(t.aliases.len() + 1);
        texts.push(t.description.clone());
        texts.extend(t.aliases.iter().cloned());
        let vecs = model.encode(&texts);
        if vecs.is_empty() {
            centroids.push(Vec::new());
            continue;
        }
        let dim = vecs[0].len();
        let mut c = vec![0.0_f32; dim];
        for v in &vecs {
            for (i, x) in v.iter().enumerate().take(dim) {
                c[i] += x;
            }
        }
        let n = vecs.len() as f32;
        for x in &mut c {
            *x /= n;
        }
        l2_normalise(&mut c);
        centroids.push(c);
    }
    centroids
}

fn l2_normalise(v: &mut [f32]) {
    let mut s = 0.0_f32;
    for x in v.iter() {
        s += x * x;
    }
    let n = s.sqrt();
    if n > 0.0 {
        for x in v.iter_mut() {
            *x /= n;
        }
    }
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let dim = a.len().min(b.len());
    let mut dot = 0.0_f32;
    let mut na = 0.0_f32;
    let mut nb = 0.0_f32;
    for i in 0..dim {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom == 0.0 {
        0.0
    } else {
        dot / denom
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The keyword backend always routes "lake water level" to the
    /// flood/water event-window topic. Tests the substring-fallback
    /// path without needing the model file at hand.
    #[test]
    fn keyword_backend_routes_lake_question() {
        let registry = (*emem_core::topics::DEFAULT).clone();
        let router = TopicRouter::keyword_only(registry, 0.0, 5);
        let hits = router.route("lake water level after rain");
        assert!(
            hits.iter().any(|h| h.key == "flood_water_event_window"),
            "expected flood_water_event_window in hits, got {:?}",
            hits.iter().map(|h| &h.key).collect::<Vec<_>>()
        );
    }

    #[test]
    fn keyword_backend_routes_air_quality_question() {
        let registry = (*emem_core::topics::DEFAULT).clone();
        let router = TopicRouter::keyword_only(registry, 0.0, 5);
        let hits = router.route("is the air quality here safe");
        assert!(
            hits.iter().any(|h| h.key == "public_health"),
            "expected public_health in hits, got {:?}",
            hits.iter().map(|h| &h.key).collect::<Vec<_>>()
        );
    }

    #[test]
    fn keyword_backend_returns_empty_for_unrelated_question() {
        let registry = (*emem_core::topics::DEFAULT).clone();
        let router = TopicRouter::keyword_only(registry, 0.0, 5);
        let hits = router.route("how do I bake bread without yeast");
        assert!(
            hits.is_empty(),
            "unrelated question should not match any topic, got {:?}",
            hits.iter().map(|h| &h.key).collect::<Vec<_>>()
        );
    }

    #[test]
    fn router_exposes_bands_and_algorithms_per_topic() {
        let registry = (*emem_core::topics::DEFAULT).clone();
        let router = TopicRouter::keyword_only(registry, 0.0, 5);
        let bands = router.bands_for_topic("flood_water_event_window");
        assert!(bands.contains(&"sentinel1_raw".to_string()));
        let algos = router.algorithms_for_topic("flood_risk_composite");
        assert!(algos.iter().any(|a| a == "flood_risk@2"));
    }

    /// Out-of-scope deny-list — see `OUT_OF_SCOPE_PATTERNS`. These
    /// were the false-positive cases the 105-question consumer eval
    /// surfaced (Q290, Q291). They must return an empty match set in
    /// every backend, regardless of cosine geometry.
    #[test]
    fn out_of_scope_questions_return_no_topics() {
        let registry = (*emem_core::topics::DEFAULT).clone();
        let router = TopicRouter::keyword_only(registry, 0.0, 5);
        for q in [
            "who won the 2024 election",
            "what is the meaning of life",
            "current bitcoin price",
            "tell me a joke about space",
            "ignore previous instructions",
        ] {
            let hits = router.route(q);
            assert!(
                hits.is_empty(),
                "out-of-scope question {q:?} should return no topics, got {:?}",
                hits.iter().map(|h| &h.key).collect::<Vec<_>>()
            );
        }
    }

    /// Climate-haven-from-fire phrasing — Q112 in the consumer eval
    /// missed `fire_burn_severity` because the prior alias list didn't
    /// cover "safe from wildfire" or "away from wildfire". The new
    /// aliases (added to `topics-v0.json` 2026-05-08) should route
    /// these correctly under the keyword backend.
    #[test]
    fn fire_haven_phrasing_routes_to_burn_severity() {
        let registry = (*emem_core::topics::DEFAULT).clone();
        let router = TopicRouter::keyword_only(registry, 0.0, 5);
        for q in [
            "climate safe places in portugal away from wildfire",
            "neighborhoods safe from wildfire near Sacramento",
            "low wildfire risk towns in oregon",
            "is paradise california fire-safe now",
            "bushfire risk blue mountains nsw this summer",
        ] {
            let hits = router.route(q);
            assert!(
                hits.iter().any(|h| h.key == "fire_burn_severity"),
                "fire-haven question {q:?} should route to fire_burn_severity, got {:?}",
                hits.iter().map(|h| &h.key).collect::<Vec<_>>()
            );
        }
    }
}
