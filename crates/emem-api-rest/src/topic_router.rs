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
//! Why **model2vec** specifically:
//!
//!   - Pure Rust inference (no ONNX/torch C++ runtime). The base
//!     model is distilled into a token-level lookup table, so a
//!     "forward pass" is sum-and-normalise over token vectors —
//!     ~50 µs per short question on CPU. No GPU. No ort. No onnxruntime
//!     binary.
//!   - Tiny model (`potion-base-8M` is ~32 MB on disk, 256-D
//!     embeddings) — comparable to a single S2 tile, much smaller
//!     than the responder binary itself.
//!   - Same embedding contract as a sentence-transformer: cosine on
//!     normalised vectors. Drop-in upgrade path to a heavier model
//!     later via `EMEM_TOPIC_MODEL`.
//!
//! Configuration (env vars):
//!
//!   - `EMEM_TOPIC_MODEL` — Hugging Face repo or local path. Default
//!     `minishlab/potion-base-8M`.
//!   - `EMEM_TOPIC_THRESHOLD` — cosine threshold below which a topic
//!     is discarded. Default `0.35` (tuned against the topic corpus).
//!   - `EMEM_TOPIC_BACKEND` — `transformer` (default) or `keyword`.
//!     Set to `keyword` to skip the model load entirely (useful for
//!     unit tests, embedded responders, or air-gapped builds). The
//!     keyword backend does substring search over `aliases[]` + the
//!     topic key itself.
//!   - `EMEM_TOPIC_DATA_DIR` — where to cache the model files.
//!     Defaults to `<EMEM_DATA>/models/` if `EMEM_DATA` is set, else
//!     a system-default cache dir picked by `hf-hub`.
//!
//! The transformer load is **lazy** — the first `/v1/ask` call
//! triggers the model download (~30 s on a fresh deploy, ~50 ms
//! after that for warm cache). Subsequent calls are sub-millisecond.

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
    /// Full ONNX-runtime sentence transformer (default backend since
    /// 2026-05-04). Loads `BAAI/bge-base-en-v1.5` (110 M params,
    /// 768-D, MTEB ~63) via fastembed-rs. Tries the CUDA execution
    /// provider first when an NVIDIA GPU is reachable through
    /// libonnxruntime at `ORT_DYLIB_PATH`; falls back to CPU EP
    /// otherwise. Centroid + per-query inference take ~1-2 ms on the
    /// A100, ~12-18 ms on CPU.
    Fastembed {
        model: std::sync::Arc<std::sync::Mutex<fastembed::TextEmbedding>>,
        centroids: Vec<Vec<f32>>,
        model_id: String,
    },
    /// Static-distillation token-lookup fallback: `model2vec` /
    /// `potion-base-8M`, 256-D, ~32 MB, sub-µs per question, pure
    /// Rust. Used when fastembed cannot load (no GPU + no CPU ORT
    /// binary, or when `EMEM_TOPIC_BACKEND=model2vec`).
    Transformer {
        model: std::sync::Arc<model2vec_rs::model::StaticModel>,
        centroids: Vec<Vec<f32>>,
    },
    /// Substring-search fallback: pre-lowercased aliases + key per
    /// topic. Used when both fastembed and model2vec load fail, or
    /// when `EMEM_TOPIC_BACKEND=keyword`.
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
        let threshold: f32 = std::env::var("EMEM_TOPIC_THRESHOLD")
            .ok()
            .and_then(|s| s.parse().ok())
            .or_else(|| policy.and_then(|p| p.threshold))
            .unwrap_or(0.35);
        let max_topics: usize = policy.and_then(|p| p.max_topics_per_question).unwrap_or(5);

        // Default = fastembed (ONNX runtime sentence transformer);
        // `transformer` kept as an alias for the legacy model2vec
        // path; `keyword` skips both for unit tests / air-gapped
        // builds.
        let want_backend =
            std::env::var("EMEM_TOPIC_BACKEND").unwrap_or_else(|_| "fastembed".into());

        if want_backend == "keyword" {
            tracing::info!(
                target: "emem::topic_router",
                "EMEM_TOPIC_BACKEND=keyword — skipping transformer load"
            );
            return TopicRouter::keyword_only(registry, threshold, max_topics);
        }

        if want_backend == "fastembed" {
            match TopicRouter::try_fastembed(&registry, threshold, max_topics) {
                Ok(r) => return r,
                Err(e) => {
                    tracing::warn!(
                        target: "emem::topic_router",
                        error = %e,
                        "topic router: fastembed load failed; falling back to model2vec"
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

    /// Build a fastembed-backed router. Tries CUDA EP first when
    /// `ORT_DYLIB_PATH` resolves to a CUDA-enabled libonnxruntime,
    /// falls back to CPU EP. Caches the ONNX model under
    /// `<EMEM_DATA>/models/fastembed/` (or HF default cache when
    /// `EMEM_DATA` is unset). Errors propagate so the caller can
    /// drop to the model2vec backend.
    fn try_fastembed(
        registry: &TopicRegistry,
        threshold: f32,
        max_topics: usize,
    ) -> Result<TopicRouter, Box<dyn std::error::Error>> {
        use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

        // Pick the model: env override wins, else registry policy,
        // else BGE-base-en-v1.5 (the 0.0.5 default).
        let model_id = std::env::var("EMEM_TOPIC_MODEL").unwrap_or_else(|_| {
            registry
                .routing
                .as_ref()
                .and_then(|p| p.transformer_model.clone())
                .unwrap_or_else(|| "BAAI/bge-base-en-v1.5".into())
        });
        let em = match model_id.as_str() {
            "BAAI/bge-base-en-v1.5" => EmbeddingModel::BGEBaseENV15,
            "BAAI/bge-small-en-v1.5" => EmbeddingModel::BGESmallENV15,
            "BAAI/bge-large-en-v1.5" => EmbeddingModel::BGELargeENV15,
            "nomic-ai/nomic-embed-text-v1.5" => EmbeddingModel::NomicEmbedTextV15,
            "intfloat/multilingual-e5-small" => EmbeddingModel::MultilingualE5Small,
            "sentence-transformers/all-MiniLM-L6-v2" => EmbeddingModel::AllMiniLML6V2,
            other => {
                return Err(format!(
                    "EMEM_TOPIC_MODEL={other:?} is not a fastembed-supported model id; \
                     pick one of BAAI/bge-{{small,base,large}}-en-v1.5, \
                     nomic-ai/nomic-embed-text-v1.5, intfloat/multilingual-e5-small, \
                     sentence-transformers/all-MiniLM-L6-v2 — or set \
                     EMEM_TOPIC_BACKEND=model2vec to use the legacy backend"
                )
                .into());
            }
        };
        // Offline path: when `EMEM_FASTEMBED_OFFLINE_DIR` points at a
        // directory that already holds the four tokenizer files +
        // model.onnx, build via `try_new_from_user_defined` which
        // bypasses hf-hub-rs entirely. This is the production-stable
        // path because hf-hub-rs 0.5 has no `HF_HUB_OFFLINE` flag and
        // its sync `Api::get` performs a network probe to
        // huggingface.co even when the file is already cached — that
        // probe hung indefinitely on emem.dev's host on 2026-05-04
        // and held the OnceLock initializer for >90s, wedging the
        // whole topic router. Pre-populate the dir manually (or run
        // `try_new` once on a host with reliable network) and then
        // pin the env var.
        if let Ok(dir) = std::env::var("EMEM_FASTEMBED_OFFLINE_DIR") {
            let mut model = build_fastembed_offline(&dir, em)?;
            let centroids = compute_centroids_fastembed(&mut model, registry)?;
            tracing::info!(
                target: "emem::topic_router",
                model = %model_id,
                offline_dir = %dir,
                threshold = threshold,
                n_topics = registry.topics.len(),
                "topic router: fastembed (offline) loaded; topic centroids embedded"
            );
            return Ok(TopicRouter {
                inner: std::sync::Arc::new(TopicRouterInner {
                    backend: Backend::Fastembed {
                        model: std::sync::Arc::new(std::sync::Mutex::new(model)),
                        centroids,
                        model_id,
                    },
                    registry: registry.clone(),
                    threshold,
                    max_topics,
                }),
            });
        }

        let mut init = InitOptions::new(em).with_show_download_progress(false);
        if let Ok(emem_data) = std::env::var("EMEM_DATA") {
            let cache = std::path::PathBuf::from(emem_data)
                .join("models")
                .join("fastembed");
            std::fs::create_dir_all(&cache).ok();
            init = init.with_cache_dir(cache);
        }
        let mut model = TextEmbedding::try_new(init)?;
        let centroids = compute_centroids_fastembed(&mut model, registry)?;
        tracing::info!(
            target: "emem::topic_router",
            model = %model_id,
            threshold = threshold,
            n_topics = registry.topics.len(),
            "topic router: fastembed loaded; topic centroids embedded"
        );
        Ok(TopicRouter {
            inner: std::sync::Arc::new(TopicRouterInner {
                backend: Backend::Fastembed {
                    model: std::sync::Arc::new(std::sync::Mutex::new(model)),
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
    pub fn route(&self, question: &str) -> Vec<TopicMatch> {
        let q = question.trim();
        if q.is_empty() {
            return Vec::new();
        }
        // Always run the keyword exact-match pass — it's pure substring
        // search over <100 short aliases per topic and costs <1 µs.
        let keyword_hits = self.keyword_match(q);
        match &self.inner.backend {
            Backend::Fastembed {
                model, centroids, ..
            } => {
                let q_vec = embed_one_fastembed(model, q).unwrap_or_default();
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
            // Fastembed and Transformer don't pre-cache the alias
            // table; build it on the fly from the registry. Cheap —
            // ~25 topics × ~20 aliases × `to_lowercase()`.
            Backend::Fastembed { .. } | Backend::Transformer { .. } => self
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
            Backend::Fastembed { .. } => "fastembed",
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
            Backend::Fastembed { model_id, .. } => Some(model_id),
            Backend::Transformer { .. } => Some("minishlab/potion-base-8M"),
            Backend::Keyword { .. } => None,
        }
    }
}

fn embed_one(model: &model2vec_rs::model::StaticModel, text: &str) -> Vec<f32> {
    let v = model.encode(&[text.to_string()]);
    v.into_iter().next().unwrap_or_default()
}

/// Build a fastembed `TextEmbedding` directly from on-disk files,
/// skipping hf-hub-rs entirely. Reads `model.onnx` (or
/// `onnx/model.onnx`) plus `tokenizer.json`, `tokenizer_config.json`,
/// `config.json`, `special_tokens_map.json` from `dir`. Picks pooling
/// from the model's hardcoded default in fastembed (Cls for BGE,
/// Mean for E5/MiniLM/Nomic — see fastembed::TextEmbedding::
/// `get_default_pooling_method`). Used by the offline path triggered
/// by `EMEM_FASTEMBED_OFFLINE_DIR`.
fn build_fastembed_offline(
    dir: &str,
    em: fastembed::EmbeddingModel,
) -> Result<fastembed::TextEmbedding, String> {
    use fastembed::{
        InitOptionsUserDefined, TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel,
    };
    let dp = std::path::Path::new(dir);
    let onnx_candidates = [dp.join("model.onnx"), dp.join("onnx").join("model.onnx")];
    let onnx_path = onnx_candidates
        .iter()
        .find(|p| p.is_file())
        .ok_or_else(|| {
            format!(
                "EMEM_FASTEMBED_OFFLINE_DIR={dir} has no model.onnx or onnx/model.onnx — \
                 expected one of {:?}",
                onnx_candidates
            )
        })?
        .clone();
    let read = |name: &str| -> Result<Vec<u8>, String> {
        std::fs::read(dp.join(name)).map_err(|e| format!("read {dir}/{name}: {e}"))
    };
    let tokenizer_files = TokenizerFiles {
        tokenizer_file: read("tokenizer.json")?,
        config_file: read("config.json")?,
        special_tokens_map_file: read("special_tokens_map.json")?,
        tokenizer_config_file: read("tokenizer_config.json")?,
    };
    let onnx_bytes = std::fs::read(&onnx_path)
        .map_err(|e| format!("read {}: {e}", onnx_path.display()))?;
    let pooling = TextEmbedding::get_default_pooling_method(&em);
    let model_def = {
        let m = UserDefinedEmbeddingModel::new(onnx_bytes, tokenizer_files);
        match pooling {
            Some(p) => m.with_pooling(p),
            None => m,
        }
    };
    let opts = InitOptionsUserDefined::new();
    TextEmbedding::try_new_from_user_defined(model_def, opts)
        .map_err(|e| format!("fastembed try_new_from_user_defined: {e}"))
}

/// Embed a single short string with fastembed. Acquires the
/// `Mutex<TextEmbedding>` for the duration of the inference (the
/// underlying ORT session is `!Sync`). On a 25-topic registry this
/// is fine — `/v1/ask` is not a high-fan-out endpoint.
fn embed_one_fastembed(
    model: &std::sync::Arc<std::sync::Mutex<fastembed::TextEmbedding>>,
    text: &str,
) -> Result<Vec<f32>, String> {
    let mut guard = model.lock().map_err(|e| format!("model mutex: {e}"))?;
    let mut v = guard
        .embed(vec![text.to_string()], None)
        .map_err(|e| format!("fastembed embed: {e}"))?;
    Ok(v.pop().unwrap_or_default())
}

/// Per-topic centroid via fastembed: embed (description + aliases)
/// for every topic, average to a single vector, L2-normalise. fastembed
/// already L2-normalises per-text outputs but the average drifts off
/// the unit sphere, so re-normalise to make cosine identity-comparable.
fn compute_centroids_fastembed(
    model: &mut fastembed::TextEmbedding,
    registry: &TopicRegistry,
) -> Result<Vec<Vec<f32>>, String> {
    let mut centroids = Vec::with_capacity(registry.topics.len());
    for t in &registry.topics {
        let mut texts: Vec<String> = Vec::with_capacity(t.aliases.len() + 1);
        texts.push(t.description.clone());
        texts.extend(t.aliases.iter().cloned());
        let vecs = model
            .embed(texts, None)
            .map_err(|e| format!("fastembed embed (topic {}): {e}", t.key))?;
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
}
