//! ESA Climate Change Initiative Biomass v6.0 connector.
//!
//! Source: **Santoro, M. et al. (2025). *ESA Biomass Climate Change
//! Initiative (Biomass_cci): Global datasets of forest above-ground
//! biomass for the years 2007, 2010, 2015, 2016, 2017, 2018, 2019, 2020,
//! 2021 and 2022, v6.0*. NERC EDS Centre for Environmental Data
//! Analysis. doi:10.5285/95913ffb6467447ca72c4e9d8cf30501**. Produced
//! by GAMMA Remote Sensing for ESA's CCI programme; published under the
//! ESA CCI Data Policy ("free and open access"). The dataset fuses
//! Sentinel-1 C-band, Envisat ASAR, and ALOS-1/2 PALSAR L-band SAR
//! through the BIOMASAR-C and BIOMASAR-L retrievals, then calibrates
//! against GEDI (NASA) + ICESat-2 (NASA) LiDAR shot-level AGB.
//!
//! **Two rasters per epoch.** Every supported year ships two GeoTIFFs
//! per 10° × 10° tile:
//! - `*-AGB-MERGED-100m-{year}-fv6.0.tif` — above-ground biomass
//!   density in **t/ha (Mg/ha)**, uint16, 100 m equivalent
//!   (0.000888889° at the equator). Pixel value `0` means
//!   "no/very-low biomass" *or* "no-data" (the dataset does not carry
//!   a GDAL_NODATA tag at the L4 product); see honesty notes below.
//! - `*-AGB_SD-MERGED-100m-{year}-fv6.0.tif` — per-pixel **standard
//!   deviation** of the AGB estimate in t/ha, uint16. ESA labels the
//!   layer "AGB_SD" (standard deviation); in geospatial usage this
//!   serves as the per-pixel standard error / 1-σ uncertainty band.
//!
//! **Tile naming.** Files are anchored by their **top-left corner**
//! using the **prefix-style** pattern `{lat_tag}{lng_tag}_…`:
//! - lat: 3 chars, e.g. `N00`, `N80`, `S10`, `S50`. The tag is the
//!   tile's **north edge**, so `N00` covers latitudes 0° down to -10°
//!   and `S10` covers -10° down to -20°.
//! - lng: 4 chars, e.g. `E000`, `E020`, `W120`, `W180`. The tag is
//!   the tile's **west edge**.
//!
//! Note: this differs from the Hansen GFC convention which uses
//! suffix-style tags (`00N_120W.tif`); ESA CCI Biomass uses prefix-style
//! and **no underscore** between the lat and lng tags (`N00E020_…`).
//!
//! **Coverage.** The dataset publishes 299 tile prefixes spanning N80
//! down to S50 in the latitude range and full lng coverage; only tiles
//! intersecting non-trivial land are uploaded. Cells over open ocean,
//! over Antarctica below ~-50°S, or above ~+80°N return a `404` from
//! CEDA — the connector translates that to [`EsaCciBiomassError::CoverageGap`]
//! so the materializer can sign an `Absence` rather than fabricate a
//! zero AGB.
//!
//! **Hosting.** The CEDA archive serves the rasters directly off
//! `https://dap.ceda.ac.uk/neodc/esacci/biomass/data/agb/maps/v6.0/geotiff/{year}/…`
//! — anonymous, no auth, no token. Range requests verified
//! `HTTP/2 206 Partial Content` on 2026-05-16 (`Accept-Ranges: bytes`,
//! `Content-Range: bytes 0-1023/156280199`). The shared
//! [`cog`](crate::cog) sampler reads only the IFD plus the relevant
//! strip (the L4 product is stripped, RowsPerStrip=1, Compression=5
//! LZW, Predictor=1, BitsPerSample=16, SampleFormat=1 unsigned int —
//! all already supported by the sampler).
//!
//! **Honest defaults.**
//! - A confirmed in-coverage pixel reading 0 t/ha **is** a Primary fact
//!   ("no measurable above-ground biomass on this 100 m pixel in the
//!   merged retrieval"); the materializer MUST NOT promote it to
//!   `Absence`. The same goes for the SD value.
//! - A `404` on the tile (open ocean, polar interior) maps to
//!   [`EsaCciBiomassError::CoverageGap`] → materializer signs an
//!   `Absence` (the cell is genuinely outside the dataset's published
//!   tile extent).
//! - A year not in [`ESA_CCI_BIOMASS_EPOCHS`] surfaces as
//!   [`EsaCciBiomassError::YearNotAvailable`] — the dataset publishes
//!   only the 10 documented epochs (2007, 2010, 2015..=2022).

use reqwest::Client;

use crate::cog::CogError;

/// Public version tag for the ESA CCI Biomass release this connector
/// targets. Matches the CEDA archive's `v6.0/` directory and the
/// `fv6.0` suffix on every published GeoTIFF filename. Bump when
/// migrating to v7.0 (already published 2026-05-12) or later.
pub const ESA_CCI_BIOMASS_VERSION_TAG: &str = "v6.0";

/// The 10 epochs published in v6.0, in chronological order. The
/// dataset is not annual: 2007 + 2010 are the original retrieval
/// vintages, then a contiguous 2015..=2022 block was added in
/// subsequent v3..v6 releases. `cog_url_for(year)` returns `None`
/// for any year not in this list.
pub const ESA_CCI_BIOMASS_EPOCHS: &[u16] =
    &[2007, 2010, 2015, 2016, 2017, 2018, 2019, 2020, 2021, 2022];

/// CEDA DAP base URL for the `v6.0/geotiff/` directory tree. Anonymous
/// HTTPS, range-readable (verified 2026-05-16; `dap.ceda.ac.uk` returns
/// `HTTP/2 206` with `Content-Range`). The browser-facing
/// `data.ceda.ac.uk` host issues a `302` to this DAP host; we point
/// directly at the DAP host to avoid the redirect on every request.
const ESA_CCI_BIOMASS_BASE_URL: &str =
    "https://dap.ceda.ac.uk/neodc/esacci/biomass/data/agb/maps/v6.0/geotiff";

/// Layer-name segment used inside the AGB filename
/// (`*-AGB-MERGED-100m-…`).
const ESA_CCI_BIOMASS_LAYER_AGB: &str = "AGB";

/// Layer-name segment used inside the standard-deviation filename
/// (`*-AGB_SD-MERGED-100m-…`). ESA labels the per-pixel uncertainty
/// "AGB_SD" — standard deviation, equivalent to a 1-σ standard error
/// band on the merged AGB retrieval.
const ESA_CCI_BIOMASS_LAYER_AGB_SD: &str = "AGB_SD";

/// One decoded ESA CCI Biomass sample at a `(lat, lng, year)` cell.
///
/// Both fields are in **t/ha (Mg/ha)** — the dataset's documented
/// physical unit per the GDAL metadata XML embedded in every tile.
/// Returned as `f32` because the underlying uint16 round-trips
/// losslessly into `f32` (max representable integer 16 777 216 ≫ the
/// 65 535 ceiling of uint16) and most downstream algorithms ingest
/// floating-point biomass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EsaCciBiomassSample {
    /// Above-ground biomass density at the cell in t/ha.
    pub agb_t_per_ha: f32,
    /// Per-pixel standard deviation (1-σ) of the AGB estimate in t/ha.
    pub se_t_per_ha: f32,
}

/// Errors specific to the ESA CCI Biomass v6.0 connector.
///
/// Each variant carries enough context for a materializer to sign the
/// correct fact shape (Primary, Absence, or hard error). Bubbled up
/// through [`crate::FetchError::Transport`] at the dispatcher boundary
/// so callers don't have to thread two error types.
#[derive(Debug, thiserror::Error)]
pub enum EsaCciBiomassError {
    /// HTTP / network failure other than 404. Caller should treat as a
    /// transport error and let the dispatcher retry.
    #[error("transport: {0}")]
    Transport(String),
    /// COG parse / decode failure (TIFF layout, LZW stream corruption,
    /// pixel out of dataset range). Indicates upstream corruption — the
    /// no-fallback rule applies.
    #[error("decode: {0}")]
    Decode(String),
    /// Cell sits outside the dataset's published tile coverage
    /// (open-ocean tile not uploaded, Antarctic interior below the
    /// southernmost tile prefix `S50`, or Arctic above the northernmost
    /// `N80`). Materializers MUST sign this as an `Absence` — the cell
    /// is genuinely outside the dataset.
    #[error(
        "coverage_gap: cell (lat={lat:.6}, lng={lng:.6}) maps to ESA CCI Biomass v6.0 tile that is not published"
    )]
    CoverageGap {
        /// Cell latitude, for diagnostics.
        lat: f64,
        /// Cell longitude, for diagnostics.
        lng: f64,
    },
    /// Caller asked for a year outside the 10 documented epochs
    /// (2007, 2010, 2015..=2022). The dataset is **not annual** —
    /// 2008, 2009, 2011..2014 simply do not exist in v6.0. Surface a
    /// hard error rather than silently falling back to the nearest
    /// year.
    #[error(
        "year_not_available: {year} is not one of the 10 v6.0 epochs (2007, 2010, 2015..=2022)"
    )]
    YearNotAvailable {
        /// The year the caller requested.
        year: u16,
    },
    /// Honest disclosure for any not-yet-implemented sub-feature.
    /// Reserved for future use (e.g. annualised aggregated 1 km / 25 km
    /// products); the per-cell live path uses the variants above.
    #[error("not_implemented: {reason}")]
    NotImplemented {
        /// Why the connector cannot fulfil the request right now.
        reason: String,
    },
}

impl EsaCciBiomassError {
    /// Map a [`CogError`] into the appropriate connector-specific
    /// variant. HTTP 404 from the COG sampler indicates a missing tile
    /// (open ocean, polar interior); transport errors surface as
    /// `Transport`; everything else (decode, codec, layout) becomes
    /// `Decode`.
    fn from_cog(e: CogError, lat: f64, lng: f64) -> Self {
        let msg = e.to_string();
        let lower = msg.to_lowercase();
        if lower.contains("status 404") || lower.contains("not found") {
            return EsaCciBiomassError::CoverageGap { lat, lng };
        }
        match e {
            CogError::Transport(s) => EsaCciBiomassError::Transport(s),
            other => EsaCciBiomassError::Decode(other.to_string()),
        }
    }
}

/// Compute the `(lat_tag, lng_tag)` pair for the 10° tile whose
/// **top-left** corner anchors the cell. The tags are prefix-style:
/// - `N00`, `N80`, `S10`, `S50` for latitude (3 chars, sign letter
///   first, two digits of the absolute degrees).
/// - `E000`, `E020`, `W120`, `W180` for longitude (4 chars).
///
/// Anchoring rule (matches Hansen GFC + JRC GFC2020 conventions):
/// `lat_top = ceil(lat / 10) * 10`, `lng_left = floor(lng / 10) * 10`.
/// A cell on a tile boundary picks the tile whose **NORTH edge** it is
/// on (lat) and whose **WEST edge** it is on (lng).
pub fn tile_corner_tags(lat: f64, lng: f64) -> (String, String) {
    let lat_top = (lat / 10.0).ceil() as i32 * 10;
    let lng_left = (lng / 10.0).floor() as i32 * 10;
    let lat_tag = if lat_top >= 0 {
        format!("N{:02}", lat_top)
    } else {
        format!("S{:02}", lat_top.unsigned_abs())
    };
    let lng_tag = if lng_left >= 0 {
        format!("E{:03}", lng_left)
    } else {
        format!("W{:03}", lng_left.unsigned_abs())
    };
    (lat_tag, lng_tag)
}

/// Return `true` iff `year` is one of the 10 documented v6.0 epochs.
pub fn year_is_supported(year: u16) -> bool {
    ESA_CCI_BIOMASS_EPOCHS.contains(&year)
}

/// Build a tile filename for a given `(lat, lng, year, layer)` quad.
/// `layer` must be `"AGB"` or `"AGB_SD"` — anything else panics in
/// debug builds (this is an internal helper). Pure — no I/O.
fn tile_filename(lat_tag: &str, lng_tag: &str, year: u16, layer: &str) -> String {
    debug_assert!(
        layer == ESA_CCI_BIOMASS_LAYER_AGB || layer == ESA_CCI_BIOMASS_LAYER_AGB_SD,
        "esa_cci_biomass tile_filename called with unknown layer {layer:?}"
    );
    format!("{lat_tag}{lng_tag}_ESACCI-BIOMASS-L4-{layer}-MERGED-100m-{year}-fv6.0.tif")
}

/// Build the full HTTPS URL for a given `(lat, lng, year, layer)` quad.
/// Pure — no I/O; returns the same URL regardless of whether the tile
/// actually exists upstream.
fn tile_url(lat: f64, lng: f64, year: u16, layer: &str) -> String {
    let (lat_tag, lng_tag) = tile_corner_tags(lat, lng);
    let fname = tile_filename(&lat_tag, &lng_tag, year, layer);
    format!("{ESA_CCI_BIOMASS_BASE_URL}/{year}/{fname}")
}

/// Return the **directory URL** containing every AGB raster for the
/// given epoch, or `None` if `year` is not in
/// [`ESA_CCI_BIOMASS_EPOCHS`]. The directory holds 299 tile pairs
/// (AGB + AGB_SD) named by their top-left corner; compose a per-cell
/// URL with [`tile_url_for`] (or [`tile_url_se_for`] for the SD
/// raster). Pure — no I/O.
///
/// Example return: `"https://dap.ceda.ac.uk/neodc/esacci/biomass/data/agb/maps/v6.0/geotiff/2022"`.
pub fn cog_url_for(year: u16) -> Option<String> {
    if !year_is_supported(year) {
        return None;
    }
    Some(format!("{ESA_CCI_BIOMASS_BASE_URL}/{year}"))
}

/// Return the **directory URL** for the AGB **standard-deviation**
/// rasters of the given epoch, or `None` if `year` is not in
/// [`ESA_CCI_BIOMASS_EPOCHS`]. The SD rasters live in the same
/// directory as the AGB rasters — only the layer segment in each
/// filename differs (`AGB_SD` vs `AGB`) — so this returns the same
/// directory URL as [`cog_url_for`] but exists as a distinct entry
/// point for callers that index the two layers independently.
pub fn cog_url_se_for(year: u16) -> Option<String> {
    cog_url_for(year)
}

/// Return the per-cell HTTPS URL of the AGB raster covering
/// `(lat, lng)` at `year`, or `None` if `year` is not in
/// [`ESA_CCI_BIOMASS_EPOCHS`] or the coordinates are out of range.
/// Pure — no I/O; the URL points at a tile that may or may not exist
/// upstream (open-ocean tiles return 404 → caller signs Absence).
pub fn tile_url_for(year: u16, lat: f64, lng: f64) -> Option<String> {
    if !year_is_supported(year) {
        return None;
    }
    if !lat.is_finite() || !lng.is_finite() {
        return None;
    }
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lng) {
        return None;
    }
    Some(tile_url(lat, lng, year, ESA_CCI_BIOMASS_LAYER_AGB))
}

/// Return the per-cell HTTPS URL of the AGB_SD (per-pixel standard
/// deviation; serves as the 1-σ uncertainty band) raster covering
/// `(lat, lng)` at `year`, or `None` under the same gating rules as
/// [`tile_url_for`].
pub fn tile_url_se_for(year: u16, lat: f64, lng: f64) -> Option<String> {
    if !year_is_supported(year) {
        return None;
    }
    if !lat.is_finite() || !lng.is_finite() {
        return None;
    }
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lng) {
        return None;
    }
    Some(tile_url(lat, lng, year, ESA_CCI_BIOMASS_LAYER_AGB_SD))
}

/// Sample one pixel from one ESA CCI Biomass raster (AGB or AGB_SD)
/// and return the value as `f32` t/ha. Internal helper — both public
/// fetches share the same EPSG:4326 + LZW + uint16 read path.
async fn sample_one(
    client: &Client,
    url: &str,
    lat: f64,
    lng: f64,
) -> Result<f32, EsaCciBiomassError> {
    let profile = crate::cog::open_profile(client, url)
        .await
        .map_err(|e| EsaCciBiomassError::from_cog(e, lat, lng))?;
    // EPSG:4326 — sample directly with (lng, lat) as world (x, y).
    // The shared sampler honours the per-tile geo-transform via
    // `world_to_pixel`.
    let raw = crate::cog::sample_pixel(client, url, &profile, lng, lat)
        .await
        .map_err(|e| EsaCciBiomassError::from_cog(e, lat, lng))?;
    if !raw.is_finite() || raw < 0.0 || raw > u16::MAX as f64 {
        return Err(EsaCciBiomassError::Decode(format!(
            "non-uint16 pixel value {raw} from {url}"
        )));
    }
    // Cast through u16 to make the integer round-trip explicit; then
    // promote to f32 (lossless for the full uint16 range).
    Ok(raw.round() as u16 as f32)
}

/// Read one pixel from the ESA CCI Biomass v6.0 AGB + AGB_SD pair for
/// the given `(lat, lng, year)` and return both as
/// [`EsaCciBiomassSample`].
///
/// Returns:
/// - `Ok(EsaCciBiomassSample { agb_t_per_ha, se_t_per_ha })` for an
///   in-coverage cell. Both values may be `0.0` legitimately (sparse
///   savanna, bare desert, snow/ice cell with no canopy); the
///   materializer signs that as a Primary fact.
/// - `Err(YearNotAvailable)` when `year` is not one of the 10
///   documented epochs.
/// - `Err(CoverageGap)` when the cell sits outside the published
///   tile extent (open-ocean tile, polar interior). The materializer
///   signs an `Absence`.
/// - `Err(Transport)` for HTTP / network failures.
/// - `Err(Decode)` for COG layout / codec / range failures.
pub async fn fetch_agb(
    client: &Client,
    lat: f64,
    lng: f64,
    year: u16,
) -> Result<EsaCciBiomassSample, EsaCciBiomassError> {
    if !year_is_supported(year) {
        return Err(EsaCciBiomassError::YearNotAvailable { year });
    }
    if !lat.is_finite() || !lng.is_finite() {
        return Err(EsaCciBiomassError::CoverageGap { lat, lng });
    }
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lng) {
        return Err(EsaCciBiomassError::CoverageGap { lat, lng });
    }

    let agb_url = tile_url(lat, lng, year, ESA_CCI_BIOMASS_LAYER_AGB);
    let agb = sample_one(client, &agb_url, lat, lng).await?;

    let sd_url = tile_url(lat, lng, year, ESA_CCI_BIOMASS_LAYER_AGB_SD);
    let se = sample_one(client, &sd_url, lat, lng).await?;

    Ok(EsaCciBiomassSample {
        agb_t_per_ha: agb,
        se_t_per_ha: se,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ESA_CCI_BIOMASS_VERSION_TAG` is pinned to the dataset directory
    /// segment on CEDA. Bumping to v7.0 (already published 2026-05-12)
    /// is intentionally a one-line, reviewable change.
    #[test]
    fn version_tag_is_v6() {
        assert_eq!(ESA_CCI_BIOMASS_VERSION_TAG, "v6.0");
    }

    /// The 10 documented epochs match the CEDA `geotiff/` directory
    /// listing exactly: 2007 + 2010 (original retrievals) and the
    /// contiguous 2015..=2022 block (added in v3..v6). Crucially this
    /// list excludes 2008, 2009, and 2011..=2014 — the dataset is
    /// **not annual**.
    #[test]
    fn epochs_match_ceda_listing() {
        assert_eq!(
            ESA_CCI_BIOMASS_EPOCHS,
            &[2007, 2010, 2015, 2016, 2017, 2018, 2019, 2020, 2021, 2022],
            "epochs MUST mirror the CEDA v6.0/geotiff/ directory tree"
        );
        assert_eq!(ESA_CCI_BIOMASS_EPOCHS.len(), 10);
        // Spot-checks: gap years that do NOT exist in v6.0.
        assert!(!year_is_supported(2008));
        assert!(!year_is_supported(2009));
        assert!(!year_is_supported(2011));
        assert!(!year_is_supported(2014));
        assert!(!year_is_supported(2023));
        // Endpoints of the supported set.
        assert!(year_is_supported(2007));
        assert!(year_is_supported(2022));
    }

    /// `tile_corner_tags` honours the prefix-style 10° anchoring used
    /// by ESA CCI Biomass: lat sign letter FIRST then two digits, lng
    /// sign letter FIRST then three digits. Reference cells:
    ///
    /// - Central Congo Basin (lat=-1.0, lng=23.0) → `N00E020`. The
    ///   tile spans 0° down to -10° (north edge at 0°N) and 20° to
    ///   30° (west edge at 20°E), matching the file we probed live
    ///   at `…/2022/N00E020_ESACCI-BIOMASS-L4-AGB-MERGED-100m-2022-fv6.0.tif`.
    /// - Central Amazon (lat=-3.0, lng=-60.5) → `N00W070`.
    /// - Sumatra (lat=-1.0, lng=100.0) → `N00E100`.
    /// - Boreal Sweden (lat=63.0, lng=15.0) → `N70E010`.
    /// - Patagonia (lat=-45.0, lng=-70.0) → `S40W070`.
    #[test]
    fn tile_corner_tags_known_cells() {
        assert_eq!(
            tile_corner_tags(-1.0, 23.0),
            ("N00".into(), "E020".into()),
            "Congo Basin (-1, 23) must anchor at N00E020"
        );
        assert_eq!(
            tile_corner_tags(-3.0, -60.5),
            ("N00".into(), "W070".into()),
            "Central Amazon (-3, -60.5) must anchor at N00W070"
        );
        assert_eq!(
            tile_corner_tags(-1.0, 100.0),
            ("N00".into(), "E100".into()),
            "Sumatra (-1, 100) must anchor at N00E100"
        );
        assert_eq!(
            tile_corner_tags(63.0, 15.0),
            ("N70".into(), "E010".into()),
            "Boreal Sweden (63, 15) must anchor at N70E010"
        );
        assert_eq!(
            tile_corner_tags(-45.0, -70.0),
            ("S40".into(), "W070".into()),
            "Patagonia (-45, -70) must anchor at S40W070"
        );
        // Boundary: lat=0 exactly → north edge at 0 → "N00". lng=0
        // exactly → west edge at 0 → "E000".
        assert_eq!(
            tile_corner_tags(0.0, 0.0),
            ("N00".into(), "E000".into()),
            "(0, 0) lands in tile N00E000 (the equator/Greenwich tile)"
        );
        // Boundary: lat=-10.0 exactly → ceil(-1)*10=-10 → "S10".
        assert_eq!(
            tile_corner_tags(-10.0, -10.0),
            ("S10".into(), "W010".into())
        );
    }

    /// `cog_url_for(year)` returns the **directory URL** that holds
    /// every per-tile AGB raster for the requested epoch, and `None`
    /// for an out-of-range year. Pinned literally so any accidental
    /// path edit (host, version, year segment) is caught at test time.
    #[test]
    fn cog_url_for_directories() {
        let url = cog_url_for(2022).expect("year 2022 is in EPOCHS");
        assert_eq!(
            url, "https://dap.ceda.ac.uk/neodc/esacci/biomass/data/agb/maps/v6.0/geotiff/2022",
            "directory URL must be the CEDA DAP path for the year"
        );
        let url = cog_url_for(2007).expect("year 2007 is in EPOCHS");
        assert!(url.ends_with("/v6.0/geotiff/2007"));
        // Year not in the 10 published epochs → None (no fallback).
        assert!(cog_url_for(2008).is_none(), "2008 is a gap year");
        assert!(
            cog_url_for(2024).is_none(),
            "2024 is past v6.0 (covered by v7.0)"
        );
        // cog_url_se_for mirrors cog_url_for at the directory level —
        // AGB and AGB_SD share the same directory; the difference is
        // in each filename's layer segment.
        assert_eq!(cog_url_se_for(2022), cog_url_for(2022));
        assert!(cog_url_se_for(2008).is_none());
    }

    /// `tile_url_for` and `tile_url_se_for` compose the per-cell
    /// HTTPS URL by anchoring the 10° tile and substituting the
    /// AGB / AGB_SD layer segment. Pinned to the live CEDA paths.
    #[test]
    fn tile_url_for_known_cells() {
        // Central Congo Basin, 2022 → N00E020 AGB tile.
        let url = tile_url_for(2022, -1.0, 23.0).expect("Congo Basin / 2022 must resolve");
        assert_eq!(
            url,
            "https://dap.ceda.ac.uk/neodc/esacci/biomass/data/agb/maps/v6.0/geotiff/2022/N00E020_ESACCI-BIOMASS-L4-AGB-MERGED-100m-2022-fv6.0.tif",
            "Congo Basin / 2022 AGB tile must point at the live CEDA path"
        );
        // SE sibling: same path, AGB_SD layer segment.
        let sd = tile_url_se_for(2022, -1.0, 23.0).expect("Congo Basin / 2022 SD must resolve");
        assert_eq!(
            sd,
            "https://dap.ceda.ac.uk/neodc/esacci/biomass/data/agb/maps/v6.0/geotiff/2022/N00E020_ESACCI-BIOMASS-L4-AGB_SD-MERGED-100m-2022-fv6.0.tif"
        );
        // The AGB and AGB_SD URLs must differ ONLY in the layer
        // segment — same year, same tile prefix, same suffix.
        assert_eq!(
            url.replace("-AGB-MERGED-", "-AGB_SD-MERGED-"),
            sd,
            "AGB and AGB_SD URLs must be identical except for the layer segment"
        );
        // Same cell, 2007 → swap year segment + filename year.
        let url = tile_url_for(2007, -1.0, 23.0).expect("Congo Basin / 2007 must resolve");
        assert!(
            url.contains("/2007/") && url.contains("-2007-fv6.0.tif"),
            "year 2007 must change both the directory segment and the filename year — got {url}"
        );
        // Central Amazon, 2020 → N00W070 AGB tile.
        let url = tile_url_for(2020, -3.0, -60.5).expect("Central Amazon / 2020 must resolve");
        assert!(
            url.ends_with("/N00W070_ESACCI-BIOMASS-L4-AGB-MERGED-100m-2020-fv6.0.tif"),
            "Central Amazon / 2020 must end at N00W070 / 2020 / fv6.0 — got {url}"
        );
        // Year not in the 10 published epochs → None (no fallback).
        assert!(tile_url_for(2008, -1.0, 23.0).is_none());
        // Bad lat/lng → None.
        assert!(tile_url_for(2022, 91.0, 0.0).is_none());
        assert!(tile_url_for(2022, 0.0, 181.0).is_none());
        assert!(tile_url_for(2022, f64::NAN, 0.0).is_none());
        assert!(tile_url_se_for(2022, 0.0, f64::INFINITY).is_none());
    }

    /// `fetch_agb` short-circuits to `YearNotAvailable` for any year
    /// not in the 10 published epochs — no network touched. This pins
    /// the no-fallback rule: we never silently round to the nearest
    /// available year.
    #[tokio::test]
    async fn fetch_agb_year_not_available() {
        let client = Client::new();
        for bad_year in [2006_u16, 2008, 2011, 2014, 2023, 2025] {
            let err = fetch_agb(&client, -1.0, 23.0, bad_year).await.unwrap_err();
            match err {
                EsaCciBiomassError::YearNotAvailable { year } => {
                    assert_eq!(year, bad_year, "round-trip the requested year");
                }
                other => panic!("year {bad_year} must surface YearNotAvailable, got {other:?}"),
            }
        }
    }

    /// `fetch_agb` short-circuits to `CoverageGap` for invalid lat/lng
    /// before issuing a network request. Mirrors the JRC GFC2020
    /// connector's bounds contract.
    #[tokio::test]
    async fn fetch_agb_invalid_coords_is_coverage_gap() {
        let client = Client::new();
        // Valid year, invalid latitude.
        let err = fetch_agb(&client, 95.0, 0.0, 2022).await.unwrap_err();
        assert!(
            matches!(err, EsaCciBiomassError::CoverageGap { .. }),
            "lat 95 must surface CoverageGap, got {err:?}"
        );
        // Valid year, invalid longitude.
        let err = fetch_agb(&client, 0.0, -181.0, 2022).await.unwrap_err();
        assert!(
            matches!(err, EsaCciBiomassError::CoverageGap { .. }),
            "lng -181 must surface CoverageGap, got {err:?}"
        );
        // NaN lat — must surface as coverage gap, not propagate to
        // the COG sampler.
        let err = fetch_agb(&client, f64::NAN, 0.0, 2022).await.unwrap_err();
        assert!(
            matches!(err, EsaCciBiomassError::CoverageGap { .. }),
            "NaN lat must surface CoverageGap, got {err:?}"
        );
    }

    /// `EsaCciBiomassError::from_cog` translates a transport-shaped COG
    /// error into [`EsaCciBiomassError::Transport`] (so the dispatcher
    /// can retry); HTTP 404 becomes `CoverageGap` (genuine missing
    /// tile); other COG errors become `Decode` (the no-fallback rule
    /// applies — we never invent a default value).
    #[test]
    fn from_cog_routes_transport_decode_and_404() {
        let cog_transport = CogError::Transport("status 503 Service Unavailable".into());
        let err = EsaCciBiomassError::from_cog(cog_transport, 0.0, 0.0);
        assert!(
            matches!(err, EsaCciBiomassError::Transport(_)),
            "Transport must round-trip as Transport, got {err:?}"
        );

        let cog_404 = CogError::Transport("status 404 Not Found".into());
        let err = EsaCciBiomassError::from_cog(cog_404, -75.0, 0.0);
        match err {
            EsaCciBiomassError::CoverageGap { lat, lng } => {
                assert!((lat - (-75.0)).abs() < 1e-9);
                assert!((lng - 0.0).abs() < 1e-9);
            }
            other => panic!("404 must surface as CoverageGap, got {other:?}"),
        }

        let cog_decode = CogError::BadMagic(0xdeadbeef);
        let err = EsaCciBiomassError::from_cog(cog_decode, 0.0, 0.0);
        assert!(
            matches!(err, EsaCciBiomassError::Decode(_)),
            "BadMagic must surface as Decode, got {err:?}"
        );

        let cog_unsupported = CogError::Unsupported("planar config 2".into());
        let err = EsaCciBiomassError::from_cog(cog_unsupported, 0.0, 0.0);
        assert!(
            matches!(err, EsaCciBiomassError::Decode(_)),
            "Unsupported must surface as Decode, got {err:?}"
        );
    }

    /// Constants sanity: base URL points at the CEDA DAP host (the
    /// authenticated-only dap subdomain that returns range-readable
    /// `image/tiff`), version tag matches the published v6.0 release,
    /// and the layer segments are the exact strings ESA puts in their
    /// filenames.
    #[test]
    fn constants_sanity() {
        assert!(
            ESA_CCI_BIOMASS_BASE_URL.starts_with("https://dap.ceda.ac.uk/"),
            "base URL must point at the CEDA DAP host (range-readable, anonymous)"
        );
        assert!(
            ESA_CCI_BIOMASS_BASE_URL.ends_with("/v6.0/geotiff"),
            "base URL must end at the v6.0/geotiff directory"
        );
        assert_eq!(ESA_CCI_BIOMASS_LAYER_AGB, "AGB");
        assert_eq!(ESA_CCI_BIOMASS_LAYER_AGB_SD, "AGB_SD");
    }
}
