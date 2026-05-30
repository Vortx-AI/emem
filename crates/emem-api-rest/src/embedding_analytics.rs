//! Real dispatcher arms for four documented embedding-region analytics
//! whose published formulas the scalar `evaluation`-AST evaluator cannot
//! express, because each operates on a **set of 128-D GeoTessera embedding
//! vectors** (a mean-pool, a cosine, or pairwise distances) rather than an
//! `f64 → f64` arithmetic tree over one cell's scalar:
//!
//!   1. `embedding_centroid@1` — `centroid = (1/N) Σ v_i`, plus the
//!      L2-normalised centroid + a stable content-addressed cid over it.
//!      The building block the other three compose.
//!
//!   2. `region_similarity@1` — `sim = cosine(centroid(A), centroid(B))`
//!      over two regions' mean GeoTessera centroids. "How alike are these
//!      two places?" in [-1, 1].
//!
//!   3. `embedding_diversity_score@1` —
//!      `diversity = (1/(N(N-1))) Σ_{i<j} (1 − cosine(v_i, v_j))`, the mean
//!      pairwise cosine distance over a region's cells. High = varied
//!      landscape (a determinantal-point-process / k-medoid diversity).
//!
//!   4. `embedding_neighborhood_consistency@1` —
//!      `consistency = (1/8) Σ_{n∈8 neighbours} cosine(centre, n)` over the
//!      8 immediate cell64 neighbours of a target cell. We also surface the
//!      complementary `region_outlier_score@1` reading
//!      (`outlier = 1 − consistency`): high = the cell looks anomalous
//!      versus its surroundings (Tobler's First Law).
//!
//! All four stay flagged `documentation_only` in the registry — the scalar
//! AST cannot reach a set of vectors. These endpoints are the honest
//! runnable surface. GeoTessera is the default 128-D embedding, CPU-fetched
//! (no GPU sidecar), so every one of these runs on a cold/GPU-less
//! responder. All cosines use [`cosine_finite`] so a NaN-masked dimension
//! never poisons the result. When too few cells materialize a vector the
//! response is a signed `inconclusive` carrying the observed count — never
//! a fabricated similarity/diversity number.

use std::time::Instant;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};

use emem_core::ErrorCode;
use emem_fact::{Fact, FactCid};
use emem_primitives::cbor_ops::{as_vec_f32, cosine_finite};
use emem_primitives::RecallReq;

use crate::{recall_with_auto_materialize, ApiError, AppState, EmemJson, ErrorBody};

const EMBED_BAND: &str = "geotessera";
/// Default cell cap for a region fan-out — same sane bound the
/// recall_polygon path uses. Surfaced honestly as `coverage_capped`.
const DEFAULT_MAX_CELLS: usize = 64;
/// A centroid over fewer than this many covered cells is too thin to be a
/// stable region descriptor; below it we degrade honestly.
const MIN_CELLS_FOR_CENTROID: usize = 1;
/// Pairwise diversity needs at least two distinct vectors.
const MIN_CELLS_FOR_DIVERSITY: usize = 2;

fn pubkey_b32(state: &AppState) -> String {
    data_encoding::BASE32_NOPAD
        .encode(&state.identity.pubkey.0)
        .to_lowercase()
}

fn bad_request(msg: impl Into<String>) -> ApiError {
    ApiError(
        StatusCode::BAD_REQUEST,
        ErrorBody {
            code: ErrorCode::InvalidArgument,
            message: msg.into(),
            details: None,
        },
    )
}

// ════════════════════════════════════════════════════════════════════
//  Pure vector math (unit-tested)
// ════════════════════════════════════════════════════════════════════

/// Mean-pool a set of equal-length vectors element-wise, skipping NaN
/// entries per dimension (a dimension is the mean of its finite values).
/// Returns `None` when the set is empty or every dimension is all-NaN.
pub(crate) fn mean_centroid(vectors: &[Vec<f32>]) -> Option<Vec<f32>> {
    let dim = vectors.iter().map(|v| v.len()).max()?;
    if dim == 0 {
        return None;
    }
    let mut sums = vec![0.0f64; dim];
    let mut counts = vec![0u32; dim];
    for v in vectors {
        for (i, &x) in v.iter().enumerate() {
            if x.is_finite() {
                sums[i] += x as f64;
                counts[i] += 1;
            }
        }
    }
    let centroid: Vec<f32> = sums
        .iter()
        .zip(&counts)
        .map(|(s, &c)| {
            if c > 0 {
                (s / c as f64) as f32
            } else {
                f32::NAN
            }
        })
        .collect();
    if centroid.iter().any(|x| x.is_finite()) {
        Some(centroid)
    } else {
        None
    }
}

/// L2-normalise a vector over its finite dimensions. NaN dims pass through
/// as NaN. Returns the input unchanged when the finite norm is zero.
pub(crate) fn l2_normalise(v: &[f32]) -> Vec<f32> {
    let norm = v
        .iter()
        .filter(|x| x.is_finite())
        .map(|x| (*x as f64) * (*x as f64))
        .sum::<f64>()
        .sqrt();
    if norm <= 0.0 || !norm.is_finite() {
        return v.to_vec();
    }
    v.iter()
        .map(|x| {
            if x.is_finite() {
                (*x as f64 / norm) as f32
            } else {
                *x
            }
        })
        .collect()
}

/// Mean pairwise cosine *distance* over a set of vectors:
/// `(1/(N(N-1))) Σ_{i<j} 2·(1 − cos(v_i, v_j))` — the registry's
/// `(1/(N(N-1))) Σ_{i<j}` normaliser counts ordered pairs, so the
/// unordered-pair sum is doubled to match. `None` when fewer than two
/// vectors share any finite cosine.
pub(crate) fn pairwise_diversity(vectors: &[Vec<f32>]) -> Option<f64> {
    let n = vectors.len();
    if n < 2 {
        return None;
    }
    let mut sum = 0.0f64;
    let mut pairs = 0u64;
    for i in 0..n {
        for j in (i + 1)..n {
            if let Some(c) = cosine_finite(&vectors[i], &vectors[j]) {
                sum += 1.0 - c as f64;
                pairs += 1;
            }
        }
    }
    if pairs == 0 {
        return None;
    }
    // Registry normaliser is N(N-1) (ordered pairs); we summed the
    // N(N-1)/2 unordered pairs, so the ordered-pair mean is 2·sum/(N(N-1)).
    Some((2.0 * sum) / (n as f64 * (n as f64 - 1.0)))
}

/// Mean cosine of a centre vector against a set of neighbour vectors:
/// `(1/k) Σ cos(centre, n_i)` over the neighbours that share a finite
/// cosine with the centre. Returns `(consistency, k_used)`; `None` when no
/// neighbour shares a finite cosine.
pub(crate) fn neighbour_consistency(
    centre: &[f32],
    neighbours: &[Vec<f32>],
) -> Option<(f64, usize)> {
    let mut sum = 0.0f64;
    let mut k = 0usize;
    for n in neighbours {
        if let Some(c) = cosine_finite(centre, n) {
            sum += c as f64;
            k += 1;
        }
    }
    if k == 0 {
        None
    } else {
        Some((sum / k as f64, k))
    }
}

// ════════════════════════════════════════════════════════════════════
//  Region → embedding vectors
// ════════════════════════════════════════════════════════════════════

/// A region spec: either an explicit list of cells, a bbox, or a place
/// name. Exactly one must be supplied (checked by the caller).
#[derive(Debug, Deserialize, Default, Clone)]
pub struct RegionSpec {
    #[serde(default)]
    pub place: Option<String>,
    #[serde(default)]
    pub polygon_bbox: Option<BboxSpec>,
    #[serde(default)]
    pub cells: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Clone, Copy)]
pub struct BboxSpec {
    pub min_lat: f64,
    pub max_lat: f64,
    pub min_lng: f64,
    pub max_lng: f64,
}

/// Resolve a region spec to a bounded list of cell64s + how the region was
/// resolved (for the receipt envelope). `cells` is taken verbatim (capped);
/// `polygon_bbox` and `place` fan out via `sample_cells_in_bbox`.
async fn cells_for_region(
    spec: &RegionSpec,
    max_cells: usize,
) -> Result<(Vec<String>, JsonValue, bool), ApiError> {
    if let Some(cells) = &spec.cells {
        let mut out: Vec<String> = Vec::new();
        for c in cells {
            // Validate each is a real cell64; reject junk loudly.
            if !emem_codec::is_cell64_shape(c) {
                return Err(bad_request(format!("`{c}` is not a valid cell64")));
            }
            out.push(c.clone());
        }
        let capped = out.len() > max_cells;
        out.truncate(max_cells);
        return Ok((out, json!({ "kind": "cells", "n": cells.len() }), capped));
    }
    if let Some(b) = spec.polygon_bbox {
        let bbox = (b.min_lat, b.max_lat, b.min_lng, b.max_lng);
        let cells = crate::sample_cells_in_bbox(bbox, max_cells);
        return Ok((
            cells,
            json!({ "kind": "polygon_bbox", "bbox": [b.min_lat, b.max_lat, b.min_lng, b.max_lng] }),
            false,
        ));
    }
    if let Some(place) = &spec.place {
        // Resolve the place to a bbox via the same locate path recall_polygon
        // uses, then sample cells inside it.
        let (bbox, via) = crate::resolve_place_bbox(place).await?;
        let cells = crate::sample_cells_in_bbox(bbox, max_cells);
        return Ok((
            cells,
            json!({ "kind": "place", "place": place, "bbox": [bbox.0, bbox.1, bbox.2, bbox.3], "via": via }),
            false,
        ));
    }
    Err(bad_request(
        "region requires one of: `cells`, `polygon_bbox`, or `place`",
    ))
}

/// Recall the GeoTessera embedding at every cell in `cells`, returning the
/// covered vectors + the fact cids. Misses (no vector at a cell) are simply
/// dropped — the count of covered cells is surfaced so the caller can tell
/// thin coverage from a wrong query.
async fn embeddings_for_cells(cells: &[String], s: &AppState) -> (Vec<Vec<f32>>, Vec<String>) {
    // Per-cell GeoTessera recalls are independent. GeoTessera reads ~640 B
    // via an HTTPS range read per cell, and cells in the same 0.1° tile
    // share one underlying tile fetch (TESSERA_HEADER_CACHE / cog single-
    // flight collapse it), so a bounded concurrent fan-out is a clean win
    // without flooding the bucket. We key each task by its cell index and
    // reassemble strictly in input order, so `vectors`/`cids` — and thus
    // every downstream centroid/diversity reduction and the receipt — are
    // byte-identical to the old serial loop.
    const EMBED_CONCURRENCY: usize = 16;
    // Per-cell result: the covered (vector, optional cid) pairs in the
    // same order the serial loop produced them.
    type CellEmbeds = Vec<(Vec<f32>, Option<String>)>;
    let mut per_cell: Vec<Option<CellEmbeds>> = vec![None; cells.len()];
    let mut set: tokio::task::JoinSet<(usize, CellEmbeds)> = tokio::task::JoinSet::new();
    let mut next = 0usize;
    loop {
        while set.len() < EMBED_CONCURRENCY && next < cells.len() {
            let idx = next;
            next += 1;
            let cell = cells[idx].clone();
            let st = s.clone();
            set.spawn(async move {
                let req = RecallReq {
                    cell,
                    bands: Some(vec![EMBED_BAND.to_string()]),
                    tslot: None,
                    ..Default::default()
                };
                let mut found: CellEmbeds = Vec::new();
                if let Ok((resp, _notes)) = recall_with_auto_materialize(&req, &st).await {
                    for (i, f) in resp.facts.iter().enumerate() {
                        if let Fact::Primary(p) = f {
                            if p.band == EMBED_BAND {
                                if let Some(v) = as_vec_f32(&p.value) {
                                    if v.iter().any(|x| x.is_finite()) {
                                        let cid = resp
                                            .receipt
                                            .fact_cids
                                            .get(i)
                                            .map(|c| c.as_str().to_string())
                                            .filter(|c| !c.is_empty());
                                        found.push((v, cid));
                                    }
                                }
                            }
                        }
                    }
                }
                (idx, found)
            });
        }
        let Some(joined) = set.join_next().await else {
            break;
        };
        if let Ok((idx, found)) = joined {
            per_cell[idx] = Some(found);
        }
    }

    let mut vectors = Vec::new();
    let mut cids = Vec::new();
    for found in per_cell.into_iter().flatten() {
        for (v, cid) in found {
            vectors.push(v);
            if let Some(c) = cid {
                cids.push(c);
            }
        }
    }
    (vectors, cids)
}

// ════════════════════════════════════════════════════════════════════
//  1. embedding_centroid
// ════════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize)]
pub struct EmbeddingCentroidReq {
    #[serde(flatten)]
    pub region: RegionSpec,
    #[serde(default)]
    pub max_cells: Option<usize>,
}

pub async fn embedding_centroid(
    req: EmbeddingCentroidReq,
    s: &AppState,
) -> Result<JsonValue, ApiError> {
    let started = Instant::now();
    let max_cells = req.max_cells.unwrap_or(DEFAULT_MAX_CELLS).clamp(1, 256);
    let (cells, region_env, capped) = cells_for_region(&req.region, max_cells).await?;
    let (vectors, cids) = embeddings_for_cells(&cells, s).await;

    let pubkey = pubkey_b32(s);
    let receipt = s.sign_receipt(
        "emem.embedding_centroid",
        cells.clone(),
        cids.iter().cloned().map(FactCid::new).collect(),
        false,
        started,
        None,
    );
    let citation = emem_core::algorithms::DEFAULT
        .lookup("embedding_centroid@1")
        .map(|a| a.citation.clone())
        .unwrap_or_default();

    if vectors.len() < MIN_CELLS_FOR_CENTROID {
        return Ok(inconclusive_region(
            "emem.embedding_centroid.v1",
            "embedding_centroid@1",
            region_env,
            cells.len(),
            vectors.len(),
            "centroid",
            "No GeoTessera embedding materialized at any sampled cell in this region (outside Tessera coverage, or all cells cold and un-fetchable). No centroid reported.",
            &pubkey,
            &receipt,
        ));
    }

    let centroid = mean_centroid(&vectors).ok_or_else(|| {
        bad_request("centroid is all-NaN despite covered vectors (should not happen)")
    })?;
    let normed = l2_normalise(&centroid);
    let centroid_cid = emem_codec::to_cid64(&emem_codec::vec64_to_cid(&centroid));

    Ok(json!({
        "schema": "emem.embedding_centroid.v1",
        "algorithm_key": "embedding_centroid@1",
        "region": region_env,
        "verdict": "computed",
        "available": true,
        "band": EMBED_BAND,
        "dim": centroid.len(),
        "n_cells_sampled": cells.len(),
        "n_cells_covered": vectors.len(),
        "coverage_capped": capped,
        "centroid": centroid,
        "centroid_l2_normalised": normed,
        "centroid_cid": centroid_cid,
        "formula": "centroid = (1/N) Σ v_i; centroid_norm = centroid/||centroid||₂",
        "citation": citation,
        "honest_note": "Mean-pooled GeoTessera centroid over the region's covered cells (NaN dims averaged over their finite contributors). centroid_cid is content-addressed over the raw mean for stable downstream citation.",
        "input_fact_cids": cids,
        "responder_pubkey_b32": pubkey,
        "receipt": receipt,
    }))
}

pub async fn post_embedding_centroid(
    State(s): State<AppState>,
    EmemJson(req): EmemJson<EmbeddingCentroidReq>,
) -> Result<Json<JsonValue>, ApiError> {
    Ok(Json(embedding_centroid(req, &s).await?))
}

// ════════════════════════════════════════════════════════════════════
//  2. region_similarity
// ════════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize)]
pub struct RegionSimilarityReq {
    pub region_a: RegionSpec,
    pub region_b: RegionSpec,
    #[serde(default)]
    pub max_cells: Option<usize>,
}

pub async fn region_similarity(
    req: RegionSimilarityReq,
    s: &AppState,
) -> Result<JsonValue, ApiError> {
    let started = Instant::now();
    let max_cells = req.max_cells.unwrap_or(DEFAULT_MAX_CELLS).clamp(1, 256);
    let (cells_a, env_a, cap_a) = cells_for_region(&req.region_a, max_cells).await?;
    let (cells_b, env_b, cap_b) = cells_for_region(&req.region_b, max_cells).await?;
    let (vec_a, cids_a) = embeddings_for_cells(&cells_a, s).await;
    let (vec_b, cids_b) = embeddings_for_cells(&cells_b, s).await;

    let mut all_cells = cells_a.clone();
    all_cells.extend(cells_b.clone());
    let mut all_cids = cids_a.clone();
    all_cids.extend(cids_b.clone());

    let pubkey = pubkey_b32(s);
    let receipt = s.sign_receipt(
        "emem.region_similarity",
        all_cells,
        all_cids.iter().cloned().map(FactCid::new).collect(),
        false,
        started,
        None,
    );
    let citation = emem_core::algorithms::DEFAULT
        .lookup("region_similarity@1")
        .map(|a| a.citation.clone())
        .unwrap_or_default();

    let ca = mean_centroid(&vec_a);
    let cb = mean_centroid(&vec_b);
    let sim = match (&ca, &cb) {
        (Some(a), Some(b)) => cosine_finite(a, b).map(|c| c as f64),
        _ => None,
    };

    match sim {
        Some(sim) => Ok(json!({
            "schema": "emem.region_similarity.v1",
            "algorithm_key": "region_similarity@1",
            "verdict": "computed",
            "available": true,
            "similarity": sim,
            "region_a": env_a,
            "region_b": env_b,
            "n_cells_covered_a": vec_a.len(),
            "n_cells_covered_b": vec_b.len(),
            "coverage_capped": cap_a || cap_b,
            "formula": "sim = cosine(centroid(region_a), centroid(region_b))",
            "citation": citation,
            "honest_note": "Cosine of the two mean-pooled GeoTessera centroids; +1 = identical landscapes, 0 = unrelated, negative = opposed. Coverage counts surface how many cells actually carried a vector.",
            "input_fact_cids": all_cids,
            "responder_pubkey_b32": pubkey,
            "receipt": receipt,
        })),
        None => Ok(json!({
            "schema": "emem.region_similarity.v1",
            "algorithm_key": "region_similarity@1",
            "verdict": "inconclusive",
            "available": false,
            "similarity": JsonValue::Null,
            "region_a": env_a,
            "region_b": env_b,
            "n_cells_covered_a": vec_a.len(),
            "n_cells_covered_b": vec_b.len(),
            "honest_note": "At least one region had no GeoTessera embedding at any sampled cell (outside Tessera coverage / cold), or the two centroids share no finite dimensions. No similarity reported.",
            "citation": citation,
            "input_fact_cids": all_cids,
            "responder_pubkey_b32": pubkey,
            "receipt": receipt,
        })),
    }
}

pub async fn post_region_similarity(
    State(s): State<AppState>,
    EmemJson(req): EmemJson<RegionSimilarityReq>,
) -> Result<Json<JsonValue>, ApiError> {
    Ok(Json(region_similarity(req, &s).await?))
}

// ════════════════════════════════════════════════════════════════════
//  3. embedding_diversity
// ════════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize)]
pub struct EmbeddingDiversityReq {
    #[serde(flatten)]
    pub region: RegionSpec,
    #[serde(default)]
    pub max_cells: Option<usize>,
}

pub async fn embedding_diversity(
    req: EmbeddingDiversityReq,
    s: &AppState,
) -> Result<JsonValue, ApiError> {
    let started = Instant::now();
    let max_cells = req.max_cells.unwrap_or(DEFAULT_MAX_CELLS).clamp(1, 256);
    let (cells, region_env, capped) = cells_for_region(&req.region, max_cells).await?;
    let (vectors, cids) = embeddings_for_cells(&cells, s).await;

    let pubkey = pubkey_b32(s);
    let receipt = s.sign_receipt(
        "emem.embedding_diversity",
        cells.clone(),
        cids.iter().cloned().map(FactCid::new).collect(),
        false,
        started,
        None,
    );
    let citation = emem_core::algorithms::DEFAULT
        .lookup("embedding_diversity_score@1")
        .map(|a| a.citation.clone())
        .unwrap_or_default();

    if vectors.len() < MIN_CELLS_FOR_DIVERSITY {
        return Ok(inconclusive_region(
            "emem.embedding_diversity.v1",
            "embedding_diversity_score@1",
            region_env,
            cells.len(),
            vectors.len(),
            "diversity",
            "Diversity needs at least two cells carrying a GeoTessera embedding; fewer materialized in this region. No diversity reported.",
            &pubkey,
            &receipt,
        ));
    }

    match pairwise_diversity(&vectors) {
        Some(div) => Ok(json!({
            "schema": "emem.embedding_diversity.v1",
            "algorithm_key": "embedding_diversity_score@1",
            "region": region_env,
            "verdict": "computed",
            "available": true,
            "diversity": div,
            "n_cells_sampled": cells.len(),
            "n_cells_covered": vectors.len(),
            "coverage_capped": capped,
            "formula": "diversity = (1/(N(N-1))) Σ_{i<j} (1 − cosine(v_i, v_j))",
            "citation": citation,
            "honest_note": "Mean pairwise cosine distance over the region's covered GeoTessera vectors. 0 = a perfectly uniform landscape, higher = more heterogeneous land cover.",
            "input_fact_cids": cids,
            "responder_pubkey_b32": pubkey,
            "receipt": receipt,
        })),
        None => Ok(inconclusive_region(
            "emem.embedding_diversity.v1",
            "embedding_diversity_score@1",
            region_env,
            cells.len(),
            vectors.len(),
            "diversity",
            "No pair of covered vectors shared a finite cosine. No diversity reported.",
            &pubkey,
            &receipt,
        )),
    }
}

pub async fn post_embedding_diversity(
    State(s): State<AppState>,
    EmemJson(req): EmemJson<EmbeddingDiversityReq>,
) -> Result<Json<JsonValue>, ApiError> {
    Ok(Json(embedding_diversity(req, &s).await?))
}

// ════════════════════════════════════════════════════════════════════
//  4. neighborhood_consistency / region_outlier
// ════════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize)]
pub struct NeighborhoodConsistencyReq {
    /// Target cell64 OR a place name (alias `place`/`q`). Omit when
    /// supplying `lat`+`lng` instead.
    #[serde(default, alias = "place", alias = "q")]
    pub cell: Option<String>,
    /// Explicit coordinates, used when `cell`/`place` is absent.
    #[serde(default)]
    pub lat: Option<f64>,
    #[serde(default)]
    pub lng: Option<f64>,
}

pub async fn neighborhood_consistency(
    req: NeighborhoodConsistencyReq,
    s: &AppState,
) -> Result<JsonValue, ApiError> {
    let started = Instant::now();
    let (cell, resolved) = crate::resolve_cell_input(req.cell.as_deref(), req.lat, req.lng).await?;
    // Reuse the terrain neighbourhood generator for the 8 immediate spatial
    // neighbours — step 1, matching /v1/locate.neighborhood_cells, which is
    // exactly the set the registry formula references.
    let (centre_cell, neighbours, _w, _h) = crate::terrain::neighbourhood(&cell, 1)?;

    // Centre embedding.
    let (centre_vecs, mut cids) = embeddings_for_cells(std::slice::from_ref(&centre_cell), s).await;
    let centre = centre_vecs.into_iter().next();

    // Neighbour embeddings.
    let neighbour_cells: Vec<String> = neighbours.iter().map(|(_d, c)| c.clone()).collect();
    let (neighbour_vecs, n_cids) = embeddings_for_cells(&neighbour_cells, s).await;
    cids.extend(n_cids);

    let pubkey = pubkey_b32(s);
    let receipt = s.sign_receipt(
        "emem.neighborhood_consistency",
        vec![centre_cell.clone()],
        cids.iter().cloned().map(FactCid::new).collect(),
        false,
        started,
        None,
    );
    let citation = emem_core::algorithms::DEFAULT
        .lookup("embedding_neighborhood_consistency@1")
        .map(|a| a.citation.clone())
        .unwrap_or_default();
    let resolved_env = crate::resolved_envelope(vec![("cell".into(), resolved)]);

    let result = centre
        .as_ref()
        .and_then(|c| neighbour_consistency(c, &neighbour_vecs));

    match result {
        Some((consistency, k)) => Ok(json!({
            "schema": "emem.neighborhood_consistency.v1",
            "algorithm_key": "embedding_neighborhood_consistency@1",
            "cell": centre_cell,
            "resolved_from": resolved_env,
            "verdict": "computed",
            "available": true,
            "consistency": consistency,
            "outlier_score": 1.0 - consistency,
            "n_neighbours_used": k,
            "formula": "consistency = (1/k) Σ cosine(centre, neighbour_i); outlier_score = 1 − consistency",
            "citation": citation,
            "honest_note": "Mean cosine of the cell's GeoTessera embedding against its 8 immediate cell64 neighbours (Tobler's First Law). High consistency = the cell blends into its surroundings; high outlier_score = it stands out (an edge, a new clearing, a built patch in farmland).",
            "input_fact_cids": cids,
            "responder_pubkey_b32": pubkey,
            "receipt": receipt,
        })),
        None => Ok(json!({
            "schema": "emem.neighborhood_consistency.v1",
            "algorithm_key": "embedding_neighborhood_consistency@1",
            "cell": centre_cell,
            "resolved_from": resolved_env,
            "verdict": "inconclusive",
            "available": false,
            "consistency": JsonValue::Null,
            "outlier_score": JsonValue::Null,
            "honest_note": "Either the centre cell or all 8 neighbours lacked a GeoTessera embedding (outside Tessera coverage / cold). No consistency reported.",
            "citation": citation,
            "input_fact_cids": cids,
            "responder_pubkey_b32": pubkey,
            "receipt": receipt,
        })),
    }
}

pub async fn post_neighborhood_consistency(
    State(s): State<AppState>,
    EmemJson(req): EmemJson<NeighborhoodConsistencyReq>,
) -> Result<Json<JsonValue>, ApiError> {
    Ok(Json(neighborhood_consistency(req, &s).await?))
}

// ── shared inconclusive shape for the region endpoints ────────────────
#[allow(clippy::too_many_arguments)]
fn inconclusive_region(
    schema: &str,
    key: &str,
    region_env: JsonValue,
    n_sampled: usize,
    n_covered: usize,
    value_field: &str,
    note: &str,
    pubkey: &str,
    receipt: &impl serde::Serialize,
) -> JsonValue {
    json!({
        "schema": schema,
        "algorithm_key": key,
        "region": region_env,
        "verdict": "inconclusive",
        "available": false,
        value_field: JsonValue::Null,
        "n_cells_sampled": n_sampled,
        "n_cells_covered": n_covered,
        "honest_note": note,
        "citation": emem_core::algorithms::DEFAULT.lookup(key).map(|a| a.citation.clone()).unwrap_or_default(),
        "responder_pubkey_b32": pubkey,
        "receipt": serde_json::to_value(receipt).unwrap_or(JsonValue::Null),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(seed: f32, dim: usize) -> Vec<f32> {
        (0..dim)
            .map(|i| ((i as f32) * 0.013 + seed).sin())
            .collect()
    }

    /// Mean centroid of identical vectors is that vector; cosine of a
    /// region with itself is 1.0.
    #[test]
    fn identical_region_similarity_is_one() {
        let v = unit(0.2, 128);
        let region = vec![v.clone(), v.clone(), v.clone()];
        let ca = mean_centroid(&region).unwrap();
        let cb = mean_centroid(&region).unwrap();
        let sim = cosine_finite(&ca, &cb).unwrap();
        assert!((sim - 1.0).abs() < 1e-5, "got {sim}");
    }

    /// A uniform region (all cells the same vector) has zero diversity.
    #[test]
    fn uniform_region_has_zero_diversity() {
        let v = unit(0.7, 64);
        let region = vec![v.clone(); 5];
        let d = pairwise_diversity(&region).unwrap();
        assert!(
            d.abs() < 1e-5,
            "uniform region diversity must be ~0, got {d}"
        );
    }

    /// Two orthogonal halves → cosine 0 → pairwise distance 1 → diversity 1
    /// for a 2-vector set.
    #[test]
    fn orthogonal_pair_diversity_is_one() {
        let mut a = vec![0.0f32; 8];
        let mut b = vec![0.0f32; 8];
        a[0] = 1.0;
        b[1] = 1.0;
        let d = pairwise_diversity(&[a, b]).unwrap();
        assert!((d - 1.0).abs() < 1e-6, "got {d}");
    }

    /// mean_centroid averages a NaN dim over only its finite contributors.
    #[test]
    fn mean_centroid_skips_nan_dims() {
        let a = vec![2.0f32, f32::NAN, 4.0];
        let b = vec![4.0f32, 8.0, 6.0];
        let c = mean_centroid(&[a, b]).unwrap();
        assert!((c[0] - 3.0).abs() < 1e-6); // (2+4)/2
        assert!((c[1] - 8.0).abs() < 1e-6); // only b finite → 8
        assert!((c[2] - 5.0).abs() < 1e-6); // (4+6)/2
    }

    /// l2_normalise yields a unit vector over finite dims.
    #[test]
    fn l2_normalise_is_unit() {
        let v = vec![3.0f32, 4.0];
        let n = l2_normalise(&v);
        let norm = (n[0] * n[0] + n[1] * n[1]).sqrt();
        assert!((norm - 1.0).abs() < 1e-6, "got {norm}");
    }

    /// Neighbourhood consistency: a centre identical to all neighbours →
    /// consistency 1, outlier 0. A centre orthogonal to all → consistency
    /// 0, outlier 1.
    #[test]
    fn neighbour_consistency_extremes() {
        let centre = unit(0.3, 32);
        let same = vec![centre.clone(); 8];
        let (cons, k) = neighbour_consistency(&centre, &same).unwrap();
        assert_eq!(k, 8);
        assert!(
            (cons - 1.0).abs() < 1e-5,
            "identical neighbours → 1, got {cons}"
        );

        let mut centre2 = vec![0.0f32; 8];
        centre2[0] = 1.0;
        let mut ortho = vec![0.0f32; 8];
        ortho[1] = 1.0;
        let (cons2, _k) = neighbour_consistency(&centre2, &[ortho]).unwrap();
        assert!(cons2.abs() < 1e-6, "orthogonal → 0, got {cons2}");
    }

    /// Pairwise diversity refuses a single-vector set (no pairs).
    #[test]
    fn diversity_single_vector_is_none() {
        assert!(pairwise_diversity(&[unit(0.1, 16)]).is_none());
    }
}
