"""Still: an area read signs each cell, not the area. Two identical recall_polygon calls over the Mount Fuji summit.

Every value comes from data/polygon/ (live read-only responses from emem.dev on 7 November 2026, see
fetch_polygon.py) and from data/hero/ (the Sentinel-2 scene of the summit and the Mount Fuji cell). Every receipt
is checked offline with the plugin's verify.py; the script asserts each number it draws.
usage: python area_receipts.py OUT.webp    (needs Playwright for Python with Chromium, Pillow and pyproj)
"""
import base64
import copy
import io
import json
import statistics
import sys
from pathlib import Path

from PIL import Image
from playwright.sync_api import sync_playwright
from pyproj import Transformer

HERE = Path(__file__).resolve().parent
D = HERE / "data"
sys.path.insert(0, str(HERE.parents[3] / "plugins/emem/skills/emem-verify-receipt/scripts"))
import verify as vr  # noqa: E402
from emem_crypto import blake3, ed25519_verify  # noqa: E402

BAND = "copdem30m.elevation_mean"
KEY = "777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka"


def load(p):
    return json.loads((D / p).read_text())


def receipt_ok(r):
    return ed25519_verify(vr._pubkey_bytes(r), vr._sig_bytes(r),
                          blake3(vr.build_preimage(r, r["preimage_version"])).digest())


first, second = load("polygon/first.json"), load("polygon/second.json")
for d in (first, second):
    assert d["schema"] == "emem.recall_polygon.v1" and d["cells_sampled"] == 16 and d["cells"] == first["cells"]
    assert d["polygon_bbox"]["source"] == "request_polygon_bbox" and "receipt" not in d
    for c, v in d["by_cell"].items():
        r = v["receipt"]
        assert r["cells"] == [c] and r["responder_pubkey_b32"] == KEY and receipt_ok(r)

t1 = next(iter(first["by_cell"].values()))["receipt"]["served_at"]
t2 = next(iter(second["by_cell"].values()))["receipt"]["served_at"]
assert t1 == "2026-11-07T16:08:38Z" and t2 == "2026-11-07T16:09:28Z"
assert first["facts_returned"] == 0 and not first["converged"] and not first["merged_facts"]
assert all(v["receipt"]["fact_cids"] == [] and v["facts"] == [] for v in first["by_cell"].values())
pend = first["pending"]
assert len(pend) == 16 and {p["cell"] for p in pend} == set(first["cells"])
assert all(p["reason_class"] == "deferred" and "cold-fetch ceiling of 600 per minute" in p["reason"] for p in pend)
assert all("signature" not in json.dumps(p) for p in pend)

assert second["facts_returned"] == 16 and second["converged"] and second["pending"] == []
assert len(second["merged_facts"]) == 16
cells = []
for c, v in second["by_cell"].items():
    (f,) = v["facts"]
    assert f["band"] == BAND and f["cell"] == c and f["kind"] == "primary" and f["unit"] == "m"
    assert len(v["receipt"]["fact_cids"]) == 1
    cells.append((c, f["derivation"]["args"][0], f["derivation"]["args"][1], f["value"], v["receipt"]))
assert [m["value"] for m in second["merged_facts"]] == [x[3] for x in cells]

# What one cell's receipt covers: put another cell's fact id, or its cell, into it and the signature fails.
a, b = cells[0], cells[-1]
swap_fact = copy.deepcopy(a[4])
swap_fact["fact_cids"] = b[4]["fact_cids"]
swap_cell = copy.deepcopy(a[4])
swap_cell["cells"] = [b[0]]
assert not receipt_ok(swap_fact) and not receipt_ok(swap_cell)

vals = [x[3] for x in cells]
mean, lo, hi = statistics.fmean(vals), min(vals), max(vals)
native = second["native_grid_cells_in_polygon"]
assert round(second["area_km2"], 2) == 7.54 and round(native) == 75359
pct = 16 / native * 100
assert abs(second["coverage_fraction"] * 100 - pct) < 1e-9

hdr = {}
for line in (D / "hero/scene.headers").read_text().splitlines():
    if line.lower().startswith("x-emem-scene-"):
        k, v = line.split(":", 1)
        hdr[k.lower()[len("x-emem-scene-"):]] = v.strip()
x0, y0, x1, y1 = map(float, hdr["bbox-crs"].split(","))
tr = Transformer.from_crs(4326, int(hdr["epsg"]), always_xy=True)


def pos(lat, lng):
    ex, ny = tr.transform(lng, lat)
    assert x0 < ex < x1 and y0 < ny < y1
    return (ex - x0) / (x1 - x0) * 100, (y1 - ny) / (y1 - y0) * 100


box = second["polygon_bbox"]
bl, tr_ = pos(box["min_lat"], box["min_lng"]), pos(box["max_lat"], box["max_lng"])
hero = load("hero/resolve.json")["fact"]
hx, hy = pos(*hero["derivation"]["args"][:2])
assert hero["cell"] == "defi.zb592.nUkO.zEzE" and hero["cell"] not in second["cells"]
step_m = tr.transform(cells[1][2], cells[1][1])[0] - tr.transform(cells[0][2], cells[0][1])[0]

img = Image.open(D / "hero/scene.png").convert("RGB")
buf = io.BytesIO()
img.save(buf, "JPEG", quality=90)
scene_uri = "data:image/jpeg;base64," + base64.b64encode(buf.getvalue()).decode()
km = (x1 - x0) / 1000


def col(v):
    t = (v - lo) / (hi - lo)
    return f"hsl({200 - 160 * t:.0f},85%,{62 + 8 * t:.0f}%)"


dots = []
for c, lat, lng, v, _ in cells:
    x, y = pos(lat, lng)
    dots.append(f'<div class="dot" style="left:{x:.2f}%;top:{y:.2f}%;border-color:{col(v)}"></div>'
                f'<div class="v" style="left:{x:.2f}%;top:{y:.2f}%">{v:,.0f}</div>')

W = 800
page = f"""<!doctype html><meta charset="utf-8"><style>
body{{margin:0;background:#0d1117;color:#e6edf3;font:32px/1.3 "DejaVu Sans",system-ui,sans-serif;width:{W}px}}
.wrap{{padding:40px}}
h1{{font-size:44px;line-height:1.2;margin:0 0 10px}}
.q{{color:#8b949e;font-size:26px;margin-bottom:8px}}
.mono{{font-family:"DejaVu Sans Mono",ui-monospace,monospace}}
.panel{{background:#161b22;border:1px solid #30363d;border-radius:10px;padding:20px 24px;margin-top:24px}}
.ph{{font-size:28px;margin-bottom:6px}}
.sub{{color:#8b949e;font-size:22px}}
.val{{font-size:24px}}
.ok{{color:#7ee787}} .bad{{color:#ff7b72}} .wait{{color:#ffa657}}
.pic{{position:relative;margin-top:10px}}
.pic img{{width:100%;display:block;border-radius:6px}}
.box{{position:absolute;border:3px dashed #e6edf3}}
.dot{{position:absolute;width:20px;height:20px;margin:-13px 0 0 -13px;border:3px solid;border-radius:50%;
background:rgba(13,17,23,.55)}}
.v{{position:absolute;transform:translate(-50%,14px);font-size:22px;font-weight:bold;color:#fff;
text-shadow:0 0 4px #000,0 0 4px #000,0 0 3px #000}}
.ring{{position:absolute;width:34px;height:34px;margin:-21px 0 0 -21px;border:4px solid #d2a8ff;border-radius:50%;
box-shadow:0 0 0 2px #0d1117}}
table{{border-collapse:collapse;width:100%;font-size:23px;margin-top:6px}}
td{{border-top:1px solid #30363d;padding:9px 8px 9px 0;vertical-align:top}}
td:last-child{{text-align:right;white-space:nowrap}}
.why{{background:#0d1117;border-left:4px solid #7ee787;padding:10px 16px;font-size:24px;margin-top:14px}}
.foot{{color:#8b949e;font-size:22px;margin-top:24px;line-height:1.4}}
</style><div class="wrap">
<h1>An area read signs each cell, not the area</h1>
<div class="q"><span class="mono">POST /v1/recall_polygon</span>: a box of {second['area_km2']:.2f}&nbsp;km&sup2; on
the Mount Fuji summit, 16 cells, Copernicus DEM elevation. The same call twice, 7 November 2026.</div>

<div class="panel">
<div class="ph">1. {t1[11:19]} UTC: <span class="wait">0 facts</span></div>
<div class="val">The 16 cells were cold. The responder was at its cold-fetch ceiling of 600 per minute, so it
returned 16 <span class="mono">pending</span> entries, reason <span class="mono">deferred</span>, and
<span class="mono">converged: false</span>.</div>
<div class="sub" style="margin-top:8px">Each cell receipt is valid and lists 0 fact ids. The pending entries carry
no signature. An empty answer here does not mean that the place has no data.</div>
</div>

<div class="panel">
<div class="ph">2. {t2[11:19]} UTC, same call: <span class="ok">16 facts, 16 receipts</span></div>
<div class="pic"><img src="{scene_uri}">
<div class="box" style="left:{bl[0]:.2f}%;top:{tr_[1]:.2f}%;width:{tr_[0] - bl[0]:.2f}%;height:{bl[1] - tr_[1]:.2f}%"></div>
{''.join(dots)}<div class="ring" style="left:{hx:.2f}%;top:{hy:.2f}%"></div></div>
<div class="sub" style="margin-top:8px">Dashed: the box. Each dot: one cell, one signed fact, elevation in metres,
about {step_m:,.0f}&nbsp;m apart. Purple ring: the Mount Fuji cell of the other stills, {hero['value']:,.2f}&nbsp;m.
It is not in this sample. Sentinel-2, 12 December 2025, {km:.1f}&nbsp;km across.</div>
</div>

<div class="panel">
<div class="ph">3. What each receipt covers, checked offline</div>
<table>
<tr><td>16 cell receipts, each with 1 cell and 1 fact id</td><td class="ok">16 valid</td></tr>
<tr><td>Receipt of cell <span class="mono">{a[0][-9:]}</span> with the fact id of cell
<span class="mono">{b[0][-9:]}</span></td><td class="bad">invalid</td></tr>
<tr><td>Same receipt with the cell <span class="mono">{b[0][-9:]}</span></td><td class="bad">invalid</td></tr>
<tr><td><span class="mono">merged_facts</span>, all 16 facts in one list</td><td class="sub">no receipt</td></tr>
</table>
</div>

<div class="panel">
<div class="ph">4. The summary is yours, and so is the claim</div>
<div class="val">Mean {mean:,.0f}&nbsp;m, lowest {lo:,.0f}&nbsp;m, highest {hi:,.0f}&nbsp;m. No signature covers
these three numbers.</div>
<div class="why">16 cells are {pct:.3f}&nbsp;% of the {native:,.0f} cells of 10&nbsp;m in the box
(<span class="mono">is_exhaustive: false</span>). Say that it is a sample, and cite the cell facts with the
summary.</div>
</div>

<div class="foot">Live read-only calls to emem.dev on 7 November 2026 at {t1[11:16]} and {t2[11:16]} UTC. All 32
receipts were checked offline with the key <span class="mono">{KEY[:8]}</span>. Elevation from the Copernicus
GLO-30 DEM. Contains modified Copernicus Sentinel data 2025.</div>
</div>"""

out = Path(sys.argv[1]).resolve()
html_path = out.with_suffix(".html")
html_path.write_text(page)
with sync_playwright() as pw:
    br = pw.chromium.launch()
    pg = br.new_page(viewport={"width": W, "height": 800}, device_scale_factor=1080 / W)
    pg.goto(html_path.as_uri(), wait_until="load")
    png = pg.screenshot(full_page=True)
    br.close()
html_path.unlink()
Image.open(io.BytesIO(png)).convert("RGB").save(out, "WEBP", quality=72, method=6)
print(out, out.stat().st_size, "bytes; mean", round(mean, 2), "lo", lo, "hi", hi, "step", round(step_m))
