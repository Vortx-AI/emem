//! The whole-plot EUDR check: every 10 m pixel of a polygon, not a sample.
//!
//! `/v1/eudr_dds` read a polygon at about 110 sampled cells a hectare and
//! signed one fact per cell per band. A fresh 11 ha plot made 4,856 facts,
//! a large one 150,000 or more, and the receipt of a plot was mostly fact
//! ids. This path reads each source once as a raster window over the plot's
//! bounding box (in blocks when the window is larger than one read), applies
//! the per-cell rule `eudr_verdict_for` to every JRC GFC2020 pixel whose
//! centre is inside the polygon, and adds up true pixel areas. It signs one
//! derivative fact per plot. The fact holds the areas and counts, the rule,
//! the content id of each source window (stored in the artifact store and
//! served at `/v1/artifacts/<cid>`), the files read, and commitments to the
//! polygon and to the caller's plot id. Anyone who holds the polygon can
//! fetch the windows, check them against their ids and recompute every
//! number: `tools/verifiers/emem-eudr-due-diligence/verify_plot.py`.
//!
//! The grid is JRC GFC2020's own (about 10 m). Hansen GFC and JRC TMF are
//! 30 m: each is read at the pixel of its own grid that contains the 10 m
//! pixel centre, so no value is resampled. A pixel is in the plot when its
//! centre is inside the outer ring and outside every hole, by the same
//! eastward ray cast `emem_core::Polygon::contains` uses. Pixel areas are
//! areas on the WGS84 ellipsoid of the pixel's latitude band.

use std::collections::BTreeMap;

use emem_codec::grid::{encode_grid, GridHeader};
use emem_fact::{Derivation, DerivativeFact, Fact};
use serde_json::{json, Value as JsonValue};

use crate::band_raster::{self, BBox, ForestTile, LayerError, LayerWindow};
use crate::{AppState, ForestRule};

pub(crate) const JRC: &str = "jrc_gfc2020.forest_2020";
pub(crate) const TREECOVER: &str = "forest_change.treecover2000";
pub(crate) const LOSSYEAR: &str = "forest_change.lossyear";
pub(crate) const TMF: &str = "jrc_tmf.deforestation_year";

/// The key the per-plot fact is signed under.
pub(crate) const FN_KEY: &str = "eudr_plot_raster@1";

/// JRC pixels a side of one chunk (about 19 km). A plot larger than this is
/// counted chunk by chunk, so the memory one plot holds is bounded and any
/// plot size is read at full resolution.
const CHUNK_PX: i64 = 2048;
/// Chunks of one plot read at once.
const CHUNKS_AT_ONCE: usize = 2;

/// TMF-only loss pixels the answer names for review, at most.
const REVIEW_PIXELS_MAX: usize = 64;
/// Failing and review pixels the answer names as examples, at most.
const EXAMPLE_PIXELS_MAX: usize = 32;

/// Domain tags. The plot commitment is
/// `blake3(PLOT_TAG || salt[16] || plot_id as UTF-8)`; the geometry
/// commitment is `blake3(GEOMETRY_TAG || per ring: u32 LE vertex count, then
/// each vertex as f64 LE lng, f64 LE lat)`, over the vertices as sent.
pub(crate) const PLOT_TAG: &[u8] = b"emem.eudr.plot_commitment.v1\0";
pub(crate) const GEOMETRY_TAG: &[u8] = b"emem.eudr.geometry.v1\0";

/// The caller's `plot_salt` (32 hex characters), or a fresh random one.
pub(crate) fn salt_from(hex: Option<&str>) -> Result<[u8; 16], String> {
    let Some(hex) = hex else {
        use rand::RngCore;
        let mut salt = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut salt);
        return Ok(salt);
    };
    let bytes = data_encoding::HEXLOWER_PERMISSIVE
        .decode(hex.trim().as_bytes())
        .map_err(|_| "`plot_salt` is 32 hex characters (16 bytes)".to_string())?;
    bytes
        .try_into()
        .map_err(|_| "`plot_salt` is 32 hex characters (16 bytes)".to_string())
}

/// The commitment that stands for the caller's plot id in a signed fact.
pub(crate) fn plot_commitment(salt: &[u8; 16], plot_id: &str) -> String {
    let mut h = blake3::Hasher::new();
    h.update(PLOT_TAG);
    h.update(salt);
    h.update(plot_id.as_bytes());
    h.finalize().to_hex().to_string()
}

/// The commitment to the polygon the pixels were counted in.
pub(crate) fn geometry_commitment(rings: &[Vec<(f64, f64)>]) -> String {
    let mut h = blake3::Hasher::new();
    h.update(GEOMETRY_TAG);
    for ring in rings {
        h.update(&(ring.len() as u32).to_le_bytes());
        for (lng, lat) in ring {
            h.update(&lng.to_le_bytes());
            h.update(&lat.to_le_bytes());
        }
    }
    h.finalize().to_hex().to_string()
}

/// A `{bbox}` plot as the ring the raster method counts pixels in.
pub(crate) fn bbox_ring(bbox: (f64, f64, f64, f64)) -> Vec<(f64, f64)> {
    let (mn_la, mx_la, mn_ln, mx_ln) = bbox;
    vec![
        (mn_ln, mn_la),
        (mx_ln, mn_la),
        (mx_ln, mx_la),
        (mn_ln, mx_la),
        (mn_ln, mn_la),
    ]
}

/// WGS84 area in m² of a strip `dlng` degrees wide between two latitudes.
pub(crate) fn band_area_m2(lat_a: f64, lat_b: f64, dlng: f64) -> f64 {
    const A: f64 = 6_378_137.0;
    const F: f64 = 1.0 / 298.257_223_563;
    let e2 = F * (2.0 - F);
    let e = e2.sqrt();
    let b2 = A * A * (1.0 - e2);
    let q = |lat: f64| {
        let s = lat.to_radians().sin();
        s / (1.0 - e2 * s * s) + (1.0 / (2.0 * e)) * ((1.0 + e * s) / (1.0 - e * s)).ln()
    };
    let (lo, hi) = (lat_a.min(lat_b), lat_a.max(lat_b));
    (b2 * dlng.abs().to_radians() / 2.0 * (q(hi) - q(lo))).abs()
}

/// One ring edge clipped to a pixel row, in pixel units: `[u0, v0, u1, v1]`.
type Seg = [f64; 4];

/// The ring's edges clipped to the row `v0..=v1`, in pixel units (`u` east
/// and `v` south of the window's north-west corner). An edge along the row
/// adds nothing to an area integral in `v` and is left out.
fn row_segments(ring: &[(f64, f64)], v0: f64, v1: f64) -> Vec<Seg> {
    let n = ring.len();
    let mut out = Vec::new();
    for i in 0..n {
        let (ua, va) = ring[i];
        let (ub, vb) = ring[(i + 1) % n];
        if va == vb || va.max(vb) <= v0 || va.min(vb) >= v1 {
            continue;
        }
        let u_at = |v: f64| {
            if v == va {
                ua
            } else if v == vb {
                ub
            } else {
                ua + (v - va) * (ub - ua) / (vb - va)
            }
        };
        let (vs, ve) = (va.clamp(v0, v1), vb.clamp(v0, v1));
        out.push([u_at(vs), vs, u_at(ve), ve]);
    }
    out
}

/// The integral of `min(u, x)` in `v` along one segment.
fn min_integral(s: &Seg, x: f64) -> f64 {
    let [u0, v0, u1, v1] = *s;
    if u0 <= x && u1 <= x {
        (u0 + u1) / 2.0 * (v1 - v0)
    } else if u0 >= x && u1 >= x {
        x * (v1 - v0)
    } else {
        let vm = v0 + (x - u0) * (v1 - v0) / (u1 - u0);
        if u0 < x {
            (u0 + x) / 2.0 * (vm - v0) + x * (v1 - vm)
        } else {
            x * (vm - v0) + (x + u1) / 2.0 * (v1 - vm)
        }
    }
}

/// The ring's area inside the row and west of `u = x`, times the ring's
/// orientation: by Green's theorem, the sum over its clipped edges of the
/// integral of `min(u, x)` in `v`.
fn left_area(segs: &[Seg], x: f64) -> f64 {
    let mut a = 0.0;
    for s in segs {
        a += min_integral(s, x);
    }
    a
}

/// +1 or -1: the orientation of a ring in pixel units.
fn ring_sign(ring: &[(f64, f64)]) -> f64 {
    let n = ring.len();
    let mut a = 0.0;
    for i in 0..n {
        let (ua, va) = ring[i];
        let (ub, vb) = ring[(i + 1) % n];
        a += (ua + ub) / 2.0 * (vb - va);
    }
    if a < 0.0 {
        -1.0
    } else {
        1.0
    }
}

/// A pixel covered by less of the polygon than this is not counted: float
/// noise on an edge that runs along a pixel side.
const COVER_EPS: f64 = 1e-9;

/// The index of the pixel of `h` that contains the point, if `h` holds it.
pub(crate) fn index_in(h: &GridHeader, lat: f64, lng: f64) -> Option<usize> {
    let col = ((lng - (h.lng0 - h.dlng * 0.5)) / h.dlng + 1e-6).floor();
    let row = ((lat - (h.lat0 - h.dlat * 0.5)) / h.dlat + 1e-6).floor();
    if !(col >= 0.0 && row >= 0.0 && col < h.width as f64 && row < h.height as f64) {
        return None;
    }
    Some(row as usize * h.width as usize + col as usize)
}

fn whole(v: f32, max: f32) -> Option<i64> {
    (v.is_finite() && v >= 0.0 && v <= max && v.fract() == 0.0).then_some(v as i64)
}

/// JRC GFC2020 is 0 or 1; anything else is no reading.
fn jrc_value(v: f32) -> Option<i64> {
    if v == 0.0 {
        Some(0)
    } else if v == 1.0 {
        Some(1)
    } else {
        None
    }
}

fn read_at(w: Option<&LayerWindow>, lat: f64, lng: f64) -> Option<f32> {
    let w = w?;
    let v = *w.values.get(index_in(&w.header, lat, lng)?)?;
    (!v.is_nan()).then_some(v)
}

/// A pixel count and the area it covers.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub(crate) struct Tally {
    pub(crate) pixels: u64,
    pub(crate) area_m2: f64,
}

impl Tally {
    fn add(&mut self, area_m2: f64) {
        self.pixels += 1;
        self.area_m2 += area_m2;
    }
    fn ha(&self) -> f64 {
        ha(self.area_m2)
    }
    fn json(&self) -> JsonValue {
        json!({"pixels": self.pixels, "area_ha": self.ha()})
    }
}

fn ha(m2: f64) -> f64 {
    (m2 / 1e4 * 1e6).round() / 1e6
}

/// One pixel's readings, named for the answer.
#[derive(Debug, Clone)]
pub(crate) struct PixelRow {
    pub(crate) lat: f64,
    pub(crate) lng: f64,
    pub(crate) verdict: u8,
    pub(crate) jrc: Option<i64>,
    pub(crate) treecover: Option<i64>,
    pub(crate) lossyear: Option<i64>,
    pub(crate) tmf: Option<i64>,
}

/// Every count the plot verdict and its support rest on.
#[derive(Debug, Default)]
pub(crate) struct Classified {
    pub(crate) plot: Tally,
    pub(crate) pass: Tally,
    pub(crate) fail: Tally,
    pub(crate) not_in_scope: Tally,
    pub(crate) indeterminate: Tally,
    pub(crate) review: Tally,
    /// Pixels the rule called forest at the cut-off: pass or fail, less the
    /// passes on JRC non-forest.
    pub(crate) forest_at_cutoff: Tally,
    /// Pass pixels JRC calls non-forest that Hansen sees as tree cover.
    pub(crate) hansen_tree_cover_jrc_non_forest: Tally,
    pub(crate) jrc_unavailable_hansen_only: Tally,
    pub(crate) both_baselines: u64,
    pub(crate) borderline: u64,
    pub(crate) jrc_read: u64,
    pub(crate) hansen_read: u64,
    pub(crate) fail_jrc_forest: Tally,
    pub(crate) fail_hansen_jrc_non_forest: Tally,
    pub(crate) fail_hansen_jrc_missing: Tally,
    /// Failing and review areas by the source that saw the loss:
    /// `[Hansen only, TMF only, both]`.
    pub(crate) fail_seen_by: [Tally; 3],
    pub(crate) review_seen_by: [Tally; 3],
    pub(crate) hansen_years: BTreeMap<i64, Tally>,
    pub(crate) hansen_no_loss: u64,
    pub(crate) hansen_unknown: u64,
    pub(crate) hansen_after: Tally,
    pub(crate) tmf_years: BTreeMap<i64, Tally>,
    pub(crate) tmf_no_loss: u64,
    pub(crate) tmf_unknown: u64,
    pub(crate) tmf_after: Tally,
    pub(crate) tmf: crate::TmfCounts,
    pub(crate) tmf_only: Tally,
    pub(crate) tmf_review: Vec<(f64, f64)>,
    pub(crate) examples: Vec<PixelRow>,
    /// Side of a pixel in metres, at the plot's mean pixel area.
    pub(crate) pixel_side_m: f64,
    /// Full areas of the counted pixels, for `pixel_side_m`.
    pub(crate) full_px_m2: f64,
}

/// Applies the per-cell rule to every JRC pixel the polygon covers and adds
/// up areas. Each pixel counts with the fraction of it inside the polygon
/// (outer ring less holes), computed exactly in degree space, times its
/// geodesic area. `rings[0]` is the outer ring, the rest are holes, each
/// `(lng, lat)`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn classify(
    jrc: &LayerWindow,
    treecover: Option<&LayerWindow>,
    lossyear: Option<&LayerWindow>,
    tmf: Option<&LayerWindow>,
    rings: &[Vec<(f64, f64)>],
    owns: &Owns,
    cutoff_year: i64,
    rule: ForestRule,
) -> Classified {
    let mut c = Classified::default();
    let h = &jrc.header;
    let half = h.dlat.abs() / 2.0;
    if rings.is_empty() {
        return c;
    }
    let left = h.lng0 - h.dlng * 0.5;
    let top = h.lat0 - h.dlat * 0.5;
    let uv: Vec<Vec<(f64, f64)>> = rings
        .iter()
        .map(|ring| {
            ring.iter()
                .map(|&(lng, lat)| ((lng - left) / h.dlng, (lat - top) / h.dlat))
                .collect()
        })
        .collect();
    let signs: Vec<f64> = uv.iter().map(|r| ring_sign(r)).collect();
    for r in 0..h.height {
        let (v0, v1) = (r as f64, r as f64 + 1.0);
        let segs: Vec<Vec<Seg>> = uv.iter().map(|ring| row_segments(ring, v0, v1)).collect();
        if segs[0].is_empty() {
            continue;
        }
        let (mut umin, mut umax) = (f64::INFINITY, f64::NEG_INFINITY);
        for s in &segs[0] {
            umin = umin.min(s[0]).min(s[2]);
            umax = umax.max(s[0]).max(s[2]);
        }
        let c0 = umin.floor().max(0.0) as i64;
        let c1 = (umax.ceil() as i64).min(i64::from(h.width));
        if c1 <= c0 {
            continue;
        }
        let edges: Vec<Vec<f64>> = segs
            .iter()
            .map(|ss| (c0..=c1).map(|k| left_area(ss, k as f64)).collect())
            .collect();
        let lat = h.lat0 + r as f64 * h.dlat;
        let px_area = band_area_m2(lat - half, lat + half, h.dlng);
        for col in c0..c1 {
            let k = (col - c0) as usize;
            let mut cover = signs[0] * (edges[0][k + 1] - edges[0][k]);
            for (hole, sign) in edges[1..].iter().zip(&signs[1..]) {
                cover -= sign * (hole[k + 1] - hole[k]);
            }
            let cover = cover.clamp(0.0, 1.0);
            if cover <= COVER_EPS {
                continue;
            }
            let lng = h.lng0 + col as f64 * h.dlng;
            if !owns.holds(lat, lng) {
                continue;
            }
            let area = cover * px_area;
            c.full_px_m2 += px_area;
            let idx = r as usize * h.width as usize + col as usize;
            let j = jrc.values.get(idx).copied().and_then(jrc_value);
            let tc = read_at(treecover, lat, lng).and_then(|v| whole(v, 100.0));
            let ly = read_at(lossyear, lat, lng).and_then(|v| whole(v, 2100.0));
            let t = read_at(tmf, lat, lng).and_then(|v| whole(v, 65_535.0));
            // Loss after the cut-off is Hansen's or TMF's: both vote here.
            let (verdict, refinement) =
                crate::eudr_verdict_for(j, tc, ly, t, None, None, cutoff_year, rule);
            if matches!(verdict, 2 | 7) {
                let hl = matches!(ly, Some(v) if v > cutoff_year);
                let tl = matches!(t, Some(v) if v > cutoff_year);
                let by = if verdict == 2 {
                    &mut c.fail_seen_by
                } else {
                    &mut c.review_seen_by
                };
                match (hl, tl) {
                    (true, true) => by[2].add(area),
                    (false, true) => by[1].add(area),
                    _ => by[0].add(area),
                }
            }
            c.plot.add(area);
            match verdict {
                1 => c.pass.add(area),
                2 => c.fail.add(area),
                3 => c.not_in_scope.add(area),
                7 => c.review.add(area),
                _ => c.indeterminate.add(area),
            }
            // A pass on JRC non-forest is a pass, not forest at the cut-off.
            if matches!(verdict, 1 | 2) && refinement != Some("hansen_tree_cover_jrc_non_forest") {
                c.forest_at_cutoff.add(area);
            }
            match refinement {
                Some("hansen_tree_cover_jrc_non_forest") => {
                    c.hansen_tree_cover_jrc_non_forest.add(area)
                }
                Some("jrc_unavailable_hansen_only") => c.jrc_unavailable_hansen_only.add(area),
                _ => {}
            }
            if verdict == 2 {
                match j {
                    Some(v) if v >= 1 => c.fail_jrc_forest.add(area),
                    Some(_) => c.fail_hansen_jrc_non_forest.add(area),
                    None => c.fail_hansen_jrc_missing.add(area),
                }
            }
            if matches!(verdict, 2 | 7) && c.examples.len() < EXAMPLE_PIXELS_MAX {
                c.examples.push(PixelRow {
                    lat,
                    lng,
                    verdict,
                    jrc: j,
                    treecover: tc,
                    lossyear: ly,
                    tmf: t,
                });
            }
            c.jrc_read += u64::from(j.is_some());
            c.hansen_read += u64::from(tc.is_some());
            c.both_baselines += u64::from(j.is_some() && ly.is_some());
            c.borderline += u64::from(matches!(tc, Some(v) if (8..=12).contains(&v)));
            match ly {
                None => c.hansen_unknown += 1,
                Some(0) => c.hansen_no_loss += 1,
                Some(y) => {
                    c.hansen_years.entry(y).or_default().add(area);
                    if y > cutoff_year {
                        c.hansen_after.add(area);
                    }
                }
            }
            match t {
                None => {
                    c.tmf_unknown += 1;
                    c.tmf.unread += 1;
                }
                Some(y) => {
                    if y == 0 {
                        c.tmf_no_loss += 1;
                    } else {
                        c.tmf_years.entry(y).or_default().add(area);
                        if y > cutoff_year {
                            c.tmf_after.add(area);
                        }
                    }
                    c.tmf.read += 1;
                    let tl = y > cutoff_year;
                    let hl = matches!(ly, Some(v) if v > cutoff_year);
                    match (hl, tl) {
                        (true, true) => c.tmf.both += 1,
                        (true, false) => c.tmf.hansen_only += 1,
                        (false, true) => {
                            c.tmf.tmf_only += 1;
                            c.tmf_only.add(area);
                            if c.tmf_review.len() < REVIEW_PIXELS_MAX {
                                c.tmf_review.push((lat, lng));
                            }
                        }
                        (false, false) => c.tmf.neither += 1,
                    }
                }
            }
        }
    }
    c.finish();
    c
}

/// The pixels one chunk of a plot counts: those whose centre has
/// `min_lng <= lng < max_lng` and `min_lat < lat <= max_lat`. A side that
/// is `None` is open. The chunks of a plot share their sides, so every pixel
/// centre belongs to exactly one chunk.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Owns {
    pub(crate) min_lng: Option<f64>,
    pub(crate) max_lng: Option<f64>,
    pub(crate) min_lat: Option<f64>,
    pub(crate) max_lat: Option<f64>,
}

impl Owns {
    fn holds(&self, lat: f64, lng: f64) -> bool {
        self.min_lng.is_none_or(|v| lng >= v)
            && self.max_lng.is_none_or(|v| lng < v)
            && self.min_lat.is_none_or(|v| lat > v)
            && self.max_lat.is_none_or(|v| lat <= v)
    }

    fn json(&self) -> JsonValue {
        json!({
            "min_lng": self.min_lng, "max_lng": self.max_lng,
            "min_lat": self.min_lat, "max_lat": self.max_lat,
            "rule": "min_lng <= lng < max_lng and min_lat < lat <= max_lat, at the pixel centre; null is open",
        })
    }
}

/// One chunk of a plot: the bbox its windows cover and the pixels it counts.
#[derive(Debug, Clone)]
struct Chunk {
    bbox: BBox,
    owns: Owns,
}

/// The plot's bbox cut into chunks of at most `CHUNK_PX` JRC pixels a side,
/// north to south and then west to east. `width` and `height` are the plot's
/// JRC window in pixels.
fn chunks_of(b: &BBox, width: i64, height: i64) -> Vec<Chunk> {
    let nx = ((width + CHUNK_PX - 1) / CHUNK_PX).max(1);
    let ny = ((height + CHUNK_PX - 1) / CHUNK_PX).max(1);
    let lng_at = |i: i64| -> Option<f64> {
        (0 < i && i < nx).then(|| b.min_lng + i as f64 * (b.max_lng - b.min_lng) / nx as f64)
    };
    let lat_at = |j: i64| -> Option<f64> {
        (0 < j && j < ny).then(|| b.max_lat - j as f64 * (b.max_lat - b.min_lat) / ny as f64)
    };
    let mut out = Vec::with_capacity((nx * ny) as usize);
    for j in 0..ny {
        for i in 0..nx {
            let owns = Owns {
                min_lng: lng_at(i),
                max_lng: lng_at(i + 1),
                min_lat: lat_at(j + 1),
                max_lat: lat_at(j),
            };
            out.push(Chunk {
                bbox: BBox {
                    min_lng: owns.min_lng.unwrap_or(b.min_lng),
                    max_lng: owns.max_lng.unwrap_or(b.max_lng),
                    min_lat: owns.min_lat.unwrap_or(b.min_lat),
                    max_lat: owns.max_lat.unwrap_or(b.max_lat),
                },
                owns,
            });
        }
    }
    out
}

impl Tally {
    fn absorb(&mut self, o: &Tally) {
        self.pixels += o.pixels;
        self.area_m2 += o.area_m2;
    }
}

impl Classified {
    fn finish(&mut self) {
        if self.plot.pixels > 0 {
            self.pixel_side_m = (self.full_px_m2 / self.plot.pixels as f64).sqrt();
        }
    }

    /// Adds a chunk's counts, in chunk order, so a verifier that adds the
    /// chunks in the same order gets the same areas to the last bit.
    fn absorb(&mut self, o: Classified) {
        for (a, b) in [
            (&mut self.plot, &o.plot),
            (&mut self.pass, &o.pass),
            (&mut self.fail, &o.fail),
            (&mut self.not_in_scope, &o.not_in_scope),
            (&mut self.indeterminate, &o.indeterminate),
            (&mut self.review, &o.review),
            (&mut self.forest_at_cutoff, &o.forest_at_cutoff),
            (
                &mut self.hansen_tree_cover_jrc_non_forest,
                &o.hansen_tree_cover_jrc_non_forest,
            ),
            (
                &mut self.jrc_unavailable_hansen_only,
                &o.jrc_unavailable_hansen_only,
            ),
            (&mut self.fail_jrc_forest, &o.fail_jrc_forest),
            (
                &mut self.fail_hansen_jrc_non_forest,
                &o.fail_hansen_jrc_non_forest,
            ),
            (
                &mut self.fail_hansen_jrc_missing,
                &o.fail_hansen_jrc_missing,
            ),
            (&mut self.hansen_after, &o.hansen_after),
            (&mut self.tmf_after, &o.tmf_after),
            (&mut self.tmf_only, &o.tmf_only),
        ] {
            a.absorb(b);
        }
        self.full_px_m2 += o.full_px_m2;
        for k in 0..3 {
            self.fail_seen_by[k].absorb(&o.fail_seen_by[k]);
            self.review_seen_by[k].absorb(&o.review_seen_by[k]);
        }
        self.both_baselines += o.both_baselines;
        self.borderline += o.borderline;
        self.jrc_read += o.jrc_read;
        self.hansen_read += o.hansen_read;
        self.hansen_no_loss += o.hansen_no_loss;
        self.hansen_unknown += o.hansen_unknown;
        self.tmf_no_loss += o.tmf_no_loss;
        self.tmf_unknown += o.tmf_unknown;
        for (y, t) in &o.hansen_years {
            self.hansen_years.entry(*y).or_default().absorb(t);
        }
        for (y, t) in &o.tmf_years {
            self.tmf_years.entry(*y).or_default().absorb(t);
        }
        self.tmf.read += o.tmf.read;
        self.tmf.unread += o.tmf.unread;
        self.tmf.both += o.tmf.both;
        self.tmf.hansen_only += o.tmf.hansen_only;
        self.tmf.tmf_only += o.tmf.tmf_only;
        self.tmf.neither += o.tmf.neither;
        let room = REVIEW_PIXELS_MAX.saturating_sub(self.tmf_review.len());
        self.tmf_review.extend(o.tmf_review.into_iter().take(room));
        let room = EXAMPLE_PIXELS_MAX.saturating_sub(self.examples.len());
        self.examples.extend(o.examples.into_iter().take(room));
        self.finish();
    }
}

/// The plot verdict from the covered areas, with the 0.5 ha floor applied to
/// those areas: no projection, and no edge pixel decided by where its centre
/// falls.
pub(crate) fn plot_verdict(c: &Classified) -> (u8, f64, crate::MmuDemotion) {
    let (raw, fail_fraction) = crate::aggregate_plot_verdict_measure(
        c.plot.area_m2,
        c.fail.area_m2,
        c.indeterminate.area_m2,
        c.not_in_scope.area_m2,
        c.review.area_m2,
    );
    let mmu = if matches!(raw, 2 | 7) {
        crate::mmu_by_area(
            c.fail.area_m2 / 1e4,
            c.review.area_m2 / 1e4,
            c.fail.pixels > 0,
            c.review.pixels > 0,
        )
    } else {
        crate::MmuDemotion {
            verdict_code: raw,
            failing_area_ha: c.fail.area_m2 / 1e4,
            mmu_floor_applied: false,
        }
    };
    (raw, fail_fraction, mmu)
}

/// One source as the plot fact names it.
#[derive(Debug, Clone)]
pub(crate) struct LayerRecord {
    pub(crate) band: &'static str,
    pub(crate) scheme: String,
    pub(crate) files: Vec<String>,
    pub(crate) artifact_cid: Option<String>,
    pub(crate) byte_len: usize,
    pub(crate) grid: Option<GridHeader>,
    pub(crate) error: Option<String>,
}

impl LayerRecord {
    fn json(&self) -> JsonValue {
        json!({
            "scheme": self.scheme,
            "files": self.files,
            "artifact_cid": self.artifact_cid,
            "byte_len": self.byte_len,
            "media_type": "application/x.emem-grid-f32.v1",
            "grid": self.grid.map(|g| json!({
                "width": g.width, "height": g.height,
                "lat0": g.lat0, "lng0": g.lng0, "dlat": g.dlat, "dlng": g.dlng,
            })),
            "native_m": self.grid.map(|g| native_m(&g)),
            "resolution": "full: the TIFF's first image (IFD 0), never an overview",
            "error": self.error,
        })
    }
}

/// The pixel's east-west and north-south sides in metres at the grid's
/// middle row, on WGS84: what "native resolution" means for this window.
fn native_m(g: &GridHeader) -> JsonValue {
    let lat = (g.lat0 + g.dlat * (g.height as f64 - 1.0) / 2.0).to_radians();
    let per_deg_lat = 111_132.954 - 559.822 * (2.0 * lat).cos() + 1.175 * (4.0 * lat).cos();
    let per_deg_lng = 111_412.84 * lat.cos() - 93.5 * (3.0 * lat).cos() + 0.118 * (5.0 * lat).cos();
    let r = |m: f64| (m * 100.0).round() / 100.0;
    json!([r(g.dlng.abs() * per_deg_lng), r(g.dlat.abs() * per_deg_lat)])
}

fn scheme_of(band: &str, jrc_version: Option<u32>) -> String {
    match band {
        JRC => match jrc_version {
            Some(v) => format!("jrc.gfc2020.v{v}"),
            None => "jrc.gfc2020".into(),
        },
        TMF => "jrc.tmf.v2025".into(),
        _ => "hansen.gfc.v1_13.2025".into(),
    }
}

/// A layer read for a plot, or why there is none.
struct ReadLayer {
    band: &'static str,
    window: Option<LayerWindow>,
    /// The files as a fact names them: the JRC dispatcher URL when TMF came
    /// from its cached whole tile, else the URL read.
    files: Vec<String>,
    error: Option<String>,
}

impl ReadLayer {
    fn new(band: &'static str, got: Result<LayerWindow, String>) -> Self {
        match got {
            Ok(w) => ReadLayer {
                band,
                files: w.files.clone(),
                window: Some(w),
                error: None,
            },
            Err(e) => ReadLayer {
                band,
                window: None,
                files: Vec::new(),
                error: Some(e),
            },
        }
    }
}

async fn read_planned(
    plan: Result<band_raster::LayerPlan, LayerError>,
) -> Result<LayerWindow, String> {
    match plan {
        Ok(p) => band_raster::read_layer(p).await,
        Err(LayerError::TooLarge { width, height }) => Err(format!(
            "the window is {width} x {height} pixels, over the cap"
        )),
        Err(LayerError::Failed(e)) => Err(e),
    }
}

/// TMF from the JRC dispatcher's whole tiles, cached on disk, when the
/// range-served mirror fails: the fallback the per-cell reader also has.
async fn read_tmf_from_dispatcher(b: BBox) -> Result<(LayerWindow, Vec<String>), String> {
    use emem_fetch::jrc_tmf;
    let cli = crate::s2_http_client();
    let mut tiles = Vec::new();
    let mut files = Vec::new();
    for (key, (lat, lng)) in band_raster::layer_squares(&b) {
        let (Some(lat_tag), Some(lng_tag)) =
            (jrc_tmf::tile_lat_tag(lat), jrc_tmf::tile_lng_tag(lng))
        else {
            return Err(format!("TMF publishes no tile at ({lat}, {lng})"));
        };
        let path = jrc_tmf::ensure_tile_cached(&cli, jrc_tmf::DATASET_DEFORESTATION_YEAR, lat, lng)
            .await
            .map_err(|e| format!("TMF dispatcher tile: {e}"))?;
        let path = std::fs::canonicalize(&path).map_err(|e| format!("TMF cached tile: {e}"))?;
        tiles.push(ForestTile {
            key,
            url: format!("file://{}", path.display()),
            jrc_version: None,
        });
        files.push(jrc_tmf::tile_url(
            jrc_tmf::DATASET_DEFORESTATION_YEAR,
            &lat_tag,
            &lng_tag,
        ));
    }
    let plan = band_raster::plan_layer_from(TMF, b, tiles, u64::MAX).await;
    let window = read_planned(plan).await?;
    Ok((window, files))
}

/// What the raster method made of one plot.
pub(crate) struct RasterPlot {
    pub(crate) classified: Classified,
    pub(crate) fail_fraction: f64,
    pub(crate) mmu: crate::MmuDemotion,
    /// Each chunk's pixels and its source windows, in chunk order.
    pub(crate) chunks: Vec<(Owns, Vec<LayerRecord>)>,
    /// The plot fact's cid: one this responder already signed with this
    /// content, or the one built here.
    pub(crate) fact_cid: Option<String>,
    /// The fact built here and not yet stored; `None` when one was reused.
    pub(crate) unsaved: Option<(String, Fact)>,
}

/// The raster method's answer for a plot, or why the sampled method runs.
pub(crate) enum Outcome {
    Done(Box<RasterPlot>),
    Sampled(String),
}

/// Degrees added round a bbox before the 30 m layers are read: an edge pixel
/// of the 10 m grid can have its centre outside the bbox, and its Hansen or
/// TMF pixel must still be in the window.
const MARGIN_DEG: f64 = 0.0005;

fn grown(b: &BBox, by: f64) -> BBox {
    BBox {
        min_lat: b.min_lat - by,
        max_lat: b.max_lat + by,
        min_lng: b.min_lng - by,
        max_lng: b.max_lng + by,
    }
}

/// Reads the four sources over `b` at once: JRC GFC2020 over `b` itself, and
/// Hansen and TMF over `b` grown by `MARGIN_DEG`. JRC is the grid the count
/// runs on, so a JRC failure is the read's; the others are recorded and the
/// count goes on without them.
async fn read_layers(b: BBox) -> Result<Vec<ReadLayer>, String> {
    let wide = grown(&b, MARGIN_DEG);
    let (jrc_plan, tc_plan, ly_plan, tmf_plan) = tokio::join!(
        band_raster::plan_layer(JRC, b.clone(), u64::MAX),
        band_raster::plan_layer(TREECOVER, wide.clone(), u64::MAX),
        band_raster::plan_layer(LOSSYEAR, wide.clone(), u64::MAX),
        band_raster::plan_layer(TMF, wide.clone(), u64::MAX),
    );
    let centre_lat = (b.min_lat + b.max_lat) / 2.0;
    let in_tmf_belt = emem_fetch::jrc_tmf::tile_lat_tag(centre_lat).is_some();
    let tmf_mirror_planned = tmf_plan.is_ok();
    let (jrc, tc, ly, tmf) = tokio::join!(
        read_planned(jrc_plan),
        read_planned(tc_plan),
        read_planned(ly_plan),
        read_planned(tmf_plan),
    );
    let jrc = jrc.map_err(|e| {
        format!("the JRC GFC2020 window the raster method counts on could not be read ({e})")
    })?;
    let mut tmf = ReadLayer::new(TMF, tmf);
    if tmf.window.is_none() && in_tmf_belt {
        let timeout = std::time::Duration::from_secs(crate::materializer_timeout_secs());
        match tokio::time::timeout(timeout, read_tmf_from_dispatcher(wide)).await {
            Ok(Ok((w, files))) => {
                tmf = ReadLayer {
                    band: TMF,
                    window: Some(w),
                    files,
                    error: None,
                }
            }
            Ok(Err(e)) => {
                tmf.error = Some(format!(
                    "{}; the JRC dispatcher fallback failed too: {e}",
                    tmf.error.unwrap_or_default()
                ))
            }
            Err(_) => {
                tmf.error = Some(format!(
                    "{}; the JRC dispatcher fallback did not finish in {} s",
                    tmf.error.unwrap_or_default(),
                    timeout.as_secs()
                ))
            }
        }
    } else if tmf.window.is_none() && !tmf_mirror_planned {
        tmf.error = Some(format!(
            "TMF covers the tropical belt (30° S to 30° N) only: {}",
            tmf.error.unwrap_or_default()
        ));
    }
    Ok(vec![
        ReadLayer {
            band: JRC,
            files: jrc.files.clone(),
            window: Some(jrc),
            error: None,
        },
        ReadLayer::new(TREECOVER, tc),
        ReadLayer::new(LOSSYEAR, ly),
        tmf,
    ])
}

/// The part of `w` over `b`: every pixel the bbox touches, as the read of
/// `b` alone would have fitted it.
fn crop(w: &LayerWindow, b: &BBox) -> LayerWindow {
    const EPS: f64 = 1e-6;
    let h = &w.header;
    let left = h.lng0 - h.dlng * 0.5;
    let top = h.lat0 - h.dlat * 0.5;
    let (wd, ht) = (i64::from(h.width), i64::from(h.height));
    let c0 = (((b.min_lng - left) / h.dlng + EPS).floor() as i64).clamp(0, wd - 1);
    let c1 = (((b.max_lng - left) / h.dlng - EPS).ceil() as i64).clamp(c0 + 1, wd);
    let r0 = (((b.max_lat - top) / h.dlat + EPS).floor() as i64).clamp(0, ht - 1);
    let r1 = (((b.min_lat - top) / h.dlat - EPS).ceil() as i64).clamp(r0 + 1, ht);
    let (cw, ch) = ((c1 - c0) as usize, (r1 - r0) as usize);
    let mut values = Vec::with_capacity(cw * ch);
    for r in r0..r1 {
        let at = r as usize * h.width as usize + c0 as usize;
        values.extend_from_slice(&w.values[at..at + cw]);
    }
    LayerWindow {
        header: GridHeader {
            width: cw as u32,
            height: ch as u32,
            lat0: h.lat0 + r0 as f64 * h.dlat,
            lng0: h.lng0 + c0 as f64 * h.dlng,
            ..*h
        },
        values,
        files: w.files.clone(),
        jrc_version: w.jrc_version,
    }
}

/// A chunk's counts and its windows as the fact names them, or why not.
type ChunkCount = Result<(Classified, Vec<LayerRecord>), String>;

/// Counts the pixels of one plot (or one chunk of it) that `owns` holds in
/// `layers`, and stores each window as an artifact.
async fn count_windows(
    layers: Vec<ReadLayer>,
    rings: Vec<Vec<(f64, f64)>>,
    owns: Owns,
    cutoff_year: i64,
    rule: ForestRule,
) -> ChunkCount {
    let counted = tokio::task::spawn_blocking(move || {
        let w = |i: usize| layers[i].window.as_ref();
        let Some(jrc) = w(0) else {
            return Err("no JRC window".to_string());
        };
        let jrc_version = jrc.jrc_version;
        let classified = classify(jrc, w(1), w(2), w(3), &rings, &owns, cutoff_year, rule);
        let mut records = Vec::with_capacity(layers.len());
        for l in &layers {
            let mut rec = LayerRecord {
                band: l.band,
                scheme: scheme_of(l.band, if l.band == JRC { jrc_version } else { None }),
                files: l.files.clone(),
                artifact_cid: None,
                byte_len: 0,
                grid: l.window.as_ref().map(|w| w.header),
                error: l.error.clone(),
            };
            if let Some(w) = &l.window {
                match encode_grid(&w.header, &w.values) {
                    Ok(bytes) => match band_raster::ARTIFACTS.put(&bytes) {
                        Ok(cid) => {
                            rec.artifact_cid = Some(cid);
                            rec.byte_len = bytes.len();
                        }
                        Err(e) => rec.error = Some(format!("artifact store: {e}")),
                    },
                    Err(e) => rec.error = Some(format!("grid encode: {e:?}")),
                }
            }
            records.push(rec);
        }
        Ok((classified, records))
    })
    .await
    .map_err(|e| format!("the pixel count stopped: {e}"))??;
    // A window the store could not keep is not evidence anyone can fetch.
    if let Some(l) = counted
        .1
        .iter()
        .find(|l| l.grid.is_some() && l.artifact_cid.is_none())
    {
        return Err(format!(
            "the {} window could not be stored ({})",
            l.band,
            l.error.clone().unwrap_or_default()
        ));
    }
    Ok(counted)
}

/// One plot of a request, ready for the raster method.
pub(crate) struct PlotInput {
    /// The plot's index in the request.
    pub(crate) idx: usize,
    /// The cell the plot fact is signed at: the centre of the plot's bbox.
    pub(crate) cell: String,
    pub(crate) bbox: BBox,
    /// Outer ring first, then holes, each `(lng, lat)`.
    pub(crate) rings: Vec<Vec<(f64, f64)>>,
    /// `(plot_commitment, geometry_commitment)`.
    pub(crate) commitments: (String, String),
}

/// Plots whose windows are read once for all of them.
struct Cluster {
    bbox: BBox,
    plots: Vec<PlotInput>,
}

fn union(a: &BBox, b: &BBox) -> BBox {
    BBox {
        min_lat: a.min_lat.min(b.min_lat),
        max_lat: a.max_lat.max(b.max_lat),
        min_lng: a.min_lng.min(b.min_lng),
        max_lng: a.max_lng.max(b.max_lng),
    }
}

/// Plots of one request evaluated read by read, not plot by plot.
///
/// Plots that fit in one chunk are grouped while the group's bbox stays
/// within one chunk of JRC pixels: each group's four sources are read once
/// as one window each, and every plot counts its pixels in its own part of
/// those windows. Fifty farms of one cooperative are then four reads, not
/// two hundred. A plot larger than one chunk is read chunk by chunk. Every
/// plot still gets its own windows (cropped from the group's) and its own
/// fact, so its evidence names nothing about another plot.
pub(crate) async fn evaluate_consignment(
    s: AppState,
    plots: Vec<PlotInput>,
    cutoff_year: i64,
    rule: ForestRule,
) -> Vec<(usize, Outcome)> {
    use futures_util::StreamExt;
    // Each plot's JRC window, planned and not read: its size and the pixel
    // size decide the grouping.
    // The other three layers' file headers are opened at the same time, so
    // the reads below find them in memory rather than waiting a round trip.
    let first = plots.first().map(|p| grown(&p.bbox, MARGIN_DEG));
    let warm = |band: &'static str| {
        let b = first.clone();
        async move {
            if let Some(b) = b {
                let _ = band_raster::plan_layer(band, b, u64::MAX).await;
            }
        }
    };
    let (planned, _, _, _) = tokio::join!(
        futures_util::future::join_all(plots.iter().map(|p| band_raster::plan_layer(
            JRC,
            p.bbox.clone(),
            u64::MAX
        )),),
        warm(TREECOVER),
        warm(LOSSYEAR),
        warm(TMF),
    );
    let mut out: Vec<(usize, Outcome)> = Vec::new();
    let mut small: Vec<(PlotInput, (f64, f64))> = Vec::new();
    let mut large: Vec<(PlotInput, i64, i64)> = Vec::new();
    for (p, plan) in plots.into_iter().zip(planned) {
        match plan {
            Ok(plan) => {
                let (w, h) = (i64::from(plan.width), i64::from(plan.height));
                if w <= CHUNK_PX && h <= CHUNK_PX {
                    small.push((p, plan.pixel_deg()));
                } else {
                    large.push((p, w, h));
                }
            }
            Err(LayerError::TooLarge { .. }) => unreachable!("no cap was set"),
            Err(LayerError::Failed(e)) => out.push((
                p.idx,
                Outcome::Sampled(format!(
                    "the JRC GFC2020 grid the raster method counts on could not be opened ({e})"
                )),
            )),
        }
    }
    // West to east, then group while the group stays one chunk wide and high.
    small.sort_by(|a, b| a.0.bbox.min_lng.total_cmp(&b.0.bbox.min_lng));
    let mut clusters: Vec<Cluster> = Vec::new();
    for (p, (dx, dy)) in small {
        let fits = |c: &Cluster| {
            let u = union(&c.bbox, &p.bbox);
            (u.max_lng - u.min_lng) / dx <= (CHUNK_PX - 2) as f64
                && (u.max_lat - u.min_lat) / dy <= (CHUNK_PX - 2) as f64
        };
        match clusters.iter_mut().find(|c| fits(c)) {
            Some(c) => {
                c.bbox = union(&c.bbox, &p.bbox);
                c.plots.push(p);
            }
            None => clusters.push(Cluster {
                bbox: p.bbox.clone(),
                plots: vec![p],
            }),
        }
    }
    let grouped: Vec<Vec<(usize, Outcome)>> =
        futures_util::stream::iter(clusters.into_iter().map(|c| {
            let s = s.clone();
            async move { evaluate_cluster(s, c, cutoff_year, rule).await }
        }))
        .buffer_unordered(CLUSTERS_AT_ONCE)
        .collect()
        .await;
    out.extend(grouped.into_iter().flatten());
    let big: Vec<(usize, Outcome)> =
        futures_util::stream::iter(large.into_iter().map(|(p, w, h)| {
            let s = s.clone();
            async move { (p.idx, evaluate_large(s, p, w, h, cutoff_year, rule).await) }
        }))
        .buffer_unordered(CHUNKS_AT_ONCE)
        .collect()
        .await;
    out.extend(big);
    out
}

/// Groups read at once.
const CLUSTERS_AT_ONCE: usize = 4;

/// One group: its windows read once, then each plot counted in its part.
async fn evaluate_cluster(
    s: AppState,
    cluster: Cluster,
    cutoff_year: i64,
    rule: ForestRule,
) -> Vec<(usize, Outcome)> {
    let shared = match read_layers(cluster.bbox.clone()).await {
        Ok(l) => l,
        Err(e) => {
            return cluster
                .plots
                .into_iter()
                .map(|p| (p.idx, Outcome::Sampled(e.clone())))
                .collect()
        }
    };
    let counts = cluster.plots.into_iter().map(|p| {
        let layers: Vec<ReadLayer> = shared
            .iter()
            .map(|l| {
                let b = if l.band == JRC {
                    p.bbox.clone()
                } else {
                    grown(&p.bbox, MARGIN_DEG)
                };
                ReadLayer {
                    band: l.band,
                    window: l.window.as_ref().map(|w| crop(w, &b)),
                    files: l.files.clone(),
                    error: l.error.clone(),
                }
            })
            .collect();
        let s = s.clone();
        async move {
            let idx = p.idx;
            let got =
                count_windows(layers, p.rings.clone(), Owns::default(), cutoff_year, rule).await;
            let outcome = match got {
                Ok((c, records)) => {
                    finish_plot(
                        &s,
                        p,
                        c,
                        vec![(Owns::default(), records)],
                        cutoff_year,
                        rule,
                    )
                    .await
                }
                Err(e) => Outcome::Sampled(e),
            };
            (idx, outcome)
        }
    });
    futures_util::future::join_all(counts).await
}

/// A plot larger than one chunk, counted chunk by chunk.
async fn evaluate_large(
    s: AppState,
    p: PlotInput,
    width: i64,
    height: i64,
    cutoff_year: i64,
    rule: ForestRule,
) -> Outcome {
    use futures_util::StreamExt;
    let chunks = chunks_of(&p.bbox, width, height);
    let mut counted: Vec<(usize, Owns, ChunkCount)> =
        futures_util::stream::iter(chunks.into_iter().enumerate().map(|(i, chunk)| {
            let rings = p.rings.clone();
            async move {
                let got = match read_layers(chunk.bbox).await {
                    Ok(layers) => count_windows(layers, rings, chunk.owns, cutoff_year, rule).await,
                    Err(e) => Err(e),
                };
                (i, chunk.owns, got)
            }
        }))
        .buffer_unordered(CHUNKS_AT_ONCE)
        .collect()
        .await;
    counted.sort_by_key(|(i, _, _)| *i);
    let mut classified = Classified::default();
    let mut records = Vec::with_capacity(counted.len());
    for (_, owns, got) in counted {
        match got {
            Ok((c, r)) => {
                classified.absorb(c);
                records.push((owns, r));
            }
            Err(e) => return Outcome::Sampled(e),
        }
    }
    finish_plot(&s, p, classified, records, cutoff_year, rule).await
}

/// The verdict, the fact value and the plot fact, reused when this
/// responder already signed one with the same content.
async fn finish_plot(
    s: &AppState,
    p: PlotInput,
    classified: Classified,
    chunks: Vec<(Owns, Vec<LayerRecord>)>,
    cutoff_year: i64,
    rule: ForestRule,
) -> Outcome {
    if classified.plot.pixels == 0 {
        return Outcome::Sampled(
            "the polygon covers no 10 m pixel, so there is nothing to count".into(),
        );
    }
    let (commit_plot, commit_geometry) = p.commitments;
    let (_, fail_fraction, mmu) = plot_verdict(&classified);
    let value = fact_value(
        &classified,
        &chunks,
        &mmu,
        fail_fraction,
        cutoff_year,
        rule,
        &commit_plot,
        &commit_geometry,
    );
    // An identical plot result this responder already signed is cited, not
    // signed again: same polygon, plot commitment, windows, rule and counts.
    let cell = p.cell;
    let key = content_key(&cell, &value);
    let (fact_cid, unsaved) = match stored_plot_fact(s, &key, &cell, &value).await {
        Some(cid) => (Some(cid), None),
        None => match crate::eudr_prepare_unsaved(vec![plot_fact(s, &cell, &value)]) {
            Ok(v) => match v.into_iter().next() {
                Some((cid, f)) => {
                    let cid = cid.as_str().to_string();
                    remember_plot_fact(&key, &cid).await;
                    (Some(cid.clone()), Some((cid, f)))
                }
                None => (None, None),
            },
            Err(e) => {
                tracing::warn!(error = %e, "eudr plot fact not built (verdict unaffected)");
                (None, None)
            }
        },
    };
    Outcome::Done(Box::new(RasterPlot {
        classified,
        fail_fraction,
        mmu,
        chunks,
        fact_cid,
        unsaved,
    }))
}

/// Where a plot fact is found again by its content: one small file a key,
/// holding the fact's cid.
fn reuse_dir() -> std::path::PathBuf {
    emem_core::data_dir::data_dir().join("eudr_plot_facts")
}

/// The content key of a plot fact: its cell and its whole value, which pins
/// the polygon and plot commitments, the source files and windows, the rule
/// and every count. The same key is the same fact.
pub(crate) fn content_key(cell: &str, value: &JsonValue) -> String {
    let mut h = blake3::Hasher::new();
    h.update(b"emem.eudr.plot_fact_key.v1\0");
    h.update(cell.as_bytes());
    h.update(&[0]);
    h.update(&serde_json::to_vec(value).unwrap_or_default());
    h.finalize().to_hex().to_string()
}

/// The stored plot fact with this content, if this responder signed one:
/// a repeat of an identical plot result is not signed again.
pub(crate) async fn stored_plot_fact(
    s: &AppState,
    key: &str,
    cell: &str,
    value: &JsonValue,
) -> Option<String> {
    let cid = tokio::fs::read_to_string(reuse_dir().join(key))
        .await
        .ok()?;
    let cid = cid.trim().to_string();
    let got = s
        .storage
        .get_facts_many(&[emem_fact::FactCid::new(cid.as_str())])
        .await
        .ok()?
        .into_iter()
        .next()
        .flatten()?;
    let want = crate::change_attribution::json_to_cbor(value);
    match got {
        Fact::Derivative(d)
            if d.cell == cell
                && d.signer.0 == s.identity.pubkey.0
                && d.derivation.fn_key == FN_KEY
                && d.value == want =>
        {
            Some(cid)
        }
        _ => None,
    }
}

/// Notes the cid of a plot fact under its content key. Best-effort: a miss
/// only means the next identical request signs a fact again.
pub(crate) async fn remember_plot_fact(key: &str, cid: &str) {
    let dir = reuse_dir();
    if tokio::fs::create_dir_all(&dir).await.is_err() {
        return;
    }
    let tmp = dir.join(format!(".{key}.{}", std::process::id()));
    if tokio::fs::write(&tmp, cid.as_bytes()).await.is_ok() {
        let _ = tokio::fs::rename(&tmp, dir.join(key)).await;
    }
}

fn seen_json(by: &[Tally; 3]) -> JsonValue {
    json!({
        "hansen_only": by[0].json(),
        "tmf_only": by[1].json(),
        "both": by[2].json(),
    })
}

fn years_json(years: &BTreeMap<i64, Tally>) -> JsonValue {
    JsonValue::Array(
        years
            .iter()
            .map(|(y, t)| json!([y, t.pixels, t.ha()]))
            .collect(),
    )
}

/// The value the plot fact signs. Every number in it can be recomputed from
/// the windows it names and the polygon its commitment binds.
#[allow(clippy::too_many_arguments)]
fn fact_value(
    c: &Classified,
    chunks: &[(Owns, Vec<LayerRecord>)],
    mmu: &crate::MmuDemotion,
    fail_fraction: f64,
    cutoff_year: i64,
    rule: ForestRule,
    commit_plot: &str,
    commit_geometry: &str,
) -> JsonValue {
    let chunks: Vec<JsonValue> = chunks
        .iter()
        .map(|(owns, layers)| {
            let sources: serde_json::Map<String, JsonValue> = layers
                .iter()
                .map(|l| (l.band.to_string(), l.json()))
                .collect();
            json!({"owns": owns.json(), "sources": sources})
        })
        .collect();
    json!({
        "schema": "emem.eudr_plot_raster.v1",
        "rule": "eudr_compliance@1",
        "forest_rule": rule.label(),
        "cutoff_year": cutoff_year,
        "area_method": "coverage_fraction_geodesic",
        "plot_commitment": commit_plot,
        "geometry_commitment": commit_geometry,
        "method": {
            "grid": JRC,
            "pixel_in_plot": "a pixel counts with the fraction of it inside the polygon (outer ring less holes), computed exactly in degree space; a pixel covered by 1e-9 or less is left out",
            "area_method": "coverage_fraction_geodesic",
            "other_layers": "read at the pixel of their own grid that contains the 10 m pixel centre; no resampling",
            "area": "WGS84 ellipsoid area of the pixel's latitude band",
            "loss": "loss after the cut-off is a Hansen loss year or a JRC TMF deforestation year after the cut-off year; RADD is not read",
            "loss_boundary_m": 30,
            "chunks": "a plot wider or taller than 2,048 JRC pixels is counted in chunks; each chunk counts the pixels its owns rule holds, and the totals add the chunks in order",
            "resolution": "every source at full resolution (the TIFF's first image, never an overview)",
        },
        "chunks": chunks,
        "pixels": {
            "in_plot": c.plot.pixels,
            "pass": c.pass.pixels,
            "fail": c.fail.pixels,
            "review": c.review.pixels,
            "not_in_scope": c.not_in_scope.pixels,
            "indeterminate": c.indeterminate.pixels,
            "forest_at_cutoff": c.forest_at_cutoff.pixels,
            "with_both_baselines": c.both_baselines,
            "borderline_canopy": c.borderline,
            "jrc_read": c.jrc_read,
            "hansen_treecover_read": c.hansen_read,
        },
        "area_ha": {
            "in_plot": c.plot.ha(),
            "pass": c.pass.ha(),
            "fail": c.fail.ha(),
            "review": c.review.ha(),
            "not_in_scope": c.not_in_scope.ha(),
            "indeterminate": c.indeterminate.ha(),
            "forest_at_cutoff": c.forest_at_cutoff.ha(),
            "hansen_tree_cover_jrc_non_forest": c.hansen_tree_cover_jrc_non_forest.ha(),
            "jrc_unavailable_hansen_only": c.jrc_unavailable_hansen_only.ha(),
        },
        "loss_seen_by": {
            "fail": seen_json(&c.fail_seen_by),
            "review": seen_json(&c.review_seen_by),
        },
        "failing_by_baseline": {
            "jrc_forest": c.fail_jrc_forest.json(),
            "hansen_only_jrc_non_forest": c.fail_hansen_jrc_non_forest.json(),
            "hansen_only_jrc_missing": c.fail_hansen_jrc_missing.json(),
        },
        "hansen_loss": {
            "by_year": years_json(&c.hansen_years),
            "after_cutoff": c.hansen_after.json(),
            "no_loss_pixels": c.hansen_no_loss,
            "unknown_pixels": c.hansen_unknown,
        },
        "tmf_loss": {
            "by_year": years_json(&c.tmf_years),
            "after_cutoff": c.tmf_after.json(),
            "no_loss_pixels": c.tmf_no_loss,
            "unknown_pixels": c.tmf_unknown,
        },
        "tmf_cross_check": {
            "pixels_read": c.tmf.read,
            "both": c.tmf.both,
            "hansen_only": c.tmf.hansen_only,
            "tmf_only": c.tmf.tmf_only,
            "neither": c.tmf.neither,
            "tmf_only_area_ha": c.tmf_only.ha(),
        },
        "verdict": {
            "label": crate::verdict_label(mmu.verdict_code),
            "code": mmu.verdict_code,
            "fail_fraction": fail_fraction,
            "failing_area_ha": ha(c.fail.area_m2),
            "review_area_ha": ha(c.review.area_m2),
            "mmu_threshold_ha": crate::EUDR_MMU_THRESHOLD_HA,
            "mmu_floor_applied": mmu.mmu_floor_applied,
        },
    })
}

/// The plot fact: one signed derivative that holds the whole plot's answer.
pub(crate) fn plot_fact(s: &AppState, cell: &str, value: &JsonValue) -> Fact {
    Fact::Derivative(DerivativeFact {
        cell: cell.to_string(),
        band: "eudr.plot_raster".into(),
        tslot_window: [
            crate::tslot_for_year(2001, LOSSYEAR),
            crate::tslot_for_year(band_raster::FOREST_RELEASE_YEAR, LOSSYEAR),
        ],
        op: "eudr_plot_raster".into(),
        parents: Vec::new(),
        value: crate::change_attribution::json_to_cbor(value),
        confidence: 1.0,
        derivation: Derivation {
            fn_key: FN_KEY.into(),
            args: None,
        },
        schema_cid: s.manifests.schema_cid.clone(),
        signer: s.identity.pubkey,
        signed_at: emem_storage::server::iso8601_now(),
    })
}

fn row_json(p: &PixelRow) -> JsonValue {
    json!({
        "cell": emem_codec::cell64_from_latlng(p.lat, p.lng),
        "lat": (p.lat * 1e7).round() / 1e7,
        "lng": (p.lng * 1e7).round() / 1e7,
        "verdict": p.verdict,
        "label": crate::verdict_label(p.verdict),
        "jrc_forest_2020": p.jrc,
        "hansen_treecover_2000": p.treecover,
        "hansen_lossyear": p.lossyear,
        "jrc_tmf_deforestation_year": p.tmf,
    })
}

/// The per-plot answer fields the raster method fills: the same names the
/// sampled method uses, with a cell read as a 10 m pixel, plus the areas.
pub(crate) fn plot_fields(
    rp: &RasterPlot,
    cutoff_year: i64,
    fact_cid: Option<&str>,
    centroid_cell: &str,
) -> serde_json::Map<String, JsonValue> {
    let c = &rp.classified;
    let code = rp.mmu.verdict_code;
    let n = c.plot.pixels;
    let tmf_review: Vec<String> = c
        .tmf_review
        .iter()
        .map(|(lat, lng)| emem_codec::cell64_from_latlng(*lat, *lng))
        .collect();
    let mut tmf = crate::tmf_cross_check_json(&c.tmf, tmf_review);
    if let Some(o) = tmf.as_object_mut() {
        o.insert("in_verdict".into(), json!(true));
        o.insert(
            "in_verdict_note".into(),
            json!("On the raster method TMF votes: a TMF deforestation year after the cut-off is loss, as a Hansen loss year is. The agreement figures still compare the two."),
        );
        o.insert("unit".into(), json!("10 m pixels"));
        o.insert("tmf_only_area_ha".into(), json!(c.tmf_only.ha()));
        o.insert(
            "review_cells_capped".into(),
            json!(c.tmf.tmf_only as usize > REVIEW_PIXELS_MAX),
        );
    }
    let by_year: Vec<JsonValue> = c
        .hansen_years
        .iter()
        .map(|(y, t)| json!({"year": y, "cells": t.pixels, "area_ha": t.ha()}))
        .collect();
    let tmf_by_year: Vec<JsonValue> = c
        .tmf_years
        .iter()
        .map(|(y, t)| json!({"year": y, "cells": t.pixels, "area_ha": t.ha()}))
        .collect();
    let mean_px_ha = if n > 0 {
        c.plot.area_m2 / n as f64 / 1e4
    } else {
        0.0
    };
    let mut out = serde_json::Map::new();
    let mut put = |k: &str, v: JsonValue| {
        out.insert(k.to_string(), v);
    };
    put("verdict", json!(crate::verdict_label(code)));
    put("area_method", json!("coverage_fraction_geodesic"));
    put("verdict_code", json!(code));
    put("fail_fraction", json!(rp.fail_fraction));
    put("failing_cells", json!(c.fail.pixels));
    put("failing_cells_basis", json!(crate::FAILING_CELLS_BASIS));
    put(
        "failing_cells_by_baseline",
        json!({
            "jrc_forest": c.fail_jrc_forest.pixels,
            "hansen_only_jrc_non_forest": c.fail_hansen_jrc_non_forest.pixels,
            "hansen_only_jrc_missing": c.fail_hansen_jrc_missing.pixels,
        }),
    );
    put(
        "failing_area_ha_by_baseline",
        json!({
            "jrc_forest": c.fail_jrc_forest.ha(),
            "hansen_only_jrc_non_forest": c.fail_hansen_jrc_non_forest.ha(),
            "hansen_only_jrc_missing": c.fail_hansen_jrc_missing.ha(),
        }),
    );
    put(
        "in_scope_failing_cells",
        json!(if code == 2 { c.fail.pixels } else { 0 }),
    );
    put("failing_area_ha", json!(ha(c.fail.area_m2)));
    put(
        "failing_area_projection",
        json!("none: the sum of the true areas of the failing 10 m pixels"),
    );
    put("review_cells", json!(c.review.pixels));
    put("review_area_ha", json!(c.review.ha()));
    put(
        "review_reason",
        json!((c.review.pixels > 0).then_some(crate::EUDR_REVIEW_REASON)),
    );
    put(
        "hansen_tree_cover_on_jrc_non_forest",
        json!({
            "cells": c.hansen_tree_cover_jrc_non_forest.pixels,
            "area_ha": c.hansen_tree_cover_jrc_non_forest.ha(),
            "note": "Pixels the JRC map calls non-forest where Hansen sees 10 % or more tree cover in 2000 and no loss. They pass: the maps disagree, often over tree crops, which are not forest under Article 2(4).",
        }),
    );
    put("forest_at_cutoff_area_ha", json!(c.forest_at_cutoff.ha()));
    put(
        "loss_seen_by",
        json!({"fail": seen_json(&c.fail_seen_by), "review": seen_json(&c.review_seen_by)}),
    );
    put("loss_boundary_m", json!(30));
    put(
        "loss_boundary_note",
        json!("Hansen GFC and JRC TMF map loss on a 30 m grid. A loss area here is the 10 m JRC pixels, by the fraction of each inside the polygon, that fall in a 30 m loss pixel, so failing_area_ha and review_area_ha carry the 30 m boundary of the loss maps."),
    );
    put(
        "loss_after_cutoff_area_ha",
        json!({"hansen": c.hansen_after.ha(), "tmf": c.tmf_after.ha()}),
    );
    put(
        "sampled_area_ha",
        json!((c.plot.area_m2 / 1e4 * 100.0).round() / 100.0),
    );
    put("sampled_polygon_fraction", json!(1.0));
    put(
        "sampling_note",
        json!("Raster method: every 10 m JRC GFC2020 pixel whose centre is inside the polygon was read, so nothing is projected. total_cells and the cell counts are those pixels."),
    );
    put("mmu_threshold_ha", json!(crate::EUDR_MMU_THRESHOLD_HA));
    put("mmu_floor_applied", json!(rp.mmu.mmu_floor_applied));
    put("total_cells", json!(n));
    put("per_cell_verdicts", json!([]));
    put("per_cell_verdicts_total", json!(0));
    put("per_cell_verdicts_truncated", json!(false));
    put("per_cell_detail", json!("raster"));
    put("per_cell_offset", JsonValue::Null);
    put("per_cell_next_offset", JsonValue::Null);
    put(
        "per_cell_verdicts_means",
        json!("The raster method signs no per-cell facts, so there are no rows. The plot fact (plot_fact_cid) holds every count and area, and names the source windows it was computed from; example_pixels lists some failing and review pixels."),
    );
    put("per_cell_rows_cid", JsonValue::Null);
    put(
        "example_pixels",
        JsonValue::Array(c.examples.iter().map(row_json).collect()),
    );
    put(
        "loss_year_histogram",
        json!({
            "by_year": by_year,
            "after_cutoff_cells": c.hansen_after.pixels,
            "after_cutoff_area_ha": c.hansen_after.ha(),
            "after_cutoff_year": cutoff_year,
            "no_loss_cells": c.hansen_no_loss,
            "unknown_cells": c.hansen_unknown,
            "total_sampled_cells": n,
            "after_cutoff_area_ha_sampled": c.hansen_after.ha(),
            "cell_area_ha": (mean_px_ha * 1e6).round() / 1e6,
            "basis": "raster_pixels",
            "tmf": {
                "by_year": tmf_by_year,
                "after_cutoff_cells": c.tmf_after.pixels,
                "after_cutoff_area_ha": c.tmf_after.ha(),
                "no_loss_cells": c.tmf_no_loss,
                "unknown_cells": c.tmf_unknown,
            },
            "note": "Counts are 10 m pixels inside the polygon (the raster method), each Hansen GFC v1.13 and JRC TMF v1.2025 year read at the 30 m pixel that contains the 10 m pixel centre; areas are true pixel areas. The tally is part of the plot fact (signed_fact_cid).",
            "signed_fact_cid": fact_cid,
            "parents_capped": false,
        }),
    );
    put("tmf_cross_check", tmf);
    let chunks: Vec<JsonValue> = rp
        .chunks
        .iter()
        .map(|(owns, layers)| {
            let sources: serde_json::Map<String, JsonValue> = layers
                .iter()
                .map(|l| {
                    (
                        l.band.to_string(),
                        json!({
                            "artifact_cid": l.artifact_cid,
                            "url": l.artifact_cid.as_ref().map(|c| format!("/v1/artifacts/{c}")),
                            "byte_len": l.byte_len,
                            "files": l.files,
                            "native_m": l.grid.map(|g| native_m(&g)),
                            "error": l.error,
                        }),
                    )
                })
                .collect();
            json!({"owns": owns.json(), "sources": sources})
        })
        .collect();
    put(
        "raster",
        json!({
            "grid": "JRC GFC2020 10 m",
            "resolution": "every source at full resolution (the TIFF's first image, never an overview): JRC GFC2020 about 10 m, Hansen GFC and JRC TMF about 30 m",
            "pixel_side_m": (c.pixel_side_m * 100.0).round() / 100.0,
            "chunks": chunks,
            "verify": "python3 tools/verifiers/emem-eudr-due-diligence/verify_plot.py <plot_fact_token> --geometry <your plot GeoJSON>",
        }),
    );
    put("plot_fact_cid", json!(fact_cid));
    put(
        "plot_fact_token",
        json!(fact_cid.map(|c| format!("emem:fact:{centroid_cell}:{c}"))),
    );
    out
}

/// The counts the support grade reads, from the pixels.
pub(crate) fn support_inputs(rp: &RasterPlot, centre_lat: f64) -> crate::SupportInputs {
    let c = &rp.classified;
    crate::SupportInputs {
        units: c.plot.pixels as usize,
        both: c.both_baselines as usize,
        borderline: c.borderline as usize,
        fails: c.fail.pixels as usize,
        fails_jrc_non_forest: c.fail_hansen_jrc_non_forest.pixels as usize,
        spacing_m: c.pixel_side_m,
        in_tmf_belt: centre_lat.abs() < 30.0,
        noun: "pixels",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(h: GridHeader, values: Vec<f32>) -> LayerWindow {
        LayerWindow {
            header: h,
            values,
            files: vec!["file:///x.tif".into()],
            jrc_version: Some(4),
        }
    }

    fn header(w: u32, h: u32, lat0: f64, lng0: f64, d: f64) -> GridHeader {
        GridHeader {
            width: w,
            height: h,
            epsg: 4326,
            nodata: f32::NAN,
            lat0,
            lng0,
            dlat: -d,
            dlng: d,
            channels: 1,
        }
    }

    /// Coverage fractions add up to the polygon's area, a pixel wholly
    /// inside counts 1, one outside or in the hole counts 0, and the result
    /// does not depend on the ring's orientation.
    #[test]
    fn coverage_fractions_add_up_to_the_polygon() {
        // A concave ring with a hole, on a grid of unit pixels.
        let outer = vec![
            (0.3, 0.2),
            (9.7, 0.4),
            (9.9, 9.6),
            (5.1, 4.3),
            (0.1, 9.8),
            (0.3, 0.2),
        ];
        let hole = vec![(6.2, 1.1), (8.1, 1.3), (7.9, 3.2), (6.3, 3.0), (6.2, 1.1)];
        let shoelace = |r: &[(f64, f64)]| {
            let mut a = 0.0;
            for i in 0..r.len() {
                let (x0, y0) = r[i];
                let (x1, y1) = r[(i + 1) % r.len()];
                a += x0 * y1 - x1 * y0;
            }
            (a / 2.0).abs()
        };
        let want = shoelace(&outer) - shoelace(&hole);
        for flip in [false, true] {
            let mut o = outer.clone();
            if flip {
                o.reverse();
            }
            let uv = [o, hole.clone()];
            let signs: Vec<f64> = uv.iter().map(|r| ring_sign(r)).collect();
            let mut total = 0.0;
            let mut cover_at = std::collections::HashMap::new();
            for row in 0..10 {
                let (v0, v1) = (row as f64, row as f64 + 1.0);
                let segs: Vec<Vec<Seg>> = uv.iter().map(|r| row_segments(r, v0, v1)).collect();
                for col in 0..10 {
                    let x = col as f64;
                    let mut c = signs[0] * (left_area(&segs[0], x + 1.0) - left_area(&segs[0], x));
                    c -= signs[1] * (left_area(&segs[1], x + 1.0) - left_area(&segs[1], x));
                    assert!((-1e-12..=1.0 + 1e-12).contains(&c), "({row}, {col}) {c}");
                    total += c;
                    cover_at.insert((row, col), c);
                }
            }
            assert!((total - want).abs() < 1e-9, "{total} against {want}");
            assert!((cover_at[&(5, 1)] - 1.0).abs() < 1e-12, "inside");
            assert!(cover_at[&(2, 6)] < 1.0, "the hole takes part of it");
            assert!(cover_at[&(8, 5)].abs() < 1e-12, "in the notch");
        }
    }

    #[test]
    fn a_pixel_at_the_equator_is_about_a_hundred_square_metres() {
        let d = 10.0 / 111_320.0;
        let a = band_area_m2(-d / 2.0, d / 2.0, d);
        assert!((a - 100.0).abs() < 1.0, "{a}");
        // The whole ellipsoid is 510,065,622 km².
        let all = band_area_m2(-90.0, 90.0, 360.0) / 1e6;
        assert!((all - 510_065_621.7).abs() < 1.0, "{all}");
    }

    #[test]
    fn a_coarse_layer_is_read_at_the_pixel_that_contains_the_centre() {
        let coarse = header(2, 2, 0.75, 0.25, 0.5);
        assert_eq!(index_in(&coarse, 0.9, 0.1), Some(0));
        assert_eq!(index_in(&coarse, 0.9, 0.6), Some(1));
        assert_eq!(index_in(&coarse, 0.1, 0.6), Some(3));
        assert_eq!(index_in(&coarse, 1.1, 0.6), None);
        assert_eq!(index_in(&coarse, 0.5, -0.01), None);
    }

    /// Four JRC pixels in a 2 x 2 polygon; one Hansen pixel over all four.
    fn plot(jrc: [f32; 4], tc: f32, ly: f32, rule: ForestRule) -> (Classified, u8) {
        let d = 10.0 / 111_320.0;
        let j = window(header(2, 2, 1.5 * d, 0.5 * d, d), jrc.to_vec());
        let tcw = window(header(1, 1, d, d, 2.0 * d), vec![tc]);
        let lyw = window(header(1, 1, d, d, 2.0 * d), vec![ly]);
        let ring = vec![
            (0.0, 0.0),
            (2.0 * d, 0.0),
            (2.0 * d, 2.0 * d),
            (0.0, 2.0 * d),
            (0.0, 0.0),
        ];
        let c = classify(
            &j,
            Some(&tcw),
            Some(&lyw),
            None,
            &[ring],
            &Owns::default(),
            2020,
            rule,
        );
        let (_, _, mmu) = plot_verdict(&c);
        (c, mmu.verdict_code)
    }

    #[test]
    fn the_raster_counts_every_pixel_under_each_rule() {
        // Cocoa: JRC non-forest, Hansen tree cover, loss in 2024.
        let (c, _) = plot([0.0; 4], 60.0, 2024.0, ForestRule::JrcOnly);
        assert_eq!(c.plot.pixels, 4);
        assert_eq!(c.review.pixels, 4, "loss on JRC non-forest is review");
        assert_eq!(c.fail.pixels, 0);
        let (c, _) = plot([0.0; 4], 60.0, 2024.0, ForestRule::JrcOrHansen);
        assert_eq!(c.fail.pixels, 4, "the union fails it");
        assert_eq!(c.fail_hansen_jrc_non_forest.pixels, 4);
        // The same land with no loss passes, and the disagreement is counted.
        let (c, code) = plot([0.0; 4], 60.0, 0.0, ForestRule::JrcOnly);
        assert_eq!(code, 1);
        assert_eq!(c.hansen_tree_cover_jrc_non_forest.pixels, 4);
        assert_eq!(
            c.forest_at_cutoff.pixels, 0,
            "a pass on JRC non-forest is not forest"
        );
        // JRC forest with loss fails under both rules.
        for rule in [ForestRule::JrcOnly, ForestRule::JrcOrHansen] {
            let (c, _) = plot([1.0; 4], 60.0, 2023.0, rule);
            assert_eq!(c.fail_jrc_forest.pixels, 4);
        }
        // Four 10 m pixels are 0.04 ha: under the 0.5 ha floor.
        let (_, code) = plot([1.0; 4], 60.0, 2023.0, ForestRule::JrcOnly);
        assert_eq!(code, 6);
        // No JRC reading: Hansen decides, and says so.
        let (c, _) = plot([f32::NAN; 4], 60.0, 0.0, ForestRule::JrcOnly);
        assert_eq!(c.jrc_unavailable_hansen_only.pixels, 4);
        assert_eq!(c.pass.pixels, 4);
    }

    #[test]
    fn chunks_count_every_pixel_centre_once() {
        let b = BBox {
            min_lat: 5.0,
            max_lat: 5.5,
            min_lng: -7.0,
            max_lng: -6.2,
        };
        let chunks = chunks_of(&b, 9_600, 6_000);
        assert_eq!(chunks.len(), 5 * 3);
        assert_eq!(chunks_of(&b, 2_048, 100).len(), 1);
        assert_eq!(chunks_of(&b, 2_048, 100)[0].owns, Owns::default());
        for k in 0..400 {
            let lat = 5.0 + 0.5 * (k as f64 + 0.5) / 400.0;
            let lng = -7.0 + 0.8 * ((k * 37 % 400) as f64 + 0.5) / 400.0;
            let n = chunks.iter().filter(|c| c.owns.holds(lat, lng)).count();
            assert_eq!(n, 1, "({lat}, {lng})");
        }
        // A centre on a shared side belongs to one chunk only.
        let side = chunks[0].owns.max_lng.unwrap();
        let n = chunks.iter().filter(|c| c.owns.holds(5.4, side)).count();
        assert_eq!(n, 1);
    }

    #[test]
    fn the_commitments_are_stable_and_salted() {
        let salt = salt_from(Some("000102030405060708090a0b0c0d0e0f")).unwrap();
        let a = plot_commitment(&salt, "farm-1");
        assert_eq!(a, plot_commitment(&salt, "farm-1"));
        assert_ne!(a, plot_commitment(&salt, "farm-2"));
        assert_ne!(a, plot_commitment(&[0u8; 16], "farm-1"));
        assert!(salt_from(Some("00")).is_err());
        assert!(salt_from(Some("zz0102030405060708090a0b0c0d0e0f")).is_err());
        assert_ne!(salt_from(None).unwrap(), salt_from(None).unwrap());
        let ring = bbox_ring((5.78, 5.78155, -6.59, -6.58845));
        assert_eq!(geometry_commitment(std::slice::from_ref(&ring)).len(), 64);
        let mut moved = ring;
        moved[0].0 += 1e-9;
        assert_ne!(
            geometry_commitment(&[moved]),
            geometry_commitment(&[bbox_ring((5.78, 5.78155, -6.59, -6.58845))])
        );
    }
}
