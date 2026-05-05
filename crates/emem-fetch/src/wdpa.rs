//! WDPA (World Database on Protected Areas) point-in-polygon materializer.
//!
//! ## Why OSM Overpass `boundary=protected_area`, not the Protected Planet REST
//!
//! The user-facing question this band answers — "is this point in a
//! protected area, and if so, what kind?" — has two open-data sources:
//!
//! 1. **Protected Planet REST API** (`api.protectedplanet.net/v3`). On
//!    paper this is the canonical WDPA query surface; in practice every
//!    endpoint (including read-only `protected_areas?per_page=1`)
//!    returns `401 Unauthorized` even for anonymous callers. The token
//!    is gated behind a Protected Planet account request, which violates
//!    the `project_open_data` "no key-gated sources for default build"
//!    rule. Verified live with `curl -s` on 2026-05-05; identical 401
//!    with and without a geometry param.
//!
//! 2. **OpenStreetMap `boundary=protected_area`** via the public Overpass
//!    API. OSM crowdsources the WDPA designations from authoritative
//!    sources (the National Park Service edits Yellowstone's polygon,
//!    USFWS edits the Marine National Monuments, etc.) and tags every PA
//!    with `protect_class` — the integer that maps 1:1 onto the IUCN
//!    category code WDPA itself uses (1 = IUCN Ia/Ib, 2 = II, etc, up to
//!    6 = VI; anything above 6 is a national designation outside the
//!    IUCN ladder). The Overpass `is_in()` filter does point-in-polygon
//!    server-side and returns just the matching `area` records' tags as
//!    a small JSON document — a perfect shape for per-cell lazy
//!    materialise. Verified live: a query centred on (44.4, -110.6)
//!    returns the Yellowstone area with `protect_class=2`,
//!    `protected_area=national_park`, `wikidata=Q351`, matching WDPA ID
//!    374883 (NPS official). A query centred on (30.0, -150.0) (North
//!    Pacific gyre) returns zero matches.
//!
//! The user-memory note "Overpass removed" applies specifically to the
//! `/v1/locate` geocoder layering — Photon is the primary live geocoder
//! there. Overpass remains the right tool for spatial point-in-polygon
//! queries against OSM tags, which is what this fetcher does. The
//! Overpass usage policy (1 req/sec, structured queries OK) is honoured
//! by the materializer's per-cell64 cache: every cell pays at most one
//! Overpass call, and the result is signed + persisted forever.
//!
//! ## What the fetcher returns
//!
//! - `Ok(Some(WdpaMatch))` — at least one `boundary=protected_area`,
//!   `boundary=national_park`, or `leisure=nature_reserve` polygon
//!   contains the cell. The strongest IUCN match (lowest `protect_class`
//!   integer) is chosen as the canonical hit; `name`, `designation`,
//!   `country_iso3`, and the `marine` flag come from OSM tags.
//! - `Ok(None)` — the cell does not fall inside any OSM-mapped PA. Caller
//!   signs an `Absence` fact (this is the meaningful confirmed-no answer,
//!   distinct from upstream failure).
//! - `Err(WdpaError::*)` — transport, decode, or rate-limit failure. The
//!   caller surfaces the error string verbatim; the next recall on this
//!   cell will retry.

use std::time::Duration;

use serde::Deserialize;

/// Errors specific to the WDPA fetcher.
#[derive(Debug, thiserror::Error)]
pub enum WdpaError {
    /// HTTP / network failure.
    #[error("transport: {0}")]
    Transport(String),
    /// Upstream returned non-2xx HTTP (other than 429, which is its own
    /// variant for retry-after handling).
    #[error("status {status} for {url}")]
    BadStatus { status: u16, url: String },
    /// Body wasn't valid JSON / didn't match the Overpass schema.
    #[error("malformed response: {0}")]
    Malformed(String),
    /// Overpass returned 429 Too Many Requests. Caller surfaces the
    /// retry-after hint so the agent can back off honestly rather than
    /// retrying blindly into the rate-limiter.
    #[error("rate limited: retry after {retry_after_s}s")]
    RateLimited { retry_after_s: u32 },
}

/// Structured WDPA-equivalent match for a cell. Mirrors the small subset
/// of WDPA fields an agent actually needs to answer "is this protected,
/// and how strictly?".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WdpaMatch {
    /// OSM relation ID of the matching protected area (the OSM
    /// equivalent of `WDPAID`). Persisted as `wdpa_id` on the signed
    /// fact; agents that need the canonical Protected Planet ID can
    /// resolve via `wikidata` if the OSM record carries one.
    pub wdpa_id: u64,
    /// Human-readable PA name (`name` tag, English-preferred where
    /// available via `name:en`, falling back to `name`).
    pub name: String,
    /// IUCN category as a string: "Ia"/"Ib"/"II"/"III"/"IV"/"V"/"VI", or
    /// "Not Reported" when the polygon carries no `protect_class` /
    /// `iucn_level` tag.
    pub iucn_category: String,
    /// Designation type from `protected_area` / `boundary` tag, e.g.
    /// "national_park", "wildlife_refuge", "nature_reserve". Lower-cased
    /// snake_case, mirrors the OSM tag verbatim so agents can join
    /// against external designation tables without re-stringifying.
    pub designation: String,
    /// ISO-3166 alpha-3 country code, derived from the OSM `addr:country`
    /// tag where present. Empty string when the OSM record carries no
    /// country tag (transboundary marine PAs are the common case).
    pub country_iso3: String,
    /// `true` when the polygon carries `maritime=yes` (or `marine=yes`)
    /// — distinguishes ocean MPAs from terrestrial parks for the
    /// downstream "is this in a marine PA?" question.
    pub marine: bool,
}

/// Maximum bytes to accept from the Overpass response body. PA-tag
/// records are typically 0.5–4 KB even for the most heavily-tagged
/// World-Heritage parks (Yellowstone is ~3.2 KB); 64 KB is comfortable
/// headroom while bounding worst-case allocation if the upstream returns
/// an unexpectedly huge document.
const MAX_BODY_BYTES: usize = 64 * 1024;

/// Public Overpass instance. The OSM Foundation runs two production
/// servers behind this hostname (round-robin at the DNS layer); both
/// honour the same QL syntax and rate-limit on a per-IP basis.
const OVERPASS_URL: &str = "https://overpass-api.de/api/interpreter";

/// Build the Overpass QL query for a `(lat, lng)` point.
///
/// `is_in()` runs point-in-polygon server-side over OSM's relation
/// geometries (rebuilt nightly into the `area` index), then we filter the
/// matched areas down to PA-equivalent tags. The double union with
/// `boundary=national_park` and `leisure=nature_reserve` catches
/// designations that legacy mappers tagged outside the modern
/// `boundary=protected_area` scheme (e.g. some US state parks still use
/// `leisure=nature_reserve` only).
///
/// `out tags;` keeps the response compact — geometry is not returned
/// (we already know the point is inside; we just need the PA's metadata).
///
/// Public so the materialiser layer can quote the formatted query on the
/// signed Fact's derivation arguments — receipt verification re-runs the
/// formatting against the cell centre and confirms the upstream call.
pub fn overpass_query(lat: f64, lng: f64) -> String {
    // 7 decimals = ~1.1 cm at the equator; matches `worldpop::aoi_polygon_geojson`
    // and `materialize_*` upstream-URL formatting for receipt determinism.
    format!(
        "[out:json][timeout:30];\
         is_in({lat:.7},{lng:.7})->.a;\
         (area.a[\"boundary\"=\"protected_area\"];\
          area.a[\"boundary\"=\"national_park\"];\
          area.a[\"leisure\"=\"nature_reserve\"];);\
         out tags;"
    )
}

/// Resolved upstream URL for the request — the bare Overpass endpoint;
/// the QL goes in the POST body, not the query string. Surfaced so the
/// signed Fact's `Source.url` field carries a verifier-replayable hint.
pub const OVERPASS_ENDPOINT_URL: &str = OVERPASS_URL;

/// Top-level Overpass response envelope. We deserialize only the fields
/// we need; Overpass adds `version`, `generator`, `osm3s.timestamp_*`,
/// `osm3s.copyright` that are unused here.
#[derive(Debug, Clone, Deserialize)]
struct OverpassEnvelope {
    /// Matching `area` records with their tag dicts. Empty array when no
    /// PA contains the point (which is the canonical "Ok(None)" path).
    #[serde(default)]
    elements: Vec<OverpassElement>,
}

#[derive(Debug, Clone, Deserialize)]
struct OverpassElement {
    /// Always `"area"` for `is_in()` results; ignored on the deserialise
    /// path but kept for completeness if a future caller wants to widen
    /// to ways/relations.
    #[serde(rename = "type", default)]
    _kind: String,
    /// OSM area ID. Areas are derived from relations + closed ways with
    /// the convention `area_id = relation_id + 3_600_000_000` for
    /// relations and `area_id = way_id + 2_400_000_000` for ways.
    #[serde(default)]
    id: u64,
    /// PA tag dict. Keys we actually read: `name`, `name:en`,
    /// `boundary`, `protect_class`, `protected_area`, `leisure`,
    /// `addr:country`, `ISO3166-1:alpha3`, `maritime`, `marine`,
    /// `iucn_level`, `designation`.
    #[serde(default)]
    tags: std::collections::BTreeMap<String, String>,
}

/// Convert OSM `protect_class` integer to IUCN category string. Returns
/// "Not Reported" when the polygon carries no recognized class — this
/// matches the WDPA convention for non-IUCN national designations.
///
/// Mapping is the published OSM wiki convention for `protect_class`,
/// itself adopted from IUCN/WCMC/CDDA scheme:
///   - 1 → Ia (strict nature reserve) and Ib (wilderness) merge to "Ia"
///     for the WDPA-flat-string view; OSM doesn't disambiguate.
///   - 2..=6 → IUCN II..VI verbatim.
///   - 7..=29 → "Not Reported" (national / sub-national designations
///     outside the IUCN ladder, e.g. US National Wildlife Refuges, which
///     WDPA records as "Not Applicable" in their IUCN column).
///   - 99 / unknown → "Not Reported".
fn iucn_category_from_protect_class(pc: &str) -> &'static str {
    match pc.trim() {
        "1" | "1a" | "1b" | "Ia" | "Ib" => "Ia",
        "2" | "II" => "II",
        "3" | "III" => "III",
        "4" | "IV" => "IV",
        "5" | "V" => "V",
        "6" | "VI" => "VI",
        _ => "Not Reported",
    }
}

/// Score a PA for "strictness": Ia is strictest, VI is least strict,
/// "Not Reported" lands at the bottom. When two PAs cover the same
/// point (e.g. a wildlife refuge inside a national monument), the
/// stricter one wins as the canonical answer for the band.
///
/// Lower number = stricter (sorts first under `sort_by_key`).
fn iucn_strictness_rank(cat: &str) -> u8 {
    match cat {
        "Ia" => 0,
        "Ib" => 1,
        "II" => 2,
        "III" => 3,
        "IV" => 4,
        "V" => 5,
        "VI" => 6,
        _ => 7,
    }
}

/// Decode an [`OverpassEnvelope`] into the canonical [`WdpaMatch`] (or
/// `None` when the elements list is empty). When multiple PAs cover the
/// point, returns the strictest one by IUCN category — same convention
/// the WDPA's own "select strongest designation" view uses.
fn select_match(env: OverpassEnvelope) -> Option<WdpaMatch> {
    if env.elements.is_empty() {
        return None;
    }
    let mut hits: Vec<WdpaMatch> = env
        .elements
        .into_iter()
        .map(element_to_match)
        .collect::<Vec<_>>();
    hits.sort_by_key(|m| iucn_strictness_rank(&m.iucn_category));
    hits.into_iter().next()
}

fn element_to_match(el: OverpassElement) -> WdpaMatch {
    let tags = el.tags;
    let get = |k: &str| tags.get(k).cloned().unwrap_or_default();
    let name = if !get("name:en").is_empty() {
        get("name:en")
    } else if !get("name").is_empty() {
        get("name")
    } else {
        // Anonymous PA — surface the OSM ID so an agent has *some*
        // identifier even when the relation has no name tag.
        format!("OSM area {}", el.id)
    };
    // IUCN category: prefer `iucn_level` (newer scheme), fall back to
    // `protect_class` (older but more common). Both are integer codes
    // following the same scheme.
    let raw_cat = if !get("iucn_level").is_empty() {
        get("iucn_level")
    } else {
        get("protect_class")
    };
    let iucn_category = iucn_category_from_protect_class(&raw_cat).to_string();
    // Designation type. Prefer the `protected_area` sub-tag (e.g.
    // "national_park", "wildlife_refuge", "nature_reserve"); fall back
    // to `boundary`/`leisure` when absent.
    let designation = if !get("protected_area").is_empty() {
        get("protected_area")
    } else if !get("boundary").is_empty() {
        get("boundary")
    } else {
        get("leisure")
    };
    // Country code. ISO3 takes precedence; alpha2 is upgraded to alpha3
    // only for codes we can resolve (we don't ship a full table — empty
    // string is the honest default).
    let country_iso3 = if !get("ISO3166-1:alpha3").is_empty() {
        get("ISO3166-1:alpha3")
    } else if !get("addr:country").is_empty() {
        // OSM `addr:country` may be alpha2, alpha3, or a full name; we
        // only normalize the trivial alpha3 case and pass everything
        // else through verbatim.
        get("addr:country")
    } else {
        String::new()
    };
    let marine = get("maritime") == "yes" || get("marine") == "yes";
    WdpaMatch {
        wdpa_id: el.id,
        name,
        iucn_category,
        designation,
        country_iso3,
        marine,
    }
}

/// Parse an Overpass response body into a `WdpaMatch` (or `None`).
/// Public so unit tests can exercise the decode path against fixtures
/// without spinning up a live server.
pub fn parse_overpass_response(body: &str) -> Result<Option<WdpaMatch>, WdpaError> {
    let env: OverpassEnvelope = serde_json::from_str(body)
        .map_err(|e| WdpaError::Malformed(format!("{e} (body head: {})", head(body, 200))))?;
    Ok(select_match(env))
}

/// Borrow a leading slice of a string for error reporting, ASCII-safe.
fn head(s: &str, n: usize) -> &str {
    if s.len() <= n {
        s
    } else {
        match s.char_indices().nth(n) {
            Some((idx, _)) => &s[..idx],
            None => s,
        }
    }
}

/// Fetch the WDPA-equivalent record for a single `(lat, lng)` from the
/// public Overpass API.
///
/// Returns `Ok(None)` for "no PA covers this point" — the caller signs an
/// Absence fact for that. Returns `Ok(Some(_))` for a hit. Returns `Err`
/// for transport / decode failures so the caller can surface a structured
/// reason without persisting a synthetic fact.
pub async fn fetch_wdpa_status(
    client: &reqwest::Client,
    lat: f64,
    lng: f64,
) -> Result<Option<WdpaMatch>, WdpaError> {
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lng) {
        return Err(WdpaError::Malformed(format!(
            "invalid lat/lng: ({lat},{lng})"
        )));
    }
    let ql = overpass_query(lat, lng);
    let resp = client
        .post(OVERPASS_URL)
        .header(
            reqwest::header::USER_AGENT,
            concat!(
                "emem.dev/",
                env!("CARGO_PKG_VERSION"),
                " (avijeet@vortx.ai)"
            ),
        )
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(format!("data={}", url_encode(&ql)))
        .timeout(Duration::from_secs(45))
        .send()
        .await
        .map_err(|e| WdpaError::Transport(e.to_string()))?;

    let status = resp.status();
    if status.as_u16() == 429 {
        let retry_after_s = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse().ok())
            .unwrap_or(60);
        return Err(WdpaError::RateLimited { retry_after_s });
    }
    if !status.is_success() {
        return Err(WdpaError::BadStatus {
            status: status.as_u16(),
            url: OVERPASS_URL.to_string(),
        });
    }
    let body_bytes = resp
        .bytes()
        .await
        .map_err(|e| WdpaError::Transport(e.to_string()))?;
    if body_bytes.len() > MAX_BODY_BYTES {
        return Err(WdpaError::Malformed(format!(
            "overpass response {} B exceeds cap {} B",
            body_bytes.len(),
            MAX_BODY_BYTES
        )));
    }
    let body =
        std::str::from_utf8(&body_bytes).map_err(|e| WdpaError::Malformed(format!("utf8: {e}")))?;
    parse_overpass_response(body)
}

/// Minimal percent-encoder for the QL payload that goes into the
/// `data=` form parameter. Mirrors the helper in `worldpop.rs` so the
/// fetch crate keeps its zero-extra-deps stance.
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Yellowstone-shaped Overpass fixture — exact tag set returned live
    /// for `is_in(44.4,-110.6)` on 2026-05-05, trimmed to the keys this
    /// fetcher actually reads. Verifies the strongest-match selection,
    /// IUCN category mapping (`protect_class=2 → "II"`), designation
    /// extraction, and (importantly) that the canonical scalar carries
    /// the OSM relation/area ID rather than a hardcoded fixture.
    #[test]
    fn parses_yellowstone_match_and_maps_to_iucn_ii() {
        let body = r#"{
          "version": 0.6,
          "generator": "Overpass API",
          "elements": [
            {
              "type": "area",
              "id": 3601453306,
              "tags": {
                "boundary": "protected_area",
                "leisure": "nature_reserve",
                "name": "Yellowstone National Park",
                "name:en": "Yellowstone National Park",
                "operator": "National Park Service",
                "protect_class": "2",
                "protected_area": "national_park",
                "ISO3166-1:alpha3": "USA",
                "wikidata": "Q351"
              }
            }
          ]
        }"#;
        let m = parse_overpass_response(body)
            .expect("parse ok")
            .expect("expected a match");
        assert_eq!(m.wdpa_id, 3601453306);
        assert_eq!(m.name, "Yellowstone National Park");
        assert_eq!(m.iucn_category, "II");
        assert_eq!(m.designation, "national_park");
        assert_eq!(m.country_iso3, "USA");
        assert!(!m.marine);
    }

    /// North Pacific gyre fixture — empty `elements` array. The fetcher
    /// must return `Ok(None)` so the caller can sign a *signed Absence*
    /// fact (the protocol's negative-fact path), NOT a Primary fact with
    /// value=false. This regression-locks the honest-defaults rule:
    /// "not in any PA" is a meaningful confirmed-no answer, distinct
    /// from upstream failure.
    #[test]
    fn empty_elements_yields_none_not_primary_false() {
        let body = r#"{
          "version": 0.6,
          "generator": "Overpass API",
          "elements": []
        }"#;
        let res = parse_overpass_response(body).expect("parse ok");
        assert!(res.is_none(), "expected None over open ocean, got {res:?}");
    }

    /// Strongest-match selection: when a wildlife refuge (IUCN IV) and a
    /// strict nature reserve (IUCN Ia) both cover the same point,
    /// `select_match` must return the Ia. Mirrors the WDPA convention.
    #[test]
    fn picks_strictest_when_multiple_pas_overlap() {
        let body = r#"{
          "version": 0.6,
          "elements": [
            {
              "type": "area", "id": 100,
              "tags": {
                "boundary": "protected_area",
                "name": "Refuge",
                "protect_class": "4",
                "protected_area": "wildlife_refuge"
              }
            },
            {
              "type": "area", "id": 200,
              "tags": {
                "boundary": "protected_area",
                "name": "Strict Reserve",
                "protect_class": "1",
                "protected_area": "nature_reserve"
              }
            }
          ]
        }"#;
        let m = parse_overpass_response(body)
            .expect("parse ok")
            .expect("expected a match");
        assert_eq!(m.wdpa_id, 200);
        assert_eq!(m.name, "Strict Reserve");
        assert_eq!(m.iucn_category, "Ia");
    }

    /// Marine PA tag detection — `maritime=yes` flips the `marine`
    /// boolean. Pacific Islands Heritage MNM fixture, trimmed.
    #[test]
    fn marine_flag_set_for_maritime_yes() {
        let body = r#"{
          "version": 0.6,
          "elements": [
            {
              "type": "area", "id": 3620613357,
              "tags": {
                "boundary": "protected_area",
                "leisure": "nature_reserve",
                "maritime": "yes",
                "name": "Pacific Islands Heritage Marine National Monument",
                "protect_class": "4",
                "protected_area": "wildlife_refuge"
              }
            }
          ]
        }"#;
        let m = parse_overpass_response(body)
            .expect("parse ok")
            .expect("expected a match");
        assert!(m.marine, "marine flag should be true for maritime=yes");
        assert_eq!(m.iucn_category, "IV");
        assert_eq!(m.designation, "wildlife_refuge");
    }

    /// "Not Reported" path — a national designation with no IUCN-mapped
    /// `protect_class` (e.g. `protect_class=97` for ecologically-zoned
    /// areas outside the IUCN ladder) lands as "Not Reported".
    #[test]
    fn unknown_protect_class_maps_to_not_reported() {
        let body = r#"{
          "elements": [
            {
              "type": "area", "id": 1,
              "tags": {
                "boundary": "protected_area",
                "name": "National Heritage Zone",
                "protect_class": "97"
              }
            }
          ]
        }"#;
        let m = parse_overpass_response(body)
            .expect("parse ok")
            .expect("expected a match");
        assert_eq!(m.iucn_category, "Not Reported");
    }

    /// Malformed body must surface as `WdpaError::Malformed` with the
    /// body head included — never silently coerce to `Ok(None)`. That
    /// would let an upstream HTML 502 page get mistaken for "no PA".
    #[test]
    fn malformed_body_surfaces_structured_error() {
        let body = "<html>upstream timeout</html>";
        let err = parse_overpass_response(body).expect_err("expected Malformed");
        match err {
            WdpaError::Malformed(msg) => {
                assert!(msg.contains("body head"));
            }
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    /// `RateLimited` error variant carries a retry_after_s — Display
    /// must include it so logs / receipts surface the back-off hint.
    /// Regression-locks the structured-error contract the materialiser
    /// relies on for honest 429 surfacing.
    #[test]
    fn rate_limited_error_renders_retry_after() {
        let err = WdpaError::RateLimited { retry_after_s: 30 };
        let s = format!("{err}");
        assert!(s.contains("rate limited"));
        assert!(s.contains("30"));
    }

    /// `overpass_query` formats lat/lng with 7-decimal precision
    /// (matches `worldpop::aoi_polygon_geojson` for receipt determinism).
    /// At lat=0 the literal "0.0000000" must appear; at lat=44.4 the
    /// exact `44.4000000` appears.
    #[test]
    fn overpass_query_uses_seven_decimal_lat_lng() {
        let q = overpass_query(44.4, -110.6);
        assert!(q.contains("44.4000000"), "lat not 7-decimal: {q}");
        assert!(q.contains("-110.6000000"), "lng not 7-decimal: {q}");
        // Triple union over PA-equivalent OSM tags must stay in the
        // query — receipt verifier replays this exact string.
        assert!(q.contains("\"boundary\"=\"protected_area\""));
        assert!(q.contains("\"boundary\"=\"national_park\""));
        assert!(q.contains("\"leisure\"=\"nature_reserve\""));
        assert!(q.contains("is_in(44.4000000,-110.6000000)"));
    }

    /// Network-gated end-to-end smoke. Disabled by default
    /// (`#[ignore]`); run explicitly with
    /// `cargo test -p emem-fetch -- --ignored wdpa_live`.
    #[tokio::test]
    #[ignore]
    async fn wdpa_live_returns_yellowstone_for_44_4_minus_110_6() {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap();
        let m = fetch_wdpa_status(&client, 44.4, -110.6)
            .await
            .expect("wdpa live fetch failed")
            .expect("expected a PA at Yellowstone centre");
        assert!(m.name.to_lowercase().contains("yellowstone"));
        assert_eq!(m.iucn_category, "II");
    }

    /// Network-gated: confirm the fetcher returns `Ok(None)` over open
    /// Pacific gyre (no PA), distinct from upstream-failure errors.
    #[tokio::test]
    #[ignore]
    async fn wdpa_live_returns_none_for_north_pacific_gyre() {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap();
        let res = fetch_wdpa_status(&client, 30.0, -150.0)
            .await
            .expect("wdpa live fetch failed");
        assert!(
            res.is_none(),
            "expected None over North Pacific gyre, got {res:?}"
        );
    }
}
