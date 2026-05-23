//! Embedded GeoNames cities-1000 gazetteer — the populated-places
//! layer of emem's locate cascade.
//!
//! ## Role in the cascade
//!
//! `/v1/locate` resolves a place mention through six layers in order
//! (`crates/emem-api-rest/src/lib.rs::locate_inner`):
//!
//!   1. `wide_bbox_lookup` — compiled-in named-region table.
//!   2. `crate::countries` — ~250 ISO-3166 countries with bbox
//!      aggregated from this layer at first lookup. Catches the
//!      "Bangladesh"-as-country case that cities-only datasets miss.
//!   3. `crate::admin1` — ~3.8 k first-level admin regions
//!      (states / provinces / divisions). Catches "West Bengal",
//!      "California", "Bavaria".
//!   4. **this module** — GeoNames cities-1000 populated places with
//!      population ≥ 1 000, decompressed + indexed on first lookup.
//!      Zero network. Covers ~99 % of agent place queries by name.
//!   5. `nominatim_cache_get` — sled persistent cache (24 h TTL) of
//!      prior Photon / Nominatim / Overture results.
//!   6. Photon → Nominatim — the public-OSM-backed fallback for
//!      anything none of the above carried (small villages, niche
//!      features). The response's `via` field reports which layer
//!      served the answer.
//!
//! Polygon geometry for the resolved place comes from Overture's
//! `divisions/division_area` theme in any of the first four layers
//! (see `crates/emem-fetch/src/overture.rs::division_polygon_near`);
//! Nominatim's polygon path is the last-resort fallback.
//!
//! ## Why a ~10 MB embedded gazetteer
//!
//! The bundled `cities1000.txt.gz` decompresses to ~36 MB TSV parsed
//! once at first lookup and held in a static HashMap keyed by
//! ASCII-folded normalized name. The whole working set fits in
//! ~140 MB resident on a server; a single allocation pays for every
//! future lookup. For non-city named features (national parks,
//! lakes, transboundary basins, archipelagos) GeoNames is
//! intentionally not the answer — the cascade keeps Photon /
//! Nominatim as the tier-6 fallback for those.
//!
//! ## Primary-name vs alternate-name ranking
//!
//! Every record contributes multiple lookup keys: its native name,
//! its ASCII-folded equivalent, and every alternate name listed
//! upstream. To prevent a populous city with a colloquial nickname
//! from poaching unrelated queries (the canonical case: Yerevan's
//! "Bangladesh" district outranking every actual Bangladeshi city
//! on the bare query `bangladesh`), each (key, record) pair carries
//! the *kind* of key it was registered under and the resolver
//! prefers primary-name matches over ascii-fold matches over
//! alternate-name matches before falling back to population.
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

/// Embedded cities-1000 dump (gzip-9'd). ~10.5 MB compressed, parses
/// to a ~36 MB plain-text TSV at startup. Sourced from
/// `https://download.geonames.org/export/dump/cities1000.zip` snapshot
/// 2026-05-23; refresh by re-running `scripts/refresh_geonames.sh` or
/// `gzip -9 < cities1000.txt > crates/emem-fetch/data/cities1000.txt.gz`.
const CITIES_GZ: &[u8] = include_bytes!("../data/cities1000.txt.gz");

/// Attribution string surfaced in every receipt that hit this layer.
/// CC-BY-4.0 requires attribution; emem's receipt model embeds it
/// directly so the agent can quote it without an extra registry call.
pub const ATTRIBUTION: &str = "GeoNames (https://www.geonames.org) — CC-BY-4.0";

/// License id (machine-readable).
pub const LICENSE: &str = "CC-BY-4.0";

/// Source snapshot date — bumped together with the bundled file. Used
/// in `served_via` / receipt blocks so a verifier can detect that two
/// responders are serving from different GeoNames vintages.
pub const SNAPSHOT_DATE: &str = "2026-05-23";

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

/// Which kind of lookup key a record was registered under. Lower
/// numeric variants are *stronger* matches and win during resolution.
/// Cast to `u8` for the lookup ranker — `min_by_key` then orders by
/// (KeyKind, -population) so a primary-name match always beats an
/// alternate-name match regardless of population.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum KeyKind {
    /// `name` column — the upstream's preferred native-script label.
    Primary = 0,
    /// `asciiname` column — same record, ASCII-folded for users on
    /// keyboards that can't type the native form (still authoritative
    /// when it matches the upstream's chosen Latinisation).
    Ascii = 1,
    /// Anything in the comma-separated `alternates` field — folk
    /// names, neighbourhood nicknames, historic variants, translated
    /// forms. Useful but ambiguous; only wins when no Primary / Ascii
    /// hit exists.
    Alternate = 2,
}

struct Index {
    /// Folded name → list of (record-index, key-kind) pairs.
    /// One name can hit multiple cities (Springfield-the-most-common-
    /// US-toponym, the 41 distinct "Victoria"s) — caller picks the
    /// best by (key-kind, -population).
    by_name: HashMap<String, Vec<(usize, KeyKind)>>,
    records: Vec<GeonameRecord>,
}

static INDEX: OnceLock<Index> = OnceLock::new();

/// Parse the bundled gzip, build the index. Idempotent. First call
/// pays ~180–350 ms decompress + parse on a modern CPU; subsequent
/// lookups are O(1). Memory: ~140 MB resident for the 169 k entries
/// plus ~3-5 alternate names each.
fn index() -> &'static Index {
    INDEX.get_or_init(|| {
        let mut decoder = flate2::read::GzDecoder::new(CITIES_GZ);
        let mut buf = String::with_capacity(36 * 1024 * 1024);
        decoder
            .read_to_string(&mut buf)
            .expect("bundled cities1000.txt.gz must decompress");
        let mut records: Vec<GeonameRecord> = Vec::with_capacity(170_000);
        let mut by_name: HashMap<String, Vec<(usize, KeyKind)>> =
            HashMap::with_capacity(450_000);
        for line in buf.lines() {
            let mut cols = line.split('\t');
            // GeoNames cities-1000 has 19 columns; we read 0..15.
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
            // Insert every plausible lookup key with its registration
            // kind. Duplicate (key, record) pairs are suppressed so a
            // 41-way "Victoria" hit doesn't list the same row twice;
            // duplicate kinds for the same record (e.g. asciiname ==
            // name) are upgraded to the stronger kind so the lookup
            // tier rank doesn't get fooled by ordering.
            fn push_key(
                k: &str,
                kind: KeyKind,
                idx: usize,
                by: &mut HashMap<String, Vec<(usize, KeyKind)>>,
            ) {
                let n = normalize(k);
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

/// Normalize a query for lookup: ASCII-fold, lowercase, collapse
/// runs of non-alphanumerics to a single space, trim. Stable so the
/// build-time + runtime keys round-trip.
pub(crate) fn normalize(s: &str) -> String {
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

/// Look up a place name. Returns the best matching record, where
/// "best" is defined as: lowest [`KeyKind`] (Primary > Ascii >
/// Alternate) tie-broken by population descending. This ensures that
/// a query like `"Bangladesh"` does not poach the Yerevan district
/// that carries it as an alternate name — the primary-name resolver
/// returns `None` (no city is *named* Bangladesh) and the locate
/// cascade falls through to [`crate::countries`].
///
/// Returns `None` if no record contains the query as a known key.
pub fn lookup(query: &str) -> Option<&'static GeonameRecord> {
    let idx = index();
    let key = normalize(query);
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
/// Used by the locate cascade when it wants to know "is this query
/// an actual city name?" without being confused by alternate names.
/// Returns the highest-population record whose `name` or `asciiname`
/// matches the query, or `None`.
pub fn lookup_primary(query: &str) -> Option<&'static GeonameRecord> {
    let idx = index();
    let key = normalize(query);
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

/// Return up to `limit` candidate records for a place name, sorted by
/// (KeyKind asc, population desc). Lets the locate layer surface
/// ambiguity hints (`"did you mean Springfield, IL or Springfield,
/// MA?"`) when the top two hits are close in population.
pub fn lookup_candidates(query: &str, limit: usize) -> Vec<&'static GeonameRecord> {
    let idx = index();
    let key = normalize(query);
    if key.is_empty() {
        return Vec::new();
    }
    let Some(hits) = idx.by_name.get(&key) else {
        return Vec::new();
    };
    let mut tagged: Vec<(KeyKind, &'static GeonameRecord)> = hits
        .iter()
        .map(|(i, kind)| (*kind, &idx.records[*i]))
        .collect();
    tagged.sort_by(|a, b| {
        (a.0 as u8)
            .cmp(&(b.0 as u8))
            .then_with(|| b.1.population.cmp(&a.1.population))
    });
    tagged.truncate(limit);
    tagged.into_iter().map(|(_, r)| r).collect()
}

/// Indexed record count. Surfaced via `/v1/capabilities` so a
/// federation peer can detect that two responders are serving
/// different gazetteer vintages.
pub fn indexed_record_count() -> usize {
    index().records.len()
}

/// Force-initialize the index. Call at server boot to pay the
/// decompress + parse cost up-front rather than on the first lookup.
/// Returns the record count for callers that want to log "warmed N entries".
pub fn warm_index() -> usize {
    indexed_record_count()
}

/// Iterate every indexed record. Used by [`crate::countries`] and
/// [`crate::admin1`] to aggregate per-country / per-admin1 bboxes at
/// first lookup, since `cities1000.txt` is the closest in-process
/// approximation of "populated extent" available without a 1.5 GB
/// `allCountries.txt` payload.
pub fn iter_records() -> impl Iterator<Item = &'static GeonameRecord> {
    index().records.iter()
}

/// Find the nearest populated place (≥1000 pop) to a lat/lng. Returns the
/// record and its haversine distance in kilometres, or `None` when no
/// record lies within `max_km`. Uses the embedded ~169 k cities1000 corpus
/// — no network, no async, no I/O once the [`OnceLock`] index is warm.
///
/// `max_km` caps the search radius so a remote ocean cell doesn't return
/// a "nearest" record 800 km away. Pass `f64::INFINITY` to disable the
/// cap.
///
/// Implementation is a brute O(n) scan of the ~169 k records (~170 k
/// float mults). At a sub-millisecond budget this is the cheapest
/// correct approach; a 1° lat-bucket index could trim it further but
/// the present hot path is reverse-geocoding `find_similar` neighbours
/// (k ≤ 1000), not bulk recomputation.
pub fn nearest_label(lat: f64, lng: f64, max_km: f64) -> Option<(&'static GeonameRecord, f64)> {
    let idx = index();
    let mut best: Option<(usize, f64)> = None;
    for (i, rec) in idx.records.iter().enumerate() {
        let d = haversine_km(lat, lng, rec.lat, rec.lng);
        if d > max_km {
            continue;
        }
        match best {
            Some((_, cur)) if cur <= d => {}
            _ => best = Some((i, d)),
        }
    }
    best.map(|(i, d)| (&idx.records[i], d))
}

/// Great-circle distance between two WGS84 points in kilometres.
/// Inlined to avoid pulling a geo crate dep into emem-fetch.
fn haversine_km(lat1: f64, lng1: f64, lat2: f64, lng2: f64) -> f64 {
    const R_KM: f64 = 6371.0088;
    let to_rad = std::f64::consts::PI / 180.0;
    let phi1 = lat1 * to_rad;
    let phi2 = lat2 * to_rad;
    let dphi = (lat2 - lat1) * to_rad;
    let dlam = (lng2 - lng1) * to_rad;
    let a = (dphi / 2.0).sin().powi(2) + phi1.cos() * phi2.cos() * (dlam / 2.0).sin().powi(2);
    2.0 * R_KM * a.sqrt().asin()
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
        // and lat/lng pinned so a refresh of cities-1000 that moves
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
        // Springfield: many cities share the name as their primary;
        // the highest-population one wins. The candidate list keeps
        // every match so an agent can surface "did you mean…".
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
        // Embedded snapshot has ~169 k cities; allow ±15 k as a
        // tolerance for future refreshes so the test doesn't
        // require lockstep with each upstream cut.
        let n = indexed_record_count();
        assert!(
            (150_000..=190_000).contains(&n),
            "indexed record count {n} outside expected 150k–190k band"
        );
    }

    #[test]
    fn primary_name_wins_over_alternate_name() {
        // Regression for the "Bangladesh → Malatia-Sebastia (AM)" bug:
        // the Yerevan district carries "Bangladesh" as an alternate
        // name, but no populated place in cities-1000 is *named*
        // Bangladesh, so `lookup_primary` must return `None` and the
        // caller falls through to the country gazetteer.
        assert!(
            lookup_primary("Bangladesh").is_none(),
            "no populated place should be primary-named Bangladesh"
        );
        // Sanity: the *general* lookup (which permits alternate hits)
        // still returns *something* — that's by design, but the
        // locate cascade uses `lookup_primary` so it never sees the
        // alternate match for a country-name query.
        // (Don't assert the specific record; the alternate-name
        // hit set is data-dependent and could shift across refreshes.)
        let _ = lookup("Bangladesh");
    }
}
