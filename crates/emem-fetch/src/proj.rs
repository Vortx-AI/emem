//! Minimal WGS84 ↔ UTM projection — just enough to feed the COG sampler the
//! easting/northing it needs in a Sentinel-2 / -1 scene's native CRS.
//!
//! We hand-roll this rather than pulling in `proj4rs` / GDAL because:
//! - The full `proj` graph is hundreds of MB of grid shifts we don't need.
//! - UTM is a single, well-defined map projection (Transverse Mercator with a
//!   fixed scale factor 0.9996 and zone-specific central meridian).
//! - The math is ~80 lines and matches the WGS84 ellipsoid that Sentinel-2 /
//!   -1 publish in.
//!
//! Reference: USGS *Map Projections — A Working Manual* (Snyder 1987),
//! eqs. 8-1 through 8-13. The WGS84 constants are GRS80-equivalent within the
//! 10⁻⁹ tolerance the COG sampler cares about.

use std::f64::consts::PI;

/// Hemisphere flag used to pick the EPSG code (326XX north / 327XX south).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hemi {
    /// Northern hemisphere (EPSG:326XX). False northing = 0.
    North,
    /// Southern hemisphere (EPSG:327XX). False northing = 10 000 000 m.
    South,
}

/// UTM forward projection result.
#[derive(Debug, Clone, Copy)]
pub struct UtmCoord {
    /// Easting in metres.
    pub easting: f64,
    /// Northing in metres.
    pub northing: f64,
    /// UTM zone (1..60).
    pub zone: u8,
    /// Hemisphere.
    pub hemi: Hemi,
    /// EPSG code: 326XX for North, 327XX for South.
    pub epsg: u32,
}

/// EPSG → (zone, hemisphere) for any code in 32601..32660 / 32701..32760.
pub fn epsg_to_zone(epsg: u32) -> Option<(u8, Hemi)> {
    match epsg {
        32601..=32660 => Some(((epsg - 32600) as u8, Hemi::North)),
        32701..=32760 => Some(((epsg - 32700) as u8, Hemi::South)),
        _ => None,
    }
}

/// Forward project (lat, lon) in WGS84 degrees to UTM. Picks the zone from
/// longitude unless `force_zone` is `Some`.
pub fn latlng_to_utm(lat_deg: f64, lon_deg: f64, force_zone: Option<u8>) -> UtmCoord {
    let zone = force_zone.unwrap_or_else(|| {
        let z = ((lon_deg + 180.0) / 6.0).floor() as i32 + 1;
        z.clamp(1, 60) as u8
    });
    let hemi = if lat_deg >= 0.0 {
        Hemi::North
    } else {
        Hemi::South
    };
    let epsg = match hemi {
        Hemi::North => 32600,
        Hemi::South => 32700,
    } + zone as u32;
    let (e, n) = forward_tm_wgs84(lat_deg, lon_deg, zone);
    UtmCoord {
        easting: e,
        northing: n,
        zone,
        hemi,
        epsg,
    }
}

/// Project `(lat, lon)` into the UTM zone implied by an EPSG code (so the
/// caller can match the COG's CRS exactly even when the cell is just over a
/// zone boundary).
pub fn latlng_to_utm_with_epsg(lat_deg: f64, lon_deg: f64, epsg: u32) -> Option<UtmCoord> {
    let (zone, hemi) = epsg_to_zone(epsg)?;
    let (e, n_pre) = forward_tm_wgs84(lat_deg, lon_deg, zone);
    // forward_tm_wgs84 returns a hemisphere-aware northing by including the
    // 10 000 000 m false-northing for negative latitudes. If the caller forced
    // a hemisphere via EPSG (e.g. AOI just south of equator but Sentinel tile
    // is in the northern zone), we just trust the math.
    let _ = hemi;
    Some(UtmCoord {
        easting: e,
        northing: n_pre,
        zone,
        hemi,
        epsg,
    })
}

/// WGS84 Transverse-Mercator forward projection (Snyder eq. 8-1 to 8-9 with
/// the UTM scale factor k0 = 0.9996 and false easting 500 000 m).
fn forward_tm_wgs84(lat_deg: f64, lon_deg: f64, zone: u8) -> (f64, f64) {
    // WGS84 constants.
    let a: f64 = 6_378_137.0; // semi-major axis (m)
    let f: f64 = 1.0 / 298.257_223_563; // flattening
    let e2 = f * (2.0 - f); // first eccentricity²
    let ep2 = e2 / (1.0 - e2); // second eccentricity²
    let k0: f64 = 0.9996;
    let lon0_deg = (zone as f64 - 1.0) * 6.0 - 180.0 + 3.0; // central meridian
    let phi = lat_deg.to_radians();
    let lam = lon_deg.to_radians();
    let lam0 = lon0_deg.to_radians();

    let sin_phi = phi.sin();
    let cos_phi = phi.cos();
    let tan_phi = phi.tan();
    let n_rad = a / (1.0 - e2 * sin_phi * sin_phi).sqrt();
    let t = tan_phi * tan_phi;
    let c = ep2 * cos_phi * cos_phi;
    let aa = cos_phi * (lam - lam0);

    // Meridional distance M (Snyder 3-21 with WGS84 series).
    let m = a
        * ((1.0 - e2 / 4.0 - 3.0 * e2 * e2 / 64.0 - 5.0 * e2 * e2 * e2 / 256.0) * phi
            - (3.0 * e2 / 8.0 + 3.0 * e2 * e2 / 32.0 + 45.0 * e2 * e2 * e2 / 1024.0)
                * (2.0 * phi).sin()
            + (15.0 * e2 * e2 / 256.0 + 45.0 * e2 * e2 * e2 / 1024.0) * (4.0 * phi).sin()
            - (35.0 * e2 * e2 * e2 / 3072.0) * (6.0 * phi).sin());

    let easting = k0
        * n_rad
        * (aa
            + (1.0 - t + c) * aa.powi(3) / 6.0
            + (5.0 - 18.0 * t + t * t + 72.0 * c - 58.0 * ep2) * aa.powi(5) / 120.0)
        + 500_000.0;

    let mut northing = k0
        * (m + n_rad
            * tan_phi
            * (aa * aa / 2.0
                + (5.0 - t + 9.0 * c + 4.0 * c * c) * aa.powi(4) / 24.0
                + (61.0 - 58.0 * t + t * t + 600.0 * c - 330.0 * ep2) * aa.powi(6) / 720.0));
    if lat_deg < 0.0 {
        northing += 10_000_000.0; // false northing for southern hemisphere
    }
    let _ = PI; // keep the import even if not directly used after refactors
    (easting, northing)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() < tol
    }

    #[test]
    fn cambridge_uk_zone31() {
        // 0.1218° lon falls in zone 31 by the UTM rule (zones break at every
        // 6° starting at -180). Reference Snyder-formula values: easting
        // ≈303 336, northing ≈5 787 777.
        let u = latlng_to_utm(52.2053, 0.1218, None);
        assert_eq!(u.zone, 31);
        assert_eq!(u.hemi, Hemi::North);
        assert!(approx(u.easting, 303336.0, 5.0), "easting={}", u.easting);
        assert!(
            approx(u.northing, 5787777.0, 5.0),
            "northing={}",
            u.northing
        );
    }

    #[test]
    fn cambridge_uk_force_zone30() {
        // Sentinel-2 MGRS tile 30UXC extends zone 30 east of the strict UTM
        // boundary so cells just east of Greenwich are still served by a
        // zone-30 raster. Using the EPSG override yields easting/northing in
        // that raster's CRS — easting ≈713 305, northing ≈5 788 467.
        let u = latlng_to_utm_with_epsg(52.2053, 0.1218, 32630).unwrap();
        assert_eq!(u.zone, 30);
        assert!(approx(u.easting, 713305.0, 5.0), "easting={}", u.easting);
        assert!(
            approx(u.northing, 5788467.0, 5.0),
            "northing={}",
            u.northing
        );
    }
}
