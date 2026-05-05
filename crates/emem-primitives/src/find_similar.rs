//! `find_similar(key, k?, filter?, band?)` — spec §11 MCP `emem.find_similar`.
//!
//! Implements the **vector-as-address** primitive from spec §3.4. The
//! `key` is either a `cell64` (look up that cell's embedding for the
//! given band) or `inline:[x,y,z,...]` for a literal vector. The corpus
//! is the canonical-key index over the configured `band` (defaults to
//! `"geotessera"` — the open 128-D foundation embedding emem materializes
//! by default).
//!
//! Brute-force k-NN over the index. For corpora >1M cells the operator
//! should layer a Lance / FAISS sidecar in front of this primitive; the
//! protocol surface (`Storage::iter_index`) is the same.

use std::time::Instant;

use serde::{Deserialize, Serialize};

use emem_claim::Claim;
use emem_core::ErrorCode;
use emem_fact::{Fact, FactCid, Receipt};
use emem_storage::{Server, StorageError};

use crate::binary_embedding::{
    hamming_distance, hamming_score, pack_bin128_slice, BIN_BYTES, BIN_DIMS,
};
use crate::cbor_ops::{as_vec_f32, cosine};

/// Scoring mode for [`find_similar`]. The default `cosine` is the
/// historical behaviour (fp32 cosine over the requested band).
/// `hamming` and `hamming_then_rerank` use the binary fast path —
/// 16 B/cell on disk, popcount scoring, ~1000× faster on the inner
/// loop. The rerank variant uses Hamming for triage on a wider
/// candidate set, then re-orders the top with full-vector cosine so
/// the precision matches cosine-only at a fraction of the cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FindSimilarMode {
    /// fp32 cosine over the full vector band. Default for backward
    /// compatibility — every existing client gets the same result
    /// shape it always did.
    #[default]
    Cosine,
    /// Hamming distance over the binary band derived from the
    /// requested band's family (e.g. `geotessera` → `geotessera.bin128`).
    /// Returns `score = 1 - 2 · dist / 128` so the ordering reads
    /// the same direction as cosine.
    Hamming,
    /// Pull `4 · k` candidates by Hamming, then re-rank that shortlist
    /// by cosine on the full vector. Matches cosine-only precision in
    /// practice while doing ~16× less work in the scan phase.
    HammingThenRerank,
}

/// find_similar request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindSimilarReq {
    /// `cell64` (e.g. `"damO.zb000.xUti.zde78"`) or `inline:[x,y,...]` vector literal.
    pub key: String,
    /// Number of neighbors (default 10, max 1000).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub k: Option<u32>,
    /// Vector band to scan (default `"geotessera"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub band: Option<String>,
    /// Optional structured filter expressed in the claim algebra. The
    /// claim evaluator is not yet wired into k-NN; setting this returns an
    /// explicit error so callers cannot mistake an unfiltered result for a
    /// filtered one. To filter today, post-filter with `/v1/verify` against
    /// the desired `Claim` on each returned neighbor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<Claim>,
    /// Scoring mode (default `cosine`). See [`FindSimilarMode`] for the
    /// trade-off between fp32 cosine and binary-quantized Hamming.
    #[serde(default, skip_serializing_if = "is_default_mode")]
    pub mode: FindSimilarMode,
}

fn is_default_mode(m: &FindSimilarMode) -> bool {
    *m == FindSimilarMode::default()
}

/// A single neighbor result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Neighbor {
    /// cell64 of the neighbor.
    pub cell: String,
    /// Cosine similarity score in [-1, 1].
    pub score: f32,
}

/// find_similar response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindSimilarResp {
    /// Ranked neighbors.
    pub neighbors: Vec<Neighbor>,
    /// What the caller asked for (`req.k`, defaulted/clamped). Surfaced
    /// alongside `returned_k` so an agent can detect honest truncation:
    /// when the corpus has fewer than `requested_k` distinct cells under
    /// the requested band, the responder returns what it has rather than
    /// padding — `returned_k < requested_k` is the signal.
    pub requested_k: u32,
    /// Number of distinct-cell neighbours actually returned. Always
    /// equals `neighbors.len()`. After per-cell deduplication this can
    /// be smaller than `requested_k` when the corpus is sparse.
    pub returned_k: u32,
    /// Signed receipt. `receipt.fact_cids` carries every Fact whose
    /// vector contributed to the final ranking — one per kept neighbour
    /// (the highest-scoring fact per cell after dedupe).
    pub receipt: Receipt,
}

/// Map a "cosine band" to its binary sibling. Today only the
/// 128-D `geotessera` family has a binary derivation
/// (`geotessera.bin128`). Future families that publish their own
/// binary band should extend this map.
pub fn binary_sibling(band: &str) -> Option<&'static str> {
    match band {
        "geotessera"
        | "geotessera.multi_year"
        | "geotessera.2017"
        | "geotessera.2018"
        | "geotessera.2019"
        | "geotessera.2020"
        | "geotessera.2021"
        | "geotessera.2022"
        | "geotessera.2023"
        | "geotessera.2024" => Some("geotessera.bin128"),
        _ => None,
    }
}

/// Run brute-force k-NN over the given band's vector facts.
pub async fn find_similar(
    req: &FindSimilarReq,
    srv: &Server,
) -> Result<FindSimilarResp, StorageError> {
    let started = Instant::now();
    let storage = srv.storage.as_ref();
    let k = req.k.unwrap_or(10).min(1000) as usize;
    let band = req.band.clone().unwrap_or_else(|| "geotessera".into());

    if req.filter.is_some() {
        return Err(StorageError::Protocol {
            code: ErrorCode::Internal,
            message: "find_similar: structured `filter` is not evaluated in k-NN today — post-filter via /v1/verify on each returned neighbor, or omit `filter` for unfiltered top-k".into(),
        });
    }

    // ── Binary fast path ───────────────────────────────────────────
    // Branch *before* loading the full vector — for `Hamming` we do
    // not need it at all, and for `HammingThenRerank` we delegate the
    // full-vector load to the rerank phase only on the shortlist.
    if matches!(
        req.mode,
        FindSimilarMode::Hamming | FindSimilarMode::HammingThenRerank
    ) {
        return find_similar_binary(req, srv, started, k, &band).await;
    }

    let query_vec: Vec<f32> = if let Some(rest) = req.key.strip_prefix("inline:") {
        parse_inline_vec(rest)?
    } else {
        load_cell_vec(storage, &req.key, &band).await?
    };

    if query_vec.is_empty() {
        // Surface what *is* attested at this cell so the agent can pick a
        // different band (or call /v1/recall to populate `geotessera`)
        // instead of silently falling back to "no neighbours".
        let cell_bands: Vec<String> = if req.key.starts_with("inline:") {
            Vec::new()
        } else {
            let pairs = storage.scan_cell(&req.key, None).await.unwrap_or_default();
            let mut b: Vec<String> = pairs.into_iter().map(|(k, _)| k.band).collect();
            b.sort();
            b.dedup();
            b
        };
        let hint = if cell_bands.is_empty() {
            format!(
                "cell {} has no attested facts at all — call /v1/recall first to materialize bands",
                req.key
            )
        } else {
            format!(
                "cell {} has bands {:?} but none under requested band {}. \
                 Either pass `band: \"<one_of_those>\"` or call /v1/recall with `bands: [\"{band}\"]` to materialize it.",
                req.key, cell_bands, band, band = band,
            )
        };
        return Err(StorageError::Protocol {
            code: ErrorCode::CidNotFound,
            message: format!(
                "find_similar: no vector found for key='{}' band='{}'. Hint: {}",
                req.key, band, hint
            ),
        });
    }

    let entries = storage.iter_index(None).await?;
    // Score every (cell, band, tslot) candidate, keeping the FactCid
    // alongside so the receipt can cite the exact Fact that contributed
    // to the ranking. Without that pairing the receipt's fact_cids would
    // be empty, breaking the protocol's "every read carries its
    // citations" contract.
    let mut scored: Vec<(Neighbor, FactCid)> = Vec::new();
    // When the query came from a cell64 (not an inline vector), the
    // top-1 will trivially be the query cell with cosine=1.0. That's a
    // self-match, not a useful neighbour — agents asking "find places
    // like X" mean "places *other than* X". Filter it.
    let self_match: Option<&str> = if req.key.starts_with("inline:") {
        None
    } else {
        Some(req.key.as_str())
    };
    for (key, cid) in entries {
        if key.band != band {
            continue;
        }
        if Some(key.cell.as_str()) == self_match {
            continue;
        }
        let facts = storage.get_facts_many(std::slice::from_ref(&cid)).await?;
        let Some(Some(fact)) = facts.into_iter().next() else {
            continue;
        };
        if let Fact::Primary(p) = fact {
            if let Some(vec) = as_vec_f32(&p.value) {
                let score = cosine(&query_vec, &vec);
                scored.push((
                    Neighbor {
                        cell: key.cell,
                        score,
                    },
                    cid,
                ));
            }
        }
    }

    // total_cmp gives a defined ordering for NaN scores (NaN sorts last),
    // so a single bad cosine doesn't silently shuffle the top-k.
    scored.retain(|(n, _)| !n.score.is_nan());
    let (kept, kept_cids) = dedupe_top_k_by_cell(scored, k);

    let cells: Vec<String> = kept.iter().map(|n| n.cell.clone()).collect();
    let returned_k = kept.len() as u32;
    let receipt = srv.sign_receipt("emem.find_similar", cells, kept_cids, true, started, None);
    Ok(FindSimilarResp {
        neighbors: kept,
        requested_k: k as u32,
        returned_k,
        receipt,
    })
}

/// Group scored candidates by `cell64`, keep the highest-scoring entry
/// per cell, sort the survivors by score descending, then truncate to
/// `k`. Returns the kept neighbours alongside the FactCids that backed
/// them so the receipt can cite each contributing fact.
///
/// The dedupe is the core of the P0 fix: `iter_index` returns one entry
/// per `(cell, band, tslot)` triple, so a cell with several historical
/// vintages would otherwise occupy multiple slots in the top-k. By
/// definition `find_similar` is a per-place ranker — per-band-vintage
/// detail belongs to `/v1/recall`. If the agent has fewer distinct
/// cells than they asked for, surfacing `returned_k < requested_k` is
/// the honest signal; we never pad with duplicates.
fn dedupe_top_k_by_cell(
    mut scored: Vec<(Neighbor, FactCid)>,
    k: usize,
) -> (Vec<Neighbor>, Vec<FactCid>) {
    // Sort by score descending so the first time we see each cell we
    // see its best score. total_cmp keeps the NaN-safe ordering applied
    // upstream.
    scored.sort_by(|a, b| b.0.score.total_cmp(&a.0.score));
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut kept: Vec<Neighbor> = Vec::with_capacity(k.min(scored.len()));
    let mut kept_cids: Vec<FactCid> = Vec::with_capacity(k.min(scored.len()));
    for (neighbor, cid) in scored {
        if kept.len() >= k {
            break;
        }
        if seen.insert(neighbor.cell.clone()) {
            kept.push(neighbor);
            kept_cids.push(cid);
        }
    }
    (kept, kept_cids)
}

fn parse_inline_vec(s: &str) -> Result<Vec<f32>, StorageError> {
    let trimmed = s.trim().trim_start_matches('[').trim_end_matches(']');
    let mut out = Vec::new();
    for tok in trimmed.split(',') {
        let t = tok.trim();
        if t.is_empty() {
            continue;
        }
        let f: f32 = t
            .parse()
            .map_err(|e: std::num::ParseFloatError| StorageError::Protocol {
                code: ErrorCode::Internal,
                message: format!("inline vector parse error '{t}': {e}"),
            })?;
        out.push(f);
    }
    Ok(out)
}

async fn load_cell_vec(
    storage: &(dyn emem_storage::Storage + Send + Sync),
    cell: &str,
    band: &str,
) -> Result<Vec<f32>, StorageError> {
    let entries = storage.scan_cell(cell, None).await?;
    let band_entries: Vec<&FactCid> = entries
        .iter()
        .filter(|(k, _)| k.band == band)
        .map(|(_, c)| c)
        .collect();

    // No facts exist on this cell for the requested band. Return empty
    // and let the caller produce a clean CidNotFound; an agent reading
    // that error knows to try a different (cell, band) combination.
    if band_entries.is_empty() {
        return Ok(Vec::new());
    }

    let cids: Vec<FactCid> = band_entries.into_iter().cloned().collect();
    let facts = storage.get_facts_many(&cids).await?;
    let mut found_band_but_not_vector = false;
    for f in facts.into_iter().flatten() {
        if let Fact::Primary(p) = f {
            if p.band == band {
                if let Some(v) = as_vec_f32(&p.value) {
                    return Ok(v);
                }
                found_band_but_not_vector = true;
            }
        }
    }
    // The band IS attested on this cell, but the value isn't a vector.
    // That's a schema mismatch, not an empty cell — distinguish it
    // explicitly so the agent doesn't think the cell is silent.
    if found_band_but_not_vector {
        return Err(StorageError::Protocol {
            code: emem_core::ErrorCode::Internal,
            message: format!(
                "find_similar: cell={cell} band={band} has attested facts but their `value` is not a Vec<f32>. \
                 Likely a band-type mismatch (expected vector, got numeric/struct). \
                 Use /v1/recall with bands=[{band:?}] to inspect what's stored."
            ),
        });
    }
    Ok(Vec::new())
}

/// Hamming-distance k-NN over the binary sibling band. Picks the
/// binary band by [`binary_sibling`] when the caller passed the cosine
/// band by name, or honours the binary band directly when the caller
/// passed e.g. `geotessera.bin128`. For `HammingThenRerank` the top
/// `4 · k` Hamming candidates are re-scored under cosine on the
/// underlying vector band so the final ranking matches cosine-only
/// precision while doing 16× less work in the scan phase.
async fn find_similar_binary(
    req: &FindSimilarReq,
    srv: &Server,
    started: Instant,
    k: usize,
    cosine_band: &str,
) -> Result<FindSimilarResp, StorageError> {
    let storage = srv.storage.as_ref();
    let bin_band: String = if cosine_band.ends_with(".bin128") {
        cosine_band.to_string()
    } else {
        match binary_sibling(cosine_band) {
            Some(b) => b.to_string(),
            None => {
                return Err(StorageError::Protocol {
                    code: ErrorCode::Internal,
                    message: format!(
                        "find_similar: mode=hamming requested for band='{cosine_band}', \
                         but no binary sibling is registered. Today only the geotessera \
                         family has a `.bin128` derivation. Either drop the mode \
                         (defaults to cosine) or pass `band: \"<family>.bin128\"` directly."
                    ),
                });
            }
        }
    };

    let query_bytes: [u8; BIN_BYTES] = if let Some(rest) = req.key.strip_prefix("inline:") {
        let v = parse_inline_vec(rest)?;
        pack_bin128_slice(&v).ok_or_else(|| StorageError::Protocol {
            code: ErrorCode::Internal,
            message: format!(
                "find_similar: inline vector for hamming mode must have exactly \
                 {BIN_DIMS} dims, got {}",
                v.len()
            ),
        })?
    } else {
        load_cell_bin128(storage, &req.key, &bin_band, cosine_band).await?
    };

    let entries = storage.iter_index(None).await?;
    // Triage uses a wider candidate window when we plan to re-rank.
    // 4× k is the same window TerraBit's writeup recommends — large
    // enough that the cosine re-rank can recover most of the recall
    // loss from sign-bit quantization, small enough to keep the
    // re-rank cost dominated by the popcount scan. We oversample the
    // raw scan by an extra factor so per-cell dedupe (multiple tslots
    // per cell collapse to one) doesn't shrink the survivor pool below
    // `triage_k` distinct cells before the cosine rerank phase.
    let triage_k = match req.mode {
        FindSimilarMode::HammingThenRerank => (k * 4).max(k),
        _ => k,
    };
    let self_match: Option<&str> = if req.key.starts_with("inline:") {
        None
    } else {
        Some(req.key.as_str())
    };
    let mut triage: Vec<(String, u32, FactCid)> = Vec::new();
    for (key, cid) in entries {
        if key.band != bin_band {
            continue;
        }
        if Some(key.cell.as_str()) == self_match {
            continue;
        }
        let facts = storage.get_facts_many(std::slice::from_ref(&cid)).await?;
        let Some(Some(fact)) = facts.into_iter().next() else {
            continue;
        };
        if let Fact::Primary(p) = fact {
            if let Some(bytes) = as_bin128(&p.value) {
                let dist = hamming_distance(&query_bytes, &bytes);
                triage.push((key.cell.clone(), dist, cid));
            }
        }
    }
    // Sort by distance ascending, then dedupe by cell64 so each cell's
    // best (closest) Hamming candidate survives. This is the per-cell
    // ranker promise; per-vintage detail belongs to /v1/recall.
    triage.sort_by_key(|(_, d, _)| *d);
    let triage = dedupe_triage_by_cell(triage, triage_k);

    let (neighbors, fact_cids): (Vec<Neighbor>, Vec<FactCid>) = match req.mode {
        FindSimilarMode::Hamming => triage
            .into_iter()
            .map(|(cell, d, cid)| {
                (
                    Neighbor {
                        cell,
                        score: hamming_score(d),
                    },
                    cid,
                )
            })
            .unzip(),
        FindSimilarMode::HammingThenRerank => {
            // Pull the underlying cosine vector for each shortlisted
            // cell and re-score. A cell that surfaced under Hamming
            // but has no cosine fact (e.g. only the binary band was
            // attested) keeps its Hamming score so it isn't silently
            // dropped — the agent can spot that case via the
            // `score < 1.0` inverse of the Hamming → cosine mapping.
            let query_vec: Vec<f32> = if let Some(rest) = req.key.strip_prefix("inline:") {
                parse_inline_vec(rest)?
            } else {
                load_cell_vec(storage, &req.key, cosine_band).await?
            };
            let mut reranked: Vec<(Neighbor, FactCid)> = Vec::with_capacity(triage.len());
            for (cell, d, cid) in triage {
                let vec = load_cell_vec(storage, &cell, cosine_band)
                    .await
                    .unwrap_or_default();
                let score = if vec.is_empty() || query_vec.is_empty() {
                    hamming_score(d)
                } else {
                    cosine(&query_vec, &vec)
                };
                reranked.push((Neighbor { cell, score }, cid));
            }
            reranked.retain(|(n, _)| !n.score.is_nan());
            // The triage list is already cell-unique, so the rerank
            // dedupe is a no-op for shape — but routing through the
            // shared helper keeps the sort + truncate-to-k contract in
            // one place and survives a future change to triage.
            let (kept, kept_cids) = dedupe_top_k_by_cell(reranked, k);
            (kept, kept_cids)
        }
        FindSimilarMode::Cosine => unreachable!("cosine handled above"),
    };

    let cells: Vec<String> = neighbors.iter().map(|n| n.cell.clone()).collect();
    let returned_k = neighbors.len() as u32;
    let receipt = srv.sign_receipt("emem.find_similar", cells, fact_cids, true, started, None);
    Ok(FindSimilarResp {
        neighbors,
        requested_k: k as u32,
        returned_k,
        receipt,
    })
}

/// Hamming-path dedupe: triage is already sorted by distance ascending,
/// so the first time we see a cell is its best Hamming candidate. Keep
/// up to `triage_k` distinct cells.
fn dedupe_triage_by_cell(
    triage: Vec<(String, u32, FactCid)>,
    triage_k: usize,
) -> Vec<(String, u32, FactCid)> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut kept: Vec<(String, u32, FactCid)> = Vec::with_capacity(triage_k.min(triage.len()));
    for entry in triage {
        if kept.len() >= triage_k {
            break;
        }
        if seen.insert(entry.0.clone()) {
            kept.push(entry);
        }
    }
    kept
}

/// Coerce a CBOR value to a 16-byte binary embedding. The
/// canonical encoding is `Bytes(16)` (CBOR major type 2), but we
/// also accept `Array(int)` of length 16 because some early-versioned
/// materialisers / cross-language clients may serialise via the
/// numeric array path.
fn as_bin128(v: &ciborium::Value) -> Option<[u8; BIN_BYTES]> {
    match v {
        ciborium::Value::Bytes(b) if b.len() == BIN_BYTES => {
            let mut out = [0u8; BIN_BYTES];
            out.copy_from_slice(b);
            Some(out)
        }
        ciborium::Value::Array(a) if a.len() == BIN_BYTES => {
            let mut out = [0u8; BIN_BYTES];
            for (i, x) in a.iter().enumerate() {
                let n = match x {
                    ciborium::Value::Integer(i) => i128::from(*i),
                    _ => return None,
                };
                if !(0..=255).contains(&n) {
                    return None;
                }
                out[i] = n as u8;
            }
            Some(out)
        }
        _ => None,
    }
}

/// Load a binary embedding fact from storage. Returns the 16-byte
/// payload or a structured error explaining what's actually stored
/// at this (cell, band) pair so the agent can decide whether to
/// materialise the binary sibling first.
async fn load_cell_bin128(
    storage: &(dyn emem_storage::Storage + Send + Sync),
    cell: &str,
    bin_band: &str,
    cosine_band: &str,
) -> Result<[u8; BIN_BYTES], StorageError> {
    let entries = storage.scan_cell(cell, None).await?;
    let bin_entries: Vec<&FactCid> = entries
        .iter()
        .filter(|(k, _)| k.band == bin_band)
        .map(|(_, c)| c)
        .collect();

    if bin_entries.is_empty() {
        return Err(StorageError::Protocol {
            code: ErrorCode::CidNotFound,
            message: format!(
                "find_similar (hamming): cell={cell} has no '{bin_band}' fact. \
                 Materialize it first: POST /v1/recall {{\"cell\":\"{cell}\", \
                 \"bands\":[\"{bin_band}\"]}} (the responder will derive it from \
                 the underlying '{cosine_band}' vector if present).",
            ),
        });
    }
    let cids: Vec<FactCid> = bin_entries.into_iter().cloned().collect();
    let facts = storage.get_facts_many(&cids).await?;
    for f in facts.into_iter().flatten() {
        if let Fact::Primary(p) = f {
            if p.band == bin_band {
                if let Some(bytes) = as_bin128(&p.value) {
                    return Ok(bytes);
                }
            }
        }
    }
    Err(StorageError::Protocol {
        code: ErrorCode::Internal,
        message: format!(
            "find_similar (hamming): cell={cell} band={bin_band} attested but \
             value is not a 16-byte binary embedding. The fact may have been \
             materialized under an older codec — re-attest with the current \
             turboquant_geotessera_bin128_v1@1 derivation.",
        ),
    })
}

#[cfg(test)]
mod tests {
    //! Tests for the P0 fix: top-k MUST contain at most one entry per
    //! cell64 (per-place ranker contract), and the receipt MUST cite
    //! every fact whose vector contributed to the ranking. We avoid the
    //! full `MaterializingStorage` path here so the test stays
    //! dependency-free and fast — a hand-rolled in-memory `Storage`
    //! impl gives us complete control over the corpus shape (multiple
    //! tslots per cell, single-cell corpora) the dedupe logic needs to
    //! be driven against.
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::time::SystemTime;
    use std::time::UNIX_EPOCH;

    use async_trait::async_trait;
    use ciborium::Value as CborValue;

    use emem_cache::CanonicalKey;
    use emem_core::AttesterKey;
    use emem_fact::{Derivation, Fact, FactCid, PrimaryFact, RegistryCid, SchemaCid, Source};
    use emem_storage::server::{ManifestCids, ResponderIdentity};
    use emem_storage::{Server, Storage, StorageError};

    /// Minimal Storage impl backed by an in-memory map. Only the
    /// surface `find_similar` actually uses (`iter_index`, `scan_cell`,
    /// `get_facts_many`) is wired; everything else returns `Internal`
    /// so a regression that starts to depend on, say, materialize_many
    /// surfaces loudly instead of silently passing.
    struct MockStorage {
        // CanonicalKey -> (FactCid, Fact)
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

        /// Insert a fp32 vector fact at (cell, band, tslot). Each
        /// invocation generates a fresh FactCid so duplicate (cell,
        /// band) pairs at different tslots stay distinct in the index.
        fn insert_vector(&self, cell: &str, band: &str, tslot: u64, vec: Vec<f32>) -> FactCid {
            let cid_str = format!("test-cid-{}", next_id());
            let cid = FactCid::new(&cid_str);
            let fact = Fact::Primary(PrimaryFact {
                cell: cell.into(),
                band: band.into(),
                tslot,
                value: CborValue::Array(
                    vec.into_iter()
                        .map(|v| CborValue::Float(v as f64))
                        .collect(),
                ),
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
            _keys: &[CanonicalKey],
        ) -> Result<Vec<Option<FactCid>>, StorageError> {
            unimplemented!("lookup_canonical_many not used by find_similar")
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
            unimplemented!("put_attestation not used by find_similar")
        }

        async fn materialize_many(
            &self,
            _keys: &[CanonicalKey],
        ) -> Result<Vec<FactCid>, StorageError> {
            unimplemented!("materialize_many not used by find_similar")
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
            limit: Option<usize>,
        ) -> Result<Vec<(CanonicalKey, FactCid)>, StorageError> {
            let entries = self.entries.lock().unwrap();
            let mut out: Vec<_> = entries
                .iter()
                .map(|(k, c, _)| (k.clone(), c.clone()))
                .collect();
            if let Some(n) = limit {
                out.truncate(n);
            }
            Ok(out)
        }
    }

    fn next_id() -> u64 {
        // Process-local monotonically-increasing id, unique across
        // tests in this module. Avoids depending on `uuid` for a
        // purely-internal test handle.
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u64)
            .unwrap_or(0);
        COUNTER.fetch_add(1, Ordering::Relaxed) ^ nanos
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

    /// P0 acceptance (a) + (b): with multiple facts at the same
    /// (cell, band) but different tslots, the top-k MUST contain
    /// each cell64 at most once, and `receipt.fact_cids` MUST be
    /// non-empty when the result list is non-empty.
    #[tokio::test]
    async fn dedupes_by_cell_and_populates_fact_cids() {
        let storage = Arc::new(MockStorage::new());
        // Three "places" — query cell + 2 candidate cells. Each
        // candidate gets two vintages (tslot 0 and tslot 1) of nearly
        // identical vectors to model a cell that has multiple historical
        // attestations under the same band.
        storage.insert_vector("cell-query", "geotessera", 0, vec![1.0, 0.0, 0.0, 0.0]);
        storage.insert_vector("cell-a", "geotessera", 0, vec![0.99, 0.01, 0.0, 0.0]);
        storage.insert_vector("cell-a", "geotessera", 1, vec![0.98, 0.02, 0.0, 0.0]);
        storage.insert_vector("cell-b", "geotessera", 0, vec![0.5, 0.5, 0.0, 0.0]);
        storage.insert_vector("cell-b", "geotessera", 1, vec![0.49, 0.51, 0.0, 0.0]);
        storage.insert_vector("cell-b", "geotessera", 2, vec![0.48, 0.52, 0.0, 0.0]);

        let srv = test_server(storage);
        let req = FindSimilarReq {
            key: "cell-query".into(),
            k: Some(8),
            band: Some("geotessera".into()),
            filter: None,
            mode: FindSimilarMode::Cosine,
        };
        let resp = find_similar(&req, &srv).await.expect("find_similar ok");

        // (a) no duplicate cell64 in the neighbours list.
        let mut cells: Vec<&str> = resp.neighbors.iter().map(|n| n.cell.as_str()).collect();
        cells.sort();
        let unique_count = {
            let mut c = cells.clone();
            c.dedup();
            c.len()
        };
        assert_eq!(
            cells.len(),
            unique_count,
            "find_similar must return at most one entry per cell64; got {cells:?}"
        );

        // (b) receipt.fact_cids non-empty when neighbours non-empty.
        assert!(!resp.neighbors.is_empty(), "expected ≥1 neighbour");
        assert!(
            !resp.receipt.fact_cids.is_empty(),
            "receipt.fact_cids must cite every Fact whose vector entered the ranking; got empty"
        );
        // One cited fact per kept neighbour (the highest-scoring vintage
        // per cell). This is the protocol's "every read carries its
        // citations" contract for find_similar.
        assert_eq!(
            resp.receipt.fact_cids.len(),
            resp.neighbors.len(),
            "fact_cids length must match neighbours length (one cite per kept fact)"
        );
        // The cell list embedded in the signed receipt must mirror the
        // deduped neighbours order — that's what got signed.
        assert_eq!(
            resp.receipt.cells,
            resp.neighbors
                .iter()
                .map(|n| n.cell.clone())
                .collect::<Vec<_>>()
        );

        // requested_k / returned_k surface honest accounting.
        assert_eq!(resp.requested_k, 8);
        assert_eq!(resp.returned_k, resp.neighbors.len() as u32);
    }

    /// P0 acceptance (c): when fewer unique cells exist than k, the
    /// response MUST surface `returned_k < requested_k` so the agent
    /// can detect honest truncation. We do NOT pad with duplicates.
    #[tokio::test]
    async fn fewer_cells_than_k_surfaces_returned_k_less_than_requested() {
        let storage = Arc::new(MockStorage::new());
        // 1 query + 2 distinct candidate cells. Asking for k=20.
        storage.insert_vector("q", "geotessera", 0, vec![1.0, 0.0]);
        storage.insert_vector("a", "geotessera", 0, vec![0.9, 0.1]);
        storage.insert_vector("a", "geotessera", 1, vec![0.85, 0.15]);
        storage.insert_vector("b", "geotessera", 0, vec![0.1, 0.9]);

        let srv = test_server(storage);
        let req = FindSimilarReq {
            key: "q".into(),
            k: Some(20),
            band: Some("geotessera".into()),
            filter: None,
            mode: FindSimilarMode::Cosine,
        };
        let resp = find_similar(&req, &srv).await.expect("find_similar ok");

        assert_eq!(resp.requested_k, 20);
        assert_eq!(resp.returned_k, 2, "only 2 distinct non-self cells exist");
        assert!(
            resp.returned_k < resp.requested_k,
            "must surface returned_k<requested_k for honest truncation"
        );
        assert_eq!(resp.neighbors.len(), 2);
        assert_eq!(resp.receipt.fact_cids.len(), 2);
    }

    /// A k=1 request still gets dedupe right (no panic, no double-up).
    #[tokio::test]
    async fn k_one_returns_single_unique_cell() {
        let storage = Arc::new(MockStorage::new());
        storage.insert_vector("q", "geotessera", 0, vec![1.0, 0.0]);
        storage.insert_vector("a", "geotessera", 0, vec![0.99, 0.01]);
        storage.insert_vector("a", "geotessera", 1, vec![0.98, 0.02]);
        storage.insert_vector("a", "geotessera", 2, vec![0.97, 0.03]);
        storage.insert_vector("b", "geotessera", 0, vec![0.5, 0.5]);

        let srv = test_server(storage);
        let req = FindSimilarReq {
            key: "q".into(),
            k: Some(1),
            band: Some("geotessera".into()),
            filter: None,
            mode: FindSimilarMode::Cosine,
        };
        let resp = find_similar(&req, &srv).await.expect("find_similar ok");

        assert_eq!(resp.returned_k, 1);
        assert_eq!(resp.neighbors.len(), 1);
        assert_eq!(resp.receipt.fact_cids.len(), 1);
        // a is closer to q than b is, so it should win.
        assert_eq!(resp.neighbors[0].cell, "a");
    }

    /// Direct unit on the dedupe helper — guards against a future
    /// refactor that drops the dedupe step or changes the
    /// "highest-score-per-cell wins" tie-break.
    #[test]
    fn dedupe_helper_keeps_best_score_per_cell() {
        let cid_a1 = FactCid::new("a-1");
        let cid_a2 = FactCid::new("a-2");
        let cid_b = FactCid::new("b");
        let scored = vec![
            (
                Neighbor {
                    cell: "a".into(),
                    score: 0.5,
                },
                cid_a1.clone(),
            ),
            (
                Neighbor {
                    cell: "a".into(),
                    score: 0.9,
                },
                cid_a2.clone(),
            ),
            (
                Neighbor {
                    cell: "b".into(),
                    score: 0.7,
                },
                cid_b.clone(),
            ),
        ];
        let (kept, kept_cids) = dedupe_top_k_by_cell(scored, 5);
        assert_eq!(kept.len(), 2);
        // Sorted by score descending: a (0.9) then b (0.7).
        assert_eq!(kept[0].cell, "a");
        assert_eq!(kept[0].score, 0.9);
        assert_eq!(
            kept_cids[0], cid_a2,
            "must cite the higher-scoring vintage of cell a"
        );
        assert_eq!(kept[1].cell, "b");
        assert_eq!(kept_cids[1], cid_b);
    }
}
