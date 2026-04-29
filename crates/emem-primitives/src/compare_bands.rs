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
    /// tslot for band A (default 0 — the static slot used by Cop-DEM,
    /// GMRT, ESA WorldCover, etc. Specify when comparing two vintages
    /// of a temporal band like `geotessera.year_*`).
    #[serde(default)]
    pub tslot_a: u64,
    /// tslot for band B (default 0).
    #[serde(default)]
    pub tslot_b: u64,
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
    pub a: BandRef,
    pub b: BandRef,
    /// `"delta"` for scalar pairs, `"cosine"` for vector pairs.
    pub metric: String,
    /// Primary metric value.
    pub value: f64,
    /// Absolute difference (scalar pairs) or L2 distance (vector pairs).
    pub absolute_diff: f64,
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

pub async fn compare_bands(
    req: &CompareBandsReq,
    srv: &Server,
) -> Result<CompareBandsResp, StorageError> {
    let started = Instant::now();
    let storage = srv.storage.as_ref();

    let key_a = CanonicalKey {
        cell: req.cell.clone(),
        band: req.a.clone(),
        tslot: req.tslot_a,
    };
    let key_b = CanonicalKey {
        cell: req.cell.clone(),
        band: req.b.clone(),
        tslot: req.tslot_b,
    };
    let cids = storage
        .lookup_canonical_many(&[key_a.clone(), key_b.clone()])
        .await?;
    let cid_a = cids[0].clone().ok_or_else(|| StorageError::Protocol {
        code: ErrorCode::CidNotFound,
        message: format!(
            "no fact at ({}, {}, tslot={})",
            req.cell, req.a, req.tslot_a
        ),
    })?;
    let cid_b = cids[1].clone().ok_or_else(|| StorageError::Protocol {
        code: ErrorCode::CidNotFound,
        message: format!(
            "no fact at ({}, {}, tslot={})",
            req.cell, req.b, req.tslot_b
        ),
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
            return Err(StorageError::Protocol {
                code: ErrorCode::Internal,
                message: format!(
                    "compare_bands: bands ({}, {}) have incomparable value types — \
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
        a: BandRef {
            band: req.a.clone(),
            tslot: req.tslot_a,
            fact_cid: cid_a.as_str().to_string(),
        },
        b: BandRef {
            band: req.b.clone(),
            tslot: req.tslot_b,
            fact_cid: cid_b.as_str().to_string(),
        },
        metric,
        value,
        absolute_diff,
        per_dim_delta,
        verdict,
        predicate: req.predicate.clone(),
        receipt,
    })
}

fn bool_str(b: bool) -> String {
    (if b { "true" } else { "false" }).to_string()
}

#[allow(dead_code)]
fn _force_use(c: &[FactCid]) {
    let _ = c;
}
