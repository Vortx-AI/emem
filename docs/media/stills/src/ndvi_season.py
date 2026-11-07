"""Still: NDVI of one farm box near Ames, Iowa, scene by scene, as one signed field_series answer.

Every value comes from the files in data/, which are live responses from emem.dev saved on 7 November 2026:
  POST /v1/field_series  {"bbox":[-93.700,42.000,-93.694,42.004],"start_date":"2024-01-01",
                          "end_date":"2025-12-31","index":"ndvi"}            -> data/field_series.json
  POST /v1/verify_receipt {"receipt": <receipt from field_series>}          -> data/verify_receipt.json
  GET  /v1/scene.png?bbox=-93.706,41.996,-93.688,42.008&datetime=2024-07-31/2024-08-02&max_cloud=50
  GET  /v1/scene.png?bbox=-93.706,41.996,-93.688,42.008&datetime=2024-10-21/2024-10-23&max_cloud=50
                                                     -> data/scene-*.png, response headers in data/scene-*.headers
usage: python ndvi_season.py OUT.webp    (needs Playwright for Python with Chromium, Pillow and pyproj)
"""
import base64
import datetime as dt
import html
import io
import json
import sys
from pathlib import Path

from PIL import Image
from playwright.sync_api import sync_playwright
from pyproj import Transformer

HERE = Path(__file__).resolve().parent
DATA = HERE / "data"
fs = json.loads((DATA / "field_series.json").read_text())
vr = json.loads((DATA / "verify_receipt.json").read_text())
rows = fs["rows"]
box = fs["receipt"]
W = 1080


def headers(name):
    out = {}
    for line in (DATA / f"{name}.headers").read_text().splitlines():
        if line.lower().startswith("x-emem-scene-"):
            k, v = line.split(":", 1)
            out[k.strip().lower()[13:]] = v.strip()
    return out


def scene_panel(name, label, row):
    h = headers(name)
    x0, y0, x1, y1 = map(float, h["bbox-crs"].split(","))
    im = Image.open(DATA / f"{name}.png")
    to_utm = Transformer.from_crs(4326, int(h["epsg"]), always_xy=True)
    ring = [(-93.700, 42.000), (-93.694, 42.000), (-93.694, 42.004), (-93.700, 42.004)]
    pts = []
    for lng, lat in ring:
        e, n = to_utm.transform(lng, lat)
        pts.append(f"{(e - x0) / (x1 - x0) * im.width:.2f},{(y1 - n) / (y1 - y0) * im.height:.2f}")
    buf = io.BytesIO()
    im.save(buf, "PNG")
    b64 = base64.b64encode(buf.getvalue()).decode()
    date = h["datetime"][:10]
    assert date == row["date"] and h["item-id"] == row["scene"], (date, h["item-id"], row)
    return f"""
    <div class="scene">
      <div class="lab">{label}</div>
      <svg viewBox="0 0 {im.width} {im.height}" class="img">
        <image href="data:image/png;base64,{b64}" width="{im.width}" height="{im.height}" style="image-rendering:pixelated"/>
        <polygon points="{' '.join(pts)}" fill="none" stroke="#ffa657" stroke-width="1.6"/>
      </svg>
      <div class="cap"><span class="v">{fmt_date(date)}</span> &nbsp;median NDVI <span class="v">{row['median']:.2f}</span></div>
    </div>"""


def fmt_date(s):
    d = dt.date.fromisoformat(s)
    return f"{d.day} {d.strftime('%b')} {d.year}"


def chart():
    cw, ch, l, r, t, b = 1000, 430, 70, 20, 20, 60
    d0, d1 = dt.date(2024, 1, 1), dt.date(2026, 1, 1)
    span = (d1 - d0).days

    def X(s):
        return l + (dt.date.fromisoformat(s) - d0).days / span * (cw - l - r)

    def Y(v):
        return t + (1 - v) * (ch - t - b)

    g = []
    for v in (0, 0.25, 0.5, 0.75, 1.0):
        g.append(f'<line x1="{l}" x2="{cw - r}" y1="{Y(v):.1f}" y2="{Y(v):.1f}" stroke="#30363d"/>'
                 f'<text x="{l - 12}" y="{Y(v) + 8:.1f}" class="ax" text-anchor="end">{v:g}</text>')
    for y in (2024, 2025):
        for m, name in ((1, "Jan"), (7, "Jul")):
            x = X(f"{y}-{m:02d}-01")
            g.append(f'<line x1="{x:.1f}" x2="{x:.1f}" y1="{t}" y2="{ch - b}" stroke="#30363d"/>'
                     f'<text x="{x + 6:.1f}" y="{ch - b + 32}" class="ax">{name} {y}</text>')
    band_top = " ".join(f"{X(p['date']):.1f},{Y(p['p90']):.1f}" for p in rows)
    band_bot = " ".join(f"{X(p['date']):.1f},{Y(p['p10']):.1f}" for p in reversed(rows))
    g.append(f'<polygon points="{band_top} {band_bot}" fill="#79c0ff" fill-opacity="0.16"/>')
    line = " ".join(f"{X(p['date']):.1f},{Y(p['median']):.1f}" for p in rows)
    g.append(f'<polyline points="{line}" fill="none" stroke="#79c0ff" stroke-width="3"/>')
    for p in rows:
        g.append(f'<circle cx="{X(p["date"]):.1f}" cy="{Y(p["median"]):.1f}" r="7" fill="#0d1117" stroke="#79c0ff" stroke-width="3"/>')
    for e in fs["excluded"]:
        g.append(f'<line x1="{X(e["date"]):.1f}" x2="{X(e["date"]):.1f}" y1="{ch - b - 14}" y2="{ch - b}" stroke="#ff7b72" stroke-width="3"/>')
    for s in ("2024-08-01", "2024-10-22"):
        p = next(q for q in rows if q["date"] == s)
        g.append(f'<circle cx="{X(s):.1f}" cy="{Y(p["median"]):.1f}" r="12" fill="none" stroke="#ffa657" stroke-width="3"/>')
    return f'<svg viewBox="0 0 {cw} {ch}" width="{cw}" height="{ch}">{"".join(g)}</svg>'


peak = next(p for p in rows if p["date"] == "2024-08-01")
bare = next(p for p in rows if p["date"] == "2024-10-22")
cell = box["cells"][0]
cid = fs["derivation_cid"]
assert vr["valid"] and vr["signature_valid"] and vr["signer_pubkey_b32"] == box["responder_pubkey_b32"]
assert fs["token"] == f"emem:fact:{cell}:{cid}"
n_px = rows[0]["n_clear"]
years = sorted({p["date"][:4] for p in rows})

page = f"""<!doctype html><meta charset="utf-8"><style>
body{{margin:0;background:#0d1117;color:#e6edf3;font:32px/1.3 "DejaVu Sans",system-ui,sans-serif;width:{W}px}}
.wrap{{padding:40px}}
h1{{font-size:44px;line-height:1.2;margin:0 0 10px}}
.q{{color:#8b949e;font-size:26px;margin-bottom:28px}}
.mono,.v{{font-family:"DejaVu Sans Mono",ui-monospace,monospace}}
.v{{color:#e6edf3}}
.scenes{{display:flex;gap:40px}}
.scene{{flex:1}}
.lab{{color:#8b949e;font-size:24px;margin-bottom:8px}}
.img{{width:100%;display:block;border:1px solid #30363d}}
.cap{{color:#8b949e;font-size:26px;margin-top:8px}}
.panel{{background:#161b22;border:1px solid #30363d;border-radius:10px;padding:22px 24px;margin-top:30px}}
.ax{{fill:#8b949e;font:22px "DejaVu Sans",system-ui,sans-serif}}
.key{{color:#8b949e;font-size:23px;margin-top:6px}}
.sw{{display:inline-block;width:30px;height:14px;vertical-align:middle;margin-right:8px}}
.cards{{display:flex;gap:20px;margin-top:30px}}
.card{{background:#161b22;border:1px solid #30363d;border-radius:10px;padding:16px 20px;flex:1}}
.card .lab{{font-size:22px;margin:0 0 4px}}
.tok{{color:#ffa657;font:24px/1.35 "DejaVu Sans Mono",ui-monospace,monospace;word-break:break-all}}
.ok{{color:#7ee787}} .sig{{color:#d2a8ff}} .id{{color:#79c0ff}}
.foot{{color:#8b949e;font-size:22px;margin-top:26px;line-height:1.4}}
</style><div class="wrap">
<h1>One signed answer holds two crop seasons of NDVI for this box</h1>
<div class="q">A farm box near Ames, Iowa, USA: {n_px} Sentinel-2 pixels of 10 m. ESA WorldCover 2021 marks its centre cell as cropland.</div>
<div class="scenes">
{scene_panel("scene-2024-08-01", "Sentinel-2 true colour, box in orange", peak)}
{scene_panel("scene-2024-10-22", "The same box, 82 days later", bare)}
</div>
<div class="panel">
{chart()}
<div class="key"><span class="sw" style="background:#79c0ff"></span>median NDVI of the box, one point per clear scene ({len(rows)})
&nbsp; <span class="sw" style="background:#79c0ff;opacity:.3"></span>p10 to p90</div>
<div class="key"><span class="sw" style="background:#ff7b72;height:6px"></span>{len(fs['excluded'])} cloudy scenes left out, each with its reason in the answer</div>
</div>
<div class="cards">
<div class="card" style="flex:2.2"><div class="lab">token for the whole series ({fs['algorithm_key']})</div><div class="tok">{html.escape(fs['token'])}</div></div>
<div class="card"><div class="lab">verify_receipt</div><div class="ok mono">valid</div>
<div class="lab" style="margin-top:10px">signer</div><div class="sig mono">{box['responder_pubkey_b32'][:8]}…</div></div>
</div>
<div class="foot">Live calls to emem.dev on 7 November 2026: field_series, scene.png, verify_receipt. Scene colours are stretched
per image. A signature shows who computed the series. It does not show that the crop is maize or soy.<br>
Contains modified Copernicus Sentinel data {years[0]}-{years[-1]}.</div>
</div>"""

out = Path(sys.argv[1]).resolve()
html_path = out.with_suffix(".html")
html_path.write_text(page)
with sync_playwright() as p:
    b = p.chromium.launch()
    pg = b.new_page(viewport={"width": W, "height": 800}, device_scale_factor=1)
    pg.goto(html_path.as_uri(), wait_until="load")
    png = pg.screenshot(full_page=True)
    b.close()
html_path.unlink()
Image.open(io.BytesIO(png)).convert("RGB").save(out, "WEBP", quality=88, method=6)
print(out, out.stat().st_size, "bytes")
