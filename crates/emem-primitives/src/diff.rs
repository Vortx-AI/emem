//! `diff(cell, band, tslot_a, tslot_b)` — produces a DerivativeFact.
//!
//! Generic numeric delta: looks up the canonical `(cell, band, tslot_a)` and
//! `(cell, band, tslot_b)` facts, asserts both are scalar, and emits a
//! `DerivativeFact` with `op="delta"` and `value = b - a`. Vector bands
//! return a per-dimension delta vector instead.

use std::time::Instant;

use serde::{Deserialize, Serialize};

use emem_cache::CanonicalKey;
use emem_core::ErrorCode;
use emem_fact::{Derivation, DerivativeFact, Fact, Receipt};
use emem_storage::{server::iso8601_now, Server, StorageError};

use crate::cbor_ops::{as_f64, as_vec_f32};

/// diff request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffReq {
    /// cell64. `cell64` is accepted as an alias.
    #[serde(alias = "cell64")]
    pub cell: String,
    /// Band key.
    pub band: String,
    /// Earlier slot.
    pub tslot_a: u64,
    /// Later slot.
    pub tslot_b: u64,
}

/// diff response — the derivative fact + receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffResp {
    /// The derivative fact.
    pub delta_fact: DerivativeFact,
    /// Signed receipt.
    pub receipt: Receipt,
}

/// Compute the delta.
pub async fn diff(req: &DiffReq, srv: &Server) -> Result<DiffResp, StorageError> {
    let started = Instant::now();
    let storage = srv.storage.as_ref();

    let key_a = CanonicalKey {
        cell: req.cell.clone(),
        band: req.band.clone(),
        tslot: req.tslot_a,
    };
    let key_b = CanonicalKey {
        cell: req.cell.clone(),
        band: req.band.clone(),
        tslot: req.tslot_b,
    };
    let cids = storage
        .lookup_canonical_many(&[key_a.clone(), key_b.clone()])
        .await?;
    let cid_a = cids[0].clone().ok_or_else(|| StorageError::Protocol {
        code: ErrorCode::CidNotFound,
        message: format!(
            "no fact at tslot_a={} for ({},{})",
            req.tslot_a, req.cell, req.band
        ),
    })?;
    let cid_b = cids[1].clone().ok_or_else(|| StorageError::Protocol {
        code: ErrorCode::CidNotFound,
        message: format!(
            "no fact at tslot_b={} for ({},{})",
            req.tslot_b, req.cell, req.band
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
                message: "diff requires primary facts at both tslots".into(),
            })
        }
    };

    let value = if let (Some(av), Some(bv)) = (as_vec_f32(va), as_vec_f32(vb)) {
        let n = av.len().min(bv.len());
        let arr: Vec<ciborium::Value> = (0..n)
            .map(|i| ciborium::Value::Float((bv[i] - av[i]) as f64))
            .collect();
        ciborium::Value::Array(arr)
    } else if let (Some(an), Some(bn)) = (as_f64(va), as_f64(vb)) {
        ciborium::Value::Float(bn - an)
    } else {
        return Err(StorageError::Protocol {
            code: ErrorCode::Internal,
            message: format!("diff: band {} is neither numeric nor a vector", req.band),
        });
    };

    let derivative = DerivativeFact {
        cell: req.cell.clone(),
        band: req.band.clone(),
        tslot_window: [req.tslot_a, req.tslot_b],
        op: "delta".into(),
        parents: vec![cid_a.clone(), cid_b.clone()],
        value,
        confidence: 1.0,
        derivation: Derivation {
            fn_key: "nd.delta@1".into(),
            args: None,
        },
        schema_cid: srv.manifests.schema_cid.clone(),
        signer: srv.identity.pubkey,
        signed_at: iso8601_now(),
    };

    let receipt = srv.sign_receipt(
        "emem.diff",
        vec![req.cell.clone()],
        vec![cid_a, cid_b],
        true,
        started,
        None,
    );
    Ok(DiffResp {
        delta_fact: derivative,
        receipt,
    })
}
