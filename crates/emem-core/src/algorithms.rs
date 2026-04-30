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

/// Sensor-tier ranking used by the multimodal-fusion policy.
///
/// Earth observation is a game of precision, resolution, and repetition,
/// and the protocol's commitment is: **deliver at 10 m where physics
/// allows**. To ensure that, every algorithm that claims a delivery
/// resolution ≤10 m MUST have at least one S1/S2/Landsat input on its
/// variance side. Coarse-physics algorithms (SPI on POWER precip, GDD
/// on POWER temperature) declare their honest native resolution
/// instead.
///
/// Order is significant — the registry validator enforces priority
/// `S1 > S2 > Landsat > IoT > OtherSat > Static` and refuses entries
/// where the declared anchor's tier is below a higher-tier input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceTier {
    /// Sentinel-1 SAR (10 m, 6–12 d revisit, all-weather).
    S1,
    /// Sentinel-2 MSL + every derived spectral index + Tessera embedding
    /// (10/20 m, 5-day revisit; Tessera anchors at 10 m grid).
    S2,
    /// Landsat 8/9 OLI (30 m / 15 m pan, 16 d revisit). Reserved —
    /// not yet wired in this responder; entries CAN reference it once
    /// a materializer lands.
    Landsat,
    /// In-situ / IoT sensor stream (per-station; resolution is the
    /// telemetered grid). Reserved.
    Iot,
    /// Coarse satellites — MODIS, CAMS, Marine, etc. Useful as
    /// baseline / context, NOT as variance source for ≤10 m claims.
    OtherSat,
    /// Reanalyses, climatologies, soil-property maps, regulatory
    /// rasters: POWER, ERA5, SoilGrids, Hansen, ESA WorldCover, JRC
    /// GSW, Cop-DEM, GMRT. ALWAYS treated as "any tier" for baseline
    /// purposes, NEVER acceptable as the sole variance source for a
    /// ≤10 m delivery claim.
    Static,
}

impl SourceTier {
    /// Map a band key to its sensor tier. The string-prefix matching
    /// here is the single source of truth used by both the algorithms
    /// registry validator and any downstream agent that wants to order
    /// inputs by tier.
    pub fn for_band(band: &str) -> Self {
        if band == "sentinel1_raw" || band.starts_with("s1.") {
            SourceTier::S1
        } else if band.starts_with("s2.")
            || band.starts_with("indices.")
            || band.starts_with("geotessera")
        {
            // Tessera is a learned representation of S2/S1 fused at the
            // S2 grid — anchors at 10 m and inherits the S2 tier.
            SourceTier::S2
        } else if band.starts_with("landsat.") {
            SourceTier::Landsat
        } else if band.starts_with("iot.") {
            SourceTier::Iot
        } else if band.starts_with("modis.")
            || band.starts_with("cams.")
            || band.starts_with("marine.")
            || band.starts_with("viirs.")
        {
            SourceTier::OtherSat
        } else {
            // power.*, era5.*, weather.*, soilgrids.*, hansen.*,
            // esa_worldcover.*, surface_water.*, copdem30m.*, gmrt.*,
            // chirps.*, openet.*, dynamic_world.*, tropomi.*, ovetrue.*
            SourceTier::Static
        }
    }
}

/// How an algorithm combines its multimodal inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FusionMethod {
    /// Simple weighted-mean over the input transforms (the default
    /// pattern for `combined` algorithms — flood_risk, water_consensus).
    WeightedMean,
    /// Multi-sensor agreement count → confidence ladder (the
    /// `residue_burn_multisensor@1` pattern).
    ConsensusVote,
    /// First-available wins, in tier order. Used for fallback chains.
    SequentialPriority,
    /// Bayesian / Kalman blend of a coarse prior with a fine residual
    /// (SOC = SoilGrids prior + S2 SWIR residual; SAR-SM = climatology
    /// envelope + current backscatter).
    BayesianBlend,
    /// No fusion — single-source algorithm (NDVI class, water from
    /// γ⁰ alone). The `multimodal.variance_sources` field still names
    /// the single anchor so the registry can verify the resolution claim.
    None,
}

/// What to do when the variance-tier source is unavailable on a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackStrategy {
    /// Drop one tier (e.g. S2 unavailable → use S1; if both gone →
    /// SequentialPriority continues into Other/Static at REDUCED
    /// `delivery_resolution_m`, surfaced in the receipt).
    TierDemote,
    /// Sign Absence rather than degrade — used when a coarse fallback
    /// would change the answer's meaning, not just its resolution
    /// (PMFBY claim assessment, EUDR compliance).
    Absence,
    /// Use the baseline / prior alone with an explicit confidence
    /// drop (SOC tier-A: SoilGrids prior with no S2 residual when
    /// the cell is canopy-covered or cloud-blocked).
    PriorOnlyWithConfidenceDrop,
}

/// Multimodal-fusion declaration for an algorithm. Populated entries
/// let the registry validator mechanically prove the algorithm earns
/// its claimed delivery resolution, and let agents pick algorithms
/// whose anchor matches the question's required precision.
///
/// **Optional** at v0 — older entries that pre-date the field stay
/// valid but cannot claim a 10 m delivery resolution (the validator
/// silently accepts them as Static-tier).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Multimodal {
    /// Claimed final per-cell pixel pitch in metres. The registry
    /// validator enforces: `delivery_resolution_m <= 10` requires at
    /// least one S1/S2/Landsat input in `variance_sources`.
    pub delivery_resolution_m: u32,
    /// Band keys that supply the slow / climatological / prior term.
    /// Any tier permitted.
    pub baseline_sources: Vec<String>,
    /// Band keys that supply the fast / event / current term. MUST
    /// include at least one S1, S2, or Landsat band when
    /// `delivery_resolution_m <= 10`.
    pub variance_sources: Vec<String>,
    /// Composition method used to fuse the inputs.
    pub fusion_method: FusionMethod,
    /// Ordered tier list — declares the algorithm's primary observation
    /// path, with `priority_chain[0]` matching the anchor's tier. The
    /// validator enforces this ordering so an algorithm can't silently
    /// promote a Static input over an S2 input in its receipt narrative.
    pub priority_chain: Vec<SourceTier>,
    /// The single band that defines `delivery_resolution_m`. MUST be
    /// present in the algorithm's declared `inputs[]`. The validator
    /// also checks that `tier_of(anchor_band) == priority_chain[0]`.
    pub anchor_band: String,
    /// Behaviour when the variance-tier source isn't materializable
    /// on a given cell.
    pub fallback_strategy: FallbackStrategy,
    /// Set to `true` on entries whose variance flows through other
    /// algorithm composites (`<composite>` pseudo-bands). The
    /// validator walks composites to resolve the effective
    /// `priority_chain` and `delivery_resolution_m` instead of failing
    /// on the `<composite>` placeholder.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub composite_inherit: bool,
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
    /// Recommended re-computation cadence for this algorithm. Typed as a
    /// free-form string so editorial entries can be specific (e.g.
    /// `"5-day cloud-permitting (Sentinel-2 revisit)"`,
    /// `"annual; carbon-credit baselines require ≥1 yr separation"`).
    /// Optional — older entries that pre-date the field stay valid.
    /// Surfaced via `/v1/algorithms` so an agent can pick a sensible
    /// re-query interval rather than hammering the cache or, worse,
    /// reporting yesterday's stale answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency_of_calculation: Option<String>,
    /// Editorial accuracy band — `"R²~0.4-0.7 (S2 alone)"` etc. Lets a
    /// downstream UI declare confidence honestly rather than presenting
    /// every algorithm at the same fidelity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accuracy_band: Option<String>,
    /// Multimodal-fusion declaration — see [`Multimodal`]. When present,
    /// the registry validator enforces the 10 m delivery rule: any
    /// algorithm claiming `delivery_resolution_m <= 10` MUST list at
    /// least one S1, S2, or Landsat band in `variance_sources`. Coarse
    /// physics products (SPI on POWER, GDD on weather) declare honest
    /// large resolutions and stay valid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multimodal: Option<Multimodal>,
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
            // Multimodal-fusion validation. Skipped silently when an
            // entry has no `multimodal` block (older entries pre-date
            // the field and stay valid).
            if let Some(mm) = &a.multimodal {
                let input_keys: std::collections::HashSet<&str> =
                    a.inputs.iter().filter_map(|i| i.band.as_deref()).collect();

                // R1 — anchor must be a declared input. The
                // composite_inherit escape lets parametric_trigger@1
                // and other composite-of-composites use a `<composite>`
                // anchor by reference; the validator stops at that
                // boundary because it can't resolve `<composite>`
                // structurally.
                let anchor_is_composite = mm.anchor_band.starts_with('<');
                if !mm.composite_inherit
                    && !anchor_is_composite
                    && !input_keys.contains(mm.anchor_band.as_str())
                {
                    return Err(ManifestError::Invalid(format!(
                        "algorithm {}: multimodal.anchor_band '{}' is not declared in inputs[]",
                        a.key, mm.anchor_band
                    )));
                }

                // R2 — ≤10 m delivery requires S1/S2/Landsat variance.
                if mm.delivery_resolution_m <= 10 && !mm.composite_inherit {
                    let has_high_res = mm.variance_sources.iter().any(|b| {
                        matches!(
                            SourceTier::for_band(b),
                            SourceTier::S1 | SourceTier::S2 | SourceTier::Landsat
                        )
                    });
                    if !has_high_res {
                        return Err(ManifestError::Invalid(format!(
                            "algorithm {}: claims delivery_resolution_m={} but variance_sources has no S1/S2/Landsat band ({:?})",
                            a.key, mm.delivery_resolution_m, mm.variance_sources
                        )));
                    }
                }

                // R3 — variance_sources must NOT be Static-only.
                // Variance is, by definition, an *observation* of the
                // current state; a climatology can't carry that signal.
                // Empty list is allowed only on `none` fusion (single
                // baseline-only algorithms), checked in R3a.
                let has_observational_variance = mm
                    .variance_sources
                    .iter()
                    .any(|b| !matches!(SourceTier::for_band(b), SourceTier::Static));
                let allow_no_variance =
                    matches!(mm.fusion_method, FusionMethod::None) || mm.composite_inherit;
                if !has_observational_variance
                    && !allow_no_variance
                    && !mm.variance_sources.is_empty()
                {
                    return Err(ManifestError::Invalid(format!(
                        "algorithm {}: variance_sources has only Static-tier inputs ({:?}); variance must be observational",
                        a.key, mm.variance_sources
                    )));
                }

                // R4 — priority_chain must lead with the anchor's tier.
                // This blocks an algorithm from claiming an S2 anchor
                // narrative while actually leaning on a SoilGrids prior.
                if !mm.priority_chain.is_empty() && !anchor_is_composite {
                    let anchor_tier = SourceTier::for_band(&mm.anchor_band);
                    if mm.priority_chain.first() != Some(&anchor_tier) {
                        return Err(ManifestError::Invalid(format!(
                            "algorithm {}: priority_chain[0] = {:?}, but anchor_band '{}' resolves to tier {:?}",
                            a.key,
                            mm.priority_chain.first(),
                            mm.anchor_band,
                            anchor_tier
                        )));
                    }
                }
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
    /// Filters out the `<composite>` placeholder used in `algorithms-v0.json`
    /// to mark inputs that come from other algorithms (not raw bands) —
    /// passing that string to a materializer caused a silent
    /// `no_auto_materializer_registered` skip and an empty answer for
    /// composite-of-composites questions like "air quality" / "livability".
    pub fn input_bands(&self, key: &str) -> Vec<&str> {
        self.lookup(key)
            .map(|a| {
                a.inputs
                    .iter()
                    .filter_map(|i| i.band.as_deref())
                    .filter(|b| !b.starts_with('<'))
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

    #[test]
    fn source_tier_for_band_classifies_correctly() {
        assert_eq!(SourceTier::for_band("sentinel1_raw"), SourceTier::S1);
        assert_eq!(SourceTier::for_band("s2.B11"), SourceTier::S2);
        assert_eq!(SourceTier::for_band("indices.ndvi"), SourceTier::S2);
        assert_eq!(SourceTier::for_band("geotessera"), SourceTier::S2);
        assert_eq!(SourceTier::for_band("modis.lai_8day"), SourceTier::OtherSat);
        assert_eq!(SourceTier::for_band("cams.pm25"), SourceTier::OtherSat);
        assert_eq!(SourceTier::for_band("power.t2m"), SourceTier::Static);
        assert_eq!(SourceTier::for_band("era5.precip"), SourceTier::Static);
        assert_eq!(
            SourceTier::for_band("soilgrids.soc_0_30cm"),
            SourceTier::Static
        );
        assert_eq!(SourceTier::for_band("hansen.loss_year"), SourceTier::Static);
    }

    #[test]
    fn multimodal_validator_rejects_overclaim() {
        // An algorithm claiming 10 m delivery but with only POWER + SoilGrids
        // variance must be rejected — that's the whole point of R2.
        let raw = serde_json::json!({
          "manifest": "emem-algorithms",
          "version": "v0",
          "algorithms": [{
            "key": "bogus_overclaim@1",
            "kind": "solo",
            "domain": "soil",
            "inputs": [
              { "band": "soilgrids.soc_0_30cm", "role": "scalar_in" }
            ],
            "formula": "soc",
            "output": { "kind": "scalar", "unit": "g_kg" },
            "when_to_use": "test",
            "primitive": "test",
            "deterministic": true,
            "citation": "test",
            "multimodal": {
              "delivery_resolution_m": 10,
              "baseline_sources": ["soilgrids.soc_0_30cm"],
              "variance_sources":  ["soilgrids.soc_0_30cm"],
              "fusion_method": "none",
              "priority_chain": ["static"],
              "anchor_band": "soilgrids.soc_0_30cm",
              "fallback_strategy": "absence"
            }
          }]
        });
        let r: AlgorithmRegistry = serde_json::from_value(raw).unwrap();
        let err = r
            .validate()
            .expect_err("R2 must reject 10 m claim with no S1/S2");
        assert!(format!("{err:?}").contains("delivery_resolution_m"));
    }

    #[test]
    fn multimodal_validator_accepts_honest_coarse() {
        // SPI on POWER precip is legitimately ~5500 m — accept it.
        let raw = serde_json::json!({
          "manifest": "emem-algorithms",
          "version": "v0",
          "algorithms": [{
            "key": "honest_coarse@1",
            "kind": "solo",
            "domain": "climate",
            "inputs": [
              { "band": "power.precip", "role": "scalar_in" }
            ],
            "formula": "spi",
            "output": { "kind": "scalar", "unit": "z_score" },
            "when_to_use": "test",
            "primitive": "test",
            "deterministic": true,
            "citation": "test",
            "multimodal": {
              "delivery_resolution_m": 55000,
              "baseline_sources": ["power.precip"],
              "variance_sources":  ["power.precip"],
              "fusion_method": "none",
              "priority_chain": ["static"],
              "anchor_band": "power.precip",
              "fallback_strategy": "absence"
            }
          }]
        });
        let r: AlgorithmRegistry = serde_json::from_value(raw).unwrap();
        r.validate().expect("honest coarse declaration is valid");
    }
}
