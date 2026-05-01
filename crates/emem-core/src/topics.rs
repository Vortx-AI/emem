//! Topic registry — the routing brain of `/v1/ask` and the MCP
//! `emem_ask` tool, lifted out of code into a content-addressed manifest.
//!
//! Each topic carries:
//!
//!   - `description` — a short paragraph used to compute a sentence-
//!     transformer embedding for *semantic* routing of a free-text
//!     question (cosine similarity, threshold-driven, multi-topic OK).
//!   - `aliases[]` — short example phrases. They serve two purposes:
//!     (a) more text in the embedding pool for the topic, (b) a
//!     substring fallback when the transformer cannot be loaded
//!     (offline build, embedded responder, deliberate disable via
//!     `EMEM_TOPIC_BACKEND=keyword`).
//!   - `bands[]` — canonical band keys this topic concerns;
//!     `/v1/ask` recalls these for the matched cell.
//!   - `algorithms[]` — composition recipe keys from
//!     `algorithms-v0.json` that should be applied on top of those
//!     bands; the agent gets each algorithm's formula + temporal
//!     recipe inline in the response.
//!
//! Loaded from `data/topics-v0.json` and exposed via a process-wide
//! `LazyLock` (see [`DEFAULT`]). Operators may publish a different
//! topic registry CID; topic keys are stable across registries.
//!
//! Why a separate manifest (and not a `topic` field on each algorithm):
//!
//!   - Some topics have no algorithm at all — `optical_raw_reflectance`,
//!     `scene_classification`, `radar_all_weather_sar` are just band
//!     groups for users that want to do their own band math. Putting
//!     `topic` on `Algorithm` would orphan them.
//!   - Routing semantics (description, aliases, bands grouping) are
//!     editorial; algorithm semantics (formula, inputs, citation) are
//!     scientific. Keeping them in separate manifests lets each evolve
//!     under its own CID without dragging the other.

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use crate::manifest::{Manifest, ManifestError, MANIFEST_TOPIC_REG};

const TOPICS_V0_JSON: &str = include_str!("../data/topics-v0.json");

/// One topic — the unit of routing in `/v1/ask`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Topic {
    /// Stable, kebab-or-snake-case identifier. Examples:
    /// `flood_water_event_window`, `urban_livability`,
    /// `parametric_insurance`.
    pub key: String,
    /// One-paragraph description used to compute the topic embedding
    /// for semantic routing. Should describe what kind of question
    /// the topic answers, not how the topic is implemented.
    pub description: String,
    /// Short example phrases that frequently appear in real
    /// questions for this topic. Used both as additional embedding-
    /// side text and as the substring search basis when the
    /// transformer is offline.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Canonical band keys this topic concerns. `/v1/ask` recalls
    /// these for the matched cell.
    #[serde(default)]
    pub bands: Vec<String>,
    /// Algorithm keys (from `algorithms-v0.json`) that should be
    /// applied on top of the bands. Order is preserved (deterministic
    /// suggestion order in the response).
    #[serde(default)]
    pub algorithms: Vec<String>,
}

/// Routing-policy hint block. Editorial — not enforced by the
/// validator. Reads as a single source of truth for which model and
/// threshold the responder will pick when no env override is set.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TopicRoutingPolicy {
    /// Hugging-Face model id of the sentence transformer used to
    /// embed questions and topic descriptions.
    #[serde(default)]
    pub transformer_model: Option<String>,
    /// Output dimensionality of the embedding model.
    #[serde(default)]
    pub embedding_dims: Option<usize>,
    /// Similarity metric (currently only `"cosine"`).
    #[serde(default)]
    pub similarity: Option<String>,
    /// Cosine threshold below which a candidate topic is discarded.
    #[serde(default)]
    pub threshold: Option<f32>,
    /// Hard cap on how many topics one question can route to.
    #[serde(default)]
    pub max_topics_per_question: Option<usize>,
    /// Free-form description of what happens when the transformer
    /// cannot be loaded.
    #[serde(default)]
    pub fallback_when_offline: Option<String>,
    /// Documentation of the env vars the operator can override.
    #[serde(default)]
    pub env_overrides: serde_json::Map<String, serde_json::Value>,
}

/// The full topic manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicRegistry {
    /// MUST equal `"emem.topics.v0"` (matches `manifest::MANIFEST_TOPIC_REG`
    /// once we drop the schema-name prefix; for now both are accepted).
    pub schema: String,
    /// Editorial doc-string.
    #[serde(default, rename = "_doc", skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Routing-policy hints (editorial, not load-bearing).
    #[serde(default, rename = "_routing", skip_serializing_if = "Option::is_none")]
    pub routing: Option<TopicRoutingPolicy>,
    /// Topic entries.
    pub topics: Vec<Topic>,
}

impl Manifest for TopicRegistry {
    const KIND: &'static str = MANIFEST_TOPIC_REG;

    fn validate(&self) -> Result<(), ManifestError> {
        // Accept either `emem.topics.v0` (schema-style) or
        // `emem-topics` (manifest-kind-style). Editorial — both
        // resolve to the same registry.
        if self.schema != "emem.topics.v0" && self.schema != Self::KIND {
            return Err(ManifestError::WrongKind {
                expected: Self::KIND,
                actual: self.schema.clone(),
            });
        }
        let mut seen: std::collections::HashSet<&str> = Default::default();
        for t in &self.topics {
            if !seen.insert(&t.key) {
                return Err(ManifestError::Invalid(format!(
                    "duplicate topic key: {}",
                    t.key
                )));
            }
            if t.description.trim().is_empty() {
                return Err(ManifestError::Invalid(format!(
                    "topic {} has empty description (needed for transformer routing)",
                    t.key
                )));
            }
            if t.bands.is_empty() && t.algorithms.is_empty() {
                return Err(ManifestError::Invalid(format!(
                    "topic {} has no bands and no algorithms — what would /v1/ask serve for it?",
                    t.key
                )));
            }
        }
        Ok(())
    }
}

impl TopicRegistry {
    /// Embedded v0 default.
    pub fn parse_default() -> Result<Self, ManifestError> {
        Self::parse_json(TOPICS_V0_JSON.as_bytes())
    }

    /// Look up a topic by key.
    pub fn lookup(&self, key: &str) -> Option<&Topic> {
        self.topics.iter().find(|t| t.key == key)
    }

    /// Return every topic that names `band` in its `bands[]`.
    /// Useful for the inverse query "which topics rely on this
    /// band?" without forcing the responder to keep its own index.
    pub fn topics_for_band(&self, band: &str) -> Vec<&Topic> {
        self.topics
            .iter()
            .filter(|t| t.bands.iter().any(|b| b == band))
            .collect()
    }

    /// Return every topic that names `algorithm_key` in its
    /// `algorithms[]`.
    pub fn topics_for_algorithm(&self, algorithm_key: &str) -> Vec<&Topic> {
        self.topics
            .iter()
            .filter(|t| t.algorithms.iter().any(|a| a == algorithm_key))
            .collect()
    }

    /// Concatenate `description` + each entry of `aliases[]` into one
    /// embedding pool per topic. The transformer router embeds each
    /// pool string and averages — gives a more robust topic centroid
    /// than embedding only the description.
    pub fn embedding_corpus(&self) -> Vec<(String, Vec<String>)> {
        self.topics
            .iter()
            .map(|t| {
                let mut texts = Vec::with_capacity(t.aliases.len() + 1);
                texts.push(t.description.clone());
                texts.extend(t.aliases.iter().cloned());
                (t.key.clone(), texts)
            })
            .collect()
    }
}

/// Process-wide cached default registry.
pub static DEFAULT: LazyLock<TopicRegistry> =
    LazyLock::new(|| TopicRegistry::parse_default().expect("embedded topics-v0.json is malformed"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_loads_and_validates() {
        let r = &*DEFAULT;
        assert!(!r.topics.is_empty(), "topics-v0.json should not be empty");
        // Spot-check the topics that the Katihar test report exposed.
        assert!(r.lookup("flood_water_event_window").is_some());
        assert!(r.lookup("flood_risk_composite").is_some());
        assert!(r.lookup("public_health").is_some());
    }

    #[test]
    fn every_topic_has_either_bands_or_algorithms() {
        for t in &DEFAULT.topics {
            assert!(
                !t.bands.is_empty() || !t.algorithms.is_empty(),
                "topic {} has neither bands nor algorithms",
                t.key
            );
        }
    }

    #[test]
    fn embedding_corpus_includes_description_and_aliases() {
        let corpus = DEFAULT.embedding_corpus();
        let (_, texts) = corpus
            .iter()
            .find(|(k, _)| k == "flood_water_event_window")
            .expect("flood_water_event_window present");
        // Description + at least one alias.
        assert!(texts.len() >= 2);
        assert!(
            texts[0].to_lowercase().contains("wet")
                || texts[0].to_lowercase().contains("flood")
                || texts[0].to_lowercase().contains("water"),
            "first text should be the description"
        );
    }
}
