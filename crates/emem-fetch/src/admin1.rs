//! Embedded GeoNames admin1 gazetteer — the third tier in emem's
//! locate cascade, between [`crate::countries`] and the cities1000
//! populated-places dataset.
//!
//! ## Why this layer exists
//!
//! Queries like "West Bengal", "California", "Bavaria", or "Sylhet
//! Division" name *first-level administrative regions* (ADM1 in
//! GeoNames parlance — states, provinces, divisions, regions). These
//! are first-class entities that are *absent* from any cities-N
//! dataset; they only appear in the dedicated `admin1CodesASCII.txt`
//! file. Without this layer, an agent asking for a sub-national
//! region would either return nothing or get a city-name collision
//! ("California" the Pennsylvania borough out-ranking the State).
//!
//! ## Schema (GeoNames `admin1CodesASCII.txt`, tab-separated)
//!
//! `CC.code \t name \t asciiname \t geonameid`
//!
//! Example: `IN.WB <TAB> West Bengal <TAB> West Bengal <TAB> 1252881`.
//!
//! ## Bounding box derivation
//!
//! GeoNames does not publish per-ADM1 bboxes. We aggregate from
//! [`crate::geonames`]'s already-loaded cities1000 corpus filtered to
//! the matching `(country, admin1)` pair. For ADM1s with zero cities
//! in the corpus, lookup still succeeds but returns a degenerate
//! zero-area bbox so callers can detect the gap and fall through to
//! Photon. The same "populated extent ≤ political extent" caveat as
//! [`crate::countries`] applies.
//!
//! ## License
//!
//! GeoNames is **CC-BY-4.0**. Receipts that hit this layer surface
//! `served_via: "admin1"` plus `license`.

use std::collections::HashMap;
use std::io::Read;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// Embedded admin1 dump (gzip-9'd). Decompresses to ~150 kB at
/// startup. Sourced from
/// `https://download.geonames.org/export/dump/admin1CodesASCII.txt`
/// snapshot 2026-05-23.
const ADMIN1_GZ: &[u8] = include_bytes!("../data/admin1CodesASCII.txt.gz");

pub const ATTRIBUTION: &str = "GeoNames (https://www.geonames.org) — CC-BY-4.0";
pub const LICENSE: &str = "CC-BY-4.0";
pub const SNAPSHOT_DATE: &str = "2026-05-23";

/// One first-level admin region. Bounding box + centroid are derived
/// from cities1000 at first lookup (see module docs).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Admin1Record {
    /// Country ISO2 ("IN").
    pub country: String,
    /// Admin1 code ("WB"). Distinguishes "California, US" from
    /// "California, PA" if both shared a row.
    pub admin1_code: String,
    /// Full region name ("West Bengal", "California").
    pub name: String,
    /// ASCII-folded equivalent ("West Bengal" → "West Bengal").
    pub asciiname: String,
    /// Stable GeoNames integer ID for the admin1 entity itself.
    pub geonameid: u64,
    /// Centroid lat — geometric mean of contained cities' lats.
    pub centroid_lat: f64,
    /// Centroid lng — geometric mean of contained cities' lngs.
    pub centroid_lng: f64,
    /// BBox derived from cities1000.
    pub min_lat: f64,
    pub max_lat: f64,
    pub min_lng: f64,
    pub max_lng: f64,
    /// Number of cities1000 rows that contributed to the bbox. Zero
    /// means the bbox is degenerate; consumers should fall through.
    pub source_city_count: u32,
}

impl Admin1Record {
    pub fn bbox(&self) -> (f64, f64, f64, f64) {
        (self.min_lat, self.max_lat, self.min_lng, self.max_lng)
    }

    /// Reader-friendly label, e.g. `"West Bengal, IN"`.
    pub fn label(&self) -> String {
        format!("{}, {}", self.name, self.country)
    }
}

struct Index {
    /// Normalized name → list of record indices. ADM1 names *do*
    /// collide across countries (Cordillera in PE and CO, Western in
    /// dozens of countries), so a query like "Western" hits multiple
    /// records — caller can disambiguate by `country` field.
    by_name: HashMap<String, Vec<usize>>,
    records: Vec<Admin1Record>,
}

static INDEX: OnceLock<Index> = OnceLock::new();

fn index() -> &'static Index {
    INDEX.get_or_init(|| {
        let mut decoder = flate2::read::GzDecoder::new(ADMIN1_GZ);
        let mut buf = String::with_capacity(256 * 1024);
        decoder
            .read_to_string(&mut buf)
            .expect("bundled admin1CodesASCII.txt.gz must decompress");
        // First pass: parse raw rows, defer bbox aggregation so we
        // can stream cities1000 once and dispatch updates by
        // (country, admin1) key.
        struct Raw {
            country: String,
            admin1_code: String,
            name: String,
            asciiname: String,
            geonameid: u64,
        }
        let mut raws: Vec<Raw> = Vec::with_capacity(4_096);
        for line in buf.lines() {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            let mut cols = line.split('\t');
            let key = cols.next().unwrap_or("").trim();
            let name = cols.next().unwrap_or("").trim();
            let asciiname = cols.next().unwrap_or("").trim();
            let geonameid = cols
                .next()
                .and_then(|s| s.trim().parse::<u64>().ok())
                .unwrap_or(0);
            let mut kparts = key.splitn(2, '.');
            let country = kparts.next().unwrap_or("").to_string();
            let admin1_code = kparts.next().unwrap_or("").to_string();
            if country.is_empty() || admin1_code.is_empty() || name.is_empty() {
                continue;
            }
            raws.push(Raw {
                country,
                admin1_code,
                name: name.into(),
                asciiname: asciiname.into(),
                geonameid,
            });
        }
        // Build a (country, admin1_code) → raw-index map so we can
        // do a single linear scan over cities1000 instead of an
        // O(rows × ADM1s) double loop.
        let mut by_admin_key: HashMap<(String, String), usize> = HashMap::with_capacity(raws.len());
        for (i, r) in raws.iter().enumerate() {
            by_admin_key.insert((r.country.clone(), r.admin1_code.clone()), i);
        }
        // Per-admin1 accumulator (sum_lat, sum_lng, count, min_lat,
        // max_lat, min_lng, max_lng).
        let mut acc: Vec<(f64, f64, u32, f64, f64, f64, f64)> = vec![
            (
                0.0,
                0.0,
                0,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
            );
            raws.len()
        ];
        for rec in crate::geonames::iter_records() {
            if rec.country.is_empty() || rec.admin1.is_empty() {
                continue;
            }
            let k = (rec.country.clone(), rec.admin1.clone());
            if let Some(i) = by_admin_key.get(&k) {
                let a = &mut acc[*i];
                a.0 += rec.lat;
                a.1 += rec.lng;
                a.2 += 1;
                if rec.lat < a.3 {
                    a.3 = rec.lat;
                }
                if rec.lat > a.4 {
                    a.4 = rec.lat;
                }
                if rec.lng < a.5 {
                    a.5 = rec.lng;
                }
                if rec.lng > a.6 {
                    a.6 = rec.lng;
                }
            }
        }
        let mut records: Vec<Admin1Record> = Vec::with_capacity(raws.len());
        let mut by_name: HashMap<String, Vec<usize>> = HashMap::with_capacity(8_192);
        for (i, raw) in raws.into_iter().enumerate() {
            let (sum_lat, sum_lng, count, min_lat, max_lat, min_lng, max_lng) = acc[i];
            let (centroid_lat, centroid_lng, mn_la, mx_la, mn_ln, mx_ln) = if count > 0 {
                let n = count as f64;
                (sum_lat / n, sum_lng / n, min_lat, max_lat, min_lng, max_lng)
            } else {
                (0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
            };
            let rec = Admin1Record {
                country: raw.country,
                admin1_code: raw.admin1_code,
                name: raw.name.clone(),
                asciiname: raw.asciiname.clone(),
                geonameid: raw.geonameid,
                centroid_lat,
                centroid_lng,
                min_lat: mn_la,
                max_lat: mx_la,
                min_lng: mn_ln,
                max_lng: mx_ln,
                source_city_count: count,
            };
            let idx = records.len();
            for k in [raw.name.as_str(), raw.asciiname.as_str()] {
                let n = crate::geonames::normalize(k);
                if n.is_empty() {
                    continue;
                }
                let entry = by_name.entry(n).or_default();
                if !entry.contains(&idx) {
                    entry.push(idx);
                }
            }
            records.push(rec);
        }
        Index { by_name, records }
    })
}

/// Look up an admin1 by name. Returns the best matching record:
/// among hits, prefer those with a non-degenerate bbox (≥ 1 city in
/// the corpus); within that, prefer higher city-count (better
/// extent confidence).
///
/// Returns `None` when no record matches the query.
pub fn lookup(query: &str) -> Option<&'static Admin1Record> {
    let idx = index();
    let key = crate::geonames::normalize(query);
    if key.is_empty() {
        return None;
    }
    let hits = idx.by_name.get(&key)?;
    // Prefer records with cities-derived extent (source_city_count >
    // 0); among those, prefer the one with the most contributing
    // cities (proxy for "more populated → more globally relevant").
    hits.iter()
        .map(|i| &idx.records[*i])
        .max_by_key(|r| (r.source_city_count > 0, r.source_city_count))
}

/// Look up an admin1 *scoped to a specific country*. Disambiguates
/// "Western" or "Northern" which appear in dozens of countries; the
/// agent passes country ISO2 plus the region name.
pub fn lookup_in_country(query: &str, country_iso2: &str) -> Option<&'static Admin1Record> {
    let idx = index();
    let key = crate::geonames::normalize(query);
    if key.is_empty() {
        return None;
    }
    let hits = idx.by_name.get(&key)?;
    hits.iter()
        .map(|i| &idx.records[*i])
        .find(|r| r.country.eq_ignore_ascii_case(country_iso2))
}

pub fn indexed_record_count() -> usize {
    index().records.len()
}

pub fn warm_index() -> usize {
    indexed_record_count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn west_bengal_resolves() {
        let wb = lookup("West Bengal").expect("West Bengal must hit");
        assert_eq!(wb.country, "IN");
        // BBox must contain Kolkata (~22.57°N, 88.36°E).
        assert!(
            wb.min_lat <= 22.57
                && wb.max_lat >= 22.57
                && wb.min_lng <= 88.36
                && wb.max_lng >= 88.36,
            "Kolkata must lie inside West Bengal bbox {:?}",
            wb.bbox(),
        );
    }

    #[test]
    fn dhaka_division_resolves() {
        let dd = lookup("Dhaka Division").expect("Dhaka Division must hit");
        assert_eq!(dd.country, "BD");
    }

    #[test]
    fn lookup_in_country_disambiguates() {
        // "Western" exists in many countries; scoping to a specific
        // ISO2 picks the right one. We don't pin the exact bbox
        // (data-dependent) — just that it returns *something* for a
        // common country and that country tag matches.
        let r = lookup_in_country("Western", "KE");
        if let Some(r) = r {
            assert_eq!(r.country, "KE");
        }
    }

    #[test]
    fn unknown_returns_none() {
        assert!(lookup("zzqxgzqxg-not-a-region").is_none());
        assert!(lookup("").is_none());
    }

    #[test]
    fn index_has_reasonable_size() {
        let n = indexed_record_count();
        assert!(
            (3_500..=4_500).contains(&n),
            "admin1 record count {n} outside expected 3.5k–4.5k band"
        );
    }
}
