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
    /// Bands that already have at least one signed fact at THIS cell —
    /// the answer to the agent's "what else can I read here without
    /// going through materialise?" question. The list is the union of
    /// every band attested at this cell64 across all tslots; it is NOT
    /// a list of globally wired connectors (that's `/v1/bands`). When
    /// a caller's filter matches zero facts, this lets them tell
    /// "wrong band name" (cell has data, just not for the requested
    /// band) apart from "this place is genuinely empty" (no facts at
    /// all). The wire field name was renamed from `bands_available`
    /// in the 2026-05-05 deepscan because the old name suggested
    /// global wiring; LLMs were reading it as "what bands does emem
    /// support" instead of "what bands have already been signed
    /// here". Renamed cleanly with no backwards-compat alias —
    /// callers that watched the misleading name should re-read this
    /// docstring before reaching for the new spelling.
    #[serde(rename = "bands_already_attested_at_cell")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bands_already_attested_at_cell: Option<Vec<String>>,
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
            let keys: Vec<CanonicalKey> = bands
                .iter()
                .map(|b| CanonicalKey {
                    cell: req.cell.clone(),
                    band: b.clone(),
                    tslot,
                })
                .collect();
            let cids = storage.lookup_canonical_many(&keys).await?;
            keys.into_iter()
                .zip(cids)
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
    let bands_already_attested_at_cell = {
        let all = storage.scan_cell(&req.cell, None).await.unwrap_or_default();
        let mut bands: Vec<String> = all.into_iter().map(|(k, _)| k.band).collect();
        bands.sort();
        bands.dedup();
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
    Ok(RecallResp {
        facts,
        receipt,
        bands_already_attested_at_cell,
    })
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    /// Mirror of the relevant `RecallResp` field — same `#[serde(rename)]`
    /// attribute, no Receipt or Fact dependency. The shape under test
    /// here is the wire field name only; the surrounding response
    /// structure is exercised through the live API tests.
    #[derive(Debug, Serialize, Deserialize)]
    struct RecallShape {
        #[serde(rename = "bands_already_attested_at_cell")]
        #[serde(skip_serializing_if = "Option::is_none")]
        bands_already_attested_at_cell: Option<Vec<String>>,
    }

    /// Wire-shape regression: the field that used to be
    /// `bands_available` (deepscan 2026-05-05: misleading name) MUST
    /// serialize as `bands_already_attested_at_cell` after the rename.
    /// If a future refactor accidentally re-introduces the old name,
    /// this test fails — agents and docs would silently drift.
    #[test]
    fn bands_field_serialises_as_bands_already_attested_at_cell() {
        let resp = RecallShape {
            bands_already_attested_at_cell: Some(vec!["indices.ndvi".into()]),
        };
        let v = serde_json::to_value(&resp).expect("serialises");
        assert!(
            v.get("bands_already_attested_at_cell").is_some(),
            "expected new field name on the wire; got: {v}"
        );
        assert!(
            v.get("bands_available").is_none(),
            "old `bands_available` name MUST be gone from the wire; got: {v}"
        );
    }

    /// Symmetric round-trip: deserializing the new field name lands
    /// back in `bands_already_attested_at_cell`. Confirms callers
    /// reading the new name see the data.
    #[test]
    fn bands_field_round_trips_under_new_name() {
        let v = serde_json::json!({
            "bands_already_attested_at_cell": ["a", "b"],
        });
        let resp: RecallShape = serde_json::from_value(v).expect("deserialises");
        assert_eq!(
            resp.bands_already_attested_at_cell.as_deref(),
            Some(&["a".to_string(), "b".to_string()][..])
        );
    }

    /// The old wire name MUST be rejected on input — re-introducing
    /// it as a serde alias would defeat the point of the rename
    /// (agents that learned the misleading name would keep working
    /// instead of being prompted to update). serde_json
    /// `from_value` returns None for a missing-but-optional field;
    /// a key under the OLD name should land that way (i.e. NOT
    /// silently mapped to the new field).
    #[test]
    fn old_name_does_not_alias_new_field() {
        let v = serde_json::json!({"bands_available": ["a", "b"]});
        let resp: RecallShape = serde_json::from_value(v).expect("deserialises with no field");
        assert!(
            resp.bands_already_attested_at_cell.is_none(),
            "the legacy `bands_available` key must NOT silently populate \
             the new field — keeping the alias would defeat the rename"
        );
    }
}
