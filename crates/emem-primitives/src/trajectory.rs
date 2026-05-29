//! `trajectory(cell, band, window)` — time series for a single (cell, band).
//!
//! Walks the canonical-index prefix scan for `cell`, filters to `band`, and
//! sorts by tslot. Inclusive `[start, end]` window.

use std::time::Instant;

use serde::{Deserialize, Serialize};

use emem_fact::{Fact, FactCid, Receipt, Scope};
use emem_storage::{Server, StorageError};

use crate::recall::{build_as_of_bound, TemporalAdvice};

/// trajectory request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryReq {
    /// cell64. `cell64` is accepted as an alias.
    #[serde(alias = "cell64")]
    pub cell: String,
    /// Band key.
    pub band: String,
    /// [start, end] inclusive tslot window.
    pub window: [u64; 2],
    /// Bi-temporal valid-time bound — clips the window's upper edge to
    /// `min(end, as_of_tslot)` and skips points with `tslot > as_of_tslot`.
    /// See [`crate::recall::RecallReq::as_of_tslot`] for semantics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_of_tslot: Option<u64>,
    /// Bi-temporal transaction-time bound — restricts the series to
    /// points whose attesting fact was signed at or before the given
    /// RFC 3339 instant. See [`crate::recall::RecallReq::as_of_signed_at`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_of_signed_at: Option<String>,
    /// Multi-tenant scope (v0.0.8). When at least one field is set the
    /// series is restricted to facts written under the same four-tuple
    /// (via the scope index) and the receipt binds the scope. Omit for
    /// the global series. See [`crate::recall::RecallReq::scope`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Scope>,
}

/// A single (tslot, value) point with its individual fact_cid so
/// every series sample can be verified, re-fetched by cid, or pasted
/// back as a memory_token. Without `fact_cid` per point the agent has
/// no way to cite an individual reading from a trajectory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Point {
    /// Time slot.
    pub tslot: u64,
    /// Band-typed value.
    pub value: ciborium::Value,
    /// Content-addressed cid of the underlying signed fact.
    pub fact_cid: String,
}

/// Diagnostic block emitted when the caller's window excluded all the
/// stored tslots — so the agent sees "wrong query" instead of "no
/// data". Avoids the silent-empty trap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmptySeriesDiag {
    /// Tslots the responder actually has for (cell, band), ignoring
    /// the window. Length is capped at 64 to keep the diagnostic light.
    pub stored_tslots: Vec<u64>,
    /// Echo of the caller's window for easy comparison.
    pub your_window: [u64; 2],
    /// Plain-text hint naming the most common root cause.
    pub hint: String,
}

/// trajectory response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryResp {
    /// Time series, ascending tslot.
    pub series: Vec<Point>,
    /// Present only when `series.is_empty()` and the responder holds
    /// at least one fact for (cell, band) outside the caller's window.
    /// Tells the agent "your window missed; here is what we actually
    /// have" so it can re-query without giving up.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub empty_series_diag: Option<EmptySeriesDiag>,
    /// Populated when the response is empty BECAUSE the bi-temporal
    /// `as_of_*` bound excluded every otherwise-recallable point — same
    /// contract as `recall.temporal_advice`. Distinct from
    /// `empty_series_diag` (which describes a wrong `window`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temporal_advice: Option<TemporalAdvice>,
    /// Signed receipt.
    pub receipt: Receipt,
}

/// Build the time series.
pub async fn trajectory(req: &TrajectoryReq, srv: &Server) -> Result<TrajectoryResp, StorageError> {
    let started = Instant::now();
    let storage = srv.storage.as_ref();
    let [s, e] = req.window;

    // trajectory has no explicit `tslot` field — it's a window query —
    // so the only conflict the bound can introduce is a malformed
    // signed_at. Reuse the shared validator.
    let bound = build_as_of_bound(None, req.as_of_tslot, req.as_of_signed_at.as_deref())?;

    let scope_filter: Option<&Scope> = req.scope.as_ref().filter(|s| !s.is_empty());
    let pairs = if let Some(_sc) = scope_filter {
        // Scoped: walk the scope index, then apply the valid-time half
        // of the bound in-process (the scope tree is keyed by tslot, so
        // `tslot <= as_of_tslot` is decidable from the key). The
        // transaction-time half is checked below via fact_passes.
        let mut s = storage
            .scan_cell_in_scope(&req.cell, None, scope_filter)
            .await?;
        if let Some(vt) = bound.valid_time {
            s.retain(|(k, _)| k.tslot <= vt);
        }
        s
    } else if bound.is_unbounded() {
        storage.scan_cell(&req.cell, None).await?
    } else {
        storage.scan_cell_as_of(&req.cell, None, &bound).await?
    };
    let all_band_pairs: Vec<(u64, FactCid)> = pairs
        .into_iter()
        .filter(|(k, _)| k.band == req.band)
        .map(|(k, c)| (k.tslot, c))
        .collect();
    let mut filtered: Vec<(u64, FactCid)> = all_band_pairs
        .iter()
        .filter(|(t, _)| *t >= s && *t <= e)
        .cloned()
        .collect();
    filtered.sort_by_key(|(t, _)| *t);

    let cids: Vec<FactCid> = filtered.iter().map(|(_, c)| c.clone()).collect();
    let facts = storage.get_facts_many(&cids).await?;

    // For the scoped path the scope index is keyed by valid-time only,
    // so a caller-set transaction-time bound is honoured here by checking
    // `fact_passes` on each loaded fact (the unscoped path already pushed
    // this into `scan_cell_as_of`).
    let scoped_tx_filter = scope_filter.is_some() && bound.transaction_time.is_some();
    let mut series = Vec::with_capacity(filtered.len());
    for (idx, (tslot, _)) in filtered.iter().enumerate() {
        if let Some(Some(Fact::Primary(p))) = facts.get(idx) {
            if scoped_tx_filter && !bound.fact_passes(&Fact::Primary(p.clone())) {
                continue;
            }
            series.push(Point {
                tslot: *tslot,
                value: p.value.clone(),
                fact_cid: cids[idx].0.clone(),
            });
        }
    }

    // Empty-series diagnostic: distinguishes "wrong window" from "no
    // data on this band at this cell". If we got nothing inside the
    // window but at least one stored tslot exists outside it, surface
    // the stored set so the caller can re-query.
    let empty_series_diag = if series.is_empty() && !all_band_pairs.is_empty() {
        let mut stored: Vec<u64> = all_band_pairs.iter().map(|(t, _)| *t).collect();
        stored.sort_unstable();
        stored.dedup();
        let truncated = stored.len() > 64;
        if truncated {
            stored.truncate(64);
        }
        let hint = if stored == [0] {
            format!(
                "band {} is stored at the snapshot sentinel tslot=0 (a static or static-vintage band). Your window [{s}, {e}] excludes 0. Either pass window:[0, …] or call POST /v1/recall with bands:[\"{}\"] (no tslot) to fetch the static fact.",
                req.band, req.band
            )
        } else {
            format!(
                "{} stored tslot(s) for band {}; none fall inside your window [{s}, {e}]. Widen the window or pick a tslot from `stored_tslots[]` above.{}",
                stored.len(),
                req.band,
                if truncated { " (list truncated to 64)" } else { "" }
            )
        };
        Some(EmptySeriesDiag {
            stored_tslots: stored,
            your_window: [s, e],
            hint,
        })
    } else {
        None
    };

    // Temporal advice: surfaced when the response is empty AND there
    // are facts at this (cell, band) outside the bi-temporal bound that
    // would have answered without it.
    let temporal_advice = if series.is_empty() && !bound.is_unbounded() {
        let unbounded = storage
            .scan_cell_in_scope(&req.cell, None, scope_filter)
            .await
            .unwrap_or_default();
        let unbounded_count = unbounded.iter().filter(|(k, _)| k.band == req.band).count();
        if unbounded_count > 0 {
            Some(TemporalAdvice {
                as_of_tslot: bound.valid_time,
                as_of_signed_at: bound.transaction_time.clone(),
                facts_at_cell_unbounded: unbounded_count,
                hint: format!(
                    "(cell, band={}) has {unbounded_count} attested fact(s) without the as_of bound; \
                     all of them were filtered by your as_of_tslot/as_of_signed_at. \
                     Either drop the bound or widen it.",
                    req.band
                ),
            })
        } else {
            None
        }
    } else {
        None
    };

    let receipt = srv.sign_receipt_full(
        "emem.trajectory",
        vec![req.cell.clone()],
        cids,
        true,
        started,
        None,
        req.scope.clone(),
        &bound,
    );
    Ok(TrajectoryResp {
        series,
        empty_series_diag,
        temporal_advice,
        receipt,
    })
}
