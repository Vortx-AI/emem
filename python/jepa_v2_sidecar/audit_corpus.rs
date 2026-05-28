// One-off audit: read sled directly, decode cell|band|tslot keys from
// `emem.canonical_index` tree, count per-band (cell, tslot) pairs.
// Then for candidate bands, also read facts from `emem.facts` tree to
// extract scalar values and dump CSVs for training.
//
// Build (out-of-tree, won't pollute the main workspace):
//
//   mkdir -p /tmp/emem-audit/src && cd /tmp/emem-audit
//   cp /path/to/audit_corpus.rs src/audit.rs
//   cp /path/to/audit_corpus.Cargo.toml Cargo.toml
//   cargo run --release -- /path/to/cache.sled /tmp/emem_audit_dump
//
// The audit holds an exclusive lock on the sled database; copy
// $EMEM_DATA/cache.sled/ first if a server is running:
//
//   cp -r $EMEM_DATA/cache.sled /tmp/emem_sled_snapshot/
//   cargo run --release -- /tmp/emem_sled_snapshot/cache.sled /tmp/emem_audit_dump
//
// Output is `/tmp/emem_audit_dump/<band>.csv` with (cell, tslot,
// value) columns. `train_dynamics_v2.py` consumes the
// `indices_ndvi.csv` slice for the held-out real-NDVI evaluation.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

const TREE_INDEX: &str = "emem.canonical_index";
const TREE_FACTS: &str = "emem.facts";
const SEP: u8 = 0x00;

// Decoder uses raw ciborium::Value — simpler than chasing the exact Fact enum.

fn decode_key(b: &[u8]) -> Option<(String, String, u64)> {
    let mut parts = b.splitn(3, |c| *c == SEP);
    let cell = std::str::from_utf8(parts.next()?).ok()?.to_string();
    let band = std::str::from_utf8(parts.next()?).ok()?.to_string();
    let rest = parts.next()?;
    if rest.len() != 8 {
        return None;
    }
    let mut t = [0u8; 8];
    t.copy_from_slice(rest);
    Some((cell, band, u64::from_be_bytes(t)))
}

fn cbor_to_f64(v: &ciborium::Value) -> Option<f64> {
    match v {
        ciborium::Value::Float(f) => Some(*f),
        ciborium::Value::Integer(i) => {
            let x: i128 = (*i).into();
            Some(x as f64)
        }
        _ => None,
    }
}

/// Walk a CBOR Map looking for a key named `field` and extract its
/// scalar value (handles primitive bands that wrap value in a struct).
fn extract_scalar(v: &ciborium::Value) -> Option<f64> {
    if let Some(f) = cbor_to_f64(v) {
        return Some(f);
    }
    if let ciborium::Value::Map(m) = v {
        for (k, val) in m {
            if let ciborium::Value::Text(s) = k {
                if s == "value" || s == "v" || s == "scalar" {
                    if let Some(f) = cbor_to_f64(val) {
                        return Some(f);
                    }
                }
            }
        }
        // Otherwise take the first numeric field present.
        for (_, val) in m {
            if let Some(f) = cbor_to_f64(val) {
                return Some(f);
            }
        }
    }
    None
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/emem_sled_snapshot/cache.sled".into());
    let csv_out = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "/tmp/emem_audit_dump".into());
    std::fs::create_dir_all(&csv_out)?;

    eprintln!("opening sled @ {path}");
    let db = sled::Config::new()
        .path(&path)
        .use_compression(false)
        .open()?;
    let idx = db.open_tree(TREE_INDEX)?;
    let facts = db.open_tree(TREE_FACTS)?;
    eprintln!("idx len = {}", idx.len());
    eprintln!("facts len = {}", facts.len());

    // Pass 1: per-band counts.
    let mut per_band_cells: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut per_band_tslots: BTreeMap<String, BTreeSet<u64>> = BTreeMap::new();
    let mut per_band_total: BTreeMap<String, usize> = BTreeMap::new();
    let mut per_band_cell_tslot_pairs: BTreeMap<String, BTreeSet<(String, u64)>> = BTreeMap::new();
    let mut per_cell_band_tslots: BTreeMap<(String, String), BTreeSet<u64>> = BTreeMap::new();
    let mut all_band_cid: BTreeMap<(String, String, u64), String> = BTreeMap::new();

    for kv in idx.iter() {
        let (k, v) = kv?;
        let cid = String::from_utf8_lossy(&v).to_string();
        if let Some((cell, band, tslot)) = decode_key(&k) {
            per_band_cells.entry(band.clone()).or_default().insert(cell.clone());
            per_band_tslots.entry(band.clone()).or_default().insert(tslot);
            *per_band_total.entry(band.clone()).or_default() += 1;
            per_band_cell_tslot_pairs.entry(band.clone()).or_default().insert((cell.clone(), tslot));
            per_cell_band_tslots
                .entry((cell.clone(), band.clone()))
                .or_default()
                .insert(tslot);
            all_band_cid.insert((cell, band, tslot), cid);
        }
    }

    println!("\n== BAND CORPUS AUDIT ==");
    println!("{:50} {:>10} {:>10} {:>10}", "band", "facts", "cells", "tslots");
    for (band, total) in per_band_total.iter() {
        let cells = per_band_cells.get(band).map(|s| s.len()).unwrap_or(0);
        let tslots = per_band_tslots.get(band).map(|s| s.len()).unwrap_or(0);
        println!("{band:50} {total:>10} {cells:>10} {tslots:>10}");
    }

    // For candidate bands, compute per-cell tslot-count distribution.
    let candidates = [
        "indices.ndvi",
        "modis.lst_day_8day",
        "modis.lst_night_8day",
        "cams.pm25",
        "cams.no2",
        "cams.o3",
        "weather.temperature_2m",
        "weather.precipitation_mm",
        "surface_water.recurrence",
        "hansen.loss_year",
    ];
    println!("\n== TEMPORAL DEPTH PER CELL ==");
    for band in candidates {
        let mut hist: BTreeMap<usize, usize> = BTreeMap::new();
        let mut max_depth = 0;
        let mut total_cells = 0;
        for ((_cell, b), tset) in per_cell_band_tslots.iter() {
            if b == band {
                total_cells += 1;
                let n = tset.len();
                max_depth = max_depth.max(n);
                *hist.entry(n).or_default() += 1;
            }
        }
        if total_cells == 0 {
            println!("{band:50}  (no facts)");
        } else {
            print!("{band:50}  cells={total_cells:>6} max_depth={max_depth:>3}  histogram:");
            for (d, c) in hist.iter() {
                print!(" {d}t→{c}c");
            }
            println!();
        }
    }

    // Dump per-band scalar CSV for the candidates.
    println!("\n== DUMPING TRAINING CSVS ==");
    for band in &candidates {
        let pairs = match per_band_cell_tslot_pairs.get(*band) {
            Some(s) => s,
            None => continue,
        };
        let csv_path = PathBuf::from(&csv_out).join(format!("{}.csv", band.replace('.', "_")));
        let mut wtr = csv::Writer::from_path(&csv_path)?;
        wtr.write_record(["cell", "tslot", "value"])?;
        let mut n_rows = 0_usize;
        let mut n_skipped_nonscalar = 0_usize;
        for (cell, tslot) in pairs.iter() {
            let key = (cell.clone(), (*band).to_string(), *tslot);
            let cid = match all_band_cid.get(&key) {
                Some(c) => c,
                None => continue,
            };
            let v = match facts.get(cid.as_bytes())? {
                Some(b) => b,
                None => continue,
            };
            // Decode raw CBOR Value first so we can debug shape variations
            let raw: Result<ciborium::Value, _> = ciborium::de::from_reader(&v[..]);
            let scalar = match raw {
                Ok(val) => {
                    // Find the `value` field if it's a tagged Map
                    let mut found: Option<f64> = None;
                    if let ciborium::Value::Map(m) = &val {
                        for (k, vv) in m {
                            if let ciborium::Value::Text(ks) = k {
                                if ks == "value" {
                                    found = extract_scalar(vv);
                                    break;
                                }
                            }
                        }
                    }
                    if found.is_none() && n_rows == 0 && n_skipped_nonscalar < 3 {
                        eprintln!("DEBUG {band}: cbor value shape = {val:#?}");
                    }
                    found
                }
                Err(e) => {
                    if n_skipped_nonscalar < 3 {
                        eprintln!("DEBUG cbor decode err: {e}");
                    }
                    None
                }
            };
            match scalar {
                Some(s) if s.is_finite() => {
                    wtr.write_record([cell.as_str(), &tslot.to_string(), &s.to_string()])?;
                    n_rows += 1;
                }
                _ => n_skipped_nonscalar += 1,
            }
        }
        wtr.flush()?;
        println!("{}: wrote {n_rows} rows, skipped {n_skipped_nonscalar} non-scalar → {}", band, csv_path.display());
    }
    Ok(())
}
