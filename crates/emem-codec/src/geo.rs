//! lat/lng ↔ cell64 codec (10 m square-at-equator grid).
//!
//! emem's cell64 layout encodes a 64-bit cell ID with the leading bits
//! reserved for `mode | resolution | base | path`. This codec maps a
//! WGS-84 (lat, lng) pair onto a packed (lat_q, lng_q) Hilbert-ordered
//! cell key.
//!
//! ## Resolution
//!
//! The grid uses **21 bits on lat × 22 bits on lng** so each cell is
//! ~10 m × ~10 m **square at the equator** (matching Sentinel-1/Sentinel-2
//! native pixel pitch). Asymmetric bit count is necessary because the
//! lng axis spans 360° while lat spans 180° — equal bit counts produce
//! 1:2-rectangular cells. lat at 21 bits gives ~9.54 m; lng at 22 bits
//! gives ~9.55 m. Above the equator, lng pixels narrow with cos(lat) so
//! cells become taller than wide; this is the same effect every
//! lat/lng grid has and the spec target H3 hex grid is the eventual
//! migration to per-cell equal-area pixels.
//!
//! Earlier emem responders served a ~305 m grid (16-bit-per-axis
//! quantisation, Hilbert-ordered). That older encoding is **not**
//! decodable under this codec: the resolution tag in the prefix
//! changes from 12 → 21, so legacy strings fail `NotGeoCell` rather
//! than silently misplacing facts by hundreds of metres.
//!
//! ## Locality
//!
//! Hilbert locality at the cell-key level is dropped here (the curve
//! requires equal-bit axes). String-prefix locality at the bigram
//! level is unchanged: the cell64 alphabet itself is Hilbert-ordered
//! by `tools/measure_alphabet.py`, so adjacent codepoints still tend
//! to map to nearby cells in the visual ordering. For exact spatial
//! neighbourhoods, agents should use `/v1/locate`'s `neighborhood_cells`
//! field rather than relying on cell64 string prefixes.
//!
//! ## Layout
//!
//! ```text
//!   bits 63..60   mode      = 0b0001 (geo)
//!   bits 59..52   resolution = 21 (active 10 m grid v2)
//!   bits 51..44   base       = 0xab (geo aperture marker)
//!   bits 43..43   reserved   = 0
//!   bits 42..22   lat_q      (21 bits, [0, 2^21))
//!   bits 21..00   lng_q      (22 bits, [0, 2^22))
//! ```

use crate::cell64::{from_cell64, to_cell64, CodecError};
use emem_core::Cell;

const GEO_MODE: u64 = 1;

/// Bit count for the lat axis quantisation. 21 bits → 2,097,152 buckets
/// over the 180° lat range → ~9.54 m on the lat axis at the equator.
pub const GEO_LAT_BITS: u32 = 21;

/// Bit count for the lng axis quantisation. 22 bits → 4,194,304 buckets
/// over the 360° lng range → ~9.55 m on the lng axis at the equator.
pub const GEO_LNG_BITS: u32 = 22;

const GEO_LAT_MAX: u64 = (1u64 << GEO_LAT_BITS) - 1;
const GEO_LNG_MAX: u64 = (1u64 << GEO_LNG_BITS) - 1;
const GEO_LAT_MASK: u64 = GEO_LAT_MAX;
const GEO_LNG_MASK: u64 = GEO_LNG_MAX;

/// Encoded resolution tag. Distinct from the older 16-bit encoding
/// (`GEO_RES = 12`) so a legacy cell64 string fails `NotGeoCell`
/// instead of silently decoding into wrong-sized buckets.
const GEO_RES: u64 = GEO_LAT_BITS as u64;
const GEO_BASE: u64 = 0xab;
const GEO_PREFIX_MASK: u64 = 0xFFFF_F000_0000_0000;
const GEO_PREFIX: u64 = (GEO_MODE << 60) | (GEO_RES << 52) | (GEO_BASE << 44);

/// Encode a WGS-84 (lat_deg, lng_deg) point to a cell64.
/// Lat clamped to [-90, 90]; lng wrapped to [-180, 180).
pub fn cell_from_latlng(lat_deg: f64, lng_deg: f64) -> Cell {
    let lat = lat_deg.clamp(-90.0, 90.0);
    let lng = ((lng_deg + 180.0).rem_euclid(360.0)) - 180.0;
    let lat_q = (((lat + 90.0) / 180.0) * GEO_LAT_MAX as f64).round() as u64 & GEO_LAT_MASK;
    let lng_q = (((lng + 180.0) / 360.0) * GEO_LNG_MAX as f64).round() as u64 & GEO_LNG_MASK;
    let path = (lat_q << GEO_LNG_BITS) | lng_q;
    Cell::from_raw(GEO_PREFIX | path)
}

/// Convenience that emits the dot-bigram cell64 string.
pub fn cell64_from_latlng(lat_deg: f64, lng_deg: f64) -> String {
    to_cell64(cell_from_latlng(lat_deg, lng_deg))
}

/// Decode a cell64 string back to (lat_deg, lng_deg) — the **center**
/// of the lat/lng quantisation bucket. The bucket spans roughly
/// `180/2^21 ≈ 8.59e-5°` (~9.54 m) on lat and `360/2^22 ≈ 8.58e-5°`
/// (~9.55 m at equator) on lng — square ~10 m × ~10 m at the equator.
///
/// Returns `Err` if the cell64 is well-formed but not a `geo` cell at
/// the active resolution.
pub fn latlng_from_cell64(s: &str) -> Result<LatLng, CodecError> {
    let cell = from_cell64(s)?;
    if (cell.0 & GEO_PREFIX_MASK) != GEO_PREFIX {
        return Err(CodecError::NotGeoCell(cell.0));
    }
    let lng_q = cell.0 & GEO_LNG_MASK;
    let lat_q = (cell.0 >> GEO_LNG_BITS) & GEO_LAT_MASK;
    let lat_deg = (lat_q as f64 / GEO_LAT_MAX as f64) * 180.0 - 90.0;
    let lng_deg = (lng_q as f64 / GEO_LNG_MAX as f64) * 360.0 - 180.0;
    let half_lat = 90.0 / GEO_LAT_MAX as f64; // half-bucket edge in degrees
    let half_lng = 180.0 / GEO_LNG_MAX as f64;
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

    /// Round-trip a coordinate through encode → decode and back. The
    /// recovered lat/lng must land within one half-bucket on each axis.
    fn roundtrip_within_quantum(lat: f64, lng: f64) {
        let s = cell64_from_latlng(lat, lng);
        let back = latlng_from_cell64(&s).unwrap();
        let half_lat = 90.0 / GEO_LAT_MAX as f64;
        let half_lng = 180.0 / GEO_LNG_MAX as f64;
        let dlat = (back.lat_deg - lat).abs();
        let dlng = (back.lng_deg - lng).abs();
        assert!(
            dlat <= half_lat + 1e-9,
            "lat round-trip drift {dlat} > {half_lat} (lat={lat} lng={lng})"
        );
        assert!(
            dlng <= half_lng + 1e-9,
            "lng round-trip drift {dlng} > {half_lng} (lat={lat} lng={lng})"
        );
    }

    #[test]
    fn roundtrip_equator() {
        roundtrip_within_quantum(0.0, 0.0);
    }

    #[test]
    fn roundtrip_punjab() {
        roundtrip_within_quantum(30.5, 75.85);
    }

    #[test]
    fn roundtrip_high_lat() {
        roundtrip_within_quantum(60.0, -120.0);
    }

    #[test]
    fn roundtrip_antimeridian() {
        roundtrip_within_quantum(-30.0, 179.99);
    }

    /// Two queries 12 m apart (lat or lng) must produce *different*
    /// cells — the 10 m grid commitment. (12 m × 1°/111000 m ≈ 1.08e-4°,
    /// safely above the ~9.54 m lat half-bucket and ~9.55 m lng
    /// half-bucket at the equator.)
    #[test]
    fn cells_distinguish_12_metre_neighbors() {
        let s_a = cell64_from_latlng(0.0, 0.0);
        let s_lat = cell64_from_latlng(1.08e-4, 0.0);
        let s_lng = cell64_from_latlng(0.0, 1.08e-4);
        assert_ne!(s_a, s_lat, "12 m N must produce a distinct cell");
        assert_ne!(s_a, s_lng, "12 m E must produce a distinct cell");
    }

    /// Sub-quantum nudges (1 m) MUST produce the same cell — that's
    /// the cell's grain, not a bug.
    #[test]
    fn cells_collide_under_1_metre() {
        let s_a = cell64_from_latlng(0.0, 0.0);
        let s_b = cell64_from_latlng(9e-6, 9e-6);
        assert_eq!(s_a, s_b);
    }

    /// Square-at-equator: the bucket's lat extent in metres equals
    /// its lng extent in metres (within the rounding quantum).
    #[test]
    fn buckets_are_square_at_equator() {
        let s = cell64_from_latlng(0.0, 0.0);
        let info = latlng_from_cell64(&s).unwrap();
        let lat_extent_m = (info.bbox_deg.max_lat - info.bbox_deg.min_lat) * 111_000.0;
        let lng_extent_m = (info.bbox_deg.max_lng - info.bbox_deg.min_lng) * 111_000.0;
        assert!(
            (lat_extent_m - lng_extent_m).abs() < 0.05 * lat_extent_m,
            "expected square pixel at equator; got lat={lat_extent_m:.2}m lng={lng_extent_m:.2}m"
        );
        // Both extents must be in the 10 m ballpark (±2 m).
        assert!(
            (8.0..12.0).contains(&lat_extent_m),
            "lat extent {lat_extent_m:.2}m outside [8, 12] window"
        );
        assert!(
            (8.0..12.0).contains(&lng_extent_m),
            "lng extent {lng_extent_m:.2}m outside [8, 12] window"
        );
    }

    /// Legacy 16-bit-grid cell64 strings must NOT decode under the
    /// current 22-bit codec — they'd silently misplace a fact by
    /// hundreds of metres. The mode tag changed from 12→21, so a
    /// 16-bit-encoded raw word fails `NotGeoCell`.
    #[test]
    fn legacy_16bit_grid_rejected() {
        // raw word of a 16-bit cell64 with old GEO_RES=12 in bits 59..52.
        let legacy_raw: u64 = (1u64 << 60) | (12u64 << 52) | (0xabu64 << 44);
        let cell = Cell::from_raw(legacy_raw);
        let s = to_cell64(cell);
        assert!(matches!(
            latlng_from_cell64(&s),
            Err(CodecError::NotGeoCell(_))
        ));
    }
}
