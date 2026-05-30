//! Real dispatcher arms for the DEM terrain triad — three documented
//! algorithms whose published formulas the scalar `evaluation`-AST
//! evaluator cannot express, because each needs a **3×3 cell neighbourhood**
//! of elevation samples rather than a single `f64 → f64` arithmetic tree
//! over one cell's scalar:
//!
//!   1. `slope_from_dem_neighborhood@1` — Horn (1981) 8-direction slope.
//!      `dz/dx = ((Z_E+2·Z_NE+Z_SE) − (Z_W+2·Z_NW+Z_SW)) / (8·cell_w_m)`,
//!      `dz/dy = ((Z_N+2·Z_NE+Z_NW) − (Z_S+2·Z_SE+Z_SW)) / (8·cell_h_m)`,
//!      `slope_deg = atan(sqrt((dz/dx)²+(dz/dy)²))·180/π`. The east-west
//!      ground spacing narrows by cos(lat); the cell64 grid is uniform in
//!      lat/lng degrees so we use the true per-axis ground pitch.
//!
//!   2. `ruggedness_index@1` — Riley, DeGloria & Elliot (1999) TRI =
//!      `sqrt(Σ_{i=1..8} (Z_centre − Z_i)²)` over the 8 neighbours.
//!
//!   3. `topo_position_index@1` — Weiss (2001) TPI = `Z_centre −
//!      mean(Z_neighbours_8)`. Positive = ridge/crest, negative = valley.
//!
//! All three are computed from the **same** 3×3 elevation neighbourhood
//! (`copdem30m.elevation_mean` at the centre cell + its 8 immediate cell64
//! neighbours), so `/v1/terrain` materializes the neighbourhood once and
//! returns `{slope, tri, tpi}` together. Each stays flagged
//! `documentation_only` in the registry — the scalar AST sees one cell and
//! cannot reach the neighbours. These endpoints are the honest runnable
//! surface.
//!
//! Honest degradation: Copernicus DEM is bathymetry-free, so an ocean cell
//! (or any cell where fewer than the required elevation samples
//! materialize) yields a signed `inconclusive` carrying the observed count
//! — never a fabricated slope from a partial stencil. TPI degrades softly
//! (it only needs ≥1 neighbour + the centre), TRI/slope need the full ring.

use std::time::Instant;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};

use emem_core::ErrorCode;
use emem_fact::{Fact, FactCid};
use emem_primitives::cbor_ops::as_f64;
use emem_primitives::RecallReq;

use crate::{recall_with_auto_materialize, ApiError, AppState, EmemJson, ErrorBody};

const DEM_BAND: &str = "copdem30m.elevation_mean";

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
//  Geometry: the 3×3 cell64 neighbourhood
// ════════════════════════════════════════════════════════════════════

/// The eight compass directions of the Horn 3×3 stencil, in the order the
/// elevation vector is indexed below: NW, N, NE, W, E, SW, S, SE.
/// (The centre is held separately.)
pub(crate) const NEIGHBOUR_ORDER: [&str; 8] = ["NW", "N", "NE", "W", "E", "SW", "S", "SE"];

/// `(centre_cell64, [(dir, cell64); 8], cell_w_m, cell_h_m)` — the output
/// of [`neighbourhood`]. `cell_w_m`/`cell_h_m` are the true ground spacing
/// of ONE STEP at the centre latitude (Horn needs ground metres).
pub(crate) type Neighbourhood = (String, Vec<(&'static str, String)>, f64, f64);

/// Derive the centre + 8 neighbour cell64s by perturbing the centre's
/// decoded lat/lng by `step_cells` cell pitches on each axis. The cell64
/// grid is uniform in lat/lng *degrees*, so one step = the per-axis degree
/// quantum (`180/2^21` lat, `360/2^22` lng); `step_cells` multiplies it.
///
/// `step_cells` matters: at `step_cells = 1` the 8 neighbours are the
/// immediate ~9.55 m buckets (used by the embedding-neighbourhood analytic,
/// where adjacency is the point). For DEM terrain, Copernicus DEM is ~30 m
/// native, so one cell step lands inside the SAME source pixel as the
/// centre and the slope/TRI/TPI collapse to zero — a real artefact of
/// sampling below the data's resolution. Stepping `step_cells ≈ 3`
/// (~28.7 m) straddles distinct DEM pixels so the Horn finite difference
/// sees a genuine gradient. `cell_w_m`/`cell_h_m` are returned for the
/// chosen step so the slope denominator matches the stencil spacing.
pub(crate) fn neighbourhood(
    centre_cell64: &str,
    step_cells: u32,
) -> Result<Neighbourhood, ApiError> {
    let step = step_cells.max(1) as f64;
    let ll = emem_codec::latlng_from_cell64(centre_cell64)
        .map_err(|e| bad_request(format!("cannot decode centre cell64: {e}")))?;
    // One full bucket width in degrees per axis (see codec geo.rs), ×step.
    let dlat = step * 180.0 / ((1u64 << 21) - 1) as f64; // lat step
    let dlng = step * 360.0 / ((1u64 << 22) - 1) as f64; // lng step
                                                         // True ground spacing of one STEP at this latitude.
    let m_per_deg = 111_319.491_f64;
    let cell_h_m = dlat * m_per_deg; // north-south
    let cell_w_m = dlng * m_per_deg * ll.lat_deg.to_radians().cos().abs().max(1e-9); // east-west, cos(lat)

    // (d_lat_sign, d_lng_sign) per compass direction. North = +lat.
    let offsets: [(&'static str, f64, f64); 8] = [
        ("NW", 1.0, -1.0),
        ("N", 1.0, 0.0),
        ("NE", 1.0, 1.0),
        ("W", 0.0, -1.0),
        ("E", 0.0, 1.0),
        ("SW", -1.0, -1.0),
        ("S", -1.0, 0.0),
        ("SE", -1.0, 1.0),
    ];
    let mut neighbours = Vec::with_capacity(8);
    for (dir, slat, slng) in offsets {
        let nlat = (ll.lat_deg + slat * dlat).clamp(-90.0, 90.0);
        let nlng = ll.lng_deg + slng * dlng;
        let c64 = emem_codec::cell64_from_latlng(nlat, nlng);
        neighbours.push((dir, c64));
    }
    // Canonicalize the centre to the bucket-centre cell64 so the centre we
    // recall is exactly the bucket the caller named.
    let centre = emem_codec::cell64_from_latlng(ll.lat_deg, ll.lng_deg);
    Ok((centre, neighbours, cell_w_m, cell_h_m))
}

/// Default neighbour step for DEM terrain: 3 cells ≈ 28.7 m, matching the
/// ~30 m Copernicus DEM native resolution so the Horn stencil straddles
/// distinct source pixels instead of collapsing inside one.
pub(crate) const DEM_STEP_CELLS: u32 = 3;

// ════════════════════════════════════════════════════════════════════
//  Pure terrain math (unit-tested)
// ════════════════════════════════════════════════════════════════════

/// Horn (1981) 8-direction slope in degrees from a 3×3 elevation stencil.
/// `n` maps compass → elevation by [`NEIGHBOUR_ORDER`]; `cell_w_m` is the
/// east-west ground spacing, `cell_h_m` the north-south. Centre elevation
/// is unused by Horn (the kernel is a finite difference across the ring).
#[allow(clippy::too_many_arguments)]
pub(crate) fn horn_slope_deg(
    z_nw: f64,
    z_n: f64,
    z_ne: f64,
    z_w: f64,
    z_e: f64,
    z_sw: f64,
    z_s: f64,
    z_se: f64,
    cell_w_m: f64,
    cell_h_m: f64,
) -> f64 {
    let dz_dx = ((z_e + 2.0 * z_ne + z_se) - (z_w + 2.0 * z_nw + z_sw)) / (8.0 * cell_w_m);
    let dz_dy = ((z_n + 2.0 * z_ne + z_nw) - (z_s + 2.0 * z_se + z_sw)) / (8.0 * cell_h_m);
    (dz_dx * dz_dx + dz_dy * dz_dy).sqrt().atan().to_degrees()
}

/// Riley (1999) Terrain Ruggedness Index: `sqrt(Σ (Z_centre − Z_i)²)`
/// over the 8 neighbours.
pub(crate) fn tri(z_centre: f64, neighbours: &[f64]) -> f64 {
    neighbours
        .iter()
        .map(|z| (z_centre - z).powi(2))
        .sum::<f64>()
        .sqrt()
}

/// Weiss (2001) Topographic Position Index: `Z_centre − mean(neighbours)`.
pub(crate) fn tpi(z_centre: f64, neighbours: &[f64]) -> f64 {
    let m = neighbours.iter().sum::<f64>() / neighbours.len() as f64;
    z_centre - m
}

/// Weiss landform class from a TPI value, using ±1 m thresholds as a
/// coarse ridge/valley/flat split (the published landform taxonomy keys
/// off TPI sign at multiple scales; this single-scale sign+magnitude
/// split is the documented first cut).
pub(crate) fn tpi_class(tpi_m: f64) -> &'static str {
    if tpi_m > 1.0 {
        "ridge_or_crest"
    } else if tpi_m < -1.0 {
        "valley_or_depression"
    } else {
        "flat_or_midslope"
    }
}

// ════════════════════════════════════════════════════════════════════
//  /v1/terrain — slope + TRI + TPI from one 3×3 neighbourhood
// ════════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize)]
pub struct TerrainReq {
    /// cell64 OR a place name (alias `place`/`q`). The 8 neighbour cell64s
    /// are derived by perturbing the decoded lat/lng `step_cells` pitches
    /// per axis. Omit when supplying `lat`+`lng` instead.
    #[serde(default, alias = "place", alias = "q")]
    pub cell: Option<String>,
    /// Explicit coordinates, used when `cell`/`place` is absent.
    #[serde(default)]
    pub lat: Option<f64>,
    #[serde(default)]
    pub lng: Option<f64>,
    /// Stencil step in cell64 pitches (default 3 ≈ 28.7 m, matching the
    /// ~30 m Copernicus DEM native resolution so the Horn finite difference
    /// straddles distinct source pixels). Raise it to measure slope at a
    /// coarser scale (slope is scale-dependent); 1 samples below the DEM
    /// resolution and will read flat inside one source pixel.
    #[serde(default)]
    pub step_cells: Option<u32>,
}

/// Recall elevation at one cell. Returns `(Some(z), Some(cid))` when a
/// finite `copdem30m.elevation_mean` Primary materializes, else `(None, _)`.
async fn recall_elevation(cell: &str, s: &AppState) -> (Option<f64>, Option<String>) {
    let req = RecallReq {
        cell: cell.to_string(),
        bands: Some(vec![DEM_BAND.to_string()]),
        tslot: None,
        ..Default::default()
    };
    match recall_with_auto_materialize(&req, s).await {
        Ok((resp, _notes)) => {
            for (idx, f) in resp.facts.iter().enumerate() {
                if let Fact::Primary(p) = f {
                    if p.band == DEM_BAND {
                        if let Some(v) = as_f64(&p.value).filter(|x| x.is_finite()) {
                            let cid = resp
                                .receipt
                                .fact_cids
                                .get(idx)
                                .map(|c| c.as_str().to_string())
                                .filter(|c| !c.is_empty());
                            return (Some(v), cid);
                        }
                    }
                }
            }
            (None, None)
        }
        Err(_) => (None, None),
    }
}

pub async fn terrain(req: TerrainReq, s: &AppState) -> Result<JsonValue, ApiError> {
    let started = Instant::now();
    let (cell, resolved) = crate::resolve_cell_input(req.cell.as_deref(), req.lat, req.lng).await?;
    let step_cells = req.step_cells.unwrap_or(DEM_STEP_CELLS).max(1);
    let (centre_cell, neighbours, cell_w_m, cell_h_m) = neighbourhood(&cell, step_cells)?;

    // Recall the centre + 8 neighbours. Each is one elevation recall.
    let (z_centre_opt, centre_cid) = recall_elevation(&centre_cell, s).await;
    let mut cids: Vec<String> = Vec::new();
    if let Some(c) = centre_cid {
        cids.push(c);
    }
    // dir → elevation, NaN-masking misses.
    let mut by_dir: std::collections::BTreeMap<&'static str, f64> = Default::default();
    let mut neighbour_cells: Vec<JsonValue> = Vec::new();
    for (dir, c64) in &neighbours {
        let (z, cid) = recall_elevation(c64, s).await;
        if let Some(c) = cid {
            cids.push(c);
        }
        if let Some(zz) = z {
            by_dir.insert(dir, zz);
        }
        neighbour_cells.push(json!({ "dir": dir, "cell": c64, "elevation_m": z }));
    }

    let resolved_env = crate::resolved_envelope(vec![("cell".into(), resolved)]);
    let pubkey = pubkey_b32(s);
    let receipt = s.sign_receipt(
        "emem.terrain",
        vec![centre_cell.clone()],
        cids.iter().cloned().map(FactCid::new).collect(),
        false,
        started,
        None,
    );
    let cite = |key: &str| {
        emem_core::algorithms::DEFAULT
            .lookup(key)
            .map(|a| a.citation.clone())
            .unwrap_or_default()
    };

    let n_neighbours = by_dir.len();
    let neighbour_vals: Vec<f64> = by_dir.values().copied().collect();

    // ── TPI: needs the centre + at least one neighbour. ──────────────
    let tpi_block = match (z_centre_opt, n_neighbours) {
        (Some(zc), n) if n >= 1 => {
            let v = tpi(zc, &neighbour_vals);
            json!({
                "verdict": "computed",
                "available": true,
                "tpi_m": v,
                "landform_class": tpi_class(v),
                "n_neighbours_used": n,
                "formula": "TPI = Z_centre − mean(Z_neighbours)",
                "citation": cite("topo_position_index@1"),
            })
        }
        _ => json!({
            "verdict": "inconclusive",
            "available": false,
            "tpi_m": JsonValue::Null,
            "honest_note": "TPI needs the centre elevation plus ≥1 neighbour; Copernicus DEM did not materialize enough of the stencil here (e.g. an ocean cell — Cop-DEM is bathymetry-free). No TPI reported.",
            "citation": cite("topo_position_index@1"),
        }),
    };

    // ── TRI + Horn slope: need the FULL 8-neighbour ring + centre for TRI,
    //    and all 8 neighbours for slope. ──────────────────────────────
    let full_ring = NEIGHBOUR_ORDER.iter().all(|d| by_dir.contains_key(d));
    let (tri_block, slope_block) = if full_ring {
        let g = |d: &str| *by_dir.get(d).expect("full ring checked");
        let (z_nw, z_n, z_ne, z_w, z_e, z_sw, z_s, z_se) = (
            g("NW"),
            g("N"),
            g("NE"),
            g("W"),
            g("E"),
            g("SW"),
            g("S"),
            g("SE"),
        );
        let slope = horn_slope_deg(
            z_nw, z_n, z_ne, z_w, z_e, z_sw, z_s, z_se, cell_w_m, cell_h_m,
        );
        let slope_block = json!({
            "verdict": "computed",
            "available": true,
            "slope_deg": slope,
            "slope_percent": slope.to_radians().tan() * 100.0,
            "cell_w_m": cell_w_m,
            "cell_h_m": cell_h_m,
            "formula": "Horn 1981: dz/dx,dz/dy weighted 3×3 finite difference; slope = atan(sqrt(dz/dx²+dz/dy²))·180/π",
            "citation": cite("slope_from_dem_neighborhood@1"),
        });
        // TRI requires the centre too (Z_centre − Z_i).
        let tri_block = match z_centre_opt {
            Some(zc) => {
                let v = tri(zc, &neighbour_vals);
                json!({
                    "verdict": "computed",
                    "available": true,
                    "tri_m": v,
                    "n_neighbours_used": neighbour_vals.len(),
                    "formula": "TRI = sqrt(Σ_{i=1..8} (Z_centre − Z_i)²)",
                    "citation": cite("ruggedness_index@1"),
                })
            }
            None => json!({
                "verdict": "inconclusive",
                "available": false,
                "tri_m": JsonValue::Null,
                "honest_note": "TRI needs the centre elevation; it did not materialize here. No TRI reported.",
                "citation": cite("ruggedness_index@1"),
            }),
        };
        (tri_block, slope_block)
    } else {
        let absent: Vec<&str> = NEIGHBOUR_ORDER
            .iter()
            .copied()
            .filter(|d| !by_dir.contains_key(d))
            .collect();
        let note = format!(
            "Horn slope and TRI need all 8 neighbours; {}/8 materialized (missing: {}). Copernicus DEM is bathymetry-free, so a coastal/ocean cell loses part of the ring. No slope/TRI reported — a partial-stencil value would be fabricated.",
            n_neighbours,
            absent.join(", ")
        );
        let inconclusive = |key: &str, field: &str| {
            json!({
                "verdict": "inconclusive",
                "available": false,
                field: JsonValue::Null,
                "honest_note": note,
                "citation": cite(key),
            })
        };
        (
            inconclusive("ruggedness_index@1", "tri_m"),
            inconclusive("slope_from_dem_neighborhood@1", "slope_deg"),
        )
    };

    Ok(json!({
        "schema": "emem.terrain.v1",
        "cell": centre_cell,
        "resolved_from": resolved_env,
        "centre_elevation_m": z_centre_opt,
        "n_neighbours_materialized": n_neighbours,
        "step_cells": step_cells,
        "step_w_m": cell_w_m,
        "step_h_m": cell_h_m,
        "slope": slope_block,
        "ruggedness": tri_block,
        "topo_position": tpi_block,
        "neighbour_cells": neighbour_cells,
        "honest_note": "All three indices share one 3×3 copdem30m.elevation_mean neighbourhood sampled at step_cells pitches (default 3 ≈ 28.7 m, matching the ~30 m Cop-DEM native resolution; step_cells=1 would sample below it and read flat inside one source pixel). Slope/TRI require the full 8-neighbour ring; TPI degrades gracefully to ≥1 neighbour. Cop-DEM is bathymetry-free — ocean cells return inconclusive, never a fabricated terrain value.",
        "responder_pubkey_b32": pubkey,
        "input_fact_cids": cids,
        "receipt": receipt,
    }))
}

pub async fn post_terrain(
    State(s): State<AppState>,
    EmemJson(req): EmemJson<TerrainReq>,
) -> Result<Json<JsonValue>, ApiError> {
    Ok(Json(terrain(req, &s).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The request accepts `cell`, the `place`/`q` alias, and a bare
    /// `lat`+`lng` pair — so a caller isn't forced to pre-resolve a cell64.
    #[test]
    fn terrain_req_accepts_cell_place_and_latlng() {
        let by_cell: TerrainReq =
            serde_json::from_str(r#"{"cell":"defi.zb60b.fozI.piwo"}"#).unwrap();
        assert_eq!(by_cell.cell.as_deref(), Some("defi.zb60b.fozI.piwo"));
        let by_place: TerrainReq = serde_json::from_str(r#"{"place":"Matterhorn"}"#).unwrap();
        assert_eq!(by_place.cell.as_deref(), Some("Matterhorn"));
        let by_latlng: TerrainReq = serde_json::from_str(r#"{"lat":45.97,"lng":7.66}"#).unwrap();
        assert!(by_latlng.cell.is_none());
        assert_eq!(by_latlng.lat, Some(45.97));
        assert_eq!(by_latlng.lng, Some(7.66));
    }

    /// A synthetic 3×3 DEM tilted purely east-west: elevation rises 10 m
    /// per cell to the east. With cell_w = cell_h = 10 m, dz/dx = 1.0,
    /// dz/dy = 0 → slope = atan(1) = 45°.
    #[test]
    fn horn_slope_pure_east_gradient_is_45deg() {
        // W column = 0, centre column = 10, E column = 20.
        let s = horn_slope_deg(
            0.0,  // NW
            10.0, // N (centre column)
            20.0, // NE
            0.0,  // W
            20.0, // E
            0.0,  // SW
            10.0, // S
            20.0, // SE
            10.0, // cell_w_m
            10.0, // cell_h_m
        );
        assert!((s - 45.0).abs() < 1e-6, "expected 45°, got {s}");
    }

    /// A flat plateau → zero slope, zero TRI, zero TPI.
    #[test]
    fn flat_terrain_is_all_zero() {
        let z = [100.0_f64; 8];
        let s = horn_slope_deg(z[0], z[1], z[2], z[3], z[4], z[5], z[6], z[7], 10.0, 10.0);
        assert!(s.abs() < 1e-9, "flat slope must be 0, got {s}");
        assert!(tri(100.0, &z).abs() < 1e-9);
        assert!(tpi(100.0, &z).abs() < 1e-9);
    }

    /// Riley TRI on a known stencil: centre 100, all 8 neighbours 97 →
    /// each squared residual is 9, summed = 72, sqrt = 8.485…
    #[test]
    fn tri_known_value() {
        let neigh = [97.0_f64; 8];
        let v = tri(100.0, &neigh);
        assert!((v - (72.0_f64).sqrt()).abs() < 1e-9, "got {v}");
    }

    /// Weiss TPI: centre on a peak (centre 110, neighbours mean 100) →
    /// +10 → ridge. Centre in a pit (centre 90) → −10 → valley.
    #[test]
    fn tpi_ridge_and_valley() {
        let neigh = [100.0_f64; 8];
        let ridge = tpi(110.0, &neigh);
        assert!((ridge - 10.0).abs() < 1e-9);
        assert_eq!(tpi_class(ridge), "ridge_or_crest");
        let valley = tpi(90.0, &neigh);
        assert!((valley + 10.0).abs() < 1e-9);
        assert_eq!(tpi_class(valley), "valley_or_depression");
        assert_eq!(tpi_class(0.4), "flat_or_midslope");
    }

    /// The neighbourhood generator must produce 8 DISTINCT neighbour
    /// cell64s, all different from the centre, for a mid-latitude cell.
    #[test]
    fn neighbourhood_yields_8_distinct_neighbours() {
        let centre = emem_codec::cell64_from_latlng(45.0, 7.0);
        let (c, neigh, w, h) = neighbourhood(&centre, 1).expect("decode");
        assert_eq!(neigh.len(), 8);
        let mut seen = std::collections::BTreeSet::new();
        seen.insert(c.clone());
        for (_d, n) in &neigh {
            assert_ne!(n, &c, "neighbour collided with centre");
            assert!(seen.insert(n.clone()), "neighbours must be distinct: {n}");
        }
        // East-west spacing narrows by cos(45°) ≈ 0.707 vs north-south.
        assert!(w < h, "east-west pitch must be < north-south at 45°N");
        assert!((h / w - 1.0 / 45.0_f64.to_radians().cos()).abs() < 1e-3);
    }
}
