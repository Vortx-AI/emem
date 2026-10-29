//! `band_raster@1` — a field as a signed derivation, not a byte pipe.
//!
//! docs/plans/field-tokens.md, built in the order it states. The receipt
//! for a raster attests a DERIVATION: this responder computed the
//! artifact whose blake3 is `artifact_cid`, from a pinned upstream
//! scene, via this versioned recipe, over this content-addressed
//! geometry and time slot. The pixels themselves live in the evictable
//! artifact store; the signed derivation record persists as a
//! `DerivativeFact` (the attribution-ledger envelope) and carries
//! everything a stranger needs to rebuild the identical bytes.
//!
//! Verification tiers, per the design: spot-check (re-hash the artifact,
//! read the sampled `anchors[]` back out of the grid, and where an
//! anchor carries a fact_cid compare against the ordinary per-cell
//! fact) and full recompute (fetch the pinned scene, rerun the recipe,
//! compare digests). Anchors are BEST-EFFORT by owner decision: a
//! required anchor would force cold-AOI materialization and recreate
//! the exact gateway-timeout trap the partial-results roadmap item
//! warns about, so an anchor cites an existing fact when the store
//! holds one and never materializes to invent the bridge.
//!
//! Honest bounds, stated in refusals rather than discovered in
//! timeouts: the AOI is a bbox, the band allowlist is the raw
//! Sentinel-2 reflectance set below plus the DEM, the encoder fields and
//! the static forest layers, and the window caps at `MAX_SIDE_PX` per
//! side (`FOREST_MAX_SIDE_PX` for a forest layer).

use std::sync::LazyLock;
use std::time::Instant;

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};

use emem_cache::CanonicalKey;
use emem_codec::grid::{encode_grid, grid_value_at, GridHeader};
use emem_fact::{Derivation, DerivativeFact, Fact, FactCid, FieldBinding};
use emem_storage::artifacts::ArtifactStore;

use crate::change_attribution::json_to_cbor;
use crate::{ApiError, AppState, EmemJson, ErrorBody};
use emem_core::ErrorCode;

/// Hard cap per window side. 512 px at 10 m is a 5.12 km square, well
/// past the 2 km AOI the roadmap's reporting agent needed, and small
/// enough that a cold scene read stays inside the gateway budget.
pub(crate) const MAX_SIDE_PX: u32 = 512;

/// The scene classification asset: `scl` on Element84, `SCL` on Planetary
/// Computer.
pub(crate) fn scl_asset(item: &emem_fetch::stac::StacItem) -> Option<String> {
    ["scl", "SCL"]
        .iter()
        .find_map(|k| item.assets.get(*k).cloned())
}

/// A DN on the harmonised scale (Element84's), whichever catalogue served it.
/// 0 is L2A no-data in both conventions and stays 0.
fn harmonise(dn: f64, offset: f64) -> f32 {
    if dn == 0.0 {
        0.0
    } else {
        (dn + offset) as f32
    }
}

/// The raw Sentinel-2 bands this executor serves, with the Element84
/// STAC asset aliases each may appear under. An allowlist, not a
/// routing guess: any other band is a typed refusal naming this list.
pub(crate) const S2_BANDS: [(&str, &[&str]); 6] = [
    ("s2.B02", &["blue", "B02"]),
    ("s2.B03", &["green", "B03"]),
    ("s2.B04", &["red", "B04"]),
    ("s2.B08", &["nir", "B08"]),
    ("s2.B11", &["swir16", "B11"]),
    ("s2.B12", &["swir22", "B12"]),
];

/// Band spellings that route to the Copernicus GLO-30 DEM raster path
/// (static global elevation COG — no scene selection). WB-3.
const DEM_BAND_ALIASES: [&str; 4] = [
    "copdem30m.elevation",
    "copdem30m.elevation_mean",
    "elevation",
    "dem",
];

fn is_dem_band(band: &str) -> bool {
    DEM_BAND_ALIASES.contains(&band)
}

/// Encoder bands whose recall yields an N-D vector, so their raster is a
/// multi-channel EMBEDDING FIELD (WB-5) rather than a scalar grid.
const EMBEDDING_BANDS: &[&str] = &["geotessera", "geotessera.multi_year"];
/// Native sampling step for the embedding grid (geotessera is a 0.1° global
/// grid; sampling finer just repeats a cell's vector).
const EMBEDDING_STEP_DEG: f64 = 0.1;
/// Bounds the recall fan-out for one embedding field.
const MAX_EMBEDDING_CELLS: u32 = 256;

fn is_embedding_band(band: &str) -> bool {
    EMBEDDING_BANDS.contains(&band)
}

/// The evictable blob store, module-owned like the koppen cache:
/// `$EMEM_DATA/artifacts`, capped by `EMEM_ARTIFACTS_MAX_BYTES`
/// (default 4 GiB).
pub(crate) static ARTIFACTS: LazyLock<ArtifactStore> = LazyLock::new(|| {
    let base = emem_core::data_dir::data_dir()
        .to_string_lossy()
        .into_owned();
    let cap = std::env::var("EMEM_ARTIFACTS_MAX_BYTES")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(4 * 1024 * 1024 * 1024);
    ArtifactStore::open(format!("{base}/artifacts"), cap)
        .expect("artifact store root must be creatable under EMEM_DATA")
});

/// Media type of the canonical grid encoding (emem-codec `grid.rs`).
const GRID_MEDIA_TYPE: &str = "application/x.emem-grid-f32.v1";

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct BBox {
    pub min_lat: f64,
    pub min_lng: f64,
    pub max_lat: f64,
    pub max_lng: f64,
}

#[derive(Debug, Deserialize)]
pub struct BandRasterReq {
    pub bbox: BBox,
    /// One of the `S2_BANDS` keys, e.g. `"s2.B04"`.
    pub band: String,
    /// Optional target capture date `YYYY-MM-DD`; defaults to now. The
    /// scene actually chosen is pinned in the derivation record either
    /// way.
    #[serde(default)]
    pub observed_on: Option<String>,
}

/// `YYYY-MM-DD` into (year, month, day); the repo's `days_from_civil`
/// turns it into Unix time without a date crate.
pub(crate) fn parse_ymd(s: &str) -> Option<(i32, u32, u32)> {
    let mut it = s.split('-');
    let y: i32 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let d: u32 = it.next()?.parse().ok()?;
    if it.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some((y, m, d))
}

pub(crate) fn bad_request(message: String) -> ApiError {
    ApiError(
        StatusCode::BAD_REQUEST,
        ErrorBody {
            code: ErrorCode::InvalidArgument,
            message,
            details: None,
        },
    )
}

pub(crate) fn upstream_error(message: String) -> ApiError {
    ApiError(
        StatusCode::BAD_GATEWAY,
        ErrorBody {
            code: ErrorCode::SourceFetchFailed,
            message,
            details: None,
        },
    )
}

/// `aoi_cid`: the full blake3 of the bbox's canonical CBOR, base32, 52
/// chars: the same identifier discipline every fact uses, so the same
/// area always names the same field.
pub(crate) fn aoi_cid(b: &BBox) -> Result<String, ApiError> {
    let bytes = emem_fact::cbor::to_canonical_cbor(b)
        .map_err(|e| bad_request(format!("bbox does not canonicalize: {e}")))?;
    Ok(data_encoding::BASE32_NOPAD
        .encode(blake3::hash(&bytes).as_bytes())
        .to_lowercase())
}

pub async fn band_raster(req: BandRasterReq, s: &AppState) -> Result<JsonValue, ApiError> {
    let started = Instant::now();
    let b = &req.bbox;
    if !(b.min_lat < b.max_lat && b.min_lng < b.max_lng) {
        return Err(bad_request(
            "bbox must satisfy min_lat < max_lat and min_lng < max_lng".into(),
        ));
    }
    if b.min_lat < -90.0 || b.max_lat > 90.0 || b.min_lng < -180.0 || b.max_lng > 180.0 {
        return Err(bad_request("bbox exceeds WGS-84 bounds".into()));
    }
    // WB-3: the DEM is a static global COG, so its raster path skips scene
    // selection entirely and reads a native-degree window straight out of
    // the Copernicus GLO-30 tile covering the bbox.
    if is_dem_band(&req.band) {
        return dem_raster(req, s).await;
    }
    // WB-5: an encoder band rasterises as a multi-channel embedding field.
    if is_embedding_band(&req.band) {
        return embedding_raster(req, s).await;
    }
    if FOREST_BANDS.iter().any(|f| f.band == req.band) {
        return forest_raster(req, s).await;
    }
    let Some((band_key, aliases)) = S2_BANDS.iter().find(|(k, _)| *k == req.band) else {
        let names: Vec<&str> = S2_BANDS
            .iter()
            .map(|(k, _)| *k)
            .chain(DEM_BAND_ALIASES.iter().take(1).copied())
            .chain(EMBEDDING_BANDS.iter().copied())
            .chain(FOREST_BANDS.iter().map(|f| f.band))
            .collect();
        return Err(bad_request(format!(
            "band `{}` is not rasterizable; this executor serves {}",
            req.band,
            names.join(", ")
        )));
    };

    // ── scene selection, pinned. ───────────────────────────────────────
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let target_unix: Option<i64> = match &req.observed_on {
        Some(d) => Some(
            parse_ymd(d)
                .map(|(y, m, dd)| crate::days_from_civil(y, m, dd) * 86_400 + 43_200)
                .ok_or_else(|| bad_request(format!("observed_on `{d}` is not YYYY-MM-DD")))?,
        ),
        None => None,
    };
    let centre_lat = (b.min_lat + b.max_lat) / 2.0;
    let centre_lng = (b.min_lng + b.max_lng) / 2.0;
    let cli = crate::s2_http_client();
    let (item, _cloud, _days, host) =
        crate::s2_search_with_fallback(&cli, centre_lng, centre_lat, target_unix, now_unix)
            .await
            .map_err(upstream_error)?;
    // Values stay on the harmonised DN scale whichever catalogue served the
    // scene: see `crate::s2_dn_offset`.
    let dn_offset = crate::s2_dn_offset(host, &item).map_err(upstream_error)?;
    let epsg = item
        .epsg
        .ok_or_else(|| upstream_error("stac item missing proj:epsg".into()))?;
    let url = aliases
        .iter()
        .find_map(|a| item.assets.get(*a).cloned())
        .ok_or_else(|| upstream_error(format!("scene {} has no asset for {band_key}", item.id)))?;

    // ── window math in the scene's CRS. ────────────────────────────────
    let utm_c = emem_fetch::proj::latlng_to_utm_with_epsg(centre_lat, centre_lng, epsg)
        .ok_or_else(|| upstream_error(format!("epsg {epsg} is not a UTM code")))?;
    let utm_min = emem_fetch::proj::latlng_to_utm_with_epsg(b.min_lat, b.min_lng, epsg)
        .ok_or_else(|| upstream_error("bbox corner outside the scene's UTM zone".into()))?;
    let utm_max = emem_fetch::proj::latlng_to_utm_with_epsg(b.max_lat, b.max_lng, epsg)
        .ok_or_else(|| upstream_error("bbox corner outside the scene's UTM zone".into()))?;
    let prof = emem_fetch::cog::open_profile(&cli, &url)
        .await
        .map_err(|e| upstream_error(format!("open COG: {e}")))?;
    let native_m = prof.pixel_scale.0.abs();
    if !(native_m > 0.0 && native_m.is_finite()) {
        return Err(upstream_error(format!(
            "COG has implausible pixel scale {:?}",
            prof.pixel_scale
        )));
    }
    let width_px = (((utm_max.easting - utm_min.easting).abs() / native_m).ceil() as u32).max(1);
    let height_px = (((utm_max.northing - utm_min.northing).abs() / native_m).ceil() as u32).max(1);
    if width_px > MAX_SIDE_PX || height_px > MAX_SIDE_PX {
        return Err(bad_request(format!(
            "window is {width_px}x{height_px} px at the band's native {native_m} m; the cap is {MAX_SIDE_PX} per side. Shrink the bbox or page it."
        )));
    }

    let raw = emem_fetch::cog::sample_window(
        &cli,
        &url,
        &prof,
        utm_c.easting,
        utm_c.northing,
        width_px,
        height_px,
    )
    .await
    .map_err(|e| upstream_error(format!("sample_window: {e}")))?;

    // The window's own origin in pixel space is what sample_window used:
    // centred on the point, half the dims each way.
    let (centre_col, centre_row) = prof.world_to_pixel(utm_c.easting, utm_c.northing);
    let col0 = centre_col - (width_px as i64) / 2;
    let row0 = centre_row - (height_px as i64) / 2;
    let (ti, tj, tx, ty) = prof.tiepoint;
    let (sx, sy) = prof.pixel_scale;
    // The header names the first pixel's centre; on a PixelIsArea raster
    // the tiepoint is its corner, half a pixel up and to the left.
    let half = if prof.pixel_is_point { 0.0 } else { 0.5 };
    let x0 = tx + (col0 as f64 - ti + half) * sx;
    let y0 = ty - (row0 as f64 - tj + half) * sy.abs();

    // ── the canonical artifact. ────────────────────────────────────────
    let header = GridHeader {
        width: width_px,
        height: height_px,
        epsg,
        nodata: f32::NAN,
        lat0: y0,
        lng0: x0,
        dlat: -sy.abs(),
        dlng: sx,
        channels: 1,
    };
    let values: Vec<f32> = raw.iter().map(|v| harmonise(*v, dn_offset)).collect();
    let artifact_bytes =
        encode_grid(&header, &values).map_err(|e| upstream_error(format!("grid encode: {e:?}")))?;
    let byte_len = artifact_bytes.len();
    let artifact_cid = ARTIFACTS
        .put(&artifact_bytes)
        .map_err(|e| upstream_error(format!("artifact store: {e}")))?;

    // ── anchors: corners inset one pixel, plus the centre. Best-effort
    //    fact bridge; never materializes. ────────────────────────────────
    let scene_date = item.datetime.get(..10).unwrap_or("").to_string();
    let scene_unix = parse_ymd(&scene_date)
        .map(|(y, m, d)| crate::days_from_civil(y, m, d) * 86_400)
        .unwrap_or(now_unix);
    let tslot = (scene_unix.max(0) as u64) / 86_400;

    let inset_lat = (b.max_lat - b.min_lat) * 0.02;
    let inset_lng = (b.max_lng - b.min_lng) * 0.02;
    let anchor_points = [
        (b.min_lat + inset_lat, b.min_lng + inset_lng),
        (b.min_lat + inset_lat, b.max_lng - inset_lng),
        (b.max_lat - inset_lat, b.min_lng + inset_lng),
        (b.max_lat - inset_lat, b.max_lng - inset_lng),
        (centre_lat, centre_lng),
    ];
    let mut anchors: Vec<JsonValue> = Vec::with_capacity(anchor_points.len());
    let mut anchor_fact_cids: Vec<FactCid> = Vec::new();
    let keys: Vec<CanonicalKey> = anchor_points
        .iter()
        .map(|(lat, lng)| CanonicalKey {
            cell: emem_codec::geo::cell64_from_latlng(*lat, *lng),
            band: band_key.to_string(),
            tslot,
        })
        .collect();
    let looked_up = s
        .storage
        .lookup_canonical_many(&keys)
        .await
        .unwrap_or_else(|_| vec![None; keys.len()]);
    for (((lat, lng), key), existing) in anchor_points.iter().zip(&keys).zip(&looked_up) {
        let utm_a = emem_fetch::proj::latlng_to_utm_with_epsg(*lat, *lng, epsg);
        let (row, col) = match utm_a {
            Some(u) => {
                let (c, r) = prof.world_to_pixel(u.easting, u.northing);
                (
                    ((r - row0).max(0) as u32).min(height_px.saturating_sub(1)),
                    ((c - col0).max(0) as u32).min(width_px.saturating_sub(1)),
                )
            }
            None => (0, 0),
        };
        let value = grid_value_at(&header, &values, row, col);
        if let Some(cid) = existing {
            anchor_fact_cids.push(cid.clone());
        }
        anchors.push(json!({
            "cell": key.cell,
            "row": row,
            "col": col,
            "value": value,
            "fact_cid": existing.as_ref().map(|c| c.as_str()),
        }));
    }

    // ── the derivation record, persisted on the ledger envelope. ──────
    let aoi = aoi_cid(b)?;
    let record_body = json!({
        "schema": "emem.field_derivation.v1",
        "aoi": { "bbox": { "min_lat": b.min_lat, "min_lng": b.min_lng, "max_lat": b.max_lat, "max_lng": b.max_lng } },
        "aoi_cid": aoi,
        "band": band_key,
        "tslot": tslot,
        "fn_key": "band_raster@1",
        "sources": [{
            "scheme": "sentinel2.l2a",
            "id": item.id,
            "asset": crate::s2_url_for_record(&url),
            "catalogue": host,
            "dn_offset": dn_offset,
            "captured_at": item.datetime,
            "cloud_cover": item.cloud_cover,
            // The illumination the scene was acquired under, carried into the
            // signed record so a shadow read out of this raster can be checked
            // against the geometry that cast it. Null on collections that
            // publish no sun.
            "sun_azimuth_deg": item.sun_azimuth,
            "sun_elevation_deg": item.sun_elevation,
            "view_azimuth_deg": item.view_azimuth,
            "view_incidence_deg": item.view_incidence,
        }],
        "artifact": {
            "artifact_cid": artifact_cid,
            "byte_len": byte_len,
            "media_type": GRID_MEDIA_TYPE,
            "grid": {
                "width": width_px, "height": height_px, "epsg": epsg,
                "x0": x0, "y0": y0, "dx": sx, "dy": -sy.abs(),
                "crs_note": "header origin and steps are in the scene's UTM CRS named by epsg; the request bbox is WGS-84",
            },
        },
        "anchors": anchors,
        "anchor_policy": "best_effort: an anchor cites an existing per-cell fact when the store holds one; it never materializes one, so a cold AOI costs no upstream fetches beyond the scene read",
    });
    let signed_at = emem_storage::server::iso8601_now();
    let centre_cell = emem_codec::geo::cell64_from_latlng(centre_lat, centre_lng);
    let derivation_fact = DerivativeFact {
        cell: centre_cell.clone(),
        band: "field.derivation".to_string(),
        tslot_window: [tslot, tslot],
        op: "band_raster".to_string(),
        parents: anchor_fact_cids.clone(),
        value: json_to_cbor(&record_body),
        confidence: 1.0,
        derivation: Derivation {
            fn_key: "band_raster@1".to_string(),
            args: None,
        },
        schema_cid: s.manifests.schema_cid.clone(),
        signer: s.identity.pubkey,
        signed_at: signed_at.clone(),
    };
    let att = emem_fact::Attestation::build_and_sign_v1(
        vec![Fact::Derivative(derivation_fact)],
        vec![],
        s.manifests.registry_cid.clone(),
        s.manifests.schema_cid.clone(),
        &s.identity.signing,
        s.identity.epoch,
        signed_at,
        None,
    )
    .map_err(|e| upstream_error(format!("derivation attestation: {e}")))?;
    let cids = s
        .storage
        .put_attestation(&att)
        .await
        .map_err(ApiError::from)?;
    let derivation_cid = cids
        .first()
        .map(|c| c.as_str().to_string())
        .ok_or_else(|| upstream_error("store returned no cid for the derivation".into()))?;

    // ── the field receipt: (aoi_cid, derivation_cid) in the preimage. ──
    let binding = FieldBinding {
        aoi_cid: aoi.clone(),
        derivation_cid: derivation_cid.clone(),
    };
    let mut receipt_cids = anchor_fact_cids;
    receipt_cids.push(FactCid::new(derivation_cid.clone()));
    let receipt = s.sign_receipt_field(
        "emem.band_raster",
        vec![centre_cell.clone()],
        receipt_cids,
        false,
        started,
        binding,
    );

    Ok(json!({
        "schema": "emem.band_raster.v1",
        "algorithm_key": "band_raster@1",
        "aoi_cid": aoi,
        "band": band_key,
        "tslot": tslot,
        "grid": { "width": width_px, "height": height_px, "epsg": epsg, "native_m": native_m },
        "artifact": {
            "artifact_cid": artifact_cid,
            "byte_len": byte_len,
            "media_type": GRID_MEDIA_TYPE,
            "url": format!("/v1/artifacts/{artifact_cid}"),
            "eviction_note": "the artifact is evictable; the derivation record persists and pins everything needed to rebuild the identical bytes",
        },
        "derivation": record_body,
        "derivation_cid": derivation_cid,
        "tokens": {
            "raster": format!("emem:raster:{aoi}:{band_key}:{tslot}:{derivation_cid}"),
            "derivation_fact": format!("emem:fact:{centre_cell}:{derivation_cid}"),
        },
        "docs": "https://emem.dev/reference#token-raster",
        "receipt": receipt,
    }))
}

/// WB-3: elevation as a field token. Copernicus GLO-30 is a static global
/// COG in EPSG:4326, so unlike the S2 path there is no scene selection and
/// no UTM projection — we resolve the one 1°×1° tile the bbox sits in and
/// read a native-degree window straight out of it. Emits the same
/// `emem:raster:` token/artifact/receipt shape as `band_raster`, under
/// `fn_key: dem_raster@1`.
async fn dem_raster(req: BandRasterReq, s: &AppState) -> Result<JsonValue, ApiError> {
    let started = Instant::now();
    let b = &req.bbox;
    let band_key = "copdem30m.elevation";
    let centre_lat = (b.min_lat + b.max_lat) / 2.0;
    let centre_lng = (b.min_lng + b.max_lng) / 2.0;

    // Single-tile only: the GLO-30 archive is tiled 1°×1°. A bbox that
    // straddles a tile edge needs stitching we don't do yet — refuse
    // clearly rather than silently read one tile. (The AOIs this serves are
    // kilometre-scale, well inside a tile unless placed on a boundary.)
    let tag_lo = emem_fetch::copernicus_dem::tile_corner_tags(b.min_lat, b.min_lng);
    let tag_hi = emem_fetch::copernicus_dem::tile_corner_tags(
        (b.max_lat - 1e-9).max(b.min_lat),
        (b.max_lng - 1e-9).max(b.min_lng),
    );
    if tag_lo != tag_hi {
        return Err(bad_request(format!(
            "bbox crosses a Copernicus GLO-30 DEM tile boundary ({}_{} vs {}_{}); DEM tiles are 1°×1° and this executor reads one tile — shrink the bbox to fit within a single 1° cell, or page it.",
            tag_lo.0, tag_lo.1, tag_hi.0, tag_hi.1
        )));
    }
    let url = emem_fetch::copernicus_dem::tile_url_for(centre_lat, centre_lng);
    let cli = crate::s2_http_client();
    let prof = emem_fetch::cog::open_profile(&cli, &url).await.map_err(|e| {
        upstream_error(format!(
            "open DEM COG {url}: {e}. Over open ocean the archive publishes no tile (404) — there is no elevation field to raster there."
        ))
    })?;

    // EPSG:4326 window math: world coords are degrees, x=lng, y=lat.
    let epsg: u32 = 4326;
    let (sx, sy) = prof.pixel_scale;
    let native_deg = sx.abs();
    if !(native_deg > 0.0 && native_deg.is_finite()) {
        return Err(upstream_error(format!(
            "DEM COG has implausible pixel scale {:?}",
            prof.pixel_scale
        )));
    }
    let width_px = (((b.max_lng - b.min_lng) / native_deg).ceil() as u32).max(1);
    let height_px = (((b.max_lat - b.min_lat) / sy.abs()).ceil() as u32).max(1);
    if width_px > MAX_SIDE_PX || height_px > MAX_SIDE_PX {
        return Err(bad_request(format!(
            "window is {width_px}x{height_px} px at the DEM's native {native_deg}° (~30 m); the cap is {MAX_SIDE_PX} per side. Shrink the bbox."
        )));
    }

    let raw = emem_fetch::cog::sample_window(
        &cli, &url, &prof, centre_lng, centre_lat, width_px, height_px,
    )
    .await
    .map_err(|e| upstream_error(format!("sample_window: {e}")))?;

    let (centre_col, centre_row) = prof.world_to_pixel(centre_lng, centre_lat);
    let col0 = centre_col - (width_px as i64) / 2;
    let row0 = centre_row - (height_px as i64) / 2;
    let (ti, tj, tx, ty) = prof.tiepoint;
    // First pixel's centre. Cop-DEM tiles are PixelIsPoint, already centred.
    let half = if prof.pixel_is_point { 0.0 } else { 0.5 };
    let x0 = tx + (col0 as f64 - ti + half) * sx;
    let y0 = ty - (row0 as f64 - tj + half) * sy.abs();

    let header = GridHeader {
        width: width_px,
        height: height_px,
        epsg,
        nodata: f32::NAN,
        lat0: y0,
        lng0: x0,
        dlat: -sy.abs(),
        dlng: sx,
        channels: 1,
    };
    let values: Vec<f32> = raw.iter().map(|v| *v as f32).collect();
    let artifact_bytes =
        encode_grid(&header, &values).map_err(|e| upstream_error(format!("grid encode: {e:?}")))?;
    let byte_len = artifact_bytes.len();
    let artifact_cid = ARTIFACTS
        .put(&artifact_bytes)
        .map_err(|e| upstream_error(format!("artifact store: {e}")))?;

    // Static band: one canonical vintage, tslot 0. Anchors cite existing
    // per-cell `copdem30m.elevation_mean` facts when the store holds them;
    // never materialises.
    let tslot: u64 = 0;
    let anchor_band = "copdem30m.elevation_mean";
    let inset_lat = (b.max_lat - b.min_lat) * 0.02;
    let inset_lng = (b.max_lng - b.min_lng) * 0.02;
    let anchor_points = [
        (b.min_lat + inset_lat, b.min_lng + inset_lng),
        (b.min_lat + inset_lat, b.max_lng - inset_lng),
        (b.max_lat - inset_lat, b.min_lng + inset_lng),
        (b.max_lat - inset_lat, b.max_lng - inset_lng),
        (centre_lat, centre_lng),
    ];
    let keys: Vec<CanonicalKey> = anchor_points
        .iter()
        .map(|(lat, lng)| CanonicalKey {
            cell: emem_codec::geo::cell64_from_latlng(*lat, *lng),
            band: anchor_band.to_string(),
            tslot,
        })
        .collect();
    let looked_up = s
        .storage
        .lookup_canonical_many(&keys)
        .await
        .unwrap_or_else(|_| vec![None; keys.len()]);
    let mut anchors: Vec<JsonValue> = Vec::with_capacity(anchor_points.len());
    let mut anchor_fact_cids: Vec<FactCid> = Vec::new();
    for (((lat, lng), key), existing) in anchor_points.iter().zip(&keys).zip(&looked_up) {
        let (col, row) = prof.world_to_pixel(*lng, *lat);
        let r = ((row - row0).max(0) as u32).min(header.height.saturating_sub(1));
        let c = ((col - col0).max(0) as u32).min(header.width.saturating_sub(1));
        let value = grid_value_at(&header, &values, r, c);
        if let Some(cid) = existing {
            anchor_fact_cids.push(cid.clone());
        }
        anchors.push(json!({
            "cell": key.cell,
            "row": r,
            "col": c,
            "value": value,
            "fact_cid": existing.as_ref().map(|c| c.as_str()),
        }));
    }

    let aoi = aoi_cid(b)?;
    let record_body = json!({
        "schema": "emem.field_derivation.v1",
        "aoi": { "bbox": { "min_lat": b.min_lat, "min_lng": b.min_lng, "max_lat": b.max_lat, "max_lng": b.max_lng } },
        "aoi_cid": aoi,
        "band": band_key,
        "tslot": tslot,
        "fn_key": "dem_raster@1",
        "sources": [{
            "scheme": "copernicus.dem.glo30",
            "id": url,
            "asset": url,
            "captured_at": "GLO-30 v1 (2021 release)",
        }],
        "artifact": {
            "artifact_cid": artifact_cid,
            "byte_len": byte_len,
            "media_type": GRID_MEDIA_TYPE,
            "grid": {
                "width": width_px, "height": height_px, "epsg": epsg,
                "x0": x0, "y0": y0, "dx": sx, "dy": -sy.abs(),
                "native_deg": native_deg,
                "crs_note": "header origin and steps are in EPSG:4326 degrees (the DEM's native geographic CRS); the request bbox is WGS-84, same datum, so no reprojection",
            },
        },
        "anchors": anchors,
        "anchor_policy": "best_effort: an anchor cites an existing per-cell copdem30m.elevation_mean fact when the store holds one; it never materialises one, so a cold AOI costs no upstream fetch beyond the one tile read",
    });
    let signed_at = emem_storage::server::iso8601_now();
    let centre_cell = emem_codec::geo::cell64_from_latlng(centre_lat, centre_lng);
    let derivation_fact = DerivativeFact {
        cell: centre_cell.clone(),
        band: "field.derivation".to_string(),
        tslot_window: [tslot, tslot],
        op: "dem_raster".to_string(),
        parents: anchor_fact_cids.clone(),
        value: json_to_cbor(&record_body),
        confidence: 1.0,
        derivation: Derivation {
            fn_key: "dem_raster@1".to_string(),
            args: None,
        },
        schema_cid: s.manifests.schema_cid.clone(),
        signer: s.identity.pubkey,
        signed_at: signed_at.clone(),
    };
    let att = emem_fact::Attestation::build_and_sign_v1(
        vec![Fact::Derivative(derivation_fact)],
        vec![],
        s.manifests.registry_cid.clone(),
        s.manifests.schema_cid.clone(),
        &s.identity.signing,
        s.identity.epoch,
        signed_at,
        None,
    )
    .map_err(|e| upstream_error(format!("derivation attestation: {e}")))?;
    let cids = s
        .storage
        .put_attestation(&att)
        .await
        .map_err(ApiError::from)?;
    let derivation_cid = cids
        .first()
        .map(|c| c.as_str().to_string())
        .ok_or_else(|| upstream_error("store returned no cid for the derivation".into()))?;

    let binding = FieldBinding {
        aoi_cid: aoi.clone(),
        derivation_cid: derivation_cid.clone(),
    };
    let mut receipt_cids = anchor_fact_cids;
    receipt_cids.push(FactCid::new(derivation_cid.clone()));
    let receipt = s.sign_receipt_field(
        "emem.band_raster",
        vec![centre_cell.clone()],
        receipt_cids,
        false,
        started,
        binding,
    );

    Ok(json!({
        "schema": "emem.band_raster.v1",
        "algorithm_key": "dem_raster@1",
        "aoi_cid": aoi,
        "band": band_key,
        "tslot": tslot,
        "grid": { "width": width_px, "height": height_px, "epsg": epsg, "native_deg": native_deg },
        "artifact": {
            "artifact_cid": artifact_cid,
            "byte_len": byte_len,
            "media_type": GRID_MEDIA_TYPE,
            "url": format!("/v1/artifacts/{artifact_cid}"),
            "eviction_note": "the artifact is evictable; the derivation record persists and pins the tile, recipe, and geometry needed to rebuild the identical bytes",
        },
        "derivation": record_body,
        "derivation_cid": derivation_cid,
        "tokens": {
            "raster": format!("emem:raster:{aoi}:{band_key}:{tslot}:{derivation_cid}"),
            "derivation_fact": format!("emem:fact:{centre_cell}:{derivation_cid}"),
        },
        "docs": "https://emem.dev/reference#token-raster",
        "receipt": receipt,
    }))
}

/// A static forest layer this executor rasterises straight out of its
/// publisher's tile, with the terms the publisher states for it.
struct ForestBand {
    band: &'static str,
    scheme: &'static str,
    unit: &'static str,
    licence: &'static str,
    attribution: &'static str,
}

const HANSEN_LICENCE: &str = "CC BY 4.0, as the publisher states on its download page";
const HANSEN_ATTRIBUTION: &str = "Hansen, M. C. et al. 2013, High-Resolution Global Maps of 21st-Century Forest Cover Change, Science 342: 850-853. Data: Global Forest Change 2000-2025 v1.13, University of Maryland";
const JRC_LICENCE: &str = "free of charge, without restriction of use, under the Copernicus Regulation of the European Commission, as the publisher states; uses must cite the dataset";

const FOREST_BANDS: [ForestBand; 5] = [
    ForestBand {
        band: "forest_change.lossyear",
        scheme: "hansen.gfc.v1_13.2025",
        unit: "year_of_loss (0 = no loss)",
        licence: HANSEN_LICENCE,
        attribution: HANSEN_ATTRIBUTION,
    },
    ForestBand {
        band: "forest_change.treecover2000",
        scheme: "hansen.gfc.v1_13.2025",
        unit: "percent_canopy_cover",
        licence: HANSEN_LICENCE,
        attribution: HANSEN_ATTRIBUTION,
    },
    ForestBand {
        band: "forest_change.gain",
        scheme: "hansen.gfc.v1_13.2025",
        unit: "boolean_forest_gain_2000_2012",
        licence: HANSEN_LICENCE,
        attribution: HANSEN_ATTRIBUTION,
    },
    ForestBand {
        band: "jrc_gfc2020.forest_2020",
        scheme: "jrc.gfc2020",
        unit: "boolean_eudr_forest_2020",
        licence: JRC_LICENCE,
        attribution: "European Commission, Joint Research Centre: Global map of Forest Cover 2020 (GFC2020)",
    },
    ForestBand {
        band: "jrc_tmf.deforestation_year",
        scheme: "jrc.tmf.v2025",
        unit: "year_of_deforestation (0 = none)",
        licence: JRC_LICENCE,
        attribution: "European Commission, Joint Research Centre: Tropical Moist Forest dataset, 2025 release (Vancutsem et al. 2021, Science Advances 7), read from its Source Cooperative mirror",
    },
];

/// Per side. At 30 m a page is a ~30 km square, at JRC GFC2020's 10 m a
/// ~10 km one, so a district is read as several pages. Twice the S2 cap:
/// the read is one window of a static tile (or of up to four, across a tile
/// line), from the local copy once the tile store holds it.
const FOREST_MAX_SIDE_PX: u32 = 1024;

/// Every forest layer here is published as 10° tiles named by their
/// north-west corner: a tile holds latitudes in `(n - 10, n]` and longitudes
/// in `[w, w + 10)`, as each connector's `tile_corner_tags` names them.
const FOREST_TILE_DEG: f64 = 10.0;

/// A 2 x 2 block of tiles. A bbox under `FOREST_MAX_SIDE_PX` spans well under
/// one tile on every layer here, so it never touches more.
const FOREST_MAX_TILES: usize = 4;

/// How far, in pixels, a tile's grid may sit from the lattice it is joined to.
/// Float error in a published tiepoint is far below this; half a pixel, the
/// offset a PixelIsPoint tile taken as PixelIsArea would show, is far above.
const LATTICE_TOL_PX: f64 = 1e-3;

/// The row of 10° tiles that holds latitude `lat`, numbered by north edge.
fn tile_row_key(lat: f64) -> i64 {
    (lat / FOREST_TILE_DEG).ceil() as i64
}

/// The column of 10° tiles that holds longitude `lng`, numbered by west edge.
fn tile_col_key(lng: f64) -> i64 {
    (lng / FOREST_TILE_DEG).floor() as i64
}

/// Each 10° square the bbox touches, north to south and then west to east,
/// with a point inside both the square and the bbox to name its tile by.
/// Each edge is first moved 1e-9° inward, so a bbox that ends on a tile line
/// does not count the square beyond it.
fn forest_squares(b: &BBox) -> Vec<((i64, i64), (f64, f64))> {
    let top = (b.max_lat - 1e-9).max(b.min_lat);
    let bottom = (b.min_lat + 1e-9).min(b.max_lat);
    let left = (b.min_lng + 1e-9).min(b.max_lng);
    let right = (b.max_lng - 1e-9).max(b.min_lng);
    // On a bbox under 2e-9° the nudged edges cross; either order holds the centre.
    let (south, north) = (bottom.min(top), bottom.max(top));
    let (west, east) = (left.min(right), left.max(right));
    let mut out = Vec::new();
    for ky in (tile_row_key(south)..=tile_row_key(north)).rev() {
        let lat_lo = south.max((ky - 1) as f64 * FOREST_TILE_DEG);
        let lat_hi = north.min(ky as f64 * FOREST_TILE_DEG);
        for kx in tile_col_key(west)..=tile_col_key(east) {
            let lng_lo = west.max(kx as f64 * FOREST_TILE_DEG);
            let lng_hi = east.min((kx + 1) as f64 * FOREST_TILE_DEG);
            out.push(((ky, kx), ((lat_lo + lat_hi) / 2.0, (lng_lo + lng_hi) / 2.0)));
        }
    }
    out
}

/// One tile of a forest layer: its 10° square, its URL, and JRC GFC2020's
/// version.
struct ForestTile {
    key: (i64, i64),
    url: String,
    jrc_version: Option<u32>,
}

/// A rectangle of a forest window that one tile supplies, in window pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Piece {
    /// Index of the tile in the caller's list.
    tile: usize,
    row0: u32,
    col0: u32,
    height: u32,
    width: u32,
}

/// Runs of equal keys over `0..n`, as `(key, start, length)`.
fn key_runs(n: u32, key: impl Fn(i64) -> i64) -> Vec<(i64, u32, u32)> {
    let mut out: Vec<(i64, u32, u32)> = Vec::new();
    for i in 0..n {
        let k = key(i64::from(i));
        match out.last_mut() {
            Some(run) if run.0 == k => run.2 += 1,
            _ => out.push((k, i, 1)),
        }
    }
    out
}

/// Splits the `w` x `h` window whose first pixel is (`col0`, `row0`) on
/// `prof`'s lattice into the rectangle each tile in `keys` supplies. A pixel
/// goes to the tile whose square holds its centre, clamped to the squares in
/// `keys`: a pixel past a line the bbox only grazes stays with a tile that is
/// read, and is NaN there if that tile does not hold it, as on one tile.
fn plan_pieces(
    prof: &emem_fetch::cog::CogProfile,
    col0: i64,
    row0: i64,
    w: u32,
    h: u32,
    keys: &[(i64, i64)],
) -> Result<Vec<Piece>, String> {
    let rows = keys.iter().map(|k| k.0);
    let cols = keys.iter().map(|k| k.1);
    let (Some(ky_lo), Some(ky_hi), Some(kx_lo), Some(kx_hi)) = (
        rows.clone().min(),
        rows.max(),
        cols.clone().min(),
        cols.max(),
    ) else {
        return Err("no tile to read the window from".into());
    };
    let (ti, tj, tx, ty) = prof.tiepoint;
    let (sx, sy) = prof.pixel_scale;
    let half = if prof.pixel_is_point { 0.0 } else { 0.5 };
    let row_runs = key_runs(h, |r| {
        tile_row_key(ty - ((row0 + r) as f64 - tj + half) * sy.abs()).clamp(ky_lo, ky_hi)
    });
    let col_runs = key_runs(w, |c| {
        tile_col_key(tx + ((col0 + c) as f64 - ti + half) * sx).clamp(kx_lo, kx_hi)
    });
    let mut pieces = Vec::with_capacity(row_runs.len() * col_runs.len());
    for &(ky, r, height) in &row_runs {
        for &(kx, c, width) in &col_runs {
            let tile = keys
                .iter()
                .position(|k| *k == (ky, kx))
                .ok_or_else(|| format!("no tile is named for the 10° square ({ky}, {kx})"))?;
            pieces.push(Piece {
                tile,
                row0: r,
                col0: c,
                height,
                width,
            });
        }
    }
    Ok(pieces)
}

/// The world position of the north-west corner of pixel (0, 0).
fn grid_corner(p: &emem_fetch::cog::CogProfile) -> (f64, f64) {
    let (ti, tj, tx, ty) = p.tiepoint;
    let (sx, sy) = p.pixel_scale;
    // On a PixelIsPoint raster the tiepoint names a pixel's centre.
    let shift = if p.pixel_is_point { 0.5 } else { 0.0 };
    (tx - (ti + shift) * sx, ty + (tj + shift) * sy.abs())
}

/// The whole-pixel offset `(dc, dr)` of `piece`'s grid on `reference`'s:
/// reference pixel `(c, r)` is piece pixel `(c - dc, r - dr)`. Two grids
/// that are not one lattice (another pixel size, CRS, or a fractional
/// offset) are refused: joining them would need resampling.
fn lattice_offset(
    reference: &emem_fetch::cog::CogProfile,
    piece: &emem_fetch::cog::CogProfile,
) -> Result<(i64, i64), String> {
    let (rsx, rsy) = (reference.pixel_scale.0, reference.pixel_scale.1.abs());
    let (psx, psy) = (piece.pixel_scale.0, piece.pixel_scale.1.abs());
    if !(rsx > 0.0 && rsy > 0.0 && psx > 0.0 && psy > 0.0) {
        return Err(format!(
            "pixel sizes {psx} x {psy} and {rsx} x {rsy} are not both positive"
        ));
    }
    // A size difference that drifts the grids apart across the piece's tile.
    let span = f64::from(piece.width.max(piece.height));
    if (psx - rsx).abs() * span / rsx > LATTICE_TOL_PX
        || (psy - rsy).abs() * span / rsy > LATTICE_TOL_PX
    {
        return Err(format!("pixel size {psx} x {psy} against {rsx} x {rsy}"));
    }
    if piece.epsg != reference.epsg {
        return Err(format!(
            "EPSG {:?} against {:?}",
            piece.epsg, reference.epsg
        ));
    }
    let (rx, ry) = grid_corner(reference);
    let (px, py) = grid_corner(piece);
    let (dc, dr) = ((px - rx) / rsx, (ry - py) / rsy);
    if (dc - dc.round()).abs() > LATTICE_TOL_PX || (dr - dr.round()).abs() > LATTICE_TOL_PX {
        return Err(format!(
            "the grids are {dc} columns and {dr} rows apart, not a whole number of pixels"
        ));
    }
    Ok((dc.round() as i64, dr.round() as i64))
}

/// A forest tile's sample as the per-cell fact states it: the lossyear byte
/// as its calendar year, every other layer as read.
fn forest_value(band_key: &str, v: f64) -> f32 {
    if band_key == "forest_change.lossyear" {
        let byte = if (0.0..=255.0).contains(&v) {
            v.round() as u8
        } else {
            0
        };
        emem_fetch::hansen_gfc::lossyear_byte_to_calendar_year(byte).unwrap_or(0) as f32
    } else {
        v as f32
    }
}

/// The values of `piece`, read from `tile` at the whole-pixel offset its grid
/// sits at on `reference`'s, and the piece's first (column, row) in that
/// tile. `origin` is the window's first pixel on the reference lattice.
async fn read_forest_piece(
    cli: &reqwest::Client,
    band_key: &str,
    reference: &emem_fetch::cog::CogProfile,
    reference_url: &str,
    tile: &ForestTile,
    origin: (i64, i64),
    piece: &Piece,
) -> Result<(Vec<f32>, (i64, i64)), ApiError> {
    let (width, height) = (piece.width, piece.height);
    let at = (
        origin.0 + i64::from(piece.col0),
        origin.1 + i64::from(piece.row0),
    );
    let url = tile.url.as_str();
    let prof = emem_fetch::cog::open_profile(cli, url)
        .await
        .map_err(|e| upstream_error(format!("open {url}: {e}")))?;
    let (dc, dr) = lattice_offset(reference, &prof).map_err(|why| {
        upstream_error(format!(
            "{url} is not on the pixel lattice of {reference_url} ({why}). Joining them would need resampling, which this executor does not do. Page the bbox along the tile line."
        ))
    })?;
    let (col0, row0) = (at.0 - dc, at.1 - dr);
    let (span_w, span_h) = (i64::from(width), i64::from(height));
    let (ti, tj, tx, ty) = prof.tiepoint;
    let (sx, sy) = prof.pixel_scale;
    let half = if prof.pixel_is_point { 0.0 } else { 0.5 };
    // sample_window places its window at the pixel under the point it is
    // given, less half the size: hand it the centre of the middle pixel.
    let cx = tx + ((col0 + span_w / 2) as f64 - ti + half) * sx;
    let cy = ty - ((row0 + span_h / 2) as f64 - tj + half) * sy.abs();
    let raw = emem_fetch::cog::sample_window(cli, url, &prof, cx, cy, width, height)
        .await
        .map_err(|e| upstream_error(format!("sample_window {url}: {e}")))?;
    let in_tile = |k: usize| {
        let (r, c) = (
            row0 + (k / width as usize) as i64,
            col0 + (k % width as usize) as i64,
        );
        (0..prof.height as i64).contains(&r) && (0..prof.width as i64).contains(&c)
    };
    // sample_window fills a pixel outside the image with 0, and 0 is a real
    // value of every forest layer.
    let values = raw
        .iter()
        .enumerate()
        .map(|(k, v)| {
            if in_tile(k) {
                forest_value(band_key, *v)
            } else {
                f32::NAN
            }
        })
        .collect();
    Ok((values, (col0, row0)))
}

/// The tile of `band` that covers `(lat, lng)`, and JRC GFC2020's version.
/// A point the layer does not cover is the caller's to know about (a 400:
/// TMF is published inside the tropics only), not an upstream failure.
async fn forest_tile_url(
    band: &str,
    lat: f64,
    lng: f64,
    cli: &reqwest::Client,
) -> Result<(String, Option<u32>), ApiError> {
    if band == "jrc_gfc2020.forest_2020" {
        return emem_fetch::jrc_gfc2020::tile_url(cli, lat, lng)
            .await
            .map(|(u, v)| (u, Some(v)))
            .map_err(|e| upstream_error(format!("JRC GFC2020 tile listing: {e}")));
    }
    crate::static_cog_url_for_band(band, lat, lng)
        .map(|u| (u, None))
        .ok_or_else(|| {
            bad_request(format!(
                "{band} publishes no tile at ({lat}, {lng}); the point is outside the layer's coverage"
            ))
        })
}

/// The year the per-cell builders sign these layers at (`build_fact_hansen`,
/// `jrc_tmf_fact`), so an anchor finds the fact recall signed for its cell.
pub(crate) const FOREST_RELEASE_YEAR: i32 = 2025;

/// A forest layer as a FIELD token: one window of the layer over the bbox,
/// signed as a derivation, where callers had to read the layer cell by cell.
/// Values are the per-cell facts' values (the lossyear byte is decoded to its
/// calendar year), so an anchor checks exactly against the fact for its cell.
/// A bbox across a tile line is read from every tile it touches, up to
/// `FOREST_MAX_TILES`, and joined on the layer's own pixel lattice.
async fn forest_raster(req: BandRasterReq, s: &AppState) -> Result<JsonValue, ApiError> {
    let started = Instant::now();
    let b = &req.bbox;
    let Some(fb) = FOREST_BANDS.iter().find(|f| f.band == req.band) else {
        return Err(bad_request(format!(
            "band `{}` is not a forest layer",
            req.band
        )));
    };
    let band_key = fb.band;
    let squares = forest_squares(b);
    if squares.len() > FOREST_MAX_TILES {
        return Err(bad_request(format!(
            "bbox touches {} of the {band_key} tiles (10° squares); one raster joins at most {FOREST_MAX_TILES}, a 2 x 2 block. Page the bbox.",
            squares.len()
        )));
    }
    let centre_lat = (b.min_lat + b.max_lat) / 2.0;
    let centre_lng = (b.min_lng + b.max_lng) / 2.0;
    let cli = crate::s2_http_client();
    if squares.len() == 1 {
        let (url, jrc_version) = forest_tile_url(band_key, centre_lat, centre_lng, &cli).await?;
        return forest_raster_from(&req, s, fb, &url, jrc_version, started).await;
    }
    let mut tiles = Vec::with_capacity(squares.len());
    for (key, (lat, lng)) in squares {
        let (url, jrc_version) = forest_tile_url(band_key, lat, lng, &cli).await?;
        tiles.push(ForestTile {
            key,
            url,
            jrc_version,
        });
    }
    // The lattice is the one the bbox centre's tile is on, as on one tile.
    let centre_key = (tile_row_key(centre_lat), tile_col_key(centre_lng));
    let reference = tiles
        .iter()
        .position(|t| t.key == centre_key)
        .ok_or_else(|| {
            upstream_error(format!(
                "no {band_key} tile is named for the bbox centre ({centre_lat}, {centre_lng})"
            ))
        })?;
    forest_raster_tiles(&req, s, fb, &tiles, reference, started).await
}

/// [`forest_raster`] once the tile is chosen, for a bbox inside one tile.
async fn forest_raster_from(
    req: &BandRasterReq,
    s: &AppState,
    fb: &ForestBand,
    url: &str,
    jrc_version: Option<u32>,
    started: Instant,
) -> Result<JsonValue, ApiError> {
    let b = &req.bbox;
    let tile = ForestTile {
        key: (
            tile_row_key((b.min_lat + b.max_lat) / 2.0),
            tile_col_key((b.min_lng + b.max_lng) / 2.0),
        ),
        url: url.to_string(),
        jrc_version,
    };
    forest_raster_tiles(req, s, fb, std::slice::from_ref(&tile), 0, started).await
}

/// The forest window over the bbox, on the pixel lattice of
/// `tiles[reference]`, read from each tile in `tiles` that holds a part of it
/// and signed once. One tile gives the same bytes and record as a read that
/// never joins; several give one grid whose record cites each tile and the
/// rectangle it supplied.
async fn forest_raster_tiles(
    req: &BandRasterReq,
    s: &AppState,
    fb: &ForestBand,
    tiles: &[ForestTile],
    reference: usize,
    started: Instant,
) -> Result<JsonValue, ApiError> {
    let b = &req.bbox;
    let band_key = fb.band;
    let centre_lat = (b.min_lat + b.max_lat) / 2.0;
    let centre_lng = (b.min_lng + b.max_lng) / 2.0;
    let cli = crate::s2_http_client();
    let lattice_tile = tiles
        .get(reference)
        .ok_or_else(|| upstream_error("no tile to read the window from".into()))?;
    let url = lattice_tile.url.clone();
    let prof = emem_fetch::cog::open_profile(&cli, &url)
        .await
        .map_err(|e| upstream_error(format!("open {url}: {e}")))?;
    let epsg: u32 = 4326;
    let (sx, sy) = prof.pixel_scale;
    let native_deg = sx.abs();
    if !(native_deg > 0.0 && native_deg.is_finite() && sy.abs() > 0.0) {
        return Err(upstream_error(format!(
            "{url} has an implausible pixel scale {:?}",
            prof.pixel_scale
        )));
    }
    // The window is every pixel the bbox touches, fitted to its edges with
    // the snapping `pixel_index` uses. Placed by the bbox centre and a
    // rounded-up size, it missed a row or column of up to half the pixels of
    // a plot two to four pixels wide.
    let (ti, tj, tx, ty) = prof.tiepoint;
    let half = if prof.pixel_is_point { 0.0 } else { 0.5 };
    let shift = 0.5 - half;
    const EPS: f64 = 1e-6;
    let col_a = ti + (b.min_lng - tx) / sx + shift;
    let col_b = ti + (b.max_lng - tx) / sx + shift;
    let row_a = tj + (ty - b.max_lat) / sy.abs() + shift;
    let row_b = tj + (ty - b.min_lat) / sy.abs() + shift;
    let col0 = (col_a + EPS).floor() as i64;
    let row0 = (row_a + EPS).floor() as i64;
    let col1 = ((col_b - EPS).ceil() as i64).max(col0 + 1);
    let row1 = ((row_b - EPS).ceil() as i64).max(row0 + 1);
    let (span_w, span_h) = (col1 - col0, row1 - row0);
    if span_w > FOREST_MAX_SIDE_PX as i64 || span_h > FOREST_MAX_SIDE_PX as i64 {
        return Err(bad_request(format!(
            "window is {span_w}x{span_h} px at {band_key}'s native {native_deg}°; the cap is {FOREST_MAX_SIDE_PX} per side. Page the bbox."
        )));
    }
    let (width_px, height_px) = (span_w as u32, span_h as u32);
    let keys: Vec<(i64, i64)> = tiles.iter().map(|t| t.key).collect();
    let pieces =
        plan_pieces(&prof, col0, row0, width_px, height_px, &keys).map_err(upstream_error)?;
    let reads = pieces.iter().map(|p| {
        let cli = &cli;
        let prof = &prof;
        let url = &url;
        async move {
            let tile = tiles
                .get(p.tile)
                .ok_or_else(|| upstream_error("a piece names no tile".into()))?;
            read_forest_piece(cli, band_key, prof, url, tile, (col0, row0), p).await
        }
    });
    let read = futures_util::future::try_join_all(reads).await?;
    let mut values = vec![f32::NAN; width_px as usize * height_px as usize];
    for (p, (piece_values, _)) in pieces.iter().zip(&read) {
        for (i, row) in piece_values.chunks(p.width.max(1) as usize).enumerate() {
            let at = (p.row0 as usize + i) * width_px as usize + p.col0 as usize;
            if let Some(dst) = values.get_mut(at..at + row.len()) {
                dst.copy_from_slice(row);
            }
        }
    }

    let x0 = tx + (col0 as f64 - ti + half) * sx;
    let y0 = ty - (row0 as f64 - tj + half) * sy.abs();
    let header = GridHeader {
        width: width_px,
        height: height_px,
        epsg,
        nodata: f32::NAN,
        lat0: y0,
        lng0: x0,
        dlat: -sy.abs(),
        dlng: sx,
        channels: 1,
    };
    let artifact_bytes =
        encode_grid(&header, &values).map_err(|e| upstream_error(format!("grid encode: {e:?}")))?;
    let byte_len = artifact_bytes.len();
    let artifact_cid = ARTIFACTS
        .put(&artifact_bytes)
        .map_err(|e| upstream_error(format!("artifact store: {e}")))?;

    // Anchors at the centres of five cells, the point each cell's own fact
    // was read at, so an anchor and its fact name the same pixel.
    let tslot: u64 = 0;
    let inset_lat = (b.max_lat - b.min_lat) * 0.02;
    let inset_lng = (b.max_lng - b.min_lng) * 0.02;
    let cells: Vec<String> = [
        (b.min_lat + inset_lat, b.min_lng + inset_lng),
        (b.min_lat + inset_lat, b.max_lng - inset_lng),
        (b.max_lat - inset_lat, b.min_lng + inset_lng),
        (b.max_lat - inset_lat, b.max_lng - inset_lng),
        (centre_lat, centre_lng),
    ]
    .iter()
    .map(|(lat, lng)| emem_codec::geo::cell64_from_latlng(*lat, *lng))
    .collect();
    // Recall signs Hansen and TMF cells at the release's slot; GFC2020 and
    // the windowed EUDR read sign at 0. The record keeps 0, as the band's
    // one vintage, and an anchor cites whichever its cell has.
    let release_tslot = crate::tslot_for_year(FOREST_RELEASE_YEAR, band_key);
    let keys: Vec<CanonicalKey> = cells
        .iter()
        .flat_map(|cell| {
            [release_tslot, tslot].map(|t| CanonicalKey {
                cell: cell.clone(),
                band: band_key.to_string(),
                tslot: t,
            })
        })
        .collect();
    let looked_up: Vec<Option<FactCid>> = s
        .storage
        .lookup_canonical_many(&keys)
        .await
        .unwrap_or_else(|_| vec![None; keys.len()])
        .chunks(2)
        .map(|pair| pair.iter().flatten().next().cloned())
        .collect();
    let mut anchors: Vec<JsonValue> = Vec::with_capacity(cells.len());
    let mut anchor_fact_cids: Vec<FactCid> = Vec::new();
    for (cell, existing) in cells.iter().zip(&looked_up) {
        let Ok(at) = emem_codec::latlng_from_cell64(cell) else {
            continue;
        };
        let (col, row) = prof.world_to_pixel(at.lng_deg, at.lat_deg);
        let (r, c) = (row - row0, col - col0);
        // An anchor cites the pixel its cell is in, or nothing: clamped to
        // the edge, it named a pixel its fact was never read at.
        if !((0..height_px as i64).contains(&r) && (0..width_px as i64).contains(&c)) {
            continue;
        }
        let (r, c) = (r as u32, c as u32);
        if let Some(cid) = existing {
            anchor_fact_cids.push(cid.clone());
        }
        anchors.push(json!({
            "cell": cell,
            "row": r,
            "col": c,
            "value": grid_value_at(&header, &values, r, c),
            "fact_cid": existing.as_ref().map(|c| c.as_str()),
        }));
    }

    let scheme_of = |jrc_version: Option<u32>| match jrc_version {
        Some(v) => format!("{}.v{v}", fb.scheme),
        None => fb.scheme.to_string(),
    };
    let licence = json!({ "terms": fb.licence, "attribution": fb.attribution });
    let one_tile = matches!(pieces.as_slice(), [p] if p.tile == reference);
    let sources: Vec<JsonValue> = if one_tile {
        vec![json!({
            "scheme": scheme_of(lattice_tile.jrc_version),
            "id": url,
            "asset": url,
            "licence": licence,
        })]
    } else {
        pieces
            .iter()
            .zip(&read)
            .filter_map(|(p, (_, (tile_col0, tile_row0)))| {
                let t = tiles.get(p.tile)?;
                Some(json!({
                    "scheme": scheme_of(t.jrc_version),
                    "id": t.url,
                    "asset": t.url,
                    "licence": licence,
                    "window": {
                        "grid_row0": p.row0, "grid_col0": p.col0,
                        "height": p.height, "width": p.width,
                        "tile_row0": tile_row0, "tile_col0": tile_col0,
                    },
                }))
            })
            .collect()
    };
    let aoi = aoi_cid(b)?;
    let mut record_body = json!({
        "schema": "emem.field_derivation.v1",
        "aoi": { "bbox": { "min_lat": b.min_lat, "min_lng": b.min_lng, "max_lat": b.max_lat, "max_lng": b.max_lng } },
        "aoi_cid": aoi,
        "band": band_key,
        "tslot": tslot,
        "fn_key": "forest_raster@1",
        "unit": fb.unit,
        "sources": sources,
        "artifact": {
            "artifact_cid": artifact_cid,
            "byte_len": byte_len,
            "media_type": GRID_MEDIA_TYPE,
            "grid": {
                "width": width_px, "height": height_px, "epsg": epsg,
                "x0": x0, "y0": y0, "dx": sx, "dy": -sy.abs(),
                "native_deg": native_deg,
                "crs_note": "header origin and steps are in EPSG:4326 degrees, the tile's own CRS; the request bbox is WGS-84, same datum, so no reprojection",
            },
        },
        "anchors": anchors,
        "anchor_policy": "best_effort: an anchor cites the per-cell fact of the same band for its cell when the store holds one, and never materialises one",
    });
    if !one_tile {
        if let Some(o) = record_body.as_object_mut() {
            o.insert(
                "mosaic".into(),
                json!({
                    "pieces": pieces.len(),
                    "lattice_of": url,
                    "rule": "each pixel is read from the tile of the 10-degree square that holds its centre, among the squares the bbox touches; each source's window says where its rectangle sits in the grid and in its tile. Every tile is on lattice_of's pixel lattice at a whole-pixel offset, so no pixel is resampled",
                }),
            );
        }
    }
    let signed_at = emem_storage::server::iso8601_now();
    let centre_cell = emem_codec::geo::cell64_from_latlng(centre_lat, centre_lng);
    let derivation_fact = DerivativeFact {
        cell: centre_cell.clone(),
        band: "field.derivation".to_string(),
        tslot_window: [tslot, tslot],
        op: "forest_raster".to_string(),
        parents: anchor_fact_cids.clone(),
        value: json_to_cbor(&record_body),
        confidence: 1.0,
        derivation: Derivation {
            fn_key: "forest_raster@1".to_string(),
            args: None,
        },
        schema_cid: s.manifests.schema_cid.clone(),
        signer: s.identity.pubkey,
        signed_at: signed_at.clone(),
    };
    let att = emem_fact::Attestation::build_and_sign_v1(
        vec![Fact::Derivative(derivation_fact)],
        vec![],
        s.manifests.registry_cid.clone(),
        s.manifests.schema_cid.clone(),
        &s.identity.signing,
        s.identity.epoch,
        signed_at,
        None,
    )
    .map_err(|e| upstream_error(format!("derivation attestation: {e}")))?;
    let cids = s
        .storage
        .put_attestation(&att)
        .await
        .map_err(ApiError::from)?;
    let derivation_cid = cids
        .first()
        .map(|c| c.as_str().to_string())
        .ok_or_else(|| upstream_error("store returned no cid for the derivation".into()))?;
    let binding = FieldBinding {
        aoi_cid: aoi.clone(),
        derivation_cid: derivation_cid.clone(),
    };
    let mut receipt_cids = anchor_fact_cids;
    receipt_cids.push(FactCid::new(derivation_cid.clone()));
    let receipt = s.sign_receipt_field(
        "emem.band_raster",
        vec![centre_cell.clone()],
        receipt_cids,
        false,
        started,
        binding,
    );
    Ok(json!({
        "schema": "emem.band_raster.v1",
        "algorithm_key": "forest_raster@1",
        "aoi_cid": aoi,
        "band": band_key,
        "unit": fb.unit,
        "tslot": tslot,
        "grid": { "width": width_px, "height": height_px, "epsg": epsg, "native_deg": native_deg },
        "licence": licence,
        "artifact": {
            "artifact_cid": artifact_cid,
            "byte_len": byte_len,
            "media_type": GRID_MEDIA_TYPE,
            "url": format!("/v1/artifacts/{artifact_cid}"),
            "eviction_note": "the artifact is evictable; the derivation record persists and pins the tile, recipe, and geometry needed to rebuild the identical bytes",
        },
        "derivation": record_body,
        "derivation_cid": derivation_cid,
        "tokens": {
            "raster": format!("emem:raster:{aoi}:{band_key}:{tslot}:{derivation_cid}"),
            "derivation_fact": format!("emem:fact:{centre_cell}:{derivation_cid}"),
        },
        "docs": "https://emem.dev/reference#token-raster",
        "receipt": receipt,
    }))
}

/// WB-5: an encoder embedding as a FIELD token. The worlds agent's
/// `embedding.json` is a client-side joint-PCA over ~1,024 per-cell
/// geotessera reads; this replaces that with one signed artifact. It samples
/// the encoder band over a grid at its native ~0.1° resolution, packs each
/// cell's N-D vector into a multi-channel `emem:raster:` artifact (channels =
/// the encoder dim), and signs it under `embedding_raster@1`. The recalled
/// per-cell facts ARE the anchors, so every column of the field terminates in
/// a real signed geotessera fact — lineage a stranger can walk.
async fn embedding_raster(req: BandRasterReq, s: &AppState) -> Result<JsonValue, ApiError> {
    let started = Instant::now();
    let b = &req.bbox;
    let band = req.band.clone();
    let step = EMBEDDING_STEP_DEG;
    let width = (((b.max_lng - b.min_lng) / step).ceil() as u32).max(1);
    let height = (((b.max_lat - b.min_lat) / step).ceil() as u32).max(1);
    if width * height > MAX_EMBEDDING_CELLS {
        return Err(bad_request(format!(
            "embedding field is {width}×{height} = {} cells at the encoder's native {step}°; the cap is {MAX_EMBEDDING_CELLS}. Shrink the bbox or page it.",
            width * height
        )));
    }

    // Grid cell centres, north row first (row 0 = max_lat).
    let mut points: Vec<(u32, u32, String)> = Vec::with_capacity((width * height) as usize);
    for row in 0..height {
        for col in 0..width {
            let lat = b.max_lat - (row as f64 + 0.5) * step;
            let lng = b.min_lng + (col as f64 + 0.5) * step;
            points.push((row, col, emem_codec::geo::cell64_from_latlng(lat, lng)));
        }
    }

    // Fan out encoder recalls in bounded-concurrency chunks so a cold field
    // doesn't open hundreds of upstream range reads at once.
    let mut vectors: std::collections::HashMap<(u32, u32), (Vec<f32>, String)> =
        std::collections::HashMap::new();
    for chunk in points.chunks(16) {
        let futs = chunk.iter().map(|(row, col, cell)| {
            let band = band.clone();
            let cell = cell.clone();
            async move {
                let rq = emem_primitives::recall::RecallReq {
                    cell,
                    bands: Some(vec![band.clone()]),
                    ..Default::default()
                };
                let res = crate::recall_with_auto_materialize(&rq, s).await;
                (*row, *col, res)
            }
        });
        for (row, col, res) in futures_util::future::join_all(futs).await {
            let Ok((resp, _)) = res else { continue };
            let vec = resp.facts.iter().find_map(|f| match f {
                Fact::Primary(p) if p.band == band => {
                    emem_primitives::cbor_ops::as_vec_f32(&p.value)
                }
                _ => None,
            });
            if let Some(v) = vec {
                let cid = resp
                    .receipt
                    .fact_cids
                    .first()
                    .map(|c| c.0.clone())
                    .unwrap_or_default();
                vectors.insert((row, col), (v, cid));
            }
        }
    }

    let dim = vectors.values().map(|(v, _)| v.len()).max().unwrap_or(0);
    if dim == 0 {
        return Err(upstream_error(format!(
            "no `{band}` embedding materialised at any of the {} grid cells in this bbox (the encoder publishes no vector here)",
            width * height
        )));
    }

    // Pack width×height×dim, pixel-interleaved, NaN-filling absent or
    // wrong-length cells; collect the anchor fact_cids that back real cells.
    let mut values = vec![f32::NAN; (width as usize) * (height as usize) * dim];
    let mut anchor_fact_cids: Vec<FactCid> = Vec::new();
    let mut filled = 0usize;
    for ((row, col), (v, cid)) in &vectors {
        if v.len() != dim {
            continue;
        }
        let base = ((*row as usize) * (width as usize) + *col as usize) * dim;
        values[base..base + dim].copy_from_slice(v);
        filled += 1;
        if !cid.is_empty() {
            anchor_fact_cids.push(FactCid::new(cid.clone()));
        }
    }

    let header = GridHeader {
        width,
        height,
        epsg: 4326,
        nodata: f32::NAN,
        lat0: b.max_lat - 0.5 * step,
        lng0: b.min_lng + 0.5 * step,
        dlat: -step,
        dlng: step,
        channels: dim as u32,
    };
    let artifact_bytes =
        encode_grid(&header, &values).map_err(|e| upstream_error(format!("grid encode: {e:?}")))?;
    let byte_len = artifact_bytes.len();
    let artifact_cid = ARTIFACTS
        .put(&artifact_bytes)
        .map_err(|e| upstream_error(format!("artifact store: {e}")))?;

    let tslot: u64 = 0;
    let centre_lat = (b.min_lat + b.max_lat) / 2.0;
    let centre_lng = (b.min_lng + b.max_lng) / 2.0;
    let aoi = aoi_cid(b)?;
    let record_body = json!({
        "schema": "emem.field_derivation.v1",
        "aoi": { "bbox": { "min_lat": b.min_lat, "min_lng": b.min_lng, "max_lat": b.max_lat, "max_lng": b.max_lng } },
        "aoi_cid": aoi,
        "band": band,
        "tslot": tslot,
        "fn_key": "embedding_raster@1",
        "embedding": {
            "channels": dim,
            "cells_filled": filled,
            "cells_total": width * height,
            "encoder": band,
            "layout": "pixel-interleaved f32; each pixel carries the encoder's dim-length vector; absent cells are all-NaN",
            "sampling": format!("encoder native {step}° grid; one recall per cell, auto-materialised and signed"),
        },
        "artifact": {
            "artifact_cid": artifact_cid,
            "byte_len": byte_len,
            "media_type": GRID_MEDIA_TYPE,
            "grid": {
                "width": width, "height": height, "channels": dim, "epsg": 4326,
                "x0": header.lng0, "y0": header.lat0, "dx": step, "dy": -step,
                "crs_note": "EPSG:4326 degrees; channels axis is the encoder embedding dimension",
            },
        },
        "anchor_count": anchor_fact_cids.len(),
        "anchor_policy": "every filled cell IS a signed per-cell encoder fact (the recall that filled it); the field's columns terminate in real facts, and all anchor fact_cids enter the receipt preimage",
    });
    let signed_at = emem_storage::server::iso8601_now();
    let centre_cell = emem_codec::geo::cell64_from_latlng(centre_lat, centre_lng);
    let derivation_fact = DerivativeFact {
        cell: centre_cell.clone(),
        band: "field.derivation".to_string(),
        tslot_window: [tslot, tslot],
        op: "embedding_raster".to_string(),
        parents: anchor_fact_cids.clone(),
        value: json_to_cbor(&record_body),
        confidence: 1.0,
        derivation: Derivation {
            fn_key: "embedding_raster@1".to_string(),
            args: None,
        },
        schema_cid: s.manifests.schema_cid.clone(),
        signer: s.identity.pubkey,
        signed_at: signed_at.clone(),
    };
    let att = emem_fact::Attestation::build_and_sign_v1(
        vec![Fact::Derivative(derivation_fact)],
        vec![],
        s.manifests.registry_cid.clone(),
        s.manifests.schema_cid.clone(),
        &s.identity.signing,
        s.identity.epoch,
        signed_at,
        None,
    )
    .map_err(|e| upstream_error(format!("derivation attestation: {e}")))?;
    let cids = s
        .storage
        .put_attestation(&att)
        .await
        .map_err(ApiError::from)?;
    let derivation_cid = cids
        .first()
        .map(|c| c.as_str().to_string())
        .ok_or_else(|| upstream_error("store returned no cid for the derivation".into()))?;

    let binding = FieldBinding {
        aoi_cid: aoi.clone(),
        derivation_cid: derivation_cid.clone(),
    };
    let mut receipt_cids = anchor_fact_cids;
    receipt_cids.push(FactCid::new(derivation_cid.clone()));
    let receipt = s.sign_receipt_field(
        "emem.band_raster",
        vec![centre_cell.clone()],
        receipt_cids,
        false,
        started,
        binding,
    );

    Ok(json!({
        "schema": "emem.band_raster.v1",
        "algorithm_key": "embedding_raster@1",
        "aoi_cid": aoi,
        "band": band,
        "tslot": tslot,
        "grid": { "width": width, "height": height, "channels": dim, "epsg": 4326, "native_deg": step },
        "artifact": {
            "artifact_cid": artifact_cid,
            "byte_len": byte_len,
            "media_type": GRID_MEDIA_TYPE,
            "url": format!("/v1/artifacts/{artifact_cid}"),
            "eviction_note": "the artifact is evictable; the derivation record pins the encoder, grid, and anchor facts needed to rebuild identical bytes",
        },
        "derivation": record_body,
        "derivation_cid": derivation_cid,
        "tokens": {
            "raster": format!("emem:raster:{aoi}:{band}:{tslot}:{derivation_cid}"),
            "derivation_fact": format!("emem:fact:{centre_cell}:{derivation_cid}"),
        },
        "docs": "https://emem.dev/reference#token-raster",
        "receipt": receipt,
    }))
}

// ── emem:rasterset: — a signed manifest over N field tokens. ────────────────
//
// The composition primitive the fly-through DDS needs: bind an already-minted
// set of `emem:raster:` field tokens (RGB ground + DEM geometry + embedding +
// composite layers) into ONE citeable token whose content-address is the
// ordered membership. Unlike `band_cube` (which MINTS members by fanning out
// one band across dates), this BUNDLES existing tokens across bands/types —
// the raster analogue of `memory_bundle` (which bundles per-cell facts).
// `bundle_cid = blake3(canonical_cbor(ordered member derivation_cids))`, so
// resolve recomputes it and refuses an altered membership (typed 409).

#[derive(Debug, Deserialize)]
pub struct RasterBundleReq {
    /// 2..=64 `emem:raster:` field tokens to bind, in the order given.
    pub tokens: Vec<String>,
    /// Optional human-readable purpose, folded into the bundle_cid preimage
    /// so the same members under a different purpose get a distinct bundle.
    #[serde(default)]
    pub purpose: Option<String>,
}

/// Resolve one member token to its verified record summary, or a typed error
/// naming which token failed. Confirms the derivation exists and is
/// raster-shaped, and binds the token's aoi/band/tslot against the record.
async fn resolve_raster_member(
    token: &str,
    s: &AppState,
) -> Result<(String, String, u64, String, String, String), ApiError> {
    let (aoi, band, tslot, derivation_cid) = parse_raster_token(token).map_err(bad_request)?;
    let facts = s
        .storage
        .get_facts_many(&[FactCid::new(derivation_cid.clone())])
        .await
        .map_err(ApiError::from)?;
    let Some(Some(fact)) = facts.into_iter().next() else {
        return Err(bad_request(format!(
            "member token has no derivation record on this responder: {token}"
        )));
    };
    let fact_json = serde_json::to_value(&fact).unwrap_or(json!({}));
    let fn_key = fact_json
        .get("derivation")
        .and_then(|d| d.get("fn_key"))
        .and_then(|f| f.as_str())
        .unwrap_or("");
    let is_raster_shaped = matches!(
        fn_key,
        "band_raster@1"
            | "s2_median_composite@1"
            | "dem_raster@1"
            | "embedding_raster@1"
            | "forest_raster@1"
    );
    if fact_json.get("kind").and_then(|k| k.as_str()) != Some("derivative") || !is_raster_shaped {
        return Err(conflict(format!(
            "member {derivation_cid} is not a raster-shaped field derivation; a raster bundle takes only emem:raster: field tokens"
        )));
    }
    let body = fact_json.get("value").cloned().unwrap_or(json!({}));
    // Bind the token's claims against the signed record (same rule raster_resolve applies).
    if body.get("aoi_cid").and_then(|v| v.as_str()) != Some(aoi.as_str())
        || body.get("band").and_then(|v| v.as_str()) != Some(band.as_str())
        || body.get("tslot").and_then(|v| v.as_u64()) != Some(tslot)
    {
        return Err(conflict(format!(
            "member token {token} contradicts its signed record (aoi/band/tslot); refusing a forged handle"
        )));
    }
    let artifact_cid = body
        .pointer("/artifact/artifact_cid")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    // The member's own signed record is anchored at its AOI centre cell, and
    // that cell is what a bundle should inherit. Returned so the bundle stops
    // inventing a subject out of the aoi_cid.
    let member_cell = fact_json
        .get("cell")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();
    Ok((derivation_cid, band, tslot, aoi, artifact_cid, member_cell))
}

/// `blake3(canonical_cbor([member derivation cids..., purpose]))` — content-
/// addresses the ordered membership so the same set always names the same bundle.
fn raster_bundle_cid(member_dcids: &[String], purpose: &str) -> Result<String, String> {
    let mut v: Vec<String> = member_dcids.to_vec();
    v.push(format!("purpose:{purpose}"));
    let bytes = emem_fact::cbor::to_canonical_cbor(&v)
        .map_err(|e| format!("member list does not canonicalize: {e}"))?;
    Ok(data_encoding::BASE32_NOPAD
        .encode(blake3::hash(&bytes).as_bytes())
        .to_lowercase())
}

pub async fn raster_bundle(req: RasterBundleReq, s: &AppState) -> Result<JsonValue, ApiError> {
    let started = Instant::now();
    if req.tokens.len() < 2 {
        return Err(bad_request(
            "a raster bundle needs at least 2 member tokens; one raster is already citeable on its own".into(),
        ));
    }
    if req.tokens.len() > 64 {
        return Err(bad_request(format!(
            "a raster bundle takes at most 64 members; got {}",
            req.tokens.len()
        )));
    }
    let purpose = req.purpose.clone().unwrap_or_default();

    // Resolve every member first; a bad token fails the whole mint by name.
    let mut members: Vec<JsonValue> = Vec::with_capacity(req.tokens.len());
    let mut member_dcids: Vec<String> = Vec::with_capacity(req.tokens.len());
    let mut parent_cids: Vec<FactCid> = Vec::with_capacity(req.tokens.len());
    for token in &req.tokens {
        let (dcid, band, tslot, aoi, artifact_cid, member_cell) =
            resolve_raster_member(token, s).await?;
        members.push(json!({
            "token": token,
            "derivation_cid": dcid,
            "band": band,
            "tslot": tslot,
            "aoi_cid": aoi,
            "artifact_cid": artifact_cid,
            "cell": member_cell,
        }));
        parent_cids.push(FactCid::new(dcid.clone()));
        member_dcids.push(dcid);
    }

    let bundle_cid = raster_bundle_cid(&member_dcids, &purpose).map_err(upstream_error)?;
    let record_body = json!({
        "schema": "emem.raster_bundle.v1",
        "fn_key": "raster_bundle@1",
        "bundle_cid": bundle_cid,
        "purpose": purpose,
        "member_count": members.len(),
        "members": members,
        "note": "a signed manifest binding N emem:raster: field tokens; bundle_cid content-addresses the ordered membership. Resolve to rebind every member and refuse an altered set. Each member resolves and re-derives independently.",
    });
    let signed_at = emem_storage::server::iso8601_now();
    // Anchor the bundle at the first member's AOI centre cell for a stable
    // handle. This read `aoi_cid` until 2026-08-13, which is the blake3 of the
    // bbox and not a cell at all, so every bundle was keyed at a subject
    // nothing could resolve: not recallable by place, not dereferenceable, and
    // silently so, because the storage key is just the string's bytes. The
    // comment already said "centre cell"; only the code disagreed.
    //
    // 4b43rrtd reported the mint failing outright after the write path started
    // validating subjects. The validator was right and this was the caller
    // that was wrong, which is why the fix is here rather than a widening of
    // what counts as an address.
    let anchor_cell = members
        .first()
        .and_then(|m| m.get("cell").and_then(|v| v.as_str()))
        .unwrap_or_default()
        .to_string();
    if !emem_codec::looks_like_cell64(&anchor_cell) {
        return Err(conflict(format!(
            "member {} carries no usable centre cell ({anchor_cell:?}), so this bundle has no \
             address to be minted at. Re-derive the member with band_raster and retry; a bundle \
             keyed at a subject nobody can resolve is worse than a refused mint, because the log \
             is append-only.",
            members
                .first()
                .and_then(|m| m.get("token").and_then(|v| v.as_str()))
                .unwrap_or("<first>")
        )));
    }
    let derivation_fact = DerivativeFact {
        cell: anchor_cell.clone(),
        band: "field.raster_bundle".to_string(),
        tslot_window: [0, 0],
        op: "raster_bundle".to_string(),
        parents: parent_cids.clone(),
        value: json_to_cbor(&record_body),
        confidence: 1.0,
        derivation: Derivation {
            fn_key: "raster_bundle@1".to_string(),
            args: None,
        },
        schema_cid: s.manifests.schema_cid.clone(),
        signer: s.identity.pubkey,
        signed_at: signed_at.clone(),
    };
    let att = emem_fact::Attestation::build_and_sign_v1(
        vec![Fact::Derivative(derivation_fact)],
        vec![],
        s.manifests.registry_cid.clone(),
        s.manifests.schema_cid.clone(),
        &s.identity.signing,
        s.identity.epoch,
        signed_at,
        None,
    )
    .map_err(|e| upstream_error(format!("bundle attestation: {e}")))?;
    let cids = s
        .storage
        .put_attestation(&att)
        .await
        .map_err(ApiError::from)?;
    let derivation_cid = cids
        .first()
        .map(|c| c.as_str().to_string())
        .ok_or_else(|| upstream_error("store returned no cid for the bundle".into()))?;

    let mut receipt_cids = parent_cids;
    receipt_cids.push(FactCid::new(derivation_cid.clone()));
    let receipt = s.sign_receipt(
        "emem.raster_bundle",
        vec![anchor_cell.clone()],
        receipt_cids,
        false,
        started,
        None,
    );

    Ok(json!({
        "schema": "emem.raster_bundle.v1",
        "algorithm_key": "raster_bundle@1",
        "bundle_cid": bundle_cid,
        "member_count": members.len(),
        "derivation": record_body,
        "derivation_cid": derivation_cid,
        "tokens": {
            "raster_bundle": format!("emem:rasterset:{bundle_cid}:{derivation_cid}"),
            "derivation_fact": format!("emem:fact:{anchor_cell}:{derivation_cid}"),
        },
        "docs": "https://emem.dev/reference#token-raster",
        "receipt": receipt,
    }))
}

#[derive(Debug, Deserialize)]
pub struct RasterBundleResolveReq {
    /// `emem:rasterset:<bundle_cid>:<derivation_cid>`
    pub token: String,
}

pub async fn raster_bundle_resolve(
    req: RasterBundleResolveReq,
    s: &AppState,
) -> Result<JsonValue, ApiError> {
    let rest = req
        .token
        .trim()
        .strip_prefix("emem:rasterset:")
        .ok_or_else(|| bad_request("a raster-bundle token starts with emem:rasterset:".into()))?;
    let parts: Vec<&str> = rest.split(':').collect();
    if parts.len() != 2 {
        return Err(bad_request(
            "emem:rasterset: takes exactly bundle_cid:derivation_cid".into(),
        ));
    }
    let (token_bundle_cid, derivation_cid) = (parts[0].to_string(), parts[1].to_string());

    let facts = s
        .storage
        .get_facts_many(&[FactCid::new(derivation_cid.clone())])
        .await
        .map_err(ApiError::from)?;
    let Some(Some(fact)) = facts.into_iter().next() else {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            ErrorBody {
                code: ErrorCode::CidNotFound,
                message: format!(
                    "no raster-bundle record for cid={derivation_cid} on this responder"
                ),
                details: None,
            },
        ));
    };
    let fact_json = serde_json::to_value(&fact).unwrap_or(json!({}));
    let fn_key = fact_json
        .get("derivation")
        .and_then(|d| d.get("fn_key"))
        .and_then(|f| f.as_str())
        .unwrap_or("");
    if fn_key != "raster_bundle@1" {
        return Err(conflict(format!(
            "cid {derivation_cid} is not a raster_bundle@1 derivation; an emem:rasterset: token cannot dereference it"
        )));
    }
    let body = fact_json.get("value").cloned().unwrap_or(json!({}));
    let members = body
        .get("members")
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_default();
    let purpose = body.get("purpose").and_then(|v| v.as_str()).unwrap_or("");
    let member_dcids: Vec<String> = members
        .iter()
        .filter_map(|m| {
            m.get("derivation_cid")
                .and_then(|v| v.as_str())
                .map(String::from)
        })
        .collect();

    // Recompute bundle_cid from the record's members and refuse an altered set.
    let recomputed = raster_bundle_cid(&member_dcids, purpose).map_err(upstream_error)?;
    let record_bundle_cid = body
        .get("bundle_cid")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if recomputed != record_bundle_cid || recomputed != token_bundle_cid {
        return Err(conflict(format!(
            "bundle_cid mismatch: token={token_bundle_cid}, record={record_bundle_cid}, recomputed-from-members={recomputed}. Refusing an altered or forged membership."
        )));
    }

    // Rebind: confirm every member still resolves as a raster derivation.
    let mut rebound: Vec<JsonValue> = Vec::with_capacity(members.len());
    for m in &members {
        let tok = m.get("token").and_then(|v| v.as_str()).unwrap_or("");
        let ok = resolve_raster_member(tok, s).await.is_ok();
        rebound.push(json!({ "token": tok, "resolves": ok }));
    }

    Ok(json!({
        "schema": "emem.raster_bundle.v1",
        "bundle_cid": recomputed,
        "verified": true,
        "member_count": members.len(),
        "members": members,
        "rebound": rebound,
        "note": "bundle_cid recomputed from the signed record's members and matched against the token; every member re-verified as a live raster derivation.",
    }))
}

pub async fn post_raster_bundle(
    State(s): State<AppState>,
    EmemJson(req): EmemJson<RasterBundleReq>,
) -> Result<Json<JsonValue>, ApiError> {
    Ok(Json(raster_bundle(req, &s).await?))
}

pub async fn post_raster_bundle_resolve(
    State(s): State<AppState>,
    EmemJson(req): EmemJson<RasterBundleResolveReq>,
) -> Result<Json<JsonValue>, ApiError> {
    Ok(Json(raster_bundle_resolve(req, &s).await?))
}

pub async fn post_band_raster(
    State(s): State<AppState>,
    EmemJson(req): EmemJson<BandRasterReq>,
) -> Result<Json<JsonValue>, ApiError> {
    Ok(Json(band_raster(req, &s).await?))
}

/// `GET /v1/artifacts/:cid` — the raw canonical bytes, immutable forever
/// (content-addressed). A miss is a typed 404 that says how to rebuild,
/// because eviction is a design property, not an error.
pub async fn get_artifact(AxumPath(cid): AxumPath<String>) -> impl IntoResponse {
    match ARTIFACTS.get(&cid) {
        Ok(Some(bytes)) => (
            StatusCode::OK,
            [
                (axum::http::header::CONTENT_TYPE, GRID_MEDIA_TYPE),
                (
                    axum::http::header::CACHE_CONTROL,
                    "public, max-age=31536000, immutable",
                ),
            ],
            bytes,
        )
            .into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "code": "artifact_evicted_or_unknown",
                "message": "no artifact bytes under this cid on this responder. If you hold the derivation record, it pins the scene, recipe, and geometry needed to rebuild the identical bytes; POST /v1/band_raster with the same bbox, band, and observed_on re-derives and re-stores them.",
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "code": "invalid_argument", "message": e.to_string() })),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct RasterResolveReq {
    /// `emem:raster:<aoi_cid>:<band>:<tslot>:<derivation_cid>`
    pub token: String,
    /// Opt-in anchors spot-check (the verification tier the field-token
    /// design promises). When true and the artifact is present, fetch and
    /// decode the grid, read each anchor's value back out at its (row, col),
    /// and cross-check it against the independently-signed per-cell fact the
    /// anchor cites. This is the DDS "click-to-verify": a field pixel that
    /// resolves to a signed fact whose value matches.
    #[serde(default)]
    pub spot_check: Option<bool>,
}

fn conflict(message: String) -> ApiError {
    ApiError(
        StatusCode::CONFLICT,
        ErrorBody {
            code: ErrorCode::InvalidArgument,
            message,
            details: None,
        },
    )
}

/// Parse the raster token into its four claims. Pure, unit-tested.
fn parse_raster_token(token: &str) -> Result<(String, String, u64, String), String> {
    let rest = token
        .trim()
        .strip_prefix("emem:raster:")
        .ok_or_else(|| "a raster token starts with emem:raster:".to_string())?;
    let parts: Vec<&str> = rest.split(':').collect();
    if parts.len() != 4 {
        return Err(format!(
            "emem:raster: takes exactly aoi_cid:band:tslot:derivation_cid; got {} segments",
            parts.len()
        ));
    }
    let is_cid = |s: &str| {
        s.len() == 52
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
    };
    if !is_cid(parts[0]) {
        return Err("aoi_cid is not a 52-character base32 cid".to_string());
    }
    if !is_cid(parts[3]) {
        return Err("derivation_cid is not a 52-character base32 cid".to_string());
    }
    let tslot: u64 = parts[2]
        .parse()
        .map_err(|_| format!("tslot `{}` is not an unsigned integer", parts[2]))?;
    Ok((
        parts[0].to_string(),
        parts[1].to_string(),
        tslot,
        parts[3].to_string(),
    ))
}

/// `POST /v1/raster/resolve` — dereference an `emem:raster:` token.
///
/// Every claim in the token binds to the signed derivation record
/// before anything dereferences (the fact-token rule, applied to
/// fields): the cid must be a `band_raster@1` derivation, and the
/// token's aoi_cid, band, and tslot must each match the record's own
/// body, so a real derivation_cid cannot be passed off under a false
/// area, band, or date. The response carries the record and the
/// artifact's status; the bytes themselves come from
/// `GET /v1/artifacts/{cid}` (immutable), and an evicted artifact is a
/// rebuild recipe, not an error.
pub async fn raster_resolve(req: RasterResolveReq, s: &AppState) -> Result<JsonValue, ApiError> {
    let started = Instant::now();
    let (aoi, band, tslot, derivation_cid) = parse_raster_token(&req.token).map_err(bad_request)?;

    let cid_obj = FactCid::new(derivation_cid.clone());
    let facts = s
        .storage
        .get_facts_many(&[cid_obj])
        .await
        .map_err(ApiError::from)?;
    let Some(Some(fact)) = facts.into_iter().next() else {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            ErrorBody {
                code: ErrorCode::CidNotFound,
                message: format!(
                    "no derivation record for cid={derivation_cid} on this responder. The record persists independently of the artifact; if you minted this token elsewhere, resolve it there."
                ),
                details: None,
            },
        ));
    };
    let fact_json = serde_json::to_value(&fact).unwrap_or(json!({}));
    // Both raster-shaped field derivations resolve here: a single-scene window
    // (band_raster@1) and a masked median over a window (s2_median_composite@1).
    let fn_key = fact_json
        .get("derivation")
        .and_then(|d| d.get("fn_key"))
        .and_then(|f| f.as_str())
        .unwrap_or("");
    let is_raster_shaped = matches!(
        fn_key,
        "band_raster@1"
            | "s2_median_composite@1"
            | "dem_raster@1"
            | "embedding_raster@1"
            | "forest_raster@1"
    );
    if fact_json.get("kind").and_then(|k| k.as_str()) != Some("derivative") || !is_raster_shaped {
        return Err(conflict(format!(
            "cid {derivation_cid} is not a raster-shaped field derivation (band_raster@1 or s2_median_composite@1); an emem:raster: token cannot dereference it. Cite it in emem:fact: form instead."
        )));
    }
    let body = fact_json.get("value").cloned().unwrap_or(json!({}));
    let bind = |claim: &str, token_v: &str, record_v: Option<&str>| -> Result<(), ApiError> {
        match record_v {
            Some(r) if r == token_v => Ok(()),
            Some(r) => Err(conflict(format!(
                "token {claim} `{token_v}` contradicts the signed record's `{r}`; refusing to dereference a forged or corrupt handle"
            ))),
            None => Err(conflict(format!(
                "the signed record carries no {claim}; refusing an unbindable claim"
            ))),
        }
    };
    bind(
        "aoi_cid",
        &aoi,
        body.get("aoi_cid").and_then(|v| v.as_str()),
    )?;
    bind("band", &band, body.get("band").and_then(|v| v.as_str()))?;
    let record_tslot = body.get("tslot").and_then(|v| v.as_u64());
    if record_tslot != Some(tslot) {
        return Err(conflict(format!(
            "token tslot {tslot} contradicts the signed record's {record_tslot:?}"
        )));
    }

    let artifact_cid = body
        .pointer("/artifact/artifact_cid")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let artifact_present =
        !artifact_cid.is_empty() && matches!(ARTIFACTS.get(&artifact_cid), Ok(Some(_)));

    let cell = fact_json
        .get("cell")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();

    // Anchors spot-check (opt-in): read each anchor's value back out of the
    // artifact grid and cross-check it against the per-cell fact it cites.
    let spot_check = if req.spot_check.unwrap_or(false) {
        Some(spot_check_anchors(&body, &artifact_cid, artifact_present, s).await)
    } else {
        None
    };

    let receipt = s.sign_receipt_field(
        "emem.raster_resolve",
        vec![cell],
        vec![FactCid::new(derivation_cid.clone())],
        true,
        started,
        FieldBinding {
            aoi_cid: aoi.clone(),
            derivation_cid: derivation_cid.clone(),
        },
    );

    let mut out = json!({
        "schema": "emem.raster_resolve.v1",
        "token": req.token.trim(),
        "resolved": true,
        "aoi_cid": aoi,
        "band": band,
        "tslot": tslot,
        "derivation_cid": derivation_cid,
        "derivation": body,
        "artifact": {
            "artifact_cid": artifact_cid,
            "present": artifact_present,
            "url": format!("/v1/artifacts/{artifact_cid}"),
            "if_evicted": "the record above pins the scene, recipe, and geometry; POST /v1/band_raster with the same bbox, band, and observed_on rebuilds the identical bytes",
        },
        "docs": "https://emem.dev/reference#token-raster",
        "receipt": receipt,
        "offline_verify_at": "/verify",
    });
    if let Some(sc) = spot_check {
        if let Some(o) = out.as_object_mut() {
            o.insert("spot_check".into(), sc);
        }
    }
    Ok(out)
}

/// The anchors spot-check tier: decode the artifact grid, read each anchor's
/// value back out at its (row, col), and cross-check it against the per-cell
/// fact the anchor cites (the field-to-signed-fact bridge — the DDS
/// "click-to-verify"). Never fails the resolve; returns a per-anchor verdict.
async fn spot_check_anchors(
    body: &JsonValue,
    artifact_cid: &str,
    artifact_present: bool,
    s: &AppState,
) -> JsonValue {
    if !artifact_present {
        return json!({
            "checked": false,
            "reason": "artifact evicted; rebuild it first (the record pins the recipe), then spot-check",
        });
    }
    let bytes = match ARTIFACTS.get(artifact_cid) {
        Ok(Some(b)) => b,
        _ => return json!({ "checked": false, "reason": "artifact bytes unreadable" }),
    };
    let (header, values) = match emem_codec::grid::decode_grid(&bytes) {
        Ok(hv) => hv,
        Err(e) => return json!({ "checked": false, "reason": format!("grid decode: {e:?}") }),
    };
    let anchors = body
        .get("anchors")
        .and_then(|a| a.as_array())
        .cloned()
        .unwrap_or_default();
    // The raster's own band; an anchor whose cited fact is a DIFFERENT band is
    // a related aggregate (context), not an exact-quantity bridge.
    let raster_band = body.get("band").and_then(|v| v.as_str());
    let eps = 1e-3_f64;
    // A forest field holds classes and calendar years: 1% of 2020 is twenty
    // years, so its anchors must match their facts exactly.
    let exact = body.get("fn_key").and_then(|v| v.as_str()) == Some("forest_raster@1");
    let mut results: Vec<JsonValue> = Vec::with_capacity(anchors.len());
    let mut checked = 0usize;
    let mut grid_ok = 0usize;
    let mut fact_ok = 0usize;
    let mut fact_seen = 0usize;
    let mut fact_checked = 0usize;
    for a in &anchors {
        let (Some(row), Some(col)) = (
            a.get("row").and_then(|v| v.as_u64()),
            a.get("col").and_then(|v| v.as_u64()),
        ) else {
            continue;
        };
        checked += 1;
        let grid_v = emem_codec::grid::grid_value_at(&header, &values, row as u32, col as u32)
            .map(|v| v as f64);
        let anchor_v = a.get("value").and_then(|v| v.as_f64());
        // (1) the record's anchor value matches the artifact grid at (row,col).
        // NaN nodata and out-of-bounds edge anchors both serialise to JSON
        // null / None; None-on-both-sides is a CONSISTENT match (the record
        // recorded null because the grid was null there at mint), not a miss.
        let grid_match = match (grid_v, anchor_v) {
            (Some(g), Some(av)) => (g - av).abs() <= eps,
            (Some(g), None) => g.is_nan(),
            (None, None) => true,
            (None, Some(_)) => false,
        };
        if grid_match {
            grid_ok += 1;
        }
        // (2) the cited per-cell fact resolves and its value matches the grid.
        // Only cross-check anchors that land on a real pixel: an anchor at the
        // exact bbox edge maps to an out-of-bounds pixel (grid None) while its
        // cell-centre fact still has a value — that is INCONCLUSIVE (no pixel
        // there to compare), not a failure, so it does not count against passed.
        let mut fact_match: Option<bool> = None;
        let mut fact_delta: Option<f64> = None;
        let mut fact_relation: Option<&'static str> = None;
        if let Some(fc) = a.get("fact_cid").and_then(|v| v.as_str()) {
            fact_seen += 1;
            if let Some(g) = grid_v {
                let got = s
                    .storage
                    .get_facts_many(&[FactCid::new(fc.to_string())])
                    .await
                    .ok()
                    .and_then(|mut f| f.pop().flatten());
                let (fv, fb) = got
                    .as_ref()
                    .map(|fact| {
                        let fj = serde_json::to_value(fact).unwrap_or(json!({}));
                        (
                            fj.get("value").and_then(|v| v.as_f64()),
                            fj.get("band").and_then(|v| v.as_str()).map(String::from),
                        )
                    })
                    .unwrap_or((None, None));
                fact_delta = fv.map(|f| g - f);
                // Exact bridge ONLY when the cited fact is the SAME band as the
                // grid quantity. When it is a RELATED AGGREGATE of a different
                // band — a point-elevation grid citing copdem30m.elevation_mean
                // (a 3x3 neighbourhood mean) — the point and the mean legitimately
                // differ, so we report the delta as CONTEXT and do not pass/fail
                // on it, rather than falsely calling a real derivation broken.
                let same_band = fb.as_deref() == raster_band;
                if same_band {
                    fact_checked += 1;
                    let tol = if exact { eps } else { eps.max(0.01 * g.abs()) };
                    let m = fv.map(|f| (g - f).abs() <= tol).unwrap_or(false);
                    if m {
                        fact_ok += 1;
                    }
                    fact_match = Some(m);
                    fact_relation = Some("same_band_exact");
                } else {
                    fact_relation = Some("aggregate_context");
                }
            }
        }
        results.push(json!({
            "cell": a.get("cell"),
            "row": row,
            "col": col,
            "fact_relation": fact_relation,
            "grid_value": grid_v,
            "anchor_value": anchor_v,
            "grid_matches_record": grid_match,
            "fact_cid": a.get("fact_cid"),
            "fact_matches_grid": fact_match,
            "fact_delta": fact_delta,
        }));
    }
    json!({
        "checked": true,
        "anchors": checked,
        "grid_matches_record": grid_ok,
        "anchors_with_fact": fact_seen,
        "fact_bridge_checked": fact_checked,
        "fact_matches_grid": fact_ok,
        "fact_bridge_context_only": fact_seen - fact_checked,
        "passed": checked > 0 && grid_ok == checked && fact_ok == fact_checked,
        "results": results,
        "tiers": "grid_matches_record = the artifact bytes agree with the signed record's sampled anchors. fact_matches_grid = each same-band anchor's independently-signed per-cell fact agrees with the field at that pixel (the click-to-verify bridge). fact_bridge_context_only = anchors whose cited fact is a RELATED AGGREGATE of a different band (per-anchor fact_relation:\"aggregate_context\", e.g. a point-elevation grid citing copdem30m.elevation_mean) — the fact_delta is surfaced as provenance context, not a pass/fail, since a 3x3 mean and a point sample legitimately differ. `passed` requires grid consistency on all anchors and an exact match on every same-band bridge. For the full tier, GET the artifact and re-hash it against artifact_cid.",
    })
}

pub async fn post_raster_resolve(
    State(s): State<AppState>,
    EmemJson(req): EmemJson<RasterResolveReq>,
) -> Result<Json<JsonValue>, ApiError> {
    Ok(Json(raster_resolve(req, &s).await?))
}

// ── emem:cube: — a field over time, as a signed manifest. ───────────────────
//
// A world model is a field over an AOI ACROSS TIME; `emem:raster:` names one
// time-slice, and a 4D world's time scrub had no token to anchor. A cube is
// NOT new pixels: it mints one `band_raster` member per requested date, each
// an independent, resolvable `emem:raster:` derivation, then signs a manifest
// binding the ordered set. Lineage terminates in each member's pinned scene,
// so a stranger walks cube -> members -> scenes. `cube_cid` content-addresses
// the ordered membership (blake3 of the member derivation cids), and the
// token's terminal segment stays the persisted record handle like a raster.

/// Hard cap on cube depth. Each slice is an independent scene read, so this
/// bounds mint latency the way `MAX_SIDE_PX` bounds one window. Page a wider
/// time range across several mints.
const MAX_CUBE_SLICES: usize = 24;

#[derive(Debug, Deserialize)]
pub struct BandCubeReq {
    pub bbox: BBox,
    /// One of the `S2_BANDS` keys, e.g. `"s2.B04"`.
    pub band: String,
    /// Target capture dates `YYYY-MM-DD`, 2..=`MAX_CUBE_SLICES`. Each names the
    /// nearest scene, pinned per member; dates that resolve to the same scene
    /// collapse to one slice.
    pub observed_on: Vec<String>,
}

/// Content-address of an ordered cube membership: blake3 of the canonical CBOR
/// of the ordered member derivation cids, base32. The same slices in the same
/// order always name the same cube; reordering changes it. Pure, unit-tested.
fn cube_cid_of(member_dcids: &[String]) -> Result<String, String> {
    let v = member_dcids.to_vec();
    let bytes = emem_fact::cbor::to_canonical_cbor(&v)
        .map_err(|e| format!("member list does not canonicalize: {e}"))?;
    Ok(data_encoding::BASE32_NOPAD
        .encode(blake3::hash(&bytes).as_bytes())
        .to_lowercase())
}

pub async fn band_cube(req: BandCubeReq, s: &AppState) -> Result<JsonValue, ApiError> {
    let started = Instant::now();
    let b = &req.bbox;
    // The same bbox + band validation band_raster enforces, up front, so a bad
    // request fails before any scene fan-out rather than mid-flight.
    if !(b.min_lat < b.max_lat && b.min_lng < b.max_lng) {
        return Err(bad_request(
            "bbox must satisfy min_lat < max_lat and min_lng < max_lng".into(),
        ));
    }
    if b.min_lat < -90.0 || b.max_lat > 90.0 || b.min_lng < -180.0 || b.max_lng > 180.0 {
        return Err(bad_request("bbox exceeds WGS-84 bounds".into()));
    }
    if !S2_BANDS.iter().any(|(k, _)| *k == req.band) {
        let names: Vec<&str> = S2_BANDS.iter().map(|(k, _)| *k).collect();
        return Err(bad_request(format!(
            "band `{}` is not rasterizable; this executor serves {}",
            req.band,
            names.join(", ")
        )));
    }
    let dates = &req.observed_on;
    if dates.len() < 2 {
        return Err(bad_request(
            "a cube needs at least 2 target dates in observed_on; for a single time slice use POST /v1/band_raster".into(),
        ));
    }
    if dates.len() > MAX_CUBE_SLICES {
        return Err(bad_request(format!(
            "{} dates requested; the cube cap is {MAX_CUBE_SLICES} slices per mint. Page the time range across several cubes.",
            dates.len()
        )));
    }
    for d in dates {
        if parse_ymd(d).is_none() {
            return Err(bad_request(format!("observed_on `{d}` is not YYYY-MM-DD")));
        }
    }

    // Each member is a real, independently-resolvable band_raster derivation.
    // Fan out in input order (join_all preserves it), like eudr's parallel arm.
    let futs: Vec<_> = dates
        .iter()
        .map(|d| {
            let d = d.clone();
            let bbox = req.bbox.clone();
            let band = req.band.clone();
            async move {
                band_raster(
                    BandRasterReq {
                        bbox,
                        band,
                        observed_on: Some(d),
                    },
                    s,
                )
                .await
            }
        })
        .collect();
    let member_results = futures_util::future::join_all(futs).await;

    // A cube with a missing slice is not the cube that was asked for: the first
    // member error fails the whole mint rather than signing a partial stack.
    let mut members_raw: Vec<JsonValue> = Vec::with_capacity(dates.len());
    for r in member_results {
        members_raw.push(r?);
    }

    // Dedup by tslot (two near dates can pick one scene); a BTreeMap keeps the
    // canonical ascending-tslot member order the cube_cid is computed over.
    // `members_raw` parallels `dates` because join_all preserves order, so the
    // zip below maps each requested date to the member it produced. Echoing
    // that mapping is what lets a caller line a requested date up with its
    // slice directly, instead of guessing by tslot proximity when dates
    // collapse to one scene.
    let mut by_tslot: std::collections::BTreeMap<u64, JsonValue> =
        std::collections::BTreeMap::new();
    let mut reqs_by_tslot: std::collections::BTreeMap<u64, Vec<String>> =
        std::collections::BTreeMap::new();
    let mut aoi_seen: Option<String> = None;
    for (req_date, m) in dates.iter().zip(&members_raw) {
        let tslot = m
            .get("tslot")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| upstream_error("a raster member is missing tslot".into()))?;
        let dcid = m
            .get("derivation_cid")
            .and_then(|v| v.as_str())
            .ok_or_else(|| upstream_error("a raster member is missing derivation_cid".into()))?
            .to_string();
        let acid = m
            .pointer("/artifact/artifact_cid")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let rtok = m
            .pointer("/tokens/raster")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let member_aoi = m
            .get("aoi_cid")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        match &aoi_seen {
            None => aoi_seen = Some(member_aoi.clone()),
            Some(a) if *a != member_aoi => {
                return Err(upstream_error(
                    "members disagree on aoi_cid; refusing to stack fields over different geometry"
                        .into(),
                ))
            }
            _ => {}
        }
        let scene = m
            .pointer("/derivation/sources/0")
            .cloned()
            .unwrap_or(json!({}));
        reqs_by_tslot
            .entry(tslot)
            .or_default()
            .push(req_date.clone());
        by_tslot.entry(tslot).or_insert_with(|| {
            json!({
                "tslot": tslot,
                "derivation_cid": dcid,
                "artifact_cid": acid,
                "raster_token": rtok,
                "scene_id": scene.get("id").cloned().unwrap_or(JsonValue::Null),
                "captured_at": scene.get("captured_at").cloned().unwrap_or(JsonValue::Null),
            })
        });
    }
    if by_tslot.len() < 2 {
        return Err(bad_request(format!(
            "the {} requested date(s) resolved to only {} distinct scene(s); a cube needs at least two time slices. Widen the date spread.",
            dates.len(),
            by_tslot.len()
        )));
    }

    // Attach the requested-date mapping to each member: the observed_on
    // entries that resolved to this scene, and the nearest one's distance from
    // the scene's own capture date, in whole days. This kills the caller-side
    // tslot-proximity heuristic; it does not enter cube_cid (that is the
    // digest of the member derivation cids only).
    for (tslot, member) in by_tslot.iter_mut() {
        let Some(o) = member.as_object_mut() else {
            continue;
        };
        let Some(reqs) = reqs_by_tslot.get(tslot) else {
            continue;
        };
        let scene_days = o
            .get("captured_at")
            .and_then(|v| v.as_str())
            .and_then(|s| s.get(..10))
            .and_then(parse_ymd)
            .map(|(y, mo, d)| crate::days_from_civil(y, mo, d));
        let nearest = scene_days.and_then(|sd| {
            reqs.iter()
                .filter_map(|r| {
                    parse_ymd(r).map(|(y, mo, d)| (crate::days_from_civil(y, mo, d) - sd).abs())
                })
                .min()
        });
        o.insert("requested_dates".to_string(), json!(reqs));
        if let Some(n) = nearest {
            o.insert("requested_date_distance_days".to_string(), json!(n));
        }
    }

    let members: Vec<JsonValue> = by_tslot.values().cloned().collect();
    let member_dcids: Vec<String> = by_tslot
        .values()
        .map(|m| {
            m.get("derivation_cid")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        })
        .collect();
    let tslot_lo = *by_tslot.keys().next().expect("len >= 2 checked above");
    let tslot_hi = *by_tslot.keys().next_back().expect("len >= 2 checked above");
    let aoi = aoi_seen.unwrap_or_default();
    let band_key = req.band.clone();
    let cube_cid = cube_cid_of(&member_dcids).map_err(upstream_error)?;

    let record_body = json!({
        "schema": "emem.cube_derivation.v1",
        "aoi_cid": aoi,
        "band": band_key,
        "tslot_lo": tslot_lo,
        "tslot_hi": tslot_hi,
        "fn_key": "band_cube@1",
        "cube_cid": cube_cid,
        "member_count": members.len(),
        "members": members,
        "note": "a cube is a signed manifest over independently-resolvable band_raster members; lineage terminates in each member's pinned scene. Resolve each member with its emem:raster: token (or batch via resolve_many).",
    });
    let signed_at = emem_storage::server::iso8601_now();
    let centre_lat = (b.min_lat + b.max_lat) / 2.0;
    let centre_lng = (b.min_lng + b.max_lng) / 2.0;
    let centre_cell = emem_codec::geo::cell64_from_latlng(centre_lat, centre_lng);
    let parents: Vec<FactCid> = member_dcids
        .iter()
        .map(|c| FactCid::new(c.clone()))
        .collect();
    let derivation_fact = DerivativeFact {
        cell: centre_cell.clone(),
        band: "field.cube".to_string(),
        tslot_window: [tslot_lo, tslot_hi],
        op: "band_cube".to_string(),
        parents: parents.clone(),
        value: json_to_cbor(&record_body),
        confidence: 1.0,
        derivation: Derivation {
            fn_key: "band_cube@1".to_string(),
            args: None,
        },
        schema_cid: s.manifests.schema_cid.clone(),
        signer: s.identity.pubkey,
        signed_at: signed_at.clone(),
    };
    let att = emem_fact::Attestation::build_and_sign_v1(
        vec![Fact::Derivative(derivation_fact)],
        vec![],
        s.manifests.registry_cid.clone(),
        s.manifests.schema_cid.clone(),
        &s.identity.signing,
        s.identity.epoch,
        signed_at,
        None,
    )
    .map_err(|e| upstream_error(format!("cube attestation: {e}")))?;
    let cids = s
        .storage
        .put_attestation(&att)
        .await
        .map_err(ApiError::from)?;
    let derivation_cid = cids
        .first()
        .map(|c| c.as_str().to_string())
        .ok_or_else(|| upstream_error("store returned no cid for the cube derivation".into()))?;

    let binding = FieldBinding {
        aoi_cid: aoi.clone(),
        derivation_cid: derivation_cid.clone(),
    };
    let mut receipt_cids = parents;
    receipt_cids.push(FactCid::new(derivation_cid.clone()));
    let receipt = s.sign_receipt_field(
        "emem.band_cube",
        vec![centre_cell.clone()],
        receipt_cids,
        false,
        started,
        binding,
    );

    Ok(json!({
        "schema": "emem.band_cube.v1",
        "algorithm_key": "band_cube@1",
        "aoi_cid": aoi,
        "band": band_key,
        "tslot_lo": tslot_lo,
        "tslot_hi": tslot_hi,
        "member_count": member_dcids.len(),
        "cube_cid": cube_cid,
        "derivation": record_body,
        "derivation_cid": derivation_cid,
        "tokens": {
            "cube": format!("emem:cube:{aoi}:{band_key}:{tslot_lo}..{tslot_hi}:{derivation_cid}"),
            "derivation_fact": format!("emem:fact:{centre_cell}:{derivation_cid}"),
        },
        "docs": "https://emem.dev/reference#token-cube",
        "members_note": "each member carries an emem:raster: token you can resolve independently or batch through resolve_many",
        "receipt": receipt,
    }))
}

pub async fn post_band_cube(
    State(s): State<AppState>,
    EmemJson(req): EmemJson<BandCubeReq>,
) -> Result<Json<JsonValue>, ApiError> {
    Ok(Json(band_cube(req, &s).await?))
}

#[derive(Debug, Deserialize)]
pub struct CubeResolveReq {
    /// `emem:cube:<aoi_cid>:<band>:<tslot_lo>..<tslot_hi>:<derivation_cid>`
    pub token: String,
}

/// Parse the cube token into its claims. Pure, unit-tested.
fn parse_cube_token(token: &str) -> Result<(String, String, u64, u64, String), String> {
    let rest = token
        .trim()
        .strip_prefix("emem:cube:")
        .ok_or_else(|| "a cube token starts with emem:cube:".to_string())?;
    let parts: Vec<&str> = rest.split(':').collect();
    if parts.len() != 4 {
        return Err(format!(
            "emem:cube: takes exactly aoi_cid:band:tslot_lo..tslot_hi:derivation_cid; got {} segments",
            parts.len()
        ));
    }
    let is_cid = |s: &str| {
        s.len() == 52
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
    };
    if !is_cid(parts[0]) {
        return Err("aoi_cid is not a 52-character base32 cid".to_string());
    }
    if !is_cid(parts[3]) {
        return Err("derivation_cid is not a 52-character base32 cid".to_string());
    }
    let (lo_s, hi_s) = parts[2]
        .split_once("..")
        .ok_or_else(|| format!("tslot range `{}` must be tslot_lo..tslot_hi", parts[2]))?;
    let lo: u64 = lo_s
        .parse()
        .map_err(|_| format!("tslot_lo `{lo_s}` is not an unsigned integer"))?;
    let hi: u64 = hi_s
        .parse()
        .map_err(|_| format!("tslot_hi `{hi_s}` is not an unsigned integer"))?;
    if lo > hi {
        return Err(format!("tslot range is inverted: {lo} > {hi}"));
    }
    Ok((
        parts[0].to_string(),
        parts[1].to_string(),
        lo,
        hi,
        parts[3].to_string(),
    ))
}

/// `POST /v1/cube/resolve` — dereference an `emem:cube:` token.
///
/// Same fail-closed rule as raster: every claim in the token binds to the
/// signed cube record before anything dereferences, the cid must be a
/// `band_cube@1` derivation, and `cube_cid` is recomputed from the record's
/// members so an altered membership is refused, not silently served.
pub async fn cube_resolve(req: CubeResolveReq, s: &AppState) -> Result<JsonValue, ApiError> {
    let started = Instant::now();
    let (aoi, band, lo, hi, derivation_cid) = parse_cube_token(&req.token).map_err(bad_request)?;

    let cid_obj = FactCid::new(derivation_cid.clone());
    let facts = s
        .storage
        .get_facts_many(&[cid_obj])
        .await
        .map_err(ApiError::from)?;
    let Some(Some(fact)) = facts.into_iter().next() else {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            ErrorBody {
                code: ErrorCode::CidNotFound,
                message: format!(
                    "no cube derivation record for cid={derivation_cid} on this responder. The record persists independently of its members; if you minted this token elsewhere, resolve it there."
                ),
                details: None,
            },
        ));
    };
    let fact_json = serde_json::to_value(&fact).unwrap_or(json!({}));
    if fact_json.get("kind").and_then(|k| k.as_str()) != Some("derivative")
        || fact_json
            .get("derivation")
            .and_then(|d| d.get("fn_key"))
            .and_then(|f| f.as_str())
            != Some("band_cube@1")
    {
        return Err(conflict(format!(
            "cid {derivation_cid} is not a band_cube@1 field derivation; an emem:cube: token cannot dereference it. Cite it in emem:fact: form instead."
        )));
    }
    let body = fact_json.get("value").cloned().unwrap_or(json!({}));
    let bind = |claim: &str, token_v: &str, record_v: Option<&str>| -> Result<(), ApiError> {
        match record_v {
            Some(r) if r == token_v => Ok(()),
            Some(r) => Err(conflict(format!(
                "token {claim} `{token_v}` contradicts the signed record's `{r}`; refusing to dereference a forged or corrupt handle"
            ))),
            None => Err(conflict(format!(
                "the signed record carries no {claim}; refusing an unbindable claim"
            ))),
        }
    };
    bind(
        "aoi_cid",
        &aoi,
        body.get("aoi_cid").and_then(|v| v.as_str()),
    )?;
    bind("band", &band, body.get("band").and_then(|v| v.as_str()))?;
    let rec_lo = body.get("tslot_lo").and_then(|v| v.as_u64());
    let rec_hi = body.get("tslot_hi").and_then(|v| v.as_u64());
    if rec_lo != Some(lo) || rec_hi != Some(hi) {
        return Err(conflict(format!(
            "token tslot range {lo}..{hi} contradicts the signed record's {rec_lo:?}..{rec_hi:?}"
        )));
    }

    // Rebind cube_cid: recompute it from the members the record carries and
    // refuse if the stored membership was altered.
    let member_dcids: Vec<String> = body
        .get("members")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    m.get("derivation_cid")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                })
                .collect()
        })
        .unwrap_or_default();
    let recomputed = cube_cid_of(&member_dcids).map_err(conflict)?;
    let stored = body.get("cube_cid").and_then(|v| v.as_str()).unwrap_or("");
    if recomputed != stored {
        return Err(conflict(format!(
            "recomputed cube_cid {recomputed} does not match the record's `{stored}`; the membership was altered"
        )));
    }

    let member_tokens: Vec<JsonValue> = body
        .get("members")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .map(|m| {
                    json!({
                        "tslot": m.get("tslot"),
                        "raster_token": m.get("raster_token"),
                        "artifact_cid": m.get("artifact_cid"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let cell = fact_json
        .get("cell")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();
    let receipt = s.sign_receipt_field(
        "emem.cube_resolve",
        vec![cell],
        vec![FactCid::new(derivation_cid.clone())],
        true,
        started,
        FieldBinding {
            aoi_cid: aoi.clone(),
            derivation_cid: derivation_cid.clone(),
        },
    );

    Ok(json!({
        "schema": "emem.cube_resolve.v1",
        "token": req.token.trim(),
        "resolved": true,
        "aoi_cid": aoi,
        "band": band,
        "tslot_lo": lo,
        "tslot_hi": hi,
        "cube_cid": stored,
        "derivation_cid": derivation_cid,
        "derivation": body,
        "member_tokens": member_tokens,
        "docs": "https://emem.dev/reference#token-cube",
        "members_note": "resolve each raster_token with emem_raster_resolve, or batch them with resolve_many; each artifact is at GET /v1/artifacts/{artifact_cid}, immutable",
        "receipt": receipt,
        "offline_verify_at": "/verify",
    }))
}

pub async fn post_cube_resolve(
    State(s): State<AppState>,
    EmemJson(req): EmemJson<CubeResolveReq>,
) -> Result<Json<JsonValue>, ApiError> {
    Ok(Json(cube_resolve(req, &s).await?))
}

// ── s2_median_composite@1 — a signed, cloud-masked median over a window. ────
//
// A world's scrub frame is a median composite, not a single scene, and until
// now that composite was unsigned while the signed thing was a nearby single
// scene. This makes the composite itself the resolvable artifact: a masked
// per-pixel median over every clear scene in a date window, pinned to the
// exact scenes it read, so a stranger re-derives it pixel for pixel. The mask
// policy is pinned in the derivation, and it is the same per-pixel SCL
// discipline the scalar path uses (which the foundation-model chip path is
// missing), so a correct composite is also the SCL gate embeddings need.

/// SCL classes rejected by default: 0 no-data, 1 saturated/defective, 3 cloud
/// shadow, 8 cloud (medium prob), 9 cloud (high prob), 10 thin cirrus. Snow
/// (11) is KEPT: it is surface, not occlusion, so a snow-rejected median would
/// fabricate a bare winter scene. Overridable via `mask_policy`.
pub(crate) const DEFAULT_SCL_REJECT: [u8; 6] = [0, 1, 3, 8, 9, 10];

/// Hard cap on scenes read per composite mint; each is two COG window reads.
const MAX_COMPOSITE_SCENES: usize = 16;

#[derive(Debug, Deserialize)]
pub struct BandCompositeReq {
    pub bbox: BBox,
    /// One of the `S2_BANDS` keys, e.g. `"s2.B04"`.
    pub band: String,
    /// Window bounds, inclusive, `YYYY-MM-DD`.
    pub start_date: String,
    pub end_date: String,
    /// SCL classes to reject per pixel; defaults to `DEFAULT_SCL_REJECT`.
    #[serde(default)]
    pub mask_policy: Option<Vec<u8>>,
    /// Per-pixel minimum valid (unmasked) samples for a value; else nodata.
    /// Defaults to 1.
    #[serde(default)]
    pub min_valid_count: Option<u32>,
    /// Cap on scenes read (2..=16); defaults to 12.
    #[serde(default)]
    pub max_scenes: Option<usize>,
}

/// Day-number tslot for a scene's `YYYY-MM-DD...` capture time.
pub(crate) fn scene_tslot(datetime: &str) -> u64 {
    let d = datetime.get(..10).unwrap_or("");
    parse_ymd(d)
        .map(|(y, m, dd)| (crate::days_from_civil(y, m, dd).max(0)) as u64)
        .unwrap_or(0)
}

/// Read one scene's band window and mask it by the co-located SCL window,
/// returning the masked grid (rejected/nodata pixels are NaN) plus the scene
/// metadata to pin. `None` if the scene is unreadable or off the anchor's
/// pixel grid (a different native resolution).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn read_masked_scene(
    cli: &reqwest::Client,
    host: &'static str,
    item: &emem_fetch::stac::StacItem,
    aliases: &[&str],
    cx: f64,
    cy: f64,
    w: u32,
    h: u32,
    band_native_m: f64,
    reject: &std::collections::BTreeSet<u8>,
) -> Option<(Vec<f32>, JsonValue)> {
    let dn_offset = crate::s2_dn_offset(host, item).ok()?;
    let band_url = aliases.iter().find_map(|a| item.assets.get(*a).cloned())?;
    let scl_url = scl_asset(item)?;
    let band_prof = emem_fetch::cog::open_profile(cli, &band_url).await.ok()?;
    // Same tile grid only: a different native resolution is a different pixel
    // lattice and cannot be medianed pixel-for-pixel.
    if (band_prof.pixel_scale.0.abs() - band_native_m).abs() > 0.01 {
        return None;
    }
    let band_raw = emem_fetch::cog::sample_window(cli, &band_url, &band_prof, cx, cy, w, h)
        .await
        .ok()?;
    let scl_prof = emem_fetch::cog::open_profile(cli, &scl_url).await.ok()?;
    let scl_m = scl_prof.pixel_scale.0.abs();
    if !(scl_m > 0.0 && scl_m.is_finite()) {
        return None;
    }
    let band_m = band_native_m;
    // One SCL pixel of margin: each window centres on its own floored pixel,
    // so their corners differ by up to a pixel of the coarser grid.
    let w_scl = ((w as f64 * band_m / scl_m).ceil() as u32).max(1) + 2;
    let h_scl = ((h as f64 * band_m / scl_m).ceil() as u32).max(1) + 2;
    let scl_raw = emem_fetch::cog::sample_window(cli, &scl_url, &scl_prof, cx, cy, w_scl, h_scl)
        .await
        .ok()?;
    // Each window's world corner, from the origin sample_window used.
    let corner = |prof: &emem_fetch::cog::CogProfile, ww: u32, hh: u32| {
        let (c, r) = prof.world_to_pixel(cx, cy);
        prof.pixel_to_world(c - (ww as i64) / 2, r - (hh as i64) / 2)
    };
    let (bx0, by0) = corner(&band_prof, w, h);
    let (sx0, sy0) = corner(&scl_prof, w_scl, h_scl);
    let mut out = vec![f32::NAN; (w as usize) * (h as usize)];
    for r in 0..h as i64 {
        for c in 0..w as i64 {
            let bv = band_raw[(r as usize) * (w as usize) + (c as usize)];
            if !bv.is_finite() {
                continue;
            }
            // The band pixel's centre, located in the SCL grid.
            let x = bx0 + (c as f64 + 0.5) * band_m;
            let y = by0 - (r as f64 + 0.5) * band_m;
            let sc = ((x - sx0) / scl_m).floor() as i64;
            let sr = ((sy0 - y) / scl_m).floor() as i64;
            if sc < 0 || sr < 0 || sc >= w_scl as i64 || sr >= h_scl as i64 {
                continue; // no SCL under it -> treat as occluded
            }
            let sclv = scl_raw[(sr as usize) * (w_scl as usize) + (sc as usize)];
            if !sclv.is_finite() {
                continue; // unknown class -> treat as occluded
            }
            let scl = sclv.round();
            if (0.0..=255.0).contains(&scl) && reject.contains(&(scl as u8)) {
                continue; // rejected -> stays NaN
            }
            out[(r as usize) * (w as usize) + (c as usize)] = harmonise(bv, dn_offset);
        }
    }
    let meta = json!({
        "id": item.id,
        "asset": crate::s2_url_for_record(&band_url),
        "scl_asset": crate::s2_url_for_record(&scl_url),
        "catalogue": host,
        "dn_offset": dn_offset,
        "captured_at": item.datetime,
        "cloud_cover": item.cloud_cover,
        "sun_azimuth_deg": item.sun_azimuth,
        "sun_elevation_deg": item.sun_elevation,
        "view_azimuth_deg": item.view_azimuth,
        "view_incidence_deg": item.view_incidence,
        "tslot": scene_tslot(&item.datetime),
    });
    Some((out, meta))
}

pub async fn band_composite(req: BandCompositeReq, s: &AppState) -> Result<JsonValue, ApiError> {
    let started = Instant::now();
    let b = &req.bbox;
    if !(b.min_lat < b.max_lat && b.min_lng < b.max_lng) {
        return Err(bad_request(
            "bbox must satisfy min_lat < max_lat and min_lng < max_lng".into(),
        ));
    }
    if b.min_lat < -90.0 || b.max_lat > 90.0 || b.min_lng < -180.0 || b.max_lng > 180.0 {
        return Err(bad_request("bbox exceeds WGS-84 bounds".into()));
    }
    let Some((band_key, aliases)) = S2_BANDS.iter().find(|(k, _)| *k == req.band) else {
        let names: Vec<&str> = S2_BANDS.iter().map(|(k, _)| *k).collect();
        return Err(bad_request(format!(
            "band `{}` is not rasterizable; this executor serves {}",
            req.band,
            names.join(", ")
        )));
    };
    let (sy, sm, sd) = parse_ymd(&req.start_date)
        .ok_or_else(|| bad_request(format!("start_date `{}` is not YYYY-MM-DD", req.start_date)))?;
    let (ey, em, ed) = parse_ymd(&req.end_date)
        .ok_or_else(|| bad_request(format!("end_date `{}` is not YYYY-MM-DD", req.end_date)))?;
    let start_tslot = crate::days_from_civil(sy, sm, sd).max(0) as u64;
    let end_tslot = crate::days_from_civil(ey, em, ed).max(0) as u64;
    if start_tslot > end_tslot {
        return Err(bad_request(
            "start_date must be on or before end_date".into(),
        ));
    }
    let mut reject_vec = req
        .mask_policy
        .clone()
        .unwrap_or_else(|| DEFAULT_SCL_REJECT.to_vec());
    reject_vec.sort_unstable();
    reject_vec.dedup();
    let reject: std::collections::BTreeSet<u8> = reject_vec.iter().copied().collect();
    let min_valid = req.min_valid_count.unwrap_or(1).max(1);
    let max_scenes = req.max_scenes.unwrap_or(12).clamp(2, MAX_COMPOSITE_SCENES);

    let centre_lat = (b.min_lat + b.max_lat) / 2.0;
    let centre_lng = (b.min_lng + b.max_lng) / 2.0;
    let cli = crate::s2_http_client();

    // ── scenes in the window (newest first). ───────────────────────────────
    let datetime = format!("{}T00:00:00Z/{}T23:59:59Z", req.start_date, req.end_date);
    let mut failures: Vec<String> = Vec::new();
    let mut found = None;
    for host in crate::s2_catalogues() {
        let searched = emem_fetch::stac::search_many_at(
            &cli,
            host,
            "sentinel-2-l2a",
            centre_lng,
            centre_lat,
            &datetime,
            None,
            (max_scenes * 2).min(50),
        )
        .await;
        let signed = match searched {
            Ok(v) if host == emem_fetch::stac::STAC_MPC_V1 => {
                crate::s2_sign_mpc_items(&cli, v).await
            }
            other => other,
        };
        match signed {
            Ok(v) => {
                found = Some((host, v));
                break;
            }
            Err(e) => failures.push(format!("{host}: {e}")),
        }
    }
    let (host, items) = found.ok_or_else(|| {
        upstream_error(format!(
            "stac: every Sentinel-2 catalogue failed: {}",
            failures.join("; ")
        ))
    })?;
    // Anchor on the newest usable scene: it fixes the CRS, the native
    // resolution, and the output grid origin every other member is read onto.
    let anchor = items
        .iter()
        .find(|it| {
            it.epsg.is_some()
                && scl_asset(it).is_some()
                && aliases.iter().any(|a| it.assets.contains_key(*a))
        })
        .ok_or_else(|| {
            upstream_error(format!(
                "no Sentinel-2 scene with the band + SCL assets and a CRS in {}..{}",
                req.start_date, req.end_date
            ))
        })?;
    let epsg = anchor.epsg.unwrap();
    let anchor_url = aliases
        .iter()
        .find_map(|a| anchor.assets.get(*a).cloned())
        .unwrap();

    // ── window geometry from the anchor's grid. ────────────────────────────
    let utm_c = emem_fetch::proj::latlng_to_utm_with_epsg(centre_lat, centre_lng, epsg)
        .ok_or_else(|| upstream_error(format!("epsg {epsg} is not a UTM code")))?;
    let utm_min = emem_fetch::proj::latlng_to_utm_with_epsg(b.min_lat, b.min_lng, epsg)
        .ok_or_else(|| upstream_error("bbox corner outside the scene's UTM zone".into()))?;
    let utm_max = emem_fetch::proj::latlng_to_utm_with_epsg(b.max_lat, b.max_lng, epsg)
        .ok_or_else(|| upstream_error("bbox corner outside the scene's UTM zone".into()))?;
    let anchor_prof = emem_fetch::cog::open_profile(&cli, &anchor_url)
        .await
        .map_err(|e| upstream_error(format!("open anchor COG: {e}")))?;
    let native_m = anchor_prof.pixel_scale.0.abs();
    if !(native_m > 0.0 && native_m.is_finite()) {
        return Err(upstream_error(format!(
            "anchor COG has implausible pixel scale {:?}",
            anchor_prof.pixel_scale
        )));
    }
    let width_px = (((utm_max.easting - utm_min.easting).abs() / native_m).ceil() as u32).max(1);
    let height_px = (((utm_max.northing - utm_min.northing).abs() / native_m).ceil() as u32).max(1);
    if width_px > MAX_SIDE_PX || height_px > MAX_SIDE_PX {
        return Err(bad_request(format!(
            "window is {width_px}x{height_px} px at the band's native {native_m} m; the cap is {MAX_SIDE_PX} per side. Shrink the bbox."
        )));
    }
    let (centre_col, centre_row) = anchor_prof.world_to_pixel(utm_c.easting, utm_c.northing);
    let col0 = centre_col - (width_px as i64) / 2;
    let row0 = centre_row - (height_px as i64) / 2;
    let (ti, tj, tx, ty) = anchor_prof.tiepoint;
    let (sx, sy_scale) = anchor_prof.pixel_scale;
    // First pixel's centre, as the header states (see the single-scene path).
    let half = if anchor_prof.pixel_is_point { 0.0 } else { 0.5 };
    let x0 = tx + (col0 as f64 - ti + half) * sx;
    let y0 = ty - (row0 as f64 - tj + half) * sy_scale.abs();

    // ── read + mask each same-grid member concurrently, then median. ───────
    let members: Vec<&emem_fetch::stac::StacItem> = items
        .iter()
        .filter(|it| {
            it.epsg == Some(epsg)
                && scl_asset(it).is_some()
                && aliases.iter().any(|a| it.assets.contains_key(*a))
        })
        .take(max_scenes)
        .collect();
    let reads = members.iter().map(|it| {
        read_masked_scene(
            &cli,
            host,
            it,
            aliases,
            utm_c.easting,
            utm_c.northing,
            width_px,
            height_px,
            native_m,
            &reject,
        )
    });
    let results = futures_util::future::join_all(reads).await;
    let mut grids: Vec<Vec<f32>> = Vec::new();
    let mut member_sources: Vec<JsonValue> = Vec::new();
    for r in results.into_iter().flatten() {
        grids.push(r.0);
        member_sources.push(r.1);
    }
    if grids.len() < 2 {
        return Err(upstream_error(format!(
            "only {} scene(s) read cleanly at one CRS in the window; a composite needs at least two. Widen the date range.",
            grids.len()
        )));
    }

    let npix = (width_px as usize) * (height_px as usize);
    let mut out = vec![f32::NAN; npix];
    let mut sample: Vec<f32> = Vec::with_capacity(grids.len());
    for p in 0..npix {
        sample.clear();
        for g in &grids {
            let v = g[p];
            if v.is_finite() {
                sample.push(v);
            }
        }
        if (sample.len() as u32) >= min_valid {
            sample.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let n = sample.len();
            // Lower-of-two median for an even count: never average two
            // measurements into a value nobody took.
            out[p] = if n % 2 == 1 {
                sample[n / 2]
            } else {
                sample[n / 2 - 1]
            };
        }
    }

    // ── the canonical artifact. ────────────────────────────────────────────
    let header = GridHeader {
        width: width_px,
        height: height_px,
        epsg,
        nodata: f32::NAN,
        lat0: y0,
        lng0: x0,
        dlat: -sy_scale.abs(),
        dlng: sx,
        channels: 1,
    };
    let artifact_bytes =
        encode_grid(&header, &out).map_err(|e| upstream_error(format!("grid encode: {e:?}")))?;
    let byte_len = artifact_bytes.len();
    let artifact_cid = ARTIFACTS
        .put(&artifact_bytes)
        .map_err(|e| upstream_error(format!("artifact store: {e}")))?;

    // ── sign the composite derivation. ─────────────────────────────────────
    let aoi = aoi_cid(b)?;
    let tslot = scene_tslot(&anchor.datetime);

    // Best-effort per-cell anchors (same policy as band_raster), clamped
    // in-bounds so the spot-check tier can read each anchor's value back out
    // of the composite grid and bridge it to any existing per-cell fact.
    let inset_lat = (b.max_lat - b.min_lat) * 0.02;
    let inset_lng = (b.max_lng - b.min_lng) * 0.02;
    let anchor_points = [
        (b.min_lat + inset_lat, b.min_lng + inset_lng),
        (b.min_lat + inset_lat, b.max_lng - inset_lng),
        (b.max_lat - inset_lat, b.min_lng + inset_lng),
        (b.max_lat - inset_lat, b.max_lng - inset_lng),
        (centre_lat, centre_lng),
    ];
    let anchor_keys: Vec<CanonicalKey> = anchor_points
        .iter()
        .map(|(lat, lng)| CanonicalKey {
            cell: emem_codec::geo::cell64_from_latlng(*lat, *lng),
            band: band_key.to_string(),
            tslot,
        })
        .collect();
    let looked_up = s
        .storage
        .lookup_canonical_many(&anchor_keys)
        .await
        .unwrap_or_else(|_| vec![None; anchor_keys.len()]);
    let mut anchors: Vec<JsonValue> = Vec::with_capacity(anchor_points.len());
    let mut anchor_fact_cids: Vec<FactCid> = Vec::new();
    for (((lat, lng), key), existing) in anchor_points.iter().zip(&anchor_keys).zip(&looked_up) {
        let (row, col) = match emem_fetch::proj::latlng_to_utm_with_epsg(*lat, *lng, epsg) {
            Some(u) => {
                let c = (((u.easting - x0) / sx).round().max(0.0) as u32)
                    .min(width_px.saturating_sub(1));
                let r = (((y0 - u.northing) / sy_scale.abs()).round().max(0.0) as u32)
                    .min(height_px.saturating_sub(1));
                (r, c)
            }
            None => (0, 0),
        };
        let value = grid_value_at(&header, &out, row, col);
        if let Some(cid) = existing {
            anchor_fact_cids.push(cid.clone());
        }
        anchors.push(json!({
            "cell": key.cell,
            "row": row,
            "col": col,
            "value": value,
            "fact_cid": existing.as_ref().map(|c| c.as_str()),
        }));
    }

    let record_body = json!({
        "schema": "emem.field_derivation.v1",
        "aoi": { "bbox": { "min_lat": b.min_lat, "min_lng": b.min_lng, "max_lat": b.max_lat, "max_lng": b.max_lng } },
        "aoi_cid": aoi,
        "band": band_key,
        "tslot": tslot,
        "anchors": anchors,
        "fn_key": "s2_median_composite@1",
        "window": { "start": req.start_date, "end": req.end_date, "start_tslot": start_tslot, "end_tslot": end_tslot },
        "mask_policy": {
            "reject_scl": reject_vec,
            "keep_snow_11": !reject.contains(&11),
            "min_valid_count": min_valid,
        },
        "composite": {
            "method": "per_pixel_lower_of_two_median",
            "member_count": member_sources.len(),
            "nan_excluded_before_count": true,
            "nodata_when_below_min_valid": true,
        },
        "sources": member_sources,
        "artifact": {
            "artifact_cid": artifact_cid,
            "byte_len": byte_len,
            "media_type": GRID_MEDIA_TYPE,
            "grid": {
                "width": width_px, "height": height_px, "epsg": epsg,
                "x0": x0, "y0": y0, "dx": sx, "dy": -sy_scale.abs(),
                "crs_note": "header origin and steps are in the scene's UTM CRS named by epsg; the request bbox is WGS-84",
            },
        },
        "reproducibility": "recompute by reading the pinned member scenes, applying the pinned mask_policy per pixel, and taking the lower-of-two median with min_valid_count. The scene list is pinned, so a re-run does not depend on the catalogue adding scenes.",
    });
    let signed_at = emem_storage::server::iso8601_now();
    let centre_cell = emem_codec::geo::cell64_from_latlng(centre_lat, centre_lng);
    let derivation_fact = DerivativeFact {
        cell: centre_cell.clone(),
        band: "field.composite".to_string(),
        tslot_window: [start_tslot, end_tslot],
        op: "median_composite".to_string(),
        parents: anchor_fact_cids.clone(),
        value: json_to_cbor(&record_body),
        confidence: 1.0,
        derivation: Derivation {
            fn_key: "s2_median_composite@1".to_string(),
            args: None,
        },
        schema_cid: s.manifests.schema_cid.clone(),
        signer: s.identity.pubkey,
        signed_at: signed_at.clone(),
    };
    let att = emem_fact::Attestation::build_and_sign_v1(
        vec![Fact::Derivative(derivation_fact)],
        vec![],
        s.manifests.registry_cid.clone(),
        s.manifests.schema_cid.clone(),
        &s.identity.signing,
        s.identity.epoch,
        signed_at,
        None,
    )
    .map_err(|e| upstream_error(format!("composite attestation: {e}")))?;
    let cids = s
        .storage
        .put_attestation(&att)
        .await
        .map_err(ApiError::from)?;
    let derivation_cid = cids
        .first()
        .map(|c| c.as_str().to_string())
        .ok_or_else(|| {
            upstream_error("store returned no cid for the composite derivation".into())
        })?;

    let binding = FieldBinding {
        aoi_cid: aoi.clone(),
        derivation_cid: derivation_cid.clone(),
    };
    let mut composite_receipt_cids = anchor_fact_cids;
    composite_receipt_cids.push(FactCid::new(derivation_cid.clone()));
    let receipt = s.sign_receipt_field(
        "emem.band_composite",
        vec![centre_cell.clone()],
        composite_receipt_cids,
        false,
        started,
        binding,
    );

    Ok(json!({
        "schema": "emem.band_composite.v1",
        "algorithm_key": "s2_median_composite@1",
        "aoi_cid": aoi,
        "band": band_key,
        "tslot": tslot,
        "window": { "start": req.start_date, "end": req.end_date },
        "member_count": member_sources.len(),
        "grid": { "width": width_px, "height": height_px, "epsg": epsg, "native_m": native_m },
        "artifact": {
            "artifact_cid": artifact_cid,
            "byte_len": byte_len,
            "media_type": GRID_MEDIA_TYPE,
            "url": format!("/v1/artifacts/{artifact_cid}"),
            "eviction_note": "the artifact is evictable; the derivation record pins the scenes, mask policy, and recipe needed to rebuild the identical bytes",
        },
        "derivation": record_body,
        "derivation_cid": derivation_cid,
        "tokens": {
            "raster": format!("emem:raster:{aoi}:{band_key}:{tslot}:{derivation_cid}"),
            "derivation_fact": format!("emem:fact:{centre_cell}:{derivation_cid}"),
        },
        "docs": "https://emem.dev/reference#token-raster",
        "receipt": receipt,
    }))
}

pub async fn post_band_composite(
    State(s): State<AppState>,
    EmemJson(req): EmemJson<BandCompositeReq>,
) -> Result<Json<JsonValue>, ApiError> {
    Ok(Json(band_composite(req, &s).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 40x40 one-strip-per-row deflate TIFF placed like Hansen's 00N_070W
    /// tile at a coarse 0.25° a pixel, valued `(col + row) % 25` as the
    /// lossyear bytes are.
    fn hansen_like_tiff() -> Vec<u8> {
        tile_tiff((-70.0, 0.0), |c, r| ((c + r) % 25) as u8)
    }

    /// A 40x40 one-strip-per-row deflate PixelIsArea TIFF at 0.25° a pixel
    /// whose pixel (0, 0) has its north-west corner at `corner` (lng, lat).
    fn tile_tiff(corner: (f64, f64), value: impl Fn(u32, u32) -> u8) -> Vec<u8> {
        use std::io::Write as _;
        let (w, h) = (40u32, 40u32);
        let mut strips = Vec::new();
        for r in 0..h {
            let row: Vec<u8> = (0..w).map(|c| value(c, r)).collect();
            let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            z.write_all(&row).unwrap();
            strips.push(z.finish().unwrap());
        }
        let mut entries: Vec<(u16, u16, u32, u32)> = vec![
            (256, 4, 1, w),
            (257, 4, 1, h),
            (258, 3, 1, 8),
            (259, 3, 1, 8),
            (277, 3, 1, 1),
            (278, 4, 1, 1),
            (284, 3, 1, 1),
            (317, 3, 1, 1),
            (339, 3, 1, 1),
        ];
        let ifd_len = 2 + 12 * (entries.len() as u32 + 4) + 4;
        let scale_off = 8 + ifd_len;
        let tie_off = scale_off + 24;
        let offs_off = tie_off + 48;
        let cnts_off = offs_off + 4 * h;
        let data_off = cnts_off + 4 * h;
        entries.extend([
            (273, 4, h, offs_off),
            (279, 4, h, cnts_off),
            (33550, 12, 3, scale_off),
            (33922, 12, 6, tie_off),
        ]);
        entries.sort_by_key(|e| e.0);
        let mut buf = Vec::new();
        buf.extend_from_slice(b"II");
        buf.extend_from_slice(&42_u16.to_le_bytes());
        buf.extend_from_slice(&8_u32.to_le_bytes());
        buf.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        for (tag, typ, cnt, val) in &entries {
            buf.extend_from_slice(&tag.to_le_bytes());
            buf.extend_from_slice(&typ.to_le_bytes());
            buf.extend_from_slice(&cnt.to_le_bytes());
            buf.extend_from_slice(&val.to_le_bytes());
        }
        buf.extend_from_slice(&0_u32.to_le_bytes());
        for v in [0.25_f64, 0.25, 0.0, 0.0, 0.0, 0.0, corner.0, corner.1, 0.0] {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        let mut at = data_off;
        for st in &strips {
            buf.extend_from_slice(&at.to_le_bytes());
            at += st.len() as u32;
        }
        for st in &strips {
            buf.extend_from_slice(&(st.len() as u32).to_le_bytes());
        }
        for st in &strips {
            buf.extend_from_slice(st);
        }
        buf.resize(buf.len().max(128 * 1024), 0);
        buf
    }

    fn hansen_like_url(tag: &str) -> (std::path::PathBuf, String) {
        tiff_url(tag, hansen_like_tiff())
    }

    fn tiff_url(tag: &str, bytes: Vec<u8>) -> (std::path::PathBuf, String) {
        let path = std::env::temp_dir().join(format!(
            "emem_forest_{tag}_{}_{}.tif",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::write(&path, bytes).unwrap();
        let url = format!("file://{}", path.display());
        (path, url)
    }

    fn lossyear_req(min_lat: f64, min_lng: f64, max_lat: f64, max_lng: f64) -> BandRasterReq {
        BandRasterReq {
            bbox: BBox {
                min_lat,
                min_lng,
                max_lat,
                max_lng,
            },
            band: "forest_change.lossyear".into(),
            observed_on: None,
        }
    }

    fn lossyear_band() -> &'static ForestBand {
        FOREST_BANDS
            .iter()
            .find(|f| f.band == "forest_change.lossyear")
            .unwrap()
    }

    /// The window is every pixel the bbox touches: a bbox off the pixel
    /// lines reads all three columns it touches, not the two its width
    /// rounds to, and a bbox on the tile's north edge reads no row from
    /// outside the tile.
    #[tokio::test]
    async fn a_forest_window_is_fitted_to_the_bbox() {
        let (path, url) = hansen_like_url("fitted");
        let s = crate::tests::test_app_state();
        let year = |byte: i64| if byte == 0 { 0.0 } else { 2000.0 + byte as f32 };
        // Columns 4.4..6.4 and rows 4.4..6.4 of the 0.25-degree fixture.
        let req = lossyear_req(-1.6, -68.9, -1.1, -68.4);
        let out = forest_raster_from(&req, &s, lossyear_band(), &url, None, Instant::now())
            .await
            .unwrap();
        assert_eq!(
            (out["grid"]["width"].clone(), out["grid"]["height"].clone()),
            (json!(3), json!(3))
        );
        let bytes = ARTIFACTS
            .get(out["artifact"]["artifact_cid"].as_str().unwrap())
            .unwrap()
            .unwrap();
        let (h, v) = emem_codec::grid::decode_grid(&bytes).unwrap();
        for r in 0..3i64 {
            for c in 0..3i64 {
                let want = year(((4 + c) + (4 + r)) % 25);
                assert_eq!(
                    grid_value_at(&h, &v, r as u32, c as u32),
                    Some(want),
                    "row {r} col {c}"
                );
            }
        }
        // On the north edge: rows 0 and 1, both inside the tile.
        let req = lossyear_req(-0.3, -69.0, 0.0, -68.6);
        let out = forest_raster_from(&req, &s, lossyear_band(), &url, None, Instant::now())
            .await
            .unwrap();
        let bytes = ARTIFACTS
            .get(out["artifact"]["artifact_cid"].as_str().unwrap())
            .unwrap()
            .unwrap();
        let (h, v) = emem_codec::grid::decode_grid(&bytes).unwrap();
        assert_eq!(h.height, 2);
        assert!(
            (h.lat0 - (-0.125)).abs() < 1e-9,
            "the first row is the tile's first: {}",
            h.lat0
        );
        assert_eq!(grid_value_at(&h, &v, 0, 0), Some(year(4)));
        let _ = std::fs::remove_file(&path);
    }

    /// Recall signs a forest cell at the release's slot, not 0: the anchor
    /// over that cell cites it.
    #[tokio::test]
    async fn a_forest_anchor_cites_the_fact_recall_signed_for_its_cell() {
        let (path, url) = hansen_like_url("anchor");
        let s = crate::tests::test_app_state();
        let req = lossyear_req(-2.0, -69.0, -1.0, -68.0);
        let centre = emem_codec::geo::cell64_from_latlng(-1.5, -68.5);
        let band = "forest_change.lossyear";
        let fact = Fact::Primary(emem_fact::PrimaryFact {
            cell: centre.clone(),
            band: band.into(),
            tslot: crate::tslot_for_year(FOREST_RELEASE_YEAR, band),
            value: ciborium::Value::Integer(2012.into()),
            unit: Some("year_of_loss".into()),
            confidence: 0.93,
            uncertainty: None,
            sources: vec![emem_fact::Source {
                scheme: "hansen.gfc.v1_13.2025".into(),
                id: url.clone(),
                cid: None,
                hash: None,
                captured_at: None,
                url: Some(url.clone()),
            }],
            derivation: emem_fact::Derivation {
                fn_key: "hansen_gfc_v1_13_pixel@1".into(),
                args: None,
            },
            privacy_class: "public".into(),
            schema_cid: s.manifests.schema_cid.clone(),
            signer: s.identity.pubkey,
            signed_at: "2026-10-28T00:00:00Z".into(),
            served_via: None,
        });
        let cid = crate::sign_and_persist_many(&s, vec![fact], "2026-10-28T00:00:00Z")
            .await
            .unwrap()
            .remove(0);
        let out = forest_raster_from(&req, &s, lossyear_band(), &url, None, Instant::now())
            .await
            .unwrap();
        let cited = out["derivation"]["anchors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["cell"] == json!(centre))
            .map(|a| a["fact_cid"].clone());
        assert_eq!(
            cited,
            Some(json!(cid.as_str())),
            "{}",
            out["derivation"]["anchors"]
        );
        let _ = std::fs::remove_file(&path);
    }

    /// One window of the tile becomes one signed field whose values are the
    /// per-cell facts' values: a lossyear byte reads as its calendar year.
    #[tokio::test]
    async fn a_forest_layer_rasterises_as_one_window_of_its_tile() {
        let path = std::env::temp_dir().join(format!(
            "emem_forest_raster_{}_{}.tif",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::write(&path, hansen_like_tiff()).unwrap();
        let url = format!("file://{}", path.display());
        let req = BandRasterReq {
            bbox: BBox {
                min_lat: -2.0,
                min_lng: -69.0,
                max_lat: -1.0,
                max_lng: -68.0,
            },
            band: "forest_change.lossyear".into(),
            observed_on: None,
        };
        let fb = FOREST_BANDS
            .iter()
            .find(|f| f.band == "forest_change.lossyear")
            .unwrap();
        let s = crate::tests::test_app_state();
        let out = forest_raster_from(&req, &s, fb, &url, None, Instant::now())
            .await
            .unwrap();
        assert_eq!(out["algorithm_key"], "forest_raster@1");
        assert_eq!(out["grid"]["width"], 4);
        assert_eq!(out["grid"]["height"], 4);
        assert!(out["licence"]["terms"]
            .as_str()
            .unwrap()
            .contains("CC BY 4.0"));
        let cid = out["artifact"]["artifact_cid"].as_str().unwrap();
        let bytes = ARTIFACTS.get(cid).unwrap().expect("the artifact is stored");
        let (header, values) = emem_codec::grid::decode_grid(&bytes).unwrap();
        // The bbox is columns 4..8 and rows 4..8 of the tile.
        for r in 0..4u32 {
            for c in 0..4u32 {
                let byte = ((4 + c) + (4 + r)) % 25;
                let want = if byte == 0 { 0.0 } else { 2000.0 + byte as f32 };
                assert_eq!(
                    grid_value_at(&header, &values, r, c),
                    Some(want),
                    "row {r} col {c}"
                );
            }
        }
        assert!(
            (header.lng0 - (-69.0 + 0.125)).abs() < 1e-9,
            "{}",
            header.lng0
        );
        assert!(
            (header.lat0 - (-1.0 - 0.125)).abs() < 1e-9,
            "{}",
            header.lat0
        );
        let token = out["tokens"]["raster"].as_str().unwrap();
        assert!(token.starts_with("emem:raster:") && token.contains(":forest_change.lossyear:0:"));
        let _ = std::fs::remove_file(&path);
    }

    /// A geographic profile whose tiepoint names pixel (0, 0) at `tie`.
    fn lattice(
        tie: (f64, f64),
        scale: f64,
        size: (u32, u32),
        point: bool,
    ) -> emem_fetch::cog::CogProfile {
        emem_fetch::cog::CogProfile {
            width: size.0,
            height: size.1,
            bits_per_sample: 8,
            sample_format: 1,
            compression: 5,
            predictor: 1,
            tile_w: size.0,
            tile_h: 1,
            tile_cols: 1,
            tile_rows: size.1,
            samples_per_pixel: 1,
            planar_config: 1,
            tile_offsets: Vec::new(),
            tile_byte_counts: Vec::new(),
            pixel_scale: (scale, scale),
            tiepoint: (0.0, 0.0, tie.0, tie.1),
            epsg: None,
            nodata: None,
            pixel_is_point: point,
        }
    }

    fn bbox(min_lat: f64, min_lng: f64, max_lat: f64, max_lng: f64) -> BBox {
        BBox {
            min_lat,
            min_lng,
            max_lat,
            max_lng,
        }
    }

    /// A bbox names each tile square it touches, north first, with a probe
    /// inside both; one that ends on a tile line names one square.
    #[test]
    fn forest_squares_name_each_tile_a_bbox_touches() {
        let keys = |b: &BBox| -> Vec<(i64, i64)> {
            let sq = forest_squares(b);
            for ((ky, kx), (lat, lng)) in &sq {
                assert!(
                    (b.min_lat..=b.max_lat).contains(lat) && (b.min_lng..=b.max_lng).contains(lng),
                    "probe ({lat}, {lng}) outside {b:?}"
                );
                assert_eq!((tile_row_key(*lat), tile_col_key(*lng)), (*ky, *kx));
            }
            sq.into_iter().map(|(k, _)| k).collect()
        };
        assert_eq!(keys(&bbox(-1.6, -68.9, -1.1, -68.4)), vec![(0, -7)]);
        assert_eq!(keys(&bbox(-0.3, -60.4, 0.0, -60.0)), vec![(0, -7)]);
        assert_eq!(keys(&bbox(10.0, 30.0, 10.4, 30.5)), vec![(2, 3)]);
        // 4.8 km a side on the equator at 19.5° E.
        let d = 0.0216;
        assert_eq!(keys(&bbox(-d, 19.5 - d, d, 19.5 + d)), vec![(1, 1), (0, 1)]);
        assert_eq!(keys(&bbox(-1.5, 19.98, -1.45, 20.02)), vec![(0, 1), (0, 2)]);
        assert_eq!(
            keys(&bbox(-0.01, 19.99, 0.01, 20.01)),
            vec![(1, 1), (1, 2), (0, 1), (0, 2)]
        );
        assert_eq!(forest_squares(&bbox(-15.0, -5.0, 15.0, 5.0)).len(), 8);
    }

    /// The 10° keys group points exactly as each layer's own tile names do,
    /// so one key is one tile URL on every forest layer.
    #[test]
    fn tile_keys_agree_with_every_layers_tile_names() {
        let lats = [
            -29.99, -20.0, -10.0, -9.999, -0.001, 0.0, 0.001, 9.999, 10.0, 10.001, 29.9,
        ];
        let lngs = [
            -70.0, -69.999, -60.0001, -60.0, -0.001, 0.0, 0.001, 19.999, 20.0, 170.5,
        ];
        type Tags = (String, String);
        type Namer = fn(f64, f64) -> Tags;
        let namers: [(&str, Namer); 3] = [
            ("hansen", emem_fetch::hansen_gfc::tile_corner_tags),
            ("jrc_gfc2020", emem_fetch::jrc_gfc2020::tile_corner_tags),
            ("jrc_tmf", |lat, lng| {
                (
                    emem_fetch::jrc_tmf::tile_lat_tag(lat).unwrap(),
                    emem_fetch::jrc_tmf::tile_lng_tag(lng).unwrap(),
                )
            }),
        ];
        for (name, namer) in namers {
            let mut by_key = std::collections::HashMap::new();
            let mut by_tag = std::collections::HashMap::new();
            for lat in lats {
                for lng in lngs {
                    let key = (tile_row_key(lat), tile_col_key(lng));
                    let tag = namer(lat, lng);
                    assert_eq!(
                        *by_key.entry(key).or_insert_with(|| tag.clone()),
                        tag,
                        "{name} at ({lat}, {lng})"
                    );
                    assert_eq!(
                        *by_tag.entry(tag).or_insert(key),
                        key,
                        "{name} at ({lat}, {lng})"
                    );
                }
            }
        }
    }

    /// A window across the equator, a 10° meridian or both splits at the
    /// tile lines into the rectangles each tile supplies, placed by the
    /// whole-pixel offset of each tile's lattice.
    #[test]
    fn a_window_splits_at_the_tile_lines_onto_one_lattice() {
        let south = lattice((-70.0, 0.0), 0.00025, (40_000, 40_000), false);
        let north = lattice((-70.0, 10.0), 0.00025, (40_000, 40_000), false);
        let east = lattice((-60.0, 0.0), 0.00025, (40_000, 40_000), false);
        let piece = |tile, row0, col0, height, width| Piece {
            tile,
            row0,
            col0,
            height,
            width,
        };
        // Rows -2..2 of the south tile's lattice: two from the north tile.
        let keys = [(1, -7), (0, -7)];
        assert_eq!(
            plan_pieces(&south, 100, -2, 3, 4, &keys).unwrap(),
            vec![piece(0, 0, 0, 2, 3), piece(1, 2, 0, 2, 3)]
        );
        // The north tile's last rows are the rows just above the equator.
        let (dc, dr) = lattice_offset(&south, &north).unwrap();
        assert_eq!((dc, dr), (0, -40_000));
        assert_eq!((100 - dc, -2 - dr), (100, 39_998));
        // Columns 39998..40002: the -60° meridian is column 40000.
        let keys = [(0, -7), (0, -6)];
        assert_eq!(
            plan_pieces(&south, 39_998, 10, 4, 2, &keys).unwrap(),
            vec![piece(0, 0, 0, 2, 2), piece(1, 0, 2, 2, 2)]
        );
        assert_eq!(lattice_offset(&south, &east).unwrap(), (40_000, 0));
        // Both lines: four rectangles, north-west first.
        let keys = [(1, -7), (1, -6), (0, -7), (0, -6)];
        assert_eq!(
            plan_pieces(&south, 39_999, -1, 2, 2, &keys).unwrap(),
            vec![
                piece(0, 0, 0, 1, 1),
                piece(1, 0, 1, 1, 1),
                piece(2, 1, 0, 1, 1),
                piece(3, 1, 1, 1, 1),
            ]
        );
        // With one tile every pixel is its, inside its image or not.
        assert_eq!(
            plan_pieces(&south, 100, -2, 3, 4, &[(0, -7)]).unwrap(),
            vec![piece(0, 0, 0, 4, 3)]
        );
    }

    /// Tiles on one lattice join at a whole-pixel offset, from the published
    /// headers of each layer; a half-pixel shift, a raster type read wrongly,
    /// another pixel size or CRS is refused.
    #[test]
    fn lattices_join_whole_or_are_refused() {
        // JRC TMF (Source Cooperative mirror): N0_W70, N0_E10 and N10_W70,
        // whose tiles overlap and start a pixel apart.
        let s = 0.00026949458523585647;
        let tmf = |x, y, w, h| lattice((x, y), s, (w, h), false);
        let n0_w70 = tmf(-70.00014053667277, 0.0, 37_108, 37_246);
        let n0_e10 = tmf(9.99986607976169, s, 37_107, 37_247);
        let n10_w70 = tmf(-70.00014053667277, 10.03759532169471, 37_108, 37_247);
        assert_eq!(lattice_offset(&n0_w70, &n0_e10).unwrap(), (296_852, -1));
        assert_eq!(lattice_offset(&n0_w70, &n10_w70).unwrap(), (0, -37_246));
        // JRC GFC2020 V4: N0_E20 and N10_E20 at 1/12000°.
        let g = 8.333333333333333e-5;
        let n0_e20 = lattice((20.0, 0.0), g, (120_000, 120_000), false);
        let n10_e20 = lattice((20.0, 10.0), g, (120_000, 120_000), false);
        assert_eq!(lattice_offset(&n0_e20, &n10_e20).unwrap(), (0, -120_000));

        let hansen = lattice((-70.0, 0.0), 0.00025, (40_000, 40_000), false);
        // A PixelIsPoint tiepoint names the centre: half a pixel in, same grid.
        let centred = lattice(
            (-60.0 + 0.000125, -0.000125),
            0.00025,
            (40_000, 40_000),
            true,
        );
        assert_eq!(lattice_offset(&hansen, &centred).unwrap(), (40_000, 0));
        for (what, piece) in [
            (
                "half a pixel east",
                lattice((-59.999875, 0.0), 0.00025, (40_000, 40_000), false),
            ),
            (
                "a corner read as a centre",
                lattice((-60.0, 0.0), 0.00025, (40_000, 40_000), true),
            ),
            (
                "another pixel size",
                lattice((-60.0, 0.0), 0.00025000025, (40_000, 40_000), false),
            ),
        ] {
            assert!(
                lattice_offset(&hansen, &piece).is_err(),
                "{what} was joined"
            );
        }
        let mut utm = lattice((-60.0, 0.0), 0.00025, (40_000, 40_000), false);
        utm.epsg = Some(32_633);
        assert!(lattice_offset(&hansen, &utm).is_err());
    }

    fn year_of(byte: u32) -> f32 {
        2000.0 + byte as f32
    }

    fn decode(out: &JsonValue) -> (GridHeader, Vec<f32>) {
        let bytes = ARTIFACTS
            .get(out["artifact"]["artifact_cid"].as_str().unwrap())
            .unwrap()
            .unwrap();
        emem_codec::grid::decode_grid(&bytes).unwrap()
    }

    /// A bbox across the equator is one grid on the south tile's lattice:
    /// its top rows are the north tile's last, the rest the south tile's
    /// first, and the record cites both tiles and where each one sits.
    #[tokio::test]
    async fn a_bbox_across_the_equator_joins_both_tiles() {
        let north_v = |c: u32, r: u32| (13 + (c + r) % 12) as u8;
        let south_v = |c: u32, r: u32| (1 + (c + r) % 12) as u8;
        let (np, north) = tiff_url("eq_north", tile_tiff((-70.0, 10.0), north_v));
        let (sp, south) = tiff_url("eq_south", tile_tiff((-70.0, 0.0), south_v));
        let tiles = [
            ForestTile {
                key: (1, -7),
                url: north.clone(),
                jrc_version: None,
            },
            ForestTile {
                key: (0, -7),
                url: south.clone(),
                jrc_version: None,
            },
        ];
        let s = crate::tests::test_app_state();
        // Rows -1.6..2.4 and columns 4.4..6.4 on the south tile's lattice.
        let req = lossyear_req(-0.6, -68.9, 0.4, -68.4);
        let out = forest_raster_tiles(&req, &s, lossyear_band(), &tiles, 1, Instant::now())
            .await
            .unwrap();
        assert_eq!(out["algorithm_key"], "forest_raster@1");
        let (h, v) = decode(&out);
        assert_eq!((h.width, h.height), (3, 5));
        assert!((h.lat0 - 0.375).abs() < 1e-9, "{}", h.lat0);
        assert!((h.lng0 - (-68.875)).abs() < 1e-9, "{}", h.lng0);
        for r in 0..5u32 {
            for c in 0..3u32 {
                let want = if r < 2 {
                    year_of(north_v(4 + c, 38 + r).into())
                } else {
                    year_of(south_v(4 + c, r - 2).into())
                };
                assert_eq!(grid_value_at(&h, &v, r, c), Some(want), "row {r} col {c}");
            }
        }
        let record = &out["derivation"];
        assert_eq!(record["mosaic"]["pieces"], 2);
        assert_eq!(record["mosaic"]["lattice_of"], json!(south));
        let sources = record["sources"].as_array().unwrap();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0]["id"], json!(north));
        assert_eq!(
            sources[0]["window"],
            json!({"grid_row0": 0, "grid_col0": 0, "height": 2, "width": 3, "tile_row0": 38, "tile_col0": 4})
        );
        assert_eq!(sources[1]["id"], json!(south));
        assert_eq!(
            sources[1]["window"],
            json!({"grid_row0": 2, "grid_col0": 0, "height": 3, "width": 3, "tile_row0": 0, "tile_col0": 4})
        );
        let token = out["tokens"]["raster"].as_str().unwrap();
        assert!(token.starts_with("emem:raster:") && token.contains(":forest_change.lossyear:0:"));
        let _ = std::fs::remove_file(&np);
        let _ = std::fs::remove_file(&sp);
    }

    /// A bbox across a 10° meridian: the west tile's last columns, then the
    /// east tile's first.
    #[tokio::test]
    async fn a_bbox_across_a_ten_degree_meridian_joins_both_tiles() {
        let west_v = |c: u32, r: u32| (1 + (c + r) % 12) as u8;
        let east_v = |c: u32, r: u32| (13 + (c + 2 * r) % 12) as u8;
        let (wp, west) = tiff_url("mer_west", tile_tiff((-70.0, 0.0), west_v));
        let (ep, east) = tiff_url("mer_east", tile_tiff((-60.0, 0.0), east_v));
        let tiles = [
            ForestTile {
                key: (0, -7),
                url: west.clone(),
                jrc_version: None,
            },
            ForestTile {
                key: (0, -6),
                url: east.clone(),
                jrc_version: None,
            },
        ];
        let s = crate::tests::test_app_state();
        // Columns 37.6..41.6 and rows 4.4..6.4 of the west tile's lattice.
        let req = lossyear_req(-1.6, -60.6, -1.1, -59.6);
        let out = forest_raster_tiles(&req, &s, lossyear_band(), &tiles, 0, Instant::now())
            .await
            .unwrap();
        let (h, v) = decode(&out);
        assert_eq!((h.width, h.height), (5, 3));
        assert!((h.lng0 - (-70.0 + 37.5 * 0.25)).abs() < 1e-9, "{}", h.lng0);
        for r in 0..3u32 {
            for c in 0..5u32 {
                let want = if c < 3 {
                    year_of(west_v(37 + c, 4 + r).into())
                } else {
                    year_of(east_v(c - 3, 4 + r).into())
                };
                assert_eq!(grid_value_at(&h, &v, r, c), Some(want), "row {r} col {c}");
            }
        }
        let sources = out["derivation"]["sources"].as_array().unwrap();
        let ids: Vec<&JsonValue> = sources.iter().map(|src| &src["id"]).collect();
        assert_eq!(ids, vec![&json!(west), &json!(east)]);
        assert_eq!(sources[1]["window"]["tile_col0"], 0);
        let _ = std::fs::remove_file(&wp);
        let _ = std::fs::remove_file(&ep);
    }

    /// A tile half a pixel off the lattice is refused, never resampled.
    #[tokio::test]
    async fn tiles_on_two_lattices_are_refused() {
        let (wp, west) = tiff_url("off_west", tile_tiff((-70.0, 0.0), |_, _| 3));
        let (ep, east) = tiff_url("off_east", tile_tiff((-59.875, 0.0), |_, _| 4));
        let tiles = [
            ForestTile {
                key: (0, -7),
                url: west,
                jrc_version: None,
            },
            ForestTile {
                key: (0, -6),
                url: east.clone(),
                jrc_version: None,
            },
        ];
        let s = crate::tests::test_app_state();
        let req = lossyear_req(-1.6, -60.6, -1.1, -59.6);
        let res = forest_raster_tiles(&req, &s, lossyear_band(), &tiles, 0, Instant::now()).await;
        let Err(ApiError(status, body)) = res else {
            panic!("two lattices were joined");
        };
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(
            body.message.contains(&east) && body.message.contains("not on the pixel lattice"),
            "{}",
            body.message
        );
        let _ = std::fs::remove_file(&wp);
        let _ = std::fs::remove_file(&ep);
    }

    /// More tiles than a 2 x 2 block is a 400 that names the cap, before
    /// any tile is named or read.
    #[tokio::test]
    async fn more_than_four_tiles_is_refused() {
        let s = crate::tests::test_app_state();
        let res = forest_raster(lossyear_req(-15.0, -5.0, 15.0, 5.0), &s).await;
        let Err(ApiError(status, body)) = res else {
            panic!("eight tiles were read");
        };
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            body.message.contains("touches 8") && body.message.contains("at most 4"),
            "{}",
            body.message
        );
    }

    /// A bbox inside one tile keeps the one-source record it always had.
    #[tokio::test]
    async fn one_tile_keeps_its_one_source_record() {
        let (path, url) = hansen_like_url("one_source");
        let s = crate::tests::test_app_state();
        let fb = lossyear_band();
        let req = lossyear_req(-1.6, -68.9, -1.1, -68.4);
        let out = forest_raster_from(&req, &s, fb, &url, None, Instant::now())
            .await
            .unwrap();
        assert_eq!(
            out["derivation"]["sources"],
            json!([{
                "scheme": fb.scheme,
                "id": url,
                "asset": url,
                "licence": { "terms": fb.licence, "attribution": fb.attribution },
            }])
        );
        assert!(out["derivation"].get("mosaic").is_none());
        let _ = std::fs::remove_file(&path);
    }

    /// Both catalogues land on one pixel scale: an ESA DN from Planetary
    /// Computer plus its offset equals Element84's harmonised DN, and no-data
    /// stays no-data rather than becoming -1000.
    #[test]
    fn either_catalogue_lands_on_one_scale() {
        assert_eq!(harmonise(1834.0, -1000.0), harmonise(834.0, 0.0));
        assert_eq!(harmonise(0.0, -1000.0), 0.0);
        let item: emem_fetch::stac::StacItem = serde_json::from_value(json!({
            "id": "x", "datetime": "2026-09-01T00:00:00Z", "collection": "sentinel-2-l2a",
            "assets": {"SCL": "https://example.invalid/SCL.tif"},
        }))
        .unwrap();
        assert_eq!(
            scl_asset(&item).as_deref(),
            Some("https://example.invalid/SCL.tif")
        );
    }

    #[test]
    fn aoi_cid_is_stable_and_order_sensitive() {
        let a = BBox {
            min_lat: 12.9,
            min_lng: 77.5,
            max_lat: 13.0,
            max_lng: 77.6,
        };
        let one = aoi_cid(&a).unwrap();
        let two = aoi_cid(&a).unwrap();
        assert_eq!(one, two, "same bbox must always name the same field");
        assert_eq!(one.len(), 52);
        let b = BBox {
            min_lat: 12.9,
            min_lng: 77.5,
            max_lat: 13.0,
            max_lng: 77.7,
        };
        assert_ne!(one, aoi_cid(&b).unwrap());
    }

    #[test]
    fn raster_token_parses_and_refuses_precisely() {
        let cid = "a".repeat(20) + &"b".repeat(32); // 52 lowercase chars
        let tok = format!("emem:raster:{cid}:s2.B04:20650:{cid}");
        let (aoi, band, tslot, dcid) = parse_raster_token(&tok).unwrap();
        assert_eq!(aoi, cid);
        assert_eq!(band, "s2.B04");
        assert_eq!(tslot, 20650);
        assert_eq!(dcid, cid);
        assert!(parse_raster_token("emem:fact:x:y").is_err());
        assert!(parse_raster_token(&format!("emem:raster:{cid}:s2.B04:notanum:{cid}")).is_err());
        assert!(parse_raster_token(&format!("emem:raster:SHOUT:s2.B04:1:{cid}")).is_err());
        assert!(parse_raster_token(&format!("emem:raster:{cid}:s2.B04:1")).is_err());
    }

    #[test]
    fn cube_token_parses_and_refuses_precisely() {
        let cid = "a".repeat(20) + &"b".repeat(32); // 52 lowercase chars
        let tok = format!("emem:cube:{cid}:s2.B08:20600..20651:{cid}");
        let (aoi, band, lo, hi, dcid) = parse_cube_token(&tok).unwrap();
        assert_eq!(aoi, cid);
        assert_eq!(band, "s2.B08");
        assert_eq!(lo, 20600);
        assert_eq!(hi, 20651);
        assert_eq!(dcid, cid);
        // wrong prefix, missing range, inverted range, non-numeric bound, bad cid
        assert!(parse_cube_token(&format!("emem:raster:{cid}:s2.B04:1:{cid}")).is_err());
        assert!(parse_cube_token(&format!("emem:cube:{cid}:s2.B04:20600:{cid}")).is_err());
        assert!(parse_cube_token(&format!("emem:cube:{cid}:s2.B04:20651..20600:{cid}")).is_err());
        assert!(parse_cube_token(&format!("emem:cube:{cid}:s2.B04:aa..bb:{cid}")).is_err());
        assert!(parse_cube_token(&format!("emem:cube:SHOUT:s2.B04:1..2:{cid}")).is_err());
    }

    #[test]
    fn cube_cid_is_deterministic_and_order_sensitive() {
        let a = "a".repeat(52);
        let b = "b".repeat(52);
        let one = cube_cid_of(&[a.clone(), b.clone()]).unwrap();
        let two = cube_cid_of(&[a.clone(), b.clone()]).unwrap();
        assert_eq!(one, two, "same ordered membership must name the same cube");
        assert_eq!(one.len(), 52);
        let flipped = cube_cid_of(&[b, a]).unwrap();
        assert_ne!(
            one, flipped,
            "reordering the slices must change the cube_cid"
        );
    }

    #[test]
    fn band_allowlist_is_the_documented_six() {
        let keys: Vec<&str> = S2_BANDS.iter().map(|(k, _)| *k).collect();
        assert_eq!(
            keys,
            vec!["s2.B02", "s2.B03", "s2.B04", "s2.B08", "s2.B11", "s2.B12"]
        );
    }
}
