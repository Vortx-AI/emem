/* splat-math.js — build 3D gaussians from signed emem facts.
 *
 * Every gaussian parameter traces to a fact the responder signed:
 *   centre   lat/lng from derivation.args, height from the height band
 *   footprint  half the median nearest-neighbour spacing of the sampled grid
 *   thickness  measured sub-cell relief (p90 - p10) or a std-error band
 *   tilt       measured slope + aspect (the local tangent plane)
 *   opacity    the fact's attested confidence
 * Where a shape fact is missing the gaussian falls back to isotropic; it
 * never invents structure the memory does not hold.
 *
 * Pure math, no three.js dependency. Loads as a browser <script>
 * (window.EMEM_SPLAT_MATH) and under node via require() for the selftest:
 *   node -e "require('./splat-math.js').selftest('./test/golden-scene.json')"
 */
(function (root, factory) {
  if (typeof module === "object" && module.exports) module.exports = factory();
  else root.EMEM_SPLAT_MATH = factory();
})(typeof self !== "undefined" ? self : this, function () {
  "use strict";

  var M_PER_DEG_LAT = 111320;
  var UNITS_PER_M = 1 / 120;              // 1 world unit = 120 m
  // width of the (p10, p90) interval in sigma of a normal: 2 * Phi^-1(0.9)
  var P90P10_IN_SIGMA = 2.5631031310892007;

  // median nearest-neighbour ground distance of the sampled cells, metres
  function estimateSpacingM(rows, mLng) {
    var n = rows.length;
    if (n < 2) return 120;
    var nn = new Float64Array(n);
    for (var i = 0; i < n; i++) {
      var best = Infinity;
      for (var j = 0; j < n; j++) {
        if (j === i) continue;
        var dx = (rows[i].lng - rows[j].lng) * mLng;
        var dz = (rows[i].lat - rows[j].lat) * M_PER_DEG_LAT;
        var d2 = dx * dx + dz * dz;
        if (d2 < best) best = d2;
      }
      nn[i] = Math.sqrt(best);
    }
    var s = Array.prototype.slice.call(nn).sort(function (a, b) { return a - b; });
    var mid = n >> 1;
    return n % 2 ? s[mid] : (s[mid - 1] + s[mid]) / 2;
  }

  // tangent frame from measured slope (deg) + aspect (sin, cos of the
  // downslope azimuth, clockwise from north). Vertical exaggeration zx
  // shears the world, so the tilted plane uses theta' = atan(zx tan theta).
  // Returns 3 column vectors [t1, t2, n] or null for flat / missing / the
  // degenerate (0,0) aspect flat cells emit.
  function tangentFrame(slopeDeg, aspectSin, aspectCos, zx) {
    if (!isFinite(slopeDeg) || !isFinite(aspectSin) || !isFinite(aspectCos)) return null;
    if (Math.hypot(aspectSin, aspectCos) < 0.5) return null;
    var thp = Math.atan(zx * Math.tan(slopeDeg * Math.PI / 180));
    var psi = Math.atan2(aspectSin, aspectCos);
    // world axes: +x east, +y up, -z north; downhill = (sin psi, -cos psi) in (x, z)
    var nx = Math.sin(thp) * Math.sin(psi);
    var ny = Math.cos(thp);
    var nz = Math.sin(thp) * -Math.cos(psi);
    var cxl = Math.hypot(nz, nx);            // |n x yhat|
    if (cxl < 1e-6) return null;
    var t1 = [-nz / cxl, 0, nx / cxl];        // normalize(cross(n, yhat))
    var nrm = [nx, ny, nz];
    var t2 = [                                 // cross(n, t1)
      ny * t1[2] - nz * t1[1],
      nz * t1[0] - nx * t1[2],
      nx * t1[1] - ny * t1[0],
    ];
    return [t1, t2, nrm];
  }

  // unit quaternion (w, x, y, z), w >= 0, from column vectors [t1, t2, n]
  function quatFromCols(cols) {
    if (!cols) return [1, 0, 0, 0];
    var m = [
      [cols[0][0], cols[1][0], cols[2][0]],
      [cols[0][1], cols[1][1], cols[2][1]],
      [cols[0][2], cols[1][2], cols[2][2]],
    ];
    var tr = m[0][0] + m[1][1] + m[2][2], s, w, x, y, z;
    if (tr > 0) {
      s = Math.sqrt(tr + 1) * 2;
      w = 0.25 * s; x = (m[2][1] - m[1][2]) / s; y = (m[0][2] - m[2][0]) / s; z = (m[1][0] - m[0][1]) / s;
    } else if (m[0][0] > m[1][1] && m[0][0] > m[2][2]) {
      s = Math.sqrt(1 + m[0][0] - m[1][1] - m[2][2]) * 2;
      w = (m[2][1] - m[1][2]) / s; x = 0.25 * s; y = (m[0][1] + m[1][0]) / s; z = (m[0][2] + m[2][0]) / s;
    } else if (m[1][1] > m[2][2]) {
      s = Math.sqrt(1 + m[1][1] - m[0][0] - m[2][2]) * 2;
      w = (m[0][2] - m[2][0]) / s; x = (m[0][1] + m[1][0]) / s; y = 0.25 * s; z = (m[1][2] + m[2][1]) / s;
    } else {
      s = Math.sqrt(1 + m[2][2] - m[0][0] - m[1][1]) * 2;
      w = (m[1][0] - m[0][1]) / s; x = (m[0][2] + m[2][0]) / s; y = (m[1][2] + m[2][1]) / s; z = 0.25 * s;
    }
    var l = Math.hypot(w, x, y, z);
    var q = [w / l, x / l, y / l, z / l];
    if (q[0] < 0) { q[0] = -q[0]; q[1] = -q[1]; q[2] = -q[2]; q[3] = -q[3]; }
    return q;
  }

  // sorted representatives of a set of near-equal values (the sampled grid's
  // distinct latitudes / longitudes, which can drift by ~1% of the pitch)
  function uniqueSorted(values) {
    var s = values.slice().sort(function (a, b) { return a - b; });
    var eps = (s[s.length - 1] - s[0]) * 1e-6 + 1e-12;
    var reps = [s[0]];
    for (var i = 1; i < s.length; i++) {
      if (s[i] - reps[reps.length - 1] > eps) reps.push(s[i]);
    }
    return reps;
  }

  function rankOf(reps, v) {
    var lo = 0, hi = reps.length - 1;
    while (lo < hi) {
      var mid = (lo + hi) >> 1;
      if (reps[mid] < v) lo = mid + 1; else hi = mid;
    }
    if (lo > 0 && Math.abs(reps[lo - 1] - v) < Math.abs(reps[lo] - v)) lo--;
    return lo;
  }

  /* Terrain shape derived from the sampled grid itself: slope and aspect by
   * finite differences over the neighbouring cells' signed height facts
   * (central where both sides exist, one-sided at edges — the same
   * neighbourhood computation as the responder's /v1/terrain), and the
   * residual roughness after removing that gradient plane as the vertical
   * sigma (RMS residual over >= 2 of the 8 neighbours). Distances use the
   * neighbours' actual coordinates, so irregular grid pitch cancels out.
   * Every input is a signed fact already in the scene.
   */
  function gridShape(rows, hb, mLng) {
    var lats = uniqueSorted(rows.map(function (r) { return r.lat; }));
    var lngs = uniqueSorted(rows.map(function (r) { return r.lng; }));
    var byCell = {};
    rows.forEach(function (r, i) {
      byCell[rankOf(lngs, r.lng) + "," + rankOf(lats, r.lat)] = i;
    });
    return rows.map(function (r, i) {
      var gx = rankOf(lngs, r.lng), gz = rankOf(lats, r.lat);
      var zc = r[hb];
      function nb(dx, dz) {
        var j = byCell[(gx + dx) + "," + (gz + dz)];
        if (j === undefined) return null;
        var n = rows[j];
        return n[hb] === undefined || n[hb] === null ? null : n;
      }
      var E = nb(1, 0), W = nb(-1, 0), N = nb(0, 1), S = nb(0, -1);
      var gE = null, gN = null;
      if (E && W) gE = (E[hb] - W[hb]) / ((E.lng - W.lng) * mLng);
      else if (E) gE = (E[hb] - zc) / ((E.lng - r.lng) * mLng);
      else if (W) gE = (zc - W[hb]) / ((r.lng - W.lng) * mLng);
      if (N && S) gN = (N[hb] - S[hb]) / ((N.lat - S.lat) * M_PER_DEG_LAT);
      else if (N) gN = (N[hb] - zc) / ((N.lat - r.lat) * M_PER_DEG_LAT);
      else if (S) gN = (zc - S[hb]) / ((r.lat - S.lat) * M_PER_DEG_LAT);
      var ge = gE || 0, gn = gN || 0;
      var h = Math.hypot(ge, gn);
      var out = { hasFrame: false, hasRelief: false };
      if ((gE !== null || gN !== null) && h > 1e-12) {
        out.hasFrame = true;
        out.slopeDeg = Math.atan(h) * 180 / Math.PI;
        out.aspSin = -ge / h;                  // downhill azimuth components
        out.aspCos = -gn / h;
      }
      var sum2 = 0, cnt = 0;
      for (var dz = -1; dz <= 1; dz++) {
        for (var dx = -1; dx <= 1; dx++) {
          if (!dx && !dz) continue;
          var n = nb(dx, dz);
          if (!n) continue;
          var res = n[hb] - (zc + ge * (n.lng - r.lng) * mLng
                                + gn * (n.lat - r.lat) * M_PER_DEG_LAT);
          sum2 += res * res; cnt++;
        }
      }
      if (cnt >= 2) {
        out.hasRelief = true;
        out.sigmaZm = Math.sqrt(sum2 / cnt);
      }
      return out;
    });
  }

  // upper triangle of Sigma = R diag(s^2) R^T as [S00, S01, S02, S11, S12, S22]
  function covUpper(cols, s1, s2, s3) {
    if (!cols) cols = [[1, 0, 0], [0, 1, 0], [0, 0, 1]];
    var d = [s1 * s1, s2 * s2, s3 * s3];
    var out = [0, 0, 0, 0, 0, 0];
    // Sigma_ij = sum_k R_ik d_k R_jk with R columns = cols[k]
    function S(i, j) {
      var v = 0;
      for (var k = 0; k < 3; k++) v += cols[k][i] * d[k] * cols[k][j];
      return v;
    }
    out[0] = S(0, 0); out[1] = S(0, 1); out[2] = S(0, 2);
    out[3] = S(1, 1); out[4] = S(1, 2); out[5] = S(2, 2);
    return out;
  }

  /* scene: { cells: { cell64: {lat, lng, "<band>": value, _conf?, ...} } }
   * cfg:   { heightBand, heightExaggeration?, shape?: {
   *            gridNormals?: true,            // tilt from finite differences over the sampled grid
   *            gridRelief?: true,             // thickness from the detrended neighbour residual RMS
   *            reliefBands?: [p10Band, p90Band], slopeBand?, aspectBands?: [sinBand, cosBand],
   *            sigmaBand?,                    // std error in height-band units, alternative to reliefBands
   *            sigmaFloorM?: 8, footprintScale?: 1, opacity?: 0.85 } }
   * Band-driven shape (reliefBands / slopeBand / aspectBands) applies where a
   * responder wires those bands; grid-driven shape needs nothing beyond the
   * height band. Grid wins when both are enabled and computable.
   * Returns typed arrays, one gaussian per cell that has a position and a height fact.
   */
  function buildGaussians(scene, cfg) {
    var hb = cfg.heightBand;
    var zx = cfg.heightExaggeration || 1.6;
    var shape = cfg.shape || {};
    var floorS3 = (shape.sigmaFloorM !== undefined ? shape.sigmaFloorM : 8) * zx * UNITS_PER_M;
    var fps = shape.footprintScale || 1;
    var baseOpacity = shape.opacity !== undefined ? shape.opacity : 0.85;

    var cellIds = [], rows = [];
    for (var id in scene.cells) {
      var r = scene.cells[id];
      if (r.lat === undefined || r[hb] === undefined || r[hb] === null) continue;
      cellIds.push(id); rows.push(r);
    }
    var n = rows.length;
    if (!n) throw new Error("no facts with positions and a " + hb + " value");

    var lat0 = 0, lng0 = 0, hMin = Infinity, hMax = -Infinity;
    var lngMin = Infinity, lngMax = -Infinity;
    for (var i = 0; i < n; i++) {
      lat0 += rows[i].lat; lng0 += rows[i].lng;
      var h = rows[i][hb];
      if (h < hMin) hMin = h;
      if (h > hMax) hMax = h;
      if (rows[i].lng < lngMin) lngMin = rows[i].lng;
      if (rows[i].lng > lngMax) lngMax = rows[i].lng;
    }
    lat0 /= n; lng0 /= n;
    var mLng = M_PER_DEG_LAT * Math.cos(lat0 * Math.PI / 180);
    var spacingM = estimateSpacingM(rows, mLng);
    var sg = fps * spacingM / 2 * UNITS_PER_M;
    var grid = (shape.gridNormals || shape.gridRelief) ? gridShape(rows, hb, mLng) : null;

    var positions = new Float32Array(3 * n);
    var sigmas = new Float32Array(3 * n);
    var quats = new Float32Array(4 * n);
    var cov6 = new Float32Array(6 * n);
    var opacities = new Float32Array(n);

    for (i = 0; i < n; i++) {
      var row = rows[i];
      positions[3 * i] = (row.lng - lng0) * mLng * UNITS_PER_M;
      positions[3 * i + 1] = (row[hb] - hMin) * zx * UNITS_PER_M;
      positions[3 * i + 2] = -(row.lat - lat0) * M_PER_DEG_LAT * UNITS_PER_M;

      // thickness along the surface normal, from measured spread
      var s3;
      if (shape.gridRelief && grid && grid[i].hasRelief) {
        s3 = Math.max(floorS3, grid[i].sigmaZm * zx * UNITS_PER_M);
      } else if (shape.sigmaBand !== undefined && shape.sigmaBand !== null &&
          row[shape.sigmaBand] !== undefined && row[shape.sigmaBand] !== null) {
        s3 = Math.max(floorS3, row[shape.sigmaBand] * zx * UNITS_PER_M);
      } else if (shape.reliefBands &&
                 row[shape.reliefBands[0]] !== undefined && row[shape.reliefBands[0]] !== null &&
                 row[shape.reliefBands[1]] !== undefined && row[shape.reliefBands[1]] !== null) {
        var span = row[shape.reliefBands[1]] - row[shape.reliefBands[0]];
        s3 = span > 0
          ? Math.max(floorS3, span / P90P10_IN_SIGMA * zx * UNITS_PER_M)
          : floorS3;
      } else {
        s3 = 0.5 * sg;                       // no measurement of spread: modest isotropic-ish blob
      }

      // orientation from measured slope + aspect; identity when absent
      var cols = null;
      if (shape.gridNormals && grid && grid[i].hasFrame) {
        cols = tangentFrame(grid[i].slopeDeg, grid[i].aspSin, grid[i].aspCos, zx);
      } else if (shape.slopeBand && shape.aspectBands) {
        cols = tangentFrame(row[shape.slopeBand],
                            row[shape.aspectBands[0]], row[shape.aspectBands[1]], zx);
      }

      var q = quatFromCols(cols);
      quats[4 * i] = q[0]; quats[4 * i + 1] = q[1]; quats[4 * i + 2] = q[2]; quats[4 * i + 3] = q[3];
      sigmas[3 * i] = sg; sigmas[3 * i + 1] = sg; sigmas[3 * i + 2] = s3;
      var c6 = covUpper(cols, sg, sg, s3);
      for (var k = 0; k < 6; k++) cov6[6 * i + k] = c6[k];
      opacities[i] = baseOpacity * (row._conf !== undefined ? row._conf : 1);
    }

    return {
      count: n, rows: rows, cellIds: cellIds,
      positions: positions, sigmas: sigmas, quats: quats, cov6: cov6, opacities: opacities,
      spacingM: spacingM, hMin: hMin, hMax: hMax, lat0: lat0, lng0: lng0,
      unitsPerMeter: UNITS_PER_M, zx: zx,
      spanX: (lngMax - lngMin) * mLng * UNITS_PER_M,
    };
  }

  // value bands present in the scene (exclude bookkeeping keys), so a derived
  // node interpolates every band colorize() might read. Mirrors _data_bands().
  function dataBands(rows) {
    var skip = { lat: 1, lng: 1, _cids: 1, _conf: 1, _sem: 1 };
    var seen = [];
    for (var i = 0; i < rows.length; i++) {
      for (var k in rows[i]) {
        if (!skip[k] && seen.indexOf(k) < 0 && typeof rows[i][k] === "number") seen.push(k);
      }
    }
    return seen;
  }

  /* Provenance-preserving bilinear densification (emem.splat_provenance.v2).
   * Subdivide each grid quad F x F (cfg.densify) and emit a gaussian per fine
   * node. A node on an original cell is `measured` and keeps that cell's
   * fact_cid; every other node is `derived` and records its <= 4 source cells
   * and bilinear weights (summing to 1), so value[b] == sum_i w_i * source_i[b]
   * stays re-derivable and every source stays signature-checkable. Anisotropy
   * for a derived node is the analytic gradient of the same bilinear patch.
   * Returns the same typed-array shape as buildGaussians (so the renderer and
   * the golden selftest consume it unchanged) plus `prov` and `cells`. MUST
   * stay in lockstep with densify_gaussians() in make_splats.py — the golden
   * fixture's densify case pins both to 1e-6. */
  function densifyGaussians(scene, cfg) {
    var factor = Math.max(1, cfg.densify | 0);
    var hb = cfg.heightBand;
    var zx = cfg.heightExaggeration || 1.6;
    var shape = cfg.shape || {};
    var floorS3 = (shape.sigmaFloorM !== undefined ? shape.sigmaFloorM : 8) * zx * UNITS_PER_M;
    var fps = shape.footprintScale || 1;
    var baseOpacity = shape.opacity !== undefined ? shape.opacity : 0.85;

    var cellIds0 = [], rows = [];
    for (var id in scene.cells) {
      var r = scene.cells[id];
      if (r.lat === undefined || r[hb] === undefined || r[hb] === null) continue;
      cellIds0.push(id); rows.push(r);
    }
    var n0 = rows.length;
    if (!n0) throw new Error("no facts with positions and a " + hb + " value");

    var lat0 = 0, lng0 = 0, hMin = Infinity, hMax = -Infinity;
    var lngMin = Infinity, lngMax = -Infinity, i;
    for (i = 0; i < n0; i++) {
      lat0 += rows[i].lat; lng0 += rows[i].lng;
      var h = rows[i][hb];
      if (h < hMin) hMin = h; if (h > hMax) hMax = h;
      if (rows[i].lng < lngMin) lngMin = rows[i].lng;
      if (rows[i].lng > lngMax) lngMax = rows[i].lng;
    }
    lat0 /= n0; lng0 /= n0;
    var mLng = M_PER_DEG_LAT * Math.cos(lat0 * Math.PI / 180);
    var spacingM = estimateSpacingM(rows, mLng);
    var sg = fps * (spacingM / factor) / 2 * UNITS_PER_M;
    var gsh = gridShape(rows, hb, mLng);

    var lngs = uniqueSorted(rows.map(function (r) { return r.lng; }));
    var lats = uniqueSorted(rows.map(function (r) { return r.lat; }));
    var gidx = {};
    for (i = 0; i < n0; i++) gidx[rankOf(lngs, rows[i].lng) + "," + rankOf(lats, rows[i].lat)] = i;
    var W = lngs.length, H = lats.length;
    var bands = dataBands(rows);
    // categorical bands (e.g. a Hansen loss YEAR, or a discrete class code)
    // are inherited from the nearest signed cell, never averaged.
    var catSet = {};
    (cfg.categoricalBands || []).forEach(function (b) { catSet[b] = 1; });
    var sem = !!cfg.prepare;
    function corner(gx, gz) {
      var j = gidx[gx + "," + gz];
      return j === undefined ? -1 : j;
    }

    var P = [], SG = [], Q = [], C6 = [], OP = [], SR = [], CID = [], PROV = [];
    var OW = (W - 1) * factor + 1, OH = (H - 1) * factor + 1;
    var measured = 0, derived = 0;
    for (var I = 0; I < OW; I++) {
      for (var J = 0; J < OH; J++) {
        var gx = Math.min((I / factor) | 0, W - 2);
        var gz = Math.min((J / factor) | 0, H - 2);
        var u = (I - gx * factor) / factor, v = (J - gz * factor) / factor;
        var cs = [corner(gx, gz), corner(gx + 1, gz), corner(gx, gz + 1), corner(gx + 1, gz + 1)];
        var wt = [(1 - u) * (1 - v), u * (1 - v), (1 - u) * v, u * v];
        var isMeas = (I % factor === 0 && J % factor === 0), kk, b, bi;
        var near = 0; for (kk = 1; kk < 4; kk++) if (wt[kk] > wt[near]) near = kk;
        var lat, lng, hh, srow, s3, cols, conf, R;

        if (isMeas) {
          // a node on an original cell IS that signed cell: kept exact and
          // emitted whenever the cell exists (even at a ragged grid edge whose
          // quad is incomplete), so densifying never drops or mislabels a
          // measured fact. `near` is the corner it sits on.
          var j = cs[near];
          if (j < 0) continue;
          var r = rows[j];
          lat = r.lat; lng = r.lng; hh = r[hb];
          srow = { lat: lat, lng: lng };
          for (bi = 0; bi < bands.length; bi++) { b = bands[bi]; if (r[b] !== undefined && r[b] !== null) srow[b] = r[b]; }
          if (sem && r._sem && r._sem.length === 3) srow._sem = [r._sem[0], r._sem[1], r._sem[2]];
          var gg = gsh[j];
          s3 = gg.hasRelief ? Math.max(floorS3, gg.sigmaZm * zx * UNITS_PER_M) : 0.5 * sg * factor;
          cols = gg.hasFrame ? tangentFrame(gg.slopeDeg, gg.aspSin, gg.aspCos, zx) : null;
          conf = r._conf !== undefined ? r._conf : 1;
          measured++; CID.push(cellIds0[j]);
          PROV.push({ kind: "measured", cell: cellIds0[j] });
        } else {
          if (cs[0] < 0 || cs[1] < 0 || cs[2] < 0 || cs[3] < 0) continue;
          R = [rows[cs[0]], rows[cs[1]], rows[cs[2]], rows[cs[3]]];
          lat = 0; lng = 0; hh = 0;
          for (kk = 0; kk < 4; kk++) { lat += wt[kk] * R[kk].lat; lng += wt[kk] * R[kk].lng; hh += wt[kk] * R[kk][hb]; }
          srow = { lat: lat, lng: lng };
          for (bi = 0; bi < bands.length; bi++) {
            b = bands[bi]; var have = true;
            for (kk = 0; kk < 4; kk++) { if (R[kk][b] === undefined || R[kk][b] === null) { have = false; break; } }
            if (!have) continue;
            if (catSet[b]) { srow[b] = R[near][b]; }
            else { var acc = 0; for (kk = 0; kk < 4; kk++) acc += wt[kk] * R[kk][b]; srow[b] = acc; }
          }
          if (sem) {
            var oks = true, sm = [0, 0, 0];
            for (kk = 0; kk < 4; kk++) { var sv = R[kk]._sem; if (!(sv && sv.length === 3)) { oks = false; break; } sm[0] += wt[kk] * sv[0]; sm[1] += wt[kk] * sv[1]; sm[2] += wt[kk] * sv[2]; }
            if (oks) srow._sem = sm;
          }
          var hSW = R[0][hb], hSE = R[1][hb], hNW = R[2][hb], hNE = R[3][hb];
          var dhdu = (1 - v) * (hSE - hSW) + v * (hNE - hNW);
          var dhdv = (1 - u) * (hNW - hSW) + u * (hNE - hSE);
          var dxm = ((R[1].lng - R[0].lng) * (1 - v) + (R[3].lng - R[2].lng) * v) * mLng || 1e-9;
          var dym = ((R[2].lat - R[0].lat) * (1 - u) + (R[3].lat - R[1].lat) * u) * M_PER_DEG_LAT || 1e-9;
          var ge = dhdu / dxm, gn = dhdv / dym, mag = Math.hypot(ge, gn);
          cols = mag > 1e-12 ? tangentFrame(Math.atan(mag) * 180 / Math.PI, -ge / mag, -gn / mag, zx) : null;
          var okz = true, sz = 0;
          for (kk = 0; kk < 4; kk++) { var z = gsh[cs[kk]].sigmaZm; if (z === undefined) { okz = false; break; } sz += wt[kk] * z; }
          s3 = okz ? Math.max(floorS3, sz * zx * UNITS_PER_M) : 0.5 * sg;
          conf = Math.min(
            R[0]._conf !== undefined ? R[0]._conf : 1, R[1]._conf !== undefined ? R[1]._conf : 1,
            R[2]._conf !== undefined ? R[2]._conf : 1, R[3]._conf !== undefined ? R[3]._conf : 1);
          derived++; CID.push(null);
          var value = {};
          for (bi = 0; bi < bands.length; bi++) { b = bands[bi]; if (srow[b] !== undefined) value[b] = srow[b]; }
          PROV.push({ kind: "derived", method: "bilinear", at: { lat: lat, lng: lng },
            sources: [{ cell: cellIds0[cs[0]], weight: wt[0] }, { cell: cellIds0[cs[1]], weight: wt[1] },
                      { cell: cellIds0[cs[2]], weight: wt[2] }, { cell: cellIds0[cs[3]], weight: wt[3] }],
            nearest: cellIds0[cs[near]],   // dominant corner: source of any categorical band
            value: value });
        }
        var q = quatFromCols(cols), c6 = covUpper(cols, sg, sg, s3);
        P.push((lng - lng0) * mLng * UNITS_PER_M, (hh - hMin) * zx * UNITS_PER_M, -(lat - lat0) * M_PER_DEG_LAT * UNITS_PER_M);
        SG.push(sg, sg, s3);
        Q.push(q[0], q[1], q[2], q[3]);
        C6.push(c6[0], c6[1], c6[2], c6[3], c6[4], c6[5]);
        OP.push(baseOpacity * conf);
        SR.push(srow);
      }
    }
    var n = SR.length, cells = {};
    for (i = 0; i < n0; i++) {
      var vals = {};
      for (var b2 = 0; b2 < bands.length; b2++) { var bb = bands[b2]; if (rows[i][bb] !== undefined && rows[i][bb] !== null) vals[bb] = rows[i][bb]; }
      cells[cellIds0[i]] = { cids: rows[i]._cids || {}, vals: vals };
    }
    return {
      count: n, rows: SR, cellIds: CID,
      positions: new Float32Array(P), sigmas: new Float32Array(SG),
      quats: new Float32Array(Q), cov6: new Float32Array(C6), opacities: new Float32Array(OP),
      spacingM: spacingM, hMin: hMin, hMax: hMax, lat0: lat0, lng0: lng0,
      unitsPerMeter: UNITS_PER_M, zx: zx, spanX: (lngMax - lngMin) * mLng * UNITS_PER_M,
      prov: PROV, cells: cells, measuredCount: measured, derivedCount: derived,
      grid: [W, H], outputGrid: [OW, OH],
    };
  }

  // node-only: assert against the golden fixture, plus analytic spot checks
  function selftest(fixturePath) {
    var fs = require("fs");
    var fix = JSON.parse(fs.readFileSync(fixturePath, "utf8"));
    var tol = 1e-6, checks = 0;
    function close(a, b, what) {
      if (Math.abs(a - b) > tol) throw new Error(what + ": got " + a + " want " + b);
      checks++;
    }
    fix.cases.forEach(function (c) {
      var g = c.cfg.densify > 1 ? densifyGaussians(c.scene || fix.scene, c.cfg)
                                : buildGaussians(c.scene || fix.scene, c.cfg);
      close(g.spacingM, c.expected.spacing_m, c.name + " spacing");
      close(g.hMin, c.expected.hMin, c.name + " hMin");
      close(g.hMax, c.expected.hMax, c.name + " hMax");
      c.expected.gaussians.forEach(function (e, i) {
        for (var k = 0; k < 3; k++) {
          close(g.positions[3 * i + k], e.p[k], c.name + " g" + i + " p" + k);
          close(g.sigmas[3 * i + k], e.sigma[k], c.name + " g" + i + " sigma" + k);
        }
        for (k = 0; k < 4; k++) close(g.quats[4 * i + k], e.quat[k], c.name + " g" + i + " quat" + k);
        for (k = 0; k < 6; k++) close(g.cov6[6 * i + k], e.cov6[k], c.name + " g" + i + " cov" + k);
        // interpolated band values (continuous bilinear + categorical nearest)
        if (e.row) for (var rb in e.row) close(g.rows[i][rb], e.row[rb], c.name + " g" + i + " row." + rb);
      });
    });
    // analytic: flat cell keeps identity rotation and a diagonal covariance
    var flat = tangentFrame(0, 0, 1, 2);
    if (flat !== null) throw new Error("flat slope must return null frame");
    // analytic: 45 deg east-facing slope, zx=1 -> normal (1,1,0)/sqrt(2)
    var f = tangentFrame(45, 1, 0, 1);
    close(f[2][0], Math.SQRT1_2, "45E normal x");
    close(f[2][1], Math.SQRT1_2, "45E normal y");
    close(f[2][2], 0, "45E normal z");
    // frame is right-handed and orthonormal
    var det = f[0][0] * (f[1][1] * f[2][2] - f[1][2] * f[2][1])
            - f[1][0] * (f[0][1] * f[2][2] - f[0][2] * f[2][1])
            + f[2][0] * (f[0][1] * f[1][2] - f[0][2] * f[1][1]);
    close(det, 1, "frame det");
    return checks;
  }

  return {
    M_PER_DEG_LAT: M_PER_DEG_LAT,
    UNITS_PER_M: UNITS_PER_M,
    P90P10_IN_SIGMA: P90P10_IN_SIGMA,
    estimateSpacingM: estimateSpacingM,
    gridShape: gridShape,
    tangentFrame: tangentFrame,
    quatFromCols: quatFromCols,
    covUpper: covUpper,
    buildGaussians: buildGaussians,
    densifyGaussians: densifyGaussians,
    dataBands: dataBands,
    selftest: selftest,
  };
});
