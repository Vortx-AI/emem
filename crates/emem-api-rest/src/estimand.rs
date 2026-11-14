//! What a signed number estimates, served beside it.
//!
//! A signature binds a fact's bytes and names its signer. It does not say
//! what the number is an estimate of: the footprint one value covers, what
//! was taken over that footprint, or the period the value describes. An agent
//! that has to guess differences two cells that sample one pixel, or reads a
//! 2020 forest map as a reading of the day its file was published, because a
//! release-dated layer's `captured_at` is the release.
//!
//! [`for_fact`] states those three things for the sources it can name. Each
//! pixel pitch is the one the source's own GeoTIFF header declares, read on
//! 14 November 2026 (docs/standards/memory-v1.md lists the tiles), or the
//! materialiser's own input table for Sentinel-2. Nothing here enters a fact,
//! a fact cid or a receipt preimage. A fact this table does not cover carries
//! no `estimand`, which means "not declared", never "a point value".

use serde_json::{json, Map, Value as JsonValue};

pub(crate) const SCHEMA: &str = "emem.estimand.v1";

/// Metres along a meridian per degree of latitude, to the nearest 10 m.
const M_PER_DEG_LAT: f64 = 111_320.0;

/// Sentinel-2 MSI sampling, metres. The materialiser reads each input at the
/// pixel of its own grid, so an index over a 10 m and a 20 m band is not a
/// 10 m value.
fn s2_band_m(b: &str) -> Option<u32> {
    match b {
        "B02" | "B03" | "B04" | "B08" => Some(10),
        "B05" | "B06" | "B07" | "B8A" | "B11" | "B12" | "SCL" => Some(20),
        "B01" | "B09" => Some(60),
        _ => None,
    }
}

/// The Sentinel-2 inputs of a band and the pitch of each, from the same plan
/// the materialiser reads.
fn s2_inputs(band: &str) -> Option<Vec<(&'static str, u32)>> {
    let (inputs, _, _) = crate::s2_band_plan(band)?;
    let mut out = Vec::new();
    for aliases in inputs {
        let name = *aliases.last()?;
        if !out.iter().any(|(n, _)| *n == name) {
            out.push((name, s2_band_m(name)?));
        }
    }
    (!out.is_empty()).then_some(out)
}

/// One source grid, regular in degrees. `basis` names the product and its
/// pixel, which is the same in every tile of the product; the file a fact
/// read comes from that fact's own sources ([`read_file`]).
struct Grid {
    deg: f64,
    basis: &'static str,
}

const GFC2020: Grid = Grid {
    deg: 1.0 / 12_000.0,
    basis: "JRC GFC2020 V4, 1/12000 degree pixels in every tile",
};
const HANSEN: Grid = Grid {
    deg: 0.000_25,
    basis: "Hansen Global Forest Change, 0.00025 degree pixels in every tile",
};
const CCI_BIOMASS: Grid = Grid {
    deg: 3.2 / 3600.0,
    basis: "ESA CCI Biomass L4 AGB, 3.2 arc-second pixels in every tile",
};
const COPDEM_LAT: Grid = Grid {
    deg: 1.0 / 3600.0,
    basis: "Copernicus DEM GLO-30, 1 arc-second along meridians in every tile",
};
const ERA5: Grid = Grid {
    deg: 0.25,
    basis: "Open-Meteo serves ERA5 on a 0.25 degree grid; the model runs at about 31 km",
};
const ERA5_LAND: Grid = Grid {
    deg: 0.1,
    basis: "Open-Meteo serves ERA5-Land on a 0.1 degree grid; the model runs at about 9 km",
};

fn grid_m(deg: f64) -> f64 {
    (deg * M_PER_DEG_LAT * 10.0).round() / 10.0
}

/// GLO-30 widens its longitude spacing with latitude (Copernicus DEM product
/// handbook); 1 arc-second at 12 N and 2 at 66 N were read from the tiles.
fn copdem_lng_arcsec(lat: f64) -> f64 {
    match lat.abs() {
        a if a < 50.0 => 1.0,
        a if a < 60.0 => 1.5,
        a if a < 70.0 => 2.0,
        a if a < 80.0 => 3.0,
        a if a < 85.0 => 5.0,
        _ => 10.0,
    }
}

/// Metres along a meridian of one value of `band`, for the bands this table
/// declares. `band_input_resolution_m` reads this first, so an aggregate's
/// `data_resolution_m` and a fact's `estimand.support` cannot disagree.
pub(crate) fn support_m(band: &str) -> Option<f64> {
    if band.starts_with("s2.") || band.starts_with("indices.") {
        return s2_inputs(band)
            .and_then(|v| v.iter().map(|(_, m)| *m).max())
            .map(f64::from);
    }
    let deg = match band {
        "era5.snow_depth" => ERA5_LAND.deg,
        b if b.starts_with("era5.") => ERA5.deg,
        "copdem30m.elevation_mean" => COPDEM_LAT.deg,
        b if b.starts_with("jrc_gfc2020.") => GFC2020.deg,
        b if b.starts_with("forest_change.") || b.starts_with("hansen.") => HANSEN.deg,
        b if b.starts_with("esa_cci_biomass.") => CCI_BIOMASS.deg,
        b if b.starts_with("soilgrids.") => return Some(250.0),
        _ => return None,
    };
    Some(grid_m(deg))
}

/// The GeoTIFF a fact read, from its own sources: the file name of the
/// first source that names one. Until 2026-11-14 `basis` named one sample
/// tile per product on every fact, whatever tile the fact read.
fn read_file(f: &Map<String, JsonValue>) -> Option<String> {
    f.get("sources")?.as_array()?.iter().find_map(|s| {
        ["url", "id"].iter().find_map(|k| {
            let u = s.get(*k)?.as_str()?;
            let name = u.split(['?', '#']).next()?.rsplit('/').next()?;
            let l = name.to_ascii_lowercase();
            (l.ends_with(".tif") || l.ends_with(".tiff")).then(|| name.to_string())
        })
    })
}

fn grid_support(kind: &str, g: &Grid, f: &Map<String, JsonValue>) -> JsonValue {
    let basis = match read_file(f) {
        Some(file) => format!("{}; this fact read {file}", g.basis),
        None => g.basis.to_string(),
    };
    json!({"kind": kind, "deg": g.deg, "m": grid_m(g.deg), "basis": basis})
}

fn first_captured_at(f: &Map<String, JsonValue>) -> Option<String> {
    f.get("sources")?
        .as_array()?
        .iter()
        .find_map(|s| s.get("captured_at").and_then(|v| v.as_str()))
        .map(str::to_string)
}

fn source_ids(f: &Map<String, JsonValue>) -> String {
    f.get("sources")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|s| s.get("id").and_then(|v| v.as_str()))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}

/// The release year in a Hansen file name, `GFC-2025-v1.13`: the last year
/// of loss the release maps.
fn hansen_last_year(ids: &str) -> Option<i32> {
    let i = ids.find("GFC-")?;
    let y = ids.get(i + 4..i + 8)?;
    y.bytes()
        .all(|b| b.is_ascii_digit())
        .then(|| y.parse().ok())
        .flatten()
}

/// The year a band key ends in (`esa_cci_biomass.agb_t_per_ha_2020`).
fn trailing_year(band: &str) -> Option<i32> {
    let y = band.get(band.len().checked_sub(4)?..)?;
    let before = band.get(..band.len() - 4)?;
    (before.ends_with('_') && y.bytes().all(|b| b.is_ascii_digit()))
        .then(|| y.parse().ok())
        .flatten()
}

fn year(y: i32, basis: &str) -> JsonValue {
    json!({"kind": "year", "from": format!("{y}-01-01"), "to": format!("{y}-12-31"), "basis": basis})
}

fn shift_hour(iso: &str, hours: i64) -> Option<String> {
    let u = crate::parse_iso8601_unix(iso)? + hours * 3600;
    (u >= 0).then(|| crate::iso8601_utc(u as u64))
}

/// The `estimand` block for one serialized fact, or `None` when this table
/// does not declare its source. Takes the flat form or `{"Primary": {...}}`.
pub(crate) fn for_fact(obj: &Map<String, JsonValue>) -> Option<JsonValue> {
    let f = obj
        .get("Primary")
        .and_then(|v| v.as_object())
        .unwrap_or(obj);
    if f.get("kind").and_then(|v| v.as_str()).unwrap_or("primary") != "primary" {
        return None;
    }
    let band = f.get("band")?.as_str()?;
    let fn_key = f.get("derivation")?.get("fn_key")?.as_str()?;
    let captured = first_captured_at(f);
    let lat = f
        .get("derivation")
        .and_then(|d| d.get("args"))
        .and_then(|a| a.get(0))
        .and_then(|v| v.as_f64());

    let (support, statistic, text, valid, captured_is) = if fn_key.starts_with("sentinel2_l2a_")
        && (band.starts_with("s2.") || band.starts_with("indices."))
    {
        let inputs = s2_inputs(band)?;
        let coarsest = inputs.iter().map(|(_, m)| *m).max()?;
        let raw = band.starts_with("s2.");
        (
            json!({
                "kind": "source_pixel", "m": coarsest,
                "inputs": inputs.iter().map(|(b, m)| json!({"band": b, "m": m})).collect::<Vec<_>>(),
                "basis": "Sentinel-2 MSI band sampling (10, 20 or 60 m); inputs from the materialiser's band plan",
            }),
            match (raw, band) {
                (_, "s2.scl") => "pixel_class",
                (true, _) => "pixel_value",
                _ => "index_of_pixel_values",
            },
            if band == "s2.scl" {
                "the scene class of the one pixel that contains the cell's point"
            } else if raw {
                "the surface reflectance of the one pixel that contains the cell's point"
            } else {
                "the index computed from the one pixel of each input band that contains the cell's point, each on its own grid; not a mean over the cell"
            },
            captured.as_ref().map(|c| {
                json!({"kind": "instant", "from": c, "to": c, "basis": "the scene's sensing time"})
            }),
            "sensing_time",
        )
    } else if fn_key.starts_with("open_meteo_era5_land_archive@")
        || fn_key.starts_with("open_meteo_era5_archive@")
    {
        let g = if fn_key.starts_with("open_meteo_era5_land_") {
            &ERA5_LAND
        } else {
            &ERA5
        };
        let sum = matches!(band, "era5.precip" | "era5.snowfall");
        let valid = captured.as_ref().map(|c| {
            if sum {
                json!({"kind": "period", "from": shift_hour(c, -1), "to": c,
                       "basis": "Open-Meteo sums precipitation and snowfall over the hour before the time stamp"})
            } else {
                json!({"kind": "instant", "from": c, "to": c, "basis": "the model hour"})
            }
        });
        (
            grid_support("model_grid_cell", g, f),
            if sum { "model_hour_sum" } else { "model_value" },
            "the reanalysis value Open-Meteo serves for the point from the model grid; a model value, not an observation",
            valid,
            "model_hour",
        )
    } else if fn_key.starts_with("copernicus_dem_30m_aws_pixel@")
        && band == "copdem30m.elevation_mean"
    {
        let mut support = grid_support("source_pixel", &COPDEM_LAT, f);
        if let (Some(lat), Some(m)) = (lat, support.as_object_mut()) {
            m.insert("lng_deg".into(), json!(copdem_lng_arcsec(lat) / 3600.0));
        }
        (
            support,
            "pixel_value",
            "the height of the one surface-model pixel that contains the cell's point; a DSM, so it includes buildings and canopy, and `mean` in the band name is the product's, not a mean emem took",
            Some(json!({"kind": "edition", "basis": "a surface model built from radar acquisitions made before the edition date"})),
            "edition_date",
        )
    } else if fn_key.starts_with("jrc_gfc2020_v") && band == "jrc_gfc2020.forest_2020" {
        (
            grid_support("source_pixel", &GFC2020, f),
            "pixel_class",
            "the class of the one pixel that contains the cell's point: 1 forest, 0 not forest, under the EUDR definition",
            Some(year(2020, "the map states forest cover in 2020, the year of the EUDR cut-off of 2020-12-31")),
            "release_date",
        )
    } else if fn_key.starts_with("hansen_gfc_")
        && (band.starts_with("forest_change.") || band.starts_with("hansen."))
    {
        let valid = if band.ends_with(".lossyear") {
            hansen_last_year(&source_ids(f)).map(|y| {
                json!({"kind": "period", "from": "2001-01-01", "to": format!("{y}-12-31"),
                       "basis": "loss years 2001 to the year in the release name"})
            })
        } else if band.ends_with(".treecover2000") {
            Some(year(2000, "canopy cover in the year 2000"))
        } else {
            None
        };
        (
            grid_support("source_pixel", &HANSEN, f),
            "pixel_value",
            "the value of the one pixel that contains the cell's point",
            valid,
            "release_date",
        )
    } else if fn_key.starts_with("esa_cci_biomass_v") && band.starts_with("esa_cci_biomass.") {
        (
            grid_support("source_pixel", &CCI_BIOMASS, f),
            "pixel_value",
            if band.contains("_se_") {
                "the standard error ESA states for the biomass density of the one pixel that contains the cell's point, in t/ha"
            } else {
                "the above-ground biomass density of the one pixel that contains the cell's point, in t/ha: a mean over that pixel, so sparse trees are diluted into it"
            },
            trailing_year(band).map(|y| year(y, "the map year in the band key")),
            "release_date",
        )
    } else if fn_key.starts_with("soilgrids_v2_rest_0_30cm@") {
        (
            json!({"kind": "source_pixel", "m": 250, "basis": "SoilGrids 2.0 predicts on a 250 m grid", "depth_cm": [0, 30]}),
            "thickness_weighted_mean",
            "the thickness-weighted mean of ISRIC's predicted means for 0 to 5, 5 to 15 and 15 to 30 cm, at the pixel that contains the cell's point",
            Some(json!({"kind": "edition", "basis": "a prediction from soil profiles collected over decades"})),
            "release_date",
        )
    } else {
        return None;
    };

    let mut out = json!({
        "schema": SCHEMA,
        "support": support,
        "statistic": statistic,
        "statistic_text": text,
        "captured_at_is": captured_is,
        "declared_by": "this responder's estimand table, outside the signed bytes",
    });
    if let (Some(v), Some(m)) = (valid, out.as_object_mut()) {
        m.insert("valid_time".into(), v);
    }
    Some(out)
}

/// What kind of process made a band that the bands manifest gives no family.
/// Such a band's class is `unclassified`, and stays so: the class is what the
/// manifest declares and the recall filter reads. This only says, beside it,
/// what the upstream is.
pub(crate) fn unregistered_upstream(b: &str) -> Option<&'static str> {
    if b.starts_with("era5.") {
        Some("ECMWF ERA5 or ERA5-Land reanalysis: model output, not an observation")
    } else if b.starts_with("power.") {
        Some("NASA POWER: values from the MERRA-2 reanalysis and satellite-derived radiation, not a station reading")
    } else if b.starts_with("marine.") {
        Some("Open-Meteo marine: wave and ocean model output, not a buoy reading")
    } else if b.starts_with("modis.") {
        Some("a MODIS science product: a retrieval or model over MODIS radiances, not a direct reading")
    } else if b.starts_with("firms.") {
        Some("NASA FIRMS: a fire detection algorithm over thermal bands; a missed fire reads as none")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(band: &str, fn_key: &str, captured: &str, ids: &str, args: JsonValue) -> JsonValue {
        json!({
            "kind": "primary", "band": band, "cell": "defi.zb6f5.fUvE.xAtA", "tslot": 0,
            "value": 0.5, "derivation": {"fn_key": fn_key, "args": args},
            "sources": [{"scheme": "x", "id": ids, "captured_at": captured}],
        })
    }

    fn est(v: &JsonValue) -> JsonValue {
        for_fact(v.as_object().unwrap()).unwrap_or(JsonValue::Null)
    }

    /// The NDSI fact served at Santa Claus Village on 14 November 2026 reads
    /// B03 at 10 m and B11 at 20 m, so one value covers a 20 m pixel.
    #[test]
    fn an_index_over_a_20_m_band_is_not_a_10_m_value() {
        let f = fact(
            "indices.ndsi",
            "sentinel2_l2a_indices_ndsi@1",
            "2024-10-31T10:01:41.024000Z",
            "R10m/T35WMP_B03_10m.tif ; R20m/T35WMP_B11_20m.tif",
            json!([66.544, 25.848]),
        );
        let e = est(&f);
        assert_eq!(e["schema"], SCHEMA);
        assert_eq!(e["support"]["m"], 20, "{e}");
        assert_eq!(e["support"]["inputs"][0], json!({"band": "B03", "m": 10}));
        assert_eq!(e["support"]["inputs"][1], json!({"band": "B11", "m": 20}));
        assert_eq!(e["statistic"], "index_of_pixel_values");
        assert_eq!(e["valid_time"]["kind"], "instant");
        assert_eq!(e["captured_at_is"], "sensing_time");
        let ndvi = fact(
            "indices.ndvi",
            "sentinel2_l2a_indices_ndvi@1",
            "2026-11-11T10:21:49Z",
            "",
            json!([]),
        );
        assert_eq!(est(&ndvi)["support"]["m"], 10);
    }

    /// The pitch table must agree with the materialiser's own note for every
    /// raw band it reads (`"B11 20m SWIR-1"`): two statements, one value.
    #[test]
    fn s2_pitches_agree_with_the_band_plan_notes() {
        for b in [
            "B01", "B02", "B03", "B04", "B05", "B06", "B07", "B08", "B8A", "B09", "B11", "B12",
        ] {
            let key = format!("s2.{b}");
            let (_, _, note) = crate::s2_band_plan(&key).unwrap();
            let m = s2_band_m(b).unwrap();
            assert!(
                note.contains(&format!("{b} {m}m")),
                "{key}: table says {m} m, plan says {note:?}"
            );
            assert_eq!(support_m(&key), Some(f64::from(m)));
        }
        assert_eq!(support_m("indices.ndmi"), Some(20.0));
        assert_eq!(support_m("indices.evi"), Some(10.0));
        assert_eq!(support_m("indices.ndre"), Some(20.0));
    }

    /// The JRC map describes 2020; its `captured_at` is the file's release.
    #[test]
    fn a_release_dated_layer_says_what_period_it_describes() {
        let f = fact(
            "jrc_gfc2020.forest_2020",
            "jrc_gfc2020_v4_pixel@1",
            "2026-09-14T09:18:00Z",
            "https://jeodpp.jrc.ec.europa.eu/ftp/jrc-opendata/FOREST/GFC2020/LATEST/tiles/JRC_GFC2020_V4_N70_E20.tif",
            json!([66.544, 25.848, "v4"]),
        );
        let e = est(&f);
        assert_eq!(e["valid_time"]["from"], "2020-01-01");
        assert_eq!(e["valid_time"]["to"], "2020-12-31");
        assert_eq!(e["captured_at_is"], "release_date");
        assert_eq!(e["statistic"], "pixel_class");
        assert_eq!(e["support"]["m"], 9.3);

        let h = fact(
            "forest_change.lossyear",
            "hansen_gfc_v1_13_pixel@1",
            "2025-04-01T00:00:00Z",
            "https://storage.googleapis.com/earthenginepartners-hansen/GFC-2025-v1.13/Hansen_GFC-2025-v1.13_lossyear_70N_020E.tif",
            json!([66.544, 25.848, "70N_020E", "lossyear"]),
        );
        let e = est(&h);
        assert_eq!(e["valid_time"]["to"], "2025-12-31", "{e}");
        assert_eq!(e["support"]["m"], 27.8);

        let b = fact(
            "esa_cci_biomass.agb_t_per_ha_2020",
            "esa_cci_biomass_v7_pixel@1",
            "2025-03-01T00:00:00Z",
            "",
            json!([66.544, 25.848, 2020, "v7.0"]),
        );
        let e = est(&b);
        assert_eq!(e["valid_time"]["from"], "2020-01-01");
        assert_eq!(e["support"]["m"], 99.0);
    }

    /// Two Hansen facts from two tiles each name the tile they read, and
    /// neither names the other's. A fact with no file names the product only.
    #[test]
    fn the_basis_names_the_tile_the_fact_read() {
        let at = |tile: &str| {
            fact(
                "forest_change.lossyear",
                "hansen_gfc_v1_13_pixel@1",
                "2025-04-01T00:00:00Z",
                &format!("https://storage.googleapis.com/earthenginepartners-hansen/GFC-2025-v1.13/Hansen_GFC-2025-v1.13_lossyear_{tile}.tif"),
                json!([]),
            )
        };
        let north = est(&at("70N_020E"))["support"]["basis"].clone();
        let south = est(&at("00N_060W"))["support"]["basis"].clone();
        let (north, south) = (north.as_str().unwrap(), south.as_str().unwrap());
        assert!(
            north.contains("Hansen_GFC-2025-v1.13_lossyear_70N_020E.tif"),
            "{north}"
        );
        assert!(
            south.contains("Hansen_GFC-2025-v1.13_lossyear_00N_060W.tif"),
            "{south}"
        );
        assert!(!south.contains("70N_020E"), "{south}");
        assert!(south.starts_with(HANSEN.basis));

        let dem = fact(
            "copdem30m.elevation_mean",
            "copernicus_dem_30m_aws_pixel@1",
            "2021-04-30T00:00:00Z",
            "https://copernicus-dem-30m.s3.amazonaws.com/Copernicus_DSM_COG_10_S04_00_W061_00_DEM/Copernicus_DSM_COG_10_S04_00_W061_00_DEM.tif",
            json!([-3.5, -60.5]),
        );
        let b = est(&dem)["support"]["basis"].clone();
        assert!(
            b.as_str()
                .unwrap()
                .ends_with("Copernicus_DSM_COG_10_S04_00_W061_00_DEM.tif"),
            "{b}"
        );
        assert!(!b.as_str().unwrap().contains("N12"), "{b}");

        let bare = fact(
            "esa_cci_biomass.agb_t_per_ha_2020",
            "esa_cci_biomass_v7_pixel@1",
            "2025-03-01T00:00:00Z",
            "",
            json!([]),
        );
        assert_eq!(est(&bare)["support"]["basis"], CCI_BIOMASS.basis);
    }

    #[test]
    fn reanalysis_is_a_model_grid_cell_and_a_sum_covers_the_hour_before() {
        let sd = fact(
            "era5.snow_depth",
            "open_meteo_era5_land_archive@1",
            "2024-12-03T12:00:00Z",
            "",
            json!([]),
        );
        let e = est(&sd);
        assert_eq!(e["support"]["kind"], "model_grid_cell");
        assert_eq!(e["support"]["deg"], 0.1);
        assert_eq!(e["statistic"], "model_value");
        let p = fact(
            "era5.precip",
            "open_meteo_era5_archive@1",
            "2024-12-03T12:00:00Z",
            "",
            json!([]),
        );
        let e = est(&p);
        assert_eq!(e["support"]["deg"], 0.25);
        assert_eq!(e["statistic"], "model_hour_sum");
        assert_eq!(e["valid_time"]["from"], "2024-12-03T11:00:00Z", "{e}");
        assert_eq!(e["valid_time"]["to"], "2024-12-03T12:00:00Z");
    }

    #[test]
    fn the_dem_states_its_wider_longitude_spacing_in_the_north() {
        let at = |lat: f64| {
            let f = fact(
                "copdem30m.elevation_mean",
                "copernicus_dem_30m_aws_pixel@1",
                "2021-04-30T00:00:00Z",
                "",
                json!([lat, 25.0]),
            );
            est(&f)
        };
        assert_eq!(at(12.97)["support"]["lng_deg"], json!(1.0 / 3600.0));
        assert_eq!(at(66.54)["support"]["lng_deg"], json!(2.0 / 3600.0));
        assert_eq!(at(66.54)["support"]["m"], 30.9);
        assert_eq!(at(66.54)["captured_at_is"], "edition_date");
    }

    /// No row, no block: the absence of `estimand` must mean "not declared".
    #[test]
    fn an_undeclared_source_gets_no_block() {
        let f = fact(
            "weather.temperature_2m",
            "met_no_locationforecast_compact@1",
            "2026-11-14T03:00:00Z",
            "",
            json!([]),
        );
        assert!(for_fact(f.as_object().unwrap()).is_none());
        // An old DEM fact from the retired 90 m path is not the 30 m product.
        let old = fact(
            "copdem30m.elevation_mean",
            "open_meteo_copdem90m@1",
            "",
            "",
            json!([]),
        );
        assert!(for_fact(old.as_object().unwrap()).is_none());
        let absence = json!({"kind": "absence", "band": "jrc_tmf.annual_change"});
        assert!(for_fact(absence.as_object().unwrap()).is_none());
        assert_eq!(support_m("weather.temperature_2m"), None);
    }

    #[test]
    fn the_unregistered_families_say_what_made_them() {
        for b in [
            "era5.snow_depth",
            "power.t2m",
            "marine.sst",
            "modis.lst_day_8day",
            "firms.active_fires",
        ] {
            assert!(unregistered_upstream(b).is_some(), "{b}");
        }
        assert!(unregistered_upstream("indices.ndvi").is_none());
    }
}
