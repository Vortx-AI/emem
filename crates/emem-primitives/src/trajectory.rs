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

/// A single (tslot, value) point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Point {
    /// Time slot.
    pub tslot: u64,
    /// Band-typed value.
    pub value: ciborium::Value,
}

/// trajectory response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryResp {
    /// Time series, ascending tslot.
    pub series: Vec<Point>,
    /// Signed receipt.
    pub receipt: Receipt,
}

/// Build the time series.
pub async fn trajectory(req: &TrajectoryReq, srv: &Server) -> Result<TrajectoryResp, StorageError> {
    let started = Instant::now();
    let storage = srv.storage.as_ref();
    let [s, e] = req.window;

    let pairs = storage.scan_cell(&req.cell, None).await?;
    let mut filtered: Vec<(u64, FactCid)> = pairs
        .into_iter()
        .filter(|(k, _)| k.band == req.band && k.tslot >= s && k.tslot <= e)
        .map(|(k, c)| (k.tslot, c))
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
            });
        }
    }

    let receipt = srv.sign_receipt(
        "emem.trajectory",
        vec![req.cell.clone()],
        cids,
        true,
        started,
        None,
    );
    Ok(TrajectoryResp { series, receipt })
}
