"""Still: what emem holds at one address. Eight signed facts at one Lake Geneva cell, from six sources and six dates.

Every value comes from data/recall/, live responses from emem.dev saved on 7 November 2026:
  POST /v1/locate {"place":"Lake Geneva"}                                 -> data/recall/locate.json
  POST /v1/recall {"cell":"defi.zb610.tAhU.hejI","bands":[the 8 below]}   -> data/recall/recall.json
  POST /v1/verify_receipt {"receipt": <receipt of the recall>}            -> data/recall/verify_receipt.json
  GET  /v1/scene.png?bbox=6.49,46.437,6.62,46.527&datetime=2026-07-01/2026-10-31&max_cloud=5
                                                                          -> data/recall/scene.headers
  The live scene.png was 997 x 996 px (2.5 MB). data/recall/scene-420.png is that image resized to 420 x 420
  with Lanczos, the size the still draws, so the repo does not carry the full PNG.
usage: python recall_cell.py OUT.webp    (needs Playwright for Python with Chromium, Pillow and pyproj)
"""
import base64
import datetime as dt
import io
import json
import sys
from pathlib import Path

from PIL import Image, ImageDraw
from playwright.sync_api import sync_playwright
from pyproj import Transformer

D = Path(__file__).resolve().parent / "data" / "recall"
loc = json.loads((D / "locate.json").read_text())
rec = json.loads((D / "recall.json").read_text())
vr = json.loads((D / "verify_receipt.json").read_text())
hdr = {}
for line in (D / "scene.headers").read_text().splitlines():
    if line.lower().startswith("x-emem-scene-") and ":" in line:
        k, v = line.split(":", 1)
        hdr[k.strip().lower()] = v.strip()

cell = loc["cell64"]
facts = rec["facts"]
rc = rec["receipt"]
assert len(facts) == 8 and all(f["cell"] == cell for f in facts)
assert rc["fact_cids"] == [f["fact_cid"] for f in facts]
assert vr["valid"] and vr["signature_valid"] and vr["merkle_proof_valid"] and vr["fact_cids_count"] == 8
assert all(f["signer_pubkey_b32"] == vr["signer_pubkey_b32"] for f in facts)

# Display names only. The value, unit, date, source and class on each row come from the fact itself.
NAMES = {
    "copdem30m.elevation_mean": ("Elevation", "Copernicus DEM GLO-30"),
    "esa_worldcover.lc_2021": ("Land cover class", "ESA WorldCover 2021"),
    "surface_water.occurrence": ("Water occurrence", "JRC Global Surface Water"),
    "indices.ndvi": ("NDVI", "Sentinel-2 L2A"),
    "indices.ndwi": ("NDWI", "Sentinel-2 L2A"),
    "koppen": ("Climate class", "Beck et al. Köppen-Geiger"),
    "weather.temperature_2m": ("Air temperature", "MET Norway forecast"),
    "weather.precipitation_mm": ("Precipitation", "MET Norway forecast"),
}
assert set(NAMES) == {f["band"] for f in facts}


def value_text(f):
    v, u = f["value"], f.get("unit")
    if f["band"] == "esa_worldcover.lc_2021":
        return f"{v}: {f['value_decoded']}"
    if f["band"] == "koppen":
        return str(v)
    if f["band"].startswith("indices."):
        return f"{v:.3f}"
    unit = {"degC": " °C", "percent": " %", "m": " m", "mm": " mm"}[u]
    return f"{v:g}{unit}"


def when(f):
    t = dt.datetime.fromisoformat(f["observed_at"].replace("Z", "+00:00"))
    if t.year == 2026 and t.month == 11:
        return t.strftime("%-d %b %Y, %H:%M UTC")
    return t.strftime("%-d %b %Y")


CLASS = {"direct_sensor": "direct sensor", "deterministic_index": "index from sensor bands",
         "model_output": "model or upstream product"}

rows = []
for f in sorted(facts, key=lambda f: f["observed_at"]):
    name, src = NAMES[f["band"]]
    prov = f["band_metadata"]["provenance"]
    recompute = prov["tamper_evidence"] == "recomputable_from_source"
    note = ""
    sc = f.get("surface_class")
    if f["band"] == "indices.ndvi" and sc and sc.get("vegetation_valid") is False:
        note = (f'<div class="note">pixel class: {sc["label"]}, so this is not a vegetation reading '
                f'(<span class="mono">vegetation_valid: false</span>)</div>')
    rows.append(f"""<div class="row"><div class="dot{' rc' if recompute else ''}"></div>
<div class="when">{when(f)}</div>
<div class="body"><div class="top"><span class="name">{name}</span><span class="val">{value_text(f)}</span></div>
<div class="sub"><span class="mono">{f['band']}</span> · {src}</div>
<div class="sub">{CLASS[prov['class']]} · <span class="{'ok' if recompute else 'muted'}">{
        'recomputable from the source' if recompute else 'trust the signer and source'}</span>
· <span class="mono id">{f['fact_cid'][:8]}…</span></div>{note}</div></div>""")

n_recompute = sum(f["band_metadata"]["provenance"]["tamper_evidence"] == "recomputable_from_source" for f in facts)
n_sources = len({NAMES[f["band"]][1] for f in facts})

# Scene crop with the cell centre marked.
img = Image.open(D / "scene-420.png").convert("RGB")
side = img.width
x0, y0, x1, y1 = map(float, hdr["x-emem-scene-bbox-crs"].split(","))
to_utm = Transformer.from_crs("EPSG:4326", f"EPSG:{hdr['x-emem-scene-epsg']}", always_xy=True)
c = loc["centre"]
ex, ny = to_utm.transform(c["lng_deg"], c["lat_deg"])
fx, fy = (ex - x0) / (x1 - x0), (y1 - ny) / (y1 - y0)
assert 0 < fx < 1 and 0 < fy < 1
sx, sy = fx * side, fy * side
dr = ImageDraw.Draw(img)
for r, w, col in ((15, 6, "#0d1117"), (14, 3, "#ffa657")):
    dr.ellipse([sx - r, sy - r, sx + r, sy + r], outline=col, width=w)
buf = io.BytesIO()
img.save(buf, "PNG")
scene_b64 = base64.b64encode(buf.getvalue()).decode()
km = (x1 - x0) / 1000
scene_day = dt.datetime.fromisoformat(hdr["x-emem-scene-datetime"].replace("Z", "+00:00")).strftime("%-d %B %Y")

W = 800  # CSS px; rendered at 1080/W so the text stays readable on a 360 px phone
page = f"""<!doctype html><meta charset="utf-8"><style>
body{{margin:0;background:#0d1117;color:#e6edf3;font:30px/1.3 "DejaVu Sans",system-ui,sans-serif;width:{W}px}}
.wrap{{padding:40px}}
h1{{font-size:44px;line-height:1.2;margin:0 0 10px}}
.q{{color:#8b949e;font-size:26px;margin-bottom:24px}}
.mono{{font-family:"DejaVu Sans Mono",ui-monospace,monospace}}
.head{{display:flex;gap:24px;align-items:stretch}}
.head img{{width:250px;height:250px;flex:none;border-radius:10px;border:1px solid #30363d}}
.panel{{background:#161b22;border:1px solid #30363d;border-radius:10px;padding:20px 24px}}
.lab{{color:#8b949e;font-size:22px;margin:0 0 4px}}
.big{{font-size:32px;margin-bottom:16px}}
.id{{color:#79c0ff}} .sig{{color:#d2a8ff}} .ok{{color:#7ee787}} .muted{{color:#8b949e}}
.rows{{margin-top:24px;position:relative;padding-left:4px}}
.rows:before{{content:"";position:absolute;left:13px;top:20px;bottom:30px;width:3px;background:#30363d}}
.row{{display:flex;gap:16px;margin-bottom:14px;position:relative}}
.dot{{width:20px;height:20px;border-radius:50%;background:#0d1117;border:3px solid #8b949e;margin-top:8px;flex:none;z-index:1}}
.dot.rc{{border-color:#7ee787;background:#7ee787}}
.when{{width:130px;flex:none;color:#8b949e;font-size:22px;padding-top:6px}}
.body{{flex:1;min-width:0;background:#161b22;border:1px solid #30363d;border-radius:10px;padding:10px 18px}}
.top{{display:flex;justify-content:space-between;gap:12px}}
.name{{font-size:28px}} .val{{font-size:30px;font-family:"DejaVu Sans Mono",ui-monospace,monospace;text-align:right}}
.sub{{color:#8b949e;font-size:22px;margin-top:2px}}
.note{{color:#ffa657;font-size:22px;margin-top:4px}}
.foot{{color:#8b949e;font-size:22px;margin-top:24px;line-height:1.4}}
</style><div class="wrap">
<h1>One address holds many signed facts, each with its own source and date</h1>
<div class="q">A recall at the Lake Geneva cell returned {len(facts)} facts from {n_sources} sources, observed between
{when(min(facts, key=lambda f: f['observed_at']))} and {when(max(facts, key=lambda f: f['observed_at'])).split(',')[0]}. One receipt signs all of them.</div>
<div class="head">
<img src="data:image/png;base64,{scene_b64}" alt="">
<div class="panel" style="flex:1;min-width:0">
<div class="lab">place</div><div class="big">Lake Geneva</div>
<div class="lab">cell64 (about 10 m square)</div><div class="big mono id" style="font-size:28px">{cell}</div>
<div class="lab">centre</div><div class="big mono">{c['lat_deg']:.4f}° N<br>{c['lng_deg']:.4f}° E</div>
<div class="lab">receipt over {len(rc['fact_cids'])} fact_cids, by <span class="mono sig">{rc['responder_pubkey_b32'][:8]}…</span></div>
<div class="big mono ok" style="margin-bottom:0">signature valid<br>merkle proof valid</div>
</div></div>
<div class="rows">{''.join(rows)}</div>
<div class="foot">Live calls to emem.dev on 7 November 2026: locate, recall, verify_receipt, scene.png.
Each date is the fact's <span class="mono">observed_at</span>, oldest at the top. A green dot marks the {n_recompute} facts anyone can recompute from the cited source. For the other
{len(facts) - n_recompute}, the fact says to trust the signer and the source. Values are rounded; each fact keeps the full
value. A signature shows who signed. It does not show that the value is true.<br>
The crop is about {km:.0f} km wide (orange ring: the cell), from the Sentinel-2 scene of {scene_day}.
Contains modified Copernicus Sentinel data 2026.</div>
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
print(out, out.stat().st_size, "bytes", f"cell at {fx:.3f},{fy:.3f} of the crop")
