//! Bounding box in WGS84 — the geographic primitive used by emem-fetch
//! to resolve template variables for COG tile naming and STAC search.
//!
//! `Cell` is the cache key (content-addressed, hex-tessellated); `Bbox`
//! is the *I/O* shape that connectors actually use to compute upstream
//! tile URLs and Range windows.  A request typically carries both: the
//! cell decides hit/miss, the bbox decides what to fetch on miss.

use serde::{Deserialize, Serialize};

/// WGS84 axis-aligned bounding box in degrees.
///
/// Invariants: `lat_min <= lat_max`, `-90 <= lat_*  <= 90`,
/// `-180 <= lon_* <= 180`.  Antimeridian-crossing boxes are rejected at
/// construction; callers should split them into two boxes themselves.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bbox {
    /// South edge (degrees latitude).
    pub lat_min: f64,
    /// North edge (degrees latitude).
    pub lat_max: f64,
    /// West edge (degrees longitude).
    pub lon_min: f64,
    /// East edge (degrees longitude).
    pub lon_max: f64,
}

/// Errors constructing a [`Bbox`].
#[derive(Debug, thiserror::Error)]
pub enum BboxError {
    /// One of the coordinates is outside its valid range.
    #[error("coordinate out of range: {0}")]
    OutOfRange(&'static str),
    /// `lat_min > lat_max` or `lon_min > lon_max` (antimeridian crossing).
    #[error("inverted bounds (split antimeridian-crossing boxes)")]
    Inverted,
}

impl Bbox {
    /// Construct with validation.
    pub fn new(lat_min: f64, lat_max: f64, lon_min: f64, lon_max: f64) -> Result<Self, BboxError> {
        if !(-90.0..=90.0).contains(&lat_min) || !(-90.0..=90.0).contains(&lat_max) {
            return Err(BboxError::OutOfRange("lat"));
        }
        if !(-180.0..=180.0).contains(&lon_min) || !(-180.0..=180.0).contains(&lon_max) {
            return Err(BboxError::OutOfRange("lon"));
        }
        if lat_min > lat_max || lon_min > lon_max {
            return Err(BboxError::Inverted);
        }
        Ok(Bbox {
            lat_min,
            lat_max,
            lon_min,
            lon_max,
        })
    }

    /// Centroid (lat, lon).
    pub fn center(&self) -> (f64, f64) {
        (
            (self.lat_min + self.lat_max) / 2.0,
            (self.lon_min + self.lon_max) / 2.0,
        )
    }

    /// CSV form `"lon_min,lat_min,lon_max,lat_max"` matching STAC search
    /// API conventions.
    pub fn to_csv(&self) -> String {
        format!(
            "{},{},{},{}",
            self.lon_min, self.lat_min, self.lon_max, self.lat_max
        )
    }

    // ──────────────────────────────────────────────────────────────────
    // Tile naming helpers (per-source convention)
    // ──────────────────────────────────────────────────────────────────

    /// Copernicus DSM 30m latitude band: signed integer floor of centroid
    /// latitude rendered as `Nxx` or `Sxx` (zero-padded to 2 digits).
    /// Example: lat=42.3 → `"N42"`, lat=-15.7 → `"S16"`.
    pub fn lat_band_1deg(&self) -> String {
        let (lat, _) = self.center();
        let i = lat.floor() as i32;
        if i >= 0 {
            format!("N{:02}", i)
        } else {
            format!("S{:02}", i.abs())
        }
    }

    /// Copernicus DSM 30m longitude band: `Exxx` or `Wxxx` (zero-padded
    /// to 3 digits) of floor(centroid lon).
    pub fn lon_band_1deg(&self) -> String {
        let (_, lon) = self.center();
        let i = lon.floor() as i32;
        if i >= 0 {
            format!("E{:03}", i)
        } else {
            format!("W{:03}", i.abs())
        }
    }

    /// JRC Global Surface Water tile longitude (10° grid, LEFT edge).
    /// Tile naming uses `{west_edge_abs}{E|W}` — e.g. lon=-93.4 → `"100W"`.
    pub fn lon_left_10deg(&self) -> String {
        let (_, lon) = self.center();
        let edge = (lon / 10.0).floor() as i32 * 10;
        if edge >= 0 {
            format!("{}E", edge)
        } else {
            format!("{}W", edge.abs())
        }
    }

    /// JRC Global Surface Water tile latitude (10° grid, TOP edge).
    /// Tile naming uses `{north_edge_abs}{N|S}` — e.g. lat=42.0 → `"50N"`.
    pub fn lat_top_10deg(&self) -> String {
        let (lat, _) = self.center();
        let edge = (lat / 10.0).ceil() as i32 * 10;
        if edge >= 0 {
            format!("{}N", edge)
        } else {
            format!("{}S", edge.abs())
        }
    }

    /// Hansen GFC tile latitude band: TOP-of-tile `{NN}{N|S}` with the
    /// tile spanning lat-10 .. lat.  e.g. lat=42 → `"50N"`.
    pub fn hansen_lat_band(&self) -> String {
        let (lat, _) = self.center();
        let edge = ((lat / 10.0).ceil() as i32) * 10;
        if edge >= 0 {
            format!("{:02}N", edge)
        } else {
            format!("{:02}S", edge.abs())
        }
    }

    /// Hansen GFC tile longitude band: LEFT-of-tile `{NNN}{E|W}` with the
    /// tile spanning lon .. lon+10.  e.g. lon=-93.4 → `"100W"`.
    pub fn hansen_lon_band(&self) -> String {
        let (_, lon) = self.center();
        let edge = (lon / 10.0).floor() as i32 * 10;
        if edge >= 0 {
            format!("{:03}E", edge)
        } else {
            format!("{:03}W", edge.abs())
        }
    }

    /// ESA WorldCover tile id `Nxx{E|W}xxx` snapped to 3° grid.
    pub fn worldcover_tile_id(&self) -> String {
        let (lat, lon) = self.center();
        let lat_floor = (lat / 3.0).floor() as i32 * 3;
        let lon_floor = (lon / 3.0).floor() as i32 * 3;
        let ns = if lat_floor >= 0 { 'N' } else { 'S' };
        let ew = if lon_floor >= 0 { 'E' } else { 'W' };
        format!("{}{:02}{}{:03}", ns, lat_floor.abs(), ew, lon_floor.abs())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iowa_us_tile_names() {
        // IOWA_US bbox from agri/farm_config.py
        let b = Bbox::new(42.01, 42.05, -93.49, -93.44).unwrap();
        assert_eq!(b.lat_band_1deg(), "N42");
        assert_eq!(b.lon_band_1deg(), "W094");
        // JRC GSW tile is 100W_50N (covers 100W..90W, 40N..50N)
        assert_eq!(b.lon_left_10deg(), "100W");
        assert_eq!(b.lat_top_10deg(), "50N");
        // Hansen GFC tile is 50N_100W
        assert_eq!(b.hansen_lat_band(), "50N");
        assert_eq!(b.hansen_lon_band(), "100W");
        // WorldCover 3° tile floor for IOWA: lat 42 → 42 (mult of 3),
        // lon -93.46 → -96 (next 3° below)
        assert_eq!(b.worldcover_tile_id(), "N42W096");
    }

    #[test]
    fn southern_hemisphere() {
        let b = Bbox::new(-15.7, -15.3, -47.95, -47.85).unwrap(); // Brazil
        assert_eq!(b.lat_band_1deg(), "S16");
        assert_eq!(b.lon_band_1deg(), "W048");
        assert_eq!(b.hansen_lat_band(), "10S");
        assert_eq!(b.hansen_lon_band(), "050W");
    }

    #[test]
    fn rejects_inverted() {
        assert!(Bbox::new(10.0, 5.0, 0.0, 1.0).is_err());
    }

    #[test]
    fn csv_format_matches_stac() {
        let b = Bbox::new(1.0, 2.0, 3.0, 4.0).unwrap();
        assert_eq!(b.to_csv(), "3,1,4,2"); // lon_min,lat_min,lon_max,lat_max
    }
}
