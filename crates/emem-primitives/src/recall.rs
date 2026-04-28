//! `recall(cell, bands?, tslot?)` — spec §11 MCP tool `emem.recall`.

use std::time::Instant;

use serde::{Deserialize, Serialize};

use emem_cache::CanonicalKey;
use emem_fact::{Fact, FactCid, Receipt};
use emem_storage::{Server, StorageError};

/// Recall request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallReq {
    /// cell64 string. Accepts the alias `cell64` because that's the natural
    /// name agents reach for after reading the SPEC, and a wire mismatch
    /// here is the single most common first-call failure.
    #[serde(alias = "cell64")]
    pub cell: String,
    /// Optional band filter (defaults: all).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bands: Option<Vec<String>>,
    /// Optional time slot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tslot: Option<u64>,
}

/// Recall response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallResp {
    /// Returned facts.
    pub facts: Vec<Fact>,
    /// Signed receipt with cost.
    pub receipt: Receipt,
    /// Bands present on this cell *regardless* of the `bands` filter. When
    /// the caller's filter matches zero facts, this lets them distinguish
    /// "wrong band name" (cell has data, just not for the requested band)
    /// from "this place is genuinely empty" (no facts at all). Only
    /// populated when the request supplied `bands` and the result is
    /// empty — otherwise it would just duplicate the facts list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bands_available: Option<Vec<String>>,
}

/// Recall facts at a cell, optionally filtered by band and tslot.
///
/// - When `bands` is provided and `tslot` is provided, this is a
///   batched canonical lookup over `(cell, band_i, tslot)`.
/// - When `tslot` is set but `bands` is not, every band at the given
///   tslot is returned via prefix scan.
/// - When neither is set, every fact at the cell is returned.
pub async fn recall(req: &RecallReq, srv: &Server) -> Result<RecallResp, StorageError> {
    let started = Instant::now();
    let storage = srv.storage.as_ref();

    let pairs: Vec<(CanonicalKey, FactCid)> = match (&req.bands, req.tslot) {
        (Some(bands), Some(tslot)) => {
            let keys: Vec<CanonicalKey> = bands.iter().map(|b| CanonicalKey {
                cell: req.cell.clone(),
                band: b.clone(),
                tslot,
            }).collect();
            let cids = storage.lookup_canonical_many(&keys).await?;
            keys.into_iter().zip(cids.into_iter())
                .filter_map(|(k, c)| c.map(|cid| (k, cid)))
                .collect()
        }
        (None, t) => storage.scan_cell(&req.cell, t).await?,
        (Some(bands), None) => {
            let mut all = storage.scan_cell(&req.cell, None).await?;
            all.retain(|(k, _)| bands.iter().any(|b| b == &k.band));
            all
        }
    };

    let cids: Vec<FactCid> = pairs.iter().map(|(_, c)| c.clone()).collect();
    let fetched = storage.get_facts_many(&cids).await?;
    let facts: Vec<Fact> = fetched.into_iter().flatten().collect();

    // Always surface the full set of bands attested at this cell. The
    // agent uses this two ways:
    //
    //  1) Filtered recall returned zero hits → distinguishes "wrong band
    //     name" from "this place is empty" so the agent doesn't silently
    //     give up.
    //  2) Unfiltered recall returned facts → the agent learns which
    //     other bands exist here without a second probing call. Without
    //     this, an agent that called recall and got back {elevation,
    //     temperature} would have no idea NDVI / GeoTessera / land cover
    //     were also attested unless it guessed and asked.
    //
    // Cost: one extra `scan_cell` per recall, which is the same call we
    // already do under sled (point-in-tree scan, ~tens of microseconds).
    let bands_available = {
        let all = storage.scan_cell(&req.cell, None).await.unwrap_or_default();
        let mut bands: Vec<String> = all.into_iter().map(|(k, _)| k.band).collect();
        bands.sort(); bands.dedup();
        Some(bands)
    };

    let receipt = srv.sign_receipt(
        "emem.recall",
        vec![req.cell.clone()],
        cids,
        true,
        started,
        None,
    );
    Ok(RecallResp { facts, receipt, bands_available })
}
