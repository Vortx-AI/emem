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

use std::collections::BTreeMap;
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
            || band == "prithvi_eo2"
            || band.starts_with("prithvi_eo2.")
            || band == "galileo_base"
            || band.starts_with("galileo_base.")
            || band == "clay_v1"
            || band.starts_with("clay_v1.")
        {
            // Tessera, Prithvi-EO-2.0, and Galileo are learned
            // representations of S2 (and for Tessera also S1) fused at
            // the S2 grid. They anchor on the Sentinel-2 chip and
            // inherit the S2 tier so algorithms grounded on them can
            // claim delivery_resolution_m=10 (Prithvi/Galileo carry a
            // 30 m chip receptive field — agents reading the cell-level
            // fact get the chip-aware embedding, not a 30 m sample).
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
    /// Optional temporal recipe — see [`TemporalRecipe`]. Per-algorithm
    /// declaration of which lookback windows to materialize alongside the
    /// snapshot recall. The intent dispatcher reads this and emits a
    /// `temporal_composition` block in `/v1/ask` and `/v1/intent`
    /// responses. Algorithms without a recipe behave as before
    /// (snapshot-only). Added 2026-05 in response to the Katihar test:
    /// flood-risk needs antecedent rainfall and current radar, not just
    /// the latest static recurrence value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temporal_recipe: Option<TemporalRecipe>,
    /// Optional evaluable formula — see [`Expr`]. When present, the
    /// responder can evaluate the algorithm in-process: walk the AST,
    /// recall each referenced band, plug the values in, and return the
    /// scalar result alongside the input fact CIDs. The composite is
    /// then verifiable end-to-end (a third party with the same inputs
    /// + algorithm CID re-executes and gets the same number).
    ///
    /// Algorithms without an `evaluation` continue to be advertised
    /// only — the agent reads the human-readable `formula` string and
    /// composes itself. Added in 0.0.3 alongside `temporal_recipe`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluation: Option<Expr>,
    /// Optional inference-tier declaration — see [`InferenceTier`]. When
    /// present, the dispatcher consults the sidecar's `/health.extensions`
    /// at planning time and filters algorithms whose required hardware is
    /// not currently advertised. The `tier_chain` lists the responder's
    /// preference order (e.g. `["gpu","cpu","absence"]`); the receipt
    /// names which tier actually served (`served_via.tier`). Added 2026-05
    /// alongside the Clay Foundation Model band — Clay-anchored algorithms
    /// require a working GPU, so the tier metadata is what lets us route
    /// to them when GPU is up and skip them honestly when it isn't,
    /// without silent CPU fallback that would ship a different embedding
    /// distribution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inference: Option<InferenceTier>,
    /// Optional tunable parameters. Lets an algorithm declare its
    /// thresholds, gates, and learned constants as data rather than
    /// hardcoding them in the formula string. The dispatcher exposes
    /// `param(key)` for lookups and `learned_from` provenance is
    /// preserved alongside the value.
    ///
    /// Values are typed as `serde_json::Value` so a parameter can be a
    /// number, a string, or a sub-object carrying `{value, learned_from,
    /// rationale}`. The accessor [`Algorithm::param_f64`] / [`param_str`]
    /// unwraps the common cases; clients that want the full provenance
    /// read the raw `Value` via [`Algorithm::param`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<BTreeMap<String, serde_json::Value>>,
    /// Optional learned-from provenance (citation for any parameter
    /// values that were tuned rather than derived from first principles).
    /// Free-form object — `gate_threshold`, `rationale`, dataset
    /// references. Surfaced verbatim in `/v1/algorithms` so an auditor
    /// can trace any number that isn't physically obvious.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub learned_from: Option<serde_json::Value>,
    /// Optional prerequisites — registries / centroid tables / seed
    /// datasets the algorithm depends on. When listed, the dispatcher
    /// can pre-check availability and emit a structured
    /// `archetype_seed_unavailable` Absence rather than a runtime crash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prerequisites: Option<serde_json::Value>,
}

impl Algorithm {
    /// Read a parameter as a raw [`serde_json::Value`]. Returns `None` if
    /// either the algorithm carries no `parameters` block or the key is
    /// absent. Values may be scalars or `{value, learned_from, ...}`
    /// objects — callers that just want the number should use
    /// [`Self::param_f64`].
    pub fn param(&self, key: &str) -> Option<&serde_json::Value> {
        self.parameters.as_ref()?.get(key)
    }

    /// Read a parameter as `f64`. Accepts either a bare number
    /// (`"k": 12`) or a `{value: 12, ...}` object (the provenance-rich
    /// form). Returns `None` if missing or non-numeric.
    pub fn param_f64(&self, key: &str) -> Option<f64> {
        let v = self.param(key)?;
        if let Some(n) = v.as_f64() {
            return Some(n);
        }
        v.get("value").and_then(|x| x.as_f64())
    }

    /// Read a parameter as `&str`.
    pub fn param_str(&self, key: &str) -> Option<&str> {
        let v = self.param(key)?;
        if let Some(s) = v.as_str() {
            return Some(s);
        }
        v.get("value").and_then(|x| x.as_str())
    }
}

/// Where an algorithm wants to run. Mirrors Triton's `instance_group.kind`
/// (`KIND_GPU` / `KIND_CPU`) and the device-kind attribute in BentoML
/// `resources={"gpu":1}` / Ray Serve `accelerator_type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InferenceTierKind {
    /// Requires a CUDA / ROCm device. Refuses to run when the sidecar's
    /// `/health.cuda.available` is false (or the sidecar is unreachable).
    Gpu,
    /// Runs on CPU. May still call the sidecar (for the model registry's
    /// Galileo-Tiny CPU path), but does not require a GPU.
    Cpu,
    /// Pure scalar formula — no model, no sidecar, no GPU. Most existing
    /// `solo` and `combined` entries fit here once their tier is set
    /// explicitly. Default behaviour for entries without an `inference`
    /// block is to be treated as `Scalar` so legacy registries stay valid.
    Scalar,
    /// Reads only from the cache / persisted facts — does not invoke any
    /// upstream connector or model inference. Used by `find_similar`
    /// over a band the responder already materialised.
    Cached,
    /// Sentinel: serve a signed `Absence` instead of attempting the
    /// algorithm. Only valid as a `tier_chain` terminal — never a
    /// primary `tier`.
    Absence,
}

/// What to do when the primary tier is unavailable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TierFallbackStrategy {
    /// Sign an `Absence` fact with `unavailable_capability` reason. Use
    /// when the CPU path would change the embedding's distribution
    /// (e.g. swapping a 1024-D ViT for a 192-D distillation produces a
    /// different cosine surface — silently shipping that breaks the
    /// `bands_cid` contract).
    AbsenceWithReason,
    /// Walk the `tier_chain` to the next entry, attach `served_via.tier`
    /// to the receipt, and continue. Only safe when every tier produces
    /// a byte-compatible output schema.
    DegradeWithTierLabel,
    /// Retry the primary tier (typically `gpu`) `max_retries` times with
    /// jitter, then sign `Absence`. Used when the sidecar is briefly
    /// busy but expected to recover.
    RetryThenAbsence,
}

/// Inference-tier declaration. Lifted from Triton's `instance_group`,
/// Modal's `gpu=["h100","a100","any"]` fallback list, KServe v2's
/// `extensions[]` capability set, and the MCP capability-negotiation
/// pattern (the dispatcher filters at planning time so an agent never
/// sees a tool whose hardware is currently absent).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceTier {
    /// Primary tier — the responder's first-choice runtime for this
    /// algorithm.
    pub tier: InferenceTierKind,
    /// Ordered preference. The dispatcher walks this list and picks the
    /// first tier whose capability is currently advertised by the
    /// sidecar's `/health.extensions`. Example: `["gpu","absence"]` for
    /// a Clay encoder call (refuse rather than CPU-fallback).
    /// `["gpu","cpu","cached"]` for a degradable embedding lookup.
    pub tier_chain: Vec<InferenceTierKind>,
    /// Editorial label for the device kind (`"cuda"`, `"cpu"`). Surfaced
    /// to clients via `served_via.device_kind`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_kind: Option<String>,
    /// Estimated VRAM in bytes for batch=1 fp16 / fp32 inference. Used
    /// by the admission queue to decide whether the in-flight set fits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vram_estimate_bytes: Option<u64>,
    /// P95 latency budget the responder treats as the SLA. Above this
    /// the dispatcher prefers the next tier in `tier_chain`. Optional;
    /// when unset, no budget gating is applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p95_budget_ms: Option<u32>,
    /// Behaviour when the primary tier can't serve. See
    /// [`TierFallbackStrategy`].
    pub fallback_strategy: TierFallbackStrategy,
    /// Capability tag the algorithm needs in `/health.extensions`. Set
    /// to `Some("clay-v1.5")` for a Clay-anchored algorithm so the
    /// dispatcher only routes to it when the responder advertises
    /// Clay. Optional — pure GPU primitives that don't depend on a
    /// specific model leave it `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_extension: Option<String>,
}

/// One temporal lookback window an algorithm wants alongside the snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalWindow {
    /// Band to materialize over the window (must be a real band key).
    pub band: String,
    /// How many days back from `now` to look. `0` means "static — fetch
    /// once at tslot=0" (e.g. JRC GSW recurrence; the lookback is
    /// historically aggregated by the source itself).
    pub lookback_days: u32,
    /// Optional aggregator the agent should apply to the per-tslot facts:
    /// `"sum"`, `"mean"`, `"median"`, `"max"`, `"min"`, `"latest"`, or
    /// `"first"`. Empty / unrecognised values mean "return the raw
    /// per-tslot facts and let the caller aggregate". The dispatcher
    /// always returns the raw facts; the aggregator is a hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggregator: Option<String>,
    /// Editorial purpose tag, e.g. `"baseline_water"`,
    /// `"antecedent_rain"`, `"ndvi_baseline"`. Surfaced verbatim in the
    /// response so an agent can map facts to roles without inferring.
    pub purpose: String,
    /// Optional trigger threshold — only materialize this window if the
    /// snapshot recall's value for `band` is >= this number. Lets a
    /// flood algorithm skip the antecedent-rain backfill on a dry day.
    /// Encoded as f64; the field name in formulas is the literal string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_threshold: Option<f64>,
}

/// Evaluable formula for a composite algorithm.
///
/// Stored alongside the human-readable `formula: String` so a downstream
/// verifier can re-execute the composition exactly. Each variant maps to
/// one well-tested arithmetic primitive — the union covers every
/// composition pattern present in `algorithms-v0.json` at 0.0.3:
///
///   - `Band` / `Const`           — leaves
///   - `Add` / `Sub` / `Mul` / `Div` — pointwise arithmetic
///   - `Linear { weights, bias }` — Σ wᵢ·xᵢ + b (the workhorse for
///     weighted-mean composites: flood_risk, water_consensus,
///     parametric_trigger, walkability)
///   - `Clamp { lo, hi }`         — saturate output range
///   - `Where { cond, gt, lo, then, else_ }` — threshold-gated branch
///     (used by `flood_risk@2`'s DEM-agreement weighting)
///   - `WeightedBlend { primary, alt, alt_weight_when }` — primary +
///     alt-weighted residual (Bayesian-blend pattern)
///
/// Encoded in JSON via serde's tag-internal-with-content shape so an
/// algorithm's `evaluation: Expr` field reads as readable JSON:
///
/// ```json
/// {"op":"linear","weights":{"surface_water.recurrence":0.5,
///                            "copdem30m.elevation_mean":-0.0001,
///                            "indices.ndwi":0.3},"bias":0.0}
/// ```
///
/// Evaluation is pure: given a `samples: HashMap<band_key, f64>`, the
/// formula reduces to a single `Option<f64>` deterministically. Missing
/// bands cause `None` (the responder logs and returns
/// `algorithm_outcomes[].skip_reason: "missing_input:<band>"`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Expr {
    /// Look up a band's scalar value from the sample map.
    Band {
        /// The band key — must match an entry in
        /// `algorithm.inputs[].band` for the dispatcher to fetch it.
        band: String,
    },
    /// A literal numeric constant.
    Const {
        /// The constant value.
        value: f64,
    },
    /// Pointwise sum of every operand.
    Add {
        /// Operands to sum.
        terms: Vec<Expr>,
    },
    /// Pointwise difference: `a - b`.
    Sub {
        /// Left operand (minuend).
        a: Box<Expr>,
        /// Right operand (subtrahend).
        b: Box<Expr>,
    },
    /// Pointwise product of every operand.
    Mul {
        /// Operands to multiply.
        terms: Vec<Expr>,
    },
    /// Pointwise quotient: `a / b`. Returns `None` when `b == 0.0`.
    Div {
        /// Numerator.
        a: Box<Expr>,
        /// Denominator.
        b: Box<Expr>,
    },
    /// Linear combination: `Σ weights[band_i] · samples[band_i] + bias`.
    /// The workhorse for every weighted-mean composite. Bands not
    /// present in `weights` contribute `0`. Bands listed but missing
    /// from `samples` collapse the whole expression to `None` (so a
    /// missing required input is never silently treated as zero).
    Linear {
        /// Per-band weights.
        weights: std::collections::BTreeMap<String, f64>,
        /// Optional bias / intercept.
        #[serde(default)]
        bias: f64,
    },
    /// Clamp the inner expression to `[lo, hi]`.
    Clamp {
        /// Inner expression to clamp.
        inner: Box<Expr>,
        /// Lower bound.
        lo: f64,
        /// Upper bound.
        hi: f64,
    },
    /// Threshold branch: `if cond > gt then then_ else else_`. `cond`
    /// is evaluated; if it exceeds `gt`, return the `then_` branch's
    /// value, otherwise the `else_` branch. The 0.0.3 use case is
    /// `flood_risk@2`'s DEM-agreement gate (factor 0.5 when
    /// `|cop-dem - gmrt| > 5m`).
    Where {
        /// Condition expression.
        cond: Box<Expr>,
        /// Threshold the condition must exceed.
        gt: f64,
        /// Branch taken when `cond > gt`.
        then_: Box<Expr>,
        /// Branch taken otherwise.
        else_: Box<Expr>,
    },
    /// Weighted blend of a primary + alternative term, with the
    /// alternative's weight controlled by a third expression. Useful
    /// for "primary + αᵢ·residual" patterns where αᵢ depends on data
    /// quality (Bayesian / Kalman blends).
    WeightedBlend {
        /// Primary term.
        primary: Box<Expr>,
        /// Alternative / residual term.
        alt: Box<Expr>,
        /// Weight applied to `alt` (in `[0, 1]`; values outside that
        /// range are clamped). Often a `Where` branch on a quality
        /// indicator.
        alt_weight: Box<Expr>,
    },
    /// Take the absolute value of the inner expression.
    Abs {
        /// Inner expression.
        inner: Box<Expr>,
    },
    /// Logistic sigmoid: `1 / (1 + exp(-inner))`. Maps R → (0, 1).
    /// Used pervasively for "soft threshold" terms in the algorithm
    /// registry (S1 dB → P(water), temperature → heat-stress, etc.).
    Sigmoid {
        /// Inner expression.
        inner: Box<Expr>,
    },
    /// Rectified linear unit: `max(0, inner)`. Used for "asymmetric
    /// penalty" terms (low elevation → flood penalty, missing canopy
    /// → urban-heat penalty).
    Relu {
        /// Inner expression.
        inner: Box<Expr>,
    },
    /// Pointwise maximum across operands.
    Max {
        /// Operands; result is `f64::NEG_INFINITY` over an empty
        /// vector and that propagates through downstream arithmetic
        /// the same as `None` would.
        terms: Vec<Expr>,
    },
    /// Pointwise minimum across operands.
    Min {
        /// Operands; result is `f64::INFINITY` over an empty vector.
        terms: Vec<Expr>,
    },
    /// Hyperbolic tangent: `tanh(inner)`. Maps R → (-1, 1). The
    /// algorithm registry uses `tanh(x/scale)` as a saturating
    /// "density / count" transform (walkability, urban density, etc.).
    Tanh {
        /// Inner expression.
        inner: Box<Expr>,
    },
    /// Natural exponential: `exp(inner)`. Used by Tetens / Magnus
    /// saturation-vapour-pressure (VPD) and any Gaussian-shaped
    /// comfort kernel.
    Exp {
        /// Inner expression.
        inner: Box<Expr>,
    },
    /// Square root: `sqrt(inner)`. Returns `None` if `inner < 0`.
    /// Used by Fosberg FFWI's wind-amplification term and by RMS
    /// composites (e.g. Riley ruggedness index).
    Sqrt {
        /// Inner expression.
        inner: Box<Expr>,
    },
    /// Power: `pow(base, exp)`. Returns `None` if the result is not
    /// finite (e.g. `pow(-1, 0.5)`). Used by Osczevski-Bluestein
    /// wind-chill (`V^0.16`) and any other non-integer power term.
    Pow {
        /// Base expression.
        base: Box<Expr>,
        /// Exponent expression.
        exp: Box<Expr>,
    },
}

impl Expr {
    /// Evaluate the expression against a sample map.
    /// Returns `None` if any required band is absent or any
    /// arithmetic step is undefined (division by zero).
    pub fn evaluate(&self, samples: &std::collections::HashMap<String, f64>) -> Option<f64> {
        match self {
            Expr::Band { band } => samples.get(band).copied(),
            Expr::Const { value } => Some(*value),
            Expr::Add { terms } => {
                let mut s = 0.0_f64;
                for t in terms {
                    s += t.evaluate(samples)?;
                }
                Some(s)
            }
            Expr::Sub { a, b } => Some(a.evaluate(samples)? - b.evaluate(samples)?),
            Expr::Mul { terms } => {
                let mut p = 1.0_f64;
                for t in terms {
                    p *= t.evaluate(samples)?;
                }
                Some(p)
            }
            Expr::Div { a, b } => {
                let av = a.evaluate(samples)?;
                let bv = b.evaluate(samples)?;
                if bv == 0.0 {
                    None
                } else {
                    Some(av / bv)
                }
            }
            Expr::Linear { weights, bias } => {
                let mut acc = *bias;
                for (band, w) in weights {
                    let v = samples.get(band)?;
                    acc += w * v;
                }
                Some(acc)
            }
            Expr::Clamp { inner, lo, hi } => {
                let v = inner.evaluate(samples)?;
                Some(v.max(*lo).min(*hi))
            }
            Expr::Where {
                cond,
                gt,
                then_,
                else_,
            } => {
                let c = cond.evaluate(samples)?;
                if c > *gt {
                    then_.evaluate(samples)
                } else {
                    else_.evaluate(samples)
                }
            }
            Expr::WeightedBlend {
                primary,
                alt,
                alt_weight,
            } => {
                let p = primary.evaluate(samples)?;
                let a = alt.evaluate(samples)?;
                let mut w = alt_weight.evaluate(samples)?;
                if !w.is_finite() {
                    return None;
                }
                w = w.clamp(0.0, 1.0);
                Some((1.0 - w) * p + w * a)
            }
            Expr::Abs { inner } => Some(inner.evaluate(samples)?.abs()),
            Expr::Sigmoid { inner } => {
                let x = inner.evaluate(samples)?;
                Some(1.0 / (1.0 + (-x).exp()))
            }
            Expr::Relu { inner } => Some(inner.evaluate(samples)?.max(0.0)),
            Expr::Max { terms } => {
                if terms.is_empty() {
                    return Some(f64::NEG_INFINITY);
                }
                let mut best = f64::NEG_INFINITY;
                for t in terms {
                    let v = t.evaluate(samples)?;
                    if v > best {
                        best = v;
                    }
                }
                Some(best)
            }
            Expr::Min { terms } => {
                if terms.is_empty() {
                    return Some(f64::INFINITY);
                }
                let mut best = f64::INFINITY;
                for t in terms {
                    let v = t.evaluate(samples)?;
                    if v < best {
                        best = v;
                    }
                }
                Some(best)
            }
            Expr::Tanh { inner } => {
                let x = inner.evaluate(samples)?;
                Some(x.tanh())
            }
            Expr::Exp { inner } => {
                let x = inner.evaluate(samples)?;
                let v = x.exp();
                if v.is_finite() {
                    Some(v)
                } else {
                    None
                }
            }
            Expr::Sqrt { inner } => {
                let x = inner.evaluate(samples)?;
                if x < 0.0 {
                    None
                } else {
                    Some(x.sqrt())
                }
            }
            Expr::Pow { base, exp } => {
                let b = base.evaluate(samples)?;
                let e = exp.evaluate(samples)?;
                let v = b.powf(e);
                if v.is_finite() {
                    Some(v)
                } else {
                    None
                }
            }
        }
    }

    /// Walk the expression and collect every `Band` leaf's key.
    /// Used by the dispatcher to know which bands to recall before
    /// evaluating.
    pub fn referenced_bands(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.walk_bands(&mut out);
        out.sort();
        out.dedup();
        out
    }

    fn walk_bands(&self, out: &mut Vec<String>) {
        match self {
            Expr::Band { band } => out.push(band.clone()),
            Expr::Const { .. } => {}
            Expr::Add { terms }
            | Expr::Mul { terms }
            | Expr::Max { terms }
            | Expr::Min { terms } => {
                for t in terms {
                    t.walk_bands(out);
                }
            }
            Expr::Sub { a, b } | Expr::Div { a, b } => {
                a.walk_bands(out);
                b.walk_bands(out);
            }
            Expr::Linear { weights, .. } => {
                for k in weights.keys() {
                    out.push(k.clone());
                }
            }
            Expr::Clamp { inner, .. }
            | Expr::Abs { inner }
            | Expr::Sigmoid { inner }
            | Expr::Relu { inner }
            | Expr::Tanh { inner }
            | Expr::Exp { inner }
            | Expr::Sqrt { inner } => inner.walk_bands(out),
            Expr::Pow { base, exp } => {
                base.walk_bands(out);
                exp.walk_bands(out);
            }
            Expr::Where {
                cond, then_, else_, ..
            } => {
                cond.walk_bands(out);
                then_.walk_bands(out);
                else_.walk_bands(out);
            }
            Expr::WeightedBlend {
                primary,
                alt,
                alt_weight,
            } => {
                primary.walk_bands(out);
                alt.walk_bands(out);
                alt_weight.walk_bands(out);
            }
        }
    }
}

/// Per-algorithm temporal recipe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalRecipe {
    /// Lookback windows. Each is materialized via `/v1/backfill` (or
    /// equivalent in-process) and surfaced under
    /// `temporal_composition.windows[]` in the response.
    pub windows: Vec<TemporalWindow>,
    /// Editorial label for the temporal pattern, e.g.
    /// `"flood_event_window"`, `"drought_compounding"`. Surfaced in the
    /// response so the agent has a single phrase to use in its reply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// One-sentence operator note — *why* these windows, in plain math
    /// rather than code. Goes verbatim into the response so the agent
    /// can quote it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
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

            // I1 — when an `inference` block is present, the primary
            // tier must appear at position 0 of `tier_chain`. Without
            // this rule an entry could declare `tier=gpu` while the
            // chain led with `cpu`, and the dispatcher would skip the
            // GPU path entirely. `Absence` is never valid as a primary
            // tier — it's a terminal sentinel only.
            if let Some(inf) = &a.inference {
                if matches!(inf.tier, InferenceTierKind::Absence) {
                    return Err(ManifestError::Invalid(format!(
                        "algorithm {}: inference.tier cannot be Absence; \
                         Absence is a tier_chain terminal sentinel only",
                        a.key
                    )));
                }
                if !inf.tier_chain.is_empty() && inf.tier_chain.first() != Some(&inf.tier) {
                    return Err(ManifestError::Invalid(format!(
                        "algorithm {}: inference.tier_chain[0] = {:?} but inference.tier = {:?}; \
                         the primary tier must lead the chain",
                        a.key,
                        inf.tier_chain.first(),
                        inf.tier
                    )));
                }
                // I2 — DegradeWithTierLabel is only safe when every
                // chain step produces a byte-compatible output schema
                // (Cached and Scalar are both schema-stable; Cpu may
                // not be — flag as editorial caution rather than
                // hard-reject so the manifest stays additive).
                if matches!(
                    inf.fallback_strategy,
                    TierFallbackStrategy::DegradeWithTierLabel
                ) {
                    let has_unsafe_cpu_in_chain = inf
                        .tier_chain
                        .iter()
                        .any(|t| matches!(t, InferenceTierKind::Cpu));
                    if has_unsafe_cpu_in_chain && matches!(inf.tier, InferenceTierKind::Gpu) {
                        // Editorial caution: encoder swap mid-chain
                        // breaks the bands_cid contract. We allow it
                        // (some algorithms genuinely have a contract-
                        // safe CPU twin) but every such entry SHOULD
                        // also declare `accuracy_band` so the contract
                        // gap is visible to clients.
                        if a.accuracy_band.is_none() {
                            return Err(ManifestError::Invalid(format!(
                                "algorithm {}: gpu→cpu degrade_with_tier_label requires \
                                 accuracy_band so clients see the contract gap",
                                a.key
                            )));
                        }
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

    /// Evaluate the algorithm in-process. Returns `Ok(value)` when the
    /// algorithm has an `evaluation` AST and every required band is
    /// present in `samples`, `Ok(None)` when the algorithm has no
    /// evaluation (an `Algorithm` that only ships a human-readable
    /// formula), and `Err(missing_band)` when an evaluation expects a
    /// band that is absent from `samples`.
    ///
    /// The dispatcher in `emem-api-rest` wraps this with the recall
    /// step: walk `evaluation.referenced_bands()`, materialize each
    /// one for the cell, drop the resulting scalars into a sample
    /// map, and call back here.
    pub fn evaluate(
        &self,
        key: &str,
        samples: &std::collections::HashMap<String, f64>,
    ) -> Result<Option<f64>, String> {
        let alg = self
            .lookup(key)
            .ok_or_else(|| format!("unknown algorithm: {key}"))?;
        let Some(expr) = alg.evaluation.as_ref() else {
            return Ok(None);
        };
        for b in expr.referenced_bands() {
            if !samples.contains_key(&b) {
                return Err(format!("missing input band for {key}: {b}"));
            }
        }
        Ok(expr.evaluate(samples))
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
    fn expr_evaluates_linear_with_bias() {
        let mut samples = std::collections::HashMap::new();
        samples.insert("a".to_string(), 2.0);
        samples.insert("b".to_string(), 3.0);
        let mut weights = std::collections::BTreeMap::new();
        weights.insert("a".to_string(), 0.5);
        weights.insert("b".to_string(), 1.5);
        let e = Expr::Linear {
            weights,
            bias: 10.0,
        };
        assert_eq!(e.evaluate(&samples), Some(0.5 * 2.0 + 1.5 * 3.0 + 10.0));
    }

    #[test]
    fn expr_returns_none_on_missing_band() {
        let samples: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
        let e = Expr::Band {
            band: "missing".into(),
        };
        assert_eq!(e.evaluate(&samples), None);
    }

    #[test]
    fn expr_where_branches_on_threshold() {
        let mut samples = std::collections::HashMap::new();
        samples.insert("dem_diff".to_string(), 7.0);
        let e = Expr::Where {
            cond: Box::new(Expr::Band {
                band: "dem_diff".into(),
            }),
            gt: 5.0,
            then_: Box::new(Expr::Const { value: 0.5 }),
            else_: Box::new(Expr::Const { value: 1.0 }),
        };
        assert_eq!(e.evaluate(&samples), Some(0.5));
        samples.insert("dem_diff".to_string(), 2.0);
        assert_eq!(e.evaluate(&samples), Some(1.0));
    }

    #[test]
    fn expr_sigmoid_is_centered_at_zero() {
        let samples: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
        let e = Expr::Sigmoid {
            inner: Box::new(Expr::Const { value: 0.0 }),
        };
        assert!((e.evaluate(&samples).unwrap() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn expr_relu_is_zero_for_negative() {
        let samples: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
        let neg = Expr::Relu {
            inner: Box::new(Expr::Const { value: -3.0 }),
        };
        let pos = Expr::Relu {
            inner: Box::new(Expr::Const { value: 4.0 }),
        };
        assert_eq!(neg.evaluate(&samples), Some(0.0));
        assert_eq!(pos.evaluate(&samples), Some(4.0));
    }

    #[test]
    fn expr_div_returns_none_for_zero_denominator() {
        let samples: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
        let e = Expr::Div {
            a: Box::new(Expr::Const { value: 1.0 }),
            b: Box::new(Expr::Const { value: 0.0 }),
        };
        assert_eq!(e.evaluate(&samples), None);
    }

    #[test]
    fn flood_risk_v2_evaluates_to_a_real_number_from_dispatcher() {
        // Register a sample for every band the v2 evaluator needs.
        // Picked to fall in plausible mid-range values: the cell has
        // some historical recurrence, low elevation, DEM agreement
        // within tolerance, and very negative S1 backscatter
        // (= probable open water).
        let mut samples = std::collections::HashMap::new();
        samples.insert("surface_water.recurrence".to_string(), 40.0); // 40% recurrence
        samples.insert("copdem30m.elevation_mean".to_string(), 30.0); // 30 m amsl
        samples.insert("gmrt.topobathy_mean".to_string(), 28.0); // close to Cop-DEM
        samples.insert("sentinel1_raw".to_string(), -18.0); // wet-ish backscatter

        let r = &*DEFAULT;
        let v = r
            .evaluate("flood_risk@2", &samples)
            .expect("flood_risk@2 evaluation must succeed with all bands present")
            .expect("flood_risk@2 must have an evaluation Expr (added in 0.0.3)");
        // history term: 0.55 * 0.4 = 0.22
        // dem-agreement: |30-28|=2, NOT > 5, so factor = 1.0
        // elevation term: 1.0 * relu(50-30)/50 = 20/50 = 0.4 -> 0.25 * 0.4 = 0.10
        // radar term: sigmoid((-15 - -18)/2) = sigmoid(1.5) ≈ 0.818
        //              -> 0.20 * 0.818 ≈ 0.1636
        // total ≈ 0.4836
        assert!(
            (v - 0.4836_f64).abs() < 0.005,
            "flood_risk@2 numeric ≠ expected 0.4836: got {v}"
        );
    }

    #[test]
    fn algorithm_evaluate_returns_err_on_missing_input() {
        let samples = std::collections::HashMap::new();
        let r = &*DEFAULT;
        let err = r
            .evaluate("flood_risk@2", &samples)
            .expect_err("missing inputs must error, not silently return None");
        assert!(err.contains("missing input band for flood_risk@2"));
    }

    #[test]
    fn algorithm_evaluate_returns_ok_none_when_no_evaluation_field() {
        let r = &*DEFAULT;
        // flood_risk@1 stays as a no-evaluation algorithm so the agent
        // composes the formula itself; the dispatcher MUST return
        // Ok(None), not an error.
        let samples = std::collections::HashMap::new();
        let v = r
            .evaluate("flood_risk@1", &samples)
            .expect("v1 has no evaluation but should not error");
        assert!(v.is_none());
    }

    #[test]
    fn expr_referenced_bands_walks_the_tree() {
        let e = Expr::Add {
            terms: vec![
                Expr::Band { band: "a".into() },
                Expr::Mul {
                    terms: vec![
                        Expr::Band { band: "b".into() },
                        Expr::Sigmoid {
                            inner: Box::new(Expr::Band { band: "c".into() }),
                        },
                    ],
                },
                Expr::Where {
                    cond: Box::new(Expr::Band { band: "d".into() }),
                    gt: 0.0,
                    then_: Box::new(Expr::Const { value: 1.0 }),
                    else_: Box::new(Expr::Band { band: "a".into() }),
                },
            ],
        };
        let bands = e.referenced_bands();
        assert_eq!(bands, vec!["a", "b", "c", "d"]);
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

    #[test]
    fn expr_tanh_matches_libm() {
        let samples: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
        let e = Expr::Tanh {
            inner: Box::new(Expr::Const { value: 0.5 }),
        };
        let v = e.evaluate(&samples).expect("tanh evaluates");
        assert!((v - 0.5_f64.tanh()).abs() < 1e-12);
        // tanh is odd and bounded
        let neg = Expr::Tanh {
            inner: Box::new(Expr::Const { value: -10.0 }),
        };
        assert!(neg.evaluate(&samples).unwrap() < -0.999);
    }

    #[test]
    fn expr_exp_matches_libm_and_rejects_overflow() {
        let samples: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
        let e = Expr::Exp {
            inner: Box::new(Expr::Const { value: 1.0 }),
        };
        let v = e.evaluate(&samples).expect("exp(1) evaluates");
        assert!((v - std::f64::consts::E).abs() < 1e-12);
        // exp overflow → None
        let big = Expr::Exp {
            inner: Box::new(Expr::Const { value: 1e9 }),
        };
        assert_eq!(big.evaluate(&samples), None);
    }

    #[test]
    fn expr_sqrt_evaluates_and_rejects_negative() {
        let samples: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
        let e = Expr::Sqrt {
            inner: Box::new(Expr::Const { value: 9.0 }),
        };
        assert_eq!(e.evaluate(&samples), Some(3.0));
        let neg = Expr::Sqrt {
            inner: Box::new(Expr::Const { value: -1.0 }),
        };
        assert_eq!(neg.evaluate(&samples), None);
    }

    #[test]
    fn expr_pow_handles_fractional_exponent() {
        let samples: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
        let e = Expr::Pow {
            base: Box::new(Expr::Const { value: 16.0 }),
            exp: Box::new(Expr::Const { value: 0.5 }),
        };
        assert_eq!(e.evaluate(&samples), Some(4.0));
        // pow(-1, 0.5) is NaN -> None
        let bad = Expr::Pow {
            base: Box::new(Expr::Const { value: -1.0 }),
            exp: Box::new(Expr::Const { value: 0.5 }),
        };
        assert_eq!(bad.evaluate(&samples), None);
    }

    #[test]
    fn newly_ast_algorithms_evaluate_against_synthetic_inputs() {
        // Every algorithm we wired up in this batch must round-trip
        // from samples → finite numeric value. One synthetic input map
        // is reused across all of them, restricted to the bands each
        // algorithm references via `referenced_bands()`. This is the
        // registry-level smoke test that catches a typo in `op` keys
        // or a missing-band reference before the dispatcher hits it.
        let mut samples = std::collections::HashMap::new();
        samples.insert("indices.ndvi".to_string(), 0.45);
        samples.insert("indices.ndmi".to_string(), 0.2);
        samples.insert("indices.ndsi".to_string(), 0.5);
        samples.insert("indices.nbr".to_string(), -0.1);
        samples.insert("indices.bsi".to_string(), 0.10);
        samples.insert("indices.ndbi".to_string(), 0.15);
        samples.insert("surface_water.recurrence".to_string(), 30.0);
        samples.insert("sentinel1_raw".to_string(), -17.0);
        samples.insert("overture.buildings.count".to_string(), 250.0);
        samples.insert("overture.transportation.road_length_m".to_string(), 1200.0);
        samples.insert("weather.temperature_2m".to_string(), 28.0);
        samples.insert("weather.relative_humidity_2m".to_string(), 55.0);
        samples.insert("weather.wind_speed_10m".to_string(), 5.0);
        samples.insert("weather.precipitation_mm".to_string(), 1.0);

        let r = &*DEFAULT;
        for key in [
            "flood_history_class@1",
            "water_likelihood_from_vv@1",
            "vegetation_class_from_ndvi@1",
            "crop_stress_score@1",
            "snow_likelihood_from_ndsi@1",
            "burn_likelihood_from_nbr@1",
            "bare_soil_class@1",
            "built_up_from_ndbi@1",
            "urban_density_score@1",
            "wind_chill@1",
            "fosberg_fire_weather_index@1",
            "precip_intensity_class@1",
            "vapor_pressure_deficit@1",
        ] {
            let v = r
                .evaluate(key, &samples)
                .unwrap_or_else(|e| panic!("{key} evaluation failed: {e}"))
                .unwrap_or_else(|| panic!("{key} has no evaluation AST"));
            assert!(
                v.is_finite(),
                "{key} produced non-finite value {v} from synthetic inputs"
            );
        }
    }

    #[test]
    fn vegetation_class_from_ndvi_returns_dense_class_above_threshold() {
        let mut samples = std::collections::HashMap::new();
        samples.insert("indices.ndvi".to_string(), 0.75);
        let r = &*DEFAULT;
        let v = r
            .evaluate("vegetation_class_from_ndvi@1", &samples)
            .expect("class evaluation runs")
            .expect("ast present");
        // 0.75 > 0.5 → 'dense' = 4
        assert_eq!(v, 4.0);
        samples.insert("indices.ndvi".to_string(), -0.3);
        let v_water = r
            .evaluate("vegetation_class_from_ndvi@1", &samples)
            .unwrap()
            .unwrap();
        // < 0 → 'water_or_snow' = 0
        assert_eq!(v_water, 0.0);
    }

    #[test]
    fn precip_intensity_class_uses_wmo_bins() {
        let mut samples = std::collections::HashMap::new();
        let r = &*DEFAULT;
        for (precip, expected) in [
            (0.0_f64, 0.0_f64), // none
            (0.1, 1.0),         // trace
            (1.0, 2.0),         // light
            (5.0, 3.0),         // moderate
            (10.0, 4.0),        // heavy
            (60.0, 5.0),        // violent
        ] {
            samples.insert("weather.precipitation_mm".to_string(), precip);
            let v = r
                .evaluate("precip_intensity_class@1", &samples)
                .unwrap()
                .unwrap();
            assert!(
                (v - expected).abs() < 1e-9,
                "precip={precip} expected class {expected} got {v}"
            );
        }
    }

    #[test]
    fn vapor_pressure_deficit_matches_tetens_at_20c() {
        // At T=20°C, RH=50% the FAO-56 reference gives
        // es = 0.6108 * exp(17.27 * 20 / 257.3) = 2.3385 kPa
        // VPD = es * (1 - 0.5) = 1.1693 kPa
        let mut samples = std::collections::HashMap::new();
        samples.insert("weather.temperature_2m".to_string(), 20.0);
        samples.insert("weather.relative_humidity_2m".to_string(), 50.0);
        let r = &*DEFAULT;
        let v = r
            .evaluate("vapor_pressure_deficit@1", &samples)
            .unwrap()
            .unwrap();
        let es = 0.6108_f64 * (17.27_f64 * 20.0 / (20.0 + 237.3)).exp();
        let expected = es * 0.5;
        assert!(
            (v - expected).abs() < 1e-6,
            "VPD ≠ FAO-56 Tetens form: got {v} expected {expected}"
        );
    }

    #[test]
    fn eudr_compliance_v1_uses_multi_product_loss_consensus() {
        let alg = DEFAULT.lookup("eudr_compliance@1").unwrap();
        let inputs: Vec<&str> = alg
            .inputs
            .iter()
            .filter_map(|i| i.band.as_deref())
            .collect();
        assert!(
            inputs.contains(&"forest_change.lossyear"),
            "Hansen lossyear must remain on eudr_compliance@1 inputs"
        );
        assert!(
            inputs.contains(&"jrc_tmf.deforestation_year"),
            "JRC TMF deforestation_year (multi-product) missing"
        );
    }

    #[test]
    fn eudr_compliance_hansen_leg_demands_alive_at_2020() {
        // Synthetic input: cell was 100% canopy in 2000 but cleared in 2010
        // (Hansen lossyear=2010, treecover2000=100). JRC GFC2020 signs
        // Absence (0.0) and JRC TMF signs Absence (0.0). The Hansen-only
        // fallback baseline must now reject this cell: it was NOT forest
        // at the 2020-12-31 cut-off — it had already been cleared 10
        // years earlier. Expected verdict 3 (not_in_scope).
        let mut samples = std::collections::HashMap::new();
        samples.insert("jrc_gfc2020.forest_2020".to_string(), 0.0);
        samples.insert("forest_change.treecover2000".to_string(), 100.0);
        samples.insert("forest_change.lossyear".to_string(), 2010.0);
        samples.insert("jrc_tmf.deforestation_year".to_string(), 0.0);
        let r = DEFAULT.evaluate("eudr_compliance@1", &samples).unwrap();
        assert_eq!(
            r,
            Some(3.0),
            "pre-cutoff cleared cell with absent JRC must be not_in_scope, got {r:?}"
        );
    }

    #[test]
    fn eudr_compliance_hansen_leg_passes_intact_at_2020() {
        // Cell still alive at 2020 (lossyear=0, treecover2000=100). JRC
        // GFC2020 signs Absence (0.0); JRC TMF signs Absence (0.0). The
        // Hansen-only fallback baseline must accept this cell as forest
        // at cut-off and, with no post-cut-off loss, the verdict is 1
        // (pass).
        let mut samples = std::collections::HashMap::new();
        samples.insert("jrc_gfc2020.forest_2020".to_string(), 0.0);
        samples.insert("forest_change.treecover2000".to_string(), 100.0);
        samples.insert("forest_change.lossyear".to_string(), 0.0);
        samples.insert("jrc_tmf.deforestation_year".to_string(), 0.0);
        let r = DEFAULT.evaluate("eudr_compliance@1", &samples).unwrap();
        assert_eq!(r, Some(1.0));
    }

    #[test]
    fn eudr_compliance_hansen_leg_fails_post_2020_loss() {
        // Cell forest at 2020 then lost in 2022 (lossyear=2022,
        // treecover2000=100). Hansen-only fallback baseline must accept
        // this cell at cut-off (lossyear > 2020 means it was still
        // forest at end-of-2020), and the post-cut-off loss check then
        // emits verdict 2 (fail).
        let mut samples = std::collections::HashMap::new();
        samples.insert("jrc_gfc2020.forest_2020".to_string(), 0.0);
        samples.insert("forest_change.treecover2000".to_string(), 100.0);
        samples.insert("forest_change.lossyear".to_string(), 2022.0);
        samples.insert("jrc_tmf.deforestation_year".to_string(), 0.0);
        let r = DEFAULT.evaluate("eudr_compliance@1", &samples).unwrap();
        assert_eq!(r, Some(2.0), "post-cutoff loss must be fail, got {r:?}");
    }

    #[test]
    fn water_likelihood_from_vv_saturates_below_minus_20_db() {
        let mut samples = std::collections::HashMap::new();
        let r = &*DEFAULT;
        samples.insert("sentinel1_raw".to_string(), -20.0);
        let v_wet = r
            .evaluate("water_likelihood_from_vv@1", &samples)
            .unwrap()
            .unwrap();
        // sigmoid((-15 - -20)/2) = sigmoid(2.5) ≈ 0.924
        assert!(
            v_wet > 0.9,
            "expected high water prob at -20 dB, got {v_wet}"
        );
        samples.insert("sentinel1_raw".to_string(), -5.0);
        let v_dry = r
            .evaluate("water_likelihood_from_vv@1", &samples)
            .unwrap()
            .unwrap();
        // sigmoid((-15 - -5)/2) = sigmoid(-5) ≈ 0.0067
        assert!(
            v_dry < 0.05,
            "expected low water prob at -5 dB, got {v_dry}"
        );
    }
}
