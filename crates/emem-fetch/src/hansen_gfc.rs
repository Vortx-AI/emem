//! Hansen Global Forest Change connector.
//!
//! Source: **Hansen, M. C., P. V. Potapov, R. Moore, M. Hancher, S. A.
//! Turubanova, A. Tyukavina, D. Thau, S. V. Stehman, S. J. Goetz, T. R.
//! Loveland, A. Kommareddy, A. Egorov, L. Chini, C. O. Justice, J. R. G.
//! Townshend (2013). *High-Resolution Global Maps of 21st-Century Forest
//! Cover Change*. Science 342, 850-853. doi:10.1126/science.1244693** —
//! version v1.12 (the **2000-2024** annual update, released 2025-05).
//! Hosted on Google Earth Engine's public GCS bucket
//! `earthenginepartners-hansen` at no cost and with no auth.
//!
//! Three sub-bands of the `forest_change` family wire here:
//! - `forest_change.lossyear`     (uint8, 0=no loss, 1..=24 = 2001..=2024)
//! - `forest_change.treecover2000` (uint8, 0..=100 % canopy cover at 30 m)
//! - `forest_change.gain`          (uint8 0/1, dataset-frozen 2000-2012 mask)
//!
//! Tiles are 10° × 10°, each 40 000 × 40 000 px (0.00025° / ~30 m at the
//! equator). Naming convention is the **top-left corner** of the tile:
//! - lat (north of equator → "N", south → "S"), padded to width 3
//!   including the suffix character — e.g. "00N", "10S", "40N".
//! - lon (east of meridian → "E", west → "W"), padded to width 4
//!   including the suffix character — e.g. "000E", "010W", "120W".
//!
//! Tiles are anchored to integer 10° multiples; a cell at lat=-3.0,
//! lng=-60.5 lives in tile `00N_070W` (top edge at 0°N, west edge at
//! 70°W). This convention is documented authoritatively in the
//! download.html JavaScript at
//! `https://storage.googleapis.com/earthenginepartners-hansen/GFC-2024-v1.12/download.html`
//! (`set_paths(x, y)` builds the URL list given a 10°-spaced corner).
//!
//! The TIFFs themselves are stripped (one strip per row, 40 000 strips
//! per file), LZW-compressed, single-band uint8. We use the shared
//! [`cog`](crate::cog) sampler — it already synthesises tile-shape
//! geometry from strip tags and supports LZW + predictor=1.
//!
//! Honest defaults (firm protocol contract):
//! - lossyear=0 means "this on-land pixel had no canopy loss observed
//!   2001–2024" — a meaningful Primary fact, not an Absence.
//! - When the upstream tile does not exist (Antarctica below 60°S; the
//!   dataset is bounded ±60° to ~80°N) we surface
//!   [`HansenGfcError::TileNotFound`] so the materializer can sign an
//!   `Absence`.
//! - Network / decode errors propagate as `Transport` / `Decode`. The
//!   no-fallback rule applies — we never invent a default value.

use reqwest::Client;

use crate::cog::CogError;

/// GCS bucket / version path. The v1.12 release is the 2025-05-issued
/// annual update covering loss through calendar year 2024.
const GFC_BASE_URL: &str =
    "https://storage.googleapis.com/earthenginepartners-hansen/GFC-2024-v1.12";

/// Filename prefix used inside the bucket — matches the version path.
const GFC_FILENAME_PREFIX: &str = "Hansen_GFC-2024-v1.12";

/// First calendar year encoded by `lossyear=1`. The full mapping is
/// `lossyear=k → 2000 + k`, so k=1→2001 and k=24→2024 in v1.12.
pub const HANSEN_LOSSYEAR_BASE: u16 = 2000;

/// Highest `lossyear` integer value present in the v1.12 raster
/// (calendar year 2024). Bumped to 25, 26, … as new vintages publish.
pub const HANSEN_LOSSYEAR_MAX_VALUE: u8 = 24;

/// Known layers exposed by this connector. The strings match the
/// upstream filename's `<layer>` segment.
pub const LAYER_LOSSYEAR: &str = "lossyear";
/// Layer name for the year-2000 baseline canopy cover percentage.
pub const LAYER_TREECOVER_2000: &str = "treecover2000";
/// Layer name for the 2000-2012 forest gain binary mask.
pub const LAYER_GAIN: &str = "gain";

/// Errors specific to the Hansen GFC connector.
///
/// Bubbled up through [`crate::FetchError::Transport`] at the
/// dispatcher boundary so callers do not have to thread two error
/// types. Each variant carries enough context for a materializer to
/// sign the correct fact shape (Primary, Absence, or hard error).
#[derive(Debug, thiserror::Error)]
pub enum HansenGfcError {
    /// Tile URL exists in the naming scheme but is not present on the
    /// upstream bucket. Hansen GFC does not ship tiles below 60°S
    /// (Antarctica) or above ~80°N; the bucket also drops the all-zero
    /// open-ocean tiles. Materializers MUST sign this as an `Absence`
    /// — the cell is genuinely outside the dataset's coverage.
    #[error(
        "tile_not_found: GFC v1.12 tile {tile} ({layer}) at {url} returned 404 (cell outside dataset coverage)"
    )]
    TileNotFound {
        /// Tile name component (e.g. "10S_010W").
        tile: String,
        /// Layer component (e.g. "lossyear").
        layer: String,
        /// Full upstream URL we attempted.
        url: String,
    },
    /// HTTP / network failure other than 404. Caller should treat as a
    /// transport error and let the dispatcher retry.
    #[error("transport: {0}")]
    Transport(String),
    /// COG parse / decode failure (TIFF layout, LZW stream corruption,
    /// pixel out of dataset range). Indicates upstream corruption — the
    /// no-fallback rule applies.
    #[error("decode: {0}")]
    Decode(String),
    /// Pixel value was outside the documented `[0, max_value]` range
    /// for the requested layer. Treat as upstream corruption rather
    /// than a default.
    #[error(
        "pixel_out_of_range: layer={layer} value={value} max={max} at lat={lat:.6} lng={lng:.6}"
    )]
    PixelOutOfRange {
        /// Layer the value came from.
        layer: String,
        /// Raw pixel byte read from the COG.
        value: u8,
        /// Documented maximum for this layer (100 for treecover, 24 for
        /// lossyear in v1.12, 1 for gain).
        max: u8,
        /// Cell latitude, for diagnostics.
        lat: f64,
        /// Cell longitude, for diagnostics.
        lng: f64,
    },
}

impl HansenGfcError {
    /// Map a [`CogError`] into the appropriate Hansen-specific variant.
    /// HTTP 404 from the COG sampler indicates a missing tile (covered
    /// by `TileNotFound`); everything else is `Transport` or `Decode`.
    fn from_cog(e: CogError, tile: String, layer: String, url: String) -> Self {
        let msg = e.to_string();
        // CogError::Transport stringifies HTTP status codes; "status 404"
        // is the canonical signature emitted by `cog::http_range`.
        let lower = msg.to_lowercase();
        if lower.contains("status 404") || lower.contains("not found") {
            return HansenGfcError::TileNotFound { tile, layer, url };
        }
        match e {
            CogError::Transport(s) => HansenGfcError::Transport(s),
            other => HansenGfcError::Decode(other.to_string()),
        }
    }
}

/// Compute the Hansen GFC tile filename for a (lat, lng) point and a
/// chosen `layer`. Pure function — no I/O — so this is the
/// load-bearing helper for unit-testing the naming convention.
///
/// The result is the `<filename>` portion that goes after the version
/// path: `Hansen_GFC-2024-v1.12_<layer>_<lat3>_<lon4>.tif`. Use
/// [`tile_url_for`] when you need the full https://… URL.
///
/// Tiles are anchored at integer 10° multiples to the **north and
/// west** of the cell:
/// - `lat_top = ceil(lat / 10) * 10`  — the tile spans `lat_top`
///   down to `lat_top - 10`.
/// - `lng_left = floor(lng / 10) * 10` — the tile spans `lng_left`
///   across to `lng_left + 10`.
///
/// Boundary points (e.g. lat = 0.0 exactly) snap to the tile that has
/// the boundary as its NORTH edge for latitude (consistent with
/// `ceil` for negatives going to zero) and WEST edge for longitude
/// (consistent with `floor`).
pub fn tile_name_for(lat: f64, lng: f64, layer: &str) -> String {
    let (lat_tag, lng_tag) = tile_corner_tags(lat, lng);
    format!("{GFC_FILENAME_PREFIX}_{layer}_{lat_tag}_{lng_tag}.tif")
}

/// Full upstream URL for the tile covering `(lat, lng)` for `layer`.
pub fn tile_url_for(lat: f64, lng: f64, layer: &str) -> String {
    format!("{GFC_BASE_URL}/{}", tile_name_for(lat, lng, layer))
}

/// Return the bare tile-corner tag pair, e.g. `("00N", "070W")` —
/// useful when the caller wants to log or attest the tile identity
/// without the full URL.
pub fn tile_corner_tags(lat: f64, lng: f64) -> (String, String) {
    // ceil for lat anchors the tile at its NORTH edge; floor for lng
    // anchors at its WEST edge. Both are consistent with the
    // download.html naming scheme published with the dataset.
    let lat_top = (lat / 10.0).ceil() as i32 * 10;
    let lng_left = (lng / 10.0).floor() as i32 * 10;
    let lat_tag = if lat_top >= 0 {
        // 3-char width including the N suffix: "00N", "10N", "40N"…
        format!("{:02}N", lat_top)
    } else {
        format!("{:02}S", lat_top.unsigned_abs())
    };
    let lng_tag = if lng_left >= 0 {
        // 4-char width including the E suffix: "000E", "010E", "100E"…
        format!("{:03}E", lng_left)
    } else {
        format!("{:03}W", lng_left.unsigned_abs())
    };
    (lat_tag, lng_tag)
}

/// Convert a Hansen `lossyear` byte (0..=HANSEN_LOSSYEAR_MAX_VALUE)
/// into a calendar year, or `None` for "no loss observed". Inline so
/// every caller documents the same mapping.
///
/// The mapping is **always** `value=0 → None`, `value=k → Some(2000 +
/// k)` for k in 1..=HANSEN_LOSSYEAR_MAX_VALUE. The base year (2000)
/// and max value (24 in v1.12) are dataset constants; bumping to a
/// future v1.13 with k=25 → 2025 is a one-line edit to
/// [`HANSEN_LOSSYEAR_MAX_VALUE`].
pub fn lossyear_byte_to_calendar_year(byte: u8) -> Option<u16> {
    if byte == 0 {
        None
    } else {
        Some(HANSEN_LOSSYEAR_BASE + byte as u16)
    }
}

/// Read one pixel from the Hansen `lossyear` raster and return the
/// calendar year of forest loss for the cell, or `Ok(None)` if the
/// pixel had no loss event 2001..=current.
///
/// Returns:
/// - `Ok(Some(year))` for a real loss event (2001..=2024 in v1.12).
/// - `Ok(None)` for a confirmed on-land pixel with no observed loss
///   — a meaningful Primary fact (year_of_loss=0).
/// - `Err(TileNotFound)` for cells outside the dataset's tile
///   coverage (Antarctica, polar interior, all-ocean tiles). The
///   caller signs an `Absence`.
/// - Other `Err` variants for transport / decode / range failures.
pub async fn fetch_forest_loss_year(
    client: &Client,
    lat: f64,
    lng: f64,
) -> Result<Option<u16>, HansenGfcError> {
    let byte = sample_layer_byte(client, lat, lng, LAYER_LOSSYEAR).await?;
    if byte > HANSEN_LOSSYEAR_MAX_VALUE {
        let (lat_tag, lng_tag) = tile_corner_tags(lat, lng);
        return Err(HansenGfcError::PixelOutOfRange {
            layer: format!("{LAYER_LOSSYEAR} (tile {lat_tag}_{lng_tag})"),
            value: byte,
            max: HANSEN_LOSSYEAR_MAX_VALUE,
            lat,
            lng,
        });
    }
    Ok(lossyear_byte_to_calendar_year(byte))
}

/// Read one pixel from the `treecover2000` raster — the year-2000
/// baseline canopy cover percentage (uint8, 0..=100).
///
/// Returns the raw percent (0–100). 0 means "no canopy in 2000",
/// which is a meaningful Primary fact (e.g. an established city or
/// a desert pixel); the caller does NOT promote zero to Absence.
/// The TileNotFound variant remains the only Absence-worthy result.
pub async fn fetch_treecover_2000(
    client: &Client,
    lat: f64,
    lng: f64,
) -> Result<u8, HansenGfcError> {
    let byte = sample_layer_byte(client, lat, lng, LAYER_TREECOVER_2000).await?;
    if byte > 100 {
        return Err(HansenGfcError::PixelOutOfRange {
            layer: LAYER_TREECOVER_2000.into(),
            value: byte,
            max: 100,
            lat,
            lng,
        });
    }
    Ok(byte)
}

/// Read one pixel from the `gain` raster — the 2000-2012 binary
/// forest-gain mask (uint8 0/1). Frozen at v1.0 and not updated in
/// later releases; surfaced primarily for historical comparisons.
///
/// `0` = no observed gain in the 2000-2012 window; `1` = gain
/// observed. Both are meaningful Primary facts.
pub async fn fetch_forest_gain(client: &Client, lat: f64, lng: f64) -> Result<u8, HansenGfcError> {
    let byte = sample_layer_byte(client, lat, lng, LAYER_GAIN).await?;
    if byte > 1 {
        return Err(HansenGfcError::PixelOutOfRange {
            layer: LAYER_GAIN.into(),
            value: byte,
            max: 1,
            lat,
            lng,
        });
    }
    Ok(byte)
}

/// Internal: open the COG profile, sample one pixel as f64, round to
/// u8. All three layers ship as uint8 single-band so the same path
/// works for every sub-band; per-layer range validation happens at
/// the caller.
async fn sample_layer_byte(
    client: &Client,
    lat: f64,
    lng: f64,
    layer: &str,
) -> Result<u8, HansenGfcError> {
    let url = tile_url_for(lat, lng, layer);
    let (lat_tag, lng_tag) = tile_corner_tags(lat, lng);
    let tile = format!("{lat_tag}_{lng_tag}");

    let profile = match crate::cog::open_profile(client, &url).await {
        Ok(p) => p,
        Err(e) => return Err(HansenGfcError::from_cog(e, tile, layer.into(), url)),
    };
    // EPSG:4326 — sample directly with (lng, lat) as world (x, y).
    // The Hansen rasters tag this in their GeoKeyDirectory; the
    // shared sampler honours it via `world_to_pixel`.
    let raw = match crate::cog::sample_pixel(client, &url, &profile, lng, lat).await {
        Ok(v) => v,
        Err(e) => return Err(HansenGfcError::from_cog(e, tile, layer.into(), url)),
    };
    if !raw.is_finite() || raw < 0.0 || raw > u8::MAX as f64 {
        return Err(HansenGfcError::Decode(format!(
            "non-uint8 pixel value {raw} from {url}"
        )));
    }
    Ok(raw.round() as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `tile_name_for` produces the documented `Hansen_GFC-2024-v1.12_<layer>_<lat>_<lng>.tif`
    /// pattern across Northern, Southern, Eastern, and Western
    /// hemispheres. Reference cells:
    ///
    /// - Yosemite NP, USA  (lat ≈ 37.86°N, lng ≈ -119.54°W)
    ///   → top-left corner 40°N, 120°W → `40N_120W`.
    /// - Central Amazon    (lat = -3.0°S,  lng = -60.5°W)
    ///   → top-left corner 0°N,  70°W  → `00N_070W`.
    /// - Greenwich + Equator (0.5°N, 0.5°E)
    ///   → top-left corner 10°N, 0°E   → `10N_000E`.
    /// - Sumatra, ID       (lat = -1.0°S, lng = 100.0°E)
    ///   → top-left corner 0°N, 100°E  → `00N_100E`.
    /// - Siberia, RU       (lat = 65.0°N, lng = 120.0°E)
    ///   → top-left corner 70°N, 120°E → `70N_120E`.
    #[test]
    fn tile_name_for_known_cells() {
        // Yosemite NP → 40N_120W. Lat 37.86 → ceil(3.786)*10=40; lng
        // -119.54 → floor(-11.954)*10=-120 → 120W.
        assert_eq!(
            tile_name_for(37.86, -119.54, LAYER_LOSSYEAR),
            "Hansen_GFC-2024-v1.12_lossyear_40N_120W.tif",
            "Yosemite NP must map to 40N_120W"
        );
        // Central Amazon at -3, -60.5 → 00N_070W (the spec's worked
        // example: lng=-60.5 floors to -70, so the WEST edge of the
        // tile is at -70°). Confirms that boundary cases drop into
        // the tile whose west edge is on the lower-magnitude side.
        assert_eq!(
            tile_name_for(-3.0, -60.5, LAYER_LOSSYEAR),
            "Hansen_GFC-2024-v1.12_lossyear_00N_070W.tif",
            "Central Amazon (-3, -60.5) must map to 00N_070W"
        );
        // Greenwich + small offset north of equator. lat 0.5 →
        // ceil(0.05)*10=10 → 10N. lng 0.5 → floor(0.05)*10=0 → 000E.
        assert_eq!(
            tile_name_for(0.5, 0.5, LAYER_TREECOVER_2000),
            "Hansen_GFC-2024-v1.12_treecover2000_10N_000E.tif",
            "Greenwich+north must map to 10N_000E"
        );
        // Sumatra at -1.0, +100.0. lat -1 → ceil(-0.1)*10=0 → 00N.
        // lng 100 → floor(10)*10=100 → 100E.
        assert_eq!(
            tile_name_for(-1.0, 100.0, LAYER_GAIN),
            "Hansen_GFC-2024-v1.12_gain_00N_100E.tif",
            "Sumatra (-1, 100) must map to 00N_100E"
        );
        // Siberia at 65, 120 → 70N_120E.
        assert_eq!(
            tile_name_for(65.0, 120.0, LAYER_LOSSYEAR),
            "Hansen_GFC-2024-v1.12_lossyear_70N_120E.tif",
            "Siberia (65, 120) must map to 70N_120E"
        );
    }

    /// `tile_url_for` composes the bucket path correctly for every
    /// sub-band. Cross-checked against `download.html`'s
    /// `BASE_URL + FILES[i] + ...` JavaScript builder.
    #[test]
    fn tile_url_for_includes_bucket_and_version() {
        let url = tile_url_for(-3.0, -60.5, LAYER_LOSSYEAR);
        assert_eq!(
            url,
            "https://storage.googleapis.com/earthenginepartners-hansen/GFC-2024-v1.12/Hansen_GFC-2024-v1.12_lossyear_00N_070W.tif"
        );
    }

    /// `lossyear_byte_to_calendar_year` matches the dataset's
    /// documented mapping: `0 → None` (no loss); `k → Some(2000 + k)`
    /// for k in 1..=HANSEN_LOSSYEAR_MAX_VALUE. Pinned across all
    /// boundary values plus a representative interior.
    #[test]
    fn lossyear_byte_to_calendar_year_mapping() {
        // 0 = "no loss observed" — a Primary fact, not Absence.
        assert_eq!(
            lossyear_byte_to_calendar_year(0),
            None,
            "byte=0 must map to None (no loss observed)"
        );
        // First valid loss year.
        assert_eq!(
            lossyear_byte_to_calendar_year(1),
            Some(2001),
            "byte=1 must map to calendar year 2001"
        );
        // Representative interior — eval question reference year.
        assert_eq!(
            lossyear_byte_to_calendar_year(10),
            Some(2010),
            "byte=10 must map to calendar year 2010"
        );
        // Mid-decade.
        assert_eq!(
            lossyear_byte_to_calendar_year(12),
            Some(2012),
            "byte=12 must map to calendar year 2012"
        );
        // Last valid year published in v1.12 (2025-05 release).
        assert_eq!(
            lossyear_byte_to_calendar_year(HANSEN_LOSSYEAR_MAX_VALUE),
            Some(2024),
            "byte={} must map to 2024 (v1.12 end-of-record)",
            HANSEN_LOSSYEAR_MAX_VALUE
        );
        // Sanity: the constants line up.
        assert_eq!(HANSEN_LOSSYEAR_BASE, 2000);
        assert_eq!(HANSEN_LOSSYEAR_MAX_VALUE, 24);
    }

    /// `HansenGfcError::from_cog` translates a 404-shaped transport
    /// error into [`HansenGfcError::TileNotFound`] (the structured
    /// "cell outside dataset coverage" path). Other transport errors
    /// stay as `Transport`; non-transport COG errors become `Decode`.
    /// This pins the protocol's "Antarctica → Absence, not Err" rule.
    #[test]
    fn from_cog_404_maps_to_tile_not_found() {
        let url = tile_url_for(-75.0, 0.0, LAYER_LOSSYEAR); // Antarctica
        let (lat_tag, lng_tag) = tile_corner_tags(-75.0, 0.0);
        let tile = format!("{lat_tag}_{lng_tag}");
        // Mimic the CogError shape http_range emits on 404.
        let cog_404 =
            CogError::Transport(format!("status 404 Not Found for range 0-65535 on {url}"));
        let err = HansenGfcError::from_cog(cog_404, tile.clone(), LAYER_LOSSYEAR.into(), url);
        match &err {
            HansenGfcError::TileNotFound {
                tile: t,
                layer,
                url: u,
            } => {
                assert_eq!(t, &tile, "tile field must round-trip");
                assert_eq!(layer, LAYER_LOSSYEAR, "layer field must round-trip");
                assert!(u.contains("Hansen_GFC-2024-v1.12_lossyear_"));
            }
            other => panic!("expected TileNotFound, got {other:?}"),
        }

        // Non-404 transport stays Transport so the dispatcher can retry.
        let cog_503 = CogError::Transport("status 503 Service Unavailable".into());
        let err = HansenGfcError::from_cog(
            cog_503,
            "00N_000E".into(),
            LAYER_LOSSYEAR.into(),
            "https://example/x.tif".into(),
        );
        assert!(
            matches!(err, HansenGfcError::Transport(_)),
            "503 must surface as Transport, not TileNotFound — got {err:?}"
        );

        // Non-transport COG errors become Decode (e.g. corrupt LZW
        // stream / unexpected TIFF predictor).
        let cog_decode = CogError::BadMagic(0xdeadbeef);
        let err = HansenGfcError::from_cog(
            cog_decode,
            "00N_000E".into(),
            LAYER_LOSSYEAR.into(),
            "https://example/x.tif".into(),
        );
        assert!(
            matches!(err, HansenGfcError::Decode(_)),
            "BadMagic must surface as Decode — got {err:?}"
        );
    }

    /// Boundary-of-range smoke for `tile_corner_tags`: a cell exactly
    /// on a tile boundary picks the tile whose NORTH edge it is on
    /// (lat) and whose WEST edge it is on (lng). Documents the
    /// floor/ceil contract for future readers.
    #[test]
    fn tile_corner_tags_handles_boundaries() {
        // lat = 0.0 exactly → ceil(0)=0 → 00N. lng = 0.0 → floor(0)=0
        // → 000E. (The tile spans [0°N, -10°N] × [0°E, +10°E].)
        assert_eq!(tile_corner_tags(0.0, 0.0), ("00N".into(), "000E".into()));
        // lat = -10.0 exactly → ceil(-1)=-1*10=-10 → 10S. lng=-10.0
        // → floor(-1)=-1*10=-10 → 010W.
        assert_eq!(
            tile_corner_tags(-10.0, -10.0),
            ("10S".into(), "010W".into())
        );
        // lat = 10.0 exactly → ceil(1)=1*10=10 → 10N.
        assert_eq!(tile_corner_tags(10.0, 10.0), ("10N".into(), "010E".into()));
    }
}
