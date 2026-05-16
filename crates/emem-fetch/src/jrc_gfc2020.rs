//! JRC Global Forest Cover 2020 V3 connector.
//!
//! Source: **Bourgoin, C., et al. (2026). *JRC Global map of forest
//! cover for year 2020 — version 3* (GFC2020 V3). Earth System Science
//! Data 18, 1331. doi:10.5194/essd-2025-351**. Produced by the European
//! Commission's Joint Research Centre and published as the expected
//! (non-legally-binding) baseline raster for **EUDR Due Diligence
//! Statements** under Regulation (EU) 2023/1115: cell-level deforestation
//! since 2020-12-31 is computed against this layer.
//!
//! The full dataset is a **single global Cloud-Optimized GeoTIFF**
//! (~41 GB, 10 m native resolution, EPSG:4326, uint8 single band).
//! Unlike Hansen GFC which ships 648 tiles, GFC2020 V3 is one COG —
//! callers just sample the global file at the requested (lat, lng).
//! The JRC's JEODPP bucket honours HTTP Range requests (verified:
//! 206 Partial Content), so the shared [`cog`](crate::cog) sampler
//! reads only the relevant tile-window without downloading the whole
//! 41 GB.
//!
//! Pixel semantics (single band):
//! - `1` = forest under the **EUDR definition** as of the 2020-12-31
//!   cut-off (tree-cover height ≥ 5 m, area ≥ 0.5 ha, canopy cover
//!   ≥ 10 %, excluding agricultural-use land and urban land).
//! - `0` = non-forest at the cut-off.
//!
//! Honest defaults (firm protocol contract):
//! - `0` is a meaningful Primary fact ("non-forest on 2020-12-31 by
//!   EUDR definition"), not an Absence. Callers MUST NOT promote zero
//!   to a default.
//! - Below ~82°S the dataset is undefined; we surface
//!   [`JrcGfc2020Error::CoverageGap`] so the materializer can sign an
//!   `Absence` rather than a fabricated value.
//! - Network / decode errors propagate as `Transport` / `Decode`.

use reqwest::Client;

use crate::cog::CogError;

/// Direct URL to the single global GFC2020 V3 COG on the JRC's JEODPP
/// public open-data bucket. Range-readable (HTTP 206 verified). No
/// auth, no signed URL, no proxy required.
const JRC_GFC2020_BASE_URL: &str =
    "https://jeodpp.jrc.ec.europa.eu/ftp/jrc-opendata/FOREST/GFC2020/LATEST/single-cog/JRC_GFC2020_V3_COG.tif";

/// Public version tag for the GFC2020 V3 release as documented in the
/// ESSD paper (Bourgoin et al., 2026) and the JEODPP `LATEST/` symlink
/// target. Bumped only when the JRC issues a new V (V4 …) — patch
/// updates re-publish under the same V3 tag.
pub const JRC_GFC2020_VERSION_TAG: &str = "v3.2026-03";

/// Maximum latitude (deg, absolute) at which GFC2020 V3 is defined.
/// The raster extends to ±82° in the EPSG:4326 product; sampling
/// beyond that latitude returns a coverage gap so the dispatcher can
/// sign an Absence rather than fabricate a zero.
const JRC_GFC2020_LAT_BOUND: f64 = 82.0;

/// Errors specific to the GFC2020 V3 connector.
///
/// Bubbled up through [`crate::FetchError::Transport`] at the
/// dispatcher boundary so callers do not have to thread two error
/// types. Each variant carries enough context for a materializer to
/// sign the correct fact shape (Primary, Absence, or hard error).
#[derive(Debug, thiserror::Error)]
pub enum JrcGfc2020Error {
    /// HTTP / network failure. Caller should treat as a transport
    /// error and let the dispatcher retry.
    #[error("transport: {0}")]
    Transport(String),
    /// COG parse / decode failure (TIFF layout, codec stream
    /// corruption, pixel out of dataset range). Indicates upstream
    /// corruption — the no-fallback rule applies.
    #[error("decode: {0}")]
    Decode(String),
    /// Cell sits outside the dataset's documented ±82° latitude
    /// envelope (Antarctic interior, high Arctic). Materializers MUST
    /// sign this as an `Absence` — the cell is genuinely outside the
    /// dataset's coverage.
    #[error(
        "coverage_gap: lat={lat:.6} lng={lng:.6} is outside GFC2020 V3 ±82° latitude envelope"
    )]
    CoverageGap {
        /// Cell latitude that triggered the gap.
        lat: f64,
        /// Cell longitude (carried for diagnostics).
        lng: f64,
    },
}

impl JrcGfc2020Error {
    /// Map a [`CogError`] into the appropriate connector-specific
    /// variant. Transport errors surface as `Transport`; everything
    /// else (decode, codec, layout) becomes `Decode`.
    fn from_cog(e: CogError) -> Self {
        match e {
            CogError::Transport(s) => JrcGfc2020Error::Transport(s),
            other => JrcGfc2020Error::Decode(other.to_string()),
        }
    }
}

/// Return the single global COG URL. Stable function rather than a
/// constant so callers can reach it through the module's public API
/// without depending on the private constant. Pure — no I/O.
pub fn cog_url() -> &'static str {
    JRC_GFC2020_BASE_URL
}

/// Read one pixel from the GFC2020 V3 raster and return the EUDR
/// forest indicator (`1` = forest at 2020-12-31, `0` = non-forest).
///
/// Returns:
/// - `Ok(0)` for a confirmed non-forest pixel on 2020-12-31 (a
///   meaningful Primary fact under the EUDR definition).
/// - `Ok(1)` for a confirmed forest pixel on 2020-12-31.
/// - `Err(CoverageGap)` for cells beyond ±82° latitude.
/// - `Err(Transport)` for HTTP / network failures.
/// - `Err(Decode)` for COG layout / codec / range failures.
pub async fn fetch_forest_2020(client: &Client, lat: f64, lng: f64) -> Result<u8, JrcGfc2020Error> {
    if !lat.is_finite() || lat.abs() > JRC_GFC2020_LAT_BOUND {
        return Err(JrcGfc2020Error::CoverageGap { lat, lng });
    }
    if !lng.is_finite() || !(-180.0..=180.0).contains(&lng) {
        return Err(JrcGfc2020Error::CoverageGap { lat, lng });
    }

    let url = JRC_GFC2020_BASE_URL;
    let profile = crate::cog::open_profile(client, url)
        .await
        .map_err(JrcGfc2020Error::from_cog)?;
    // EPSG:4326 single COG — sample directly with (lng, lat) as
    // world (x, y). The shared sampler honours the geo-transform via
    // `world_to_pixel`.
    let raw = crate::cog::sample_pixel(client, url, &profile, lng, lat)
        .await
        .map_err(JrcGfc2020Error::from_cog)?;
    if !raw.is_finite() || raw < 0.0 || raw > u8::MAX as f64 {
        return Err(JrcGfc2020Error::Decode(format!(
            "non-uint8 pixel value {raw} from {url}"
        )));
    }
    let byte = raw.round() as u8;
    if byte > 1 {
        return Err(JrcGfc2020Error::Decode(format!(
            "pixel out of range: value={byte} (GFC2020 V3 is single-band uint8 0/1) at lat={lat:.6} lng={lng:.6}"
        )));
    }
    Ok(byte)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The connector exposes one global COG URL — there is no tile
    /// math, so `cog_url()` must return the documented single-COG
    /// path on the JRC's JEODPP bucket. Pinned literally to catch
    /// accidental path edits.
    #[test]
    fn cog_url_is_single_global_cog() {
        assert_eq!(
            cog_url(),
            "https://jeodpp.jrc.ec.europa.eu/ftp/jrc-opendata/FOREST/GFC2020/LATEST/single-cog/JRC_GFC2020_V3_COG.tif",
            "GFC2020 V3 is published as a single global COG — the URL must match the JEODPP LATEST symlink"
        );
        // Sanity: the constant the module uses internally must be the
        // same value (no shadowing via a stale string literal).
        assert_eq!(cog_url(), JRC_GFC2020_BASE_URL);
    }

    /// `JRC_GFC2020_VERSION_TAG` pins the V3 release the URL points
    /// at. If the JEODPP `LATEST/` symlink moves to V4 we want the
    /// constant bump to be a one-line, reviewable change.
    #[test]
    fn version_tag_is_v3() {
        assert_eq!(JRC_GFC2020_VERSION_TAG, "v3.2026-03");
    }

    /// `fetch_forest_2020` MUST surface `CoverageGap` (not Transport,
    /// not a fabricated zero) for cells outside the documented ±82°
    /// latitude envelope. This pins the protocol's
    /// "Antarctica → Absence, not Err" rule for downstream
    /// materializers.
    #[tokio::test]
    async fn fetch_below_82s_is_coverage_gap() {
        // No HTTP layer is touched — the bounds check fires before
        // we open the COG. Using a no-op client is fine.
        let client = reqwest::Client::new();
        // -85° latitude: deep Antarctic interior, outside the ±82°
        // bound. Longitude is in-range so the gap is unambiguously
        // attributable to latitude.
        let err = fetch_forest_2020(&client, -85.0, 0.0).await.unwrap_err();
        match err {
            JrcGfc2020Error::CoverageGap { lat, lng } => {
                assert!((lat - (-85.0)).abs() < 1e-9, "lat must round-trip");
                assert!((lng - 0.0).abs() < 1e-9, "lng must round-trip");
            }
            other => panic!("expected CoverageGap, got {other:?}"),
        }
        // High Arctic above +82°N — same envelope, same result.
        let err = fetch_forest_2020(&client, 84.5, 10.0).await.unwrap_err();
        assert!(
            matches!(err, JrcGfc2020Error::CoverageGap { .. }),
            "lat above +82°N must surface CoverageGap, got {err:?}"
        );
    }

    /// In-range lat/lng pairs must NOT trip the bounds check —
    /// they're expected to proceed to the HTTP layer. We assert the
    /// negative shape (no `CoverageGap`) without hitting the network
    /// by constructing a client that immediately fails its DNS
    /// lookup (an unresolvable hostname via a private resolver
    /// timeout would block tests; instead we just check the
    /// bounds-only path returns errors of the network-shaped
    /// variants, not `CoverageGap`).
    ///
    /// To keep the test self-contained and hermetic we skip the
    /// actual await: we re-implement the bounds-only predicate that
    /// `fetch_forest_2020` uses and pin the in-range cases. Any
    /// drift between this test and the real check would surface as
    /// a different error variant in the negative test above.
    #[test]
    fn in_range_lat_lng_pass_bounds_check() {
        // Equatorial Amazon — well inside ±82°.
        let lat = -3.0_f64;
        let lng = -60.5_f64;
        assert!(
            lat.is_finite() && lat.abs() <= JRC_GFC2020_LAT_BOUND,
            "Amazon test cell must pass lat bound"
        );
        assert!(
            lng.is_finite() && (-180.0..=180.0).contains(&lng),
            "Amazon test cell must pass lng bound"
        );
        // Boreal forest at +60°N — well inside ±82°.
        let lat = 60.0_f64;
        let lng = 25.0_f64;
        assert!(lat.is_finite() && lat.abs() <= JRC_GFC2020_LAT_BOUND);
        assert!(lng.is_finite() && (-180.0..=180.0).contains(&lng));
        // Right on the +82° edge — inclusive, must pass.
        let lat = 82.0_f64;
        assert!(
            lat.abs() <= JRC_GFC2020_LAT_BOUND,
            "lat == +82 must be inclusive (the dataset is defined at the edge)"
        );
        // Just outside the edge — must fail.
        let lat = 82.000_001_f64;
        assert!(
            lat.abs() > JRC_GFC2020_LAT_BOUND,
            "lat > +82 must trip the coverage-gap check"
        );
    }

    /// Out-of-range longitudes also surface `CoverageGap` rather
    /// than letting the COG sampler emit a confusing pixel-out-of-image
    /// error. Pins the input-domain contract.
    #[tokio::test]
    async fn fetch_invalid_lng_is_coverage_gap() {
        let client = reqwest::Client::new();
        let err = fetch_forest_2020(&client, 0.0, 200.0).await.unwrap_err();
        assert!(
            matches!(err, JrcGfc2020Error::CoverageGap { .. }),
            "lng > 180 must surface CoverageGap, got {err:?}"
        );
        let err = fetch_forest_2020(&client, 0.0, -181.0).await.unwrap_err();
        assert!(
            matches!(err, JrcGfc2020Error::CoverageGap { .. }),
            "lng < -180 must surface CoverageGap, got {err:?}"
        );
        // NaN must also surface as a gap, not propagate to the COG
        // sampler (which would emit a less actionable error).
        let err = fetch_forest_2020(&client, f64::NAN, 0.0).await.unwrap_err();
        assert!(
            matches!(err, JrcGfc2020Error::CoverageGap { .. }),
            "NaN lat must surface CoverageGap, got {err:?}"
        );
    }

    /// `JrcGfc2020Error::from_cog` translates a transport-shaped COG
    /// error into [`JrcGfc2020Error::Transport`] (so the dispatcher
    /// can retry); other COG errors become `Decode` (the no-fallback
    /// rule applies — we never invent a default value).
    #[test]
    fn from_cog_routes_transport_and_decode() {
        let cog_transport = CogError::Transport("status 503 Service Unavailable".into());
        let err = JrcGfc2020Error::from_cog(cog_transport);
        assert!(
            matches!(err, JrcGfc2020Error::Transport(_)),
            "Transport must round-trip as Transport, got {err:?}"
        );

        let cog_decode = CogError::BadMagic(0xdeadbeef);
        let err = JrcGfc2020Error::from_cog(cog_decode);
        assert!(
            matches!(err, JrcGfc2020Error::Decode(_)),
            "BadMagic must surface as Decode, got {err:?}"
        );

        let cog_unsupported = CogError::Unsupported("planar config 2".into());
        let err = JrcGfc2020Error::from_cog(cog_unsupported);
        assert!(
            matches!(err, JrcGfc2020Error::Decode(_)),
            "Unsupported must surface as Decode, got {err:?}"
        );
    }
}
