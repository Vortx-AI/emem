//! `compare(a, b, family?)` — spec §11 MCP `emem.compare`.
//!
//! Compares two cells over the bands they share. For vector-valued bands
//! the comparison metric is cosine similarity; for scalar bands it is
//! `b - a`. The summary `cosine` is the cosine over the concatenated
//! vector bands (or 0.0 when no vector band is shared).

use std::collections::BTreeMap;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use emem_fact::{Fact, FactCid, Receipt};
use emem_storage::{Server, StorageError};

use crate::cbor_ops::{as_f64, as_vec_f32, cosine, f32_to_cbor};

/// compare request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompareReq {
    /// Cell A (cell64).
    pub a: String,
    /// Cell B (cell64).
    pub b: String,
    /// Optional band-key prefix filter (e.g. `"indices."`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
}

/// compare response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompareResp {
    /// Cosine similarity over the concatenated vector bands shared by A
    /// and B. **`None` when the two cells share no vector band** — a
    /// previous version returned `0.0` here, which an agent can't tell
    /// apart from "orthogonal embeddings".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cosine: Option<f32>,
    /// Per-band delta (band → numeric delta or cosine over a vector band).
    pub per_band: BTreeMap<String, ciborium::Value>,
    /// Bands shared between A and B (intersection of bands at both cells).
    pub shared_bands: Vec<String>,
    /// Bands present at A but not at B. **Caveat:** this set reflects
    /// what has actually been recalled (warmed) at each cell, NOT
    /// absolute capability. A band that is materializer-wired at this
    /// responder but never recalled at cell B will appear in `only_a`
    /// even though cell B can be warmed for it on demand. See
    /// `asymmetry_note` for the recommended next call.
    pub only_a: Vec<String>,
    /// Bands present at B but not at A. Same caveat as `only_a`.
    pub only_b: Vec<String>,
    /// Set when no shared vector band existed; explains the missing cosine.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Surfaced when `only_a` or `only_b` is non-empty. Tells the agent
    /// the asymmetry might be pre-warm artefact, not a real capability
    /// gap. Without this hint, a caller will read `only_a=["clay_v1",...]`
    /// as "cell B has no Clay" when it just hasn't been recalled yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asymmetry_note: Option<String>,
    /// Signed receipt.
    pub receipt: Receipt,
}

/// Compute the comparison.
pub async fn compare(req: &CompareReq, srv: &Server) -> Result<CompareResp, StorageError> {
    let started = Instant::now();
    let storage = srv.storage.as_ref();

    let a_pairs = storage.scan_cell(&req.a, None).await?;
    let b_pairs = storage.scan_cell(&req.b, None).await?;
    let a_cids: Vec<FactCid> = a_pairs.iter().map(|(_, c)| c.clone()).collect();
    let b_cids: Vec<FactCid> = b_pairs.iter().map(|(_, c)| c.clone()).collect();

    let a_facts: Vec<Fact> = storage
        .get_facts_many(&a_cids)
        .await?
        .into_iter()
        .flatten()
        .collect();
    let b_facts: Vec<Fact> = storage
        .get_facts_many(&b_cids)
        .await?
        .into_iter()
        .flatten()
        .collect();

    let mut a_by_band: BTreeMap<String, ciborium::Value> = BTreeMap::new();
    let mut b_by_band: BTreeMap<String, ciborium::Value> = BTreeMap::new();
    for f in &a_facts {
        if let Fact::Primary(p) = f {
            if filter_band(&p.band, req.family.as_deref()) {
                a_by_band.insert(p.band.clone(), p.value.clone());
            }
        }
    }
    for f in &b_facts {
        if let Fact::Primary(p) = f {
            if filter_band(&p.band, req.family.as_deref()) {
                b_by_band.insert(p.band.clone(), p.value.clone());
            }
        }
    }

    let mut per_band: BTreeMap<String, ciborium::Value> = BTreeMap::new();
    let mut concat_a: Vec<f32> = Vec::new();
    let mut concat_b: Vec<f32> = Vec::new();
    for (band, va) in &a_by_band {
        let Some(vb) = b_by_band.get(band) else {
            continue;
        };
        if let (Some(av), Some(bv)) = (as_vec_f32(va), as_vec_f32(vb)) {
            let c = cosine(&av, &bv);
            per_band.insert(band.clone(), f32_to_cbor(c));
            let n = av.len().min(bv.len());
            concat_a.extend_from_slice(&av[..n]);
            concat_b.extend_from_slice(&bv[..n]);
        } else if let (Some(av), Some(bv)) = (as_f64(va), as_f64(vb)) {
            per_band.insert(band.clone(), ciborium::Value::Float(bv - av));
        }
    }

    let (summary_cos, note) = if concat_a.is_empty() {
        (
            None,
            Some(format!(
                "no vector band shared by both cells — cosine is undefined here. \
             Cells share {} scalar band(s): {:?}. To get a similarity score, \
             materialize a vector band like `geotessera` at both cells \
             (e.g. by passing it explicitly to /v1/recall) before comparing.",
                per_band.len(),
                per_band.keys().cloned().collect::<Vec<_>>(),
            )),
        )
    } else {
        (Some(cosine(&concat_a, &concat_b)), None)
    };

    let mut shared_bands: Vec<String> = a_by_band
        .keys()
        .filter(|k| b_by_band.contains_key(*k))
        .cloned()
        .collect();
    shared_bands.sort();
    let mut only_a: Vec<String> = a_by_band
        .keys()
        .filter(|k| !b_by_band.contains_key(*k))
        .cloned()
        .collect();
    only_a.sort();
    let mut only_b: Vec<String> = b_by_band
        .keys()
        .filter(|k| !a_by_band.contains_key(*k))
        .cloned()
        .collect();
    only_b.sort();

    let mut all_cids = a_cids.clone();
    all_cids.extend_from_slice(&b_cids);
    let receipt = srv.sign_receipt(
        "emem.compare",
        vec![req.a.clone(), req.b.clone()],
        all_cids,
        true,
        started,
        None,
    );
    let asymmetry_note = if !only_a.is_empty() || !only_b.is_empty() {
        Some(format!(
            "only_a/only_b reflect facts actually stored at each cell, not absolute capability. {} band(s) in only_a / {} in only_b may simply be unwarmed at the other cell. To confirm a band is genuinely absent, call POST /v1/recall with that band on the cell missing it; the materializer (if wired) will fetch it on miss and the band will move to shared_bands on re-compare.",
            only_a.len(),
            only_b.len()
        ))
    } else {
        None
    };

    Ok(CompareResp {
        cosine: summary_cos,
        per_band,
        shared_bands,
        only_a,
        only_b,
        note,
        asymmetry_note,
        receipt,
    })
}

fn filter_band(band: &str, family: Option<&str>) -> bool {
    match family {
        None => true,
        Some(prefix) => band.starts_with(prefix),
    }
}
