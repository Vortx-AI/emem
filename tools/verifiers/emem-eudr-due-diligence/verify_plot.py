#!/usr/bin/env python3
"""Recompute an EUDR plot verdict from its signed plot fact.

`POST /v1/eudr_dds` with the raster method signs one fact per plot
(`fn_key` `eudr_plot_raster@1`). The fact holds the plot's counts and areas,
the rule it ran, the content id of each source window, and a commitment to
the polygon. This script checks all of that against the bytes:

  1. the fact's CBOR hashes to its id;
  2. your polygon hashes to the fact's `geometry_commitment`;
  3. with --plot-id and --salt, your plot id opens `plot_commitment`;
  4. each source window hashes to its `artifact_cid`;
  5. every pixel count, every area and the verdict come out the same when
     the rule runs again over those windows inside your polygon: each 10 m
     pixel with the fraction of it inside the polygon, times its geodesic
     area, chunk by chunk as the fact lists them.

    python3 verify_plot.py emem:fact:<cell>:<cid> --geometry plot.geojson
    python3 verify_plot.py <cid> --geometry plot.geojson --plot-id FARM-1 --salt <hex>
    python3 verify_plot.py <cid> --geometry plot.geojson --dir ./evidence  # keep the bytes
    python3 verify_plot.py --self-test

The geometry file is a GeoJSON Polygon, a Feature or FeatureCollection with
one Polygon, or {"bbox": [min_lng, min_lat, max_lng, max_lat]}, exactly as
sent to /v1/eudr_dds. Nothing here reads a key. It makes GET requests to
--origin (default https://emem.dev) for the fact and the windows, unless
--dir already holds them as <cid>.cbor and <artifact_cid>.grid.

BLAKE3 comes from the `blake3` package when it is installed, else from
emem_crypto.py beside this script (plain Python, slow on large windows).
This checks the fact, not the receipt: check the receipt that cites the
fact with tools/verifiers/emem-verify-receipt/verify.py.

Exit 0 when everything matches, 1 on a mismatch, 2 on a usage or fetch error.
"""
from __future__ import annotations

import argparse
import base64
import bisect
import json
import math
import struct
import sys
import urllib.request
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))
try:
    import blake3 as _fast  # type: ignore

    def b3(data: bytes) -> bytes:
        return _fast.blake3(data).digest()

except ImportError:
    try:
        from emem_crypto import blake3 as _slow
    except ImportError:
        sys.stderr.write("emem_crypto.py not found: keep it beside this script\n")
        sys.exit(2)

    def b3(data: bytes) -> bytes:
        return _slow(data).digest()


UA = "emem-verify-plot/1"
PLOT_TAG = b"emem.eudr.plot_commitment.v1\x00"
GEOMETRY_TAG = b"emem.eudr.geometry.v1\x00"
JRC = "jrc_gfc2020.forest_2020"
TREECOVER = "forest_change.treecover2000"
LOSSYEAR = "forest_change.lossyear"
TMF = "jrc_tmf.deforestation_year"
MMU_HA = 0.5


def cid_of(data: bytes) -> str:
    return base64.b32encode(b3(data)).decode().lower().rstrip("=")


# ---- CBOR, enough for a fact's canonical bytes ----------------------------

def cbor(b: bytes, i: int = 0):
    ib = b[i]
    mt, ai = ib >> 5, ib & 31
    i += 1
    if ai < 24:
        n = ai
    elif ai in (24, 25, 26, 27):
        w = 1 << (ai - 24)
        n = int.from_bytes(b[i:i + w], "big")
        i += w
    else:
        raise ValueError("indefinite-length CBOR is not canonical")
    if mt == 0:
        return n, i
    if mt == 1:
        return -1 - n, i
    if mt == 2:
        return b[i:i + n], i + n
    if mt == 3:
        return b[i:i + n].decode("utf-8"), i + n
    if mt in (4, 5):
        out = [] if mt == 4 else {}
        for _ in range(n):
            if mt == 4:
                v, i = cbor(b, i)
                out.append(v)
            else:
                k, i = cbor(b, i)
                v, i = cbor(b, i)
                out[k] = v
        return out, i
    if mt == 6:
        return cbor(b, i)
    if ai == 25:
        return struct.unpack(">e", n.to_bytes(2, "big"))[0], i
    if ai == 26:
        return struct.unpack(">f", n.to_bytes(4, "big"))[0], i
    if ai == 27:
        return struct.unpack(">d", n.to_bytes(8, "big"))[0], i
    return {20: False, 21: True, 22: None}.get(n), i


def unwrap(f):
    if isinstance(f, dict):
        for k in ("Derivative", "derivative"):
            if isinstance(f.get(k), dict):
                return f[k]
    return f


# ---- The grid encoding (emem-codec grid.rs) --------------------------------

def decode_grid(data: bytes):
    if len(data) < 64 or data[:8] != b"EMEMGRD1":
        raise ValueError("not an emem grid")
    w, h = struct.unpack_from("<II", data, 8)
    lat0, lng0, dlat, dlng = struct.unpack_from("<dddd", data, 24)
    if len(data) != 64 + 4 * w * h:
        raise ValueError("grid length disagrees with its header")
    values = struct.unpack_from(f"<{w * h}f", data, 64)
    return {"width": w, "height": h, "lat0": lat0, "lng0": lng0, "dlat": dlat, "dlng": dlng}, values


def index_in(g, lat, lng):
    col = math.floor((lng - (g["lng0"] - g["dlng"] * 0.5)) / g["dlng"] + 1e-6)
    row = math.floor((lat - (g["lat0"] - g["dlat"] * 0.5)) / g["dlat"] + 1e-6)
    if col < 0 or row < 0 or col >= g["width"] or row >= g["height"]:
        return None
    return row * g["width"] + col


# ---- Geometry ----------------------------------------------------------------

def rings_of(geo) -> list:
    if geo.get("type") == "FeatureCollection":
        feats = geo.get("features") or []
        if len(feats) != 1:
            raise ValueError("a FeatureCollection must hold exactly one plot")
        geo = feats[0]
    if geo.get("type") == "Feature":
        geo = geo.get("geometry") or {}
    if isinstance(geo.get("bbox"), list) and geo.get("type") is None:
        mn_ln, mn_la, mx_ln, mx_la = (float(x) for x in geo["bbox"])
        return [[(mn_ln, mn_la), (mx_ln, mn_la), (mx_ln, mx_la), (mn_ln, mx_la), (mn_ln, mn_la)]]
    if geo.get("type") != "Polygon":
        raise ValueError("the raster method counts a Polygon or a bbox; a Point has no plot fact")
    return [[(float(p[0]), float(p[1])) for p in ring] for ring in geo["coordinates"]]


def geometry_commitment(rings) -> str:
    data = bytearray(GEOMETRY_TAG)
    for ring in rings:
        data += struct.pack("<I", len(ring))
        for lng, lat in ring:
            data += struct.pack("<dd", lng, lat)
    return b3(bytes(data)).hex()


def plot_commitment(salt: bytes, plot_id: str) -> str:
    return b3(PLOT_TAG + salt + plot_id.encode("utf-8")).hex()


def row_segments(ring, v0, v1):
    """The ring's edges clipped to the pixel row v0..v1, in pixel units."""
    n = len(ring)
    out = []
    for i in range(n):
        ua, va = ring[i]
        ub, vb = ring[(i + 1) % n]
        if va == vb or max(va, vb) <= v0 or min(va, vb) >= v1:
            continue

        def u_at(v):
            if v == va:
                return ua
            if v == vb:
                return ub
            return ua + (v - va) * (ub - ua) / (vb - va)

        vs = v0 if va < v0 else (v1 if va > v1 else va)
        ve = v0 if vb < v0 else (v1 if vb > v1 else vb)
        out.append((u_at(vs), vs, u_at(ve), ve))
    return out


def min_integral(seg, x):
    u0, v0, u1, v1 = seg
    if u0 <= x and u1 <= x:
        return (u0 + u1) / 2.0 * (v1 - v0)
    if u0 >= x and u1 >= x:
        return x * (v1 - v0)
    vm = v0 + (x - u0) * (v1 - v0) / (u1 - u0)
    if u0 < x:
        return (u0 + x) / 2.0 * (vm - v0) + x * (v1 - vm)
    return x * (vm - v0) + (x + u1) / 2.0 * (v1 - vm)


def left_area(segs, x):
    a = 0.0
    for sg in segs:
        a += min_integral(sg, x)
    return a


def ring_sign(ring):
    n = len(ring)
    a = 0.0
    for i in range(n):
        ua, va = ring[i]
        ub, vb = ring[(i + 1) % n]
        a += (ua + ub) / 2.0 * (vb - va)
    return -1.0 if a < 0.0 else 1.0


COVER_EPS = 1e-9


def holds(owns, lat, lng):
    """Whether a chunk counts the pixel centred here; a null side is open."""
    return ((owns.get("min_lng") is None or lng >= owns["min_lng"])
            and (owns.get("max_lng") is None or lng < owns["max_lng"])
            and (owns.get("min_lat") is None or lat > owns["min_lat"])
            and (owns.get("max_lat") is None or lat <= owns["max_lat"]))


def band_area_m2(lat_a, lat_b, dlng):
    a = 6378137.0
    f = 1.0 / 298.257223563
    e2 = f * (2.0 - f)
    e = math.sqrt(e2)
    b2 = a * a * (1.0 - e2)

    def q(lat):
        s = math.sin(math.radians(lat))
        return s / (1.0 - e2 * s * s) + (1.0 / (2.0 * e)) * math.log((1.0 + e * s) / (1.0 - e * s))

    lo, hi = min(lat_a, lat_b), max(lat_a, lat_b)
    return abs(b2 * math.radians(abs(dlng)) / 2.0 * (q(hi) - q(lo)))


# ---- The rule (eudr_verdict_for in crates/emem-api-rest/src/lib.rs) -------

def verdict_for(jrc, tc, ly, tmf, cutoff, rule):
    """(code, refinement): 1 pass, 2 fail, 3 not_in_scope, 4 indeterminate,
    7 review. Loss after the cut-off is Hansen's or TMF's."""
    jrc_absent = jrc is None
    hansen_forest = tc is not None and tc >= 10 and ly is not None and (ly == 0 or ly > cutoff)
    jrc_forest = jrc is not None and jrc >= 1
    if rule == "jrc_or_hansen":
        forest = jrc_forest or hansen_forest
    else:
        forest = hansen_forest if jrc_absent else jrc_forest
    loss = (ly is not None and ly > cutoff) or (tmf is not None and tmf > cutoff)
    if not forest:
        if jrc is None and tc is None:
            return 4, "no_baseline_input"
        if rule == "jrc_only" and not jrc_absent:
            if loss:
                return 7, "loss_on_jrc_non_forest"
            if hansen_forest:
                return 1, "hansen_tree_cover_jrc_non_forest"
        return 3, None
    hansen_only = jrc_absent and tc is not None
    if not loss:
        return 1, ("jrc_unavailable_hansen_only" if hansen_only else None)
    return 2, ("jrc_unavailable_hansen_only" if hansen_only else "no_driver_refinement_band_off_path")


def whole(v, top):
    if v is None or math.isnan(v) or math.isinf(v) or v < 0 or v > top or v != int(v):
        return None
    return int(v)


def jrc_value(v):
    if v == 0.0:
        return 0
    if v == 1.0:
        return 1
    return None


class Tally:
    def __init__(self):
        self.pixels = 0
        self.m2 = 0.0

    def add(self, a):
        self.pixels += 1
        self.m2 += a


KEYS = ("plot", "pass", "fail", "review", "nis", "indet", "forest", "hansen_tc_jrc_nf",
        "jrc_missing", "f_jrc", "f_nf", "f_missing", "h_after", "t_after", "tmf_only",
        "fail_h", "fail_t", "fail_ht", "rev_h", "rev_t", "rev_ht")
COUNTS = ("both", "border", "jrc_read", "hansen_read", "h_none", "h_zero", "t_none", "t_zero",
          "t_read", "tb", "th", "tt", "tn")


def recount(layers, rings, owns, cutoff, rule):
    """One chunk's counts: each pixel with the fraction of it in the polygon,
    times its geodesic area, in row order as the responder adds them."""
    jg, jv = layers[JRC]
    t = {k: Tally() for k in KEYS}
    n = {k: 0 for k in COUNTS}
    h_years, t_years = {}, {}

    def at(band, lat, lng):
        if band not in layers:
            return None
        g, v = layers[band]
        k = index_in(g, lat, lng)
        if k is None or math.isnan(v[k]):
            return None
        return v[k]

    half = abs(jg["dlat"]) / 2.0
    left = jg["lng0"] - jg["dlng"] * 0.5
    top = jg["lat0"] - jg["dlat"] * 0.5
    uv = [[((lng - left) / jg["dlng"], (lat - top) / jg["dlat"]) for lng, lat in ring] for ring in rings]
    signs = [ring_sign(r) for r in uv]
    for r in range(jg["height"]):
        v0, v1 = float(r), float(r) + 1.0
        segs = [row_segments(ring, v0, v1) for ring in uv]
        if not segs[0]:
            continue
        umin, umax = math.inf, -math.inf
        for sg in segs[0]:
            umin = min(umin, sg[0], sg[2])
            umax = max(umax, sg[0], sg[2])
        c0 = int(max(math.floor(umin), 0.0))
        c1 = min(int(math.ceil(umax)), jg["width"])
        if c1 <= c0:
            continue
        edges = [[left_area(ss, float(k)) for k in range(c0, c1 + 1)] for ss in segs]
        lat = jg["lat0"] + r * jg["dlat"]
        px_area = band_area_m2(lat - half, lat + half, jg["dlng"])
        for c in range(c0, c1):
            k = c - c0
            cover = signs[0] * (edges[0][k + 1] - edges[0][k])
            for hole, sign in zip(edges[1:], signs[1:]):
                cover -= sign * (hole[k + 1] - hole[k])
            cover = min(max(cover, 0.0), 1.0)
            if cover <= COVER_EPS:
                continue
            lng = jg["lng0"] + c * jg["dlng"]
            if not holds(owns, lat, lng):
                continue
            area = cover * px_area
            j = jrc_value(jv[r * jg["width"] + c])
            tc = whole(at(TREECOVER, lat, lng), 100.0)
            ly = whole(at(LOSSYEAR, lat, lng), 2100.0)
            tm = whole(at(TMF, lat, lng), 65535.0)
            code, ref = verdict_for(j, tc, ly, tm, cutoff, rule)
            if code in (2, 7):
                hl = ly is not None and ly > cutoff
                tl = tm is not None and tm > cutoff
                pre = "fail" if code == 2 else "rev"
                t[pre + ("_ht" if hl and tl else ("_t" if tl else "_h"))].add(area)
            t["plot"].add(area)
            t[{1: "pass", 2: "fail", 3: "nis", 7: "review"}.get(code, "indet")].add(area)
            if code in (1, 2) and ref != "hansen_tree_cover_jrc_non_forest":
                t["forest"].add(area)
            if ref == "hansen_tree_cover_jrc_non_forest":
                t["hansen_tc_jrc_nf"].add(area)
            elif ref == "jrc_unavailable_hansen_only":
                t["jrc_missing"].add(area)
            if code == 2:
                t["f_jrc" if (j is not None and j >= 1) else ("f_nf" if j is not None else "f_missing")].add(area)
            n["jrc_read"] += j is not None
            n["hansen_read"] += tc is not None
            n["both"] += j is not None and ly is not None
            n["border"] += tc is not None and 8 <= tc <= 12
            if ly is None:
                n["h_none"] += 1
            elif ly == 0:
                n["h_zero"] += 1
            else:
                h_years.setdefault(ly, Tally()).add(area)
                if ly > cutoff:
                    t["h_after"].add(area)
            if tm is None:
                n["t_none"] += 1
            else:
                if tm == 0:
                    n["t_zero"] += 1
                else:
                    t_years.setdefault(tm, Tally()).add(area)
                    if tm > cutoff:
                        t["t_after"].add(area)
                n["t_read"] += 1
                tl, hl = tm > cutoff, ly is not None and ly > cutoff
                if hl and tl:
                    n["tb"] += 1
                elif hl:
                    n["th"] += 1
                elif tl:
                    n["tt"] += 1
                    t["tmf_only"].add(area)
                else:
                    n["tn"] += 1
    return t, n, h_years, t_years


def absorb(total, part):
    """Adds one chunk's counts to the total, as the responder does, in chunk order."""
    t, n, hy, ty = total
    pt, pn, phy, pty = part
    for k in KEYS:
        t[k].pixels += pt[k].pixels
        t[k].m2 += pt[k].m2
    for k in COUNTS:
        n[k] += pn[k]
    for mine, theirs in ((hy, phy), (ty, pty)):
        for y, tl in theirs.items():
            m = mine.setdefault(y, Tally())
            m.pixels += tl.pixels
            m.m2 += tl.m2


def plot_verdict(t):
    """The plot verdict from the covered areas, then the 0.5 ha floor."""
    total = t["plot"].m2
    if total <= 0.0:
        return 4, 0.0, False
    frac = t["fail"].m2 / total
    if t["nis"].m2 >= total:
        raw = 3
    elif t["indet"].m2 / total > 0.5:
        raw = 4
    elif t["fail"].m2 > 0.0:
        raw = 2
    elif t["review"].m2 > 0.0:
        raw = 7
    else:
        raw = 1
    if raw not in (2, 7):
        return raw, frac, False
    fail_ha, review_ha = t["fail"].m2 / 1e4, t["review"].m2 / 1e4
    any_fail, any_review = t["fail"].pixels > 0, t["review"].pixels > 0
    if any_fail and fail_ha >= MMU_HA:
        return 2, frac, False
    if any_review and fail_ha + review_ha >= MMU_HA:
        return 7, frac, False
    if any_fail or any_review:
        return 6, frac, True
    return 1, frac, False


def ha(m2):
    return math.floor(m2 / 1e4 * 1e6 + 0.5) / 1e6


LABELS = {1: "pass", 2: "fail", 3: "not_in_scope", 4: "indeterminate", 6: "below_mmu", 7: "review"}


# ---- I/O -----------------------------------------------------------------------

def fetch(origin: str, path: str, accept: str) -> bytes:
    req = urllib.request.Request(origin.rstrip("/") + path, headers={"user-agent": UA, "accept": accept})
    with urllib.request.urlopen(req, timeout=120) as r:
        return r.read()


def obtain(name: str, folder, origin: str, path: str, accept: str) -> bytes:
    if folder is not None and (folder / name).exists():
        return (folder / name).read_bytes()
    data = fetch(origin, path, accept)
    if folder is not None:
        folder.mkdir(parents=True, exist_ok=True)
        (folder / name).write_bytes(data)
    return data


class Checks:
    def __init__(self):
        self.failed = 0

    def same(self, what, signed, got, tol=None):
        if tol is None:
            ok = signed == got
        else:
            ok = isinstance(signed, (int, float)) and abs(signed - got) <= tol
        print(f"  {'MATCH   ' if ok else 'MISMATCH'} {what}: signed {signed!r}, recomputed {got!r}")
        self.failed += not ok


def self_test() -> bool:
    ok = True
    ring = [(0.3, 0.2), (9.7, 0.4), (9.9, 9.6), (5.1, 4.3), (0.1, 9.8), (0.3, 0.2)]
    total = 0.0
    for r in range(10):
        segs = row_segments(ring, float(r), float(r) + 1.0)
        for c in range(10):
            total += ring_sign(ring) * (left_area(segs, c + 1.0) - left_area(segs, float(c)))
    shoe = abs(sum(ring[i][0] * ring[(i + 1) % 6][1] - ring[(i + 1) % 6][0] * ring[i][1] for i in range(6)) / 2)
    ok &= abs(total - shoe) < 1e-9
    d = 10.0 / 111320.0
    ok &= abs(band_area_m2(-d / 2, d / 2, d) - 100.0) < 1.0
    ok &= abs(band_area_m2(-90.0, 90.0, 360.0) / 1e6 - 510065621.7) < 1.0
    ok &= verdict_for(0, 60, 2024, 0, 2020, "jrc_only") == (7, "loss_on_jrc_non_forest")
    ok &= verdict_for(0, 60, 2024, 0, 2020, "jrc_or_hansen")[0] == 2
    ok &= verdict_for(None, 60, 0, 0, 2020, "jrc_only") == (1, "jrc_unavailable_hansen_only")
    ok &= verdict_for(1, 60, 0, 2023, 2020, "jrc_only")[0] == 2
    print("self-test", "ok" if ok else "FAILED")
    return ok


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("token", nargs="?", help="emem:fact:<cell>:<cid> or a bare fact cid")
    ap.add_argument("--geometry", help="the plot's GeoJSON, as sent to /v1/eudr_dds")
    ap.add_argument("--origin", default="https://emem.dev")
    ap.add_argument("--dir", help="read and keep the fact and windows here")
    ap.add_argument("--plot-id")
    ap.add_argument("--salt", help="plot_salt from the answer, 32 hex characters")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return 0 if self_test() else 1
    if not a.token or not a.geometry:
        ap.print_usage()
        return 2
    cid = a.token.strip().split(":")[-1]
    folder = Path(a.dir) if a.dir else None
    checks = Checks()
    try:
        body = obtain(f"{cid}.cbor", folder, a.origin, f"/v1/facts/{cid}", "application/cbor")
    except Exception as e:  # noqa: BLE001
        print(f"cannot fetch the fact: {e}")
        return 2
    print(f"fact {cid}")
    checks.same("the CBOR hashes to the fact id", cid, cid_of(body))
    fact = unwrap(cbor(body)[0])
    derivation = fact.get("derivation") or {}
    if derivation.get("fn_key") != "eudr_plot_raster@1":
        print(f"not a plot fact: fn_key {derivation.get('fn_key')!r}")
        return 2
    v = fact["value"]
    rings = rings_of(json.loads(Path(a.geometry).read_text()))
    checks.same("geometry_commitment", v["geometry_commitment"], geometry_commitment(rings))
    if a.plot_id is not None and a.salt:
        checks.same("plot_commitment", v["plot_commitment"],
                    plot_commitment(bytes.fromhex(a.salt), a.plot_id))
    cutoff, rule = v["cutoff_year"], v["forest_rule"]
    total = ({k: Tally() for k in KEYS}, {k: 0 for k in COUNTS}, {}, {})
    for i, chunk in enumerate(v["chunks"]):
        layers = {}
        for band, src in chunk["sources"].items():
            art = src.get("artifact_cid")
            if not art:
                print(f"  NOTE     chunk {i} {band}: no window ({src.get('error')})")
                continue
            try:
                data = obtain(f"{art}.grid", folder, a.origin, f"/v1/artifacts/{art}", "application/octet-stream")
            except Exception as e:  # noqa: BLE001
                print(f"cannot fetch the {band} window {art}: {e}")
                return 2
            checks.same(f"chunk {i} {band} window hashes to its artifact_cid", art, cid_of(data))
            g, vals = decode_grid(data)
            sg = src.get("grid") or {}
            for k in ("width", "height", "lat0", "lng0", "dlat", "dlng"):
                if sg.get(k) != g[k]:
                    checks.same(f"chunk {i} {band} grid {k}", sg.get(k), g[k])
            layers[band] = (g, vals)
        if JRC not in layers:
            print(f"chunk {i} has no JRC window: nothing to count on")
            return 1
        print(f"recounting chunk {i} under forest_rule {rule}, cut-off year {cutoff} ...")
        absorb(total, recount(layers, rings, chunk["owns"], cutoff, rule))
    t, n, h_years, t_years = total
    code, frac, floor = plot_verdict(t)
    tol = 2e-6
    px, ar = v["pixels"], v["area_ha"]
    for k, key in (("in_plot", "plot"), ("pass", "pass"), ("fail", "fail"), ("review", "review"),
                   ("not_in_scope", "nis"), ("indeterminate", "indet"), ("forest_at_cutoff", "forest")):
        checks.same(f"pixels.{k}", px[k], t[key].pixels)
        checks.same(f"area_ha.{k}", ar[k], ha(t[key].m2), tol)
    checks.same("area_ha.hansen_tree_cover_jrc_non_forest", ar["hansen_tree_cover_jrc_non_forest"], ha(t["hansen_tc_jrc_nf"].m2), tol)
    checks.same("area_ha.jrc_unavailable_hansen_only", ar["jrc_unavailable_hansen_only"], ha(t["jrc_missing"].m2), tol)
    for k, key in (("with_both_baselines", "both"), ("borderline_canopy", "border"),
                   ("jrc_read", "jrc_read"), ("hansen_treecover_read", "hansen_read")):
        checks.same(f"pixels.{k}", px[k], n[key])
    fb = v["failing_by_baseline"]
    for k, key in (("jrc_forest", "f_jrc"), ("hansen_only_jrc_non_forest", "f_nf"), ("hansen_only_jrc_missing", "f_missing")):
        checks.same(f"failing_by_baseline.{k}.pixels", fb[k]["pixels"], t[key].pixels)
        checks.same(f"failing_by_baseline.{k}.area_ha", fb[k]["area_ha"], ha(t[key].m2), tol)
    for name, years, after, none, zero in (("hansen_loss", h_years, "h_after", "h_none", "h_zero"),
                                           ("tmf_loss", t_years, "t_after", "t_none", "t_zero")):
        signed = v[name]
        got = [[y, years[y].pixels, ha(years[y].m2)] for y in sorted(years)]
        same_years = len(signed["by_year"]) == len(got) and all(
            s[0] == g[0] and s[1] == g[1] and abs(s[2] - g[2]) <= tol for s, g in zip(signed["by_year"], got))
        checks.same(f"{name}.by_year", "as signed" if same_years else signed["by_year"], "as signed" if same_years else got)
        checks.same(f"{name}.after_cutoff.pixels", signed["after_cutoff"]["pixels"], t[after].pixels)
        checks.same(f"{name}.after_cutoff.area_ha", signed["after_cutoff"]["area_ha"], ha(t[after].m2), tol)
        checks.same(f"{name}.unknown_pixels", signed["unknown_pixels"], n[none])
        checks.same(f"{name}.no_loss_pixels", signed["no_loss_pixels"], n[zero])
    for kind, pre in (("fail", "fail"), ("review", "rev")):
        for k, suf in (("hansen_only", "_h"), ("tmf_only", "_t"), ("both", "_ht")):
            sb = v["loss_seen_by"][kind][k]
            checks.same(f"loss_seen_by.{kind}.{k}.pixels", sb["pixels"], t[pre + suf].pixels)
            checks.same(f"loss_seen_by.{kind}.{k}.area_ha", sb["area_ha"], ha(t[pre + suf].m2), tol)
    tx = v["tmf_cross_check"]
    for k, key in (("pixels_read", "t_read"), ("both", "tb"), ("hansen_only", "th"), ("tmf_only", "tt"), ("neither", "tn")):
        checks.same(f"tmf_cross_check.{k}", tx[k], n[key])
    vd = v["verdict"]
    checks.same("verdict.code", vd["code"], code)
    checks.same("verdict.label", vd["label"], LABELS.get(code))
    checks.same("verdict.fail_fraction", vd["fail_fraction"], frac, 1e-12)
    checks.same("verdict.failing_area_ha", vd["failing_area_ha"], ha(t["fail"].m2), tol)
    checks.same("verdict.review_area_ha", vd["review_area_ha"], ha(t["review"].m2), tol)
    checks.same("verdict.mmu_floor_applied", vd["mmu_floor_applied"], floor)
    print(f"verdict {LABELS.get(code)}: {t['plot'].pixels} pixels, {ha(t['plot'].m2)} ha, "
          f"failing {ha(t['fail'].m2)} ha, review {ha(t['review'].m2)} ha")
    if checks.failed:
        print(f"{checks.failed} check(s) did not match")
        return 1
    print("every check matches")
    return 0


if __name__ == "__main__":
    sys.exit(main())
