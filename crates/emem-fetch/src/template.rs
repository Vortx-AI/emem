//! URL-template expansion. Variables in `{name}` form are resolved against
//! the FetchRequest (cell, bbox, tslot, channels, vars).
//!
//! Resolved variables:
//!   - `{cell64}`  — cell64 string of the request cell
//!   - `{tslot}`   — integer tslot
//!   - `{year}`    — derived from tslot (slow tempo)
//!   - `{month}`   — derived from tslot (medium tempo, zero-padded)
//!   - `{day}`     — derived from tslot (fast tempo, zero-padded)
//!   - `{channel}` — first channel from request
//!   - **bbox-derived** (require `req.bbox`):
//!       - `{lat_band}`     — Cop-DEM 1° lat band: `N42`, `S16`
//!       - `{lon_band}`     — Cop-DEM 1° lon band: `E090`, `W120`
//!       - `{lat_top10}`    — JRC GSW 10° tile top:  `50N`, `30S`
//!       - `{lon_left10}`   — JRC GSW 10° tile left: `100W`, `010E`
//!       - `{tile_id}`      — ESA WorldCover 3° tile id: `N42W096`
//!       - `{bbox_csv}`     — STAC search: `lon_min,lat_min,lon_max,lat_max`
//!       - `{lat_center}`, `{lon_center}` — bbox centroid, 6dp
//!   - any caller-provided `vars` entries (override built-ins)
//!
//! Custom variables in `vars` override built-ins.

use crate::{FetchError, FetchRequest};

/// Expand `{vars}` in a URL template against the FetchRequest.
pub fn expand(template: &str, req: &FetchRequest) -> Result<String, FetchError> {
    let mut out = String::with_capacity(template.len() + 32);
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '{' {
            out.push(c);
            continue;
        }
        let mut name = String::new();
        for nc in chars.by_ref() {
            if nc == '}' {
                break;
            }
            name.push(nc);
        }
        let value = resolve(&name, req)?;
        out.push_str(&value);
    }
    Ok(out)
}

fn resolve(name: &str, req: &FetchRequest) -> Result<String, FetchError> {
    // Caller-supplied vars always win.
    if let Some(v) = req.vars.get(name) {
        return Ok(v.clone());
    }
    Ok(match name {
        "tslot" => req.tslot.0.to_string(),
        "channel" => req.channels.first().cloned().unwrap_or_default(),
        // Time fields use simplified epoch math — emem-core::tslot owns
        // the canonical seconds-per-slot constants.
        "year" => derive_year(req.tslot).to_string(),
        "month" => format!("{:02}", derive_month_in_year(req.tslot)),
        "day" => format!("{:02}", derive_day(req.tslot)),
        // bbox-derived tile-naming vars
        "lat_band" => bbox(req, name)?.lat_band_1deg(),
        "lon_band" => bbox(req, name)?.lon_band_1deg(),
        "lat_top10" => bbox(req, name)?.lat_top_10deg(),
        "lon_left10" => bbox(req, name)?.lon_left_10deg(),
        "hansen_lat_band" => bbox(req, name)?.hansen_lat_band(),
        "hansen_lon_band" => bbox(req, name)?.hansen_lon_band(),
        "tile_id" => bbox(req, name)?.worldcover_tile_id(),
        "bbox_csv" => bbox(req, name)?.to_csv(),
        "lat_center" => format!("{:.6}", bbox(req, name)?.center().0),
        "lon_center" => format!("{:.6}", bbox(req, name)?.center().1),
        "cell64" => emem_codec::to_cell64(req.cell),
        other => return Err(FetchError::MissingVariable(other.into())),
    })
}

/// Pull the bbox or fail with a clear missing-variable error naming the
/// var that triggered the lookup.
fn bbox<'a>(req: &'a FetchRequest, requested_var: &str) -> Result<&'a emem_core::Bbox, FetchError> {
    req.bbox.as_ref().ok_or_else(|| FetchError::MissingVariable(
        format!("{requested_var} (template referenced a bbox-derived variable but FetchRequest.bbox is None)")
    ))
}

fn derive_year(t: emem_core::Tslot) -> i32 {
    // tslot for `slow` tempo = years since epoch (2026-01-01).
    2026 + t.0 as i32
}

fn derive_month_in_year(t: emem_core::Tslot) -> u32 {
    // tslot for `medium` tempo = months since epoch.
    1 + (t.0 as u32 % 12)
}

fn derive_day(t: emem_core::Tslot) -> u32 {
    // tslot for `fast` tempo = days since the emem epoch.
    // Emit day-of-month for use in path templates; downstream callers that
    // need the absolute date should pass `day` explicitly via `vars`.
    1 + (t.0 as u32 % 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use emem_core::{Bbox, Cell, Tslot};

    fn req_with_bbox(bbox: Bbox) -> FetchRequest {
        FetchRequest {
            scheme: "test".into(),
            cell: Cell::from_raw(0),
            bbox: Some(bbox),
            tslot: Tslot(0),
            channels: vec![],
            vars: Default::default(),
        }
    }

    #[test]
    fn copdem_url_resolves_for_iowa() {
        let b = Bbox::new(42.01, 42.05, -93.49, -93.44).unwrap();
        let req = req_with_bbox(b);
        let tmpl = "https://copernicus-dem-30m.s3.amazonaws.com/Copernicus_DSM_COG_10_{lat_band}_00_{lon_band}_00_DEM/Copernicus_DSM_COG_10_{lat_band}_00_{lon_band}_00_DEM.tif";
        let url = expand(tmpl, &req).unwrap();
        // The Cop-DEM tile filename for IOWA (lat 42, lon -94 band) is
        // `Copernicus_DSM_COG_10_N42_00_W094_00_DEM.tif`.
        assert!(url.contains("N42_00_W094_00"), "got url: {url}");
    }

    #[test]
    fn jrc_gsw_url_resolves_for_iowa() {
        let b = Bbox::new(42.01, 42.05, -93.49, -93.44).unwrap();
        let req = req_with_bbox(b);
        let tmpl = "https://storage.googleapis.com/global-surface-water/downloads2021/occurrence/occurrence_{lon_left10}_{lat_top10}v1_4_2021.tif";
        let url = expand(tmpl, &req).unwrap();
        assert!(url.contains("occurrence_100W_50N"), "got url: {url}");
    }

    #[test]
    fn worldcover_tile_id_resolves() {
        let b = Bbox::new(42.01, 42.05, -93.49, -93.44).unwrap();
        let req = req_with_bbox(b);
        let tmpl = "https://example/{tile_id}.tif";
        let url = expand(tmpl, &req).unwrap();
        assert_eq!(url, "https://example/N42W096.tif");
    }

    #[test]
    fn missing_bbox_yields_helpful_error() {
        let req = FetchRequest {
            scheme: "test".into(),
            cell: Cell::from_raw(0),
            bbox: None,
            tslot: Tslot(0),
            channels: vec![],
            vars: Default::default(),
        };
        let err = expand("https://x/{lat_band}.tif", &req).unwrap_err();
        match err {
            FetchError::MissingVariable(msg) => assert!(msg.starts_with("lat_band")),
            other => panic!("expected MissingVariable, got {other:?}"),
        }
    }

    #[test]
    fn caller_var_overrides_builtin() {
        let mut req = FetchRequest {
            scheme: "test".into(),
            cell: Cell::from_raw(0),
            bbox: None,
            tslot: Tslot(0),
            channels: vec![],
            vars: Default::default(),
        };
        req.vars.insert("year".into(), "2024".into());
        let url = expand("v={year}", &req).unwrap();
        assert_eq!(url, "v=2024");
    }
}
