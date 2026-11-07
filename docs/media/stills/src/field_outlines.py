"""Still: field outlines from emem, then one signed NDVI series per field, in a farm box near Ames, Iowa.

Every value comes from the files in data/fields/, which are live responses from emem.dev saved on 7 November 2026:
  POST /v1/field_boundaries {"polygon_bbox":{"min_lat":41.996,"max_lat":42.008,"min_lng":-93.706,"max_lng":-93.688}}
                                                                         -> data/fields/field_boundaries.json
  POST /v1/field_series {"fields":[the 20 outlines over 10,000 m2, id = first 8 chars of polygon_cid],
                         "start_date":"2025-01-01","end_date":"2025-12-31","index":"ndvi"}
                                                                         -> data/fields/field_series.json
  POST /v1/verify_receipt {"receipt": <receipt from field_series>}       -> data/fields/verify_receipt.json
  GET  /v1/scene.png?bbox=-93.7171,41.9846,-93.6861,42.0100&datetime=2025-06-21/2025-06-23&max_cloud=50
                                                                         -> data/fields/scene-2025-06-22.*
usage: python field_outlines.py OUT.webp    (needs Playwright for Python with Chromium, Pillow and pyproj)
"""
import base64
import datetime as dt
import html
import io
import json
import sys
from collections import defaultdict
from pathlib import Path

from PIL import Image
from playwright.sync_api import sync_playwright
from pyproj import Transformer

DATA = Path(__file__).resolve().parent / "data" / "fields"
fb = json.loads((DATA / "field_boundaries.json").read_text())
fs = json.loads((DATA / "field_series.json").read_text())
vr = json.loads((DATA / "verify_receipt.json").read_text())
W = 800
SCENE = "2025-06-22"
EARLY, LATE = "#ffa657", "#79c0ff"

feats = fb["geojson"]["features"]
big = [f for f in feats if f["properties"]["area_m2"] > 10000]
by_id = {f["properties"]["polygon_cid"][:8]: f for f in big}
summ = {s["id"]: s for s in fs["fields"]["summary"]}
assert set(by_id) == set(summ) and len(by_id) == 20
for k, s in summ.items():
    assert s["polygon_cid"] == by_id[k]["properties"]["polygon_cid"], k

cols = fs["fields"]["columns"]
series = defaultdict(list)
for r in fs["fields"]["rows"]:
    row = dict(zip(cols, r))
    if not row["suspect"]:
        series[row["id"]].append(row)


on_scene = {k: next(r["median"] for r in v if r["date"] == SCENE) for k, v in series.items()}
early = {k for k, v in on_scene.items() if v >= 0.8}
assert max(v for k, v in on_scene.items() if k not in early) < 0.75
late = set(series) - early
assert not fs["fields"]["skipped"]

hdr = {}
for line in (DATA / f"scene-{SCENE}.headers").read_text().splitlines():
    if line.lower().startswith("x-emem-scene-"):
        k, v = line.split(":", 1)
        hdr[k.strip().lower()[13:]] = v.strip()
scene_row = next(r for r in fs["rows"] if r["date"] == SCENE)
assert hdr["item-id"] == scene_row["scene"], (hdr["item-id"], scene_row["scene"])
assert vr["valid"] and vr["signature_valid"] and vr["field_bound"]
assert vr["signer_pubkey_b32"] == fs["receipt"]["responder_pubkey_b32"]


def fmt(s):
    d = dt.date.fromisoformat(s)
    return f"{d.day} {d.strftime('%b')} {d.year}"


def scene_svg():
    x0, y0, x1, y1 = map(float, hdr["bbox-crs"].split(","))
    im = Image.open(DATA / f"scene-{SCENE}.png")
    to_utm = Transformer.from_crs(4326, int(hdr["epsg"]), always_xy=True)
    sx, sy = im.width / (x1 - x0), im.height / (y1 - y0)
    g = []
    for k, f in by_id.items():
        pts = []
        for lng, lat in f["geometry"]["coordinates"][0]:
            e, n = to_utm.transform(lng, lat)
            pts.append(f"{(e - x0) * sx:.2f},{(y1 - n) * sy:.2f}")
        c, dash = (EARLY, "") if k in early else (LATE, ' stroke-dasharray="3 2"')
        g.append(f'<polygon points="{" ".join(pts)}" fill="none" stroke="{c}" stroke-width="1.3"{dash}/>')
    buf = io.BytesIO()
    im.save(buf, "PNG")
    b64 = base64.b64encode(buf.getvalue()).decode()
    return (f'<svg viewBox="0 0 {im.width} {im.height}" class="img"><image href="data:image/png;base64,{b64}" '
            f'width="{im.width}" height="{im.height}" style="image-rendering:pixelated"/>{"".join(g)}</svg>')


def chart():
    cw, ch, l, r, t, b = W - 130, 380, 60, 16, 20, 56
    d0, d1 = dt.date(2025, 1, 1), dt.date(2026, 1, 1)

    def X(s):
        return l + (dt.date.fromisoformat(s) - d0).days / (d1 - d0).days * (cw - l - r)

    def Y(v):
        return t + (1 - v) * (ch - t - b)

    g = []
    for v in (0, 0.25, 0.5, 0.75, 1.0):
        g.append(f'<line x1="{l}" x2="{cw - r}" y1="{Y(v):.1f}" y2="{Y(v):.1f}" stroke="#30363d"/>'
                 f'<text x="{l - 12}" y="{Y(v) + 8:.1f}" class="ax" text-anchor="end">{v:g}</text>')
    for m, name in ((1, "Jan"), (4, "Apr"), (7, "Jul"), (10, "Oct")):
        x = X(f"2025-{m:02d}-01")
        g.append(f'<line x1="{x:.1f}" x2="{x:.1f}" y1="{t}" y2="{ch - b}" stroke="#30363d"/>'
                 f'<text x="{x + 6:.1f}" y="{ch - b + 32}" class="ax">{name}</text>')
    xs = X(SCENE)
    g.append(f'<line x1="{xs:.1f}" x2="{xs:.1f}" y1="{t}" y2="{ch - b}" stroke="#e6edf3" stroke-width="2" stroke-dasharray="6 5"/>')
    for k in sorted(late) + sorted(early):
        c, dash = (EARLY, "") if k in early else (LATE, ' stroke-dasharray="9 5"')
        pts = " ".join(f"{X(p['date']):.1f},{Y(p['median']):.1f}" for p in series[k])
        g.append(f'<polyline points="{pts}" fill="none" stroke="{c}" stroke-width="2.4" stroke-opacity="0.85"{dash}/>')
    return f'<svg viewBox="0 0 {cw} {ch}" width="{cw}" height="{ch}">{"".join(g)}</svg>'


n_out = len(feats)
n_suspect = sum(1 for r in fs["fields"]["rows"] if r[cols.index("suspect")])
n_dates = len({r[1] for r in fs["fields"]["rows"]})
ex = next(iter(sorted(early)))
pcid = summ[ex]["polygon_cid"]

page = f"""<!doctype html><meta charset="utf-8"><style>
body{{margin:0;background:#0d1117;color:#e6edf3;font:32px/1.3 "DejaVu Sans",system-ui,sans-serif;width:{W}px}}
.wrap{{padding:40px}}
h1{{font-size:44px;line-height:1.2;margin:0 0 10px}}
.q{{color:#8b949e;font-size:26px;margin-bottom:24px}}
.mono{{font-family:"DejaVu Sans Mono",ui-monospace,monospace}}
.lab{{color:#8b949e;font-size:24px;margin-bottom:8px}}
.img{{width:100%;display:block;border:1px solid #30363d}}
.panel{{background:#161b22;border:1px solid #30363d;border-radius:10px;padding:22px 24px;margin-top:28px}}
.ax{{fill:#8b949e;font:22px "DejaVu Sans",system-ui,sans-serif}}
.key{{color:#8b949e;font-size:23px;margin-top:6px}}
.sw{{display:inline-block;width:34px;height:0;vertical-align:middle;margin-right:8px;border-top:4px solid}}
.cards{{display:flex;flex-direction:column;gap:14px;margin-top:28px}}
.card{{background:#161b22;border:1px solid #30363d;border-radius:10px;padding:14px 20px}}
.card .lab{{font-size:22px;margin:0 0 4px}}
.tok{{color:#ffa657;font:24px/1.35 "DejaVu Sans Mono",ui-monospace,monospace;word-break:break-all}}
.id{{color:#79c0ff;font:24px/1.35 "DejaVu Sans Mono",ui-monospace,monospace;word-break:break-all}}
.ok{{color:#7ee787}} .sig{{color:#d2a8ff}}
.foot{{color:#8b949e;font-size:22px;margin-top:24px;line-height:1.4}}
</style><div class="wrap">
<h1>One signed answer holds an NDVI series for each field outline</h1>
<div class="q">Which fields in this farm box near Ames, Iowa, USA turned green first in 2025?</div>
<div class="lab">Sentinel-2 true colour, {fmt(SCENE)}, with the {len(by_id)} outlines over 1 ha</div>
{scene_svg()}
<div class="panel">
<div class="lab">Median NDVI per field, {n_dates} clear dates in 2025</div>
{chart()}
<div class="key"><span class="sw" style="border-color:{EARLY}"></span>{len(early)} fields: NDVI 0.8 or more on {fmt(SCENE)}</div>
<div class="key"><span class="sw" style="border-color:{LATE};border-top-style:dashed"></span>{len(late)} fields: NDVI under 0.75 on that date</div>
<div class="key"><span class="sw" style="border-color:#e6edf3;border-top-style:dashed"></span>date of the scene above</div>
</div>
<div class="cards">
<div class="card"><div class="lab">1. field_boundaries: {n_out} outlines in the box, {len(by_id)} of them over 1 ha. One of them:</div>
<div class="id">polygon_cid {pcid}</div></div>
<div class="card"><div class="lab">2. field_series records the same polygon_cid for that field. Token for all {len(by_id)} series:</div>
<div class="tok">{html.escape(fs['token'])}</div></div>
<div class="card"><div class="lab">3. verify_receipt</div><div class="mono"><span class="ok">valid</span>,
signer <span class="sig">{fs['receipt']['responder_pubkey_b32'][:8]}…</span></div></div>
</div>
<div class="foot">Live calls to emem.dev on 7 November 2026: field_boundaries, field_series, scene.png, verify_receipt.
Outlines: Fields of The World ({fb['license']}), {fb['vintage_returned'][:4]} vintage, drawn by a model at 10 m.
The series leaves out {n_suspect} suspect rows. A signature shows who computed a series. It does not name the crop.<br>
Contains modified Copernicus Sentinel data 2025.</div>
</div>"""

out = Path(sys.argv[1]).resolve()
html_path = out.with_suffix(".html")
html_path.write_text(page)
with sync_playwright() as p:
    b = p.chromium.launch()
    pg = b.new_page(viewport={"width": W, "height": 800}, device_scale_factor=1080 / W)
    pg.goto(html_path.as_uri(), wait_until="load")
    png = pg.screenshot(full_page=True)
    b.close()
html_path.unlink()
Image.open(io.BytesIO(png)).convert("RGB").save(out, "WEBP", quality=80, method=6)
print(out, out.stat().st_size, "bytes", "early", sorted(early), "late", len(late))
