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

/// The operator identifier for [`regularize`], written into its record.
pub const REGULARIZE_OPERATOR: &str = "selective_regularisation@1";

/// Thresholds for [`regularize`]. The defaults are the rule the sub-metre boundary pipeline uses.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct RegularizeParams {
    /// A field becomes its minimum rotated rectangle when their IoU is at least this.
    pub rect_iou: f64,
    /// Otherwise it becomes its convex hull when their IoU is at least this.
    pub hull_iou: f64,
    /// Otherwise Douglas-Peucker with this tolerance, in metres.
    pub simplify_m: f64,
    /// A result whose area differs from the input by more than this share is refused.
    pub max_area_change: f64,
}

impl Default for RegularizeParams {
    fn default() -> Self {
        Self {
            rect_iou: 0.85,
            hull_iou: 0.90,
            simplify_m: 5.0,
            max_area_change: 0.15,
        }
    }
}

/// What [`regularize`] did to one field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegularKind {
    Rectangle,
    Hull,
    Simplified,
    Kept,
}

impl RegularKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RegularKind::Rectangle => "rectangle",
            RegularKind::Hull => "hull",
            RegularKind::Simplified => "simplified",
            RegularKind::Kept => "kept",
        }
    }
}

/// What [`regularize`] did to the whole set. Areas in square metres of the local projection.
#[derive(Debug, Clone, Serialize, Default)]
pub struct RegularizeReport {
    pub operator: &'static str,
    pub deterministic: bool,
    pub fields: usize,
    pub rectangle: usize,
    pub hull: usize,
    pub simplified: usize,
    pub kept: usize,
    pub vertices_before: usize,
    pub vertices_after: usize,
    pub area_before_m2: f64,
    pub area_after_m2: f64,
}

fn vertex_count(p: &Polygon<f64>) -> usize {
    p.exterior().0.len().saturating_sub(1)
        + p.interiors()
            .iter()
            .map(|r| r.0.len().saturating_sub(1))
            .sum::<usize>()
}

fn iou(a: &Polygon<f64>, b: &Polygon<f64>) -> f64 {
    let u = a.union(b).unsigned_area();
    if u <= 0.0 {
        return 0.0;
    }
    a.intersection(b).unsigned_area() / u
}

/// Replace each field's outline by a simpler one when the simpler one fits it closely.
///
/// Per field, in order: its minimum rotated rectangle if their IoU is at least `rect_iou`; else its
/// convex hull if their IoU is at least `hull_iou`; else Douglas-Peucker at `simplify_m`. The result
/// never takes ground that a neighbour's ORIGINAL outline holds, never changes the field's area by more
/// than `max_area_change`, and must be one valid polygon; otherwise the original is kept. A blanket
/// convex hull is not used: it fills L-shapes and notches and fuses adjacent plots.
///
/// Measured on 10 m outlines (two public 1 km boxes, 2026-11-20) this cuts vertices by about a fifth
/// and leaves the median area unchanged, and it does NOT move an outline onto a truer edge: the mean
/// NDVI gradient along the outlines was 0.0043 before and 0.0044 after. It is a cleaner drawing of the
/// same evidence, and the record says so.
pub fn regularize(
    polys: &mut [Option<Polygon<f64>>],
    prm: &RegularizeParams,
) -> (RegularizeReport, Vec<Option<RegularKind>>) {
    use geo::{ConvexHull, MinimumRotatedRect, Simplify, Validation};
    let originals: Vec<Option<Polygon<f64>>> = polys.to_vec();
    let tree = RTree::bulk_load(
        originals
            .iter()
            .enumerate()
            .filter_map(|(i, p)| {
                p.as_ref()
                    .and_then(aabb_of)
                    .map(|aabb| Item { idx: i, aabb })
            })
            .collect(),
    );
    let mut rep = RegularizeReport {
        operator: REGULARIZE_OPERATOR,
        deterministic: true,
        ..Default::default()
    };
    let mut kinds = vec![None; polys.len()];
    for (i, slot) in polys.iter_mut().enumerate() {
        let Some(orig) = originals[i].as_ref() else {
            continue;
        };
        let a0 = orig.unsigned_area();
        rep.fields += 1;
        rep.vertices_before += vertex_count(orig);
        rep.area_before_m2 += a0;
        let candidate = match orig.minimum_rotated_rect() {
            Some(r) if iou(orig, &r) >= prm.rect_iou => (r, RegularKind::Rectangle),
            _ => {
                let h = orig.convex_hull();
                if iou(orig, &h) >= prm.hull_iou {
                    (h, RegularKind::Hull)
                } else {
                    (orig.simplify(prm.simplify_m), RegularKind::Simplified)
                }
            }
        };
        let (cand, kind) = candidate;
        let neighbours: Vec<Polygon<f64>> = aabb_of(&cand)
            .map(|bb| {
                let mut hits: Vec<usize> = tree
                    .locate_in_envelope_intersecting(&bb)
                    .map(|it| it.idx)
                    .filter(|&j| j != i)
                    .collect();
                hits.sort_unstable();
                hits.into_iter()
                    .filter_map(|j| originals[j].clone())
                    .collect()
            })
            .unwrap_or_default();
        let trimmed = if neighbours.is_empty() {
            Some(cand)
        } else {
            largest(cand.difference(&MultiPolygon(neighbours)))
        };
        let accepted = trimmed.filter(|t| {
            a0 > 0.0 && (t.unsigned_area() - a0).abs() / a0 <= prm.max_area_change && t.is_valid()
        });
        let (out, kind) = match accepted {
            Some(t) => (t, kind),
            None => (orig.clone(), RegularKind::Kept),
        };
        match kind {
            RegularKind::Rectangle => rep.rectangle += 1,
            RegularKind::Hull => rep.hull += 1,
            RegularKind::Simplified => rep.simplified += 1,
            RegularKind::Kept => rep.kept += 1,
        }
        rep.vertices_after += vertex_count(&out);
        rep.area_after_m2 += out.unsigned_area();
        kinds[i] = Some(kind);
        *slot = Some(out);
    }
    (rep, kinds)
}

/// Pixels a field needs for each label of [`S2Support`]. The same thresholds the field reports use.
pub const SUPPORT_FIELD_MIN_PX: u32 = 30;
pub const SUPPORT_SMALL_MIN_PX: u32 = 4;
/// Inner buffer, in metres, that keeps mixed edge pixels out of a field's value.
pub const SUPPORT_INNER_BUFFER_M: f64 = 5.0;
/// Fields whose bounding box exceeds this many square metres are not counted (a district, not a field).
const SUPPORT_MAX_BBOX_M2: f64 = 25.0e6;

/// How many 10 m Sentinel-2 pixels a field holds, on the UTM grid of its centroid's zone.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct S2Support {
    /// Pixel centres inside the outline (holes excluded): what `field_series` counts with no buffer.
    pub px_centres: u32,
    /// Pixel centres at least 5 m inside the outline: the pixels a field value should rest on.
    pub px_inner_5m: u32,
    /// `field_value` (30 or more inner pixels), `small_field_value` (4 to 29) or `no_field_value`.
    pub label: &'static str,
    pub utm_epsg: u32,
}

struct Seg {
    a: [f64; 2],
    b: [f64; 2],
}
struct SegItem {
    idx: usize,
    aabb: AABB<[f64; 2]>,
}
impl RTreeObject for SegItem {
    type Envelope = AABB<[f64; 2]>;
    fn envelope(&self) -> Self::Envelope {
        self.aabb
    }
}

fn seg_dist(p: [f64; 2], s: &Seg) -> f64 {
    let (dx, dy) = (s.b[0] - s.a[0], s.b[1] - s.a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p[0] - s.a[0]) * dx + (p[1] - s.a[1]) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (qx, qy) = (s.a[0] + t * dx, s.a[1] + t * dy);
    ((p[0] - qx).powi(2) + (p[1] - qy).powi(2)).sqrt()
}

/// Count a field's Sentinel-2 pixels from its GeoJSON Polygon coordinates (`[lng, lat]` rings).
///
/// Sentinel-2 L2A tiles put 10 m pixel edges on multiples of 10 m in UTM, so centres sit at 10k + 5.
/// The zone is the one of the outline's centroid; a field read from a neighbouring zone's tile can
/// differ by a pixel or two. Holes are honoured (even-odd rule).
pub fn s2_support(coords: &serde_json::Value) -> Option<S2Support> {
    let rings_ll: Vec<Vec<[f64; 2]>> = coords
        .as_array()?
        .iter()
        .filter_map(|r| {
            let pts: Vec<[f64; 2]> = r
                .as_array()?
                .iter()
                .filter_map(|p| {
                    let a = p.as_array()?;
                    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?])
                })
                .collect();
            (pts.len() >= 4).then_some(pts)
        })
        .collect();
    let ext = rings_ll.first()?;
    let n = ext.len() as f64;
    let (clng, clat) = (
        ext.iter().map(|p| p[0]).sum::<f64>() / n,
        ext.iter().map(|p| p[1]).sum::<f64>() / n,
    );
    let zone = crate::proj::latlng_to_utm(clat, clng, None);
    let rings: Vec<Vec<[f64; 2]>> = rings_ll
        .iter()
        .map(|r| {
            r.iter()
                .map(|p| {
                    let u = crate::proj::latlng_to_utm(p[1], p[0], Some(zone.zone));
                    [u.easting, u.northing]
                })
                .collect()
        })
        .collect();
    let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for p in rings.iter().flatten() {
        minx = minx.min(p[0]);
        miny = miny.min(p[1]);
        maxx = maxx.max(p[0]);
        maxy = maxy.max(p[1]);
    }
    if !(maxx > minx && maxy > miny) || (maxx - minx) * (maxy - miny) > SUPPORT_MAX_BBOX_M2 {
        return None;
    }
    let segs: Vec<Seg> = rings
        .iter()
        .flat_map(|r| r.windows(2).map(|w| Seg { a: w[0], b: w[1] }))
        .collect();
    let tree = RTree::bulk_load(
        segs.iter()
            .enumerate()
            .map(|(idx, s)| SegItem {
                idx,
                aabb: AABB::from_corners(
                    [s.a[0].min(s.b[0]), s.a[1].min(s.b[1])],
                    [s.a[0].max(s.b[0]), s.a[1].max(s.b[1])],
                ),
            })
            .collect(),
    );
    let (mut centres, mut inner) = (0u32, 0u32);
    let mut y = (miny / 10.0).floor() * 10.0 + 5.0;
    while y <= maxy {
        // even-odd crossings of this row of pixel centres, over every ring
        let mut xs: Vec<f64> = segs
            .iter()
            .filter(|s| (s.a[1] <= y && y < s.b[1]) || (s.b[1] <= y && y < s.a[1]))
            .map(|s| s.a[0] + (y - s.a[1]) * (s.b[0] - s.a[0]) / (s.b[1] - s.a[1]))
            .collect();
        xs.sort_by(f64::total_cmp);
        for i in (0..xs.len().saturating_sub(1)).step_by(2) {
            let (x_in, x_out) = (xs[i], xs[i + 1]);
            let mut x = ((x_in - 5.0) / 10.0).ceil() * 10.0 + 5.0;
            while x < x_out {
                centres += 1;
                let r = SUPPORT_INNER_BUFFER_M;
                let near = AABB::from_corners([x - r, y - r], [x + r, y + r]);
                let too_close = tree
                    .locate_in_envelope_intersecting(&near)
                    // a centre exactly on the 5 m line counts as inside; 1 mm absorbs projection round-off
                    .any(|it| seg_dist([x, y], &segs[it.idx]) < r - 1e-3);
                if !too_close {
                    inner += 1;
                }
                x += 10.0;
            }
        }
        y += 10.0;
    }
    let label = if inner >= SUPPORT_FIELD_MIN_PX {
        "field_value"
    } else if inner >= SUPPORT_SMALL_MIN_PX {
        "small_field_value"
    } else {
        "no_field_value"
    };
    Some(S2Support {
        px_centres: centres,
        px_inner_5m: inner,
        label,
        utm_epsg: zone.epsg,
    })
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

    /// A 10 m staircase that is almost a rectangle becomes the rectangle; its area barely moves.
    #[test]
    fn a_staircase_rectangle_regularises_to_a_rectangle() {
        let stair = polygon![(x: 0.0, y: 0.0), (x: 100.0, y: 0.0), (x: 100.0, y: 40.0), (x: 90.0, y: 40.0),
            (x: 90.0, y: 50.0), (x: 0.0, y: 50.0), (x: 0.0, y: 0.0)];
        let a0 = stair.unsigned_area();
        let mut v = vec![Some(stair)];
        let (rep, kinds) = regularize(&mut v, &RegularizeParams::default());
        assert_eq!(kinds[0], Some(RegularKind::Rectangle));
        assert_eq!(rep.rectangle, 1);
        let out = v[0].as_ref().unwrap();
        assert_eq!(out.exterior().0.len(), 5, "a rectangle has four corners");
        assert!((out.unsigned_area() - a0).abs() / a0 <= 0.15);
        assert!(rep.vertices_after < rep.vertices_before);
    }

    /// An L-shape is not filled: neither the rectangle nor the hull fits it, so it is only simplified.
    #[test]
    fn an_l_shape_is_not_hulled() {
        let l = polygon![(x: 0.0, y: 0.0), (x: 100.0, y: 0.0), (x: 100.0, y: 30.0), (x: 30.0, y: 30.0),
            (x: 30.0, y: 100.0), (x: 0.0, y: 100.0), (x: 0.0, y: 0.0)];
        let mut v = vec![Some(l.clone())];
        let (_, kinds) = regularize(&mut v, &RegularizeParams::default());
        assert!(matches!(
            kinds[0],
            Some(RegularKind::Simplified) | Some(RegularKind::Kept)
        ));
        let out = v[0].as_ref().unwrap();
        assert!(
            (out.unsigned_area() - l.unsigned_area()).abs() < 1.0,
            "the notch stays open"
        );
    }

    /// A regularised field never takes ground its neighbour's original outline holds.
    #[test]
    fn regularisation_never_takes_a_neighbours_ground() {
        // a staircase whose rectangle would reach into the square beside it
        let stair = polygon![(x: 0.0, y: 0.0), (x: 100.0, y: 0.0), (x: 100.0, y: 40.0), (x: 90.0, y: 40.0),
            (x: 90.0, y: 50.0), (x: 0.0, y: 50.0), (x: 0.0, y: 0.0)];
        let nb = sq(90.0, 40.0, 10.0);
        let mut v = vec![Some(stair), Some(nb.clone())];
        regularize(&mut v, &RegularizeParams::default());
        let a = v[0].as_ref().unwrap();
        assert!(a.intersection(&nb).unsigned_area() < 1e-6);
    }

    fn square_ll(lng0: f64, lat0: f64, side_m: f64) -> serde_json::Value {
        // a square of side_m metres whose south-west corner sits ON a 10 m UTM grid line
        let z = crate::proj::latlng_to_utm(lat0, lng0, None);
        let (e0, n0) = (
            (z.easting / 10.0).floor() * 10.0,
            (z.northing / 10.0).floor() * 10.0,
        );
        let corner = |e: f64, n: f64| {
            let (lat, lng) = crate::proj::utm_to_latlng_with_epsg(e, n, z.epsg).unwrap();
            serde_json::json!([lng, lat])
        };
        serde_json::json!([[
            corner(e0, n0),
            corner(e0 + side_m, n0),
            corner(e0 + side_m, n0 + side_m),
            corner(e0, n0 + side_m),
            corner(e0, n0)
        ]])
    }

    /// A 100 m square on the grid holds 100 pixel centres; all of them are at least 5 m inside.
    #[test]
    fn support_counts_a_grid_aligned_square() {
        let s = s2_support(&square_ll(75.605, 30.745, 100.0)).unwrap();
        assert_eq!(s.px_centres, 100);
        assert_eq!(s.px_inner_5m, 100);
        assert_eq!(s.label, "field_value");
        assert_eq!(s.utm_epsg, 32643);
    }

    /// A 30 m square holds 9 centres (small-field value); a 10 m square holds 1 (no field value).
    #[test]
    fn support_labels_small_fields() {
        let s = s2_support(&square_ll(85.605, 25.925, 30.0)).unwrap();
        assert_eq!((s.px_centres, s.label), (9, "small_field_value"));
        let t = s2_support(&square_ll(85.605, 25.925, 10.0)).unwrap();
        assert_eq!((t.px_centres, t.label), (1, "no_field_value"));
    }
}
