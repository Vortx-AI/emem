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
    /// Signed receipt.
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
    let mut scored: Vec<Neighbor> = Vec::new();
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
                scored.push(Neighbor {
                    cell: key.cell,
                    score,
                });
            }
        }
    }

    // total_cmp gives a defined ordering for NaN scores (NaN sorts last),
    // so a single bad cosine doesn't silently shuffle the top-k.
    scored.retain(|n| !n.score.is_nan());
    scored.sort_by(|a, b| b.score.total_cmp(&a.score));
    scored.truncate(k);

    let cells: Vec<String> = scored.iter().map(|n| n.cell.clone()).collect();
    let receipt = srv.sign_receipt(
        "emem.find_similar",
        cells,
        Vec::<FactCid>::new(),
        true,
        started,
        None,
    );
    Ok(FindSimilarResp {
        neighbors: scored,
        receipt,
    })
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
    // re-rank cost dominated by the popcount scan.
    let triage_k = match req.mode {
        FindSimilarMode::HammingThenRerank => (k * 4).max(k),
        _ => k,
    };
    let self_match: Option<&str> = if req.key.starts_with("inline:") {
        None
    } else {
        Some(req.key.as_str())
    };
    let mut triage: Vec<(String, u32)> = Vec::new();
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
                triage.push((key.cell.clone(), dist));
            }
        }
    }
    triage.sort_by_key(|(_, d)| *d);
    triage.truncate(triage_k);

    let neighbors: Vec<Neighbor> = match req.mode {
        FindSimilarMode::Hamming => triage
            .into_iter()
            .map(|(cell, d)| Neighbor {
                cell,
                score: hamming_score(d),
            })
            .collect(),
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
            let mut reranked: Vec<Neighbor> = Vec::with_capacity(triage.len());
            for (cell, d) in triage {
                let vec = load_cell_vec(storage, &cell, cosine_band)
                    .await
                    .unwrap_or_default();
                let score = if vec.is_empty() || query_vec.is_empty() {
                    hamming_score(d)
                } else {
                    cosine(&query_vec, &vec)
                };
                reranked.push(Neighbor { cell, score });
            }
            reranked.retain(|n| !n.score.is_nan());
            reranked.sort_by(|a, b| b.score.total_cmp(&a.score));
            reranked.truncate(k);
            reranked
        }
        FindSimilarMode::Cosine => unreachable!("cosine handled above"),
    };

    let cells: Vec<String> = neighbors.iter().map(|n| n.cell.clone()).collect();
    let receipt = srv.sign_receipt(
        "emem.find_similar",
        cells,
        Vec::<FactCid>::new(),
        true,
        started,
        None,
    );
    Ok(FindSimilarResp {
        neighbors,
        receipt,
    })
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
