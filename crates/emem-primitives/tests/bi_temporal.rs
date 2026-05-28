//! Bi-temporal recall tests — Zep/Graphiti edge model (arXiv 2501.13956).
//!
//! Seeds three facts at the same (cell, band) with distinct
//! (tslot, signed_at) tuples and exercises:
//!
//!  * No bound — returns the latest fact (tslot=300).
//!  * `as_of_tslot=150` — returns the tslot=100 fact (latest ≤ bound).
//!  * `as_of_signed_at=t2` — returns the latest fact with signed_at ≤ t2.
//!  * Both bounds combined — most restrictive wins.
//!  * `trajectory` honours the bound across a window.
//!  * `find_similar` pre-filters candidate cells by the bound.
//!  * Empty result emits `temporal_advice`, not a 404.
//!  * Receipt body carries the `as_of` block when the bound was non-empty.
//!  * Old receipts (no `as_of` field) still round-trip.
//!  * Invalid RFC 3339 string in `as_of_signed_at` lands as a 400.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ciborium::Value as CborValue;

use emem_cache::CanonicalKey;
use emem_core::AttesterKey;
use emem_fact::{Derivation, Fact, FactCid, PrimaryFact, RegistryCid, SchemaCid, Source};
use emem_primitives::find_similar::{find_similar, FindSimilarMode, FindSimilarReq};
use emem_primitives::recall::{recall, RecallReq};
use emem_primitives::trajectory::{trajectory, TrajectoryReq};
use emem_storage::server::{ManifestCids, ResponderIdentity};
use emem_storage::{Server, Storage, StorageError};

/// Hand-rolled `Storage` impl — Lance and the materializer are not in
/// scope for these tests. Only the methods bi-temporal recall actually
/// uses are wired (`scan_cell`, `iter_index`, `get_facts_many`).
struct MemStorage {
    entries: Mutex<Vec<(CanonicalKey, FactCid)>>,
    facts: Mutex<HashMap<String, Fact>>,
}

impl MemStorage {
    fn new() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
            facts: Mutex::new(HashMap::new()),
        }
    }

    fn insert_value(
        &self,
        cell: &str,
        band: &str,
        tslot: u64,
        signed_at: &str,
        value: CborValue,
    ) -> FactCid {
        let cid_str = format!("test-cid-{cell}-{band}-{tslot}-{signed_at}");
        let cid = FactCid::new(&cid_str);
        let fact = Fact::Primary(PrimaryFact {
            cell: cell.into(),
            band: band.into(),
            tslot,
            value,
            unit: None,
            confidence: 1.0,
            uncertainty: None,
            sources: vec![Source {
                scheme: "test".into(),
                id: cid_str.clone(),
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
            signer: AttesterKey([0u8; 32]),
            signed_at: signed_at.into(),
            served_via: None,
        });
        self.entries.lock().unwrap().push((
            CanonicalKey {
                cell: cell.into(),
                band: band.into(),
                tslot,
            },
            cid.clone(),
        ));
        self.facts.lock().unwrap().insert(cid_str, fact);
        cid
    }

    fn insert_scalar(
        &self,
        cell: &str,
        band: &str,
        tslot: u64,
        signed_at: &str,
        v: f64,
    ) -> FactCid {
        self.insert_value(cell, band, tslot, signed_at, CborValue::Float(v))
    }

    fn insert_vector(
        &self,
        cell: &str,
        band: &str,
        tslot: u64,
        signed_at: &str,
        v: Vec<f32>,
    ) -> FactCid {
        let value = CborValue::Array(v.into_iter().map(|x| CborValue::Float(x as f64)).collect());
        self.insert_value(cell, band, tslot, signed_at, value)
    }
}

#[async_trait]
impl Storage for MemStorage {
    async fn lookup_canonical_many(
        &self,
        keys: &[CanonicalKey],
    ) -> Result<Vec<Option<FactCid>>, StorageError> {
        let entries = self.entries.lock().unwrap();
        Ok(keys
            .iter()
            .map(|k| {
                entries
                    .iter()
                    .find(|(ek, _)| ek == k)
                    .map(|(_, c)| c.clone())
            })
            .collect())
    }

    async fn get_facts_many(&self, cids: &[FactCid]) -> Result<Vec<Option<Fact>>, StorageError> {
        let map = self.facts.lock().unwrap();
        Ok(cids.iter().map(|c| map.get(c.as_str()).cloned()).collect())
    }

    async fn put_attestation(
        &self,
        _att: &emem_fact::Attestation,
    ) -> Result<Vec<FactCid>, StorageError> {
        unimplemented!("not used")
    }

    async fn materialize_many(&self, _keys: &[CanonicalKey]) -> Result<Vec<FactCid>, StorageError> {
        unimplemented!("not used")
    }

    async fn scan_cell(
        &self,
        cell: &str,
        tslot: Option<u64>,
    ) -> Result<Vec<(CanonicalKey, FactCid)>, StorageError> {
        let entries = self.entries.lock().unwrap();
        Ok(entries
            .iter()
            .filter(|(k, _)| k.cell == cell && tslot.map(|t| k.tslot == t).unwrap_or(true))
            .cloned()
            .collect())
    }

    async fn iter_index(
        &self,
        limit: Option<usize>,
    ) -> Result<Vec<(CanonicalKey, FactCid)>, StorageError> {
        let entries = self.entries.lock().unwrap();
        let mut out: Vec<_> = entries.clone();
        if let Some(n) = limit {
            out.truncate(n);
        }
        Ok(out)
    }
}

fn test_server(storage: Arc<MemStorage>) -> Server {
    Server {
        storage,
        identity: ResponderIdentity::fresh(),
        manifests: ManifestCids {
            registry_cid: RegistryCid::new("test-registry"),
            schema_cid: SchemaCid::new("test-schema"),
            bands_cid: "test-bands".into(),
            sources_cid: "test-sources".into(),
        },
        started_at_unix_s: 0,
    }
}

const CELL: &str = "test-cell";
const BAND: &str = "indices.ndvi";
const T1: &str = "2025-01-01T00:00:00Z";
const T2: &str = "2025-06-01T00:00:00Z";
const T3: &str = "2026-01-01T00:00:00Z";

fn seed_three_facts(storage: &MemStorage) {
    storage.insert_scalar(CELL, BAND, 100, T1, 0.10);
    storage.insert_scalar(CELL, BAND, 200, T2, 0.20);
    storage.insert_scalar(CELL, BAND, 300, T3, 0.30);
}

fn primary_tslots(facts: &[Fact]) -> Vec<u64> {
    facts
        .iter()
        .filter_map(|f| match f {
            Fact::Primary(p) => Some(p.tslot),
            _ => None,
        })
        .collect()
}

/// No bound → historical recall shape: every attested fact at the
/// cell matching the band filter (one Primary per tslot). Bi-temporal
/// recall preserves this exactly when the bound is unbounded so callers
/// already wired against the historical API see no change.
#[tokio::test]
async fn recall_no_bound_returns_all_attested_facts() {
    let storage = Arc::new(MemStorage::new());
    seed_three_facts(&storage);
    let srv = test_server(storage);

    let req = RecallReq {
        cell: CELL.into(),
        bands: Some(vec![BAND.into()]),
        tslot: None,
        as_of_tslot: None,
        as_of_signed_at: None,
        scope: None,
    };
    let resp = recall(&req, &srv).await.expect("ok");
    let mut tslots = primary_tslots(&resp.facts);
    tslots.sort_unstable();
    assert_eq!(
        tslots,
        vec![100, 200, 300],
        "historical behaviour: no bound surfaces every attested vintage"
    );
    assert!(
        resp.receipt.as_of.is_none(),
        "unbounded recall must NOT carry an as_of block in the receipt"
    );
    assert!(resp.temporal_advice.is_none());
}

/// `as_of_tslot=150` → tslot=100 wins (latest ≤ 150).
#[tokio::test]
async fn recall_with_as_of_tslot_picks_latest_le_bound() {
    let storage = Arc::new(MemStorage::new());
    seed_three_facts(&storage);
    let srv = test_server(storage);

    let req = RecallReq {
        cell: CELL.into(),
        bands: Some(vec![BAND.into()]),
        tslot: None,
        as_of_tslot: Some(150),
        as_of_signed_at: None,
        scope: None,
    };
    let resp = recall(&req, &srv).await.expect("ok");
    let tslots = primary_tslots(&resp.facts);
    assert_eq!(
        tslots,
        vec![100],
        "as_of_tslot=150 must return tslot=100 (latest ≤ bound)"
    );
    let as_of = resp
        .receipt
        .as_of
        .as_ref()
        .expect("receipt must carry as_of block for bounded recall");
    assert_eq!(as_of.valid_time, Some(150));
    assert!(as_of.transaction_time.is_none());
}

/// `as_of_signed_at=T2` → tslot=200 wins (latest fact with signed_at ≤ T2).
#[tokio::test]
async fn recall_with_as_of_signed_at_picks_latest_le_signed_at() {
    let storage = Arc::new(MemStorage::new());
    seed_three_facts(&storage);
    let srv = test_server(storage);

    let req = RecallReq {
        cell: CELL.into(),
        bands: Some(vec![BAND.into()]),
        tslot: None,
        as_of_tslot: None,
        as_of_signed_at: Some(T2.into()),
        scope: None,
    };
    let resp = recall(&req, &srv).await.expect("ok");
    let tslots = primary_tslots(&resp.facts);
    assert_eq!(
        tslots,
        vec![200],
        "as_of_signed_at=T2 must return the tslot=200/T2 fact"
    );
    let as_of = resp.receipt.as_of.as_ref().expect("as_of block");
    assert_eq!(as_of.transaction_time.as_deref(), Some(T2));
}

/// Both bounds combined — the most restrictive wins.
#[tokio::test]
async fn recall_with_both_bounds_intersects() {
    let storage = Arc::new(MemStorage::new());
    seed_three_facts(&storage);
    let srv = test_server(storage);

    // valid_time ≤ 250 admits {100, 200}; transaction_time ≤ T2 also
    // admits {100, 200}. Intersection: {100, 200}; latest in the
    // intersection is tslot=200.
    let req = RecallReq {
        cell: CELL.into(),
        bands: Some(vec![BAND.into()]),
        tslot: None,
        as_of_tslot: Some(250),
        as_of_signed_at: Some(T2.into()),
        scope: None,
    };
    let resp = recall(&req, &srv).await.expect("ok");
    let tslots = primary_tslots(&resp.facts);
    assert_eq!(
        tslots,
        vec![200],
        "both bounds together must intersect to {{100,200}}; latest=200"
    );

    // valid_time ≤ 150 admits only {100}; transaction_time ≤ T2 admits
    // {100, 200}. Intersection: {100}.
    let req = RecallReq {
        cell: CELL.into(),
        bands: Some(vec![BAND.into()]),
        tslot: None,
        as_of_tslot: Some(150),
        as_of_signed_at: Some(T2.into()),
        scope: None,
    };
    let resp = recall(&req, &srv).await.expect("ok");
    let tslots = primary_tslots(&resp.facts);
    assert_eq!(tslots, vec![100]);
}

/// Empty result with a non-empty bound triggers `temporal_advice` and
/// the response remains a 200 (Ok), not a NotFound.
#[tokio::test]
async fn empty_result_emits_temporal_advice_not_404() {
    let storage = Arc::new(MemStorage::new());
    seed_three_facts(&storage);
    let srv = test_server(storage);

    let req = RecallReq {
        cell: CELL.into(),
        bands: Some(vec![BAND.into()]),
        tslot: None,
        as_of_tslot: Some(50), // every fact is > 50
        as_of_signed_at: None,
        scope: None,
    };
    let resp = recall(&req, &srv).await.expect("ok");
    assert!(resp.facts.is_empty(), "every fact is filtered out");
    let advice = resp
        .temporal_advice
        .expect("must emit temporal_advice when the bound filtered everything");
    assert_eq!(advice.as_of_tslot, Some(50));
    assert_eq!(advice.facts_at_cell_unbounded, 3);
}

/// Conflicting bounds — `as_of_tslot < tslot` — must error with
/// `invalid_temporal_bound`.
#[tokio::test]
async fn conflicting_tslot_and_as_of_returns_400() {
    let storage = Arc::new(MemStorage::new());
    seed_three_facts(&storage);
    let srv = test_server(storage);

    let req = RecallReq {
        cell: CELL.into(),
        bands: Some(vec![BAND.into()]),
        tslot: Some(200),
        as_of_tslot: Some(150), // bound excludes tslot
        as_of_signed_at: None,
        scope: None,
    };
    let err = recall(&req, &srv).await.expect_err("must error");
    let msg = format!("{err}");
    assert!(
        msg.contains("invalid_temporal_bound"),
        "expected invalid_temporal_bound, got: {msg}"
    );
}

/// Malformed RFC 3339 in `as_of_signed_at` → `invalid_signed_at_format`.
#[tokio::test]
async fn invalid_rfc3339_returns_400() {
    let storage = Arc::new(MemStorage::new());
    seed_three_facts(&storage);
    let srv = test_server(storage);

    let req = RecallReq {
        cell: CELL.into(),
        bands: Some(vec![BAND.into()]),
        tslot: None,
        as_of_tslot: None,
        as_of_signed_at: Some("yesterday".into()),
        scope: None,
    };
    let err = recall(&req, &srv).await.expect_err("must error");
    let msg = format!("{err}");
    assert!(
        msg.contains("invalid_signed_at_format"),
        "expected invalid_signed_at_format, got: {msg}"
    );
}

/// Trajectory honours the bi-temporal bound across a window.
#[tokio::test]
async fn trajectory_respects_as_of_tslot() {
    let storage = Arc::new(MemStorage::new());
    seed_three_facts(&storage);
    let srv = test_server(storage);

    // Wide window covers all three points.
    let req_unbounded = TrajectoryReq {
        cell: CELL.into(),
        band: BAND.into(),
        window: [0, 1000],
        as_of_tslot: None,
        as_of_signed_at: None,
    };
    let r = trajectory(&req_unbounded, &srv).await.expect("ok");
    assert_eq!(r.series.len(), 3);

    // Cap at 200 → drops the 300 point.
    let req_capped = TrajectoryReq {
        cell: CELL.into(),
        band: BAND.into(),
        window: [0, 1000],
        as_of_tslot: Some(200),
        as_of_signed_at: None,
    };
    let r = trajectory(&req_capped, &srv).await.expect("ok");
    let tslots: Vec<u64> = r.series.iter().map(|p| p.tslot).collect();
    assert_eq!(
        tslots,
        vec![100, 200],
        "as_of_tslot=200 must drop tslot=300"
    );
    let as_of = r.receipt.as_of.as_ref().expect("as_of block");
    assert_eq!(as_of.valid_time, Some(200));
}

/// find_similar pre-filters candidate cells by the bound BEFORE cosine
/// scoring. A cell with no qualifying fact under the scoring band is
/// dropped (not silently included with stale data).
#[tokio::test]
async fn find_similar_prefilters_candidates_by_as_of() {
    let storage = Arc::new(MemStorage::new());
    // Query cell — vector identical at every vintage so it remains
    // self-match-filtered regardless of bound.
    storage.insert_vector("q", "geotessera", 100, T1, vec![1.0, 0.0, 0.0, 0.0]);
    storage.insert_vector("q", "geotessera", 200, T2, vec![1.0, 0.0, 0.0, 0.0]);

    // Cell A — has a vector at tslot=100. Should pass an as_of_tslot=150.
    storage.insert_vector("a", "geotessera", 100, T1, vec![0.99, 0.01, 0.0, 0.0]);

    // Cell B — only has a vector at tslot=300. Should be DROPPED by
    // as_of_tslot=150 because no qualifying fact exists under the
    // scoring band within the bound.
    storage.insert_vector("b", "geotessera", 300, T3, vec![0.95, 0.05, 0.0, 0.0]);

    let srv = test_server(storage);

    // Unbounded — both a and b should appear.
    let req = FindSimilarReq {
        key: "q".into(),
        k: Some(10),
        band: Some("geotessera".into()),
        filter: None,
        mode: FindSimilarMode::Cosine,
        ..Default::default()
    };
    let resp = find_similar(&req, &srv).await.expect("ok");
    let cells: Vec<&str> = resp.neighbors.iter().map(|n| n.cell.as_str()).collect();
    assert!(cells.contains(&"a"), "unbounded: a should appear");
    assert!(cells.contains(&"b"), "unbounded: b should appear");

    // Bounded — a passes, b dropped.
    let req = FindSimilarReq {
        key: "q".into(),
        k: Some(10),
        band: Some("geotessera".into()),
        filter: None,
        mode: FindSimilarMode::Cosine,
        as_of_tslot: Some(150),
        as_of_signed_at: None,
    };
    let resp = find_similar(&req, &srv).await.expect("ok");
    let cells: Vec<&str> = resp.neighbors.iter().map(|n| n.cell.as_str()).collect();
    assert!(
        cells.contains(&"a"),
        "as_of_tslot=150 must keep cell a (has a fact at tslot=100)"
    );
    assert!(
        !cells.contains(&"b"),
        "as_of_tslot=150 must drop cell b (only fact is at tslot=300); got cells={cells:?}"
    );
    let as_of = resp.receipt.as_of.as_ref().expect("as_of in receipt");
    assert_eq!(as_of.valid_time, Some(150));
}

/// Old receipts (no `as_of` field) must still round-trip through serde
/// — the field is `Option<AsOfReceipt>` with `serde(skip_serializing_if =
/// "Option::is_none")`, so legacy JSON deserialises with `as_of=None`.
#[tokio::test]
async fn legacy_receipt_round_trips_without_as_of() {
    use serde_json::json;
    let pubkey: Vec<u8> = vec![0u8; 32];
    let sig: Vec<u8> = vec![0u8; 64];
    let legacy = json!({
        "request_id": "01H8R000000000000000000000",
        "served_at": "2026-05-28T00:00:00Z",
        "primitive": "emem.recall",
        "cells": ["test-cell"],
        "fact_cids": [],
        "schema_cid": "test-schema",
        "responder": pubkey,
        "responder_key_epoch": 0,
        "signature": sig,
        "source_versions": {},
        "registry_cid": "test-registry",
        "cost": {
            "credits": 0,
            "latency_p50_ms": 1,
            "latency_p99_ms": 1,
            "source_freshness_s": 0,
            "was_cached": false
        }
    });
    let parsed: emem_fact::Receipt = serde_json::from_value(legacy).expect("legacy round-trip");
    assert!(parsed.as_of.is_none());
    // And the round-trip serialise must omit the as_of key entirely so
    // old verifiers diffing JSON see no change.
    let v = serde_json::to_value(&parsed).expect("serialises");
    assert!(
        v.get("as_of").is_none(),
        "as_of must be skipped from JSON when None"
    );
}
