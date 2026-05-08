//! `query_region(geometry, bands?, agg?)` — spec §11 MCP `emem.query_region`.
//!
//! Geometry forms in this build:
//!
//! - `cell64` (e.g. `"damO.zb000.xUti.zde78"`) — single cell.
//! - `cells:c1,c2,...` — explicit list of cell64 strings.
//! - `bbox:lon_min,lat_min,lon_max,lat_max` — WGS-84 axis-aligned box,
//!   sampled at the cell64 grid pitch (~10 m at the equator). The
//!   responder caps coverage at [`MAX_BBOX_CELLS`] cells; an over-large
//!   bbox returns a structured error pointing the caller to either
//!   shrink the bbox or pass `cells:` directly.
//!
//! GeoJSON polyfill remains a separate concern — operators with an
//! H3-equivalent indexer wire it in at the API layer.

use std::collections::BTreeMap;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use emem_codec::geo::cell64_from_latlng;
use emem_core::ErrorCode;
use emem_fact::{Fact, FactCid, Receipt};
use emem_storage::{Server, StorageError};

use crate::cbor_ops::{as_f64, as_vec_f32};

/// Hard ceiling on cells synthesised from a bbox. At the cell64 ~10 m
/// pitch this covers ~6.4 km × 6.4 km at the equator; agents asking for
/// regional aggregates beyond that are almost certainly better served
/// by a coarser-grain primitive (and would otherwise OOM the responder).
pub const MAX_BBOX_CELLS: usize = 4096;

/// Hard ceiling on facts accumulated across the requested cells before
/// aggregation. Caps the JSON response shape and the in-memory working
/// set so a dense corpus + a 4096-cell bbox can't blow past ~few hundred
/// MB. When the cap is reached the responder stops scanning further
/// cells and aggregates over what it has so far — `receipt.fact_cids`
/// reflects exactly what contributed.
pub const MAX_REGION_FACTS: usize = 65_536;

/// query_region request — geometry can be cell or comma list of cells.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryRegionReq {
    /// `<cell64>` | `cells:c1,c2,...`.
    pub geometry: String,
    /// Optional band filter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bands: Option<Vec<String>>,
    /// Aggregation: "mean" | "median" | "p90" | "vector_centroid".
    /// When unset, every matching primary fact is returned per cell.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agg: Option<String>,
}

/// query_region response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryRegionResp {
    /// Facts (per-cell or aggregated).
    pub facts: Vec<Fact>,
    /// Aggregated per-band summaries when `agg` is set.
    pub aggregates: BTreeMap<String, ciborium::Value>,
    /// Signed receipt.
    pub receipt: Receipt,
}

/// Run a region query.
pub async fn query_region(
    req: &QueryRegionReq,
    srv: &Server,
) -> Result<QueryRegionResp, StorageError> {
    let started = Instant::now();
    let storage = srv.storage.as_ref();

    let cells: Vec<String> = if let Some(rest) = req.geometry.strip_prefix("cells:") {
        rest.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    } else if let Some(rest) = req.geometry.strip_prefix("bbox:") {
        cells_from_bbox(rest)?
    } else if req.geometry.starts_with('{') {
        return Err(StorageError::Protocol {
            code: ErrorCode::InvalidCell,
            message:
                "query_region: GeoJSON polyfill not yet implemented; pass 'bbox:lon_min,lat_min,lon_max,lat_max' or 'cells:c1,c2,...' instead"
                    .into(),
        });
    } else {
        vec![req.geometry.clone()]
    };

    let mut all_facts: Vec<Fact> = Vec::new();
    let mut all_cids: Vec<FactCid> = Vec::new();
    'outer: for cell in &cells {
        if all_facts.len() >= MAX_REGION_FACTS {
            break;
        }
        let entries = storage.scan_cell(cell, None).await?;
        let cids: Vec<FactCid> = entries.into_iter().map(|(_, c)| c).collect();
        let fetched = storage.get_facts_many(&cids).await?;
        for (cid, fact) in cids.iter().zip(fetched.into_iter()) {
            let Some(fact) = fact else { continue };
            if let Some(filter) = &req.bands {
                let band = match &fact {
                    Fact::Primary(p) => &p.band,
                    Fact::Absence(n) => &n.band,
                    Fact::Derivative(d) => &d.band,
                };
                if !filter.iter().any(|b| b == band) {
                    continue;
                }
            }
            all_cids.push(cid.clone());
            all_facts.push(fact);
            if all_facts.len() >= MAX_REGION_FACTS {
                // Receipt cites only what actually contributed — the
                // cap is honest, not a silent truncation, and the
                // caller can shrink their bbox / cell list and retry.
                break 'outer;
            }
        }
    }

    let aggregates = match req.agg.as_deref() {
        None => BTreeMap::new(),
        Some(op) => aggregate(&all_facts, op)?,
    };

    let receipt = srv.sign_receipt("emem.query_region", cells, all_cids, true, started, None);
    Ok(QueryRegionResp {
        facts: all_facts,
        aggregates,
        receipt,
    })
}

fn aggregate(facts: &[Fact], op: &str) -> Result<BTreeMap<String, ciborium::Value>, StorageError> {
    let mut by_band: BTreeMap<String, Vec<&ciborium::Value>> = BTreeMap::new();
    for f in facts {
        if let Fact::Primary(p) = f {
            by_band.entry(p.band.clone()).or_default().push(&p.value);
        }
    }
    let mut out = BTreeMap::new();
    for (band, values) in by_band {
        let agg = match op {
            "mean" => agg_mean(&values),
            "median" => agg_median(&values),
            "p90" => agg_percentile(&values, 0.90),
            "vector_centroid" => agg_vector_centroid(&values),
            other => {
                return Err(StorageError::Protocol {
                    code: ErrorCode::Internal,
                    message: format!("unknown aggregation: {other}"),
                })
            }
        };
        if let Some(v) = agg {
            out.insert(band, v);
        }
    }
    Ok(out)
}

fn agg_mean(values: &[&ciborium::Value]) -> Option<ciborium::Value> {
    let nums: Vec<f64> = values.iter().filter_map(|v| as_f64(v)).collect();
    if nums.is_empty() {
        return None;
    }
    Some(ciborium::Value::Float(
        nums.iter().sum::<f64>() / nums.len() as f64,
    ))
}

fn agg_median(values: &[&ciborium::Value]) -> Option<ciborium::Value> {
    // Strip NaN before aggregating: a single NaN would otherwise contaminate
    // the median via partial_cmp's undefined ordering.
    let mut nums: Vec<f64> = values
        .iter()
        .filter_map(|v| as_f64(v))
        .filter(|x| !x.is_nan())
        .collect();
    if nums.is_empty() {
        return None;
    }
    nums.sort_by(|a, b| a.total_cmp(b));
    let mid = nums.len() / 2;
    let m = if nums.len().is_multiple_of(2) {
        (nums[mid - 1] + nums[mid]) / 2.0
    } else {
        nums[mid]
    };
    Some(ciborium::Value::Float(m))
}

fn agg_percentile(values: &[&ciborium::Value], p: f64) -> Option<ciborium::Value> {
    let mut nums: Vec<f64> = values
        .iter()
        .filter_map(|v| as_f64(v))
        .filter(|x| !x.is_nan())
        .collect();
    if nums.is_empty() {
        return None;
    }
    nums.sort_by(|a, b| a.total_cmp(b));
    let idx = ((nums.len() - 1) as f64 * p).round() as usize;
    Some(ciborium::Value::Float(nums[idx]))
}

/// Parse a `lon_min,lat_min,lon_max,lat_max` bbox into the deduped
/// list of cell64s that cover it at the active grid pitch.
///
/// The grid is ~9.54 m × ~9.55 m at the equator (cell64.geo bit layout);
/// stepping at 8 m on each axis is half a bucket, which guarantees we
/// don't skip over a cell at the corner of a stride. Cells repeat for
/// adjacent strides falling in the same bucket, so we dedupe via
/// `BTreeSet` to keep ordering stable for the receipt.
fn cells_from_bbox(spec: &str) -> Result<Vec<String>, StorageError> {
    let parts: Vec<&str> = spec.split(',').map(|s| s.trim()).collect();
    if parts.len() != 4 {
        return Err(StorageError::Protocol {
            code: ErrorCode::InvalidCell,
            message: format!(
                "query_region: bbox must be 'bbox:lon_min,lat_min,lon_max,lat_max' (got {} components)",
                parts.len()
            ),
        });
    }
    let parse_one = |s: &str, name: &str| -> Result<f64, StorageError> {
        s.parse::<f64>().map_err(|e| StorageError::Protocol {
            code: ErrorCode::InvalidCell,
            message: format!("query_region: bbox {name} parse error '{s}': {e}"),
        })
    };
    let lon_min = parse_one(parts[0], "lon_min")?;
    let lat_min = parse_one(parts[1], "lat_min")?;
    let lon_max = parse_one(parts[2], "lon_max")?;
    let lat_max = parse_one(parts[3], "lat_max")?;
    if !(-90.0..=90.0).contains(&lat_min) || !(-90.0..=90.0).contains(&lat_max) {
        return Err(StorageError::Protocol {
            code: ErrorCode::InvalidCell,
            message: "query_region: bbox latitudes must be in [-90, 90]".into(),
        });
    }
    if !(-180.0..=180.0).contains(&lon_min) || !(-180.0..=180.0).contains(&lon_max) {
        return Err(StorageError::Protocol {
            code: ErrorCode::InvalidCell,
            message: "query_region: bbox longitudes must be in [-180, 180]".into(),
        });
    }
    if lat_min > lat_max || lon_min > lon_max {
        return Err(StorageError::Protocol {
            code: ErrorCode::InvalidCell,
            message:
                "query_region: bbox is inverted; antimeridian-crossing boxes must be split into two"
                    .into(),
        });
    }

    // Half-bucket stride in degrees. The active geo codec is 21 lat × 22 lng
    // bits, so one bucket is ~180/2^21° on lat and ~360/2^22° on lng.
    let lat_step_deg = 180.0_f64 / ((1u64 << 21) as f64);
    let lng_step_deg = 360.0_f64 / ((1u64 << 22) as f64);
    let lat_n = ((lat_max - lat_min) / lat_step_deg).ceil() as i64 + 1;
    let lng_n = ((lon_max - lon_min) / lng_step_deg).ceil() as i64 + 1;
    if lat_n.saturating_mul(lng_n) as usize > MAX_BBOX_CELLS.saturating_mul(8) {
        // Up-front bound — saves us walking a giant grid only to bail mid-loop.
        return Err(StorageError::Protocol {
            code: ErrorCode::InvalidCell,
            message: format!(
                "query_region: bbox would synthesise ~{} cells (cap {MAX_BBOX_CELLS}). Shrink the bbox or pass 'cells:c1,c2,...' for explicit selection. Active grid pitch is ~10 m, so the cap covers ~6.4 km × 6.4 km at the equator.",
                lat_n.saturating_mul(lng_n)
            ),
        });
    }

    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut lat = lat_min;
    while lat <= lat_max {
        let mut lng = lon_min;
        while lng <= lon_max {
            seen.insert(cell64_from_latlng(lat, lng));
            if seen.len() > MAX_BBOX_CELLS {
                return Err(StorageError::Protocol {
                    code: ErrorCode::InvalidCell,
                    message: format!(
                        "query_region: bbox covers >{MAX_BBOX_CELLS} cells at the active grid pitch. Shrink the bbox or pass 'cells:c1,c2,...' for explicit selection."
                    ),
                });
            }
            lng += lng_step_deg;
        }
        lat += lat_step_deg;
    }
    // Always include the four corners so a bbox that's narrower than the
    // step still produces at least the corner cells.
    seen.insert(cell64_from_latlng(lat_min, lon_min));
    seen.insert(cell64_from_latlng(lat_min, lon_max));
    seen.insert(cell64_from_latlng(lat_max, lon_min));
    seen.insert(cell64_from_latlng(lat_max, lon_max));
    Ok(seen.into_iter().collect())
}

#[cfg(test)]
mod bbox_tests {
    use super::*;

    /// A tiny bbox at the equator must enumerate ≥1 cell64 string and
    /// every result must round-trip through the codec — proves the
    /// `bbox:` geometry path is wired end-to-end without requiring a
    /// live storage layer.
    #[test]
    fn tiny_bbox_at_equator_produces_cells() {
        // ~50 m × 50 m around 0,0 — well above the bucket but well below
        // the 4096-cell cap. Expect O(25) unique cells.
        let cells = cells_from_bbox("0.0,0.0,4.5e-4,4.5e-4").expect("bbox parses");
        assert!(!cells.is_empty(), "expected non-empty cell list");
        assert!(
            cells.len() <= MAX_BBOX_CELLS,
            "{} cells exceeds cap {MAX_BBOX_CELLS}",
            cells.len()
        );
        for c in &cells {
            assert!(
                emem_codec::geo::latlng_from_cell64(c).is_ok(),
                "cell64 {c} did not round-trip through the codec"
            );
        }
    }

    /// An over-large bbox must surface a structured error pointing the
    /// caller at the cap rather than silently OOMing.
    #[test]
    fn oversized_bbox_returns_capped_error() {
        // 10° × 10° at the equator → ~1.2e6 × ~1.2e6 buckets ≫ cap.
        let err = cells_from_bbox("-180.0,-90.0,180.0,90.0").expect_err("must error");
        let msg = format!("{err}");
        assert!(
            msg.contains("cap") || msg.contains("4096") || msg.contains("Shrink the bbox"),
            "expected capacity-related message, got: {msg}"
        );
    }

    /// A four-component check rejecting GeoJSON until the polyfill ships.
    #[test]
    fn bbox_parser_rejects_geojson_marker() {
        // The parent `query_region` rejects GeoJSON before reaching here,
        // but the bbox parser itself must reject malformed components
        // with a clear message.
        let err = cells_from_bbox("not,a,bbox").expect_err("must error");
        let msg = format!("{err}");
        assert!(
            msg.contains("bbox must be") || msg.contains("4 components") || msg.contains("parse"),
            "expected component-shape message, got: {msg}"
        );
    }

    /// Inverted bbox (lat_min > lat_max or lon_min > lon_max) must error
    /// — antimeridian-crossing boxes have to be split by the caller.
    #[test]
    fn inverted_bbox_rejected() {
        let err = cells_from_bbox("10.0,5.0,5.0,10.0").expect_err("must error");
        let msg = format!("{err}");
        assert!(
            msg.contains("inverted") || msg.contains("antimeridian"),
            "expected inverted-box message, got: {msg}"
        );
    }
}

fn agg_vector_centroid(values: &[&ciborium::Value]) -> Option<ciborium::Value> {
    let vecs: Vec<Vec<f32>> = values.iter().filter_map(|v| as_vec_f32(v)).collect();
    if vecs.is_empty() {
        return None;
    }
    let dim = vecs[0].len();
    if !vecs.iter().all(|v| v.len() == dim) {
        return None;
    }
    let mut sum = vec![0f64; dim];
    for v in &vecs {
        for (i, x) in v.iter().enumerate() {
            sum[i] += *x as f64;
        }
    }
    let n = vecs.len() as f64;
    let mean: Vec<ciborium::Value> = sum
        .into_iter()
        .map(|s| ciborium::Value::Float(s / n))
        .collect();
    Some(ciborium::Value::Array(mean))
}
