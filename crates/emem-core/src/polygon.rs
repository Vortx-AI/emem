//! Polygon-mask primitives — point-in-polygon for cell-level filtering
//! when callers care about the *true* boundary of a region instead of
//! its bounding box.
//!
//! The geocoder layer in `emem-api-rest` returns Nominatim-shape
//! polygons (one outer ring, optional inner holes — multi-polygon is
//! flattened to a list of rings here). Downstream paths sample candidate
//! cells inside the bbox and call [`Polygon::contains`] to decide which
//! cells actually fall inside the region. Without this filter an
//! L-shaped admin boundary or a coastal feature whose bbox extends far
//! into the sea aggregates over cells that don't belong to the area —
//! over-counting by 25–40 % is typical, more for archipelagos.
//!
//! The implementation is intentionally dependency-free (no `geo`,
//! `geos`, or `proj`): WGS-84 is treated as planar Cartesian for the
//! ray-cast which is correct for all polygons that are not larger than
//! a continent. The polygon edges are short enough relative to the
//! Earth's curvature that the equirectangular approximation introduces
//! sub-pixel error on the cell grid.

use serde::{Deserialize, Serialize};

/// A planar polygon in `(lng, lat)` order. Holes are second-and-later
/// rings; the first ring is the outer boundary. Each ring is closed
/// implicitly (the last vertex does NOT need to repeat the first).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Polygon {
    /// Rings as `[[(lng, lat), …], …]`. Ring 0 is the outer boundary;
    /// rings 1.. are inner holes. Empty polygons are rejected at
    /// construction.
    pub rings: Vec<Vec<(f64, f64)>>,
}

impl Polygon {
    /// Build from GeoJSON-shape `Polygon` coordinates: a vector of
    /// rings, each ring a vector of `[lng, lat]` pairs. Returns `None`
    /// when the outer ring is missing or has fewer than 3 unique
    /// vertices.
    pub fn from_geojson_coords(coords: &[Vec<[f64; 2]>]) -> Option<Self> {
        if coords.is_empty() {
            return None;
        }
        let mut rings = Vec::with_capacity(coords.len());
        for ring in coords {
            // GeoJSON closes rings explicitly (last == first); we keep
            // them as-is — the ray-cast walks edges via `chunks(2)` so
            // the closure is harmless.
            if ring.len() < 4 {
                return None;
            }
            rings.push(ring.iter().map(|p| (p[0], p[1])).collect());
        }
        Some(Polygon { rings })
    }

    /// Build a `MultiPolygon` (list of `Polygon`s) from GeoJSON
    /// coordinates: `[[ring, hole, …], [ring, hole, …]]`. Returns the
    /// empty vector when no outer rings parse.
    pub fn many_from_geojson_multi(coords: &[Vec<Vec<[f64; 2]>>]) -> Vec<Self> {
        coords
            .iter()
            .filter_map(|polygon_coords| Polygon::from_geojson_coords(polygon_coords))
            .collect()
    }

    /// Outer-ring axis-aligned bbox `(lat_min, lat_max, lng_min, lng_max)`.
    /// Used to derive an envelope when a caller has the polygon but
    /// not a separate bbox — keeps the recall path's "sample inside
    /// bbox, filter by polygon" pipeline coherent.
    pub fn outer_bbox(&self) -> (f64, f64, f64, f64) {
        let outer = &self.rings[0];
        let mut lat_min = f64::INFINITY;
        let mut lat_max = f64::NEG_INFINITY;
        let mut lng_min = f64::INFINITY;
        let mut lng_max = f64::NEG_INFINITY;
        for &(lng, lat) in outer {
            if lat < lat_min {
                lat_min = lat;
            }
            if lat > lat_max {
                lat_max = lat;
            }
            if lng < lng_min {
                lng_min = lng;
            }
            if lng > lng_max {
                lng_max = lng;
            }
        }
        (lat_min, lat_max, lng_min, lng_max)
    }

    /// Test whether `(lat, lng)` is inside the polygon. Outer ring +
    /// holes are honoured (a point in a hole is reported as outside).
    /// Uses the standard non-zero ray-cast: a horizontal eastward ray
    /// from the test point counts edge crossings; odd → inside.
    pub fn contains(&self, lat: f64, lng: f64) -> bool {
        if self.rings.is_empty() {
            return false;
        }
        if !ring_contains(&self.rings[0], lat, lng) {
            return false;
        }
        for hole in &self.rings[1..] {
            if ring_contains(hole, lat, lng) {
                return false;
            }
        }
        true
    }
}

/// Ray-cast point-in-polygon for a single ring. The ray runs east at
/// constant `lat`; we count edges that straddle the ray and lie
/// strictly to the east of the test point. Edges that touch the ray
/// at a vertex use the half-open convention `(p1.lat <= lat) ^
/// (p2.lat <= lat)` so vertex hits don't double-count.
fn ring_contains(ring: &[(f64, f64)], lat: f64, lng: f64) -> bool {
    let n = ring.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = ring[i]; // (lng, lat)
        let (xj, yj) = ring[j];
        // The classic inclusion test: `(yi > lat) != (yj > lat)` means
        // the edge straddles the horizontal line; the second condition
        // checks the intersection lng is east of the test point.
        let intersects = (yi > lat) != (yj > lat) && {
            let denom = yj - yi;
            // denom == 0 means a horizontal edge; the straddle test
            // already excludes it because `yi > lat` and `yj > lat`
            // would both be the same boolean. Still, guard against the
            // degenerate case to keep the divide safe.
            if denom == 0.0 {
                false
            } else {
                let x_at_lat = xi + (lat - yi) * (xj - xi) / denom;
                lng < x_at_lat
            }
        };
        if intersects {
            inside = !inside;
        }
        j = i;
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Polygon {
        // (0,0)..(10,10) square in (lng, lat).
        Polygon::from_geojson_coords(&[vec![
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 10.0],
            [0.0, 10.0],
            [0.0, 0.0],
        ]])
        .unwrap()
    }

    #[test]
    fn rejects_short_ring() {
        assert!(Polygon::from_geojson_coords(&[vec![[0.0, 0.0], [1.0, 1.0]]]).is_none());
    }

    #[test]
    fn inside_outside_basic() {
        let p = square();
        assert!(p.contains(5.0, 5.0)); // centre
        assert!(!p.contains(15.0, 5.0)); // east of square
        assert!(!p.contains(-1.0, 5.0)); // west
        assert!(!p.contains(5.0, 20.0)); // north (lng=20 → far east of square edge)
        assert!(!p.contains(-5.0, -5.0));
    }

    #[test]
    fn outer_bbox_matches_extent() {
        let p = square();
        let bbox = p.outer_bbox();
        assert_eq!(bbox, (0.0, 10.0, 0.0, 10.0));
    }

    #[test]
    fn l_shape_rejects_concave_corner() {
        // L-shape: outer hull bbox is (0,10) x (0,10) but the NE
        // quadrant (5..10, 5..10) is excluded.
        let p = Polygon::from_geojson_coords(&[vec![
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 5.0],
            [5.0, 5.0],
            [5.0, 10.0],
            [0.0, 10.0],
            [0.0, 0.0],
        ]])
        .unwrap();
        assert!(p.contains(2.0, 2.0)); // SW
        assert!(p.contains(2.0, 8.0)); // NW
        assert!(p.contains(8.0, 2.0)); // SE
        assert!(!p.contains(8.0, 8.0)); // excluded NE corner
                                        // Bbox would lie (it's the full 10×10 square)
        assert_eq!(p.outer_bbox(), (0.0, 10.0, 0.0, 10.0));
    }

    #[test]
    fn hole_is_excluded() {
        // Outer 0..10 square with a (4..6, 4..6) hole.
        let p = Polygon::from_geojson_coords(&[
            vec![
                [0.0, 0.0],
                [10.0, 0.0],
                [10.0, 10.0],
                [0.0, 10.0],
                [0.0, 0.0],
            ],
            vec![[4.0, 4.0], [6.0, 4.0], [6.0, 6.0], [4.0, 6.0], [4.0, 4.0]],
        ])
        .unwrap();
        assert!(p.contains(2.0, 2.0)); // outer ring, outside hole
        assert!(!p.contains(5.0, 5.0)); // inside the hole
    }

    #[test]
    fn multipolygon_parse() {
        let coords = vec![
            vec![vec![
                [0.0, 0.0],
                [1.0, 0.0],
                [1.0, 1.0],
                [0.0, 1.0],
                [0.0, 0.0],
            ]],
            vec![vec![
                [10.0, 10.0],
                [11.0, 10.0],
                [11.0, 11.0],
                [10.0, 11.0],
                [10.0, 10.0],
            ]],
        ];
        let polys = Polygon::many_from_geojson_multi(&coords);
        assert_eq!(polys.len(), 2);
        assert!(polys[0].contains(0.5, 0.5));
        assert!(!polys[0].contains(10.5, 10.5));
        assert!(polys[1].contains(10.5, 10.5));
    }
}
