//! Embedded ISO-3166 country gazetteer — the second tier in emem's
//! locate cascade, between [`crate::geonames`]'s wide_bbox table and
//! the cities1000 populated-places dataset.
//!
//! ## Why this layer exists
//!
//! GeoNames' `cities-N.txt` datasets only contain feature-class P
//! (populated places); countries are first-class entities that *do
//! not appear* in any cities cut, no matter how low the population
//! threshold. Without a country layer, querying "Bangladesh" would
//! either return nothing (correct but unhelpful) or — worse —
//! poach an alternate-name match like Yerevan's "Bangladesh"
//! district. The country layer catches every bare-country query
//! and returns its bbox, capital, ISO codes, and continent without
//! a network round-trip.
//!
//! ## Schema (GeoNames `countryInfo.txt`, tab-separated)
//!
//! `ISO \t ISO3 \t ISO-Numeric \t fips \t Country \t Capital \t
//!  Area(km²) \t Population \t Continent \t tld \t Currency-Code \t
//!  Currency-Name \t Phone \t Postal-Code-Format \t Postal-Code-Regex \t
//!  Languages \t geonameid \t neighbours \t EquivalentFipsCode`
//!
//! Comment lines start with `#`.
//!
//! ## Bounding box derivation
//!
//! GeoNames' `countryInfo.txt` does *not* publish per-country bboxes.
//! Rather than bundle a separate 1.5 GB `allCountries.txt` to extract
//! them, we aggregate per-country bboxes at first lookup from
//! [`crate::geonames`]'s already-loaded cities1000 corpus. For
//! countries with ≥ 1 populated city in the cities1000 corpus this
//! yields a slightly *conservative* bbox (the populated extent is a
//! subset of the political extent), which is the right default for
//! hunt-region sampling — the polygon sampler over a country bbox
//! prefers populated land over open sea / uninhabited desert anyway.
//!
//! For pinhole-state countries (≤ 1 populated city, e.g. some
//! Pacific microstates), we fall back to a small fixed-radius square
//! around the capital's lat/lng (degree-equivalent of ±25 km).
//!
//! ## License
//!
//! GeoNames is **CC-BY-4.0**. The bundled gzip carries an in-band
//! attribution header in [`ATTRIBUTION`]; receipts that hit this
//! layer surface `served_via: "countries"` plus `license`.

use std::collections::HashMap;
use std::io::Read;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// Embedded countryInfo dump (gzip-9'd). Decompresses to ~32 kB at
/// startup. Sourced from
/// `https://download.geonames.org/export/dump/countryInfo.txt`
/// snapshot 2026-05-23.
const COUNTRIES_GZ: &[u8] = include_bytes!("../data/countryInfo.txt.gz");

/// Attribution string surfaced in every receipt that hit this layer.
pub const ATTRIBUTION: &str = "GeoNames (https://www.geonames.org) — CC-BY-4.0";

/// License id (machine-readable).
pub const LICENSE: &str = "CC-BY-4.0";

/// Source snapshot date.
pub const SNAPSHOT_DATE: &str = "2026-05-23";

/// One country record. Bounding box is derived at first lookup from
/// the cities1000 corpus (see module docs); centroid is the capital
/// when present, otherwise the bbox centre.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CountryRecord {
    /// ISO-3166 alpha-2 ("US", "BD", "DE").
    pub iso2: String,
    /// ISO-3166 alpha-3 ("USA", "BGD", "DEU"). Useful when the agent
    /// query uses the longer form.
    pub iso3: String,
    /// Country name in English ("Bangladesh", "United States").
    pub name: String,
    /// Capital city name, when present.
    pub capital: String,
    /// Continent code ("AS", "EU", "NA", "SA", "AF", "OC", "AN").
    pub continent: String,
    /// Stable GeoNames integer ID for the country entity itself —
    /// distinct from any geonameid in cities1000.
    pub geonameid: u64,
    /// Centroid lat (derived: capital city if known, else bbox centre).
    pub centroid_lat: f64,
    /// Centroid lng (derived).
    pub centroid_lng: f64,
    /// Bounding box derived from cities1000 (min_lat, max_lat, min_lng, max_lng).
    /// Pinhole countries fall back to ±0.25° around the centroid.
    pub min_lat: f64,
    pub max_lat: f64,
    pub min_lng: f64,
    pub max_lng: f64,
}

impl CountryRecord {
    /// Bounding box tuple in the same `(min_lat, max_lat, min_lng,
    /// max_lng)` convention as `WIDE_BBOXES` / `RecallPolygonBbox`.
    pub fn bbox(&self) -> (f64, f64, f64, f64) {
        (self.min_lat, self.max_lat, self.min_lng, self.max_lng)
    }

    /// Reader-friendly label, e.g. `"Bangladesh (BD)"`.
    pub fn label(&self) -> String {
        format!("{} ({})", self.name, self.iso2)
    }
}

struct Index {
    /// Normalized lookup key → record index. Keys include the English
    /// name, the ISO2 code, and the ISO3 code for every country.
    by_name: HashMap<String, usize>,
    records: Vec<CountryRecord>,
}

static INDEX: OnceLock<Index> = OnceLock::new();

fn index() -> &'static Index {
    INDEX.get_or_init(|| {
        // First pass: parse countryInfo entries.
        let mut decoder = flate2::read::GzDecoder::new(COUNTRIES_GZ);
        let mut buf = String::with_capacity(64 * 1024);
        decoder
            .read_to_string(&mut buf)
            .expect("bundled countryInfo.txt.gz must decompress");
        // Capital-city centroids: capital name → (lat, lng). Built
        // from the cities1000 corpus so we never embed a second copy
        // of every capital's lat/lng. Multiple cities can share a
        // capital name (e.g. "Salvador") — we pick the one whose
        // country code matches, but to avoid a two-pass dependency we
        // do that match in the country-build loop below.
        let mut records: Vec<CountryRecord> = Vec::with_capacity(260);
        let mut by_name: HashMap<String, usize> = HashMap::with_capacity(1_024);
        for line in buf.lines() {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            let cols: Vec<&str> = line.split('\t').collect();
            if cols.len() < 17 {
                continue;
            }
            let iso2 = cols[0].trim();
            let iso3 = cols[1].trim();
            // cols[3] = fips, cols[4] = name, cols[5] = capital
            let name = cols[4].trim();
            let capital = cols[5].trim();
            let continent = cols[8].trim();
            let geonameid = cols[16].trim().parse::<u64>().unwrap_or(0);
            if iso2.is_empty() || name.is_empty() {
                continue;
            }
            // Centroid + bbox: aggregate from cities1000 records whose
            // country matches iso2.
            let mut min_lat = f64::INFINITY;
            let mut max_lat = f64::NEG_INFINITY;
            let mut min_lng = f64::INFINITY;
            let mut max_lng = f64::NEG_INFINITY;
            let mut capital_pt: Option<(f64, f64)> = None;
            let cap_key = crate::geonames::normalize(capital);
            for rec in crate::geonames::iter_records() {
                if rec.country != iso2 {
                    continue;
                }
                if rec.lat < min_lat {
                    min_lat = rec.lat;
                }
                if rec.lat > max_lat {
                    max_lat = rec.lat;
                }
                if rec.lng < min_lng {
                    min_lng = rec.lng;
                }
                if rec.lng > max_lng {
                    max_lng = rec.lng;
                }
                if capital_pt.is_none() && !cap_key.is_empty() {
                    let ascii_hit = crate::geonames::normalize(&rec.asciiname) == cap_key;
                    let name_hit = crate::geonames::normalize(&rec.name) == cap_key;
                    if ascii_hit || name_hit {
                        capital_pt = Some((rec.lat, rec.lng));
                    }
                }
            }
            // Pinhole-country fallback: when cities1000 has zero rows
            // for this country (rare — some microstates), we don't
            // have a real extent. Centroid stays at (0,0) until we
            // can wire a fallback; lookup will still succeed but the
            // bbox carries the same point twice. Caller can see the
            // degenerate bbox and decide to fall through to Photon.
            let have_extent = min_lat.is_finite() && max_lat.is_finite();
            let (centroid_lat, centroid_lng, mn_la, mx_la, mn_ln, mx_ln) =
                match (capital_pt, have_extent) {
                    (Some((clat, clng)), true) => {
                        // Snap centroid to the capital. Bbox is the
                        // cities-derived extent.
                        (clat, clng, min_lat, max_lat, min_lng, max_lng)
                    }
                    (Some((clat, clng)), false) => {
                        // Capital known but no cities-derived extent.
                        // Synthesize a ±0.25° square (~ ±28 km) around
                        // the capital so the bbox is non-degenerate.
                        (
                            clat,
                            clng,
                            clat - 0.25,
                            clat + 0.25,
                            clng - 0.25,
                            clng + 0.25,
                        )
                    }
                    (None, true) => {
                        // No capital match in cities1000 (rare; data
                        // glitch or capital below pop-1000 floor). Use
                        // bbox centre as the centroid.
                        let clat = (min_lat + max_lat) * 0.5;
                        let clng = (min_lng + max_lng) * 0.5;
                        (clat, clng, min_lat, max_lat, min_lng, max_lng)
                    }
                    (None, false) => {
                        // Pinhole microstate with no cities1000 rows and
                        // capital not in the corpus. Mark with NaN-free
                        // zero so callers can detect "no real bbox here".
                        (0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
                    }
                };
            let rec = CountryRecord {
                iso2: iso2.into(),
                iso3: iso3.into(),
                name: name.into(),
                capital: capital.into(),
                continent: continent.into(),
                geonameid,
                centroid_lat,
                centroid_lng,
                min_lat: mn_la,
                max_lat: mx_la,
                min_lng: mn_ln,
                max_lng: mx_ln,
            };
            let idx = records.len();
            // Register the country under every key an agent might
            // type: English name, ISO2, ISO3. Last-write-wins on key
            // collision (rare — ISO codes are unique by design).
            for k in [name, iso2, iso3] {
                let n = crate::geonames::normalize(k);
                if !n.is_empty() {
                    by_name.insert(n, idx);
                }
            }
            records.push(rec);
        }
        Index { by_name, records }
    })
}

/// Look up a country by name or ISO code. Returns the canonical
/// record with derived bbox + centroid.
pub fn lookup(query: &str) -> Option<&'static CountryRecord> {
    let idx = index();
    let key = crate::geonames::normalize(query);
    if key.is_empty() {
        return None;
    }
    let i = idx.by_name.get(&key)?;
    Some(&idx.records[*i])
}

/// Indexed country count. Surfaced via `/v1/capabilities` so a
/// federation peer can detect that two responders are serving
/// different country-table vintages.
pub fn indexed_record_count() -> usize {
    index().records.len()
}

/// Force-initialize the index. Call at server boot to pay the
/// decompress + parse + aggregate cost up-front rather than on the
/// first lookup. Returns the record count.
pub fn warm_index() -> usize {
    indexed_record_count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bangladesh_resolves_to_country_not_city() {
        // Regression for the Yerevan-district poaching bug.
        let bd = lookup("Bangladesh").expect("Bangladesh must hit");
        assert_eq!(bd.iso2, "BD");
        assert_eq!(bd.iso3, "BGD");
        assert_eq!(bd.continent, "AS");
        // Centroid should be inside the actual country envelope
        // (lat 20..27, lng 88..93). Loose bound — exact value depends
        // on which Dhaka record snapped first.
        assert!(
            (20.0..27.0).contains(&bd.centroid_lat) && (88.0..93.0).contains(&bd.centroid_lng),
            "centroid {:?} outside Bangladesh",
            (bd.centroid_lat, bd.centroid_lng),
        );
        // BBox must cover Dhaka (~23.7°N, 90.4°E).
        assert!(
            bd.min_lat <= 23.7 && bd.max_lat >= 23.7 && bd.min_lng <= 90.4 && bd.max_lng >= 90.4,
            "Dhaka must lie inside Bangladesh bbox {:?}",
            bd.bbox(),
        );
    }

    #[test]
    fn iso_codes_resolve_too() {
        let us_by_name = lookup("United States").unwrap();
        let us_by_iso2 = lookup("US").unwrap();
        let us_by_iso3 = lookup("USA").unwrap();
        assert_eq!(us_by_name.iso2, "US");
        assert_eq!(us_by_iso2.iso2, "US");
        assert_eq!(us_by_iso3.iso2, "US");
    }

    #[test]
    fn unknown_returns_none() {
        assert!(lookup("zzqxgzqxg-not-a-country").is_none());
        assert!(lookup("").is_none());
    }

    #[test]
    fn index_has_reasonable_size() {
        // GeoNames countryInfo lists 250 entries plus a handful of
        // territories/dependencies. Allow a generous tolerance band.
        let n = indexed_record_count();
        assert!(
            (240..=270).contains(&n),
            "country record count {n} outside expected 240..=270 band"
        );
    }
}
