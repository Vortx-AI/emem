//! `trajectory(cell, band, window)` — time series for a single (cell, band).
//!
//! Walks the canonical-index prefix scan for `cell`, filters to `band`, and
//! sorts by tslot. Inclusive `[start, end]` window.

use std::time::Instant;

use serde::{Deserialize, Serialize};

use emem_fact::{Fact, FactCid, Receipt};
use emem_storage::{Server, StorageError};

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
    /// Signed receipt.
    pub receipt: Receipt,
}

/// Build the time series.
pub async fn trajectory(req: &TrajectoryReq, srv: &Server) -> Result<TrajectoryResp, StorageError> {
    let started = Instant::now();
    let storage = srv.storage.as_ref();
    let [s, e] = req.window;

    let pairs = storage.scan_cell(&req.cell, None).await?;
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

    let mut series = Vec::with_capacity(filtered.len());
    for (idx, (tslot, _)) in filtered.iter().enumerate() {
        if let Some(Some(Fact::Primary(p))) = facts.get(idx) {
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

    let receipt = srv.sign_receipt(
        "emem.trajectory",
        vec![req.cell.clone()],
        cids,
        true,
        started,
        None,
    );
    Ok(TrajectoryResp {
        series,
        empty_series_diag,
        receipt,
    })
}
