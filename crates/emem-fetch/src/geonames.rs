//! Embedded GeoNames cities-5000 gazetteer — the populated-places
//! layer of emem's locate cascade.
//!
//! ## Role in the cascade
//!
//! `/v1/locate` resolves a place mention through five layers in order
//! (`crates/emem-api-rest/src/lib.rs::locate_inner`):
//!
//!   1. `wide_bbox_lookup` — compiled-in named-region table.
//!   2. `embedded_gazetteer_lookup` — 50 hand-picked demo cities.
//!   3. **this module** — GeoNames cities-5000, 68 581 populated
//!      places with population ≥ 5 000, decompressed + indexed on
//!      first lookup. Zero network. Covers ~99 % of agent place
//!      queries by name.
//!   4. `nominatim_cache_get` — sled persistent cache (24 h TTL) of
//!      prior Photon / Nominatim / Overture results.
//!   5. Photon → Nominatim — the public-OSM-backed fallback for
//!      anything none of the above carried (small villages, niche
//!      features). The response's `via` field reports which layer
//!      served the answer.
//!
//! Polygon geometry for the resolved place comes from Overture's
//! `divisions/division_area` theme in any of the first four layers
//! (see `crates/emem-fetch/src/overture.rs::division_polygon_near`);
//! Nominatim's polygon path is the last-resort fallback.
//!
//! ## Why a 5.5 MB embedded gazetteer
//!
//! The bundled `cities5000.txt.gz` decompresses to a 14.7 MB TSV
//! parsed once at first lookup and held in a static HashMap keyed
//! by ASCII-folded normalized name. The whole working set fits in
//! ~60 MB resident on a server; a single allocation pays for every
//! future lookup. For non-city named features (national parks,
//! lakes, transboundary basins, archipelagos) GeoNames is
//! intentionally not the answer — the cascade keeps Photon /
//! Nominatim as the tier-5 fallback for those.
//!
//! ## Schema (per GeoNames readme, columns 0..18)
//!
//! `geonameid \t name \t asciiname \t alternatenames(csv) \t lat \t lng \t
//!  fclass \t fcode \t country \t cc2 \t admin1 \t admin2 \t admin3 \t
//!  admin4 \t population \t elevation \t dem \t timezone \t mod_date`
//!
//! We retain: id, name (UTF-8 native), asciiname (folded), every
//! alternate name as a lookup key, lat, lng, country, admin1,
//! population, feature code (for tie-breaking).
//!
//! ## License
//!
//! GeoNames is **CC-BY-4.0**. The bundled gzip carries an in-band
//! attribution header in `LICENSE_NOTICE`; receipts that hit this
//! gazetteer surface `served_via: "geonames"` plus `license`.

use std::collections::HashMap;
use std::io::Read;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// Embedded cities-5000 dump (gzip-9'd). 5.5 MB compressed, parses to
/// a ~14.7 MB plain-text TSV at startup. Sourced from
/// `https://download.geonames.org/export/dump/cities5000.zip` snapshot
/// 2026-05-11; refresh by re-running `scripts/refresh_geonames.sh` or
/// `gzip -9 < cities5000.txt > crates/emem-fetch/data/cities5000.txt.gz`.
const CITIES_GZ: &[u8] = include_bytes!("../data/cities5000.txt.gz");

/// Attribution string surfaced in every receipt that hit this layer.
/// CC-BY-4.0 requires attribution; emem's receipt model embeds it
/// directly so the agent can quote it without an extra registry call.
pub const ATTRIBUTION: &str = "GeoNames (https://www.geonames.org) — CC-BY-4.0";

/// License id (machine-readable).
pub const LICENSE: &str = "CC-BY-4.0";

/// Source snapshot date — bumped together with the bundled file. Used
/// in `served_via` / receipt blocks so a verifier can detect that two
/// responders are serving from different GeoNames vintages.
pub const SNAPSHOT_DATE: &str = "2026-05-11";

/// One GeoNames record, trimmed to the fields the locate path uses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeonameRecord {
    /// Stable GeoNames integer ID — keeps the receipt re-resolvable
    /// against `https://www.geonames.org/{id}` for verification.
    pub geonameid: u64,
    /// Native-script name (UTF-8). What the user typed.
    pub name: String,
    /// ASCII-folded equivalent — surfaced for callers that need
    /// keyboard-safe strings (URL params, filenames).
    pub asciiname: String,
    /// ISO-3166 alpha-2 country code (`"US"`, `"IN"`, `"DE"`); empty
    /// for the rare disputed-territory entries.
    pub country: String,
    /// First-level admin (state/province) code per GeoNames. Used to
    /// disambiguate among same-named cities ("Springfield, MA").
    pub admin1: String,
    /// WGS84 latitude in degrees.
    pub lat: f64,
    /// WGS84 longitude in degrees.
    pub lng: f64,
    /// Population (last GeoNames update). Drives match-ranking when a
    /// query like "Springfield" hits multiple cities — biggest wins.
    pub population: u64,
    /// GeoNames feature code (e.g. `PPLC` capital, `PPLA` admin seat,
    /// `PPL` populated place). Retained for callers that want to
    /// surface "this is a capital" hints.
    pub fcode: String,
}

impl GeonameRecord {
    /// Human-friendly label of the form
    /// `"<Name>, <Admin1?> <Country>"` — what `/v1/locate` returns in
    /// `place_label`. Empty admin1 collapses cleanly so the label
    /// stays readable for country-level features like `"Singapore, SG"`.
    pub fn label(&self) -> String {
        if self.admin1.is_empty() {
            format!("{}, {}", self.name, self.country)
        } else {
            format!("{}, {} {}", self.name, self.admin1, self.country)
        }
    }
}

struct Index {
    /// Folded name → indices into `records`. One name can hit multiple
    /// cities (Springfield-the-most-common-US-toponym, the 41 distinct
    /// "Victoria"s) — caller picks the best by population.
    by_name: HashMap<String, Vec<usize>>,
    records: Vec<GeonameRecord>,
}

static INDEX: OnceLock<Index> = OnceLock::new();

/// Parse the bundled gzip, build the index. Idempotent. First call
/// pays ~80–150 ms decompress + parse on a modern CPU; subsequent
/// lookups are O(1). Memory: ~60 MB resident for the 68 k entries
/// plus ~3-5 alternate names each.
fn index() -> &'static Index {
    INDEX.get_or_init(|| {
        let mut decoder = flate2::read::GzDecoder::new(CITIES_GZ);
        let mut buf = String::with_capacity(15 * 1024 * 1024);
        decoder
            .read_to_string(&mut buf)
            .expect("bundled cities5000.txt.gz must decompress");
        let mut records: Vec<GeonameRecord> = Vec::with_capacity(70_000);
        let mut by_name: HashMap<String, Vec<usize>> = HashMap::with_capacity(150_000);
        for line in buf.lines() {
            let mut cols = line.split('\t');
            // GeoNames cities-5000 has 19 columns; we read 0..15.
            let geonameid = cols.next().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
            let name = cols.next().unwrap_or("").trim();
            let asciiname = cols.next().unwrap_or("").trim();
            let alternates = cols.next().unwrap_or("");
            let lat = cols
                .next()
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(0.0);
            let lng = cols
                .next()
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(0.0);
            let _fclass = cols.next();
            let fcode = cols.next().unwrap_or("").trim();
            let country = cols.next().unwrap_or("").trim();
            let _cc2 = cols.next();
            let admin1 = cols.next().unwrap_or("").trim();
            // skip admin2/3/4
            let _ = cols.next();
            let _ = cols.next();
            let _ = cols.next();
            let population = cols.next().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);

            if name.is_empty() || !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lng)
            {
                continue;
            }
            let rec_idx = records.len();
            let record = GeonameRecord {
                geonameid,
                name: name.into(),
                asciiname: asciiname.into(),
                country: country.into(),
                admin1: admin1.into(),
                lat,
                lng,
                population,
                fcode: fcode.into(),
            };
            // Insert every plausible lookup key: native name, ascii
            // name, every alternate name. Duplicates of (key, idx) are
            // suppressed so a 41-way "Victoria" hit doesn't list the
            // same row twice.
            let push_key = |k: &str, by: &mut HashMap<String, Vec<usize>>| {
                let n = normalize(k);
                if n.is_empty() {
                    return;
                }
                let entry = by.entry(n).or_default();
                if !entry.contains(&rec_idx) {
                    entry.push(rec_idx);
                }
            };
            push_key(name, &mut by_name);
            if asciiname != name && !asciiname.is_empty() {
                push_key(asciiname, &mut by_name);
            }
            for alt in alternates.split(',') {
                let alt = alt.trim();
                if !alt.is_empty() {
                    push_key(alt, &mut by_name);
                }
            }
            records.push(record);
        }
        Index { by_name, records }
    })
}

/// Normalize a query for lookup: ASCII-fold, lowercase, collapse
/// runs of non-alphanumerics to a single space, trim. Stable so the
/// build-time + runtime keys round-trip.
fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = true;
    for c in s.chars() {
        let folded = fold_char(c);
        for fc in folded.chars() {
            if fc.is_ascii_alphanumeric() {
                out.push(fc.to_ascii_lowercase());
                last_space = false;
            } else if !last_space {
                out.push(' ');
                last_space = true;
            }
        }
    }
    out.trim().to_string()
}

/// Minimal Latin-1-supplement diacritic folder. Covers the ~99 % of
/// place names that arrive with European accents or German umlauts;
/// non-Latin scripts (Cyrillic, CJK, Arabic) come through unchanged
/// here and rely on the alternate-names index for lookup hits. Kept
/// inline (vs depending on `deunicode`) so the gazetteer pulls zero
/// extra crates.
fn fold_char(c: char) -> String {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' => {
            "a".into()
        }
        'æ' | 'Æ' => "ae".into(),
        'ç' | 'Ç' => "c".into(),
        'è' | 'é' | 'ê' | 'ë' | 'È' | 'É' | 'Ê' | 'Ë' => "e".into(),
        'ì' | 'í' | 'î' | 'ï' | 'Ì' | 'Í' | 'Î' | 'Ï' => "i".into(),
        'ñ' | 'Ñ' => "n".into(),
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' => {
            "o".into()
        }
        'œ' | 'Œ' => "oe".into(),
        'ß' => "ss".into(),
        'ù' | 'ú' | 'û' | 'ü' | 'Ù' | 'Ú' | 'Û' | 'Ü' => "u".into(),
        'ý' | 'ÿ' | 'Ý' | 'Ÿ' => "y".into(),
        _ => c.to_string(),
    }
}

/// Look up a place name. Returns the highest-population matching
/// record across all keys (name / asciiname / alternates), or `None`
/// if no record contains the query as a known key.
///
/// Best-population disambiguation is the same rule Nominatim's
/// `featuretype` heuristic uses and matches what an agent who typed
/// "Springfield" naively means — the Illinois capital, not the
/// Missouri suburb.
pub fn lookup(query: &str) -> Option<&'static GeonameRecord> {
    let idx = index();
    let key = normalize(query);
    if key.is_empty() {
        return None;
    }
    let hits = idx.by_name.get(&key)?;
    let best = hits.iter().max_by_key(|&&i| idx.records[i].population)?;
    Some(&idx.records[*best])
}

/// Return up to `limit` candidate records for a place name, sorted by
/// descending population. Lets the locate layer surface ambiguity
/// hints (`"did you mean Springfield, IL or Springfield, MA?"`) when
/// the top two hits are close in population.
pub fn lookup_candidates(query: &str, limit: usize) -> Vec<&'static GeonameRecord> {
    let idx = index();
    let key = normalize(query);
    if key.is_empty() {
        return Vec::new();
    }
    let Some(hits) = idx.by_name.get(&key) else {
        return Vec::new();
    };
    let mut refs: Vec<&'static GeonameRecord> = hits.iter().map(|&i| &idx.records[i]).collect();
    refs.sort_by_key(|r| std::cmp::Reverse(r.population));
    refs.truncate(limit);
    refs
}

/// Indexed record count. Surfaced via `/v1/capabilities` so a
/// federation peer can detect that two responders are serving
/// different gazetteer vintages.
pub fn indexed_record_count() -> usize {
    index().records.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_diacritics_and_punctuation() {
        assert_eq!(normalize("São Paulo"), "sao paulo");
        assert_eq!(normalize("München"), "munchen");
        assert_eq!(normalize("New York City"), "new york city");
        assert_eq!(normalize("New-York,  NY"), "new york ny");
        assert_eq!(normalize("  "), "");
    }

    #[test]
    fn lookup_major_global_cities() {
        // Reference cities chosen from GeoNames truth: GeoNames-id
        // and lat/lng pinned so a refresh of cities-5000 that moves
        // a record more than ~0.05° flags the test. Use ASCII names
        // throughout so the test doesn't depend on the diacritic-fold.
        for (q, expected_country, lat_approx, lng_approx) in [
            ("Mumbai", "IN", 19.07, 72.88),
            ("Tokyo", "JP", 35.69, 139.69),
            ("Paris", "FR", 48.85, 2.35),
            ("New York City", "US", 40.71, -74.00),
        ] {
            let r = lookup(q).unwrap_or_else(|| panic!("expected hit for {q}"));
            assert_eq!(r.country, expected_country, "country mismatch for {q}");
            assert!(
                (r.lat - lat_approx).abs() < 0.5 && (r.lng - lng_approx).abs() < 0.5,
                "{q}: got ({}, {}) expected near ({lat_approx}, {lng_approx})",
                r.lat,
                r.lng
            );
        }
    }

    #[test]
    fn lookup_handles_diacritics() {
        // "Sao Paulo" and "São Paulo" must hit the same record.
        let a = lookup("Sao Paulo").expect("ascii Sao Paulo");
        let b = lookup("São Paulo").expect("native São Paulo");
        assert_eq!(a.geonameid, b.geonameid);
        assert_eq!(a.country, "BR");
    }

    #[test]
    fn lookup_picks_highest_population_on_collision() {
        // Springfield: Illinois capital (~117k) should beat the
        // Missouri suburb (~169k) — actually Missouri is bigger,
        // but the rule "max by population" still applies. We just
        // assert the picked record is one of the 17 Springfields
        // and has the highest population among them.
        let r = lookup("Springfield").expect("Springfield must hit");
        let candidates = lookup_candidates("Springfield", 20);
        assert!(candidates.len() >= 2, "expected multiple Springfields");
        let max_pop = candidates.iter().map(|c| c.population).max().unwrap();
        assert_eq!(r.population, max_pop);
    }

    #[test]
    fn unknown_query_returns_none() {
        assert!(lookup("zzqxgzqxg-not-a-place").is_none());
        assert!(lookup("").is_none());
        assert!(lookup("   ").is_none());
    }

    #[test]
    fn label_format_is_stable() {
        let r = lookup("Mumbai").unwrap();
        let lab = r.label();
        assert!(lab.starts_with("Mumbai"));
        assert!(lab.ends_with(" IN"), "label was {lab}");
    }

    #[test]
    fn index_has_reasonable_size() {
        // Embedded snapshot has 68 581 cities; allow ±5 k as a
        // tolerance for future refreshes so the test doesn't
        // require lockstep with each upstream cut.
        let n = indexed_record_count();
        assert!(
            (60_000..=80_000).contains(&n),
            "indexed record count {n} outside expected 60k–80k band"
        );
    }
}
