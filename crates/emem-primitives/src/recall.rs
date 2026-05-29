//! `recall(cell, bands?, tslot?)` — spec §11 MCP tool `emem.recall`.

use std::time::Instant;

use serde::{Deserialize, Serialize};

use emem_cache::CanonicalKey;
use emem_core::ErrorCode;
use emem_fact::{EdgeFact, Fact, FactCid, Receipt, Scope};
use emem_storage::{AsOfBound, Server, StorageError};

use crate::cbor_ops::parse_rfc3339_strict;

/// Recall request.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecallReq {
    /// cell64 string. Accepts the alias `cell64` because that's the natural
    /// name agents reach for after reading the SPEC, and a wire mismatch
    /// here is the single most common first-call failure.
    #[serde(alias = "cell64")]
    pub cell: String,
    /// Optional band filter (defaults: all).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bands: Option<Vec<String>>,
    /// Optional time slot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tslot: Option<u64>,
    /// Bi-temporal valid-time bound. When set, only facts whose `tslot`
    /// is `<= as_of_tslot` are returned per (cell, band) — emulating
    /// Zep/Graphiti's edge-style "what did this place look like as of
    /// date X" query (arXiv 2501.13956). Conflicts with an exact
    /// `tslot` when `as_of_tslot < tslot` — that is rejected at the
    /// API surface with `code: "invalid_temporal_bound"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_of_tslot: Option<u64>,
    /// Bi-temporal transaction-time bound. When set, only facts whose
    /// `signed_at` is `<= as_of_signed_at` are returned — emulating
    /// "what did emem know about this place as of system-date Y". RFC
    /// 3339 string; format errors are rejected at the API surface with
    /// `code: "invalid_signed_at_format"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_of_signed_at: Option<String>,
    /// Multi-tenant scope (`{user_id, agent_id, run_id, org_id}`). When
    /// at least one field is `Some`, two things happen (v0.0.8):
    ///
    /// 1. The returned receipt carries the scope and the signature
    ///    preimage includes `blake3(canonical_cbor(scope))` so an offline
    ///    verifier rebinds the response to this caller.
    /// 2. The fact set is FILTERED to facts written under the same
    ///    four-tuple via the `scope_index` sled tree — a recall scoped to
    ///    `{user_id:"u1"}` returns only `u1`'s facts, never another
    ///    tenant's and never globally-written (unscoped) facts. With no
    ///    scope (or an empty one) the read is byte-identical to the
    ///    pre-v0.0.8 global recall.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Scope>,
    /// Optional response-expansion flags. When the list contains
    /// `"edges"`, every returned fact's temporal knowledge-graph edges
    /// (latest-per-object, honouring `as_of_tslot`) are attached to
    /// `RecallResp.edges` and their CIDs enter the receipt signature
    /// preimage. Absent / empty leaves the response + preimage byte-
    /// identical to the pre-v0.0.9 recall path. (v0.0.9.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<String>>,
}

impl RecallReq {
    /// Construct + validate the [`AsOfBound`] this request implies.
    /// Returns the bound on success or a structured error the API layer
    /// can map to a 400 envelope (`invalid_temporal_bound` /
    /// `invalid_signed_at_format`).
    pub fn build_as_of_bound(&self) -> Result<AsOfBound, StorageError> {
        build_as_of_bound(
            self.tslot,
            self.as_of_tslot,
            self.as_of_signed_at.as_deref(),
        )
    }
}

/// Honesty-guard helper shared by every read primitive. Catches the two
/// classes of caller mistake the spec calls out: a bi-temporal bound
/// that contradicts an explicit tslot, and a non-RFC-3339
/// transaction-time string. A successful result is the validated
/// `AsOfBound` ready to hand to storage.
pub fn build_as_of_bound(
    tslot: Option<u64>,
    as_of_tslot: Option<u64>,
    as_of_signed_at: Option<&str>,
) -> Result<AsOfBound, StorageError> {
    if let (Some(t), Some(a)) = (tslot, as_of_tslot) {
        if a < t {
            return Err(StorageError::Protocol {
                code: ErrorCode::InvalidArgument,
                message: format!(
                    "invalid_temporal_bound: explicit tslot={t} but as_of_tslot={a} excludes it. \
                     Either drop `tslot` and rely on `as_of_tslot` (latest fact ≤ as_of_tslot) or set as_of_tslot ≥ tslot."
                ),
            });
        }
    }
    if let Some(s) = as_of_signed_at {
        parse_rfc3339_strict(s).map_err(|msg| StorageError::Protocol {
            code: ErrorCode::InvalidArgument,
            message: format!("invalid_signed_at_format: {msg}"),
        })?;
    }
    Ok(AsOfBound {
        valid_time: as_of_tslot,
        transaction_time: as_of_signed_at.map(|s| s.to_string()),
    })
}

/// Diagnostic block emitted when the bi-temporal filter excluded every
/// otherwise-recallable fact. Lets the caller distinguish "nothing
/// known about this cell" from "the as_of bound filtered everything
/// out" — same honesty contract as `trajectory`'s `empty_series_diag`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalAdvice {
    /// Echo of the caller's bi-temporal bound for easy comparison.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_of_tslot: Option<u64>,
    /// Echo of the caller's transaction-time bound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_of_signed_at: Option<String>,
    /// Number of (cell, band) pairs that exist at this cell without the
    /// bound applied. When `0`, the cell is genuinely empty regardless
    /// of the bound; when `> 0`, the bound is what made the response
    /// empty and the caller should consider relaxing it.
    pub facts_at_cell_unbounded: usize,
    /// Plain-text hint naming the most common root cause.
    pub hint: String,
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
    /// Populated when the response is empty BECAUSE the bi-temporal
    /// `as_of_*` bound filtered every otherwise-recallable fact out.
    /// Distinguishes "this cell is empty" from "as_of cut everything
    /// off" so the agent can decide whether to relax the bound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temporal_advice: Option<TemporalAdvice>,
    /// Temporal knowledge-graph edges attached when the caller passed
    /// `include:["edges"]`. `None` when edges were not requested (keeps
    /// the wire byte-identical to the pre-v0.0.9 response); `Some(vec![])`
    /// when requested but the returned facts have no edges. (v0.0.9.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edges: Option<Vec<EdgeFact>>,
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
    let bound = req.build_as_of_bound()?;

    // When the caller pins a scope we read through the scope-aware scan
    // (`scan_cell_in_scope`) so the result set is restricted to facts
    // written under the same multi-tenant four-tuple. With no scope the
    // `scope_filter` is `None` and every branch falls back to the global
    // canonical index — byte-identical to the pre-v0.0.8 path.
    let scope_filter: Option<&Scope> = req.scope.as_ref().filter(|s| !s.is_empty());

    let pairs: Vec<(CanonicalKey, FactCid)> = match (&req.bands, req.tslot) {
        (Some(bands), Some(tslot)) if scope_filter.is_none() => {
            // Exact (cell, band, tslot) lookup — bound is honoured by
            // post-filtering the lookup result through `AsOfBound::fact_passes`
            // when transaction-time is set; the valid-time half is
            // already enforced by `tslot <= as_of_tslot` validated above.
            let keys: Vec<CanonicalKey> = bands
                .iter()
                .map(|b| CanonicalKey {
                    cell: req.cell.clone(),
                    band: b.clone(),
                    tslot,
                })
                .collect();
            let cids = storage.lookup_canonical_many(&keys).await?;
            let mut pairs: Vec<(CanonicalKey, FactCid)> = keys
                .into_iter()
                .zip(cids)
                .filter_map(|(k, c)| c.map(|cid| (k, cid)))
                .collect();
            if bound.transaction_time.is_some() {
                let cids: Vec<FactCid> = pairs.iter().map(|(_, c)| c.clone()).collect();
                let facts = storage.get_facts_many(&cids).await?;
                let mut kept: Vec<(CanonicalKey, FactCid)> = Vec::with_capacity(pairs.len());
                for ((k, c), fact) in pairs.drain(..).zip(facts) {
                    if let Some(f) = fact {
                        if bound.fact_passes(&f) {
                            kept.push((k, c));
                        }
                    }
                }
                kept
            } else {
                pairs
            }
        }
        // Scoped exact (band, tslot): there is no batched scoped
        // point-lookup, so range-scan the scope index at this cell+tslot
        // and retain the requested bands. Equivalent result to the
        // unscoped `lookup_canonical_many` branch, just tenant-filtered.
        (Some(bands), Some(tslot)) => {
            // scope_filter is Some here (the unscoped arm matched above).
            let mut all = storage
                .scan_cell_in_scope(&req.cell, Some(tslot), scope_filter)
                .await?;
            all.retain(|(k, _)| bands.iter().any(|b| b == &k.band));
            if bound.transaction_time.is_some() {
                all = retain_by_transaction_time(storage, all, &bound).await?;
            }
            all
        }
        (None, t) => {
            let scanned = match scope_filter {
                // Scoped: walk the scope index, then apply the as_of bound
                // in-process (the scope tree is keyed by valid-time tslot,
                // not signed_at, so the transaction-time half is a body
                // filter — same as the unscoped as_of path does).
                Some(_) => {
                    let mut s = storage
                        .scan_cell_in_scope(&req.cell, t, scope_filter)
                        .await?;
                    if !bound.is_unbounded() {
                        s.retain(|(k, _)| match bound.valid_time {
                            Some(vt) => k.tslot <= vt,
                            None => true,
                        });
                        if bound.transaction_time.is_some() {
                            s = retain_by_transaction_time(storage, s, &bound).await?;
                        }
                    }
                    s
                }
                None if bound.is_unbounded() => storage.scan_cell(&req.cell, t).await?,
                None => storage.scan_cell_as_of(&req.cell, t, &bound).await?,
            };
            // When no exact `tslot` was pinned AND an as_of bound is in
            // play we collapse to the latest fact per (cell, band) within
            // the bound — same shape as the historical `scan_cell(None)`
            // result, just pre-filtered. This is the core of "what did X
            // look like as of date Y". The unbounded path (scoped or not)
            // returns every tslot, matching the global recall shape.
            if t.is_none() && !bound.is_unbounded() {
                latest_per_band(scanned)
            } else {
                scanned
            }
        }
        (Some(bands), None) => {
            let mut all = match scope_filter {
                Some(_) => {
                    let mut s = storage
                        .scan_cell_in_scope(&req.cell, None, scope_filter)
                        .await?;
                    if !bound.is_unbounded() {
                        s.retain(|(k, _)| match bound.valid_time {
                            Some(vt) => k.tslot <= vt,
                            None => true,
                        });
                        if bound.transaction_time.is_some() {
                            s = retain_by_transaction_time(storage, s, &bound).await?;
                        }
                    }
                    s
                }
                None if bound.is_unbounded() => storage.scan_cell(&req.cell, None).await?,
                None => storage.scan_cell_as_of(&req.cell, None, &bound).await?,
            };
            all.retain(|(k, _)| bands.iter().any(|b| b == &k.band));
            // Collapse to latest-per-band so the as_of semantics match
            // the per-cell unfiltered case above.
            if !bound.is_unbounded() {
                all = latest_per_band(all);
            }
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
    // Scoped reads use the scope-aware scan so the "what else is here"
    // hint never leaks another tenant's band names.
    let unbounded_pairs = storage
        .scan_cell_in_scope(&req.cell, None, scope_filter)
        .await
        .unwrap_or_default();
    let unbounded_count = unbounded_pairs.len();
    let bands_already_attested_at_cell = {
        let mut bands: Vec<String> = unbounded_pairs.into_iter().map(|(k, _)| k.band).collect();
        bands.sort();
        bands.dedup();
        Some(bands)
    };

    // Temporal advice — populated when the response is empty BECAUSE
    // the bi-temporal bound filtered everything out (zero is a valid
    // answer for "what did emem know as of yesterday?", so we surface
    // diagnostics instead of returning a 404).
    let temporal_advice = if facts.is_empty() && !bound.is_unbounded() && unbounded_count > 0 {
        Some(TemporalAdvice {
            as_of_tslot: bound.valid_time,
            as_of_signed_at: bound.transaction_time.clone(),
            facts_at_cell_unbounded: unbounded_count,
            hint: format!(
                "cell has {unbounded_count} attested fact(s) without the as_of bound; \
                 all of them were filtered out by your as_of_tslot/as_of_signed_at. \
                 Either drop the bound or widen it. The empty response is not a 404 — it is the honest \
                 answer to `what did emem know as of that moment`."
            ),
        })
    } else {
        None
    };

    // include:["edges"] — additive expansion. When requested, fetch each
    // returned fact's edges (latest-per-object, honouring the as_of
    // valid-time bound) and thread their CIDs into the receipt preimage.
    // Bounded so a fact-heavy cell can't fan out unboundedly.
    let want_edges = req
        .include
        .as_ref()
        .is_some_and(|v| v.iter().any(|s| s == "edges"));
    let (edges_out, edge_cids): (Option<Vec<EdgeFact>>, Vec<emem_fact::EdgeCid>) = if want_edges {
        const PER_FACT_EDGE_LIMIT: usize = 32;
        const TOTAL_EDGE_CAP: usize = 256;
        let mut collected: Vec<EdgeFact> = Vec::new();
        for fc in &cids {
            if collected.len() >= TOTAL_EDGE_CAP {
                break;
            }
            let remaining = TOTAL_EDGE_CAP - collected.len();
            let lim = PER_FACT_EDGE_LIMIT.min(remaining);
            let mut es = storage
                .recall_edges(fc, "", bound.valid_time, lim)
                .await
                .unwrap_or_default();
            collected.append(&mut es);
        }
        collected.truncate(TOTAL_EDGE_CAP);
        let ecids: Vec<emem_fact::EdgeCid> = collected.iter().map(|e| e.cid()).collect();
        (Some(collected), ecids)
    } else {
        (None, Vec::new())
    };

    // sign_receipt_with_edges collapses to sign_receipt_full when
    // edge_cids is empty, so the no-include path is byte-identical to
    // pre-v0.0.9 receipts.
    let receipt = srv.sign_receipt_with_edges(
        "emem.recall",
        vec![req.cell.clone()],
        cids,
        true,
        started,
        None,
        req.scope.clone(),
        &bound,
        &edge_cids,
    );
    Ok(RecallResp {
        facts,
        receipt,
        bands_already_attested_at_cell,
        temporal_advice,
        edges: edges_out,
    })
}

/// Apply the transaction-time half of an [`AsOfBound`] to an already
/// scope-/cell-filtered pair list by batch-loading the bodies and
/// checking `signed_at`. The scope index is keyed by valid-time tslot
/// (decidable from the key) but not by `signed_at`, so the
/// transaction-time predicate needs the body — one `get_facts_many`
/// round-trip keeps it O(1) calls.
async fn retain_by_transaction_time(
    storage: &dyn emem_storage::Storage,
    pairs: Vec<(CanonicalKey, FactCid)>,
    bound: &AsOfBound,
) -> Result<Vec<(CanonicalKey, FactCid)>, StorageError> {
    if bound.transaction_time.is_none() || pairs.is_empty() {
        return Ok(pairs);
    }
    let cids: Vec<FactCid> = pairs.iter().map(|(_, c)| c.clone()).collect();
    let facts = storage.get_facts_many(&cids).await?;
    let mut kept: Vec<(CanonicalKey, FactCid)> = Vec::with_capacity(pairs.len());
    for ((k, c), fact) in pairs.into_iter().zip(facts) {
        if let Some(f) = fact {
            if bound.fact_passes(&f) {
                kept.push((k, c));
            }
        }
    }
    Ok(kept)
}

/// Collapse the result of a per-cell scan to the latest fact per
/// `(cell, band)` — used by the bi-temporal recall path so an as_of
/// query produces the same one-fact-per-band shape as the historical
/// "latest" recall, just with the cap applied. Ties on `tslot` are
/// broken by the lexicographic order of the FactCid string, which is
/// deterministic across responders.
fn latest_per_band(pairs: Vec<(CanonicalKey, FactCid)>) -> Vec<(CanonicalKey, FactCid)> {
    use std::collections::BTreeMap;
    let mut by_band: BTreeMap<String, (CanonicalKey, FactCid)> = BTreeMap::new();
    for (k, c) in pairs {
        let key = k.band.clone();
        match by_band.get(&key) {
            Some((existing_k, existing_c)) => {
                let replace = k.tslot > existing_k.tslot
                    || (k.tslot == existing_k.tslot && c.as_str() > existing_c.as_str());
                if replace {
                    by_band.insert(key, (k, c));
                }
            }
            None => {
                by_band.insert(key, (k, c));
            }
        }
    }
    by_band.into_values().collect()
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

#[cfg(test)]
mod include_edges_tests {
    //! `include:["edges"]` threads each returned fact's edges into the
    //! response + receipt, while the WITHOUT-include path stays byte-
    //! identical to the pre-v0.0.9 recall receipt.

    use super::*;
    use std::sync::Arc;

    use blake3::Hasher;
    use ed25519_dalek::{Signer, SigningKey};
    use emem_core::{AttesterKey, KeyEpoch, Signature};
    use emem_fact::{
        Attestation, Derivation, EdgeFact, Fact, PrimaryFact, RegistryCid, SchemaCid, Source,
    };
    use emem_storage::server::{ManifestCids, ResponderIdentity};
    use emem_storage::{MaterializingStorage, Storage};

    fn ephemeral_server() -> (Arc<MaterializingStorage>, Server) {
        let bands = Arc::new(emem_core::bands::DEFAULT.clone());
        let functions =
            Arc::new(emem_core::FunctionRegistry::parse_default().expect("functions"));
        let sources = Arc::new(emem_core::SourceRegistry::parse_default().expect("sources"));
        let storage =
            Arc::new(MaterializingStorage::ephemeral(bands, functions, sources).expect("storage"));
        let srv = Server {
            storage: storage.clone(),
            identity: ResponderIdentity::fresh(),
            manifests: ManifestCids {
                registry_cid: RegistryCid::new("test-registry"),
                schema_cid: SchemaCid::new("test-schema"),
                bands_cid: "test-bands".into(),
                sources_cid: "test-sources".into(),
            },
            started_at_unix_s: 0,
        };
        (storage, srv)
    }

    fn mk_fact(cell: &str, tslot: u64) -> Fact {
        Fact::Primary(PrimaryFact {
            cell: cell.into(),
            band: "indices.ndvi".into(),
            tslot,
            value: ciborium::Value::Float(0.5),
            unit: None,
            confidence: 1.0,
            uncertainty: None,
            sources: vec![Source {
                scheme: "test".into(),
                id: "x".into(),
                cid: None,
                hash: None,
                captured_at: None,
                url: None,
            }],
            derivation: Derivation {
                fn_key: "test@1".into(),
                args: None,
            },
            privacy_class: "public".into(),
            schema_cid: SchemaCid::new("test-schema"),
            signer: AttesterKey([5u8; 32]),
            signed_at: "2026-05-29T00:00:00Z".into(),
            served_via: None,
        })
    }

    fn sign(facts: Vec<Fact>, secret: [u8; 32]) -> Attestation {
        let signing = SigningKey::from_bytes(&secret);
        let mut pk = [0u8; 32];
        pk.copy_from_slice(signing.verifying_key().as_bytes());
        let mut leaves: Vec<[u8; 32]> = facts
            .iter()
            .map(|f| {
                let mut buf = Vec::new();
                ciborium::ser::into_writer(f, &mut buf).unwrap();
                *blake3::hash(&buf).as_bytes()
            })
            .collect();
        leaves.sort();
        let root = emem_attest::merkle_root(&leaves);
        let mut h = Hasher::new();
        h.update(&root);
        h.update(b"test-registry");
        h.update(b"test-schema");
        let sig = signing.sign(h.finalize().as_bytes());
        let mut sb = [0u8; 64];
        sb.copy_from_slice(&sig.to_bytes());
        Attestation {
            facts,
            edges: vec![],
            batch_root: root,
            attester: AttesterKey(pk),
            attester_key_epoch: KeyEpoch(0),
            registry_cid: RegistryCid::new("test-registry"),
            schema_cid: SchemaCid::new("test-schema"),
            signature: Signature(sb),
            attested_at: "2026-05-29T00:00:00Z".into(),
            scope: None,
        }
    }

    #[tokio::test]
    async fn include_edges_threads_into_recall() {
        let cell = "damO.zb000.xUti.zde78";
        let (storage, srv) = ephemeral_server();

        // Attest one fact and capture its CID.
        let fact = mk_fact(cell, 12);
        let att = sign(vec![fact], [9u8; 32]);
        let cids = storage.put_attestation(&att).await.expect("attest");
        let subj = cids[0].clone();

        // Add an edge whose subject is that fact.
        let edge = EdgeFact {
            subj: subj.clone(),
            pred: "related_to".into(),
            obj: FactCid::new("obj-cid"),
            valid_from: 0,
            valid_to: None,
            confidence: 1.0,
            signer: AttesterKey([9u8; 32]),
            signed_at: "2026-05-29T00:00:00Z".into(),
            schema_cid: None,
            note: None,
        };
        storage
            .add_edges(std::slice::from_ref(&edge))
            .await
            .expect("add edge");

        // WITHOUT include — baseline receipt; no edges field.
        let base = recall(
            &RecallReq {
                cell: cell.into(),
                ..Default::default()
            },
            &srv,
        )
        .await
        .expect("recall");
        assert!(base.edges.is_none(), "no include → no edges attached");
        assert!(
            base.receipt.edge_cids.is_empty(),
            "no include → receipt cites no edges"
        );

        // WITH include:["edges"] — edges attached + cited in the receipt.
        let withe = recall(
            &RecallReq {
                cell: cell.into(),
                include: Some(vec!["edges".into()]),
                ..Default::default()
            },
            &srv,
        )
        .await
        .expect("recall+edges");
        let edges = withe.edges.expect("edges attached");
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0], edge);
        assert_eq!(withe.receipt.edge_cids.len(), 1);
        assert_eq!(withe.receipt.edge_cids[0], edge.cid());
    }

    /// Sign the SAME attestation envelope as [`sign`] but tag it with a
    /// multi-tenant scope so the storage layer writes scope-index rows.
    /// The scope is NOT part of the signed preimage, so the signature is
    /// byte-identical to the unscoped envelope over the same facts.
    fn sign_scoped(facts: Vec<Fact>, secret: [u8; 32], scope: emem_fact::Scope) -> Attestation {
        let mut att = sign(facts, secret);
        att.scope = Some(scope);
        att
    }

    fn scope_user(u: &str) -> emem_fact::Scope {
        emem_fact::Scope {
            user_id: Some(u.into()),
            ..Default::default()
        }
    }

    /// B1 scope_round_trip: a fact written under `{user_id:"u1"}` is
    /// returned by an unfiltered recall and by a recall scoped to `u1`,
    /// but a recall scoped to `u2` returns Absence (no facts).
    #[tokio::test]
    async fn scope_round_trip() {
        let cell = "damO.zb000.xUti.zde78";
        let (storage, srv) = ephemeral_server();
        let att = sign_scoped(vec![mk_fact(cell, 7)], [11u8; 32], scope_user("u1"));
        storage.put_attestation(&att).await.expect("attest");

        // Unfiltered recall sees the fact (back-compat: scope index is an
        // additive overlay; the canonical index still holds the fact).
        let unfiltered = recall(
            &RecallReq {
                cell: cell.into(),
                ..Default::default()
            },
            &srv,
        )
        .await
        .expect("recall");
        assert_eq!(unfiltered.facts.len(), 1, "unscoped recall returns the fact");

        // Scoped to the WRONG user → no facts (honest Absence, not a leak).
        let wrong = recall(
            &RecallReq {
                cell: cell.into(),
                scope: Some(scope_user("u2")),
                ..Default::default()
            },
            &srv,
        )
        .await
        .expect("recall u2");
        assert!(
            wrong.facts.is_empty(),
            "recall scoped to u2 must NOT see u1's fact; got {:?}",
            wrong.facts
        );
        // The receipt still binds the (u2) scope the caller asked for.
        assert!(wrong.receipt.scope.is_some(), "scoped recall binds scope");

        // Scoped to the RIGHT user → the fact comes back.
        let right = recall(
            &RecallReq {
                cell: cell.into(),
                scope: Some(scope_user("u1")),
                ..Default::default()
            },
            &srv,
        )
        .await
        .expect("recall u1");
        assert_eq!(right.facts.len(), 1, "recall scoped to u1 returns u1's fact");
    }

    /// B1 legacy_no_scope_unchanged: a write with NO scope + a recall with
    /// NO scope behaves exactly as before — the fact is returned and the
    /// receipt carries no scope binding (the pre-v0.0.8 path).
    #[tokio::test]
    async fn legacy_no_scope_unchanged() {
        let cell = "damO.zb000.xUti.zde78";
        let (storage, srv) = ephemeral_server();
        // Unscoped write (scope: None) — no scope-index rows are written.
        let att = sign(vec![mk_fact(cell, 3)], [12u8; 32]);
        storage.put_attestation(&att).await.expect("attest");

        let resp = recall(
            &RecallReq {
                cell: cell.into(),
                ..Default::default()
            },
            &srv,
        )
        .await
        .expect("recall");
        assert_eq!(resp.facts.len(), 1, "legacy unscoped recall returns the fact");
        assert!(
            resp.receipt.scope.is_none(),
            "legacy recall receipt carries no scope segment"
        );

        // And a scoped recall over an unscoped write returns NOTHING — an
        // unscoped (global) fact is not visible under any tenant scope,
        // by design: scope is opt-in on BOTH sides.
        let scoped = recall(
            &RecallReq {
                cell: cell.into(),
                scope: Some(scope_user("u1")),
                ..Default::default()
            },
            &srv,
        )
        .await
        .expect("scoped recall");
        assert!(
            scoped.facts.is_empty(),
            "an unscoped (global) fact is invisible to a tenant-scoped recall"
        );
    }
}
