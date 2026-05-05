//! WorldPop population materializer — anonymous JSON REST against the
//! public WorldPop Statistics API at `api.worldpop.org/v1/services/stats`.
//!
//! ## Why this path (and not vsicurl COG range reads)
//!
//! WorldPop publishes its global mosaic as a single ~870 MB GeoTIFF at
//! `data.worldpop.org/.../ppp_2020_1km_Aggregated.tif`. The hosting
//! Apache instance advertises `Accept-Ranges: bytes` but does **not**
//! actually honour `Range:` requests — every request returns
//! `HTTP/1.1 200 OK` with the full `Content-Length: 869715253` body
//! instead of a `206 Partial Content`. Verified live with `curl -v`
//! against multiple sub-ranges; the server consistently sends the
//! whole file. That makes the standard `emem_fetch::cog::open_profile`
//! path unusable here — a single per-cell recall would have to download
//! ~830 MB before extracting one pixel.
//!
//! GHSL (R2023A) does honour ranges but ships its tiles as `.zip`
//! archives containing the GeoTIFF, not raw COGs, so the COG sampler
//! cannot read them either without a multi-stage extract step.
//!
//! WorldPop themselves expose a *synchronous* REST endpoint
//! (`runasync=false`) that returns the integrated population for an
//! arbitrary GeoJSON polygon as a small JSON document, typically in
//! 2–4 s for a 1 km² window. That matches the architectural pattern
//! the existing `materialize_soilgrids_band` uses (REST → JSON →
//! signed Fact).
//!
//! ## Resolution semantics
//!
//! The dataset behind the REST API is the same WorldPop Global 100 m
//! Constrained 2020 product (`wpgppop`) that powers the static GeoTIFF.
//! For a per-cell recall we sample a **1 km × 1 km** AOI centred on the
//! cell — large enough to fully cover the 100 m WorldPop pixels in the
//! neighbourhood (so the integration includes 100 pixels, not just the
//! single one the cell centre falls in) and small enough that the API
//! returns synchronously in a few seconds.
//!
//! The returned `total_population` is therefore "people inside the
//! 1 km² window centred on this cell", which equals "population density
//! at this cell, in persons · km⁻²". For an emem `population` band
//! that is exactly the scalar an agent needs to answer
//! "how dense is the population here?".
//!
//! ## Honest empties
//!
//! The API returns `{"data":{"total_population":0}}` for AOIs over
//! ocean / unpopulated terrain. Callers must distinguish that from an
//! upstream failure; this module surfaces a structured `EmptyAoi`
//! variant so the caller can sign an Absence fact rather than a zero.

use std::time::Duration;

use serde::Deserialize;

/// Errors specific to the WorldPop fetcher.
#[derive(Debug, thiserror::Error)]
pub enum WorldPopError {
    /// HTTP / network failure.
    #[error("transport: {0}")]
    Transport(String),
    /// Upstream returned non-2xx HTTP.
    #[error("status {status} for {url}")]
    BadStatus { status: u16, url: String },
    /// Body wasn't valid JSON / didn't match the WorldPop schema.
    #[error("malformed response: {0}")]
    Malformed(String),
    /// API returned `error: true` in the result envelope.
    #[error("worldpop api error: {0}")]
    Upstream(String),
    /// The query returned `total_population == 0`. Caller should sign an
    /// Absence fact rather than persist a synthetic zero — over ocean
    /// and unpopulated terrain this is the correct, non-Primary answer.
    #[error("empty aoi: total_population=0 for window centred on ({lat:.6},{lng:.6})")]
    EmptyAoi { lat: f64, lng: f64 },
}

/// Response envelope returned by `/v1/services/stats` (sync mode).
///
/// We deserialise only the fields we need — WorldPop adds optional
/// metadata (`taskid`, `executionTime`, `startTime`, …) on every reply
/// that we ignore here.
#[derive(Debug, Clone, Deserialize)]
struct StatsEnvelope {
    /// `false` on success, `true` when `error_message` is set.
    error: bool,
    /// Populated when `error == true`.
    #[serde(default)]
    error_message: Option<String>,
    /// Populated when `error == false` and the task finished (sync mode
    /// nearly always finishes inline for our 1 km² AOIs).
    #[serde(default)]
    data: Option<StatsData>,
}

#[derive(Debug, Clone, Deserialize)]
struct StatsData {
    /// Integrated population over the AOI (persons). May be 0 for ocean
    /// / unpopulated cells.
    total_population: f64,
}

/// Half-edge in degrees of latitude that yields a ~1000 m window: at
/// any latitude, 1° of latitude is ~111.32 km, so 500 m → ~0.00449°.
/// Using a fixed value keeps the AOI deterministic across the API
/// (WorldPop reprojects internally), and the small absolute error from
/// ignoring meridional convergence is dominated by the underlying
/// 100 m raster pixel edge.
const HALF_EDGE_LAT_DEG: f64 = 500.0 / 111_320.0;

/// One-km half-edge in longitude scaled by `cos(lat)` so the AOI stays
/// ~1 km wide regardless of latitude. Caller passes `lat` in degrees.
fn half_edge_lng_deg(lat_deg: f64) -> f64 {
    let cos_lat = lat_deg.to_radians().cos().abs().max(0.05);
    500.0 / (111_320.0 * cos_lat)
}

/// Build the GeoJSON-encoded AOI polygon (closed ring, lon-lat order)
/// for a 1 km² window centred on `(lat, lng)`.
///
/// Public so the materialiser layer can quote it on the signed Fact's
/// derivation arguments — receipt verification re-runs the formatting
/// against the cell centre and confirms the polygon matches.
pub fn aoi_polygon_geojson(lat: f64, lng: f64) -> String {
    let dlat = HALF_EDGE_LAT_DEG;
    let dlng = half_edge_lng_deg(lat);
    // GeoJSON polygon coordinates are [[lng,lat],...] with the ring
    // closed (first == last). The order below traces the bbox CCW,
    // which is the GeoJSON spec's outer-ring convention.
    format!(
        "{{\"type\":\"Polygon\",\"coordinates\":[[[{w:.7},{s:.7}],[{e:.7},{s:.7}],[{e:.7},{n:.7}],[{w:.7},{n:.7}],[{w:.7},{s:.7}]]]}}",
        w = lng - dlng,
        e = lng + dlng,
        s = lat - dlat,
        n = lat + dlat,
    )
}

/// Resolved upstream URL for the recall (with the AOI inlined as the
/// `geojson` query parameter). Returned alongside the value so the
/// caller can persist it as the Fact's `Source.id` / receipt evidence.
#[derive(Debug, Clone)]
pub struct WorldPopSample {
    /// Persons per square kilometre at this cell (== integrated
    /// population over the 1 km² AOI window).
    pub people_per_km2: f64,
    /// Fully-resolved REST URL the responder hit. Surfaced on the
    /// signed Fact so a verifier can re-issue the same query.
    pub upstream_url: String,
    /// WorldPop dataset key (always `wpgppop` for this 100 m product).
    pub dataset: &'static str,
    /// Vintage year the API resolved against.
    pub year: u16,
}

/// Range of WorldPop vintages the public REST API exposes for the
/// `wpgppop` dataset. The product itself is built per-country and
/// covers 2000–2020; we materialise against the most recent year by
/// default, but an operator could backfill across the full range by
/// calling [`fetch_population_density_for_year`] directly.
pub const WORLDPOP_YEARS: std::ops::RangeInclusive<u16> = 2000..=2020;
/// Default vintage to use for the `population` band when the caller
/// does not specify one. 2020 is the latest year the WorldPop Stats
/// API serves for the `wpgppop` (100 m global per-country) product.
pub const WORLDPOP_DEFAULT_YEAR: u16 = 2020;

/// Fetch persons-per-square-kilometre for a single cell using the
/// default vintage ([`WORLDPOP_DEFAULT_YEAR`]).
///
/// This is the entry point a band materialiser calls. The heavy lift
/// (URL formatting, REST round-trip, JSON parsing, error mapping) is
/// shared with the per-year variant.
pub async fn fetch_population_density(
    client: &reqwest::Client,
    lat: f64,
    lng: f64,
) -> Result<WorldPopSample, WorldPopError> {
    fetch_population_density_for_year(client, lat, lng, WORLDPOP_DEFAULT_YEAR).await
}

/// Fetch persons-per-square-kilometre for one (lat, lng) at an
/// explicit WorldPop vintage. Returns the integrated population over a
/// 1 km² window (the AOI built by [`aoi_polygon_geojson`]) — equivalent
/// to people · km⁻² because the AOI is exactly 1 km².
pub async fn fetch_population_density_for_year(
    client: &reqwest::Client,
    lat: f64,
    lng: f64,
    year: u16,
) -> Result<WorldPopSample, WorldPopError> {
    if !WORLDPOP_YEARS.contains(&year) {
        return Err(WorldPopError::Upstream(format!(
            "year {year} outside WorldPop wpgppop range {}..={}",
            WORLDPOP_YEARS.start(),
            WORLDPOP_YEARS.end()
        )));
    }
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lng) {
        return Err(WorldPopError::Upstream(format!(
            "invalid lat/lng: ({lat},{lng})"
        )));
    }
    let geojson = aoi_polygon_geojson(lat, lng);
    let url = format!(
        "https://api.worldpop.org/v1/services/stats?dataset=wpgppop&year={year}&runasync=false&geojson={geo_enc}",
        geo_enc = url_encode(&geojson),
    );

    let resp = client
        .get(&url)
        .header(
            reqwest::header::USER_AGENT,
            concat!(
                "emem.dev/",
                env!("CARGO_PKG_VERSION"),
                " (avijeet@vortx.ai)"
            ),
        )
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| WorldPopError::Transport(e.to_string()))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(WorldPopError::BadStatus {
            status: status.as_u16(),
            url: url.clone(),
        });
    }
    let body = resp
        .text()
        .await
        .map_err(|e| WorldPopError::Transport(e.to_string()))?;
    let env: StatsEnvelope = serde_json::from_str(&body)
        .map_err(|e| WorldPopError::Malformed(format!("{e} (body: {body})")))?;
    if env.error {
        return Err(WorldPopError::Upstream(
            env.error_message
                .unwrap_or_else(|| "unspecified worldpop error".into()),
        ));
    }
    let data = env.data.ok_or_else(|| {
        WorldPopError::Malformed(
            "worldpop response missing data field (sync mode timed out?)".into(),
        )
    })?;
    let value = data.total_population;
    if !value.is_finite() {
        return Err(WorldPopError::Malformed(format!(
            "worldpop returned non-finite total_population: {value}"
        )));
    }
    if value <= 0.0 {
        // 0 (or, defensively, a negative noise value rounded toward
        // zero) signals an unpopulated AOI — over ocean, polar
        // interior, or genuinely empty terrain. Surface as a typed
        // empty so the caller signs an Absence rather than a Primary
        // zero (which would be indistinguishable from "we've never
        // measured here" downstream).
        return Err(WorldPopError::EmptyAoi { lat, lng });
    }
    Ok(WorldPopSample {
        people_per_km2: value,
        upstream_url: url,
        dataset: "wpgppop",
        year,
    })
}

/// Minimal percent-encoder for the JSON payload that goes into the
/// `geojson=` query parameter.
///
/// Encodes everything outside the unreserved + a small safe set; matches
/// the encoding the WorldPop API expects in practice (verified live).
///
/// Kept inline rather than pulling in `urlencoding` as a new dep — the
/// fetch crate's policy is to add deps only when there is no small,
/// auditable alternative.
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            // Unreserved per RFC 3986 §2.3 + GeoJSON's structural
            // characters — keep alphanumerics + `.,-~_` literal so the
            // URL stays human-greppable in logs.
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

    /// AOI polygon must be a closed CCW ring of the right size at the
    /// equator. At lat=0 the lng half-edge is `500/111320` (no `cos`
    /// scaling), giving a near-square ~1 km² window.
    #[test]
    fn aoi_polygon_at_equator_is_one_km_square() {
        let geo = aoi_polygon_geojson(0.0, 0.0);
        let half = HALF_EDGE_LAT_DEG;
        // The polygon string contains the four bbox corners in CCW
        // order. We don't pull in serde_json here — just substring-check
        // each corner appears with the expected sign + magnitude.
        let neg = format!("{:.7}", -half);
        let pos = format!("{:.7}", half);
        assert!(geo.contains(&neg), "missing -half: {geo}");
        assert!(geo.contains(&pos), "missing +half: {geo}");
        // Closed ring: first coordinate pair must equal the last.
        let first = format!("[{neg},{neg}]");
        let occurrences = geo.matches(&first).count();
        assert_eq!(occurrences, 2, "ring not closed: {geo}");
    }

    /// At ±60° latitude the longitude half-edge must be ~2× the
    /// equatorial value (cos 60° = 0.5). This is the deterministic
    /// math the receipt verifier re-runs.
    #[test]
    fn aoi_polygon_compensates_for_latitude_convergence() {
        let eq = half_edge_lng_deg(0.0);
        let high = half_edge_lng_deg(60.0);
        // Allow some slack for the cos floor (`max(0.05)`); at lat=60
        // cos = 0.5 exactly, so high should be ~2× eq.
        let ratio = high / eq;
        assert!(
            (1.95..=2.05).contains(&ratio),
            "lat-60 vs eq half-edge ratio {ratio} not ~2.0"
        );
    }

    /// At the poles the cosine floor caps the lng half-edge so the
    /// polygon stays bounded (no division-by-zero, no NaN).
    #[test]
    fn aoi_polygon_bounded_at_poles() {
        let geo = aoi_polygon_geojson(89.999_9, 0.0);
        assert!(!geo.contains("NaN"));
        assert!(!geo.contains("inf"));
        // Substring check: there are at least 5 coordinate pairs ([)
        // — the polygon ring still closes.
        assert!(geo.matches('[').count() >= 5);
    }

    /// `url_encode` must percent-escape JSON structural characters and
    /// preserve unreserved ASCII verbatim. Regression test for the
    /// `geojson=` query parameter — if commas/braces leak through
    /// unencoded the WorldPop API rejects the request as malformed.
    #[test]
    fn url_encode_escapes_json_structurals() {
        let raw = r#"{"type":"Point","coordinates":[1.5,-2.5]}"#;
        let enc = url_encode(raw);
        assert!(!enc.contains('{'));
        assert!(!enc.contains('}'));
        assert!(!enc.contains('['));
        assert!(!enc.contains(']'));
        assert!(!enc.contains('"'));
        assert!(!enc.contains(','));
        // Unreserved chars survive as-is.
        assert!(enc.contains("type"));
        assert!(enc.contains("Point"));
        // Sign / digits survive.
        assert!(enc.contains("1.5"));
        assert!(enc.contains("-2.5"));
    }

    /// Round-trip the URL builder with a known cell centre and confirm
    /// the formatted polygon round-trips through the response decoder
    /// shape — purely deterministic, no network.
    #[test]
    fn aoi_polygon_geojson_format_is_stable() {
        // Manhattan (40.7579554, -73.9855319), ~10m cell centre.
        let geo = aoi_polygon_geojson(40.757_955_4, -73.985_531_9);
        // Closed ring: 5 lon-lat pairs separated by `],[`.
        let pairs = geo.matches("],[").count();
        assert_eq!(
            pairs, 4,
            "expected 5 vertices (4 separators), got {pairs}: {geo}"
        );
        // Header + footer present.
        assert!(geo.starts_with(r#"{"type":"Polygon","coordinates":[[["#));
        assert!(geo.ends_with("]]]}"));
    }

    /// Network-gated end-to-end smoke test. Disabled by default
    /// (`#[ignore]`) so CI stays offline-clean; run explicitly with
    /// `cargo test -p emem-fetch -- --ignored worldpop_live`.
    #[tokio::test]
    #[ignore]
    async fn worldpop_live_returns_density_for_manhattan() {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(45))
            .build()
            .unwrap();
        let sample = fetch_population_density(&client, 40.7579554, -73.9855319)
            .await
            .expect("worldpop live fetch failed");
        // Manhattan is ~28k people/km²; a 1 km AOI centred on
        // Times Square should land somewhere in the 5k–60k range
        // even after the API's polygon trimming.
        assert!(
            sample.people_per_km2 > 1_000.0,
            "Manhattan density too low: {}",
            sample.people_per_km2
        );
        assert!(sample.upstream_url.starts_with("https://api.worldpop.org/"));
        assert_eq!(sample.dataset, "wpgppop");
        assert_eq!(sample.year, WORLDPOP_DEFAULT_YEAR);
    }

    /// Network-gated: confirm the fetcher distinguishes ocean (empty
    /// AOI) from a populated cell, returning `EmptyAoi` rather than
    /// silently surfacing a zero.
    #[tokio::test]
    #[ignore]
    async fn worldpop_live_returns_empty_for_open_ocean() {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(45))
            .build()
            .unwrap();
        // Mid-Pacific gyre: 30°N 150°W — open ocean, far from any
        // shipping density; expected total_population == 0.
        let res = fetch_population_density(&client, 30.0, -150.0).await;
        match res {
            Err(WorldPopError::EmptyAoi { .. }) => {}
            other => panic!("expected EmptyAoi over open ocean, got {other:?}"),
        }
    }
}
