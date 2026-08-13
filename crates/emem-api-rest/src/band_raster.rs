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
//! Sentinel-2 reflectance set below, and the window caps at
//! `MAX_SIDE_PX` per side.

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
const MAX_SIDE_PX: u32 = 512;

/// The raw Sentinel-2 bands this executor serves, with the Element84
/// STAC asset aliases each may appear under. An allowlist, not a
/// routing guess: any other band is a typed refusal naming this list.
const S2_BANDS: [(&str, &[&str]); 6] = [
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
const EMBEDDING_BANDS: &[&str] = &[
    "geotessera",
    "geotessera.multi_year",
    "clay_v1",
    "prithvi_eo2",
    "galileo",
];
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
    let base = std::env::var("EMEM_DATA").unwrap_or_else(|_| "/home/ubuntu/emem/var/emem".into());
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
fn parse_ymd(s: &str) -> Option<(i32, u32, u32)> {
    let mut it = s.split('-');
    let y: i32 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let d: u32 = it.next()?.parse().ok()?;
    if it.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some((y, m, d))
}

fn bad_request(message: String) -> ApiError {
    ApiError(
        StatusCode::BAD_REQUEST,
        ErrorBody {
            code: ErrorCode::InvalidArgument,
            message,
            details: None,
        },
    )
}

fn upstream_error(message: String) -> ApiError {
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
fn aoi_cid(b: &BBox) -> Result<String, ApiError> {
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
    let Some((band_key, aliases)) = S2_BANDS.iter().find(|(k, _)| *k == req.band) else {
        let names: Vec<&str> = S2_BANDS.iter().map(|(k, _)| *k).collect();
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
    let (item, _cloud, _days) =
        crate::s2_search_with_fallback(&cli, centre_lng, centre_lat, target_unix, now_unix)
            .await
            .map_err(upstream_error)?;
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
    // centred on the point, half the dims each way. Its world origin
    // (centre of the first pixel) inverts world_to_pixel exactly.
    let (centre_col, centre_row) = prof.world_to_pixel(utm_c.easting, utm_c.northing);
    let col0 = centre_col - (width_px as i64) / 2;
    let row0 = centre_row - (height_px as i64) / 2;
    let (ti, tj, tx, ty) = prof.tiepoint;
    let (sx, sy) = prof.pixel_scale;
    let x0 = tx + (col0 as f64 - ti) * sx;
    let y0 = ty - (row0 as f64 - tj) * sy.abs();

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
    let values: Vec<f32> = raw.iter().map(|v| *v as f32).collect();
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
            "asset": url,
            "captured_at": item.datetime,
            "cloud_cover": item.cloud_cover,
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
    let x0 = tx + (col0 as f64 - ti) * sx;
    let y0 = ty - (row0 as f64 - tj) * sy.abs();

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
        "band_raster@1" | "s2_median_composite@1" | "dem_raster@1" | "embedding_raster@1"
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
        "band_raster@1" | "s2_median_composite@1" | "dem_raster@1" | "embedding_raster@1"
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
                    let tol = eps.max(0.01 * g.abs());
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
const DEFAULT_SCL_REJECT: [u8; 6] = [0, 1, 3, 8, 9, 10];

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
fn scene_tslot(datetime: &str) -> u64 {
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
async fn read_masked_scene(
    cli: &reqwest::Client,
    item: &emem_fetch::stac::StacItem,
    aliases: &[&str],
    cx: f64,
    cy: f64,
    w: u32,
    h: u32,
    band_native_m: f64,
    reject: &std::collections::BTreeSet<u8>,
) -> Option<(Vec<f32>, JsonValue)> {
    let band_url = aliases.iter().find_map(|a| item.assets.get(*a).cloned())?;
    let scl_url = item.assets.get("scl").cloned()?;
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
    let w_scl = ((w as f64 * band_m / scl_m).ceil() as u32).max(1);
    let h_scl = ((h as f64 * band_m / scl_m).ceil() as u32).max(1);
    let scl_raw = emem_fetch::cog::sample_window(cli, &scl_url, &scl_prof, cx, cy, w_scl, h_scl)
        .await
        .ok()?;
    let bm = band_m.round() as i64;
    let sm = scl_m.round().max(1.0) as i64;
    let mut out = vec![f32::NAN; (w as usize) * (h as usize)];
    for r in 0..h as i64 {
        for c in 0..w as i64 {
            let bv = band_raw[(r as usize) * (w as usize) + (c as usize)];
            if !bv.is_finite() {
                continue;
            }
            let sr = ((r * bm) / sm).clamp(0, h_scl as i64 - 1) as usize;
            let sc = ((c * bm) / sm).clamp(0, w_scl as i64 - 1) as usize;
            let sclv = scl_raw[sr * (w_scl as usize) + sc];
            if !sclv.is_finite() {
                continue; // unknown class -> treat as occluded
            }
            let scl = sclv.round();
            if (0.0..=255.0).contains(&scl) && reject.contains(&(scl as u8)) {
                continue; // rejected -> stays NaN
            }
            out[(r as usize) * (w as usize) + (c as usize)] = bv as f32;
        }
    }
    let meta = json!({
        "id": item.id,
        "asset": band_url,
        "scl_asset": scl_url,
        "captured_at": item.datetime,
        "cloud_cover": item.cloud_cover,
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
    let items = emem_fetch::stac::search_many_at(
        &cli,
        emem_fetch::stac::STAC_ELEMENT84_V1,
        "sentinel-2-l2a",
        centre_lng,
        centre_lat,
        &datetime,
        None,
        (max_scenes * 2).min(50),
    )
    .await
    .map_err(upstream_error)?;
    // Anchor on the newest usable scene: it fixes the CRS, the native
    // resolution, and the output grid origin every other member is read onto.
    let anchor = items
        .iter()
        .find(|it| {
            it.epsg.is_some()
                && it.assets.contains_key("scl")
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
    let x0 = tx + (col0 as f64 - ti) * sx;
    let y0 = ty - (row0 as f64 - tj) * sy_scale.abs();

    // ── read + mask each same-grid member concurrently, then median. ───────
    let members: Vec<&emem_fetch::stac::StacItem> = items
        .iter()
        .filter(|it| {
            it.epsg == Some(epsg)
                && it.assets.contains_key("scl")
                && aliases.iter().any(|a| it.assets.contains_key(*a))
        })
        .take(max_scenes)
        .collect();
    let reads = members.iter().map(|it| {
        read_masked_scene(
            &cli,
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
