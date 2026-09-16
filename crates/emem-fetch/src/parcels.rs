//! Parcels that tessellate: overlap resolution and duplicate removal over a set of field polygons,
//! with a synthesis record saying exactly what was done.
//!
//! The Fields of The World product this responder serves arrives as two annual vintages, and a
//! downstream refinement that straightens each ring on its own pushes neighbours into each other. On a
//! 3 km disc in Hapur district, measured on 2026-09-16, that was 137 ha of ground counted twice (7% of the
//! mapped area) and 11 rings wholly inside another. A renderer that extrudes each ring then draws every
//! shared edge twice and every overlap as a seam, which is the mosaic-of-cards look a partner asked us to
//! help fix. The fix belongs here, at the source, so every consumer gets tessellating parcels and a record
//! of the operation rather than re-deriving it.
//!
//! What this does, in order, deterministically:
//!   1. drop a polygon whose area lies (almost) entirely inside another: a duplicate, not a field;
//!   2. for every remaining intersecting pair, give the contested ground to the polygon for which it is
//!      the LARGER share of its own area -- the field whose claim on it is stronger -- and cut it from
//!      the other. Ties go to the larger polygon. Afterwards no two parcels share area.
//!
//! What this does NOT do, and says so: it does not close the gaps between parcels. On that plain the
//! strips between rings are 1.5 m at the median and up to 3 m -- bunds and cart tracks, real features the
//! rendering should draw as ground, not slivers to swallow. It does not regularise (that needs the
//! shared-edge graph, done downstream), and it does not infer parcels for unmapped ground. Each of those
//! is a different claim with a different provenance, and folding them into one flag would hide which
//! one a consumer is looking at.
//!
//! Provenance. The output is a deterministic function of the input rings and one parameter, so it is
//! re-runnable by anyone holding the same vintage: the synthesis record names the operator, its version,
//! the inputs and the parameter, in the shape the estimand work proposes for every derived quantity.

use geo::algorithm::area::Area;
use geo::algorithm::bool_ops::BooleanOps;
use geo::algorithm::bounding_rect::BoundingRect;
use geo::{Coord, LineString, MultiPolygon, Polygon};
use rstar::{RTree, RTreeObject, AABB};
use serde::Serialize;

/// The operator identifier written into every synthesis record. Bump the `@n` when the rule changes,
/// because a consumer comparing two responses needs to know whether the same function produced them.
pub const OPERATOR: &str = "overlap_resolution@1";

/// What the operation did, for the response. Areas in square metres of the local projection.
#[derive(Debug, Clone, Serialize, Default)]
pub struct CleanReport {
    pub operator: &'static str,
    pub deterministic: bool,
    pub input_polygons: usize,
    pub output_polygons: usize,
    pub duplicates_dropped: usize,
    pub pairs_resolved: usize,
    pub overlap_before_m2: f64,
    pub overlap_after_m2: f64,
    pub area_before_m2: f64,
    pub area_after_m2: f64,
    /// A polygon that lost all of its area to a neighbour with a stronger claim. Reported, not hidden.
    pub emptied: usize,
    pub gaps_note: &'static str,
}

struct Item {
    idx: usize,
    aabb: AABB<[f64; 2]>,
}
impl RTreeObject for Item {
    type Envelope = AABB<[f64; 2]>;
    fn envelope(&self) -> Self::Envelope {
        self.aabb
    }
}

fn aabb_of(p: &Polygon<f64>) -> Option<AABB<[f64; 2]>> {
    let r = p.bounding_rect()?;
    Some(AABB::from_corners(
        [r.min().x, r.min().y],
        [r.max().x, r.max().y],
    ))
}

fn largest(mp: MultiPolygon<f64>) -> Option<Polygon<f64>> {
    mp.0.into_iter()
        .max_by(|a, b| a.unsigned_area().total_cmp(&b.unsigned_area()))
}

/// Resolve overlaps in place. `polys` are in a planar metre frame. Returns the report.
///
/// `dup_share`: a polygon whose intersection with another covers at least this share of its own area
/// is a duplicate and is dropped (0.98 catches rings that differ by rounding; a genuinely nested field
/// smaller than 98% of its container is kept and resolved as an overlap instead).
pub fn resolve_overlaps(polys: &mut [Option<Polygon<f64>>], dup_share: f64) -> CleanReport {
    let n = polys.len();
    let mut rep = CleanReport {
        operator: OPERATOR,
        deterministic: true,
        input_polygons: n,
        gaps_note: "strips between parcels are left as ground: on this class of landscape they are bunds and tracks, not slivers",
        ..Default::default()
    };
    rep.area_before_m2 = polys.iter().flatten().map(|p| p.unsigned_area()).sum();

    let tree = RTree::bulk_load(
        polys
            .iter()
            .enumerate()
            .filter_map(|(i, p)| {
                p.as_ref()
                    .and_then(aabb_of)
                    .map(|aabb| Item { idx: i, aabb })
            })
            .collect(),
    );
    // Deterministic order: pairs (i, j) with i < j, by index. The same input always yields the same output.
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for (i, slot) in polys.iter().enumerate() {
        let Some(pi) = slot.as_ref() else {
            continue;
        };
        let Some(bb) = aabb_of(pi) else { continue };
        for it in tree.locate_in_envelope_intersecting(&bb) {
            if it.idx > i {
                pairs.push((i, it.idx));
            }
        }
    }
    pairs.sort_unstable();

    for (i, j) in pairs {
        let (Some(a), Some(b)) = (polys[i].clone(), polys[j].clone()) else {
            continue;
        };
        let inter = a.intersection(&b);
        let ia = inter.unsigned_area();
        if ia <= 1e-6 {
            continue;
        }
        rep.overlap_before_m2 += ia;
        let (aa, ab) = (a.unsigned_area(), b.unsigned_area());
        // 1. a duplicate: one ring (almost) wholly inside the other
        if ia >= dup_share * aa.min(ab) {
            let drop = if aa <= ab { i } else { j };
            polys[drop] = None;
            rep.duplicates_dropped += 1;
            continue;
        }
        // 2. the contested ground goes to the field for which it is the larger share of itself
        let (share_a, share_b) = (ia / aa, ia / ab);
        let loser = if share_a > share_b || (share_a == share_b && aa >= ab) {
            j
        } else {
            i
        };
        let (keeper, lost) = if loser == j { (a, b) } else { (b, a) };
        let cut = lost.difference(&keeper);
        polys[loser] = largest(cut);
        if polys[loser]
            .as_ref()
            .map(|p| p.unsigned_area() < 1e-6)
            .unwrap_or(true)
        {
            polys[loser] = None;
            rep.emptied += 1;
        }
        rep.pairs_resolved += 1;
    }

    // verify, do not assume: the residual overlap after the pass, measured the same way
    let mut after = 0.0;
    let live: Vec<(usize, &Polygon<f64>)> = polys
        .iter()
        .enumerate()
        .filter_map(|(i, p)| p.as_ref().map(|q| (i, q)))
        .collect();
    let tree2 = RTree::bulk_load(
        live.iter()
            .filter_map(|(i, p)| aabb_of(p).map(|aabb| Item { idx: *i, aabb }))
            .collect(),
    );
    for (i, p) in &live {
        let Some(bb) = aabb_of(p) else { continue };
        for it in tree2.locate_in_envelope_intersecting(&bb) {
            if it.idx > *i {
                if let Some(q) = polys[it.idx].as_ref() {
                    after += p.intersection(q).unsigned_area();
                }
            }
        }
    }
    rep.overlap_after_m2 = after;
    rep.area_after_m2 = live.iter().map(|(_, p)| p.unsigned_area()).sum();
    rep.output_polygons = live.len();
    rep
}

/// GeoJSON Polygon coordinates (lng, lat rings) to a planar polygon in metres, equirectangular about
/// `lat0`. Adequate for parcel-scale geometry: under 1 % distortion for the disc sizes this route serves.
pub fn to_planar(coords: &serde_json::Value, lat0: f64) -> Option<Polygon<f64>> {
    let k = (lat0.to_radians()).cos() * 111_320.0;
    let ring = |r: &serde_json::Value| -> Option<LineString<f64>> {
        let pts: Vec<Coord<f64>> = r
            .as_array()?
            .iter()
            .filter_map(|p| {
                let a = p.as_array()?;
                Some(Coord {
                    x: a.first()?.as_f64()? * k,
                    y: a.get(1)?.as_f64()? * 111_320.0,
                })
            })
            .collect();
        (pts.len() >= 4).then(|| LineString::from(pts))
    };
    let rings = coords.as_array()?;
    let ext = ring(rings.first()?)?;
    let holes: Vec<LineString<f64>> = rings.iter().skip(1).filter_map(ring).collect();
    Some(Polygon::new(ext, holes))
}

/// Back to GeoJSON coordinates in degrees.
pub fn to_geojson(p: &Polygon<f64>, lat0: f64) -> serde_json::Value {
    let k = (lat0.to_radians()).cos() * 111_320.0;
    let ring = |ls: &LineString<f64>| -> serde_json::Value {
        serde_json::Value::Array(
            ls.0.iter()
                .map(|c| serde_json::json!([c.x / k, c.y / 111_320.0]))
                .collect(),
        )
    };
    let mut out = vec![ring(p.exterior())];
    out.extend(p.interiors().iter().map(ring));
    serde_json::Value::Array(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::polygon;

    fn sq(x0: f64, y0: f64, s: f64) -> Polygon<f64> {
        polygon![(x: x0, y: y0), (x: x0 + s, y: y0), (x: x0 + s, y: y0 + s), (x: x0, y: y0 + s), (x: x0, y: y0)]
    }

    /// Two squares sharing a 10x40 strip. The strip is 25% of the small one and 10% of the big one, so
    /// it goes to the small one, and afterwards nothing overlaps and no area was lost.
    #[test]
    fn contested_ground_goes_to_the_field_it_is_more_of() {
        let mut v = vec![Some(sq(0.0, 0.0, 100.0)), Some(sq(90.0, 0.0, 40.0))];
        let rep = resolve_overlaps(&mut v, 0.98);
        assert_eq!(rep.pairs_resolved, 1);
        assert_eq!(rep.duplicates_dropped, 0);
        assert!(rep.overlap_before_m2 > 399.0, "{}", rep.overlap_before_m2);
        assert!(
            rep.overlap_after_m2 < 1e-6,
            "residual overlap {}",
            rep.overlap_after_m2
        );
        // the big square lost the strip: 10000 - 400; the small one kept all 1600
        let a = v[0].as_ref().unwrap().unsigned_area();
        let b = v[1].as_ref().unwrap().unsigned_area();
        assert!((a - 9600.0).abs() < 1e-6, "{a}");
        assert!((b - 1600.0).abs() < 1e-6, "{b}");
        assert!((rep.area_after_m2 - (rep.area_before_m2 - 400.0)).abs() < 1e-6);
    }

    /// A ring wholly inside another is a duplicate and is dropped, not carved.
    #[test]
    fn a_nested_duplicate_is_dropped() {
        let mut v = vec![Some(sq(0.0, 0.0, 100.0)), Some(sq(0.5, 0.5, 99.0))];
        let rep = resolve_overlaps(&mut v, 0.98);
        assert_eq!(rep.duplicates_dropped, 1);
        assert_eq!(rep.output_polygons, 1);
        assert!(v[1].is_none(), "the smaller ring is the one dropped");
    }

    /// The control: parcels that do not touch are returned exactly as they came, and the report says
    /// nothing happened. A cleaner that changed a clean input would be the worst kind of quiet.
    #[test]
    fn disjoint_parcels_are_untouched() {
        let before = vec![
            Some(sq(0.0, 0.0, 50.0)),
            Some(sq(60.0, 0.0, 50.0)),
            Some(sq(0.0, 60.0, 50.0)),
        ];
        let mut v = before.clone();
        let rep = resolve_overlaps(&mut v, 0.98);
        assert_eq!(
            (rep.pairs_resolved, rep.duplicates_dropped, rep.emptied),
            (0, 0, 0)
        );
        assert_eq!(rep.overlap_before_m2, 0.0);
        assert_eq!(rep.output_polygons, 3);
        for (a, b) in before.iter().zip(v.iter()) {
            assert_eq!(
                a.as_ref().unwrap().exterior(),
                b.as_ref().unwrap().exterior()
            );
        }
    }

    /// The projection round-trips to within a centimetre at parcel scale.
    #[test]
    fn planar_round_trip() {
        let gj = serde_json::json!([[
            [77.7380, 28.6278],
            [77.7390, 28.6278],
            [77.7390, 28.6288],
            [77.7380, 28.6288],
            [77.7380, 28.6278]
        ]]);
        let p = to_planar(&gj, 28.6283).unwrap();
        assert!(
            p.unsigned_area() > 9000.0 && p.unsigned_area() < 12000.0,
            "{}",
            p.unsigned_area()
        );
        let back = to_geojson(&p, 28.6283);
        let x = back[0][1][0].as_f64().unwrap();
        assert!((x - 77.7390).abs() < 1e-7, "{x}");
    }
}
