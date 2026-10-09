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
        northing: n + false_northing(hemi),
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
    let (e, n) = forward_tm_wgs84(lat_deg, lon_deg, zone);
    // The false northing belongs to the CRS, not the point: a Sentinel-2
    // tile in a northern zone (326xx) reaches ~10 km south of the equator,
    // and a point there has a small negative northing in it. Adding it by
    // the point's latitude put those reads 10 000 km off the image.
    Some(UtmCoord {
        easting: e,
        northing: n + false_northing(hemi),
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

    let northing = k0
        * (m + n_rad
            * tan_phi
            * (aa * aa / 2.0
                + (5.0 - t + 9.0 * c + 4.0 * c * c) * aa.powi(4) / 24.0
                + (61.0 - 58.0 * t + t * t + 600.0 * c - 330.0 * ep2) * aa.powi(6) / 720.0));
    let _ = PI; // keep the import even if not directly used after refactors
    (easting, northing)
}

/// Inverse of [`latlng_to_utm_with_epsg`]: a point in the UTM CRS named by
/// `epsg` back to WGS84 `(lat, lon)` degrees, by the footpoint-latitude series
/// (Snyder eqs. 8-12 to 8-25). Inside a zone it round-trips the forward
/// projection to well under a millimetre, which is what a map of pixel blocks
/// needs: bounds a reader can draw, in the CRS the request was made in.
pub fn utm_to_latlng_with_epsg(easting: f64, northing: f64, epsg: u32) -> Option<(f64, f64)> {
    let (zone, hemi) = epsg_to_zone(epsg)?;
    let a: f64 = 6_378_137.0;
    let f: f64 = 1.0 / 298.257_223_563;
    let e2 = f * (2.0 - f);
    let ep2 = e2 / (1.0 - e2);
    let k0: f64 = 0.9996;
    let lon0 = ((zone as f64 - 1.0) * 6.0 - 180.0 + 3.0).to_radians();
    let x = easting - 500_000.0;
    let y = northing - false_northing(hemi);

    let m = y / k0;
    let mu = m / (a * (1.0 - e2 / 4.0 - 3.0 * e2 * e2 / 64.0 - 5.0 * e2 * e2 * e2 / 256.0));
    let e1 = (1.0 - (1.0 - e2).sqrt()) / (1.0 + (1.0 - e2).sqrt());
    let phi1 = mu
        + (3.0 * e1 / 2.0 - 27.0 * e1.powi(3) / 32.0) * (2.0 * mu).sin()
        + (21.0 * e1 * e1 / 16.0 - 55.0 * e1.powi(4) / 32.0) * (4.0 * mu).sin()
        + (151.0 * e1.powi(3) / 96.0) * (6.0 * mu).sin()
        + (1097.0 * e1.powi(4) / 512.0) * (8.0 * mu).sin();

    let (sin1, cos1, tan1) = (phi1.sin(), phi1.cos(), phi1.tan());
    let c1 = ep2 * cos1 * cos1;
    let t1 = tan1 * tan1;
    let n1 = a / (1.0 - e2 * sin1 * sin1).sqrt();
    let r1 = a * (1.0 - e2) / (1.0 - e2 * sin1 * sin1).powf(1.5);
    let d = x / (n1 * k0);

    let lat = phi1
        - (n1 * tan1 / r1)
            * (d * d / 2.0
                - (5.0 + 3.0 * t1 + 10.0 * c1 - 4.0 * c1 * c1 - 9.0 * ep2) * d.powi(4) / 24.0
                + (61.0 + 90.0 * t1 + 298.0 * c1 + 45.0 * t1 * t1 - 252.0 * ep2 - 3.0 * c1 * c1)
                    * d.powi(6)
                    / 720.0);
    let lon = lon0
        + (d - (1.0 + 2.0 * t1 + c1) * d.powi(3) / 6.0
            + (5.0 - 2.0 * c1 + 28.0 * t1 - 3.0 * c1 * c1 + 8.0 * ep2 + 24.0 * t1 * t1)
                * d.powi(5)
                / 120.0)
            / cos1;
    Some((lat.to_degrees(), lon.to_degrees()))
}

fn false_northing(hemi: Hemi) -> f64 {
    match hemi {
        Hemi::North => 0.0,
        Hemi::South => 10_000_000.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sentinel-2 northern tiles (e.g. 36NVF, EPSG:32636) reach ~0.09°S.
    #[test]
    fn the_false_northing_follows_the_crs_not_the_point() {
        let n = latlng_to_utm_with_epsg(-0.05, 33.0, 32636)
            .unwrap()
            .northing;
        assert!((-5_600.0..-5_400.0).contains(&n), "{n}");
        let sth = latlng_to_utm_with_epsg(-0.05, 33.0, 32736)
            .unwrap()
            .northing;
        assert!((sth - (10_000_000.0 + n)).abs() < 1e-6);
        assert!(latlng_to_utm(-0.05, 33.0, None).northing > 9_990_000.0);
    }

    /// The inverse undoes the forward projection across a zone and both
    /// hemispheres, including a northern-zone point just south of the equator.
    #[test]
    fn the_inverse_round_trips_the_forward_projection() {
        for (lat, lng, epsg) in [
            (35.3628, 138.7307, 32654),
            (6.8512, -5.3001, 32630),
            (-33.9249, 18.4241, 32734),
            (-0.05, 33.0, 32636),
            (60.1699, 24.9384, 32635),
        ] {
            let u = latlng_to_utm_with_epsg(lat, lng, epsg).unwrap();
            let (la, lo) = utm_to_latlng_with_epsg(u.easting, u.northing, epsg).unwrap();
            assert!(
                (la - lat).abs() < 1e-8 && (lo - lng).abs() < 1e-8,
                "{lat},{lng} -> {la},{lo}"
            );
        }
        assert!(utm_to_latlng_with_epsg(500_000.0, 0.0, 4326).is_none());
    }

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
