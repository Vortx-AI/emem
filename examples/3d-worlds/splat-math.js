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
   *            reliefBands?: [p10Band, p90Band], slopeBand?, aspectBands?: [sinBand, cosBand],
   *            sigmaBand?,                    // std error in height-band units, alternative to reliefBands
   *            sigmaFloorM?: 8, footprintScale?: 1, opacity?: 0.85 } }
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
      if (shape.sigmaBand !== undefined && shape.sigmaBand !== null &&
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
      if (shape.slopeBand && shape.aspectBands) {
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
      var g = buildGaussians(fix.scene, c.cfg);
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
    tangentFrame: tangentFrame,
    quatFromCols: quatFromCols,
    covUpper: covUpper,
    buildGaussians: buildGaussians,
    selftest: selftest,
  };
});
