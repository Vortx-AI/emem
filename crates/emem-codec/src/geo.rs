//! lat/lng ↔ cell64 codec.
//!
//! emem's cell64 layout encodes a 64-bit cell ID with the leading bits
//! reserved for `mode | resolution | base | path`. For agent-side
//! geospatial work, we expose a stable mapping between WGS-84 lat/lng
//! and the 32-bit Hilbert path so an agent that has only a coordinate
//! pair (or a place name) can land on the canonical cell64 string.
//!
//! This is **not** a substitute for a full H3-equivalent indexer (which
//! lives in `emem-cubes`) — it's the address that the L0/L1 surface
//! uses for geo queries when the operator hasn't registered a richer
//! tessellation. The mapping is locality-preserving (Hilbert order),
//! reversible, and stable across releases.
//!
//! Layout:
//! ```text
//!   bits 63..60   mode      = 0b0001 (geo)
//!   bits 59..52   resolution = 12 (default 16-bit lat/lng quantisation)
//!   bits 51..44   base       = 0xab (geo aperture marker)
//!   bits 43..32   reserved   = 0
//!   bits 31..00   hilbert_d(order=16, lat_q, lng_q)
//! ```

use crate::cell64::{from_cell64, to_cell64, CodecError};
use crate::hilbert::{d_to_xy, xy_to_d};
use emem_core::Cell;

const GEO_MODE: u64 = 1;
const GEO_RES: u64 = 12;
const GEO_BASE: u64 = 0xab;
const GEO_PREFIX_MASK: u64 = 0xFFFF_F000_0000_0000;
const GEO_PREFIX: u64 = (GEO_MODE << 60) | (GEO_RES << 52) | (GEO_BASE << 44);

/// Encode a WGS-84 (lat_deg, lng_deg) point to a cell64 string.
/// Lat clamped to [-90, 90]; lng wrapped to [-180, 180).
pub fn cell_from_latlng(lat_deg: f64, lng_deg: f64) -> Cell {
    let lat = lat_deg.clamp(-90.0, 90.0);
    let lng = ((lng_deg + 180.0).rem_euclid(360.0)) - 180.0;
    let lat_q = (((lat + 90.0) / 180.0) * 65535.0).round() as u32 & 0xFFFF;
    let lng_q = (((lng + 180.0) / 360.0) * 65535.0).round() as u32 & 0xFFFF;
    let d = xy_to_d(16, lat_q, lng_q);
    Cell::from_raw(GEO_PREFIX | (d as u64 & 0xFFFF_FFFF))
}

/// Convenience that emits the dot-bigram cell64 string.
pub fn cell64_from_latlng(lat_deg: f64, lng_deg: f64) -> String {
    to_cell64(cell_from_latlng(lat_deg, lng_deg))
}

/// Decode a cell64 string back to (lat_deg, lng_deg) — the **center**
/// of the lat/lng quantisation bucket. The bucket spans roughly
/// `180/2^16 ≈ 0.00275°` (~ 305 m at the equator) in each axis.
///
/// Returns `Err` if the cell64 is well-formed but not a `geo` cell.
pub fn latlng_from_cell64(s: &str) -> Result<LatLng, CodecError> {
    let cell = from_cell64(s)?;
    if (cell.0 & GEO_PREFIX_MASK) != GEO_PREFIX {
        return Err(CodecError::NotGeoCell(cell.0));
    }
    let d = (cell.0 & 0xFFFF_FFFF) as u32;
    let (lat_q, lng_q) = d_to_xy(16, d);
    let lat_deg = (lat_q as f64 / 65535.0) * 180.0 - 90.0;
    let lng_deg = (lng_q as f64 / 65535.0) * 360.0 - 180.0;
    let half_lat = 90.0 / 65535.0; // half-bucket edge in degrees
    let half_lng = 180.0 / 65535.0;
    Ok(LatLng {
        lat_deg,
        lng_deg,
        bbox_deg: BboxDeg {
            min_lat: (lat_deg - half_lat).max(-90.0),
            max_lat: (lat_deg + half_lat).min(90.0),
            min_lng: lng_deg - half_lng,
            max_lng: lng_deg + half_lng,
        },
    })
}

/// Output of `latlng_from_cell64`: the bucket centre + its bbox in degrees.
#[derive(Debug, Clone, Copy)]
pub struct LatLng {
    /// Centre latitude in degrees.
    pub lat_deg: f64,
    /// Centre longitude in degrees.
    pub lng_deg: f64,
    /// Bounding box of the cell's lat/lng bucket.
    pub bbox_deg: BboxDeg,
}

/// Lat/lng bounding box.
#[derive(Debug, Clone, Copy)]
pub struct BboxDeg {
    pub min_lat: f64,
    pub max_lat: f64,
    pub min_lng: f64,
    pub max_lng: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_landmarks_roundtrip() {
        // Mt. Fuji
        let s = cell64_from_latlng(35.3606, 138.7274);
        let back = latlng_from_cell64(&s).unwrap();
        assert!(
            (back.lat_deg - 35.3606).abs() < 0.005,
            "lat off: {}",
            back.lat_deg
        );
        assert!(
            (back.lng_deg - 138.7274).abs() < 0.005,
            "lng off: {}",
            back.lng_deg
        );

        // Mt. Everest
        let s = cell64_from_latlng(27.9881, 86.9250);
        let back = latlng_from_cell64(&s).unwrap();
        assert!((back.lat_deg - 27.9881).abs() < 0.005);
        assert!((back.lng_deg - 86.9250).abs() < 0.005);
    }

    #[test]
    fn antimeridian_roundtrip() {
        // Lng exactly at +180 wraps to -180 (closed-open).
        let s = cell64_from_latlng(0.0, 180.0);
        let back = latlng_from_cell64(&s).unwrap();
        assert!(back.lng_deg.abs() <= 180.0);
    }

    #[test]
    fn poles_clamp() {
        let s = cell64_from_latlng(91.0, 0.0);
        let back = latlng_from_cell64(&s).unwrap();
        assert!(back.lat_deg <= 90.0);
    }
}
