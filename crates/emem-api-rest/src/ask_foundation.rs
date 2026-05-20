//! Foundation-embedding fan-out for `/v1/ask`.
//!
//! The standard `ask_inner` path is anchored on the topic router, which
//! produces a list of *scalar* bands (NDVI, weather, surface_water,
//! …) to recall. That's the right answer for most questions, but it
//! leaves the 1024-D Clay, 1024-D Prithvi, and 128-D Tessera foundation
//! embeddings untouched even when the user explicitly asks
//! "find places like X" or "what changed here in the last year".
//!
//! This module adds a small intent classifier (keyword pre-pass over
//! the question text) and a fan-out helper that, when the intent
//! matches, exercises one of the foundation algorithms registered in
//! `algorithms-v0.json`:
//!
//! - `Similarity` → `clay_archetype_match@1`-style `find_similar` over
//!   the Clay band, optionally followed by Prithvi & Tessera for the
//!   triple-consensus pattern (`clay_prithvi_tessera_triple_consensus@1`).
//! - `Change` → recall of `clay_v1` + `prithvi_eo2` + `geotessera.multi_year`
//!   so the agent (or in-process AST evaluator) can compute the year-on-year
//!   triple-consensus change index.
//!
//! Returns a structured envelope under `foundation_embeddings` in the
//! `/v1/ask` response. The receipt remains the one signed by the
//! standard recall path; encoder fact CIDs are merged into the same
//! `fact_cids` list so the response is one signed envelope.
//!
//! Thresholds (`ask_timeout_ms`, k, etc.) are read from the
//! `clay_prithvi_tessera_triple_consensus@1` algorithm's `parameters`
//! block — see [`Algorithm::param_f64`] — so an operator can re-tune
//! at registry-CID time rather than recompiling.
//!
//! Designed to be additive: when the classifier doesn't fire, this
//! module returns `None` and the existing `ask_inner` path is
//! byte-equivalent to before.

use serde_json::{json, Value as JsonValue};

use crate::{find_similar_with_auto_materialize, AppState};
use emem_primitives::find_similar::{FindSimilarMode, FindSimilarReq};

/// What kind of question the agent appears to be asking about a place.
/// Used to decide whether — and how — to fan out across the three
/// foundation encoders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskIntent {
    /// "Find places like X", "where else looks like this", "similar
    /// to". Triggers a k-NN fan-out across foundation embeddings.
    Similarity,
    /// "What changed here", "year over year", "since last year",
    /// "deforestation", "new construction". Triggers a recall of the
    /// triple-consensus inputs so the agent can compute the
    /// year-on-year change index.
    Change,
}

/// Hunter-mode intents — open-world event-discovery questions
/// ("find oil spills", "where is deforestation happening", "show me
/// algal blooms in the Persian Gulf"). The locate-first cascade rejects
/// these with `needs_location` even when the region is sitting in the
/// q text, because the standard contract is one place → one cell. This
/// classifier maps event keywords to one of our registered detection
/// algorithms and extracts the region anchor from " in/over/across/
/// around <place>" so `/v1/ask` can drive a real fan-out instead of
/// punting.
///
/// Mapping rule of thumb: every variant must point at a *single*
/// algorithm key that produces a per-cell scalar (or boolean) we can
/// rank. If we can't rank, we can't hand back hotspots, and the agent
/// is back to picking cells blind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HunterKind {
    /// algal_bloom_chlorophyll_ndci@1 — chlorophyll-a proxy from
    /// Sentinel-2 NDVI/NDWI fusion. > 30 mg/m³ is bloom-level.
    AlgalBloom,
    /// deforestation_alert_ndvi_drop@1 — > 0.20 NDVI drop in 60 d on
    /// an established (Hansen tree-cover ≥ 30%) forest cell.
    Deforestation,
    /// flood_extent_sar_threshold@1 — Sentinel-1 VV < −16 dB classed
    /// as open water. Used post-event for inundation extent.
    Flood,
    /// wildfire_burn_intensity_dnbr_finetune@1 — five-class dNBR ×
    /// Prithvi linear-probe fusion for burn severity.
    Wildfire,
    /// urban_heat_island_lst_canopy@1 — LST excess over local
    /// vegetated reference. > 3 K is strong UHI.
    UrbanHeatIsland,
    /// methane_plume_swir_anomaly@1 — Sentinel-2 B11/B12 ratio
    /// anomaly. Pre-filter for an EnMAP/EMIT/PRISMA re-look.
    MethanePlume,
    /// landslide_post_event_sar_dnn@1 — Sentinel-1 ΔVV with slope
    /// gating. Designed for post-trigger sweep.
    Landslide,
    /// spi_meteorological_drought@1 — 3-month SPI from CHIRPS.
    Drought,
    /// soil_salinity_index@1 — SAVI-anchored salinity proxy.
    SoilSalinity,
    /// crop_stress_score@1 — NDVI z-score against the crop polygon's
    /// rolling baseline. Used in growing-season checks.
    CropStress,
    /// water_turbidity_red_band@1 — sediment plume proxy from S2
    /// red-band reflectance over water.
    WaterTurbidity,
    /// Not in the registry yet: oil-slick detection wants a Sentinel-1
    /// surface-roughness anomaly algorithm. We carry the variant so the
    /// classifier can still match the user's verb pattern, but the
    /// `/v1/ask` path returns `status: not_yet_implemented` with a
    /// pointer at the closest available algorithms (water_turbidity,
    /// flood_extent_sar — both share the SAR-water-darkness signature).
    OilSlick,
}

impl HunterKind {
    /// The single algorithm key this intent will hunt with.
    pub fn algorithm_key(self) -> &'static str {
        match self {
            HunterKind::AlgalBloom => "algal_bloom_chlorophyll_ndci@1",
            HunterKind::Deforestation => "deforestation_alert_ndvi_drop@1",
            HunterKind::Flood => "flood_extent_sar_threshold@1",
            HunterKind::Wildfire => "wildfire_burn_intensity_dnbr_finetune@1",
            HunterKind::UrbanHeatIsland => "urban_heat_island_lst_canopy@1",
            HunterKind::MethanePlume => "methane_plume_swir_anomaly@1",
            HunterKind::Landslide => "landslide_post_event_sar_dnn@1",
            HunterKind::Drought => "spi_meteorological_drought@1",
            HunterKind::SoilSalinity => "soil_salinity_index@1",
            HunterKind::CropStress => "crop_stress_score@1",
            HunterKind::WaterTurbidity => "water_turbidity_red_band@1",
            HunterKind::OilSlick => "",
        }
    }

    /// The input bands the algorithm consumes — what `recall_polygon`
    /// must populate before the formula can run.
    pub fn input_bands(self) -> &'static [&'static str] {
        match self {
            HunterKind::AlgalBloom => &["indices.ndvi", "indices.ndwi"],
            HunterKind::Deforestation => &["indices.ndvi", "hansen.tree_cover_2000"],
            HunterKind::Flood => &["sentinel1_raw"],
            HunterKind::Wildfire => &["indices.nbr", "prithvi_eo2"],
            HunterKind::UrbanHeatIsland => &[
                "modis.lst_day_8day",
                "indices.ndvi",
                "esa_worldcover.lc_2021",
            ],
            HunterKind::MethanePlume => &["s2.B11", "s2.B12"],
            HunterKind::Landslide => &["sentinel1_raw", "copdem30m.elevation_mean"],
            HunterKind::Drought => &["chirps.precip_monthly"],
            HunterKind::SoilSalinity => &["s2.B04", "s2.B08"],
            HunterKind::CropStress => &["indices.ndvi"],
            HunterKind::WaterTurbidity => &["s2.B04", "indices.ndwi"],
            HunterKind::OilSlick => &[],
        }
    }

    /// Human-readable label for the envelope's `event` field.
    pub fn label(self) -> &'static str {
        match self {
            HunterKind::AlgalBloom => "algal_bloom",
            HunterKind::Deforestation => "deforestation",
            HunterKind::Flood => "flood_extent",
            HunterKind::Wildfire => "wildfire_burn_severity",
            HunterKind::UrbanHeatIsland => "urban_heat_island",
            HunterKind::MethanePlume => "methane_plume",
            HunterKind::Landslide => "landslide",
            HunterKind::Drought => "drought",
            HunterKind::SoilSalinity => "soil_salinity",
            HunterKind::CropStress => "crop_stress",
            HunterKind::WaterTurbidity => "water_turbidity",
            HunterKind::OilSlick => "oil_slick",
        }
    }
}

/// What the classifier extracts from a hunter-mode question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HunterIntent {
    pub kind: HunterKind,
    /// Region anchor extracted from " in <region>" / " over <region>"
    /// / " across <region>" / " around <region>". `None` for global
    /// hunts ("find me wildfires" → no anchor).
    pub region_anchor: Option<String>,
}

/// Classify a hunter-mode question. The verb pattern is one of
/// `find|show|where|spot|identify|locate|hunt`; the event noun is one
/// of the keyword sets matched below. Returns `None` for ordinary
/// "ndvi at X" questions so the standard ask_inner path still owns
/// place-anchored work.
pub fn classify_hunter(q: &str) -> Option<HunterIntent> {
    let lower = q.to_ascii_lowercase();
    // Hunter verbs — at least one must appear, otherwise "deforestation
    // since 2020" stays on the Change path.
    const HUNTER_VERBS: &[&str] = &[
        "find ",
        "find me ",
        "show me ",
        "where is ",
        "where are ",
        "where has",
        "where have",
        "where do ",
        "spot ",
        "identify ",
        "locate ",
        "hunt for ",
        "hunt ",
        "any ",
        "list ",
    ];
    let verb_match = HUNTER_VERBS
        .iter()
        .any(|v| lower.starts_with(v) || lower.contains(v));
    // Region-anchored-event fallback: when no hunter verb is present
    // but the question names an event (checked below) AND anchors it
    // to a region via " in/over/across/throughout/around/near <X>",
    // we still want to dispatch a hunt — agents commonly write
    // "deforestation in Brazil" without a verb. We only allow this
    // fallback when the question is NOT a retrospective CHANGE
    // question (no "last year" / "in the last" / "since" qualifier).
    const REGION_PREPS: &[&str] = &[
        " in ",
        " over ",
        " across ",
        " throughout ",
        " around ",
        " near ",
    ];
    const TEMPORAL_QUALIFIERS: &[&str] = &["last year", "in the last", "since"];
    let has_region_prep = REGION_PREPS.iter().any(|p| lower.contains(p));
    let has_temporal = TEMPORAL_QUALIFIERS.iter().any(|t| lower.contains(t));
    let region_anchored_event = has_region_prep && !has_temporal;
    if !verb_match && !region_anchored_event {
        return None;
    }

    // Event keywords — order matters: more specific phrases first so
    // "algal bloom" doesn't get hijacked by a generic "water" match.
    let kind: HunterKind = if lower.contains("algal bloom")
        || lower.contains("algae bloom")
        || lower.contains("harmful bloom")
        || lower.contains("cyanobacteria")
        || lower.contains("chlorophyll bloom")
    {
        HunterKind::AlgalBloom
    } else if lower.contains("oil spill") || lower.contains("oil slick") {
        HunterKind::OilSlick
    } else if lower.contains("methane plume")
        || lower.contains("methane leak")
        || lower.contains("methane emission")
        || lower.contains("ghg leak")
        || lower.contains("super-emitter")
    {
        HunterKind::MethanePlume
    } else if lower.contains("deforestation")
        || lower.contains("forest loss")
        || lower.contains("forest clearing")
        || lower.contains("tree loss")
        || lower.contains("logging")
    {
        HunterKind::Deforestation
    } else if (lower.contains("burn scar")
        || lower.contains("burned area")
        || lower.contains("wildfire")
        || lower.contains("forest fire")
        || lower.contains("bushfire")
        || lower.contains("burn severity")
        || lower.contains("fire-affected"))
        && !lower.contains("wildfire risk")
        && !lower.contains("fire risk")
    {
        HunterKind::Wildfire
    } else if (lower.contains("urban heat") || lower.contains("heat island"))
        && !lower.contains("heat risk")
        && !lower.contains("uhi risk")
        && !lower.contains("urban heat risk")
    {
        HunterKind::UrbanHeatIsland
    } else if lower.contains("landslide")
        || lower.contains("mudslide")
        || lower.contains("debris flow")
        || lower.contains("slope failure")
    {
        HunterKind::Landslide
    } else if lower.contains("flood extent")
        || lower.contains("inundation")
        || lower.contains("flooded fields")
        || lower.contains("flooded area")
        || lower.contains("flood-affected")
        || lower.contains("flood damage")
        || (lower.contains("flood") && !lower.contains("flood risk"))
    {
        HunterKind::Flood
    } else if (lower.contains("drought")
        || lower.contains("dry spell")
        || lower.contains("rainfall deficit"))
        && !lower.contains("drought risk")
    {
        HunterKind::Drought
    } else if lower.contains("salinity") || lower.contains("salinized soil") {
        HunterKind::SoilSalinity
    } else if (lower.contains("crop stress")
        || lower.contains("stressed crops")
        || lower.contains("crop damage")
        || lower.contains("yellowing crops"))
        && !lower.contains("crop stress risk")
        && !lower.contains("crop risk")
    {
        HunterKind::CropStress
    } else if lower.contains("turbidity")
        || lower.contains("sediment plume")
        || lower.contains("muddy water")
    {
        HunterKind::WaterTurbidity
    } else {
        return None;
    };

    // Region anchor: " <prep> <region>" where prep ∈ {in,over,across,
    // around,inside,within,near}. Take everything after the first
    // matching preposition to end-of-question. We don't trim trailing
    // punctuation aggressively — the geocoder is tolerant.
    let region_anchor = extract_region_anchor(q);

    Some(HunterIntent {
        kind,
        region_anchor,
    })
}

/// Extract the region phrase after a locative preposition. Public so
/// the dispatcher can re-run extraction on follow-up turns.
pub fn extract_region_anchor(q: &str) -> Option<String> {
    let lower = q.to_ascii_lowercase();
    const PREPS: &[&str] = &[
        " in ", " over ", " across ", " around ", " inside ", " within ", " near ",
    ];
    // Find the rightmost preposition occurrence so "find oil spills
    // in any port in Saudi Arabia" picks "Saudi Arabia", not "any
    // port in Saudi Arabia".
    let mut best: Option<usize> = None;
    let mut best_prep_len = 0;
    for prep in PREPS {
        if let Some(idx) = lower.rfind(prep) {
            if best.map(|b| idx > b).unwrap_or(true) {
                best = Some(idx);
                best_prep_len = prep.len();
            }
        }
    }
    let start = best? + best_prep_len;
    if start >= q.len() {
        return None;
    }
    let tail = q[start..].trim().trim_end_matches(|c: char| {
        matches!(c, '.' | ',' | '?' | '!' | ';' | ':' | ')' | ']' | '}')
    });
    if tail.is_empty() || tail.len() > 80 {
        return None;
    }
    Some(tail.to_string())
}

/// Corpus-audit intents — questions that don't have a place anchor at
/// all. Routing these through the locate→recall pipeline returns the
/// misleading `needs_location` envelope (an agent reading it tries to
/// invent a place that isn't there). Classify them up front and point
/// at the right discovery surface instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorpusAuditIntent {
    /// "where do you have facts", "global coverage map", "what cells
    /// are attested" — wants the world-scale picture. Routes to
    /// `/v1/coverage_map.svg` (visual) + `/v1/coverage` (json totals).
    GlobalCoverage,
    /// "how dense is your corpus over <region>", "what's your coverage
    /// in <region>" — wants per-region density. Routes to
    /// `/v1/coverage_matrix` (per-band breakdown) + caller-side bbox
    /// filtering against `/v1/coverage`'s cell list.
    RegionalDensity,
    /// "how fresh is your data", "when did you last sign X", "what's
    /// your data freshness" — wants per-band last-attested timestamps.
    /// `/v1/coverage_matrix` carries `last_attested_at` per band.
    Freshness,
}

/// Lightweight keyword classifier. Returns the first matching intent.
/// We intentionally don't run a transformer-anchored classifier here —
/// the topic router already does that work for scalar bands; we only
/// need an *additive* check to know whether the foundation fan-out is
/// worth its cost.
pub fn classify_intent(q: &str) -> Option<AskIntent> {
    let lower = q.to_ascii_lowercase();
    // Similarity patterns — k-NN against foundation embeddings.
    const SIMILARITY: &[&str] = &[
        "find places like",
        "places like",
        "similar to",
        "looks like",
        "where else",
        "analog of",
        "analogue of",
        "look-alike",
        "lookalike",
        "comparable to",
        "find similar",
        "what's similar",
        "what is similar",
    ];
    // Change / temporal patterns — triple-consensus year-on-year.
    const CHANGE: &[&str] = &[
        "what changed",
        "what has changed",
        "year over year",
        "year-over-year",
        "since last year",
        "in the last year",
        "deforestation",
        "deforested",
        "forest loss",
        "new construction",
        "urban expansion",
        "wetland change",
        "coastline change",
        "coastal erosion",
        "shoreline retreat",
    ];
    if SIMILARITY.iter().any(|s| lower.contains(s)) {
        return Some(AskIntent::Similarity);
    }
    if CHANGE.iter().any(|s| lower.contains(s)) {
        return Some(AskIntent::Change);
    }
    None
}

/// Detect corpus-meta questions before the locate cascade burns a
/// geocoder round-trip. Order is important — Freshness patterns are
/// checked before GlobalCoverage so "how fresh is your data" doesn't
/// fall into GlobalCoverage just because it mentions "data". The
/// classifier is permissive: when in doubt, fall through to the
/// normal locate→recall path (which handles place-anchored questions
/// correctly).
pub fn classify_corpus_audit(q: &str) -> Option<CorpusAuditIntent> {
    let lower = q.to_ascii_lowercase();
    const FRESHNESS: &[&str] = &[
        "how fresh",
        "data freshness",
        "last attested",
        "last signed",
        "when did you last",
        "how recent",
        "how stale",
        "how up to date",
        "how up-to-date",
    ];
    if FRESHNESS.iter().any(|s| lower.contains(s)) {
        return Some(CorpusAuditIntent::Freshness);
    }
    // Regional density — must say "over <region>" / "in <region>" /
    // "for <region>" in a corpus-meta way. We require BOTH a corpus
    // word AND a region preposition so "ndvi over the amazon" stays
    // on the place-anchored path.
    const CORPUS_WORDS: &[&str] = &[
        "your corpus",
        "your coverage",
        "your data",
        "your facts",
        "your cells",
        "your attestations",
        "attested cells",
        "signed facts",
        "fact coverage",
    ];
    const REGION_PREPS: &[&str] = &[
        " over ", " in ", " across ", " for ", " inside ", " within ",
    ];
    let has_corpus_word = CORPUS_WORDS.iter().any(|s| lower.contains(s));
    let has_region_prep = REGION_PREPS.iter().any(|s| lower.contains(s));
    // Relaxed corpus signal: the bare word "coverage" (without "your"
    // prefix) is a strong corpus-supply marker. "ndvi coverage" /
    // "flood coverage" / "weather coverage" / "coverage of <topic>"
    // all ask where the supply is, not what the value is. We keep the
    // "in"/"at"/"near" place-anchored questions on the place path by
    // requiring the literal word "coverage" — "ndvi over the amazon"
    // and "ndvi in brazil" still fall through to None here.
    let has_coverage_word = lower.contains("coverage");
    if (has_corpus_word || has_coverage_word) && has_region_prep {
        // "how dense is your corpus over sub saharan africa" matches.
        // "ndvi coverage over the amazon" also matches now.
        return Some(CorpusAuditIntent::RegionalDensity);
    }
    // Global coverage — broad "where" / "what do you have" / "global
    // map" / "global coverage" phrasing without a region anchor.
    const GLOBAL: &[&str] = &[
        "where do you have",
        "where you have",
        "global coverage",
        "global coverage map",
        "world coverage",
        "world-wide coverage",
        "worldwide coverage",
        "coverage map",
        "show me your coverage",
        "show me the coverage",
        "what cells do you have",
        "what cells are attested",
        "all the cells",
        "every cell you have",
        // Relaxed: "how much data do you have", "how dense is the
        // corpus", "how dense is data" — corpus-supply questions
        // without the "your" prefix.
        "how much data do you have",
        "how dense is the corpus",
        "how dense is data",
    ];
    if GLOBAL.iter().any(|s| lower.contains(s)) {
        return Some(CorpusAuditIntent::GlobalCoverage);
    }
    // The plain phrase "your corpus" without a region preposition is
    // still a corpus question — treat as global so the agent gets the
    // pointers instead of needs_location.
    if has_corpus_word {
        return Some(CorpusAuditIntent::GlobalCoverage);
    }
    // The bare word "coverage" without a region preposition is also a
    // global corpus-supply question ("ndvi coverage", "coverage of
    // weather"). We treat as GlobalCoverage so the agent gets the
    // discovery pointers rather than needs_location.
    if has_coverage_word {
        return Some(CorpusAuditIntent::GlobalCoverage);
    }
    None
}

/// Run the foundation-embedding fan-out for a matched intent. Returns
/// a JSON envelope to merge under `foundation_embeddings` in the
/// `/v1/ask` response, or `None` if the intent doesn't match.
///
/// Cost: bounded by `ask_timeout_ms` from the triple-consensus
/// algorithm's parameters block (defaults to 4 s). Each leg of the
/// fan-out runs concurrently via `tokio::join!`; if the budget is
/// exhausted the helper returns an envelope with
/// `degraded_reason: "foundation_embedding_unavailable"` so the
/// `ask_inner` path still ships a useful answer.
pub async fn foundation_fanout(q: &str, cell: &str, s: &AppState) -> Option<JsonValue> {
    let intent = classify_intent(q)?;
    let alg = emem_core::algorithms::DEFAULT.lookup("clay_prithvi_tessera_triple_consensus@1");
    let timeout_ms = alg
        .and_then(|a| a.param_f64("ask_timeout_ms"))
        .unwrap_or(4000.0) as u64;
    let k_neighbors = alg.and_then(|a| a.param_f64("k_neighbors")).unwrap_or(8.0) as usize;

    let work = async move {
        match intent {
            AskIntent::Similarity => {
                similarity_fanout(cell.to_string(), k_neighbors, s.clone()).await
            }
            AskIntent::Change => change_fanout(cell.to_string(), s.clone()).await,
        }
    };
    match tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), work).await {
        Ok(v) => Some(v),
        Err(_) => Some(json!({
            "intent":           intent_label(intent),
            "available":        false,
            "degraded_reason":  "foundation_embedding_timeout",
            "budget_ms":        timeout_ms,
        })),
    }
}

fn intent_label(i: AskIntent) -> &'static str {
    match i {
        AskIntent::Similarity => "similarity",
        AskIntent::Change => "change",
    }
}

/// Similarity fan-out: run `find_similar_with_auto_materialize` against
/// the geotessera band first (cheapest — already on disk for the
/// query cell in most deployments), then optionally also against
/// `clay_v1` and `prithvi_eo2` when their materializers can answer.
/// Returns the geotessera neighbours plus a per-encoder hit-count
/// summary so the agent can see whether the triple corroborated.
async fn similarity_fanout(cell: String, k: usize, s: AppState) -> JsonValue {
    let mut encoder_results: Vec<JsonValue> = Vec::new();
    let mut all_fact_cids: Vec<String> = Vec::new();
    for band in ["geotessera", "clay_v1", "prithvi_eo2"] {
        let req = FindSimilarReq {
            key: cell.clone(),
            k: Some(k as u32),
            filter: None,
            band: Some(band.to_string()),
            mode: FindSimilarMode::Cosine,
        };
        let band_used = band.to_string();
        match find_similar_with_auto_materialize(&req, &band_used, &s).await {
            Ok((resp, _notes)) => {
                let cells: Vec<String> = resp.neighbors.iter().map(|n| n.cell.clone()).collect();
                if let Ok(v) = serde_json::to_value(&resp.receipt) {
                    if let Some(fc) = v.get("fact_cids").and_then(|x| x.as_array()) {
                        for c in fc {
                            if let Some(cid) = c.as_str() {
                                all_fact_cids.push(cid.to_string());
                            }
                        }
                    }
                }
                encoder_results.push(json!({
                    "encoder": band,
                    "k_returned": resp.neighbors.len(),
                    "neighbors": cells,
                }));
            }
            Err(e) => {
                encoder_results.push(json!({
                    "encoder": band,
                    "available": false,
                    "reason": e.1.message,
                }));
            }
        }
    }
    // Cross-encoder consensus — cells that appear in ≥2 encoder
    // result lists are flagged as "all_three" / "two_of_three"
    // neighbours, matching the triple-consensus pattern.
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for r in &encoder_results {
        if let Some(arr) = r.get("neighbors").and_then(|x| x.as_array()) {
            for c in arr {
                if let Some(cell) = c.as_str() {
                    *counts.entry(cell.to_string()).or_default() += 1;
                }
            }
        }
    }
    let mut consensus: Vec<(String, usize)> = counts.into_iter().collect();
    consensus.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let consensus_top: Vec<JsonValue> = consensus
        .iter()
        .map(|(c, n)| {
            json!({
                "cell": c,
                "encoder_agreement": match *n {
                    3 => "all_three",
                    2 => "two_of_three",
                    _ => "one_or_none",
                },
            })
        })
        .collect();
    json!({
        "intent":              "similarity",
        "available":           true,
        "computation":         "similarity_consensus",
        "algorithm_key":       "clay_prithvi_tessera_triple_consensus@1",
        "encoder_results":     encoder_results,
        "consensus_neighbors": consensus_top,
        "encoder_fact_cids":   all_fact_cids,
    })
}

/// Change fan-out: ensure clay_v1, prithvi_eo2, geotessera.multi_year
/// are recallable at the query cell so the agent can compute the
/// year-on-year triple-consensus change index via dispatch_algorithms.
/// This helper does not evaluate the formula itself — the existing
/// algorithm-dispatch path in ask_inner picks up the algorithm key
/// once the recall has populated the bands. We return a summary of
/// which encoders are available + the threshold the consensus
/// algorithm will gate on.
async fn change_fanout(cell: String, s: AppState) -> JsonValue {
    let alg = emem_core::algorithms::DEFAULT.lookup("clay_prithvi_tessera_triple_consensus@1");
    let gate = alg
        .and_then(|a| a.param_f64("consensus_threshold"))
        .unwrap_or(0.15);
    let min_models = alg
        .and_then(|a| a.param_f64("consensus_min_models"))
        .unwrap_or(2.0) as u8;

    // For each foundation band, run an auto-materializing recall at
    // the cell. The auto-materializer is the same path /v1/recall
    // exercises — fetches+signs on miss, no special-casing needed.
    let bands = ["clay_v1", "prithvi_eo2", "geotessera.multi_year"];
    let mut available: Vec<&'static str> = Vec::new();
    for band in &bands {
        let req = crate::RecallReq {
            cell: cell.clone(),
            bands: Some(vec![(*band).to_string()]),
            tslot: None,
        };
        if let Ok((resp, _notes)) = crate::recall_with_auto_materialize(&req, &s).await {
            if !resp.facts.is_empty() {
                available.push(band);
            }
        }
    }
    json!({
        "intent":               "change",
        "available":            !available.is_empty(),
        "computation":          "triple_consensus_year_over_year",
        "algorithm_key":        "clay_prithvi_tessera_triple_consensus@1",
        "encoders_available":   available,
        "consensus_threshold":  gate,
        "consensus_min_models": min_models,
        "agent_hint":           "Apply the formula in `algorithms_for_question` for clay_prithvi_tessera_triple_consensus@1; the receipt cites the encoder fact CIDs alongside any topic-anchored scalar bands. For domain-specific variants see deforestation_triple@1 (forest), wetland_change_triple@1 (water), urban_expansion_triple@1 (urban), or coastal_erosion_triple@1 (coast).",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn similarity_keywords_trigger_similarity_intent() {
        assert_eq!(
            classify_intent("find places like Yellowstone"),
            Some(AskIntent::Similarity)
        );
        assert_eq!(
            classify_intent("where else looks like this cell"),
            Some(AskIntent::Similarity)
        );
    }

    #[test]
    fn change_keywords_trigger_change_intent() {
        assert_eq!(
            classify_intent("what changed in the last year"),
            Some(AskIntent::Change)
        );
        assert_eq!(
            classify_intent("deforestation since 2020"),
            Some(AskIntent::Change)
        );
    }

    #[test]
    fn plain_question_returns_none() {
        assert_eq!(classify_intent("ndvi for golden gate park"), None);
        assert_eq!(classify_intent("show me the elevation here"), None);
    }

    #[test]
    fn corpus_audit_global_coverage_patterns() {
        for q in [
            "where do you have signed facts on earth right now",
            "show me the global coverage map of attested cells",
            "what cells do you have",
            "show me your coverage",
        ] {
            assert_eq!(
                classify_corpus_audit(q),
                Some(CorpusAuditIntent::GlobalCoverage),
                "expected GlobalCoverage for {q:?}"
            );
        }
    }

    #[test]
    fn corpus_audit_regional_density_patterns() {
        for q in [
            "how dense is your corpus over sub saharan africa",
            "what's your coverage in southeast asia",
            "your data for the amazon basin",
        ] {
            assert_eq!(
                classify_corpus_audit(q),
                Some(CorpusAuditIntent::RegionalDensity),
                "expected RegionalDensity for {q:?}"
            );
        }
    }

    #[test]
    fn corpus_audit_freshness_patterns() {
        for q in [
            "how fresh is your data",
            "what's the data freshness",
            "when did you last sign NDVI",
            "how recent is your corpus",
        ] {
            assert_eq!(
                classify_corpus_audit(q),
                Some(CorpusAuditIntent::Freshness),
                "expected Freshness for {q:?}"
            );
        }
    }

    #[test]
    fn hunter_classifies_known_events() {
        let cases = &[
            (
                "find me oil spills in Persian Gulf",
                HunterKind::OilSlick,
                Some("Persian Gulf"),
            ),
            (
                "where are algal blooms in Lake Erie",
                HunterKind::AlgalBloom,
                Some("Lake Erie"),
            ),
            (
                "show me deforestation in the Amazon",
                HunterKind::Deforestation,
                Some("the Amazon"),
            ),
            (
                "find wildfires across California",
                HunterKind::Wildfire,
                Some("California"),
            ),
            (
                "locate methane plumes in the Permian Basin",
                HunterKind::MethanePlume,
                Some("the Permian Basin"),
            ),
            (
                "hunt for landslides near Kathmandu",
                HunterKind::Landslide,
                Some("Kathmandu"),
            ),
            (
                "where is drought in the Sahel",
                HunterKind::Drought,
                Some("the Sahel"),
            ),
            (
                "find me crop stress in the Punjab",
                HunterKind::CropStress,
                Some("the Punjab"),
            ),
            (
                "show me sediment plumes in the Ganges delta",
                HunterKind::WaterTurbidity,
                Some("the Ganges delta"),
            ),
            (
                "where are heat islands in Phoenix",
                HunterKind::UrbanHeatIsland,
                Some("Phoenix"),
            ),
            (
                "find flooded fields in Bangladesh",
                HunterKind::Flood,
                Some("Bangladesh"),
            ),
        ];
        for (q, want_kind, want_anchor) in cases {
            let got = classify_hunter(q);
            assert!(got.is_some(), "expected hunter intent for {q:?}");
            let got = got.unwrap();
            assert_eq!(got.kind, *want_kind, "wrong kind for {q:?}");
            assert_eq!(
                got.region_anchor.as_deref(),
                *want_anchor,
                "wrong anchor for {q:?}"
            );
        }
    }

    #[test]
    fn hunter_handles_no_region_anchor() {
        let r = classify_hunter("find me wildfires").expect("should classify");
        assert_eq!(r.kind, HunterKind::Wildfire);
        assert_eq!(r.region_anchor, None);
    }

    #[test]
    fn hunter_rightmost_preposition_wins() {
        // "find oil spills in any port in Saudi Arabia" should anchor on
        // Saudi Arabia, not on "any port in Saudi Arabia".
        let r = classify_hunter("find oil spills in any port in Saudi Arabia")
            .expect("should classify");
        assert_eq!(r.region_anchor.as_deref(), Some("Saudi Arabia"));
    }

    #[test]
    fn hunter_strips_trailing_punctuation() {
        let r = classify_hunter("where are algal blooms in Lake Erie?").expect("should classify");
        assert_eq!(r.region_anchor.as_deref(), Some("Lake Erie"));
    }

    #[test]
    fn hunter_skips_place_anchored_topical_questions() {
        // These are place-anchored questions, NOT discovery — the
        // standard ask_inner path owns them.
        assert!(classify_hunter("ndvi for golden gate park").is_none());
        assert!(classify_hunter("elevation at Mt Fuji").is_none());
        assert!(classify_hunter("what's the flood risk in Mumbai").is_none());
        // "flood risk" must NOT be classified as Flood hunter intent —
        // the user is asking about risk at a place, not hunting events.
        // We carve it out via the explicit !contains("flood risk") gate.
        let r = classify_hunter("find me flood risk in Mumbai");
        // The verb "find" matches but there's no event-noun other than
        // "flood risk" which is excluded. So we should return None.
        assert!(r.is_none(), "flood risk should not trigger Flood hunt");
    }

    #[test]
    fn extract_region_anchor_handles_basic_cases() {
        assert_eq!(
            extract_region_anchor("show me X in Persian Gulf"),
            Some("Persian Gulf".to_string())
        );
        assert_eq!(
            extract_region_anchor("show me X over the Sahara"),
            Some("the Sahara".to_string())
        );
        assert_eq!(
            extract_region_anchor("show me X across the Indo-Gangetic plain"),
            Some("the Indo-Gangetic plain".to_string())
        );
        assert_eq!(extract_region_anchor("no preposition here"), None);
    }

    #[test]
    fn anomaly_keywords_no_longer_trigger_change_intent() {
        assert_eq!(
            classify_intent("what's anomalous about delhi's air quality"),
            None
        );
        assert_eq!(classify_intent("any anomaly in this cell"), None);
        // Confirmed change phrasings still match:
        assert_eq!(
            classify_intent("what changed here in the last year"),
            Some(AskIntent::Change)
        );
        assert_eq!(
            classify_intent("deforestation since 2020"),
            Some(AskIntent::Change)
        );
    }

    #[test]
    fn risk_phrasings_route_to_topic_not_hunter() {
        // Each of these previously matched hunter due to verb + event keyword.
        // After the fix, they return None so the fall-through path can route
        // them through topic_router for risk-composite algorithms.
        for q in [
            "find wildfire risk in oregon",
            "find heat risk in phoenix",
            "find drought risk in india",
            "find crop stress risk in punjab",
        ] {
            assert_eq!(
                classify_hunter(q),
                None,
                "expected None for risk question: {q}"
            );
        }
        // Sanity: the bare event keyword + verb still matches.
        assert!(classify_hunter("find wildfires in oregon").is_some());
    }

    #[test]
    fn region_anchored_event_without_verb_routes_to_hunter() {
        // Previously fell to CHANGE classifier.
        assert!(classify_hunter("deforestation in brazil").is_some());
        assert!(classify_hunter("methane plume over the permian").is_some());
        // But "deforestation since 2020" (temporal CHANGE) should still NOT match hunter.
        assert!(classify_hunter("deforestation since 2020").is_none());
        assert!(classify_hunter("deforestation in the last year").is_none());
    }

    #[test]
    fn coverage_questions_route_to_corpus_audit_without_your_prefix() {
        assert_eq!(
            classify_corpus_audit("ndvi coverage over the amazon"),
            Some(CorpusAuditIntent::RegionalDensity),
        );
        assert_eq!(
            classify_corpus_audit("how much data do you have"),
            Some(CorpusAuditIntent::GlobalCoverage),
        );
        // But a value question must still NOT match.
        assert_eq!(classify_corpus_audit("ndvi at south mumbai"), None);
        assert_eq!(classify_corpus_audit("what's the ndvi in brazil"), None);
    }

    #[test]
    fn place_anchored_questions_skip_corpus_audit() {
        // "ndvi over the amazon" mentions a region preposition but no
        // corpus word — must NOT match RegionalDensity, otherwise we'd
        // hijack a normal place-anchored question.
        assert_eq!(classify_corpus_audit("ndvi over the amazon"), None);
        assert_eq!(classify_corpus_audit("flood risk in delhi"), None);
        assert_eq!(classify_corpus_audit("ndvi near here"), None);
        assert_eq!(
            classify_corpus_audit("show me the elevation at Mt Fuji"),
            None
        );
    }
}
