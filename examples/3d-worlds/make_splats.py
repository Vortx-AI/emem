#!/usr/bin/env python3
"""make_splats.py — export an emem region as signed gaussian splats.

Fetches a region from an emem responder exactly like the browser templates
(query_region -> recall_many), builds one anisotropic 3-D gaussian per cell
with the same math as splat-math.js (pinned by test/golden-scene.json), and
writes four artifacts:

  <out>.ply             standard 3D Gaussian Splatting PLY (opens in
                        SuperSplat, gsplat, antimatter15/splat, PlayCanvas)
  <out>.splat           antimatter15 32-byte-per-splat format
  <out>.provenance.json who signed what: per-splat fact CIDs, the verbatim
                        signed receipts, artifact hashes, re-check recipe
  <out>.scene.json      the fetched scene, usable as window.EMEM_DATA for
                        offline rendering and capture.mjs

Every gaussian parameter traces to a signed fact: position from lat/lng and
the height band, footprint from the sampled grid pitch, thickness from
measured sub-cell relief (p90 - p10) or a std-error band, tilt from measured
slope + aspect, opacity from the fact's attested confidence.

Usage:
  python3 make_splats.py --preset carbon --out out/rondonia
  python3 make_splats.py --preset carbon --out out/rondonia --verify
  python3 make_splats.py --bbox -63.5,-9.5,-63.0,-9.1 --bands ... (see --help)
  python3 make_splats.py --selftest

Stdlib only. Receipt verification round-trips through POST /v1/verify_receipt.
"""

import argparse
import base64
import hashlib
import json
import math
import http.client
import struct
import sys
import time
import urllib.error
import urllib.request

M_PER_DEG_LAT = 111320.0
UNITS_PER_M = 1.0 / 120.0            # 1 world unit = 120 m
P90P10_IN_SIGMA = 2.5631031310892007  # width of (p10, p90) in sigma of a normal
SH_C0 = 0.28209479177387814           # Y_0^0; f_dc = (rgb - 0.5) / SH_C0

# ---------------------------------------------------------------------------
# gaussian construction — must stay in lockstep with splat-math.js
# ---------------------------------------------------------------------------

def estimate_spacing_m(rows, m_lng):
    n = len(rows)
    if n < 2:
        return 120.0
    nn = []
    for i in range(n):
        best = float("inf")
        for j in range(n):
            if j == i:
                continue
            dx = (rows[i]["lng"] - rows[j]["lng"]) * m_lng
            dz = (rows[i]["lat"] - rows[j]["lat"]) * M_PER_DEG_LAT
            d2 = dx * dx + dz * dz
            if d2 < best:
                best = d2
        nn.append(math.sqrt(best))
    nn.sort()
    mid = n // 2
    return nn[mid] if n % 2 else (nn[mid - 1] + nn[mid]) / 2.0


def tangent_frame(slope_deg, aspect_sin, aspect_cos, zx):
    for v in (slope_deg, aspect_sin, aspect_cos):
        if v is None or not math.isfinite(v):
            return None
    if math.hypot(aspect_sin, aspect_cos) < 0.5:
        return None
    thp = math.atan(zx * math.tan(math.radians(slope_deg)))
    psi = math.atan2(aspect_sin, aspect_cos)
    nx = math.sin(thp) * math.sin(psi)
    ny = math.cos(thp)
    nz = math.sin(thp) * -math.cos(psi)
    cxl = math.hypot(nz, nx)
    if cxl < 1e-6:
        return None
    t1 = (-nz / cxl, 0.0, nx / cxl)
    t2 = (ny * t1[2] - nz * t1[1], nz * t1[0] - nx * t1[2], nx * t1[1] - ny * t1[0])
    return [t1, t2, (nx, ny, nz)]


def quat_from_cols(cols):
    if cols is None:
        return [1.0, 0.0, 0.0, 0.0]
    m = [[cols[c][r] for c in range(3)] for r in range(3)]
    tr = m[0][0] + m[1][1] + m[2][2]
    if tr > 0:
        s = math.sqrt(tr + 1) * 2
        w, x, y, z = 0.25 * s, (m[2][1] - m[1][2]) / s, (m[0][2] - m[2][0]) / s, (m[1][0] - m[0][1]) / s
    elif m[0][0] > m[1][1] and m[0][0] > m[2][2]:
        s = math.sqrt(1 + m[0][0] - m[1][1] - m[2][2]) * 2
        w, x, y, z = (m[2][1] - m[1][2]) / s, 0.25 * s, (m[0][1] + m[1][0]) / s, (m[0][2] + m[2][0]) / s
    elif m[1][1] > m[2][2]:
        s = math.sqrt(1 + m[1][1] - m[0][0] - m[2][2]) * 2
        w, x, y, z = (m[0][2] - m[2][0]) / s, (m[0][1] + m[1][0]) / s, 0.25 * s, (m[1][2] + m[2][1]) / s
    else:
        s = math.sqrt(1 + m[2][2] - m[0][0] - m[1][1]) * 2
        w, x, y, z = (m[1][0] - m[0][1]) / s, (m[0][2] + m[2][0]) / s, (m[1][2] + m[2][1]) / s, 0.25 * s
    l = math.sqrt(w * w + x * x + y * y + z * z)
    q = [w / l, x / l, y / l, z / l]
    return [-v for v in q] if q[0] < 0 else q


def unique_sorted(values):
    s = sorted(values)
    eps = (s[-1] - s[0]) * 1e-6 + 1e-12
    reps = [s[0]]
    for v in s[1:]:
        if v - reps[-1] > eps:
            reps.append(v)
    return reps


def rank_of(reps, v):
    import bisect
    i = bisect.bisect_left(reps, v)
    if i >= len(reps):
        i = len(reps) - 1
    if i > 0 and abs(reps[i - 1] - v) < abs(reps[i] - v):
        i -= 1
    return i


def grid_shape(rows, hb, m_lng):
    """Slope/aspect by finite differences over the sampled grid's signed
    height facts, plus the detrended neighbour residual RMS as vertical
    sigma — mirrors gridShape() in splat-math.js."""
    lats = unique_sorted([r["lat"] for r in rows])
    lngs = unique_sorted([r["lng"] for r in rows])
    by_cell = {}
    for i, r in enumerate(rows):
        by_cell[(rank_of(lngs, r["lng"]), rank_of(lats, r["lat"]))] = i
    out = []
    for r in rows:
        gx, gz = rank_of(lngs, r["lng"]), rank_of(lats, r["lat"])
        zc = r[hb]

        def nb(dx, dz):
            j = by_cell.get((gx + dx, gz + dz))
            if j is None:
                return None
            n = rows[j]
            return None if n.get(hb) is None else n

        E, W, N, S = nb(1, 0), nb(-1, 0), nb(0, 1), nb(0, -1)
        gE = gN = None
        if E and W:
            gE = (E[hb] - W[hb]) / ((E["lng"] - W["lng"]) * m_lng)
        elif E:
            gE = (E[hb] - zc) / ((E["lng"] - r["lng"]) * m_lng)
        elif W:
            gE = (zc - W[hb]) / ((r["lng"] - W["lng"]) * m_lng)
        if N and S:
            gN = (N[hb] - S[hb]) / ((N["lat"] - S["lat"]) * M_PER_DEG_LAT)
        elif N:
            gN = (N[hb] - zc) / ((N["lat"] - r["lat"]) * M_PER_DEG_LAT)
        elif S:
            gN = (zc - S[hb]) / ((r["lat"] - S["lat"]) * M_PER_DEG_LAT)
        ge, gn = gE or 0.0, gN or 0.0
        h = math.hypot(ge, gn)
        rec = {"hasFrame": False, "hasRelief": False}
        if (gE is not None or gN is not None) and h > 1e-12:
            rec.update(hasFrame=True, slopeDeg=math.degrees(math.atan(h)),
                       aspSin=-ge / h, aspCos=-gn / h)
        sum2 = cnt = 0
        for dz in (-1, 0, 1):
            for dx in (-1, 0, 1):
                if not dx and not dz:
                    continue
                n = nb(dx, dz)
                if not n:
                    continue
                res = n[hb] - (zc + ge * (n["lng"] - r["lng"]) * m_lng
                                  + gn * (n["lat"] - r["lat"]) * M_PER_DEG_LAT)
                sum2 += res * res
                cnt += 1
        if cnt >= 2:
            rec.update(hasRelief=True, sigmaZm=math.sqrt(sum2 / cnt))
        out.append(rec)
    return out


def cov_upper(cols, s1, s2, s3):
    if cols is None:
        cols = [(1, 0, 0), (0, 1, 0), (0, 0, 1)]
    d = [s1 * s1, s2 * s2, s3 * s3]

    def S(i, j):
        return sum(cols[k][i] * d[k] * cols[k][j] for k in range(3))

    return [S(0, 0), S(0, 1), S(0, 2), S(1, 1), S(1, 2), S(2, 2)]


def build_gaussians(scene, cfg):
    hb = cfg["heightBand"]
    zx = cfg.get("heightExaggeration", 1.6)
    shape = cfg.get("shape") or {}
    floor_s3 = shape.get("sigmaFloorM", 8) * zx * UNITS_PER_M
    fps = shape.get("footprintScale", 1)
    base_opacity = shape.get("opacity", 0.85)

    cell_ids, rows = [], []
    for cid, r in scene["cells"].items():
        if r.get("lat") is None or r.get(hb) is None:
            continue
        cell_ids.append(cid)
        rows.append(r)
    if not rows:
        raise SystemExit("no facts with positions and a %s value" % hb)

    n = len(rows)
    lat0 = sum(r["lat"] for r in rows) / n
    lng0 = sum(r["lng"] for r in rows) / n
    hs = [r[hb] for r in rows]
    h_min, h_max = min(hs), max(hs)
    m_lng = M_PER_DEG_LAT * math.cos(math.radians(lat0))
    spacing = estimate_spacing_m(rows, m_lng)
    sg = fps * spacing / 2.0 * UNITS_PER_M
    grid = (grid_shape(rows, hb, m_lng)
            if shape.get("gridNormals") or shape.get("gridRelief") else None)

    out = []
    for i, r in enumerate(rows):
        p = [(r["lng"] - lng0) * m_lng * UNITS_PER_M,
             (r[hb] - h_min) * zx * UNITS_PER_M,
             -(r["lat"] - lat0) * M_PER_DEG_LAT * UNITS_PER_M]

        sb = shape.get("sigmaBand")
        rb = shape.get("reliefBands")
        if shape.get("gridRelief") and grid and grid[i]["hasRelief"]:
            s3 = max(floor_s3, grid[i]["sigmaZm"] * zx * UNITS_PER_M)
        elif sb is not None and r.get(sb) is not None:
            s3 = max(floor_s3, r[sb] * zx * UNITS_PER_M)
        elif rb and r.get(rb[0]) is not None and r.get(rb[1]) is not None:
            span = r[rb[1]] - r[rb[0]]
            s3 = max(floor_s3, span / P90P10_IN_SIGMA * zx * UNITS_PER_M) if span > 0 else floor_s3
        else:
            s3 = 0.5 * sg

        cols = None
        if shape.get("gridNormals") and grid and grid[i]["hasFrame"]:
            g = grid[i]
            cols = tangent_frame(g["slopeDeg"], g["aspSin"], g["aspCos"], zx)
        elif shape.get("slopeBand") and shape.get("aspectBands"):
            ab = shape["aspectBands"]
            cols = tangent_frame(r.get(shape["slopeBand"]), r.get(ab[0]), r.get(ab[1]), zx)

        out.append({
            "p": p,
            "sigma": [sg, sg, s3],
            "quat": quat_from_cols(cols),
            "cov6": cov_upper(cols, sg, sg, s3),
            "opacity": base_opacity * r.get("_conf", 1.0),
            "row": r,
        })
    return {
        "gaussians": out, "cell_ids": cell_ids, "rows": rows,
        "spacing_m": spacing, "h_min": h_min, "h_max": h_max,
        "lat0": lat0, "lng0": lng0,
    }

# ---------------------------------------------------------------------------
# responder I/O
# ---------------------------------------------------------------------------

def http_post(base, path, body, retries=5, timeout=90):
    """POST with backoff on 429/5xx — cold regions materialize slowly."""
    payload = json.dumps(body).encode()
    delay = 2.0
    for attempt in range(retries + 1):
        req = urllib.request.Request(base + path, data=payload,
                                     headers={"content-type": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=timeout) as r:
                return json.loads(r.read())
        except urllib.error.HTTPError as e:
            if e.code in (429, 500, 502, 503, 504) and attempt < retries:
                time.sleep(delay)
                delay = min(delay * 2, 30)
                continue
            raise SystemExit("%s -> HTTP %s: %s" % (path, e.code, e.read()[:200]))
        except (urllib.error.URLError, TimeoutError, ConnectionError,
                http.client.HTTPException, OSError):
            if attempt < retries:
                time.sleep(delay)
                delay = min(delay * 2, 30)
                continue
            raise
    raise SystemExit("%s: retries exhausted" % path)


def recall_batch(base, scene, receipts, cells, band):
    """recall_many with split-on-timeout, mirroring emem-world.js."""
    try:
        d = http_post(base, "/v1/recall_many", {"cells": cells, "bands": [band]},
                      retries=3, timeout=120)
    except (SystemExit, Exception) as e:
        if len(cells) > 8:
            mid = len(cells) // 2
            recall_batch(base, scene, receipts, cells[:mid], band)
            recall_batch(base, scene, receipts, cells[mid:], band)
            return
        # a cold floor-size batch that still cannot materialize is skipped:
        # those cells get no fact and so no splat; a warm rerun fills them
        print("skipping %d cells for %s (%s)" % (len(cells), band, e),
              file=sys.stderr)
        return
    for cell, entry in (d.get("by_cell") or {}).items():
        rec = scene["cells"].setdefault(cell, {})
        cids = rec.setdefault("_cids", {})
        for f in entry.get("facts") or []:
            if f.get("value") is None:
                continue
            args = (f.get("derivation") or {}).get("args") or []
            if rec.get("lat") is None and len(args) > 1:
                rec["lat"], rec["lng"] = args[0], args[1]
            rec[f["band"]] = f["value"]
            cids[f["band"]] = f.get("fact_cid")
            if isinstance(f.get("confidence"), (int, float)):
                rec["_conf"] = min(rec.get("_conf", 1.0), f["confidence"])
            scene["fact_count"] += 1
        if entry.get("receipt"):
            receipts.append({"cell": cell, "band": band, "receipt": entry["receipt"]})


def fetch_scene(base, bbox, bands, max_cells, batch=None, pause=0.0):
    print("query_region over", bbox, file=sys.stderr)
    qr = http_post(base, "/v1/query_region",
                   {"bbox": bbox, "bands": [bands[0]], "max_cells": max_cells})
    cells = qr["receipt"]["cells"]
    batch = batch or (128 if len(bands) > 2 else 256)
    scene = {"cells": {}, "fact_count": 0}
    receipts = [{"cell": None, "band": None, "receipt": qr["receipt"]}]
    for band in bands:
        for i in range(0, len(cells), batch):
            print("recall %s %d-%d of %d" % (band, i, min(i + batch, len(cells)),
                                             len(cells)), file=sys.stderr)
            recall_batch(base, scene, receipts, cells[i:i + batch], band)
            # a cold bake against a shared responder is a materialization
            # storm; --sleep spaces the batches so interactive traffic
            # keeps getting scheduled between them
            if pause > 0 and i + batch < len(cells):
                time.sleep(pause)
    return scene, receipts

# ---------------------------------------------------------------------------
# colour presets — mirrors of the template colorize() functions
# ---------------------------------------------------------------------------

def clamp01(v):
    return max(0.0, min(1.0, v))


def colorize_canyon(row, ctx):
    t = (row["copdem30m.elevation_mean"] - ctx["h_min"]) / (ctx["h_max"] - ctx["h_min"] + 1e-9)
    if t < 0.45:
        u = t / 0.45
        return (0.30 + 0.55 * u, 0.07 + 0.20 * u, 0.05 + 0.06 * u)
    u = (t - 0.45) / 0.55
    return (min(1.0, 0.85 + 0.15 * u), 0.27 + 0.58 * u, 0.11 + 0.45 * u)


def colorize_interlaken(row, ctx):
    if (row.get("surface_water.recurrence") or 0) > 40:
        return (0.18, 0.52, 0.95)
    h = (row["copdem30m.elevation_mean"] - ctx["h_min"]) / (ctx["h_max"] - ctx["h_min"] + 1e-9)
    v = clamp01(row.get("indices.ndvi") if row.get("indices.ndvi") is not None else 0.15)
    if h > 0.72:
        s = (h - 0.72) / 0.28
        return (0.62 + 0.38 * s, 0.62 + 0.38 * s, 0.66 + 0.34 * s)
    return (0.34 - 0.14 * v + 0.28 * h, 0.42 + 0.55 * v, 0.20 + 0.12 * h)


def colorize_carbon(row, ctx):
    loss = row.get("forest_change.lossyear") or 0
    tc = row.get("forest_change.treecover2000") or 0
    agb = row.get("esa_cci_biomass.agb_t_per_ha_2022") or 0
    if loss >= 1:
        u = clamp01((loss - 1) / 23.0)
        return (0.45 + 0.55 * u, 0.10 + 0.30 * u, 0.03 + 0.07 * u)
    if tc < 30:
        return (0.52, 0.44, 0.28)
    v = clamp01(agb / 300.0)
    return (0.05, 0.30 + 0.45 * v, 0.10 + 0.10 * v)


def semantic_prepare(rows):
    """PCA of the geotessera vectors -> row['_sem'], as in semantic-world.html."""
    idx = [i for i, r in enumerate(rows)
           if isinstance(r.get("geotessera"), list) and len(r["geotessera"]) >= 8]
    if not idx:
        return
    dim = len(rows[idx[0]]["geotessera"])
    n = len(idx)
    mean = [0.0] * dim
    for i in idx:
        v = rows[i]["geotessera"]
        for j in range(dim):
            mean[j] += v[j] / n
    X = [[rows[i]["geotessera"][j] - mean[j] for j in range(dim)] for i in idx]
    scores = []
    for _ in range(3):
        pc = [1.0 / math.sqrt(dim)] * dim
        for _ in range(60):
            nv = [0.0] * dim
            for x in X:
                dot = sum(x[j] * pc[j] for j in range(dim))
                for j in range(dim):
                    nv[j] += x[j] * dot
            l = math.sqrt(sum(v * v for v in nv)) or 1.0
            pc = [v / l for v in nv]
        s = []
        for x in X:
            d = sum(x[j] * pc[j] for j in range(dim))
            s.append(d)
            for j in range(dim):
                x[j] -= d * pc[j]
        scores.append(s)
    for k in range(3):
        srt = sorted(scores[k])
        lo, hi = srt[int(0.02 * (n - 1))], srt[math.ceil(0.98 * (n - 1))]
        span = (hi - lo) or 1.0
        for pos, i in enumerate(idx):
            t = clamp01((scores[k][pos] - lo) / span)
            rows[i].setdefault("_sem", [0.0, 0.0, 0.0])[k] = t


def colorize_semantic(row, ctx):
    s = row.get("_sem")
    if not s:
        return (0.25, 0.24, 0.22)
    return (0.12 + 0.85 * s[0], 0.12 + 0.85 * s[1], 0.12 + 0.85 * s[2])


TERRAIN_SHAPE = {"gridNormals": True, "gridRelief": True}

PRESETS = {
    "canyon": {
        "bbox": [-112.18, 36.03, -112.02, 36.14],
        "bands": ["copdem30m.elevation_mean"],
        "heightBand": "copdem30m.elevation_mean",
        "heightExaggeration": 1.7,
        "shape": TERRAIN_SHAPE,
        "colorize": colorize_canyon,
    },
    "interlaken": {
        "bbox": [7.62, 46.55, 7.98, 46.76],
        "bands": ["copdem30m.elevation_mean", "indices.ndvi", "surface_water.recurrence"],
        "heightBand": "copdem30m.elevation_mean",
        "heightExaggeration": 1.9,
        "shape": TERRAIN_SHAPE,
        "colorize": colorize_interlaken,
    },
    "semantic": {
        "bbox": [31.00, 29.85, 31.45, 30.15],
        "bands": ["copdem30m.elevation_mean", "geotessera"],
        "heightBand": "copdem30m.elevation_mean",
        "heightExaggeration": 3.0,
        "shape": TERRAIN_SHAPE,
        "prepare": semantic_prepare,
        "colorize": colorize_semantic,
    },
    "carbon": {
        "bbox": [-63.50, -9.50, -63.00, -9.10],
        "bands": ["esa_cci_biomass.agb_t_per_ha_2022",
                  "forest_change.lossyear", "forest_change.treecover2000"],
        "heightBand": "esa_cci_biomass.agb_t_per_ha_2022",
        "heightExaggeration": 15,
        "shape": {"sigmaBand": "esa_cci_biomass.agb_se_t_per_ha_2022", "sigmaFloorM": 4},
        "colorize": colorize_carbon,
    },
}

# ---------------------------------------------------------------------------
# writers
# ---------------------------------------------------------------------------

# 3DGS viewers assume the COLMAP frame (y down, z forward); our world is
# y up. F = diag(1,-1,-1) is a proper rotation (det +1) equal to a half
# turn about x, so positions flip and quaternions premultiply by (0,1,0,0).
AXIS_FLIP = [1, -1, -1]


def flip_quat(q):
    w, x, y, z = q
    return [-x, w, -z, y]      # (0,1,0,0) * (w,x,y,z)


PLY_PROPS = ["x", "y", "z", "nx", "ny", "nz", "f_dc_0", "f_dc_1", "f_dc_2",
             "opacity", "scale_0", "scale_1", "scale_2",
             "rot_0", "rot_1", "rot_2", "rot_3"]


def write_ply(path, gaussians, colors):
    header = ["ply", "format binary_little_endian 1.0",
              "comment emem signed gaussian splats; see the .provenance.json sidecar",
              "element vertex %d" % len(gaussians)]
    header += ["property float " + p for p in PLY_PROPS]
    header.append("end_header")
    with open(path, "wb") as f:
        f.write(("\n".join(header) + "\n").encode())
        for g, c in zip(gaussians, colors):
            o = min(max(g["opacity"], 1e-4), 1 - 1e-4)
            q = flip_quat(g["quat"])
            f.write(struct.pack(
                "<17f",
                g["p"][0], -g["p"][1], -g["p"][2],
                0.0, 0.0, 0.0,
                (c[0] - 0.5) / SH_C0, (c[1] - 0.5) / SH_C0, (c[2] - 0.5) / SH_C0,
                math.log(o / (1 - o)),
                math.log(g["sigma"][0]), math.log(g["sigma"][1]), math.log(g["sigma"][2]),
                q[0], q[1], q[2], q[3]))


def write_splat(path, gaussians, colors):
    order = sorted(range(len(gaussians)), key=lambda i: -(
        gaussians[i]["sigma"][0] * gaussians[i]["sigma"][1] * gaussians[i]["sigma"][2]
        * gaussians[i]["opacity"]))
    with open(path, "wb") as f:
        for i in order:
            g, c = gaussians[i], colors[i]
            q = flip_quat(g["quat"])
            f.write(struct.pack("<3f", g["p"][0], -g["p"][1], -g["p"][2]))
            f.write(struct.pack("<3f", *g["sigma"]))
            f.write(bytes(min(255, max(0, round(v * 255)))
                          for v in (c[0], c[1], c[2], g["opacity"])))
            f.write(bytes(min(255, max(0, round(v * 128 + 128))) for v in q))


def b32(data):
    return base64.b32encode(bytes(data)).decode().lower().rstrip("=")


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()

# ---------------------------------------------------------------------------
# selftest / verify
# ---------------------------------------------------------------------------

def selftest(fixture_path):
    fix = json.load(open(fixture_path))
    tol, checks = 1e-6, 0

    def close(a, b, what):
        nonlocal checks
        if abs(a - b) > tol:
            raise SystemExit("%s: got %r want %r" % (what, a, b))
        checks += 1

    for case in fix["cases"]:
        g = build_gaussians(case.get("scene") or fix["scene"], case["cfg"])
        exp = case["expected"]
        close(g["spacing_m"], exp["spacing_m"], case["name"] + " spacing")
        close(g["h_min"], exp["hMin"], case["name"] + " hMin")
        close(g["h_max"], exp["hMax"], case["name"] + " hMax")
        for i, e in enumerate(exp["gaussians"]):
            got = g["gaussians"][i]
            for k in range(3):
                close(got["p"][k], e["p"][k], "%s g%d p%d" % (case["name"], i, k))
                close(got["sigma"][k], e["sigma"][k], "%s g%d sigma%d" % (case["name"], i, k))
            for k in range(4):
                close(got["quat"][k], e["quat"][k], "%s g%d quat%d" % (case["name"], i, k))
            for k in range(6):
                close(got["cov6"][k], e["cov6"][k], "%s g%d cov%d" % (case["name"], i, k))
    print("selftest ok: %d checks" % checks)


def verify_receipts(base, receipts):
    ok = bad = 0
    for i, item in enumerate(receipts):
        r = dict(item["receipt"])
        if "responder_pubkey_b32" not in r and isinstance(r.get("responder"), list):
            r["responder_pubkey_b32"] = b32(r["responder"])
        if "signature_b32" not in r and isinstance(r.get("signature"), list):
            r["signature_b32"] = b32(r["signature"])
        resp = http_post(base, "/v1/verify_receipt", {"receipt": r})
        if resp.get("valid"):
            ok += 1
        else:
            bad += 1
            print("INVALID receipt %d (%s): %s" %
                  (i, r.get("request_id"), resp.get("errors")), file=sys.stderr)
    print("verify_receipt: %d valid, %d invalid of %d" % (ok, bad, len(receipts)))
    if bad:
        raise SystemExit(1)

# ---------------------------------------------------------------------------

def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--preset", choices=sorted(PRESETS))
    ap.add_argument("--responder", default="https://emem.dev")
    ap.add_argument("--bbox", help="west,south,east,north (overrides the preset)")
    ap.add_argument("--max-cells", type=int, default=1024)
    ap.add_argument("--batch", type=int, help="recall_many batch size")
    ap.add_argument("--sleep", type=float, default=0.0,
                    help="seconds to pause between recall batches (gentle mode "
                         "for cold bakes against a shared responder)")
    ap.add_argument("--out", help="output prefix, e.g. out/rondonia")
    ap.add_argument("--verify", action="store_true",
                    help="round-trip every stored receipt through /v1/verify_receipt")
    ap.add_argument("--selftest", action="store_true",
                    help="assert the gaussian math against test/golden-scene.json")
    args = ap.parse_args()

    if args.selftest:
        import os
        selftest(os.path.join(os.path.dirname(os.path.abspath(__file__)),
                              "test", "golden-scene.json"))
        return
    if not args.preset or not args.out:
        ap.error("--preset and --out are required (or --selftest)")

    cfg = dict(PRESETS[args.preset])
    if args.bbox:
        cfg["bbox"] = [float(v) for v in args.bbox.split(",")]

    shape = cfg.get("shape") or {}
    extra = list(shape.get("reliefBands") or []) + list(shape.get("aspectBands") or [])
    for k in ("slopeBand", "sigmaBand"):
        if shape.get(k):
            extra.append(shape[k])
    bands = list(dict.fromkeys(cfg["bands"] + extra))

    scene, receipts = fetch_scene(args.responder, cfg["bbox"], bands,
                                  args.max_cells, args.batch, args.sleep)
    print("fetched %d cells, %d signed facts" %
          (len(scene["cells"]), scene["fact_count"]), file=sys.stderr)

    built = build_gaussians(scene, cfg)
    if cfg.get("prepare"):
        cfg["prepare"](built["rows"])
    ctx = {"h_min": built["h_min"], "h_max": built["h_max"]}
    colors = [tuple(clamp01(v) for v in cfg["colorize"](r, ctx)) for r in built["rows"]]
    gaussians = built["gaussians"]

    import os
    os.makedirs(os.path.dirname(args.out) or ".", exist_ok=True)
    ply_path, splat_path = args.out + ".ply", args.out + ".splat"
    write_ply(ply_path, gaussians, colors)
    write_splat(splat_path, gaussians, colors)

    # scene.json: drop in as window.EMEM_DATA (the _cids keys ride along
    # harmlessly; splat-math.js only reads lat/lng/band values/_conf)
    scene_path = args.out + ".scene.json"
    with open(scene_path, "w") as f:
        json.dump({"cells": scene["cells"], "fact_count": scene["fact_count"]}, f)

    responder_b32 = None
    for item in receipts:
        r = item["receipt"]
        if isinstance(r.get("responder"), list):
            responder_b32 = b32(r["responder"])
            break

    provenance = {
        "schema": "emem.splat_provenance.v1",
        "responder": args.responder,
        "responder_pubkey_b32": responder_b32,
        "preset": args.preset,
        "bbox": cfg["bbox"],
        "bands": bands,
        "transform": {
            "lat0": built["lat0"], "lng0": built["lng0"], "h_min": built["h_min"],
            "meters_per_unit": 1 / UNITS_PER_M,
            "height_exaggeration": cfg.get("heightExaggeration", 1.6),
            "axis_flip": AXIS_FLIP,
        },
        "artifacts": {
            "ply": {"file": os.path.basename(ply_path),
                    "sha256": sha256_file(ply_path),
                    "bytes": os.path.getsize(ply_path)},
            "splat": {"file": os.path.basename(splat_path),
                      "sha256": sha256_file(splat_path),
                      "bytes": os.path.getsize(splat_path)},
            # scene.json is what a browser actually renders from, so it is
            # bound to the receipts the same way the splat files are: a
            # viewer can hash the fetched bytes and compare before drawing
            "scene": {"file": os.path.basename(scene_path),
                      "sha256": sha256_file(scene_path),
                      "bytes": os.path.getsize(scene_path)},
        },
        "splats": [{"i": i, "cell": built["cell_ids"][i],
                    "fact_cids": built["rows"][i].get("_cids", {})}
                   for i in range(len(gaussians))],
        "receipts": receipts,
        "verify": {
            "receipt": "POST %s/v1/verify_receipt with {\"receipt\": <any entry of "
                       "receipts[].receipt>} -> {valid: true}; pubkey/signature byte "
                       "arrays may be passed as responder_pubkey_b32/signature_b32 "
                       "(lowercase unpadded RFC 4648 base32)" % args.responder,
            "fact": "any fact_cid re-checks at %s/verify/<cid>" % args.responder,
        },
    }
    with open(args.out + ".provenance.json", "w") as f:
        json.dump(provenance, f, indent=1)

    print("wrote %s (.ply %.1f KB, .splat %.1f KB, %d splats)" %
          (args.out, os.path.getsize(ply_path) / 1024,
           os.path.getsize(splat_path) / 1024, len(gaussians)))

    if args.verify:
        verify_receipts(args.responder, receipts)


if __name__ == "__main__":
    main()
