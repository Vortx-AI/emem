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
//! ## Why tiles, not the single global COG
//!
//! The JRC publishes GFC2020 V3 in three forms under
//! `…/GFC2020/LATEST/`: `single/` (per-region uncompressed), `single-cog/`
//! (one **41 GB** global Cloud-Optimized BigTIFF), and `tiles/`
//! (10°×10° COG tiles, ~280 MB each). **We read the tiles.**
//!
//! The single 41 GB global COG is a trap for a per-cell sampler: at 10 m
//! resolution the global raster carries tens of millions of internal
//! tiles, so its TileOffsets/TileByteCounts arrays (8-byte LONG8 in
//! BigTIFF) run to **hundreds of MB**. Before a sampler can read a single
//! pixel it must download that entire tile index — a multi-hundred-MB
//! cold read at the JEODPP host's ~7 MB/s. That read routinely exceeds
//! the per-band materializer timeout, so the [`crate::cog`] PROFILE_CACHE
//! `OnceCell` never initialises, never caches, and **every** request re-pays
//! the cold cost: a permanent wedge that took `/v1/eudr_dds` down for
//! hours on 2026-06-02.
//!
//! A 10° tile, by contrast, has a tiny IFD (a few thousand internal tiles)
//! and opens in ~1 s cold — the same shape as the Hansen GFC connector,
//! which has always tiled. Tiling makes cold reads fast and uniform across
//! every geography, with no boot-time pre-warm dance and no single-point
//! IFD liability.
//!
//! ## Tile naming
//!
//! Tiles are 10°×10°, anchored to the **top-left (north-west) corner** and
//! named `JRC_GFC2020_V3_<lat><LAT>_<lon><LON>.tif`, where the tiling
//! starts at 80°N / 180°W (`N80`, `W180`). Unlike Hansen (suffix-letter,
//! zero-padded: `00N_070W`), the JRC scheme is **prefix-letter, no
//! padding**: `N0_E0`, `N10_W10`, `N20_E30`, `S10_E20`, `N80_W180`. A cell
//! at lat=5.69, lng=-6.70 (Côte d'Ivoire cocoa) lives in tile `N10_W10`
//! (top edge 10°N, west edge 10°W).
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
//! - The JRC ships no tile for fully-oceanic 10° cells and none beyond
//!   the ±82° latitude envelope; both surface as
//!   [`JrcGfc2020Error::TileNotFound`] / [`JrcGfc2020Error::CoverageGap`]
//!   so the materializer can sign an `Absence` rather than a fabricated
//!   value.
//! - Network / decode errors propagate as `Transport` / `Decode`.

use reqwest::Client;

use crate::cog::CogError;

/// Base directory for the 10°×10° GFC2020 V3 COG tiles on the JRC's
/// JEODPP public open-data bucket. Range-readable (HTTP 206 verified),
/// no auth, no signed URL, no proxy required. Each tile is ~280 MB with
/// a small IFD that opens in ~1 s cold — see the module docs for why we
/// tile instead of reading the 41 GB single-COG.
const JRC_GFC2020_TILES_BASE_URL: &str =
    "https://jeodpp.jrc.ec.europa.eu/ftp/jrc-opendata/FOREST/GFC2020/LATEST/tiles";

/// The single 41 GB global COG. **Not** used on any fetch path — kept
/// only for provenance/diagnostics and the ignored live BigTIFF smoke
/// test in [`crate::cog`]. See the module docs for why per-cell sampling
/// must never read this file.
const JRC_GFC2020_SINGLE_COG_URL: &str =
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
    /// The 10° tile that would contain the cell is not published on the
    /// JEODPP bucket (404). The JRC drops fully-oceanic 10° cells; such
    /// a tile is genuinely absent rather than an upstream failure.
    /// Materializers MUST sign this as an `Absence`.
    #[error(
        "tile_not_found: GFC2020 V3 tile {tile} at {url} returned 404 (cell outside published tile coverage)"
    )]
    TileNotFound {
        /// Tile name component (e.g. "N10_W10").
        tile: String,
        /// Full upstream URL we attempted.
        url: String,
    },
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
    /// variant. HTTP 404 from the COG sampler indicates a missing tile
    /// (`TileNotFound` → Absence); other transport errors surface as
    /// `Transport` (the dispatcher may retry); everything else (decode,
    /// codec, layout) becomes `Decode`.
    fn from_cog(e: CogError, tile: String, url: String) -> Self {
        let lower = e.to_string().to_lowercase();
        // CogError::Transport stringifies HTTP status codes; "status 404"
        // is the canonical signature emitted by `cog::http_range`.
        if lower.contains("status 404") || lower.contains("not found") {
            return JrcGfc2020Error::TileNotFound { tile, url };
        }
        match e {
            CogError::Transport(s) => JrcGfc2020Error::Transport(s),
            other => JrcGfc2020Error::Decode(other.to_string()),
        }
    }
}

/// Return the legacy single global COG URL. **Not** a fetch path — kept
/// for provenance/diagnostics only. Per-cell and windowed sampling use
/// [`tile_url_for`]. Pure — no I/O.
pub fn cog_url() -> &'static str {
    JRC_GFC2020_SINGLE_COG_URL
}

/// Bare tile-corner tag pair for a cell, e.g. `("N10", "W10")`.
///
/// Tiles are anchored at integer 10° multiples to the **north and west**
/// of the cell:
/// - `lat_top = ceil(lat / 10) * 10`  — the tile spans `lat_top` down to
///   `lat_top - 10`; tagged `N{lat_top}` (≥0) or `S{|lat_top|}` (<0).
/// - `lng_left = floor(lng / 10) * 10` — the tile spans `lng_left` across
///   to `lng_left + 10`; tagged `E{lng_left}` (≥0) or `W{|lng_left|}` (<0).
///
/// The JRC scheme carries **no zero padding** (e.g. `W10`, `E0`, `W180`),
/// unlike the Hansen connector's fixed-width tags.
pub fn tile_corner_tags(lat: f64, lng: f64) -> (String, String) {
    let lat_top = (lat / 10.0).ceil() as i32 * 10;
    let lng_left = (lng / 10.0).floor() as i32 * 10;
    let lat_tag = if lat_top >= 0 {
        format!("N{lat_top}")
    } else {
        format!("S{}", lat_top.unsigned_abs())
    };
    let lng_tag = if lng_left >= 0 {
        format!("E{lng_left}")
    } else {
        format!("W{}", lng_left.unsigned_abs())
    };
    (lat_tag, lng_tag)
}

/// Filename of the 10° tile covering `(lat, lng)`:
/// `JRC_GFC2020_V3_<lat>_<lon>.tif`. Pure — no I/O — so it's the
/// load-bearing helper for unit-testing the naming convention.
pub fn tile_name_for(lat: f64, lng: f64) -> String {
    let (lat_tag, lng_tag) = tile_corner_tags(lat, lng);
    format!("JRC_GFC2020_V3_{lat_tag}_{lng_tag}.tif")
}

/// Full upstream URL of the 10° tile covering `(lat, lng)`.
pub fn tile_url_for(lat: f64, lng: f64) -> String {
    format!("{JRC_GFC2020_TILES_BASE_URL}/{}", tile_name_for(lat, lng))
}

/// Read one pixel from the GFC2020 V3 raster and return the EUDR
/// forest indicator (`1` = forest at 2020-12-31, `0` = non-forest).
///
/// Reads the 10° COG tile containing `(lat, lng)` — a small-IFD, ~1 s
/// cold open — never the 41 GB global COG.
///
/// Returns:
/// - `Ok(0)` for a confirmed non-forest pixel on 2020-12-31 (a
///   meaningful Primary fact under the EUDR definition).
/// - `Ok(1)` for a confirmed forest pixel on 2020-12-31.
/// - `Err(CoverageGap)` for cells beyond ±82° latitude.
/// - `Err(TileNotFound)` when the containing 10° tile is unpublished
///   (fully-oceanic cell) — sign an `Absence`.
/// - `Err(Transport)` for HTTP / network failures.
/// - `Err(Decode)` for COG layout / codec / range failures.
pub async fn fetch_forest_2020(client: &Client, lat: f64, lng: f64) -> Result<u8, JrcGfc2020Error> {
    if !lat.is_finite() || lat.abs() > JRC_GFC2020_LAT_BOUND {
        return Err(JrcGfc2020Error::CoverageGap { lat, lng });
    }
    if !lng.is_finite() || !(-180.0..=180.0).contains(&lng) {
        return Err(JrcGfc2020Error::CoverageGap { lat, lng });
    }

    let url = tile_url_for(lat, lng);
    let (lat_tag, lng_tag) = tile_corner_tags(lat, lng);
    let tile = format!("{lat_tag}_{lng_tag}");

    let profile = crate::cog::open_profile(client, &url)
        .await
        .map_err(|e| JrcGfc2020Error::from_cog(e, tile.clone(), url.clone()))?;
    // EPSG:4326 tile — sample directly with (lng, lat) as world (x, y).
    // The shared sampler honours the geo-transform via `world_to_pixel`.
    let raw = crate::cog::sample_pixel(client, &url, &profile, lng, lat)
        .await
        .map_err(|e| JrcGfc2020Error::from_cog(e, tile, url.clone()))?;
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

    /// `tile_name_for` produces the documented
    /// `JRC_GFC2020_V3_<lat>_<lon>.tif` pattern across all four
    /// hemispheres, with **no zero padding** (the JRC convention).
    /// Reference cells:
    /// - Côte d'Ivoire cocoa (5.69°N, -6.70°W) → top-left corner 10°N,
    ///   10°W → `N10_W10` (the 2026-06-02 repro plot).
    /// - Central Amazon (-3.0°S, -60.5°W) → top-left 0°N, 70°W → `N0_W70`.
    /// - Greenwich + Equator (0.5°N, 0.5°E) → top-left 10°N, 0°E
    ///   → `N10_E0`.
    /// - Sumatra (-1.0°S, 100.0°E) → top-left 0°N, 100°E → `N0_E100`.
    /// - Tiling origin (75°N, -175°W) → top-left 80°N, 180°W → `N80_W180`.
    #[test]
    fn tile_name_for_known_cells() {
        // Côte d'Ivoire cocoa repro plot — lat 5.69 → ceil(0.569)*10=10
        // → N10; lng -6.70 → floor(-0.67)*10=-10 → W10.
        assert_eq!(
            tile_name_for(5.69, -6.70),
            "JRC_GFC2020_V3_N10_W10.tif",
            "CIV cocoa (5.69, -6.70) must map to N10_W10"
        );
        // Central Amazon at -3, -60.5 → N0_W70.
        assert_eq!(
            tile_name_for(-3.0, -60.5),
            "JRC_GFC2020_V3_N0_W70.tif",
            "Central Amazon (-3, -60.5) must map to N0_W70"
        );
        // Greenwich + small offset north of equator → N10_E0.
        assert_eq!(
            tile_name_for(0.5, 0.5),
            "JRC_GFC2020_V3_N10_E0.tif",
            "Greenwich+north must map to N10_E0"
        );
        // Sumatra at -1.0, +100.0 → N0_E100.
        assert_eq!(
            tile_name_for(-1.0, 100.0),
            "JRC_GFC2020_V3_N0_E100.tif",
            "Sumatra (-1, 100) must map to N0_E100"
        );
        // Tiling origin corner.
        assert_eq!(
            tile_name_for(75.0, -175.0),
            "JRC_GFC2020_V3_N80_W180.tif",
            "(75, -175) must map to the origin tile N80_W180"
        );
    }

    /// `tile_url_for` composes the JEODPP `tiles/` path correctly.
    #[test]
    fn tile_url_for_includes_bucket_and_tiles_dir() {
        let url = tile_url_for(5.69, -6.70);
        assert_eq!(
            url,
            "https://jeodpp.jrc.ec.europa.eu/ftp/jrc-opendata/FOREST/GFC2020/LATEST/tiles/JRC_GFC2020_V3_N10_W10.tif"
        );
    }

    /// Boundary cells snap to the tile whose NORTH edge (lat) and WEST
    /// edge (lng) they sit on — the `ceil`/`floor` contract. Documents
    /// the southern-hemisphere `S` tag and the zero-crossing to `N0`.
    #[test]
    fn tile_corner_tags_handles_boundaries_and_hemispheres() {
        // Equator/meridian: lat 0 → ceil(0)=0 → N0; lng 0 → floor(0)=0
        // → E0. (Tile spans [0°N,-10°N] × [0°E,+10°E].)
        assert_eq!(tile_corner_tags(0.0, 0.0), ("N0".into(), "E0".into()));
        // Just south of the equator stays in the N0 tile (top edge 0°).
        assert_eq!(tile_corner_tags(-5.0, 5.0), ("N0".into(), "E0".into()));
        // Deep southern hemisphere → S tag. lat -15 → ceil(-1.5)=-1*10
        // =-10 → S10; lng 25 → floor(2.5)*10=20 → E20.
        assert_eq!(tile_corner_tags(-15.0, 25.0), ("S10".into(), "E20".into()));
        // lat = -10 exactly → ceil(-1)=-1*10=-10 → S10; lng -10 → W10.
        assert_eq!(tile_corner_tags(-10.0, -10.0), ("S10".into(), "W10".into()));
    }

    /// `cog_url` still returns the legacy single-COG URL (provenance
    /// only — never a fetch path). Pinned literally to catch accidental
    /// edits that would re-route a hot path onto the 41 GB file.
    #[test]
    fn cog_url_is_single_global_cog() {
        assert_eq!(
            cog_url(),
            "https://jeodpp.jrc.ec.europa.eu/ftp/jrc-opendata/FOREST/GFC2020/LATEST/single-cog/JRC_GFC2020_V3_COG.tif",
        );
        assert_eq!(cog_url(), JRC_GFC2020_SINGLE_COG_URL);
    }

    /// `JRC_GFC2020_VERSION_TAG` pins the V3 release the tiles point at.
    #[test]
    fn version_tag_is_v3() {
        assert_eq!(JRC_GFC2020_VERSION_TAG, "v3.2026-03");
    }

    /// `fetch_forest_2020` MUST surface `CoverageGap` (not Transport,
    /// not a fabricated zero) for cells outside the documented ±82°
    /// latitude envelope. Pins the "Antarctica → Absence, not Err" rule.
    #[tokio::test]
    async fn fetch_below_82s_is_coverage_gap() {
        let client = reqwest::Client::new();
        // -85° latitude: deep Antarctic interior, outside the ±82° bound.
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

    /// Out-of-range longitudes (and NaN) surface `CoverageGap` rather
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
        let err = fetch_forest_2020(&client, f64::NAN, 0.0).await.unwrap_err();
        assert!(
            matches!(err, JrcGfc2020Error::CoverageGap { .. }),
            "NaN lat must surface CoverageGap, got {err:?}"
        );
    }

    /// `from_cog` routes a 404 to `TileNotFound` (→ Absence), a non-404
    /// transport error to `Transport` (→ retry), and any other COG error
    /// to `Decode` (no-fallback). Pins the "oceanic tile → Absence" rule.
    #[test]
    fn from_cog_routes_404_transport_and_decode() {
        let url = tile_url_for(0.0, -30.0); // mid-Atlantic, oceanic tile
        let cog_404 = CogError::Transport(format!("status 404 Not Found on {url}"));
        let err = JrcGfc2020Error::from_cog(cog_404, "N0_W30".into(), url.clone());
        match &err {
            JrcGfc2020Error::TileNotFound { tile, url: u } => {
                assert_eq!(tile, "N0_W30", "tile field must round-trip");
                assert!(u.contains("JRC_GFC2020_V3_N0_W30.tif"));
            }
            other => panic!("expected TileNotFound, got {other:?}"),
        }

        let cog_503 = CogError::Transport("status 503 Service Unavailable".into());
        let err = JrcGfc2020Error::from_cog(cog_503, "N10_W10".into(), "https://x/y.tif".into());
        assert!(
            matches!(err, JrcGfc2020Error::Transport(_)),
            "503 must surface as Transport, not TileNotFound — got {err:?}"
        );

        let cog_decode = CogError::BadMagic(0xdeadbeef);
        let err = JrcGfc2020Error::from_cog(cog_decode, "N10_W10".into(), "https://x/y.tif".into());
        assert!(
            matches!(err, JrcGfc2020Error::Decode(_)),
            "BadMagic must surface as Decode, got {err:?}"
        );
    }

    /// End-to-end materializer round-trip against the live 10° JRC
    /// GFC2020 V3 COG tiles. Gated behind `#[ignore]` so CI doesn't hit
    /// the JEODPP bucket. Verifies the tiled path returns a real
    /// EUDR-baseline forest indicator at known sample points.
    ///
    /// - Yasuni rainforest, Ecuador (-1.15°N, -76.45°E) → tile N0_W80,
    ///   expected `1` (forest under EUDR ≥10% canopy, ≥5 m, ≥0.5 ha).
    /// - Côte d'Ivoire cocoa (5.69°N, -6.70°W) → tile N10_W10, the
    ///   2026-06-02 repro plot — expected to return a 0/1 indicator fast.
    ///
    /// On transport failure the test skips silently rather than failing
    /// the suite — it's a smoke check on the tiled path, not a JEODPP
    /// availability gate.
    #[tokio::test]
    #[ignore = "live network test against jeodpp.jrc.ec.europa.eu — run with --ignored"]
    async fn fetch_forest_2020_tiled_smoke() {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .expect("client");
        let v = match fetch_forest_2020(&client, -1.15, -76.45).await {
            Ok(v) => v,
            Err(JrcGfc2020Error::Transport(s)) => {
                eprintln!("[skip] yasuni live test: transport: {s}");
                return;
            }
            Err(other) => panic!("yasuni unexpectedly errored: {other:?}"),
        };
        assert_eq!(v, 1, "Yasuni rainforest (-1.15, -76.45) must read forest=1");

        // CIV cocoa repro plot — must return a valid 0/1 indicator.
        match fetch_forest_2020(&client, 5.69, -6.70).await {
            Ok(v) => assert!(v <= 1, "CIV indicator must be 0 or 1, got {v}"),
            Err(JrcGfc2020Error::Transport(s)) => {
                eprintln!("[skip] civ live test: transport: {s}");
            }
            Err(other) => panic!("CIV unexpectedly errored: {other:?}"),
        }
    }
}
