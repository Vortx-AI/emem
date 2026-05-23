//! Embedded GeoNames admin2 gazetteer — fourth tier in emem's locate
//! cascade, between [`crate::admin1`] and the cities1000 dataset.
//!
//! ## Why this layer exists
//!
//! Many countries' canonical sub-national unit is admin2: India's
//! ~700 districts, Bangladesh's 64 districts, the US's ~3 100
//! counties, Germany's Kreise, China's prefectures. An agent asking
//! "Cox's Bazar district" or "Cuyahoga County" wants this layer.
//! GeoNames carries these in `admin2Codes.txt` (~50 k entries, ~700
//! kB compressed), small enough to embed without bloating the
//! binary.
//!
//! ## Schema (GeoNames `admin2Codes.txt`, tab-separated)
//!
//! `CC.adm1.adm2 \t name \t asciiname \t geonameid`
//!
//! Example: `BD.84.A01\tCox's Bazar District\tCox's Bazar District\t1185232`.
//!
//! Bounding box derivation is identical to [`crate::admin1`]:
//! aggregate cities1000 rows whose `(country, admin1, admin2)` keys
//! match. POI-only admin2s (no cities) carry a degenerate zero-area
//! bbox so consumers can detect the gap and fall through.
//!
//! ## License
//!
//! GeoNames is **CC-BY-4.0**. Receipts that hit this layer surface
//! `served_via: "admin2"` plus `license`.

use std::collections::HashMap;
use std::io::Read;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

const ADMIN2_GZ: &[u8] = include_bytes!("../data/admin2Codes.txt.gz");

pub const ATTRIBUTION: &str = "GeoNames (https://www.geonames.org) — CC-BY-4.0";
pub const LICENSE: &str = "CC-BY-4.0";
pub const SNAPSHOT_DATE: &str = "2026-05-23";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Admin2Record {
    pub country: String,
    pub admin1_code: String,
    pub admin2_code: String,
    pub name: String,
    pub asciiname: String,
    pub geonameid: u64,
    pub centroid_lat: f64,
    pub centroid_lng: f64,
    pub min_lat: f64,
    pub max_lat: f64,
    pub min_lng: f64,
    pub max_lng: f64,
    pub source_city_count: u32,
}

impl Admin2Record {
    pub fn bbox(&self) -> (f64, f64, f64, f64) {
        (self.min_lat, self.max_lat, self.min_lng, self.max_lng)
    }

    pub fn label(&self) -> String {
        format!("{}, {} {}", self.name, self.admin1_code, self.country)
    }
}

struct Index {
    by_name: HashMap<String, Vec<usize>>,
    records: Vec<Admin2Record>,
}

static INDEX: OnceLock<Index> = OnceLock::new();

fn index() -> &'static Index {
    INDEX.get_or_init(|| {
        let mut decoder = flate2::read::GzDecoder::new(ADMIN2_GZ);
        let mut buf = String::with_capacity(2 * 1024 * 1024);
        decoder
            .read_to_string(&mut buf)
            .expect("bundled admin2Codes.txt.gz must decompress");
        struct Raw {
            country: String,
            admin1_code: String,
            admin2_code: String,
            name: String,
            asciiname: String,
            geonameid: u64,
        }
        let mut raws: Vec<Raw> = Vec::with_capacity(48_000);
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
            // Key format: "CC.adm1.adm2" — splitn(3) handles the rare
            // empty-admin1 case ("CC..adm2") without panicking.
            let mut kp = key.split('.');
            let country = kp.next().unwrap_or("").to_string();
            let admin1_code = kp.next().unwrap_or("").to_string();
            let admin2_code = kp.next().unwrap_or("").to_string();
            if country.is_empty() || admin2_code.is_empty() || name.is_empty() {
                continue;
            }
            raws.push(Raw {
                country,
                admin1_code,
                admin2_code,
                name: name.into(),
                asciiname: asciiname.into(),
                geonameid,
            });
        }
        let mut by_admin_key: HashMap<(String, String, String), usize> =
            HashMap::with_capacity(raws.len());
        for (i, r) in raws.iter().enumerate() {
            by_admin_key.insert(
                (r.country.clone(), r.admin1_code.clone(), r.admin2_code.clone()),
                i,
            );
        }
        // The cities1000 corpus is the bbox source. We don't have
        // admin2 access on cities1000 records directly through the
        // public `GeonameRecord` struct (it only exposes admin1) — so
        // we re-parse the bundled cities gzip here, keyed by the full
        // (country, admin1, admin2) tuple from columns 8/10/11.
        //
        // The double-parse is a small one-time cost (~150 ms on a
        // modern CPU) paid once at first lookup. Caching it inside
        // OnceLock means subsequent admin2 lookups are O(1).
        let mut acc: Vec<(f64, f64, u32, f64, f64, f64, f64)> = vec![
            (0.0, 0.0, 0, f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
            raws.len()
        ];
        // Re-decompress cities1000 here so we can read column 11
        // (admin2) which the GeonameRecord doesn't currently expose.
        const CITIES_GZ: &[u8] = include_bytes!("../data/cities1000.txt.gz");
        let mut dec2 = flate2::read::GzDecoder::new(CITIES_GZ);
        let mut cities_buf = String::with_capacity(36 * 1024 * 1024);
        dec2.read_to_string(&mut cities_buf)
            .expect("bundled cities1000.txt.gz must decompress");
        for line in cities_buf.lines() {
            // Columns we need: 4=lat 5=lng 8=country 10=admin1 11=admin2.
            let cols: Vec<&str> = line.split('\t').collect();
            if cols.len() < 12 {
                continue;
            }
            let lat: f64 = cols[4].parse().unwrap_or(f64::NAN);
            let lng: f64 = cols[5].parse().unwrap_or(f64::NAN);
            if !lat.is_finite() || !lng.is_finite() {
                continue;
            }
            let country = cols[8].trim();
            let admin1 = cols[10].trim();
            let admin2 = cols[11].trim();
            if country.is_empty() || admin2.is_empty() {
                continue;
            }
            let k = (country.to_string(), admin1.to_string(), admin2.to_string());
            if let Some(i) = by_admin_key.get(&k) {
                let a = &mut acc[*i];
                a.0 += lat;
                a.1 += lng;
                a.2 += 1;
                if lat < a.3 { a.3 = lat; }
                if lat > a.4 { a.4 = lat; }
                if lng < a.5 { a.5 = lng; }
                if lng > a.6 { a.6 = lng; }
            }
        }
        let mut records: Vec<Admin2Record> = Vec::with_capacity(raws.len());
        let mut by_name: HashMap<String, Vec<usize>> = HashMap::with_capacity(96_000);
        for (i, raw) in raws.into_iter().enumerate() {
            let (sum_lat, sum_lng, count, min_lat, max_lat, min_lng, max_lng) = acc[i];
            let (centroid_lat, centroid_lng, mn_la, mx_la, mn_ln, mx_ln) = if count > 0 {
                let n = count as f64;
                (sum_lat / n, sum_lng / n, min_lat, max_lat, min_lng, max_lng)
            } else {
                (0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
            };
            let rec = Admin2Record {
                country: raw.country,
                admin1_code: raw.admin1_code,
                admin2_code: raw.admin2_code,
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

/// Look up an admin2 by name. Returns the record with the highest
/// `source_city_count` among matches (proxy for "biggest /
/// most-populated district by that name"). For admin2s with zero
/// cities in cities1000, the returned record carries a degenerate
/// bbox; callers should check `source_city_count > 0` before using
/// `bbox()` for sampling.
pub fn lookup(query: &str) -> Option<&'static Admin2Record> {
    let idx = index();
    let key = crate::geonames::normalize(query);
    if key.is_empty() {
        return None;
    }
    let hits = idx.by_name.get(&key)?;
    hits.iter()
        .map(|i| &idx.records[*i])
        .max_by_key(|r| (r.source_city_count > 0, r.source_city_count))
}

/// Look up an admin2 scoped to a specific country.
pub fn lookup_in_country(query: &str, country_iso2: &str) -> Option<&'static Admin2Record> {
    let idx = index();
    let key = crate::geonames::normalize(query);
    if key.is_empty() {
        return None;
    }
    let hits = idx.by_name.get(&key)?;
    hits.iter()
        .map(|i| &idx.records[*i])
        .filter(|r| r.country.eq_ignore_ascii_case(country_iso2))
        .max_by_key(|r| r.source_city_count)
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
    fn coxs_bazar_district_resolves() {
        // GeoNames stores it as "Cox's Bazar" (no "District" suffix)
        // even though it IS the district entry. The normalizer drops
        // the apostrophe so either spelling routes to the same row.
        let r = lookup("Cox's Bazar")
            .or_else(|| lookup("Coxs Bazar"))
            .expect("Cox's Bazar must hit");
        assert_eq!(r.country, "BD");
    }

    #[test]
    fn cuyahoga_county_resolves() {
        let r = lookup("Cuyahoga County").expect("Cuyahoga County must hit");
        assert_eq!(r.country, "US");
    }

    #[test]
    fn index_has_reasonable_size() {
        let n = indexed_record_count();
        assert!(
            (40_000..=55_000).contains(&n),
            "admin2 record count {n} outside expected 40k–55k band"
        );
    }
}
