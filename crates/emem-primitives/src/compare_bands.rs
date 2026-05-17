//! `compare_bands(cell, a, b, tslot_a?, tslot_b?)` — compare two bands at
//! a single cell.
//!
//! The existing `compare(a_cell, b_cell)` primitive is cell-to-cell. There
//! was no way to ask "is the Cop-DEM elevation here within 200 m of the
//! GMRT bathymetry here?" or "how did the GeoTessera embedding at this
//! cell change between vintage 2017 and vintage 2024?" without two
//! `recall` round-trips and client-side arithmetic, which throws away
//! the receipt chain. This primitive returns one signed envelope citing
//! both source fact CIDs.
//!
//! Behaviour by value type:
//! - both scalar           → `metric = "delta"`,  `value = b - a`
//! - both vector (eq dim)  → `metric = "cosine"`, `value = cos(a, b)`,
//!   `l2_distance` and per-dim diff also reported
//! - mismatched / wrong    → returns Internal error (so the agent can
//!   branch on `incomparable_band_types`)
//!
//! tslot resolution:
//! - `tslot_a`/`tslot_b` are `Option<u64>` on the wire. When supplied
//!   the lookup uses that exact tslot (legacy behaviour — error on
//!   miss). When omitted, the responder scans the cell's index and
//!   picks the **latest tslot** with an attested fact for that band.
//!   The `tslot_resolution` block on the response surfaces what was
//!   chosen and why, so the agent never sees a silent "tslot=0 was
//!   empty so we said nothing was here" failure on medium- or
//!   fast-tempo bands (NDVI, weather, MODIS, CAMS).
//! - When auto-pick finds no fact for a band, the band is surfaced in
//!   `bands_with_no_history` instead of being silently dropped.

use std::time::Instant;

use serde::{Deserialize, Serialize};

use emem_cache::CanonicalKey;
use emem_core::ErrorCode;
use emem_fact::{Fact, FactCid, Receipt};
use emem_storage::{Server, StorageError};

use crate::cbor_ops::{as_f64, as_vec_f32, cosine};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompareBandsReq {
    /// cell64. `cell64` is accepted as an alias.
    #[serde(alias = "cell64")]
    pub cell: String,
    /// Band A.
    pub a: String,
    /// Band B.
    pub b: String,
    /// tslot for band A. When `None` (omitted on the wire) the
    /// responder picks the latest tslot with an attested fact for
    /// band A at this cell. When `Some(t)` the lookup uses exactly
    /// `t` and errors with `CidNotFound` if no fact exists at that
    /// slot. Specify explicitly when comparing two vintages of a
    /// temporal band like `geotessera.year_*`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tslot_a: Option<u64>,
    /// tslot for band B. Same semantics as `tslot_a`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tslot_b: Option<u64>,
    /// Optional predicate. When present the response includes a signed
    /// `verdict: true|false|"incomparable"` — folds the multi-source
    /// consistency-check pattern ("DEM and GMRT agree within 200m") into
    /// one round-trip instead of compare-then-verify-locally.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predicate: Option<ConsistencyPredicate>,
}

/// Predicate over the comparison's primary metric (`absolute_diff` for
/// scalar pairs, `cosine`/`l2_distance` for vectors).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConsistencyPredicate {
    /// Pass when |b-a| ≤ threshold (scalar pairs only).
    AbsDiffLe { threshold: f64 },
    /// Pass when |b-a| < threshold.
    AbsDiffLt { threshold: f64 },
    /// Pass when cosine(a,b) ≥ threshold (vector pairs only).
    CosineGe { threshold: f64 },
    /// Pass when cosine(a,b) > threshold.
    CosineGt { threshold: f64 },
    /// Pass when L2 distance ≤ threshold (vector pairs only).
    L2DistanceLe { threshold: f64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompareBandsResp {
    pub cell: String,
    /// Band A descriptor — `None` only when band A was unresolvable
    /// (auto-pick found no attested fact and the band landed in
    /// `bands_with_no_history`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub a: Option<BandRef>,
    /// Band B descriptor — same semantics as `a`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub b: Option<BandRef>,
    /// `"delta"` for scalar pairs, `"cosine"` for vector pairs.
    /// Omitted when no comparison was performed (see
    /// `bands_with_no_history`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric: Option<String>,
    /// Primary metric value. Omitted when no comparison was performed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    /// Absolute difference (scalar pairs) or L2 distance (vector pairs).
    /// Omitted when no comparison was performed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub absolute_diff: Option<f64>,
    /// Optional per-dimension delta vector when both bands are vectors.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_dim_delta: Option<Vec<f64>>,
    /// Verdict — present iff the request supplied a `predicate`.
    /// `"true"` / `"false"` / `"incomparable"` (predicate type doesn't
    /// match value type, e.g. AbsDiffLe over vector bands).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verdict: Option<String>,
    /// Echoed back when verdict is set, so the receipt envelope is
    /// self-describing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predicate: Option<ConsistencyPredicate>,
    /// How `tslot_a` / `tslot_b` were chosen. Always present so the
    /// agent can distinguish "I asked for tslot X and got it" from
    /// "I let the responder auto-pick and it picked Y" — without this
    /// the silent default of 0 caused medium-tempo bands (NDVI 30-day,
    /// MODIS 8-day) to look empty when there was plenty of data at
    /// later tslots.
    pub tslot_resolution: TslotResolution,
    /// Bands for which auto-pick found no attested fact at this cell.
    /// Empty when the comparison succeeded for both bands. Surfaced as
    /// a structured list (not silently dropped or zeroed) so the agent
    /// can decide whether to backfill or ask elsewhere.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub bands_with_no_history: Vec<BandWithNoHistory>,
    /// Signed receipt naming both source fact CIDs.
    pub receipt: Receipt,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BandRef {
    pub band: String,
    pub tslot: u64,
    /// CID of the cited fact.
    pub fact_cid: String,
}

/// How tslots were chosen for the two bands. Sits outside the signed
/// receipt — it is editorial guidance for the caller, not part of the
/// content-addressed claim. The fact CIDs inside `receipt.fact_cids`
/// are still authoritative.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TslotResolution {
    /// Echo of the request. `tslot_a` / `tslot_b` are `null` when the
    /// caller omitted them and we auto-picked.
    pub requested: TslotResolutionRequested,
    /// Resolved tslot per band. Keyed by band name (`a`, `b`) so the
    /// agent can correlate by position when the same band key appears
    /// twice (vintage-vs-vintage compare).
    pub per_band: TslotResolutionPerBand,
    /// `"caller_supplied"` when both tslots were explicitly given,
    /// `"auto_picked_latest"` when at least one was auto-picked from
    /// the index.
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TslotResolutionRequested {
    pub tslot_a: Option<u64>,
    pub tslot_b: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TslotResolutionPerBand {
    /// Resolved tslot used for band A — `None` when band A had no
    /// attested fact and auto-pick fell into `bands_with_no_history`.
    pub tslot_used_a: Option<u64>,
    /// Resolved tslot used for band B — same semantics.
    pub tslot_used_b: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BandWithNoHistory {
    /// Which slot of the request this corresponds to: `"a"` or `"b"`.
    pub slot: String,
    pub band: String,
    pub cell: String,
    /// `"EmptyHistory"` when the cell has zero attested facts for the
    /// band. Stable string so agents can branch on it.
    pub reason: String,
}

pub async fn compare_bands(
    req: &CompareBandsReq,
    srv: &Server,
) -> Result<CompareBandsResp, StorageError> {
    let started = Instant::now();
    let storage = srv.storage.as_ref();

    // Resolve tslots. Caller-supplied tslots win; omitted tslots are
    // auto-picked as the latest tslot for that band at this cell.
    // `auto_picked` is true iff at least one band was auto-picked —
    // drives the `tslot_resolution.reason` echo below.
    let auto_picked = req.tslot_a.is_none() || req.tslot_b.is_none();
    let scan_pairs = if auto_picked {
        // Single index walk — reused across both bands. Safe even when
        // only one band needs auto-pick; the cost is one prefix scan.
        Some(storage.scan_cell(&req.cell, None).await?)
    } else {
        None
    };

    let resolved_a = resolve_tslot(req.tslot_a, &req.a, scan_pairs.as_deref());
    let resolved_b = resolve_tslot(req.tslot_b, &req.b, scan_pairs.as_deref());

    // Auto-pick that found nothing → EmptyHistory entry. Caller-
    // supplied tslot misses still surface as CidNotFound below
    // (legacy behaviour — explicit ask gets explicit error).
    let mut bands_with_no_history: Vec<BandWithNoHistory> = Vec::new();
    if matches!(resolved_a, ResolvedTslot::AutoPickedEmpty) {
        bands_with_no_history.push(BandWithNoHistory {
            slot: "a".into(),
            band: req.a.clone(),
            cell: req.cell.clone(),
            reason: "EmptyHistory".into(),
        });
    }
    if matches!(resolved_b, ResolvedTslot::AutoPickedEmpty) {
        bands_with_no_history.push(BandWithNoHistory {
            slot: "b".into(),
            band: req.b.clone(),
            cell: req.cell.clone(),
            reason: "EmptyHistory".into(),
        });
    }

    let tslot_used_a = resolved_a.tslot();
    let tslot_used_b = resolved_b.tslot();
    let resolution = TslotResolution {
        requested: TslotResolutionRequested {
            tslot_a: req.tslot_a,
            tslot_b: req.tslot_b,
        },
        per_band: TslotResolutionPerBand {
            tslot_used_a,
            tslot_used_b,
        },
        reason: if auto_picked {
            "auto_picked_latest".into()
        } else {
            "caller_supplied".into()
        },
    };

    // If either band had no attested fact, we cannot perform the
    // comparison. Return Ok with the diagnostic — the agent gets a
    // signed (empty-cite) receipt plus structured `bands_with_no_history`
    // so it can decide whether to backfill or ask elsewhere. This is
    // the no-silent-fallback contract: empty result is *labelled*
    // empty, never zeroed.
    if !bands_with_no_history.is_empty() {
        let receipt = srv.sign_receipt(
            "emem.compare_bands",
            vec![req.cell.clone()],
            Vec::new(),
            true,
            started,
            None,
        );
        return Ok(CompareBandsResp {
            cell: req.cell.clone(),
            a: tslot_used_a.map(|t| BandRef {
                band: req.a.clone(),
                tslot: t,
                fact_cid: String::new(),
            }),
            b: tslot_used_b.map(|t| BandRef {
                band: req.b.clone(),
                tslot: t,
                fact_cid: String::new(),
            }),
            metric: None,
            value: None,
            absolute_diff: None,
            per_dim_delta: None,
            verdict: None,
            predicate: req.predicate.clone(),
            tslot_resolution: resolution,
            bands_with_no_history,
            receipt,
        });
    }

    // From here on both tslots are known — either supplied or
    // auto-picked. unwrap is safe: bands_with_no_history would have
    // been populated otherwise and we'd have returned above.
    let t_a = tslot_used_a.expect("resolved tslot for band A");
    let t_b = tslot_used_b.expect("resolved tslot for band B");

    let key_a = CanonicalKey {
        cell: req.cell.clone(),
        band: req.a.clone(),
        tslot: t_a,
    };
    let key_b = CanonicalKey {
        cell: req.cell.clone(),
        band: req.b.clone(),
        tslot: t_b,
    };
    let cids = storage
        .lookup_canonical_many(&[key_a.clone(), key_b.clone()])
        .await?;
    let cid_a = cids[0].clone().ok_or_else(|| StorageError::Protocol {
        code: ErrorCode::CidNotFound,
        message: format!("no fact at ({}, {}, tslot={})", req.cell, req.a, t_a),
    })?;
    let cid_b = cids[1].clone().ok_or_else(|| StorageError::Protocol {
        code: ErrorCode::CidNotFound,
        message: format!("no fact at ({}, {}, tslot={})", req.cell, req.b, t_b),
    })?;

    let facts = storage
        .get_facts_many(&[cid_a.clone(), cid_b.clone()])
        .await?;
    let fa = facts[0].clone().ok_or_else(|| StorageError::Protocol {
        code: ErrorCode::CidNotFound,
        message: format!("missing fact bytes for {}", cid_a.as_str()),
    })?;
    let fb = facts[1].clone().ok_or_else(|| StorageError::Protocol {
        code: ErrorCode::CidNotFound,
        message: format!("missing fact bytes for {}", cid_b.as_str()),
    })?;

    let (va, vb) = match (&fa, &fb) {
        (Fact::Primary(a), Fact::Primary(b)) => (&a.value, &b.value),
        _ => {
            return Err(StorageError::Protocol {
                code: ErrorCode::Internal,
                message: "compare_bands requires Primary facts on both bands".into(),
            })
        }
    };

    let (metric, value, absolute_diff, per_dim_delta) =
        if let (Some(av), Some(bv)) = (as_vec_f32(va), as_vec_f32(vb)) {
            let n = av.len().min(bv.len());
            if n == 0 {
                return Err(StorageError::Protocol {
                    code: ErrorCode::Internal,
                    message: "compare_bands: empty vectors".into(),
                });
            }
            let cos = cosine(&av[..n], &bv[..n]) as f64;
            let mut sumsq = 0f64;
            let mut delta = Vec::with_capacity(n);
            for i in 0..n {
                let d = (bv[i] - av[i]) as f64;
                sumsq += d * d;
                delta.push(d);
            }
            let l2 = sumsq.sqrt();
            ("cosine".to_string(), cos, l2, Some(delta))
        } else if let (Some(an), Some(bn)) = (as_f64(va), as_f64(vb)) {
            let d = bn - an;
            ("delta".to_string(), d, d.abs(), None)
        } else {
            // Caller-visible shape mismatch (scalar vs vector or
            // vector-length mismatch) is a 400 InvalidArgument, not a 500
            // Internal — the caller can fix it by passing two scalar bands
            // or two equal-length vector bands.
            return Err(StorageError::Protocol {
                code: ErrorCode::InvalidArgument,
                message: format!(
                    "compare_bands: bands ({}, {}) have incomparable value types; \
                     both must be scalar or both vector of equal length",
                    req.a, req.b
                ),
            });
        };

    let verdict = req.predicate.as_ref().map(|p| {
        let scalar_pair = metric == "delta";
        match (p, scalar_pair) {
            (ConsistencyPredicate::AbsDiffLe { threshold }, true) => {
                bool_str(absolute_diff <= *threshold)
            }
            (ConsistencyPredicate::AbsDiffLt { threshold }, true) => {
                bool_str(absolute_diff < *threshold)
            }
            (ConsistencyPredicate::CosineGe { threshold }, false) => bool_str(value >= *threshold),
            (ConsistencyPredicate::CosineGt { threshold }, false) => bool_str(value > *threshold),
            (ConsistencyPredicate::L2DistanceLe { threshold }, false) => {
                bool_str(absolute_diff <= *threshold)
            }
            // Predicate type does not match the value-pair type
            _ => "incomparable".to_string(),
        }
    });

    let receipt = srv.sign_receipt(
        "emem.compare_bands",
        vec![req.cell.clone()],
        vec![cid_a.clone(), cid_b.clone()],
        true,
        started,
        None,
    );

    Ok(CompareBandsResp {
        cell: req.cell.clone(),
        a: Some(BandRef {
            band: req.a.clone(),
            tslot: t_a,
            fact_cid: cid_a.as_str().to_string(),
        }),
        b: Some(BandRef {
            band: req.b.clone(),
            tslot: t_b,
            fact_cid: cid_b.as_str().to_string(),
        }),
        metric: Some(metric),
        value: Some(value),
        absolute_diff: Some(absolute_diff),
        per_dim_delta,
        verdict,
        predicate: req.predicate.clone(),
        tslot_resolution: resolution,
        bands_with_no_history,
        receipt,
    })
}

/// Outcome of resolving a per-band tslot.
enum ResolvedTslot {
    /// Caller supplied this tslot — pass through unchanged.
    Supplied(u64),
    /// Caller omitted; we picked the latest tslot from the index.
    AutoPicked(u64),
    /// Caller omitted, and the index has zero facts for this band at
    /// this cell. Cannot compare; surfaced as `EmptyHistory`.
    AutoPickedEmpty,
}

impl ResolvedTslot {
    fn tslot(&self) -> Option<u64> {
        match self {
            ResolvedTslot::Supplied(t) => Some(*t),
            ResolvedTslot::AutoPicked(t) => Some(*t),
            ResolvedTslot::AutoPickedEmpty => None,
        }
    }
}

fn resolve_tslot(
    requested: Option<u64>,
    band: &str,
    scan_pairs: Option<&[(CanonicalKey, FactCid)]>,
) -> ResolvedTslot {
    if let Some(t) = requested {
        return ResolvedTslot::Supplied(t);
    }
    // Auto-pick path: scan the cell's index, filter to the band,
    // take the maximum tslot. Reuses the same `scan_cell` walker
    // that `/v1/recall` and `trajectory` use — single source of
    // truth for "what's attested at this cell".
    let pairs = scan_pairs.expect("scan_pairs must be provided when requested is None");
    let latest = pairs
        .iter()
        .filter(|(k, _)| k.band == band)
        .map(|(k, _)| k.tslot)
        .max();
    match latest {
        Some(t) => ResolvedTslot::AutoPicked(t),
        None => ResolvedTslot::AutoPickedEmpty,
    }
}

fn bool_str(b: bool) -> String {
    (if b { "true" } else { "false" }).to_string()
}

#[allow(dead_code)]
fn _force_use(c: &[FactCid]) {
    let _ = c;
}

#[cfg(test)]
mod tests {
    //! Tests for the P1-A1 fix: when `tslot_a`/`tslot_b` are omitted
    //! the responder must auto-pick the latest tslot per band from
    //! the cell's index (not silently default to 0). When a band has
    //! zero attested facts the response surfaces it under
    //! `bands_with_no_history` instead of dropping or zeroing the
    //! comparison.
    //!
    //! Same hand-rolled `MockStorage` shape as `find_similar::tests`
    //! — only the surface that `compare_bands` actually uses
    //! (`scan_cell`, `lookup_canonical_many`, `get_facts_many`) is
    //! wired. Everything else `unimplemented!` so regressions that
    //! start to depend on a broader surface fail loudly.
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use ciborium::Value as CborValue;

    use emem_cache::CanonicalKey;
    use emem_core::AttesterKey;
    use emem_fact::{Derivation, Fact, FactCid, PrimaryFact, RegistryCid, SchemaCid, Source};
    use emem_storage::server::{ManifestCids, ResponderIdentity};
    use emem_storage::{Server, Storage, StorageError};

    struct MockStorage {
        // (CanonicalKey, FactCid, Fact) per row — same shape as
        // find_similar's mock so the two test suites share intuition.
        entries: Mutex<Vec<(CanonicalKey, FactCid, Fact)>>,
        cid_to_fact: Mutex<HashMap<String, Fact>>,
    }

    impl MockStorage {
        fn new() -> Self {
            Self {
                entries: Mutex::new(Vec::new()),
                cid_to_fact: Mutex::new(HashMap::new()),
            }
        }

        /// Insert a scalar fact at (cell, band, tslot). Each call mints
        /// a fresh FactCid so the index is uniquely keyed by tslot.
        fn insert_scalar(&self, cell: &str, band: &str, tslot: u64, value: f64) -> FactCid {
            let cid_str = format!("test-cid-{}-{}-{}", band.replace('.', "_"), tslot, value);
            let cid = FactCid::new(&cid_str);
            let fact = Fact::Primary(PrimaryFact {
                cell: cell.into(),
                band: band.into(),
                tslot,
                value: CborValue::Float(value),
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
                signed_at: "2026-05-05T00:00:00Z".into(),
                served_via: None,
            });
            self.entries.lock().unwrap().push((
                CanonicalKey {
                    cell: cell.into(),
                    band: band.into(),
                    tslot,
                },
                cid.clone(),
                fact.clone(),
            ));
            self.cid_to_fact.lock().unwrap().insert(cid_str, fact);
            cid
        }
    }

    #[async_trait]
    impl Storage for MockStorage {
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
                        .find(|(ek, _, _)| ek == k)
                        .map(|(_, cid, _)| cid.clone())
                })
                .collect())
        }

        async fn get_facts_many(
            &self,
            cids: &[FactCid],
        ) -> Result<Vec<Option<Fact>>, StorageError> {
            let map = self.cid_to_fact.lock().unwrap();
            Ok(cids.iter().map(|c| map.get(c.as_str()).cloned()).collect())
        }

        async fn put_attestation(
            &self,
            _att: &emem_fact::Attestation,
        ) -> Result<Vec<FactCid>, StorageError> {
            unimplemented!("put_attestation not used by compare_bands")
        }

        async fn materialize_many(
            &self,
            _keys: &[CanonicalKey],
        ) -> Result<Vec<FactCid>, StorageError> {
            unimplemented!("materialize_many not used by compare_bands")
        }

        async fn scan_cell(
            &self,
            cell: &str,
            tslot: Option<u64>,
        ) -> Result<Vec<(CanonicalKey, FactCid)>, StorageError> {
            let entries = self.entries.lock().unwrap();
            Ok(entries
                .iter()
                .filter(|(k, _, _)| k.cell == cell && tslot.map(|t| k.tslot == t).unwrap_or(true))
                .map(|(k, c, _)| (k.clone(), c.clone()))
                .collect())
        }

        async fn iter_index(
            &self,
            _limit: Option<usize>,
        ) -> Result<Vec<(CanonicalKey, FactCid)>, StorageError> {
            unimplemented!("iter_index not used by compare_bands")
        }
    }

    fn test_server(storage: Arc<MockStorage>) -> Server {
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

    /// Acceptance (a): when `tslot_a`/`tslot_b` are omitted, the
    /// response carries `tslot_resolution.reason == "auto_picked_latest"`
    /// and the per-band map names the latest tslot found for each band.
    #[tokio::test]
    async fn auto_picks_latest_tslot_when_omitted() {
        let storage = Arc::new(MockStorage::new());
        // Two scalar bands at the same cell — band a has facts at
        // tslots {0, 7, 12}, band b at {3, 9}. With both tslots
        // omitted the responder must compare a@12 vs b@9 (each band's
        // own latest), not a@0 vs b@0.
        storage.insert_scalar("cell-1", "indices.ndvi", 0, 0.10);
        storage.insert_scalar("cell-1", "indices.ndvi", 7, 0.30);
        storage.insert_scalar("cell-1", "indices.ndvi", 12, 0.50);
        storage.insert_scalar("cell-1", "weather.temperature_2m", 3, 18.0);
        storage.insert_scalar("cell-1", "weather.temperature_2m", 9, 22.0);

        let srv = test_server(storage);
        let req = CompareBandsReq {
            cell: "cell-1".into(),
            a: "indices.ndvi".into(),
            b: "weather.temperature_2m".into(),
            tslot_a: None,
            tslot_b: None,
            predicate: None,
        };
        let resp = compare_bands(&req, &srv).await.expect("compare_bands ok");

        assert_eq!(resp.tslot_resolution.reason, "auto_picked_latest");
        assert_eq!(resp.tslot_resolution.requested.tslot_a, None);
        assert_eq!(resp.tslot_resolution.requested.tslot_b, None);
        assert_eq!(resp.tslot_resolution.per_band.tslot_used_a, Some(12));
        assert_eq!(resp.tslot_resolution.per_band.tslot_used_b, Some(9));

        // Comparison happened; metric / value populated; receipt
        // cites both facts (auto-pick is editorial; the cited CIDs
        // are still authoritative).
        assert_eq!(resp.metric.as_deref(), Some("delta"));
        assert!(resp.value.is_some());
        assert!(resp.absolute_diff.is_some());
        assert_eq!(resp.receipt.fact_cids.len(), 2);
        assert!(resp.bands_with_no_history.is_empty());
        // BandRefs reflect the resolved tslots, not the omitted
        // request tslots.
        assert_eq!(resp.a.as_ref().map(|r| r.tslot), Some(12));
        assert_eq!(resp.b.as_ref().map(|r| r.tslot), Some(9));
    }

    /// Acceptance (b): when both bands have facts at *different*
    /// latest tslots, the response surfaces both `tslot_used_a` and
    /// `tslot_used_b` honestly. The protocol does NOT collapse them
    /// to a single number — the agent needs to see staleness drift.
    #[tokio::test]
    async fn surfaces_both_tslots_when_bands_diverge() {
        let storage = Arc::new(MockStorage::new());
        // band a is fresh (latest=2024 = tslot 100), band b is stale
        // (latest=2018 = tslot 50). The agent must learn this gap.
        storage.insert_scalar("cell-x", "alpha", 0, 1.0);
        storage.insert_scalar("cell-x", "alpha", 50, 5.0);
        storage.insert_scalar("cell-x", "alpha", 100, 9.0);
        storage.insert_scalar("cell-x", "beta", 10, 2.0);
        storage.insert_scalar("cell-x", "beta", 50, 6.0);

        let srv = test_server(storage);
        let req = CompareBandsReq {
            cell: "cell-x".into(),
            a: "alpha".into(),
            b: "beta".into(),
            tslot_a: None,
            tslot_b: None,
            predicate: None,
        };
        let resp = compare_bands(&req, &srv).await.expect("compare_bands ok");

        assert_eq!(resp.tslot_resolution.per_band.tslot_used_a, Some(100));
        assert_eq!(resp.tslot_resolution.per_band.tslot_used_b, Some(50));

        // BandRefs carry the same divergent tslots so an agent that
        // only reads the body fields (not the resolution block) still
        // sees the gap.
        assert_eq!(resp.a.as_ref().map(|r| r.tslot), Some(100));
        assert_eq!(resp.b.as_ref().map(|r| r.tslot), Some(50));

        // Comparison succeeded — alpha@100 (9.0) vs beta@50 (6.0).
        assert_eq!(resp.value, Some(6.0 - 9.0));
        assert_eq!(resp.absolute_diff, Some(3.0));
    }

    /// Acceptance (c): when one band has zero attested facts, that
    /// band lands in `bands_with_no_history[]` with `reason ==
    /// "EmptyHistory"`. The comparison is NOT performed (no metric,
    /// no value, no zeroed-out delta) — silence is labelled silence,
    /// not a synthetic zero.
    #[tokio::test]
    async fn empty_history_surfaces_under_bands_with_no_history() {
        let storage = Arc::new(MockStorage::new());
        // Only band a exists at this cell; band b has zero facts.
        storage.insert_scalar("cell-q", "indices.ndvi", 0, 0.4);
        storage.insert_scalar("cell-q", "indices.ndvi", 5, 0.6);

        let srv = test_server(storage);
        let req = CompareBandsReq {
            cell: "cell-q".into(),
            a: "indices.ndvi".into(),
            b: "modis.gpp_8day".into(),
            tslot_a: None,
            tslot_b: None,
            predicate: None,
        };
        let resp = compare_bands(&req, &srv).await.expect("compare_bands ok");

        // Band a was resolvable (latest=5); band b lands in the no-
        // history list with the stable EmptyHistory reason.
        assert_eq!(resp.tslot_resolution.per_band.tslot_used_a, Some(5));
        assert_eq!(resp.tslot_resolution.per_band.tslot_used_b, None);
        assert_eq!(resp.bands_with_no_history.len(), 1);
        let entry = &resp.bands_with_no_history[0];
        assert_eq!(entry.slot, "b");
        assert_eq!(entry.band, "modis.gpp_8day");
        assert_eq!(entry.cell, "cell-q");
        assert_eq!(entry.reason, "EmptyHistory");

        // No comparison was performed → no metric, no value, no
        // absolute_diff. The receipt is still signed (proof of who
        // told you "I don't know") but cites zero facts.
        assert!(resp.metric.is_none());
        assert!(resp.value.is_none());
        assert!(resp.absolute_diff.is_none());
        assert!(resp.receipt.fact_cids.is_empty());
        assert_eq!(resp.tslot_resolution.reason, "auto_picked_latest");
    }

    /// Caller-supplied tslots take the legacy code path: lookup
    /// fails with CidNotFound rather than auto-picking. This guards
    /// the "do not change behaviour when caller supplied tslot
    /// explicitly" constraint of the P1-A1 fix.
    #[tokio::test]
    async fn caller_supplied_tslot_preserves_legacy_error_on_miss() {
        let storage = Arc::new(MockStorage::new());
        // Plenty of facts — but none at tslot=0 for either band.
        storage.insert_scalar("cell-y", "alpha", 7, 1.0);
        storage.insert_scalar("cell-y", "beta", 7, 2.0);

        let srv = test_server(storage);
        let req = CompareBandsReq {
            cell: "cell-y".into(),
            a: "alpha".into(),
            b: "beta".into(),
            tslot_a: Some(0),
            tslot_b: Some(0),
            predicate: None,
        };
        let err = compare_bands(&req, &srv)
            .await
            .expect_err("supplied tslot=0 with no fact must error");
        match err {
            StorageError::Protocol { code, .. } => {
                assert_eq!(code, ErrorCode::CidNotFound);
            }
            other => panic!("expected Protocol/CidNotFound, got {other:?}"),
        }
    }

    /// Mixed: caller supplied tslot_a, omitted tslot_b. Reason is
    /// `auto_picked_latest` (because at least one was auto-picked)
    /// and the per-band map honours both choices independently.
    #[tokio::test]
    async fn mixed_supplied_and_auto_picked_marks_resolution_auto() {
        let storage = Arc::new(MockStorage::new());
        storage.insert_scalar("cell-z", "alpha", 3, 1.0);
        storage.insert_scalar("cell-z", "alpha", 11, 2.0);
        storage.insert_scalar("cell-z", "beta", 4, 7.0);
        storage.insert_scalar("cell-z", "beta", 8, 9.0);

        let srv = test_server(storage);
        let req = CompareBandsReq {
            cell: "cell-z".into(),
            a: "alpha".into(),
            b: "beta".into(),
            tslot_a: Some(3),
            tslot_b: None,
            predicate: None,
        };
        let resp = compare_bands(&req, &srv).await.expect("compare_bands ok");

        assert_eq!(resp.tslot_resolution.reason, "auto_picked_latest");
        assert_eq!(resp.tslot_resolution.requested.tslot_a, Some(3));
        assert_eq!(resp.tslot_resolution.requested.tslot_b, None);
        assert_eq!(resp.tslot_resolution.per_band.tslot_used_a, Some(3));
        assert_eq!(resp.tslot_resolution.per_band.tslot_used_b, Some(8));
    }
}
