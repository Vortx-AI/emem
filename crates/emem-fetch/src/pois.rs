//! Embedded GeoNames POI gazetteer — well-known landmarks (mountains,
//! lakes, parks, airports, monuments) that don't fit in any cities-N
//! cut because they aren't populated places.
//!
//! ## Role in the cascade
//!
//! Runs after [`crate::geonames`] cities and before the network
//! fallback. Catches queries like "Mount Everest", "Lake Victoria",
//! "Yellowstone National Park", "JFK Airport" without a Photon /
//! Nominatim round-trip.
//!
//! ## Selection criteria
//!
//! We bundle GeoNames `allCountries.txt` rows whose feature code is
//! in the well-known-place set AND that carry at least 3 alternate
//! names (a global-notability heuristic — well-known features are
//! routinely transliterated across languages, obscure local ones
//! aren't). Airports always pass regardless of alternate-name count.
//!
//! Kept feature codes:
//!   T.MT/PK/MTS — mountains, peaks, mountain ranges
//!   H.LK/LKS    — lakes
//!   L.PRK/RGN/VAL/DSRT/ISL/ISLS — parks, regions, valleys, deserts, islands
//!   S.AIRP                       — airports
//!   S.MNMT/MUS/UNIV/HSTS         — monuments, museums, universities, historical sites
//!
//! ~196 k entries, ~11 MB compressed. Refresh recipe in
//! `scripts/refresh_geonames.sh`.
//!
//! ## Schema
//!
//! Same 19-column GeoNames TSV as cities1000. We retain id, native
//! name, asciiname, lat/lng, feature class, feature code, country,
//! population, elevation.
//!
//! ## License
//!
//! GeoNames is **CC-BY-4.0**.

use std::collections::HashMap;
use std::io::Read;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

const POIS_GZ: &[u8] = include_bytes!("../data/pois.txt.gz");

pub const ATTRIBUTION: &str = "GeoNames (https://www.geonames.org) — CC-BY-4.0";
pub const LICENSE: &str = "CC-BY-4.0";
pub const SNAPSHOT_DATE: &str = "2026-05-23";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PoiRecord {
    pub geonameid: u64,
    pub name: String,
    pub asciiname: String,
    pub country: String,
    pub fclass: String,
    pub fcode: String,
    pub lat: f64,
    pub lng: f64,
    pub population: u64,
    pub elevation_m: Option<i64>,
}

impl PoiRecord {
    /// One-line reader-friendly label. Includes feature-code label
    /// so the agent can immediately see whether the hit is a peak, a
    /// lake, an airport, etc. — same data shape the cities label
    /// gives.
    pub fn label(&self) -> String {
        let kind = match self.fcode.as_str() {
            "MT" | "MTS" => "mountain",
            "PK" => "peak",
            "LK" | "LKS" => "lake",
            "PRK" => "park",
            "RGN" => "region",
            "VAL" => "valley",
            "DSRT" => "desert",
            "ISL" | "ISLS" => "island",
            "AIRP" => "airport",
            "MNMT" => "monument",
            "MUS" => "museum",
            "UNIV" => "university",
            "HSTS" => "historical site",
            _ => self.fcode.as_str(),
        };
        format!("{} ({}) — {}", self.name, kind, self.country)
    }
}

/// Which key registered the record. Identical semantics to
/// [`crate::geonames`]: primary > ascii > alternate, used to
/// prevent alternate-name poaching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum KeyKind {
    Primary = 0,
    Ascii = 1,
    Alternate = 2,
}

struct Index {
    by_name: HashMap<String, Vec<(usize, KeyKind)>>,
    records: Vec<PoiRecord>,
}

static INDEX: OnceLock<Index> = OnceLock::new();

fn index() -> &'static Index {
    INDEX.get_or_init(|| {
        let mut decoder = flate2::read::GzDecoder::new(POIS_GZ);
        let mut buf = String::with_capacity(40 * 1024 * 1024);
        decoder
            .read_to_string(&mut buf)
            .expect("bundled pois.txt.gz must decompress");
        let mut records: Vec<PoiRecord> = Vec::with_capacity(200_000);
        let mut by_name: HashMap<String, Vec<(usize, KeyKind)>> =
            HashMap::with_capacity(800_000);
        for line in buf.lines() {
            // 19-column GeoNames TSV (same as cities). Columns we use:
            // 0=geonameid 1=name 2=asciiname 3=alternates 4=lat 5=lng
            // 6=fclass 7=fcode 8=country 14=population 15=elevation.
            let cols: Vec<&str> = line.split('\t').collect();
            if cols.len() < 16 {
                continue;
            }
            let geonameid: u64 = cols[0].parse().unwrap_or(0);
            let name = cols[1].trim();
            let asciiname = cols[2].trim();
            let alternates = cols[3];
            let lat: f64 = cols[4].parse().unwrap_or(f64::NAN);
            let lng: f64 = cols[5].parse().unwrap_or(f64::NAN);
            let fclass = cols[6].trim();
            let fcode = cols[7].trim();
            let country = cols[8].trim();
            let population: u64 = cols[14].parse().unwrap_or(0);
            let elevation: Option<i64> = cols[15].parse().ok();
            if name.is_empty() || !lat.is_finite() || !lng.is_finite() {
                continue;
            }
            let rec_idx = records.len();
            let record = PoiRecord {
                geonameid,
                name: name.into(),
                asciiname: asciiname.into(),
                country: country.into(),
                fclass: fclass.into(),
                fcode: fcode.into(),
                lat,
                lng,
                population,
                elevation_m: elevation,
            };
            fn push_key(
                k: &str,
                kind: KeyKind,
                idx: usize,
                by: &mut HashMap<String, Vec<(usize, KeyKind)>>,
            ) {
                let n = crate::geonames::normalize(k);
                if n.is_empty() {
                    return;
                }
                let entry = by.entry(n).or_default();
                for slot in entry.iter_mut() {
                    if slot.0 == idx {
                        if (kind as u8) < (slot.1 as u8) {
                            slot.1 = kind;
                        }
                        return;
                    }
                }
                entry.push((idx, kind));
            }
            push_key(name, KeyKind::Primary, rec_idx, &mut by_name);
            if !asciiname.is_empty() && asciiname != name {
                push_key(asciiname, KeyKind::Ascii, rec_idx, &mut by_name);
            }
            for alt in alternates.split(',') {
                let alt = alt.trim();
                if !alt.is_empty() {
                    push_key(alt, KeyKind::Alternate, rec_idx, &mut by_name);
                }
            }
            records.push(record);
        }
        Index { by_name, records }
    })
}

/// Look up a POI by name. Same primary > ascii > alternate ranking
/// as the cities lookup, tie-broken by population (which for natural
/// features is often 0 — in those cases the first registered record
/// wins, which is stable across reruns because the input file is
/// stable).
pub fn lookup(query: &str) -> Option<&'static PoiRecord> {
    let idx = index();
    let key = crate::geonames::normalize(query);
    if key.is_empty() {
        return None;
    }
    let hits = idx.by_name.get(&key)?;
    let best = hits.iter().min_by_key(|(rec_idx, kind)| {
        (*kind as u8, std::cmp::Reverse(idx.records[*rec_idx].population))
    })?;
    Some(&idx.records[best.0])
}

/// Look up only the *primary-name* match (no alternate-name fallback).
pub fn lookup_primary(query: &str) -> Option<&'static PoiRecord> {
    let idx = index();
    let key = crate::geonames::normalize(query);
    if key.is_empty() {
        return None;
    }
    let hits = idx.by_name.get(&key)?;
    let best = hits
        .iter()
        .filter(|(_, kind)| matches!(kind, KeyKind::Primary | KeyKind::Ascii))
        .max_by_key(|(rec_idx, _)| idx.records[*rec_idx].population)?;
    Some(&idx.records[best.0])
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
    fn mount_everest_resolves() {
        // Everest has dozens of alternate names; passes the ≥3-alt filter
        // and should be in the bundled POI set.
        let r = lookup("Mount Everest")
            .or_else(|| lookup("Everest"))
            .expect("Everest must hit");
        assert!(
            r.lat > 27.0 && r.lat < 28.5 && r.lng > 86.0 && r.lng < 87.5,
            "Everest at ({}, {}) outside Himalayan envelope",
            r.lat, r.lng
        );
        assert!(matches!(r.fcode.as_str(), "MT" | "PK"));
    }

    #[test]
    fn lake_victoria_resolves() {
        let r = lookup("Lake Victoria").expect("Lake Victoria must hit");
        assert!(matches!(r.fcode.as_str(), "LK" | "LKS"));
        assert!(r.lat.abs() < 5.0 && r.lng > 30.0 && r.lng < 36.0);
    }

    #[test]
    fn index_has_reasonable_size() {
        let n = indexed_record_count();
        assert!(
            (200_000..=320_000).contains(&n),
            "POI record count {n} outside expected 200k–320k band"
        );
    }
}
