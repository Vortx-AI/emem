//! `query_region(geometry, bands?, agg?)` — spec §11 MCP `emem.query_region`.
//!
//! Geometry forms in this build:
//!
//! - `cell64` (e.g. `"damO.zb000.xUti.zde78"`) — single cell.
//! - `cells:c1,c2,...` — explicit list of cell64 strings.
//!
//! Bbox / GeoJSON polyfill requires the H3-equivalent indexer which is
//! a separate concern; operators wire it in by replacing `query_region`
//! at the API layer when a polyfill backend is present.

use std::collections::BTreeMap;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use emem_core::ErrorCode;
use emem_fact::{Fact, FactCid, Receipt};
use emem_storage::{Server, StorageError};

use crate::cbor_ops::{as_f64, as_vec_f32};

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
pub async fn query_region(req: &QueryRegionReq, srv: &Server) -> Result<QueryRegionResp, StorageError> {
    let started = Instant::now();
    let storage = srv.storage.as_ref();

    let cells: Vec<String> = if let Some(rest) = req.geometry.strip_prefix("cells:") {
        rest.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    } else if req.geometry.starts_with("bbox:") || req.geometry.starts_with('{') {
        return Err(StorageError::Protocol {
            code: ErrorCode::InvalidCell,
            message: "query_region accepts cell64 or 'cells:c1,c2,...' geometries; bbox/GeoJSON polyfill requires an H3-equivalent indexer".into(),
        });
    } else {
        vec![req.geometry.clone()]
    };

    let mut all_facts: Vec<Fact> = Vec::new();
    let mut all_cids: Vec<FactCid> = Vec::new();
    for cell in &cells {
        let entries = storage.scan_cell(cell, None).await?;
        let cids: Vec<FactCid> = entries.into_iter().map(|(_, c)| c).collect();
        all_cids.extend(cids.iter().cloned());
        let fetched = storage.get_facts_many(&cids).await?;
        for fact in fetched.into_iter().flatten() {
            if let Some(filter) = &req.bands {
                let band = match &fact {
                    Fact::Primary(p) => &p.band,
                    Fact::Absence(n) => &n.band,
                    Fact::Derivative(d) => &d.band,
                };
                if !filter.iter().any(|b| b == band) { continue; }
            }
            all_facts.push(fact);
        }
    }

    let aggregates = match req.agg.as_deref() {
        None => BTreeMap::new(),
        Some(op) => aggregate(&all_facts, op)?,
    };

    let receipt = srv.sign_receipt(
        "emem.query_region",
        cells,
        all_cids,
        true,
        started,
        None,
    );
    Ok(QueryRegionResp { facts: all_facts, aggregates, receipt })
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
            other => return Err(StorageError::Protocol {
                code: ErrorCode::Internal,
                message: format!("unknown aggregation: {other}"),
            }),
        };
        if let Some(v) = agg { out.insert(band, v); }
    }
    Ok(out)
}

fn agg_mean(values: &[&ciborium::Value]) -> Option<ciborium::Value> {
    let nums: Vec<f64> = values.iter().filter_map(|v| as_f64(v)).collect();
    if nums.is_empty() { return None; }
    Some(ciborium::Value::Float(nums.iter().sum::<f64>() / nums.len() as f64))
}

fn agg_median(values: &[&ciborium::Value]) -> Option<ciborium::Value> {
    // Strip NaN before aggregating: a single NaN would otherwise contaminate
    // the median via partial_cmp's undefined ordering.
    let mut nums: Vec<f64> = values.iter().filter_map(|v| as_f64(v))
        .filter(|x| !x.is_nan()).collect();
    if nums.is_empty() { return None; }
    nums.sort_by(|a, b| a.total_cmp(b));
    let mid = nums.len() / 2;
    let m = if nums.len() % 2 == 0 { (nums[mid - 1] + nums[mid]) / 2.0 } else { nums[mid] };
    Some(ciborium::Value::Float(m))
}

fn agg_percentile(values: &[&ciborium::Value], p: f64) -> Option<ciborium::Value> {
    let mut nums: Vec<f64> = values.iter().filter_map(|v| as_f64(v))
        .filter(|x| !x.is_nan()).collect();
    if nums.is_empty() { return None; }
    nums.sort_by(|a, b| a.total_cmp(b));
    let idx = ((nums.len() - 1) as f64 * p).round() as usize;
    Some(ciborium::Value::Float(nums[idx]))
}

fn agg_vector_centroid(values: &[&ciborium::Value]) -> Option<ciborium::Value> {
    let vecs: Vec<Vec<f32>> = values.iter().filter_map(|v| as_vec_f32(v)).collect();
    if vecs.is_empty() { return None; }
    let dim = vecs[0].len();
    if !vecs.iter().all(|v| v.len() == dim) { return None; }
    let mut sum = vec![0f64; dim];
    for v in &vecs {
        for (i, x) in v.iter().enumerate() { sum[i] += *x as f64; }
    }
    let n = vecs.len() as f64;
    let mean: Vec<ciborium::Value> = sum.into_iter()
        .map(|s| ciborium::Value::Float(s / n))
        .collect();
    Some(ciborium::Value::Array(mean))
}
