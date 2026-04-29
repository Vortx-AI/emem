//! Algorithm registry — content-addressed recipes that compose attested
//! band facts (and embeddings) into derived scores or classifications.
//!
//! Distinct from [`crate::functions`] (which derives a single band value
//! from raw upstream sources) and from [`crate::bands`] (the data slot
//! itself). An algorithm names a deterministic recipe — formula in plain
//! math, input bands, output kind — so receipts can cite an
//! `algorithm_cid` alongside `fact_cids` and a downstream verifier can
//! replay the same composition exactly.
//!
//! Three kinds of entry:
//!   - **solo**     — single input band → derived value (e.g. NDVI →
//!     vegetation class). Most useful for hiding hand-tuned thresholds
//!     behind a stable name.
//!   - **combined** — multiple band facts → composite score
//!     (e.g. flood-risk = recurrence + elevation + radar).
//!   - **embedding** — operations on the geotessera embedding vector
//!     (cosine, novelty, change, neighborhood consistency).
//!
//! The registry is loaded from `data/algorithms-v0.json` and exposed via
//! a process-wide `LazyLock` (see [`DEFAULT`]). Operators may publish a
//! distinct algorithms manifest CID; entry keys are stable across
//! manifests, weights/thresholds are not.

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use crate::manifest::{Manifest, ManifestError, MANIFEST_ALGORITHM_REG};

const ALGORITHMS_V0_JSON: &str = include_str!("../data/algorithms-v0.json");

/// Discriminator for the structural family of an algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlgorithmKind {
    /// Single-band → derived classification or scalar.
    Solo,
    /// Multi-band composite score / classification.
    Combined,
    /// Operates on the geotessera embedding (or its multi-year fusion).
    Embedding,
}

/// One declared input to an algorithm. The `band` field references a
/// key from the band registry; the `transform` and `weight` fields are
/// editorial and let the formula string round-trip into a structured
/// representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlgorithmInput {
    /// Band key the input draws from. Optional only for the special
    /// `_corpus` marker on embedding-novelty-style entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub band: Option<String>,
    /// Free-form role label inside the formula (e.g. `"history"`,
    /// `"vec_a"`). Helps an agent map the formula's variables to the
    /// fact_cids it just fetched.
    pub role: String,
    /// Optional weight in a weighted-sum composition. Decorative — the
    /// `formula` field is the source of truth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
    /// Optional pre-multiplication transform expressed in plain math.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<String>,
    /// Editorial unit (matches the band's declared unit — repeated here
    /// so an agent reading just one entry sees enough context).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// Editorial note explaining the variable's role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explanation: Option<String>,
    /// Set to `true` on entries that pull from the responder's k-NN
    /// corpus instead of a single band fact (used by `embedding_novelty@1`).
    #[serde(default, rename = "_corpus", skip_serializing_if = "Option::is_none")]
    pub corpus: Option<bool>,
}

/// Description of what the algorithm produces.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlgorithmOutput {
    /// `"scalar"`, `"classification"`, `"vector"`.
    pub kind: String,
    /// Free-form unit (e.g. `"probability"`, `"risk_index"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// Closed-form admissible range (numeric scalars only). May contain
    /// the string `"+inf"` for half-open ranges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<serde_json::Value>,
    /// Closed value set for `kind == "classification"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<String>>,
    /// Editorial explanation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explanation: Option<String>,
}

/// One registry entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Algorithm {
    /// Algorithm key including version, e.g. `"flood_risk@1"`.
    pub key: String,
    /// Solo / combined / embedding.
    pub kind: AlgorithmKind,
    /// Editorial domain tag for routing & discovery (e.g. `"water"`,
    /// `"climate"`, `"human"`, `"topography"`, `"embedding"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// Declared inputs.
    pub inputs: Vec<AlgorithmInput>,
    /// Plain-math formula. Source of truth for verifiers; the structured
    /// `inputs` are decorative.
    pub formula: String,
    /// What the algorithm outputs.
    pub output: AlgorithmOutput,
    /// Editorial guidance for agent routing.
    pub when_to_use: String,
    /// Concrete primitive call (REST or local) that gathers the inputs.
    pub primitive: String,
    /// Whether the formula is deterministic given the input fact_cids.
    /// `false` for entries that depend on corpus state at request time.
    #[serde(default = "default_true")]
    pub deterministic: bool,
    /// Optional editorial note explaining a `deterministic = false` entry.
    #[serde(
        default,
        rename = "_deterministic_note",
        skip_serializing_if = "Option::is_none"
    )]
    pub deterministic_note: Option<String>,
    /// Citation — preferred peer-reviewed source for the underlying math.
    pub citation: String,
}

fn default_true() -> bool {
    true
}

/// Algorithms manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlgorithmRegistry {
    /// MUST equal `"emem-algorithms"`.
    pub manifest: String,
    /// Version, e.g. `"v0"`.
    pub version: String,
    /// Algorithm entries.
    pub algorithms: Vec<Algorithm>,
    /// Editorial note.
    #[serde(default, rename = "_note", skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Manifest for AlgorithmRegistry {
    const KIND: &'static str = MANIFEST_ALGORITHM_REG;

    fn validate(&self) -> Result<(), ManifestError> {
        if self.manifest != Self::KIND {
            return Err(ManifestError::WrongKind {
                expected: Self::KIND,
                actual: self.manifest.clone(),
            });
        }
        let mut seen: std::collections::HashSet<&str> = Default::default();
        for a in &self.algorithms {
            if !seen.insert(&a.key) {
                return Err(ManifestError::Invalid(format!(
                    "duplicate algorithm key: {}",
                    a.key
                )));
            }
            if a.inputs.is_empty() {
                return Err(ManifestError::Invalid(format!(
                    "algorithm {} has no inputs",
                    a.key
                )));
            }
            if !["scalar", "classification", "vector"].contains(&a.output.kind.as_str()) {
                return Err(ManifestError::Invalid(format!(
                    "algorithm {}: output.kind must be scalar|classification|vector, got {}",
                    a.key, a.output.kind
                )));
            }
        }
        Ok(())
    }
}

impl AlgorithmRegistry {
    /// Embedded v0 default.
    pub fn parse_default() -> Result<Self, ManifestError> {
        Self::parse_json(ALGORITHMS_V0_JSON.as_bytes())
    }

    /// Look up an algorithm by key.
    pub fn lookup(&self, key: &str) -> Option<&Algorithm> {
        self.algorithms.iter().find(|a| a.key == key)
    }

    /// All algorithms of a given kind.
    pub fn by_kind(&self, kind: AlgorithmKind) -> impl Iterator<Item = &Algorithm> {
        self.algorithms.iter().filter(move |a| a.kind == kind)
    }

    /// Every key that this algorithm reads from. Useful for an agent
    /// that wants to assemble the right `/v1/recall` body in one shot.
    pub fn input_bands(&self, key: &str) -> Vec<&str> {
        self.lookup(key)
            .map(|a| {
                a.inputs
                    .iter()
                    .filter_map(|i| i.band.as_deref())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }
}

/// Process-wide cached default registry.
pub static DEFAULT: LazyLock<AlgorithmRegistry> = LazyLock::new(|| {
    AlgorithmRegistry::parse_default().expect("embedded algorithms-v0.json is malformed")
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_loads_and_validates() {
        let r = &*DEFAULT;
        assert_eq!(r.manifest, MANIFEST_ALGORITHM_REG);
        assert!(r.lookup("flood_risk@1").is_some());
        assert!(r.lookup("water_consensus@1").is_some());
        assert!(r.lookup("embedding_cosine@1").is_some());
    }

    #[test]
    fn flood_risk_inputs_match_homepage_example() {
        let r = &*DEFAULT;
        let inputs = r.input_bands("flood_risk@1");
        assert!(inputs.contains(&"surface_water.recurrence"));
        assert!(inputs.contains(&"copdem30m.elevation_mean"));
        assert!(inputs.contains(&"sentinel1_raw"));
    }

    #[test]
    fn three_kinds_present() {
        let r = &*DEFAULT;
        assert!(r.by_kind(AlgorithmKind::Solo).count() >= 1);
        assert!(r.by_kind(AlgorithmKind::Combined).count() >= 1);
        assert!(r.by_kind(AlgorithmKind::Embedding).count() >= 1);
    }
}
