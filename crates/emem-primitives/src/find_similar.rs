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

use crate::cbor_ops::{as_vec_f32, cosine};

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
