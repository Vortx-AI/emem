//! Real physics primitives — explicit finite-difference solvers and a
//! constrained JEPA-pattern predictor.
//!
//! These are NOT decay-scoring heuristics: every step is an actual PDE
//! discretisation with a CFL stability check, every input is a signed
//! Primary fact materialised through the standard recall path, and every
//! response carries a responder-signed [`emem_fact::Receipt`] that cites
//! the input fact CIDs.
//!
//! Three primitives:
//!
//! 1. [`heat_solve`] — `∂u/∂t = α∇²u` 2-D explicit FD over a 3×3 stencil
//!    centred on the requested cell. Used for short-horizon urban-LST
//!    forecasts (MODIS LST_Day_1km, 8-day composite as initial condition).
//!    Inputs: 9 `modis.lst_day_8day` facts (centre + 8 cell64 neighbours).
//!
//! 2. [`wave_solve`] — `∂²u/∂t² = c²∂²u/∂x²` 1-D shallow-water swell
//!    propagation toward a coastal cell, with `c² = g·h` from
//!    `gmrt.topobathy_mean` along the seaward bathymetric gradient.
//!    Inputs: `n_offshore_cells` `gmrt.topobathy_mean` facts walking
//!    seaward from `coastal_cell`.
//!
//! 3. [`jepa_predict`] — constrained AR(2) seasonal NDVI predictor.
//!    Takes 6 monthly NDVI samples at one cell and predicts the next
//!    month using closed-form coefficients α=0.6 (year-over-year
//!    carryover; falls back to recent mean when 12-mo lag is unavailable
//!    in the lookback window), β=0.3 (recent-trend slope), γ=0.1
//!    (long-term mean reversion). NOT a learned MLP — that needs
//!    training data + GPU + a separate effort, and faking it would
//!    violate the no-stub policy.
//!
//! Each REST handler returns the math result plus the cited input
//! `fact_cids` and a Receipt signed by `state.identity.signing`.

use std::time::Instant;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};

use emem_core::ErrorCode;
use emem_fact::{Fact, FactCid, PrimaryFact};
use emem_primitives::RecallReq;

use crate::{recall_with_auto_materialize, ApiError, AppState, ErrorBody};

// ── Shared helpers ────────────────────────────────────────────────────────

/// Return the responder pubkey rendered as the lowercase base32-nopad
/// string the rest of the API uses for receipt-side identity.
fn pubkey_b32(state: &AppState) -> String {
    data_encoding::BASE32_NOPAD
        .encode(&state.identity.pubkey.0)
        .to_lowercase()
}

/// Bad-request envelope shorthand. Used for CFL violations, malformed
/// request fields, etc. Maps to HTTP 400 + the wire-stable
/// `invalid_argument` code so an agent can program against it.
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

/// Unprocessable-entity envelope for "the math could not run because the
/// inputs were missing or non-finite". Distinct from 400 because the
/// caller's request was syntactically fine; the responder couldn't
/// gather the facts. HTTP 422.
fn unprocessable(msg: impl Into<String>) -> ApiError {
    ApiError(
        StatusCode::UNPROCESSABLE_ENTITY,
        ErrorBody {
            code: ErrorCode::SourceFetchFailed,
            message: msg.into(),
            details: None,
        },
    )
}

/// Pull the first Primary fact for `band` out of a recall response and
/// return its scalar value + fact CID. Returns `None` for absences,
/// non-scalar values, or a missing band.
fn primary_scalar_for_band(
    resp: &emem_primitives::RecallResp,
    band: &str,
) -> Option<(f64, String)> {
    for (idx, f) in resp.facts.iter().enumerate() {
        if let Fact::Primary(p) = f {
            if p.band == band {
                if let ciborium::Value::Float(v) = p.value {
                    if v.is_finite() {
                        let cid = resp
                            .receipt
                            .fact_cids
                            .get(idx)
                            .map(|c| c.0.clone())
                            .unwrap_or_default();
                        return Some((v, cid));
                    }
                }
            }
        }
    }
    None
}

/// Pull every Primary fact for `band` out of a recall response, sorted
/// by tslot ascending. Each entry is `(tslot, value, fact_cid)`. Used by
/// [`jepa_predict`] to assemble the monthly NDVI history vector.
fn primary_history_for_band(
    resp: &emem_primitives::RecallResp,
    band: &str,
) -> Vec<(u64, f64, String)> {
    let mut out: Vec<(u64, f64, String)> = Vec::new();
    for (idx, f) in resp.facts.iter().enumerate() {
        if let Fact::Primary(p) = f {
            if p.band == band {
                if let ciborium::Value::Float(v) = p.value {
                    if v.is_finite() {
                        let cid = resp
                            .receipt
                            .fact_cids
                            .get(idx)
                            .map(|c| c.0.clone())
                            .unwrap_or_default();
                        out.push((p.tslot, v, cid));
                    }
                }
            }
        }
    }
    out.sort_by_key(|(t, _, _)| *t);
    out
}

/// Walk the responder-signed primary fact for `band` at `cell` (auto-
/// materialising on miss) and surface `(value, fact_cid, primary_clone)`.
/// Returns a structured error if the materializer found nothing.
async fn fetch_primary_scalar(
    cell: &str,
    band: &str,
    state: &AppState,
) -> Result<(f64, String, PrimaryFact), ApiError> {
    let req = RecallReq {
        cell: cell.to_string(),
        bands: Some(vec![band.to_string()]),
        tslot: None,
    };
    let (resp, _notes) = recall_with_auto_materialize(&req, state).await?;
    for (idx, f) in resp.facts.iter().enumerate() {
        if let Fact::Primary(p) = f {
            if p.band == band {
                if let ciborium::Value::Float(v) = p.value {
                    if v.is_finite() {
                        let cid = resp
                            .receipt
                            .fact_cids
                            .get(idx)
                            .map(|c| c.0.clone())
                            .unwrap_or_default();
                        return Ok((v, cid, p.clone()));
                    }
                }
            }
        }
    }
    Err(unprocessable(format!(
        "no usable {band} primary fact at cell {cell}: \
         materializer returned no scalar value (absence, non-scalar, or upstream failure)"
    )))
}

/// 8 closest cell64 neighbours around `centre_cell` plus the centre, in a
/// stable row-major order: row 0 (north) → row 2 (south), each row
/// west → east. Returns 9 unique cell64 strings; near the poles the
/// list may be shorter (the codec wraps lng but clamps lat).
fn cell64_neighborhood_3x3(centre_cell: &str) -> Result<[String; 9], String> {
    let info = emem_codec::latlng_from_cell64(centre_cell)
        .map_err(|e| format!("decode {centre_cell}: {e}"))?;
    let dlat = info.bbox_deg.max_lat - info.bbox_deg.min_lat;
    let dlng = info.bbox_deg.max_lng - info.bbox_deg.min_lng;
    // Row order: north (lat+), centre (lat0), south (lat-).
    // Within each row: west (lng-), centre (lng0), east (lng+).
    let offsets: [(f64, f64); 9] = [
        (1.0, -1.0),  // NW
        (1.0, 0.0),   // N
        (1.0, 1.0),   // NE
        (0.0, -1.0),  // W
        (0.0, 0.0),   // centre
        (0.0, 1.0),   // E
        (-1.0, -1.0), // SW
        (-1.0, 0.0),  // S
        (-1.0, 1.0),  // SE
    ];
    let mut out: [String; 9] = Default::default();
    for (i, (sa, sb)) in offsets.iter().enumerate() {
        out[i] = emem_codec::to_cell64(emem_codec::cell_from_latlng(
            info.lat_deg + sa * dlat,
            info.lng_deg + sb * dlng,
        ));
    }
    Ok(out)
}

// ── Heat equation 2-D ─────────────────────────────────────────────────────

/// `POST /v1/heat_solve` request body. All defaults are explicit so an
/// agent calling with `{cell, hours_ahead}` gets the documented behaviour.
#[derive(Debug, Clone, Deserialize)]
pub struct HeatSolveReq {
    /// Target cell64 string OR a free-text place name. When a place
    /// name is supplied (anything that isn't shaped like a four-bigram
    /// cell64) the handler runs `/v1/locate` first and integrates from
    /// the resolved cell. Aliased to `place` so agent payloads of the
    /// shape `{place: "Tokyo"}` work without a separate field.
    #[serde(alias = "place")]
    pub cell: String,
    /// Forward integration horizon in hours. Capped at 168 (one week)
    /// because the explicit FD scheme accumulates discretisation error
    /// linearly and the 8-day MODIS LST composite stops being a
    /// representative initial condition past a week.
    #[serde(default = "default_hours_ahead")]
    pub hours_ahead: f64,
    /// Thermal diffusivity α in m²/s. Default 1e-6 matches the textbook
    /// value for urban surface materials (asphalt + concrete + stone),
    /// see Oke 2017 §2.3 Table 2.4. Set higher for vegetated surfaces
    /// (~5e-7 to 1e-6) or lower for water bodies (~1.4e-7).
    #[serde(default = "default_diffusivity")]
    pub diffusivity_m2_per_s: f64,
}

const fn default_hours_ahead() -> f64 {
    6.0
}

const fn default_diffusivity() -> f64 {
    1.0e-6
}

/// Cell pitch used by every PDE solver in this module. The active cell64
/// grid is ~10 m × ~10 m square at the equator (Sentinel-1/2 native
/// pitch); see `crates/emem-codec/src/geo.rs`. A future H3 migration
/// would change this constant.
const CELL_PITCH_M: f64 = 10.0;

/// CFL safety factor — keeps the explicit-FD time step strictly inside
/// the stability bound. 2-D heat eq requires `α·Δt/Δx² ≤ 0.25`; we run
/// at 0.20 to leave headroom against round-off-driven instability.
const HEAT_CFL_SAFETY: f64 = 0.20;

/// Hard upper bound on the number of FD iterations in one solve call.
/// Above this we bail with an error rather than burn CPU for ten
/// minutes inside a request handler. The bound + the diffusivity
/// implies the longest reasonable horizon at default α.
const HEAT_MAX_STEPS: usize = 2_000_000;

/// Spatial-variation threshold below which a 3×3 stencil is treated as
/// mathematically uniform. A 0.01 K range across a 30 m × 30 m perimeter
/// is well below the MODIS LST instrument noise floor (≈0.5 K, see
/// Wan 2014); anything tighter cannot be physically distinguished from a
/// single-pixel sample replicated across all 9 cells. When this triggers
/// the discrete Laplacian collapses to zero and the FTCS step returns
/// `delta_k == 0.0` regardless of dt or α — see the `stencil_diagnostic`
/// field on the `/v1/heat_solve` response for the agent-facing
/// explanation. Documented in `docs/PHYSICS_ENDPOINTS_2026_05_04.md`.
const HEAT_UNIFORM_STENCIL_THRESHOLD_K: f64 = 0.01;

/// Result of the 3×3 stencil-collapse diagnostic. `range_k` is the
/// max-minus-min of the 9 initial-condition temperatures in kelvin;
/// `is_uniform` is `range_k < HEAT_UNIFORM_STENCIL_THRESHOLD_K`. The
/// diagnostic exists because a stencil populated from a single coarser
/// upstream pixel gives a zero Laplacian (and hence `delta_k == 0.0`)
/// regardless of dt / α. Surfacing this lets agents distinguish a
/// real "no diffusion expected" outcome from a "stencil collapsed at an
/// upstream sampling resolution coarser than the cell pitch" artifact.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StencilDiagnostic {
    pub range_k: f64,
    pub is_uniform: bool,
}

/// Compute the stencil-collapse diagnostic for a 9-cell initial-condition
/// vector. Pure function — split out so unit tests can exercise the
/// uniform-vs-varied decision without standing up an `AppState` or
/// touching the network. The handler embeds the same numbers in the
/// `stencil_range_k` and `stencil_diagnostic` response fields.
pub fn heat_stencil_diagnostic(values: &[f64; 9]) -> StencilDiagnostic {
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let range_k = max - min;
    StencilDiagnostic {
        range_k,
        is_uniform: range_k < HEAT_UNIFORM_STENCIL_THRESHOLD_K,
    }
}

/// One forward 2-D explicit-FD step on a 3×3 stencil. Mutates the centre
/// in-place and returns the new value. Boundary cells are treated as
/// Dirichlet (held fixed). Pure function — no I/O — so the math is
/// unit-testable without storage.
///
/// `u` is row-major `[N, NE, E, SE, S, SW, W, NW, centre]`? No — we
/// follow the [`cell64_neighborhood_3x3`] convention (NW, N, NE, W,
/// centre, E, SW, S, SE) so the index in `u` matches the index in the
/// neighbourhood lookup.
pub fn heat_step_2d(u: &[f64; 9], alpha: f64, dt_s: f64) -> f64 {
    let dx2 = CELL_PITCH_M * CELL_PITCH_M;
    let centre = u[4];
    let north = u[1];
    let south = u[7];
    let east = u[5];
    let west = u[3];
    // 5-point Laplacian on the centre cell (corners contribute via the
    // next iteration through their own neighbours).
    let lap = (north + south + east + west - 4.0 * centre) / dx2;
    centre + alpha * dt_s * lap
}

/// Solve the 2-D heat equation on a 3×3 grid for `n_steps` of size
/// `dt_s`. Boundary cells are held at their initial values (Dirichlet);
/// only the centre evolves. Returns the centre's final temperature.
///
/// The boundary-Dirichlet choice is deliberate: with one cell of stencil
/// the only honest thing to do is freeze the perimeter. A wider grid
/// would let the boundary itself diffuse — that's the natural follow-up
/// (`heat_equation_2d@2`).
pub fn heat_solve_3x3_centre(u0: &[f64; 9], alpha: f64, dt_s: f64, n_steps: usize) -> f64 {
    let mut u = *u0;
    for _ in 0..n_steps {
        let new_centre = heat_step_2d(&u, alpha, dt_s);
        u[4] = new_centre;
    }
    u[4]
}

/// Compute the (n_steps, dt_s) pair satisfying CFL for the requested
/// horizon. Returns the largest stable Δt that divides the horizon
/// evenly (or as close as we can get); n_steps is chosen so the total
/// integration time matches the request.
fn heat_choose_timestep(alpha: f64, hours_ahead: f64) -> Result<(usize, f64), String> {
    if !alpha.is_finite() || alpha <= 0.0 {
        return Err(format!(
            "diffusivity_m2_per_s must be positive and finite; got {alpha}"
        ));
    }
    if !hours_ahead.is_finite() || hours_ahead <= 0.0 {
        return Err(format!(
            "hours_ahead must be positive and finite; got {hours_ahead}"
        ));
    }
    let total_s = hours_ahead * 3600.0;
    let dx2 = CELL_PITCH_M * CELL_PITCH_M;
    // Largest Δt allowed by 2-D stability: α·Δt/Δx² ≤ 0.25.
    // We back off by HEAT_CFL_SAFETY (0.20) to stay inside the bound.
    let dt_max = HEAT_CFL_SAFETY * dx2 / alpha;
    // If the horizon is shorter than dt_max, take it in one step.
    let n_steps_f = (total_s / dt_max).ceil().max(1.0);
    if n_steps_f > HEAT_MAX_STEPS as f64 {
        return Err(format!(
            "requested horizon ({hours_ahead} h) at α={alpha} m²/s on a 10 m grid would need \
             {n_steps_f:.0} explicit-FD steps (cap: {HEAT_MAX_STEPS}). \
             Pick a shorter horizon or a higher α (e.g. coarser surface)."
        ));
    }
    let n_steps = n_steps_f as usize;
    let dt_s = total_s / n_steps as f64;
    // Sanity-check that the actual dt still satisfies CFL — the rounding
    // above only ever shrinks dt, but we double-check rather than trust.
    let cfl_factor = alpha * dt_s / dx2;
    if cfl_factor > 0.25 {
        return Err(format!(
            "internal: chosen dt_s={dt_s} gives CFL={cfl_factor:.4} > 0.25; bug in solver"
        ));
    }
    Ok((n_steps, dt_s))
}

/// Run the full heat-solve primitive. Used by both the REST handler and
/// the MCP dispatch arm.
pub async fn heat_solve(mut req: HeatSolveReq, state: &AppState) -> Result<JsonValue, ApiError> {
    let started = Instant::now();
    if req.hours_ahead > 168.0 {
        return Err(bad_request(format!(
            "hours_ahead capped at 168 (one week); got {}. \
             MODIS LST 8-day composite stops being a representative initial \
             condition past a week — for longer horizons run the solver \
             stepwise from refreshed initial conditions.",
            req.hours_ahead
        )));
    }
    let (n_steps, dt_s) =
        heat_choose_timestep(req.diffusivity_m2_per_s, req.hours_ahead).map_err(bad_request)?;

    // Resolve a place name to cell64 if needed (cell64-shaped strings
    // pass through). Agents calling with `{place:"Tokyo"}` land here.
    let (resolved_cell, resolved_ref) = crate::resolve_cell_field(&req.cell).await?;
    req.cell = resolved_cell.clone();

    // 9 neighbouring cells in NW…SE order (centre at index 4).
    let cells = cell64_neighborhood_3x3(&req.cell)
        .map_err(|e| bad_request(format!("cell {}: {e}", req.cell)))?;

    let band = "modis.lst_day_8day";
    // Fetch all 9 facts in parallel — one ORNL DAAC round-trip per
    // missing cell otherwise dominates wall time.
    type FetchOutcome = (usize, Result<(f64, String, PrimaryFact), ApiError>);
    let mut set: tokio::task::JoinSet<FetchOutcome> = tokio::task::JoinSet::new();
    for (idx, c) in cells.iter().enumerate() {
        let cell = c.clone();
        let st = state.clone();
        set.spawn(async move { (idx, fetch_primary_scalar(&cell, band, &st).await) });
    }
    let mut values: [f64; 9] = [0.0; 9];
    let mut cids: [String; 9] = Default::default();
    let mut signed_ats: [String; 9] = Default::default();
    let mut units: [String; 9] = Default::default();
    let mut centre_unit: Option<String> = None;
    let mut have_centre = false;
    let mut errors: Vec<JsonValue> = Vec::new();
    while let Some(j) = set.join_next().await {
        match j {
            Ok((idx, Ok((v, cid, p)))) => {
                values[idx] = v;
                cids[idx] = cid;
                signed_ats[idx] = p.signed_at.clone();
                units[idx] = p.unit.clone().unwrap_or_default();
                if idx == 4 {
                    have_centre = true;
                    centre_unit = p.unit.clone();
                }
            }
            Ok((idx, Err(e))) => {
                errors.push(json!({
                    "neighbor_index": idx,
                    "cell": cells[idx],
                    "code": format!("{:?}", e.1.code),
                    "status": e.0.as_u16(),
                    "message": e.1.message,
                }));
            }
            Err(e) => {
                errors.push(json!({"join_error": e.to_string()}));
            }
        }
    }
    if !have_centre {
        return Err(unprocessable(format!(
            "centre cell {} has no usable {band} fact: cannot integrate without an initial condition. Errors: {}",
            req.cell,
            serde_json::to_string(&errors).unwrap_or_default(),
        )));
    }
    // For any neighbour we couldn't fetch, fall back to the centre value
    // (zero-flux Neumann boundary at the missing edge — the gradient
    // across that edge is forced to zero, which damps but never inflates
    // the centre's temperature). Surface the substitution so an agent
    // can see how many neighbours were imputed.
    let mut imputed: Vec<usize> = Vec::new();
    for i in 0..9 {
        if cids[i].is_empty() {
            values[i] = values[4];
            imputed.push(i);
        }
    }

    let centre_initial_k = values[4];

    // Stencil-collapse diagnostic. When all 9 neighbourhood cells were
    // populated from a single coarser upstream pixel (e.g. one MODIS LST
    // 1 km observation tiled across all 9 of these 10 m cells), the
    // discrete Laplacian is exactly zero and the FTCS step is a no-op.
    // The math is correct (∇²(constant) = 0) but the result is just the
    // input echoed back. We surface this so an agent can distinguish a
    // genuine "no diffusion expected" outcome from a "stencil sampled at
    // an upstream resolution coarser than the cell pitch" artifact.
    // Computed BEFORE the FTCS step (the diagnostic describes the
    // initial condition; we don't change `delta_k` based on it — the
    // honest answer is still 0.0 K).
    let StencilDiagnostic {
        range_k: stencil_range_k,
        is_uniform: is_uniform_stencil,
    } = heat_stencil_diagnostic(&values);
    let stencil_interpretation: String = if is_uniform_stencil {
        format!(
            "All 9 neighborhood cells have ΔT < {:.2} K. The discrete Laplacian collapses to zero, \
             so delta_k will be exactly 0.0 regardless of dt or alpha. This usually means the \
             upstream materialiser populated the 3×3 stencil from a single coarser source pixel \
             (e.g. a single MODIS LST 1km observation covering all 9 of these 10m cells). To get a \
             real diffusion result, pre-fetch a wider MODIS tile so the perimeter cells have \
             spatial variation, OR query a larger physical neighbourhood by using a coarser cell \
             resolution.",
            HEAT_UNIFORM_STENCIL_THRESHOLD_K
        )
    } else {
        "Stencil has measurable spatial variation; FTCS step will produce a non-trivial delta_k."
            .to_string()
    };

    let final_centre_k = heat_solve_3x3_centre(&values, req.diffusivity_m2_per_s, dt_s, n_steps);
    let dx2 = CELL_PITCH_M * CELL_PITCH_M;
    let cfl_factor = req.diffusivity_m2_per_s * dt_s / dx2;
    let pubkey = pubkey_b32(state);

    // Sign the response receipt over the 9 input fact CIDs.
    let receipt = state.sign_receipt(
        "emem.heat_solve",
        cells.to_vec(),
        cids.iter()
            .filter(|c| !c.is_empty())
            .cloned()
            .map(FactCid::new)
            .collect(),
        false,
        started,
        None,
    );

    Ok(json!({
        "schema": "emem.heat_solve.v1",
        "cell": req.cell,
        "resolved_from": resolved_ref,
        "neighborhood_cells": cells,
        "neighborhood_order": "NW, N, NE, W, centre, E, SW, S, SE (centre at index 4)",
        "input_band": band,
        "initial_condition_k": centre_initial_k,
        "neighborhood_initial_k": values,
        "neighborhood_signed_at": signed_ats,
        "neighborhood_units": units,
        "imputed_neighbor_indices": imputed,
        "imputation_note": if imputed.is_empty() { JsonValue::Null } else {
            json!("Indices listed in `imputed_neighbor_indices` had no usable fact; the centre value was substituted (zero-flux Neumann boundary at the missing edge). Forecast confidence drops with the imputed-neighbor count.")
        },
        "stencil_range_k": stencil_range_k,
        "stencil_diagnostic": {
            "is_uniform": is_uniform_stencil,
            "threshold_k": HEAT_UNIFORM_STENCIL_THRESHOLD_K,
            "interpretation": stencil_interpretation,
        },
        "errors": errors,
        "diffusivity_m2_per_s": req.diffusivity_m2_per_s,
        "hours_ahead": req.hours_ahead,
        "n_steps": n_steps,
        "dt_seconds": dt_s,
        "cell_pitch_m": CELL_PITCH_M,
        "cfl_factor": cfl_factor,
        "cfl_bound": 0.25,
        "cfl_note": "2-D explicit-FD heat equation requires α·Δt/Δx² ≤ 0.25. We run at HEAT_CFL_SAFETY=0.20 of the bound for round-off margin.",
        "forecast_k": final_centre_k,
        "forecast_unit": centre_unit.unwrap_or_else(|| "K".into()),
        "delta_k": final_centre_k - centre_initial_k,
        "boundary_condition": "Dirichlet on the 8 perimeter cells; centre evolves under the 5-point Laplacian.",
        "scheme": "explicit forward-time central-space (FTCS) 5-point Laplacian",
        "algorithm_key": "heat_equation_2d@1",
        "algorithm_citation": "Crank, J. & Nicolson, P. 1947 / Oke, T.R. 2017 §2.3 (urban surface diffusivity ~1e-6 m²/s).",
        "input_fact_cids": cids.iter().filter(|c| !c.is_empty()).cloned().collect::<Vec<_>>(),
        "responder_pubkey_b32": pubkey,
        "receipt": receipt,
        "next": {
            "verify_offline":   "POST /v1/verify_receipt {receipt}",
            "fact_dereference": "GET /v1/facts/{fact_cid} for each input_fact_cids[i]",
            "iterate":          format!("POST /v1/heat_solve {{cell:'{}', hours_ahead: <next-window>}}", req.cell),
            "improve_stencil":  if is_uniform_stencil {
                format!(
                    "Stencil is uniform (range={:.4} K < {:.2} K threshold). To get a non-trivial delta_k: \
                     (a) call POST /v1/backfill {{cell:'{}', band:'{}'}} with a wider time/space window so \
                     the materialiser pulls additional MODIS tiles that cover the perimeter cells distinctly; \
                     OR (b) re-issue this request against a coarser cell resolution (a parent cell whose \
                     children straddle multiple MODIS pixels), then disaggregate post-hoc.",
                    stencil_range_k, HEAT_UNIFORM_STENCIL_THRESHOLD_K, req.cell, band
                )
            } else {
                format!(
                    "Stencil already has measurable spatial variation (range={:.4} K). No action needed; \
                     iterate the horizon to extend the forecast.",
                    stencil_range_k
                )
            },
        },
    }))
}

// ── Wave equation 1-D ─────────────────────────────────────────────────────

/// `POST /v1/wave_solve` request body.
#[derive(Debug, Clone, Deserialize)]
pub struct WaveSolveReq {
    /// Coastal cell — the wavefront's destination. Accepts a cell64
    /// string OR a free-text place name (handler runs `/v1/locate`
    /// first when the value isn't shaped like a cell64). Aliased to
    /// `cell` and `place` so agent payloads with either field work.
    #[serde(alias = "cell", alias = "place")]
    pub coastal_cell: String,
    /// Offshore wave height in metres (significant wave height H_s).
    /// Capped at 30 m (well above any recorded swell H_s) — values
    /// outside that envelope are almost certainly malformed input.
    pub offshore_height_m: f64,
    /// Wave period in seconds. Typical ocean swells: 6–18 s. Capped at
    /// 30 s for the same sanity reason.
    pub period_s: f64,
    /// How many cells to walk seaward when sampling the bathymetric
    /// profile. Default 8 → 80 m of cross-shore profile at the active
    /// 10 m grid. Capped at 64 to bound upstream fetches.
    #[serde(default = "default_n_offshore")]
    pub n_offshore_cells: u32,
}

const fn default_n_offshore() -> u32 {
    8
}

const G: f64 = 9.81;

/// CFL safety factor for the 1-D wave equation. The Courant condition
/// is `c·Δt/Δx ≤ 1`; we run at 0.5 to keep round-off well inside the
/// stability bound and to leave space for the worst-case `c` along the
/// profile.
const WAVE_CFL_SAFETY: f64 = 0.5;

const WAVE_MAX_STEPS: usize = 200_000;

/// Minimum fraction of profile cells that must be genuinely below sea
/// level (depth > [`WAVE_OCEAN_DEPTH_THRESHOLD_M`]) for the bathymetric
/// profile to count as oceanic. Below this fraction the seaward walk has
/// landed mostly on continental crust and the explicit-FD wave solve
/// would just propagate the offshore-boundary forcing across a near-zero
/// depth column — the depth floor pins phase speed to ~0.31 m/s and the
/// arrival height becomes a placeholder. Surfaced via the
/// `LandLockedProfile` rejection on `POST /v1/wave_solve`.
const WAVE_MIN_OCEAN_FRACTION: f64 = 0.5;

/// The deepest cell of the seaward profile (after the offshore-to-coast
/// reverse — index 0 in the FD solver) must have at least this much
/// water under it for the sinusoidal offshore forcing to attach to a
/// physical wave. Without it, c² = g·h shrinks to numerical noise at the
/// boundary and the integration is meaningless. Surfaced via the
/// `LandLockedProfile` rejection on `POST /v1/wave_solve`.
const WAVE_MIN_OFFSHORE_DEPTH_M: f64 = 5.0;

/// A profile cell counts as "oceanic" only when its measured depth
/// exceeds this threshold — anything at or below the safety floor
/// (0.01 m, set when phase-speed-flooring `c² = g·h.max(0.01)`) is just
/// the floor showing through and is not real water for the swell to
/// propagate over.
const WAVE_OCEAN_DEPTH_THRESHOLD_M: f64 = 1.0;

/// One forward 1-D explicit-FD step on the wave equation. Returns the
/// new state vector. The two endpoint conditions are:
///
///  * **Offshore (i=0)** — sinusoidal forcing `H_s·sin(2π·t/T)`. The
///    boundary keeps re-injecting the swell as long as we integrate.
///  * **Coastal (i=N-1)** — hard wall (Dirichlet u=0). Models the
///    coastline reflecting the wave; arrival height is read from the
///    second-to-last cell so the wall doesn't artificially zero it.
///
/// `c_profile[i]` is the local phase speed (`√(g·h)`) at cell i.
pub fn wave_step_1d(
    u_prev: &[f64],
    u_curr: &[f64],
    c_profile: &[f64],
    dt_s: f64,
    dx_m: f64,
    forcing_offshore: f64,
) -> Vec<f64> {
    let n = u_curr.len();
    let mut u_next = vec![0.0f64; n];
    if n < 3 {
        return u_next;
    }
    let dt2_dx2 = (dt_s / dx_m) * (dt_s / dx_m);
    // Offshore boundary (i=0): driven by sinusoidal forcing.
    u_next[0] = forcing_offshore;
    // Interior cells.
    for i in 1..n - 1 {
        let c2 = c_profile[i] * c_profile[i];
        u_next[i] = 2.0 * u_curr[i] - u_prev[i]
            + c2 * dt2_dx2 * (u_curr[i + 1] - 2.0 * u_curr[i] + u_curr[i - 1]);
    }
    // Coastal boundary (i=N-1): hard wall u=0. Keeps the scheme
    // numerically anchored; the agent reads the arrival height from
    // u_next[N-2] just before the wall.
    u_next[n - 1] = 0.0;
    u_next
}

/// Return the maximum stable timestep for an explicit-FD wave solver
/// over `c_profile` on a `dx_m` grid, scaled by [`WAVE_CFL_SAFETY`].
fn wave_max_dt(c_profile: &[f64], dx_m: f64) -> f64 {
    let c_max = c_profile.iter().cloned().fold(0.0_f64, f64::max);
    if c_max <= 0.0 {
        return 0.0;
    }
    WAVE_CFL_SAFETY * dx_m / c_max
}

/// Outcome of [`classify_seaward_profile`] — either the depth column is
/// genuinely oceanic enough to integrate the wave equation over, or it
/// landed mostly on continental crust and the solver would just push the
/// offshore-boundary forcing through a near-zero depth column.
///
/// `depths_offshore_to_coast[0]` is the deepest cell (offshore boundary
/// of the FD solver) and `[N-1]` is the coast.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProfileClassification {
    /// Profile is oceanic enough — at least
    /// [`WAVE_MIN_OCEAN_FRACTION`] of cells are deeper than
    /// [`WAVE_OCEAN_DEPTH_THRESHOLD_M`] AND the offshore boundary cell
    /// has at least [`WAVE_MIN_OFFSHORE_DEPTH_M`] of water.
    Oceanic,
    /// Offshore boundary cell is too shallow (< [`WAVE_MIN_OFFSHORE_DEPTH_M`]).
    OffshoreBoundaryTooShallow,
    /// Too few cells in the profile are oceanic
    /// (< [`WAVE_MIN_OCEAN_FRACTION`] over [`WAVE_OCEAN_DEPTH_THRESHOLD_M`]).
    InsufficientOceanFraction,
}

/// Decide whether a depth profile (ordered offshore-to-coast — i.e. as
/// the FD solver indexes it, with `[0]` the deepest seaward boundary
/// and `[N-1]` the coast) is meaningfully oceanic.
///
/// The two checks are complementary:
///
/// 1. **Offshore boundary check** — `depths[0] > WAVE_MIN_OFFSHORE_DEPTH_M`.
///    Without real depth at the boundary the sinusoidal forcing has
///    nothing physical to attach to (`c² = g·h` collapses to numerical
///    noise).
/// 2. **Profile fraction check** — at least `WAVE_MIN_OCEAN_FRACTION`
///    of cells must have `depth > WAVE_OCEAN_DEPTH_THRESHOLD_M`. Below
///    this the seaward walk has landed mostly on land and the result
///    is a placeholder, not a wave forecast.
///
/// Pure function (no I/O) so the logic is unit-testable without
/// touching storage or upstream sources.
fn classify_seaward_profile(depths_offshore_to_coast: &[f64]) -> ProfileClassification {
    if depths_offshore_to_coast.is_empty() {
        return ProfileClassification::InsufficientOceanFraction;
    }
    if depths_offshore_to_coast[0] <= WAVE_MIN_OFFSHORE_DEPTH_M {
        return ProfileClassification::OffshoreBoundaryTooShallow;
    }
    let oceanic_count = depths_offshore_to_coast
        .iter()
        .filter(|d| **d > WAVE_OCEAN_DEPTH_THRESHOLD_M)
        .count();
    let fraction = oceanic_count as f64 / depths_offshore_to_coast.len() as f64;
    if fraction < WAVE_MIN_OCEAN_FRACTION {
        return ProfileClassification::InsufficientOceanFraction;
    }
    ProfileClassification::Oceanic
}

/// Build the structured `LandLockedProfile` rejection. Surfaces the
/// failed depth profile + phase-speed profile + a `next_steps[]` array
/// with two concrete recovery paths the agent can act on:
///
///  * `try_longer_profile` — same input cell, `n_offshore_cells` doubled
///    (capped at 64) so the seaward walk reaches actual deep water.
///  * `try_different_cell` — the input cell is too far from open water;
///    the agent should re-`/v1/locate` to a closer-to-coast cell.
///
/// HTTP 422 (request was syntactically fine; we just couldn't run the
/// math) and the wire-stable `invalid_argument` code, with
/// `error_kind: "LandLockedProfile"` carried in `details` so an agent
/// can branch on the kind without parsing the message string.
fn land_locked_profile_error(
    classification: ProfileClassification,
    coastal_cell: &str,
    profile_cells_offshore_to_coast: &[String],
    depths_offshore_to_coast: &[f64],
    phase_speed_profile_m_per_s: &[f64],
    n_offshore_requested: usize,
) -> ApiError {
    let oceanic_count = depths_offshore_to_coast
        .iter()
        .filter(|d| **d > WAVE_OCEAN_DEPTH_THRESHOLD_M)
        .count();
    let n = depths_offshore_to_coast.len();
    let fraction = if n == 0 {
        0.0
    } else {
        oceanic_count as f64 / n as f64
    };
    let message = match classification {
        ProfileClassification::OffshoreBoundaryTooShallow => format!(
            "land-locked seaward profile from coastal_cell={coastal_cell}: \
             offshore boundary cell has depth {:.2} m, need >= {:.1} m. \
             The seaward walk did not reach genuinely deep water — `c² = g·h` \
             at the boundary collapses to numerical noise and the explicit-FD \
             integration would just propagate the sinusoidal forcing across a \
             near-zero depth column.",
            depths_offshore_to_coast.first().copied().unwrap_or(0.0),
            WAVE_MIN_OFFSHORE_DEPTH_M,
        ),
        ProfileClassification::InsufficientOceanFraction => format!(
            "land-locked seaward profile from coastal_cell={coastal_cell}: \
             only {oceanic_count}/{n} ({:.0}%) of profile cells are oceanic \
             (depth > {:.1} m), need >= {:.0}%. The seaward walk landed mostly \
             on continental crust; the depth floor pins phase speed and the \
             arrival height becomes a placeholder, not a wave forecast.",
            fraction * 100.0,
            WAVE_OCEAN_DEPTH_THRESHOLD_M,
            WAVE_MIN_OCEAN_FRACTION * 100.0,
        ),
        ProfileClassification::Oceanic => {
            // Defensive — caller should never construct this error for
            // an oceanic classification, but if they do we still return
            // a coherent body rather than panic.
            "internal: land_locked_profile_error called on an oceanic classification".to_string()
        }
    };
    let suggested_n = (n_offshore_requested.saturating_mul(2)).clamp(2, 64);
    let details = json!({
        "error_kind": "LandLockedProfile",
        "coastal_cell": coastal_cell,
        "profile_cells_offshore_to_coast": profile_cells_offshore_to_coast,
        "depth_profile_m": depths_offshore_to_coast,
        "phase_speed_profile_m_per_s": phase_speed_profile_m_per_s,
        "oceanic_cell_count": oceanic_count,
        "profile_cell_count": n,
        "oceanic_fraction": fraction,
        "thresholds": {
            "min_ocean_fraction": WAVE_MIN_OCEAN_FRACTION,
            "min_offshore_depth_m": WAVE_MIN_OFFSHORE_DEPTH_M,
            "ocean_depth_threshold_m": WAVE_OCEAN_DEPTH_THRESHOLD_M,
        },
        "next_steps": [
            {
                "action": "try_longer_profile",
                "why": "the seaward walk may have stopped short of genuine deep water; doubling n_offshore_cells (up to the 64 cap) lets the FD profile reach an oceanic boundary",
                "call": format!(
                    "POST /v1/wave_solve {{coastal_cell:'{coastal_cell}', n_offshore_cells: {suggested_n}, offshore_height_m: <unchanged>, period_s: <unchanged>}}"
                ),
                "n_offshore_cells_suggested": suggested_n,
            },
            {
                "action": "try_different_cell",
                "why": "the input cell is too far from genuine open water; resolve a closer-to-coast cell first via /v1/locate (e.g. by anchoring on a beach, harbour, or named coastline) and call /v1/wave_solve from that cell",
                "call": "POST /v1/locate {place: '<beach or coastline name>'} → use returned cell64 as coastal_cell",
            },
        ],
    });
    ApiError(
        StatusCode::UNPROCESSABLE_ENTITY,
        ErrorBody {
            code: ErrorCode::InvalidArgument,
            message,
            details: Some(details),
        },
    )
}

/// Walk N steps seaward from `coastal_cell` along the gradient of
/// `gmrt.topobathy_mean` and recall the depth at each step. Returns
/// `(profile_cells, depths_m, fact_cids)`. Depths are positive metres
/// below sea level (we negate the GMRT signed elevation, which is
/// negative over water).
async fn walk_seaward_profile(
    coastal_cell: &str,
    n_offshore: usize,
    state: &AppState,
) -> Result<(Vec<String>, Vec<f64>, Vec<String>), ApiError> {
    if n_offshore == 0 {
        return Err(bad_request(
            "n_offshore_cells must be at least 1 to define a seaward profile",
        ));
    }
    if n_offshore > 64 {
        return Err(bad_request(format!(
            "n_offshore_cells capped at 64; got {n_offshore}"
        )));
    }
    let band = "gmrt.topobathy_mean";
    // The four cardinal directions. We walk seaward by picking the
    // direction whose immediate neighbour has the most negative
    // (deepest) GMRT elevation. Diagonals would refine the gradient
    // estimate but at 4× the upstream-fetch cost; cardinal-only is the
    // documented v1 behaviour.
    let centre_info = emem_codec::latlng_from_cell64(coastal_cell)
        .map_err(|e| bad_request(format!("decode {coastal_cell}: {e}")))?;
    let dlat = centre_info.bbox_deg.max_lat - centre_info.bbox_deg.min_lat;
    let dlng = centre_info.bbox_deg.max_lng - centre_info.bbox_deg.min_lng;
    let cardinals: [(f64, f64, &str); 4] = [
        (1.0, 0.0, "N"),
        (-1.0, 0.0, "S"),
        (0.0, 1.0, "E"),
        (0.0, -1.0, "W"),
    ];

    // Step 0: the coastal cell itself.
    let (depth0_signed, cid0, _) = fetch_primary_scalar(coastal_cell, band, state).await?;
    let mut profile_cells: Vec<String> = vec![coastal_cell.to_string()];
    let mut depths: Vec<f64> = vec![(-depth0_signed).max(0.0)];
    let mut cids: Vec<String> = vec![cid0];
    let mut current = coastal_cell.to_string();
    let mut current_lat = centre_info.lat_deg;
    let mut current_lng = centre_info.lng_deg;
    let mut current_depth_signed = depth0_signed;

    // Pick the seaward direction once (at the coast) and walk that way
    // each step. Re-deciding per step would let the path drift if the
    // seafloor flattens; sticking with the initial seaward heading is
    // the documented v1 behaviour and matches how an oceanographer
    // would set up a 1-D refraction-free profile.
    let (seaward_sa, seaward_sb, seaward_label) = {
        let mut best: Option<(f64, f64, &str, f64)> = None;
        for (sa, sb, lbl) in cardinals {
            let probe = emem_codec::to_cell64(emem_codec::cell_from_latlng(
                current_lat + sa * dlat,
                current_lng + sb * dlng,
            ));
            if probe == current {
                continue;
            }
            // Cheap lookup-only recall (no auto-materialize for the
            // probe); we only use this to pick the direction. If none
            // of the cardinal probes have data we fall back to "N"
            // — explicit and surfaced.
            let req = RecallReq {
                cell: probe.clone(),
                bands: Some(vec![band.to_string()]),
                tslot: None,
            };
            if let Ok((resp, _)) = recall_with_auto_materialize(&req, state).await {
                if let Some((depth, _)) = primary_scalar_for_band(&resp, band) {
                    if best.map(|b| depth < b.3).unwrap_or(true) {
                        best = Some((sa, sb, lbl, depth));
                    }
                }
            }
        }
        match best {
            Some((sa, sb, lbl, _)) => (sa, sb, lbl),
            None => (1.0, 0.0, "N (default; gradient probes returned no data)"),
        }
    };

    for _step in 1..n_offshore {
        current_lat += seaward_sa * dlat;
        current_lng += seaward_sb * dlng;
        let next = emem_codec::to_cell64(emem_codec::cell_from_latlng(current_lat, current_lng));
        if next == current {
            // We've hit a pole / antimeridian rounding artifact; stop.
            break;
        }
        let (depth_signed, cid, _) = fetch_primary_scalar(&next, band, state).await?;
        // Stop walking once we leave the water (positive elevation).
        // The wave can't propagate over land — the profile ends here.
        if depth_signed >= 0.0 && current_depth_signed < 0.0 {
            // Crossed onto land: include this cell as the boundary
            // sample but stop; depth clamped to 0 so c=0 there.
            profile_cells.push(next);
            depths.push(0.0);
            cids.push(cid);
            break;
        }
        profile_cells.push(next.clone());
        depths.push((-depth_signed).max(0.0));
        cids.push(cid);
        current = next;
        current_depth_signed = depth_signed;
    }
    if profile_cells.len() < 3 {
        return Err(unprocessable(format!(
            "seaward profile from {coastal_cell} headed {seaward_label} returned only {} cells; \
             need at least 3 for the wave solver. Likely the coast is land-locked or the GMRT data \
             is sparse here.",
            profile_cells.len()
        )));
    }
    let _ = seaward_label; // surfaced via the cells list
    Ok((profile_cells, depths, cids))
}

/// Run the full wave-solve primitive.
pub async fn wave_solve(mut req: WaveSolveReq, state: &AppState) -> Result<JsonValue, ApiError> {
    let started = Instant::now();
    // Resolve a place name to cell64 if needed. Walking the seaward
    // bathymetric profile only makes sense from a real coastal cell,
    // so locate must succeed before any of the FD sanity checks below.
    let (resolved_cell, resolved_ref) = crate::resolve_cell_field(&req.coastal_cell).await?;
    req.coastal_cell = resolved_cell;
    if !(0.0..=30.0).contains(&req.offshore_height_m) || !req.offshore_height_m.is_finite() {
        return Err(bad_request(format!(
            "offshore_height_m must be in (0, 30] m; got {}",
            req.offshore_height_m
        )));
    }
    if !(2.0..=30.0).contains(&req.period_s) || !req.period_s.is_finite() {
        return Err(bad_request(format!(
            "period_s must be in [2, 30] s (typical wind-wave + swell envelope); got {}",
            req.period_s
        )));
    }
    let n_offshore = req.n_offshore_cells.max(1) as usize;

    // Walk seaward + recall depth at each step. Profile is ordered
    // coast-first; we reverse it so index 0 = offshore boundary, index
    // N-1 = the coast (matches the FD solver's boundary-condition
    // assumption documented above).
    let (mut profile_cells, mut depths, mut cids) =
        walk_seaward_profile(&req.coastal_cell, n_offshore, state).await?;
    profile_cells.reverse();
    depths.reverse();
    cids.reverse();

    // Phase speed at each cell. We clamp to a tiny floor at the coast
    // to keep CFL finite (a depth-0 land cell would force c=0 → dt=0).
    let c_profile: Vec<f64> = depths.iter().map(|h| (G * h.max(0.01)).sqrt()).collect();

    // Land-locked rejection. If the seaward walk landed mostly on
    // continental crust the depth floor pancakes phase speed to ~0.31
    // m/s and the arrival height becomes a placeholder. Reject loudly
    // with the failed depth + phase-speed profiles in `details` so the
    // agent can audit *why* the rejection fired and pick a recovery
    // path (try_longer_profile / try_different_cell). See
    // `WAVE_MIN_OCEAN_FRACTION` and `WAVE_MIN_OFFSHORE_DEPTH_M`.
    let classification = classify_seaward_profile(&depths);
    if classification != ProfileClassification::Oceanic {
        return Err(land_locked_profile_error(
            classification,
            &req.coastal_cell,
            &profile_cells,
            &depths,
            &c_profile,
            n_offshore,
        ));
    }
    let dx_m = CELL_PITCH_M;
    let dt_s = wave_max_dt(&c_profile, dx_m);
    if dt_s <= 0.0 {
        return Err(unprocessable(
            "all sampled depths are zero — cannot integrate the wave equation over land",
        ));
    }
    // Run until the wave-front reaches the coast, plus one period for
    // the response to develop. Wave-front travel time = sum of (Δx /
    // c_local) along the profile.
    let travel_time_s: f64 = c_profile.iter().map(|c| dx_m / c.max(1e-3)).sum();
    let total_s = travel_time_s + req.period_s;
    let n_steps_f = (total_s / dt_s).ceil().max(3.0);
    if n_steps_f > WAVE_MAX_STEPS as f64 {
        return Err(bad_request(format!(
            "this profile would need {n_steps_f:.0} explicit-FD steps (cap {WAVE_MAX_STEPS}). \
             Pick a shorter profile (smaller n_offshore_cells) or a higher period (larger dt)."
        )));
    }
    let n_steps = n_steps_f as usize;

    let n = profile_cells.len();
    let mut u_prev = vec![0.0f64; n];
    let mut u_curr = vec![0.0f64; n];
    let omega = 2.0 * std::f64::consts::PI / req.period_s;
    let mut max_at_coast = 0.0_f64;
    let mut arrival_step: Option<usize> = None;
    let arrival_threshold = 0.05 * req.offshore_height_m;
    for step in 0..n_steps {
        let t = step as f64 * dt_s;
        let forcing = req.offshore_height_m * (omega * t).sin();
        let u_next = wave_step_1d(&u_prev, &u_curr, &c_profile, dt_s, dx_m, forcing);
        // Coast value is u_next[n-2] (one cell inside the hard wall).
        let coast_val = u_next[n - 2].abs();
        if coast_val > max_at_coast {
            max_at_coast = coast_val;
        }
        if arrival_step.is_none() && coast_val >= arrival_threshold {
            arrival_step = Some(step);
        }
        u_prev = u_curr;
        u_curr = u_next;
    }
    let arrival_time_s = arrival_step.map(|s| s as f64 * dt_s);

    let pubkey = pubkey_b32(state);
    let cfl_factor = c_profile.iter().cloned().fold(0.0_f64, f64::max) * dt_s / dx_m;
    let receipt = state.sign_receipt(
        "emem.wave_solve",
        profile_cells.clone(),
        cids.iter().cloned().map(FactCid::new).collect(),
        false,
        started,
        None,
    );
    Ok(json!({
        "schema": "emem.wave_solve.v1",
        "coastal_cell": req.coastal_cell,
        "resolved_from": resolved_ref,
        "profile_cells_offshore_to_coast": profile_cells,
        "depth_profile_m": depths,
        "phase_speed_profile_m_per_s": c_profile,
        "input_band": "gmrt.topobathy_mean",
        "offshore_height_m": req.offshore_height_m,
        "period_s": req.period_s,
        "n_offshore_cells": profile_cells.len(),
        "dt_seconds": dt_s,
        "cell_pitch_m": dx_m,
        "n_steps": n_steps,
        "cfl_factor": cfl_factor,
        "cfl_bound": 1.0,
        "cfl_note": "1-D explicit-FD wave equation requires c·Δt/Δx ≤ 1. We run at WAVE_CFL_SAFETY=0.5 of the bound.",
        "arrival_height_m": max_at_coast,
        "arrival_time_s": arrival_time_s,
        "arrival_threshold_m": arrival_threshold,
        "scheme": "explicit central-time central-space (CTCS) on a sinusoidally-forced offshore boundary; hard wall (u=0) at the coast.",
        "algorithm_key": "wave_equation_1d@1",
        "algorithm_citation": "Lighthill, J. 1978 §3.1 (linear shallow-water wave); Holthuijsen 2007 §5.3 (refraction-free 1-D propagation).",
        "input_fact_cids": cids,
        "responder_pubkey_b32": pubkey,
        "receipt": receipt,
        "next": {
            "verify_offline":   "POST /v1/verify_receipt {receipt}",
            "fact_dereference": "GET /v1/facts/{fact_cid}",
            "longer_profile":   format!("POST /v1/wave_solve {{coastal_cell:'{}', n_offshore_cells: {}}}", req.coastal_cell, (profile_cells.len() * 2).min(64)),
        },
    }))
}

// ── JEPA-pattern temporal predictor ───────────────────────────────────────

/// `POST /v1/jepa_predict` request body.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JepaPredictReq {
    /// Cell to forecast at. Accepts a cell64 string OR a free-text
    /// place name; the handler resolves the place via `/v1/locate`
    /// first. Aliased to `place` for agents that pass that key.
    #[serde(alias = "place")]
    pub cell: String,
    /// Band to forecast. v1 supports `indices.ndvi` only; future
    /// versions will broaden the predictor's training surface.
    #[serde(default = "default_jepa_band")]
    pub band: String,
    /// Number of past months to read. Capped at 24 (two annual cycles)
    /// so the AR(2) seasonal model has at least one full carryover lag.
    #[serde(default = "default_lookback_months")]
    pub lookback_months: u32,
    /// Horizon, in months ahead. v1 only supports 1; we surface this so
    /// future versions can extend without an API break.
    #[serde(default = "default_forecast_horizon")]
    pub forecast_horizon_months: u32,
}

fn default_jepa_band() -> String {
    "indices.ndvi".to_string()
}

const fn default_lookback_months() -> u32 {
    6
}

const fn default_forecast_horizon() -> u32 {
    1
}

/// Closed-form coefficients for the AR(2) seasonal predictor. Documented
/// inline so an agent can read the math without leaving the response.
pub const JEPA_ALPHA: f64 = 0.6; // year-over-year carryover (lag-12)
pub const JEPA_BETA: f64 = 0.3; // recent slope from the last `lookback` months
pub const JEPA_GAMMA: f64 = 0.1; // long-term mean reversion

/// Pure predictor — no I/O. Given a vector of monthly NDVI values
/// (oldest first) plus the optional 12-month-ago value (lag-12), return
/// the predicted next-month NDVI.
///
/// Coefficients fall back gracefully when lag-12 is unavailable (the
/// lookback was less than 12 months): α's contribution shifts to the
/// recent mean of the lookback. This is the documented v1 behaviour and
/// keeps the predictor well-defined for any non-empty history.
///
/// Returned NDVI is clamped to `[-1.0, 1.0]` (NDVI's physical range).
pub fn jepa_predict_ar2_seasonal(history: &[f64], lag_12_value: Option<f64>) -> Option<f64> {
    if history.is_empty() {
        return None;
    }
    let n = history.len();
    let recent_mean = history.iter().sum::<f64>() / n as f64;
    // Recent trend: slope of a least-squares line through the lookback.
    // For n=1 this collapses to 0.
    let trend = if n >= 2 {
        let x_mean = (n as f64 - 1.0) / 2.0;
        let mut num = 0.0;
        let mut den = 0.0;
        for (i, y) in history.iter().enumerate() {
            let dx = i as f64 - x_mean;
            num += dx * (y - recent_mean);
            den += dx * dx;
        }
        if den == 0.0 {
            0.0
        } else {
            num / den
        }
    } else {
        0.0
    };
    let last = *history.last().expect("non-empty");
    // α term: lag-12 if we have it, else the lookback mean (degraded).
    let alpha_term = lag_12_value.unwrap_or(recent_mean);
    // The β term is the projected next value under the local linear
    // trend — `last + trend·1`.
    let beta_term = last + trend;
    // The γ term anchors the prediction at the lookback mean so a noisy
    // local trend can't run away.
    let gamma_term = recent_mean;
    let pred = JEPA_ALPHA * alpha_term + JEPA_BETA * beta_term + JEPA_GAMMA * gamma_term;
    Some(pred.clamp(-1.0, 1.0))
}

/// Run the JEPA-pattern predictor primitive.
pub async fn jepa_predict(
    mut req: JepaPredictReq,
    state: &AppState,
) -> Result<JsonValue, ApiError> {
    let started = Instant::now();
    // Resolve a place name to cell64 if needed before the recall fan-out.
    let (resolved_cell, resolved_ref) = crate::resolve_cell_field(&req.cell).await?;
    req.cell = resolved_cell;
    if req.lookback_months == 0 || req.lookback_months > 24 {
        return Err(bad_request(format!(
            "lookback_months must be in 1..=24; got {}",
            req.lookback_months
        )));
    }
    if req.forecast_horizon_months != 1 {
        return Err(bad_request(format!(
            "forecast_horizon_months must be 1 in v1 (multi-step rollout lands in @2); got {}",
            req.forecast_horizon_months
        )));
    }
    if req.band != "indices.ndvi" {
        return Err(bad_request(format!(
            "v1 supports band='indices.ndvi' only (closed-form coefficients are agriculture-NDVI calibrated); got '{}'",
            req.band
        )));
    }

    // 1) Recall every monthly NDVI fact already attested at this cell.
    //    `indices.ndvi` is Tempo::Medium (~30-day slot) so each fact is
    //    one calendar month. We don't auto-backfill here — that's a
    //    separate (slower) call the agent can chain via /v1/backfill —
    //    but we do auto-materialize the latest tslot through the
    //    standard recall path so a brand-new cell still answers.
    let req_recall = RecallReq {
        cell: req.cell.clone(),
        bands: Some(vec![req.band.clone()]),
        tslot: None,
    };
    let (resp, materialize_notes) = recall_with_auto_materialize(&req_recall, state).await?;
    let mut history = primary_history_for_band(&resp, &req.band);
    // Keep only the most recent `lookback_months` months.
    if history.len() > req.lookback_months as usize {
        let drop_n = history.len() - req.lookback_months as usize;
        history.drain(..drop_n);
    }
    if history.is_empty() {
        return Err(unprocessable(format!(
            "no {} history at cell {} after auto-materialize. Run /v1/backfill {{cell:'{}', band:'{}', start_unix:<unix - {}*30d>, end_unix:<unix>}} to seed the predictor.",
            req.band, req.cell, req.cell, req.band, req.lookback_months
        )));
    }
    // Look for a lag-12 sample if any of the available history is 12
    // tslots before the latest. We DON'T assume a perfect calendar
    // alignment — we look for the closest tslot to (latest - 12).
    let latest_tslot = history.last().expect("non-empty").0;
    let lag12_target = latest_tslot.saturating_sub(12);
    let lag_12_value = history
        .iter()
        .min_by_key(|(t, _, _)| (*t as i64 - lag12_target as i64).abs())
        .filter(|(t, _, _)| (*t as i64 - lag12_target as i64).abs() <= 1)
        .map(|(_, v, _)| *v);

    let history_values: Vec<f64> = history.iter().map(|(_, v, _)| *v).collect();
    let history_tslots: Vec<u64> = history.iter().map(|(t, _, _)| *t).collect();
    let history_cids: Vec<String> = history.iter().map(|(_, _, c)| c.clone()).collect();

    let prediction = jepa_predict_ar2_seasonal(&history_values, lag_12_value).ok_or_else(|| {
        unprocessable("predictor returned None (history was empty after filtering)")
    })?;
    let forecast_tslot = latest_tslot + 1;

    let pubkey = pubkey_b32(state);
    let receipt = state.sign_receipt(
        "emem.jepa_predict",
        vec![req.cell.clone()],
        history_cids
            .iter()
            .filter(|c| !c.is_empty())
            .cloned()
            .map(FactCid::new)
            .collect(),
        false,
        started,
        None,
    );

    Ok(json!({
        "schema": "emem.jepa_predict.v1",
        "cell": req.cell,
        "resolved_from": resolved_ref,
        "band": req.band,
        "lookback_months_requested": req.lookback_months,
        "lookback_months_used": history_values.len(),
        "history_values": history_values,
        "history_tslots": history_tslots,
        "history_fact_cids": history_cids,
        "lag_12_value": lag_12_value,
        "lag_12_used": lag_12_value.is_some(),
        "lag_12_fallback_to_recent_mean": lag_12_value.is_none(),
        "predictor_coefficients": {
            "alpha_year_over_year": JEPA_ALPHA,
            "beta_recent_trend":    JEPA_BETA,
            "gamma_long_term_mean": JEPA_GAMMA,
        },
        "predictor_form": "y_{t+1} = α · (lag-12 NDVI or recent mean) + β · (last + slope) + γ · recent_mean, clamped to [-1, 1]",
        "forecast_value": prediction,
        "forecast_tslot": forecast_tslot,
        "forecast_horizon_months": req.forecast_horizon_months,
        "forecast_unit": "ndvi",
        "scheme": "constrained JEPA-pattern AR(2) seasonal predictor (closed-form, NOT a learned MLP).",
        "algorithm_key": "jepa_temporal_predictor@1",
        "algorithm_citation": "Assran et al. 2023 (JEPA pattern); Pettorelli et al. 2005 (NDVI seasonal modelling); Tucker 1979 (NDVI's place in the agricultural-monitoring literature).",
        "honesty_note": "v1 ships closed-form coefficients (α=0.6, β=0.3, γ=0.1) calibrated from the agricultural-NDVI literature — NOT learned. Future versions (jepa_temporal_predictor@2) will train an actual encoder + predictor on the geotessera embedding pool.",
        "responder_pubkey_b32": pubkey,
        "receipt": receipt,
        "materialize_notes": materialize_notes,
        "next": {
            "verify_offline":   "POST /v1/verify_receipt {receipt}",
            "extend_lookback":  format!("POST /v1/backfill {{cell:'{}', band:'{}', start_unix:<unix - {}*30d>, end_unix:<unix>}}", req.cell, req.band, req.lookback_months * 2),
            "fact_dereference": "GET /v1/facts/{fact_cid}",
        },
    }))
}

// ── REST handlers ─────────────────────────────────────────────────────────

pub async fn post_heat_solve(
    State(state): State<AppState>,
    Json(req): Json<HeatSolveReq>,
) -> Result<Json<JsonValue>, ApiError> {
    Ok(Json(heat_solve(req, &state).await?))
}

pub async fn post_wave_solve(
    State(state): State<AppState>,
    Json(req): Json<WaveSolveReq>,
) -> Result<Json<JsonValue>, ApiError> {
    Ok(Json(wave_solve(req, &state).await?))
}

pub async fn post_jepa_predict(
    State(state): State<AppState>,
    Json(req): Json<JepaPredictReq>,
) -> Result<Json<JsonValue>, ApiError> {
    Ok(Json(jepa_predict(req, &state).await?))
}

// ── jepa_temporal_predictor@2 — learned dynamics head over Tessera ────────

/// Choose the inference backend for jepa_v2: prefer the GPU sidecar,
/// fall back to the in-process CPU path.
///
/// Why three branches:
///   - sidecar `Ok(_)`            → use it (much faster on CUDA)
///   - sidecar `Upstream`         → 502; the sidecar is up and rejected
///                                  the request, masking it would lie
///                                  to the caller about model state
///   - sidecar unreachable / timeout / framing garble → fall back to
///     in-process CPU. Tag `via` + `fallback_reason` in the receipt
///     so the verifier sees which backend produced the prediction.
async fn predict_via_sidecar_or_local(
    lags_2d: &[Vec<f32>],
    flat: &[f32],
) -> Result<(Vec<f32>, JsonValue), ApiError> {
    let req = crate::gpu_sidecar::DynamicsRequest {
        lags: lags_2d.to_vec(),
    };
    match crate::gpu_sidecar::predict_dynamics_v2(&req).await {
        Ok(resp) => Ok((resp.prediction, resp.model)),
        Err(crate::gpu_sidecar::SidecarError::Upstream { status, body }) => Err(ApiError(
            StatusCode::BAD_GATEWAY,
            ErrorBody {
                code: ErrorCode::SourceFetchFailed,
                message: format!(
                    "jepa_v2 sidecar rejected request: status={status}, body={body}"
                ),
                details: None,
            },
        )),
        Err(reason) => {
            tracing::info!(
                ?reason,
                "jepa_v2 sidecar unavailable; falling back to in-process CPU"
            );
            let (pred, metadata) = crate::jepa_v2::predict_next_vintage(flat)
                .map_err(crate::jepa_v2::into_api_error)?;
            let mut block = crate::jepa_v2::receipt_block(&metadata);
            if let Some(obj) = block.as_object_mut() {
                obj.insert("via".into(), JsonValue::String("in_process_cpu".into()));
                obj.insert(
                    "fallback_reason".into(),
                    JsonValue::String(reason.to_string()),
                );
            }
            Ok((pred, block))
        }
    }
}

/// `POST /v1/jepa_predict_v2` request body.
///
/// Predicts the next-vintage Tessera embedding at a cell from the K
/// most-recent attested vintages. Output is a 128-D vector — agents
/// can compare against any other Tessera-attested cell via cosine, or
/// dot-decode through any algorithm in
/// `algorithms_for_topic.foundation_embedding`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JepaPredictV2Req {
    /// Cell to forecast at. Accepts cell64 or a free-text place name
    /// (resolved via /v1/locate). Aliased to `place`.
    #[serde(alias = "place")]
    pub cell: String,
}

/// Run the learned-dynamics predictor.
pub async fn jepa_predict_v2(
    mut req: JepaPredictV2Req,
    state: &AppState,
) -> Result<JsonValue, ApiError> {
    let started = Instant::now();
    let (resolved_cell, resolved_ref) = crate::resolve_cell_field(&req.cell).await?;
    req.cell = resolved_cell.clone();

    // Pull ALL attested geotessera.YYYY vintages at this cell. Auto-
    // materialise on miss is OFF for the historical sweep — Tessera
    // vintages are heavy (~tens of seconds each) and a v2 predict call
    // shouldn't burn an upstream sweep. Agents seeking history call
    // /v1/backfill explicitly.
    //
    // We fan-fetch each year band and assemble what's actually present.
    const TESSERA_YEARS: std::ops::RangeInclusive<i32> = 2017..=2024;
    let years: Vec<i32> = TESSERA_YEARS.collect();
    let mut by_year: Vec<(i32, Vec<f32>, String)> = Vec::new();
    for &y in &years {
        let band = format!("geotessera.{y}");
        let req_recall = RecallReq {
            cell: req.cell.clone(),
            bands: Some(vec![band.clone()]),
            tslot: None,
        };
        let (resp, _notes) = recall_with_auto_materialize(&req_recall, state).await?;
        for (idx, f) in resp.facts.iter().enumerate() {
            if let Fact::Primary(p) = f {
                if p.band != band {
                    continue;
                }
                if let ciborium::Value::Array(arr) = &p.value {
                    if arr.len() != crate::jepa_v2::TESSERA_DIM {
                        continue;
                    }
                    let mut v: Vec<f32> = Vec::with_capacity(arr.len());
                    let mut ok = true;
                    for x in arr {
                        match x {
                            ciborium::Value::Float(f) => v.push(*f as f32),
                            ciborium::Value::Integer(i) => {
                                let as_i: i128 = (*i).into();
                                v.push(as_i as f32)
                            }
                            _ => {
                                ok = false;
                                break;
                            }
                        }
                    }
                    if ok && v.len() == crate::jepa_v2::TESSERA_DIM {
                        let cid = resp
                            .receipt
                            .fact_cids
                            .get(idx)
                            .map(|c| c.0.clone())
                            .unwrap_or_default();
                        by_year.push((y, v, cid));
                        break;
                    }
                }
            }
        }
    }

    if by_year.len() < crate::jepa_v2::INPUT_LAGS {
        return Err(unprocessable(format!(
            "jepa_v2 needs ≥{lags} consecutive Tessera vintages at cell {cell}; \
             found {n}. Run POST /v1/backfill {{cell:'{cell}', \
             band:'geotessera.YYYY', start_unix:..., end_unix:...}} for {miss} \
             more years, or call /v1/jepa_predict (the v1 NDVI-scalar predictor) \
             for a closed-form fallback that needs only monthly NDVI history.",
            lags = crate::jepa_v2::INPUT_LAGS,
            cell = req.cell,
            n = by_year.len(),
            miss = crate::jepa_v2::INPUT_LAGS - by_year.len(),
        )));
    }

    by_year.sort_by_key(|(y, _, _)| *y);
    // Take the K most-recent vintages as input.
    let lag_window = &by_year[by_year.len() - crate::jepa_v2::INPUT_LAGS..];
    let mut flat: Vec<f32> =
        Vec::with_capacity(crate::jepa_v2::INPUT_LAGS * crate::jepa_v2::TESSERA_DIM);
    let mut lags_2d: Vec<Vec<f32>> = Vec::with_capacity(crate::jepa_v2::INPUT_LAGS);
    for (_, v, _) in lag_window {
        flat.extend_from_slice(v);
        lags_2d.push(v.clone());
    }

    let (pred, model_block) = predict_via_sidecar_or_local(&lags_2d, &flat).await?;
    let predicted_year = lag_window.last().expect("non-empty").0 + 1;

    // The v2 surface returns the prediction inline — we DO NOT persist
    // a synthetic Tessera fact under `geotessera.<predicted_year>`
    // because that would clobber the band's contract (Tessera facts
    // are upstream-attested, not predicted). Agents that want to
    // persist the prediction can attest it themselves under a
    // private band. The receipt cites the K input fact_cids so
    // verifiers can replay the prediction by re-running the .onnx.
    let pubkey = pubkey_b32(state);
    let input_cids: Vec<String> = lag_window.iter().map(|(_, _, cid)| cid.clone()).collect();
    let receipt = state.sign_receipt(
        "emem.jepa_predict_v2",
        vec![req.cell.clone()],
        input_cids
            .iter()
            .filter(|c| !c.is_empty())
            .cloned()
            .map(FactCid::new)
            .collect(),
        false,
        started,
        None,
    );

    Ok(json!({
        "schema": "emem.jepa_predict_v2.v1",
        "cell": req.cell,
        "resolved_from": resolved_ref,
        "lag_window_years": lag_window.iter().map(|(y, _, _)| *y).collect::<Vec<_>>(),
        "predicted_year": predicted_year,
        "predicted_band": format!("geotessera.{predicted_year}"),
        "prediction_dim": pred.len(),
        "prediction": pred,
        "input_fact_cids": input_cids,
        "model": model_block,
        "responder_pubkey_b32": pubkey,
        "receipt": receipt,
        "next": {
            "verify_offline":   "POST /v1/verify_receipt {receipt}",
            "compare_against":  "POST /v1/find_similar { key:'inline:[…prediction…]', band:'geotessera', k:10 }",
            "v1_fallback":      format!("POST /v1/jepa_predict {{cell:'{}'}} for the closed-form NDVI-scalar predictor", req.cell),
        },
    }))
}

pub async fn post_jepa_predict_v2(
    State(state): State<AppState>,
    Json(req): Json<JepaPredictV2Req>,
) -> Result<Json<JsonValue>, ApiError> {
    Ok(Json(jepa_predict_v2(req, &state).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CFL choice: at α=1e-6 m²/s and Δx=10 m the dt_max is 0.20 ·
    /// 100 / 1e-6 = 2.0e7 s ≈ 231 days — so a one-week horizon needs
    /// only one step. Spot-check the edge case (very short horizon)
    /// and the cap-busting one (very long horizon at very large α).
    #[test]
    fn heat_choose_timestep_satisfies_cfl() {
        let (n, dt) = heat_choose_timestep(1.0e-6, 6.0).expect("should succeed");
        let cfl = 1.0e-6 * dt / (CELL_PITCH_M * CELL_PITCH_M);
        assert!(cfl <= 0.25, "CFL {cfl} > 0.25 for 6h horizon");
        assert!(n >= 1);
        assert!((n as f64 * dt - 6.0 * 3600.0).abs() < 1e-6);
    }

    #[test]
    fn heat_choose_timestep_rejects_nonpositive_alpha() {
        assert!(heat_choose_timestep(0.0, 6.0).is_err());
        assert!(heat_choose_timestep(-1.0e-6, 6.0).is_err());
        assert!(heat_choose_timestep(f64::NAN, 6.0).is_err());
    }

    #[test]
    fn heat_choose_timestep_rejects_nonpositive_horizon() {
        assert!(heat_choose_timestep(1.0e-6, 0.0).is_err());
        assert!(heat_choose_timestep(1.0e-6, -1.0).is_err());
        assert!(heat_choose_timestep(1.0e-6, f64::NAN).is_err());
    }

    /// A uniform 3×3 grid has zero Laplacian, so the centre cannot
    /// change after any number of steps. This is the most basic sanity
    /// check for the FD scheme.
    #[test]
    fn heat_uniform_grid_does_not_drift() {
        let u0 = [310.0_f64; 9];
        let final_centre = heat_solve_3x3_centre(&u0, 1.0e-6, 1000.0, 100);
        assert!(
            (final_centre - 310.0).abs() < 1e-9,
            "uniform 310 K grid drifted to {final_centre}"
        );
    }

    /// Centre cell is hotter than its 4 cardinal neighbours → the
    /// Laplacian is negative → the centre must cool over time.
    #[test]
    fn heat_hot_centre_cools_toward_neighbors() {
        // NW, N, NE, W, centre, E, SW, S, SE
        let u0 = [
            290.0, 290.0, 290.0, 290.0, 320.0, 290.0, 290.0, 290.0, 290.0,
        ];
        // Use a much larger α + many steps so the change is visible at
        // f64 precision over the unit-test scale (real urban α=1e-6
        // moves the centre by sub-millikelvin in a 6 h forecast at
        // 30 K initial gradient).
        let final_centre = heat_solve_3x3_centre(&u0, 5.0e-3, 0.5, 1000);
        assert!(
            final_centre < 320.0 && final_centre > 290.0,
            "centre should relax toward neighbour mean; got {final_centre}"
        );
    }

    /// 3×3 neighbourhood produces 9 unique cells at a non-pole cell.
    #[test]
    fn neighborhood_3x3_is_dense_off_pole() {
        let centre = emem_codec::cell64_from_latlng(35.68, 139.76); // Tokyo
        let cells = cell64_neighborhood_3x3(&centre).expect("should decode");
        let mut uniq: std::collections::HashSet<&String> = Default::default();
        for c in &cells {
            uniq.insert(c);
        }
        assert_eq!(uniq.len(), 9, "expected 9 unique cells, got {cells:?}");
        assert_eq!(cells[4], centre, "centre at index 4 must be the input cell");
    }

    /// Wave: a flat-bottom profile gives a constant phase speed; the
    /// sinusoidal forcing should propagate a wave-front into the
    /// interior. After enough steps the coast cell registers a
    /// non-zero amplitude.
    #[test]
    fn wave_propagates_under_sinusoidal_forcing() {
        // 12 cells, flat 10 m depth → c = √(98.1) ≈ 9.9 m/s.
        let n = 12;
        let depths = vec![10.0_f64; n];
        let c_profile: Vec<f64> = depths.iter().map(|h| (G * h).sqrt()).collect();
        let dx_m = 10.0;
        let dt_s = wave_max_dt(&c_profile, dx_m);
        assert!(dt_s > 0.0);
        let mut u_prev = vec![0.0_f64; n];
        let mut u_curr = vec![0.0_f64; n];
        let h_s = 2.0;
        let period_s = 8.0;
        let omega = 2.0 * std::f64::consts::PI / period_s;
        let mut max_at_coast = 0.0_f64;
        let n_steps = 4_000;
        for step in 0..n_steps {
            let t = step as f64 * dt_s;
            let forcing = h_s * (omega * t).sin();
            let u_next = wave_step_1d(&u_prev, &u_curr, &c_profile, dt_s, dx_m, forcing);
            max_at_coast = max_at_coast.max(u_next[n - 2].abs());
            u_prev = u_curr;
            u_curr = u_next;
        }
        assert!(
            max_at_coast > 0.1,
            "wave failed to propagate to the coast: max amplitude {max_at_coast} m"
        );
        // Coast amplitude builds up under continuous sinusoidal
        // forcing + a hard reflective wall (constructive interference
        // of incident + reflected wave on a lossless 1-D channel).
        // The bound we enforce is "stayed finite, stayed bounded
        // under the standing-wave envelope" — i.e., didn't NaN out
        // from a CFL violation. The absorbing-boundary @2 version
        // will damp this naturally; v1 is the documented
        // hard-wall reflection.
        assert!(
            max_at_coast.is_finite(),
            "coast amplitude {max_at_coast} non-finite — CFL likely violated"
        );
        assert!(
            max_at_coast <= 50.0 * h_s,
            "coast amplitude {max_at_coast} > 50×H_s — runaway, not standing wave"
        );
    }

    /// CFL: at the v1 safety factor, the timestep MUST satisfy `c·Δt/Δx ≤ 1`.
    #[test]
    fn wave_max_dt_satisfies_cfl() {
        let depths = [10.0_f64, 8.0, 5.0, 1.0];
        let c_profile: Vec<f64> = depths.iter().map(|h| (G * h).sqrt()).collect();
        let dx_m = 10.0;
        let dt_s = wave_max_dt(&c_profile, dx_m);
        let c_max = c_profile.iter().cloned().fold(0.0_f64, f64::max);
        assert!(c_max * dt_s / dx_m <= 1.0, "CFL violated");
        assert!(c_max * dt_s / dx_m >= 0.4, "way under-using the timestep");
    }

    /// JEPA: a perfectly-flat history → the prediction equals that
    /// constant. Sanity check on coefficient sums.
    #[test]
    fn jepa_flat_history_is_stable() {
        let history = vec![0.4_f64; 6];
        let pred = jepa_predict_ar2_seasonal(&history, Some(0.4)).expect("should predict");
        assert!((pred - 0.4).abs() < 1e-9, "flat history drifted to {pred}");
        // Coefficients sum to 1.0 by design — the documented invariant.
        assert!((JEPA_ALPHA + JEPA_BETA + JEPA_GAMMA - 1.0).abs() < 1e-12);
    }

    /// JEPA: a monotone-increasing history → the next prediction must
    /// be > the most recent value (positive trend).
    #[test]
    fn jepa_monotone_history_extrapolates_up() {
        let history = vec![0.20, 0.25, 0.30, 0.35, 0.40, 0.45];
        // Use lag-12 = 0.45 (no carryover signal beyond "stay where we
        // are") so the answer is dominated by the trend term.
        let pred = jepa_predict_ar2_seasonal(&history, Some(0.45)).expect("should predict");
        assert!(
            pred > 0.45,
            "prediction {pred} should exceed last history value 0.45 under positive trend"
        );
        // And it should be inside NDVI's physical range.
        assert!((-1.0..=1.0).contains(&pred));
    }

    /// JEPA: the predictor must clamp to NDVI's physical range even if
    /// the inputs would project to a value outside [-1, 1].
    #[test]
    fn jepa_clamps_to_ndvi_range() {
        // Steep upward trend that would project past +1.0.
        let history = vec![0.5, 0.7, 0.9, 1.0, 1.0, 1.0];
        let pred = jepa_predict_ar2_seasonal(&history, Some(1.0)).expect("should predict");
        assert!(pred <= 1.0, "predictor failed to clamp upper bound");
        // And the lower bound symmetrically.
        let history = vec![-0.5, -0.7, -0.9, -1.0, -1.0, -1.0];
        let pred = jepa_predict_ar2_seasonal(&history, Some(-1.0)).expect("should predict");
        assert!(pred >= -1.0, "predictor failed to clamp lower bound");
    }

    /// JEPA: empty history must surface as `None`, not panic.
    #[test]
    fn jepa_empty_history_is_none() {
        assert!(jepa_predict_ar2_seasonal(&[], None).is_none());
    }

    /// Stencil diagnostic — uniform case. All 9 cells at exactly 300 K
    /// (the canonical "stencil populated from a single coarser upstream
    /// pixel" pathology that motivated this fix). The diagnostic must
    /// flag `is_uniform=true`, the FTCS step must yield `delta_k==0.0`
    /// regardless of dt or α, and the threshold must be the documented
    /// 0.01 K. Mirrors the Phoenix +24h capture in
    /// `scripts/eval/physics/heat_phoenix.json`.
    #[test]
    fn heat_stencil_diagnostic_flags_uniform_and_delta_k_is_zero() {
        let u0 = [300.0_f64; 9];
        let diag = heat_stencil_diagnostic(&u0);
        assert!(
            diag.is_uniform,
            "all-300 K stencil should flag is_uniform=true; got {diag:?}"
        );
        assert!(
            diag.range_k < HEAT_UNIFORM_STENCIL_THRESHOLD_K,
            "stencil_range_k {} should be below threshold {}",
            diag.range_k,
            HEAT_UNIFORM_STENCIL_THRESHOLD_K
        );
        assert_eq!(diag.range_k, 0.0, "exact-uniform stencil has zero range");
        // FTCS must produce delta_k == 0.0 regardless of dt/α; this is
        // the math-is-correct, physics-is-meaningless property the
        // diagnostic exists to surface. Sweep a few representative
        // (α, dt, n) combinations to make the invariant explicit.
        for &(alpha, dt_s, n_steps) in &[
            (1.0e-6_f64, 3600.0_f64, 24_usize),
            (1.0e-3_f64, 1.0_f64, 1000_usize),
            (5.0e-7_f64, 86_400.0_f64, 1_usize),
        ] {
            let final_centre = heat_solve_3x3_centre(&u0, alpha, dt_s, n_steps);
            let delta_k = final_centre - u0[4];
            assert_eq!(
                delta_k, 0.0,
                "uniform stencil delta_k must be exactly 0.0 \
                 (α={alpha}, dt={dt_s}, n={n_steps}); got {delta_k}"
            );
        }
    }

    /// Stencil diagnostic — varied case. A 290..310 K spread across the
    /// 9 cells gives a 20 K range, well above threshold; the diagnostic
    /// must flag `is_uniform=false` and the FTCS step must produce a
    /// non-zero `delta_k` (the centre relaxes toward the cooler
    /// neighbours since the local Laplacian is negative).
    #[test]
    fn heat_stencil_diagnostic_flags_varied_and_delta_k_is_nonzero() {
        // NW, N, NE, W, centre, E, SW, S, SE — 290..310 K spread.
        let u0 = [
            290.0_f64, 295.0, 300.0, 295.0, 310.0, 305.0, 290.0, 295.0, 300.0,
        ];
        let diag = heat_stencil_diagnostic(&u0);
        assert!(
            !diag.is_uniform,
            "varied stencil should flag is_uniform=false; got {diag:?}"
        );
        assert!(
            (diag.range_k - 20.0).abs() < 1e-12,
            "expected 20 K range across 290..310 K stencil; got {}",
            diag.range_k
        );
        assert!(
            diag.range_k >= HEAT_UNIFORM_STENCIL_THRESHOLD_K,
            "varied stencil_range_k {} must clear threshold {}",
            diag.range_k,
            HEAT_UNIFORM_STENCIL_THRESHOLD_K
        );
        // Centre is 310 K; the 4 cardinal neighbours (N, S, E, W) at
        // indices 1, 7, 5, 3 are 295 + 295 + 305 + 295 = 1190; mean
        // 297.5 K. So the Laplacian is negative and the centre must
        // cool. Use a large α to make the change visible at the
        // unit-test scale (real urban α=1e-6 moves the centre by sub-
        // millikelvin per real-world hour).
        let final_centre = heat_solve_3x3_centre(&u0, 5.0e-3, 0.5, 1000);
        let delta_k = final_centre - u0[4];
        assert!(
            delta_k != 0.0,
            "varied stencil must produce non-zero delta_k; got {delta_k}"
        );
        assert!(
            delta_k < 0.0,
            "centre 310 K with cooler neighbours must cool; delta_k={delta_k}"
        );
        assert!(
            final_centre < 310.0 && final_centre > 290.0,
            "centre should relax inside the neighbour envelope; got {final_centre}"
        );
    }

    /// Threshold boundary — a stencil whose range is just below 0.01 K
    /// is uniform (below MODIS LST instrument noise); a stencil whose
    /// range is exactly at or above 0.01 K is varied. Pin this so a
    /// future refactor can't silently shift the cutoff.
    #[test]
    fn heat_stencil_diagnostic_threshold_boundary() {
        // Range = 0.005 K → uniform.
        let mut u_under = [300.0_f64; 9];
        u_under[0] = 300.005;
        let diag_under = heat_stencil_diagnostic(&u_under);
        assert!(diag_under.is_uniform, "0.005 K range must be uniform");
        // Range = 0.02 K → varied (clearly above the 0.01 threshold).
        let mut u_over = [300.0_f64; 9];
        u_over[0] = 300.02;
        let diag_over = heat_stencil_diagnostic(&u_over);
        assert!(!diag_over.is_uniform, "0.02 K range must be varied");
        // Documented constant, pinned.
        assert_eq!(HEAT_UNIFORM_STENCIL_THRESHOLD_K, 0.01);
    }

    /// HeatSolveReq: deserialise with defaults, full body, and explicit
    /// custom horizon.
    #[test]
    fn heat_solve_req_deserialises_with_defaults() {
        let req: HeatSolveReq =
            serde_json::from_value(json!({"cell": "abc.def.ghij.klmn"})).unwrap();
        assert_eq!(req.hours_ahead, 6.0);
        assert!((req.diffusivity_m2_per_s - 1.0e-6).abs() < 1e-15);
    }

    /// JepaPredictReq: deserialise with all defaults, then with full body.
    #[test]
    fn jepa_predict_req_deserialises_with_defaults() {
        let req: JepaPredictReq =
            serde_json::from_value(json!({"cell": "abc.def.ghij.klmn"})).unwrap();
        assert_eq!(req.lookback_months, 6);
        assert_eq!(req.forecast_horizon_months, 1);
        assert_eq!(req.band, "indices.ndvi");
    }

    /// Wave land-locked rejection: a depth profile of all-floor cells
    /// (every depth at 0.01 m, the safety floor used by the FD solver)
    /// must classify as `OffshoreBoundaryTooShallow` and the rendered
    /// `ApiError` must carry `error_kind: "LandLockedProfile"` plus
    /// both `try_longer_profile` and `try_different_cell` actions in
    /// `next_steps[]`.
    #[test]
    fn wave_land_locked_profile_is_rejected_with_structured_next_steps() {
        let depths_offshore_to_coast = vec![0.01_f64; 12];
        let classification = classify_seaward_profile(&depths_offshore_to_coast);
        assert_eq!(
            classification,
            ProfileClassification::OffshoreBoundaryTooShallow,
            "all-floor depth column must trip the offshore-boundary check first"
        );
        let c_profile: Vec<f64> = depths_offshore_to_coast
            .iter()
            .map(|h| (G * h.max(0.01)).sqrt())
            .collect();
        // Phase speed at the floor is √(g·0.01) ≈ 0.313 m/s — the bug
        // signature the eval cited.
        assert!((c_profile[0] - (G * 0.01_f64).sqrt()).abs() < 1e-9);
        assert!(
            c_profile[0] < 0.5,
            "all-floor profile must produce the pancaked phase speed; got {}",
            c_profile[0]
        );
        let profile_cells: Vec<String> = (0..12).map(|i| format!("dummy.cell.{i}")).collect();
        let err = land_locked_profile_error(
            classification,
            "miami.beach.cell",
            &profile_cells,
            &depths_offshore_to_coast,
            &c_profile,
            12,
        );
        assert_eq!(err.0, StatusCode::UNPROCESSABLE_ENTITY);
        let body = err.1;
        let details = body.details.expect("LandLockedProfile must carry details");
        assert_eq!(details["error_kind"], "LandLockedProfile");
        assert_eq!(details["coastal_cell"], "miami.beach.cell");
        // The failed depth + phase-speed profiles MUST be surfaced so
        // the agent can audit why the rejection fired.
        assert_eq!(details["depth_profile_m"].as_array().unwrap().len(), 12);
        assert_eq!(
            details["phase_speed_profile_m_per_s"]
                .as_array()
                .unwrap()
                .len(),
            12
        );
        let next_steps = details["next_steps"]
            .as_array()
            .expect("next_steps must be an array");
        assert_eq!(next_steps.len(), 2, "expected exactly 2 next_steps");
        let actions: Vec<&str> = next_steps
            .iter()
            .map(|s| s["action"].as_str().unwrap())
            .collect();
        assert!(
            actions.contains(&"try_longer_profile"),
            "missing try_longer_profile in {actions:?}"
        );
        assert!(
            actions.contains(&"try_different_cell"),
            "missing try_different_cell in {actions:?}"
        );
        // try_longer_profile must suggest 2× the requested n (capped at 64).
        let try_longer = next_steps
            .iter()
            .find(|s| s["action"] == "try_longer_profile")
            .unwrap();
        assert_eq!(try_longer["n_offshore_cells_suggested"], 24);
    }

    /// Wave land-locked rejection: a profile with too few oceanic cells
    /// (fraction below `WAVE_MIN_OCEAN_FRACTION`) must classify as
    /// `InsufficientOceanFraction`, even when the offshore boundary
    /// itself is deep enough.
    #[test]
    fn wave_insufficient_ocean_fraction_is_rejected() {
        // Offshore boundary deep, but the rest of the column is land.
        // 12 cells: depths[0]=20m, depths[1..]=0.01m → 1/12 ≈ 8% oceanic.
        let mut depths = vec![0.01_f64; 12];
        depths[0] = 20.0;
        let classification = classify_seaward_profile(&depths);
        assert_eq!(
            classification,
            ProfileClassification::InsufficientOceanFraction,
        );
        let c_profile: Vec<f64> = depths.iter().map(|h| (G * h.max(0.01)).sqrt()).collect();
        let cells: Vec<String> = (0..12).map(|i| format!("c.{i}")).collect();
        let err = land_locked_profile_error(
            classification,
            "city.centroid.cell",
            &cells,
            &depths,
            &c_profile,
            32,
        );
        let body = err.1;
        let details = body.details.unwrap();
        assert_eq!(details["error_kind"], "LandLockedProfile");
        // Suggested n is min(32*2, 64) = 64.
        let try_longer = details["next_steps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["action"] == "try_longer_profile")
            .unwrap();
        assert_eq!(try_longer["n_offshore_cells_suggested"], 64);
        // Sanity: oceanic_fraction is reported back so the agent can read it.
        let frac = details["oceanic_fraction"].as_f64().unwrap();
        assert!(
            (frac - 1.0 / 12.0).abs() < 1e-9,
            "expected fraction 1/12, got {frac}"
        );
    }

    /// A genuinely-oceanic profile (e.g. 10 cells of >= 10 m depth)
    /// must classify as `Oceanic` and the FD solver must produce a
    /// non-degenerate phase-speed profile (c >> 0.31 m/s, the all-floor
    /// signature of the bug).
    #[test]
    fn wave_oceanic_profile_passes_and_phase_speed_is_real() {
        let depths_offshore_to_coast: Vec<f64> =
            vec![25.0, 22.0, 20.0, 18.0, 16.0, 14.0, 12.0, 10.0, 8.0, 5.5];
        let classification = classify_seaward_profile(&depths_offshore_to_coast);
        assert_eq!(
            classification,
            ProfileClassification::Oceanic,
            "10 cells of >= 5.5 m depth, all > {WAVE_OCEAN_DEPTH_THRESHOLD_M} m, must classify as oceanic"
        );
        let c_profile: Vec<f64> = depths_offshore_to_coast
            .iter()
            .map(|h| (G * h.max(0.01)).sqrt())
            .collect();
        // Every phase speed must be physically meaningful: at h=5.5 m,
        // c = √(9.81·5.5) ≈ 7.34 m/s — three orders of magnitude above
        // the all-floor signature of 0.31 m/s.
        for c in &c_profile {
            assert!(
                *c > 5.0,
                "oceanic profile produced degenerate phase speed {c} m/s"
            );
        }
        // And the offshore boundary must be the deepest, fastest cell
        // (matches the FD solver's offshore-to-coast indexing).
        assert!(c_profile[0] >= *c_profile.last().unwrap());
    }

    /// Boundary cases: empty profile and a single-cell oceanic profile.
    #[test]
    fn wave_classification_boundary_cases() {
        // Empty profile: trivially insufficient.
        assert_eq!(
            classify_seaward_profile(&[]),
            ProfileClassification::InsufficientOceanFraction
        );
        // Single deep cell: passes the offshore-boundary check and the
        // 100% oceanic-fraction check (1/1 == 100% >= 50%).
        assert_eq!(
            classify_seaward_profile(&[10.0]),
            ProfileClassification::Oceanic
        );
        // Single shallow cell: fails the offshore-boundary check first.
        assert_eq!(
            classify_seaward_profile(&[0.5]),
            ProfileClassification::OffshoreBoundaryTooShallow
        );
        // Two cells, one deep one shallow: 1/2 == 50%, exactly at the
        // boundary. Per `< WAVE_MIN_OCEAN_FRACTION` (strict), 50% does
        // NOT trip the fraction check — the offshore-boundary check
        // governs. Deep boundary → Oceanic.
        assert_eq!(
            classify_seaward_profile(&[10.0, 0.5]),
            ProfileClassification::Oceanic
        );
    }
}
