"""Still: what emem can fetch for a new place, and how far back each source goes.

Every row comes from GET /v1/materializers on emem.dev, saved on 7 November 2026 by fetch_wired.py
(data/bands/materializers.json). History windows are the history_available_from_unix / _to_unix fields.
Group names and upstream hosts come from the code: the fetch URLs in crates/emem-api-rest/src/lib.rs
(archive-api, air-quality-api and marine-api.open-meteo.com, power.larc.nasa.gov) and the input pixel size
from band_input_resolution_m. all_materializable_bands() also counts opera_dist.veg_dist_status and
opera_dist.veg_dist_date, which /v1/materializers does not list; the foot line says so.
usage: python wired_bands.py OUT.webp    (needs Playwright for Python with Chromium and Pillow)
"""
import io
import json
import sys
from datetime import datetime, timezone
from pathlib import Path

from PIL import Image
from playwright.sync_api import sync_playwright

HERE = Path(__file__).resolve().parent
M = json.loads((HERE / "data" / "bands" / "materializers.json").read_text())
items = M["items"]
assert len(items) == M["total"] == 113, len(items)
AGENT_CARD_WIRED = 115
CALL_DAY = "7 November 2026"

# (key, label, resolution, match) in the order the rows are drawn; first match wins.
TIMED = [
    ("era5", "ERA5 reanalysis, through Open-Meteo", "11 km", lambda b: b.startswith("era5.")),
    ("koppen", "Köppen climate class", "", lambda b: b == "koppen"),
    ("power", "NASA POWER daily", "55 km", lambda b: b.startswith("power.")),
    ("gsw", "JRC surface water", "30 m", lambda b: b.startswith("surface_water.")),
    ("tmf", "JRC tropical moist forest", "", lambda b: b.startswith("jrc_tmf.")),
    ("terra", "TerraClimate 30-year normals", "", lambda b: b.startswith("terraclimate.")),
    ("modis", "MODIS: NDVI, LST, ET, GPP, LAI, burned area", "250 m to 1 km", lambda b: b.startswith("modis.")),
    ("hansen", "Hansen forest change (two names per layer)", "30 m",
     lambda b: b.startswith("hansen.") or b.startswith("forest_change.")),
    ("pop", "WorldPop population", "1 km", lambda b: b == "population"),
    ("cams", "CAMS air quality, through Open-Meteo", "11 km", lambda b: b.startswith("cams.")),
    ("s1", "Sentinel-1 radar backscatter", "10 m", lambda b: b == "sentinel1_raw"),
    ("s2", "Sentinel-2 bands, indices, NDVI change", "10 m",
     lambda b: b.startswith("s2.") or b.startswith("indices.") or b.startswith("temporal_diff:indices.")),
    ("gfc", "JRC forest map 2020", "", lambda b: b == "jrc_gfc2020.forest_2020"),
    ("wc", "ESA WorldCover 2021", "10 m", lambda b: b == "esa_worldcover.lc_2021"),
    ("marine", "Waves and sea temperature, Open-Meteo", "11 km", lambda b: b.startswith("marine.")),
]
UNTIMED = [
    ("now", "Weather now (met.no forecast)", "2.5 km",
     lambda b: b.startswith("weather.") or b.startswith("temporal_diff:weather.")),
    ("ovt", "Overture buildings, places, roads", "305 m", lambda b: b.startswith("overture.")),
    ("soil", "SoilGrids, top 30 cm", "250 m", lambda b: b.startswith("soilgrids.")),
    ("dem", "Copernicus DEM elevation", "30 m", lambda b: b == "copdem30m.elevation_mean"),
    ("gmrt", "GMRT land and sea floor height", "100 m", lambda b: b == "gmrt.topobathy_mean"),
    ("prot", "Protected areas", "", lambda b: b.startswith("protected")),
    ("firms", "FIRMS active fires, last 24 h", "", lambda b: b == "firms.active_fires"),
]


def year(ts):
    return datetime.fromtimestamp(ts, timezone.utc).year


groups = {k: [] for k, *_ in TIMED + UNTIMED}
for it in items:
    hit = [k for k, _, _, f in TIMED + UNTIMED if f(it["band"])]
    assert hit, it["band"]
    groups[hit[0]].append(it)

timed = []
for k, label, res, _ in TIMED:
    g = groups[k]
    assert g, k
    froms = {year(x["history_available_from_unix"]) for x in g}
    tos = {x["history_available_to_unix"] for x in g}
    assert None not in {x["history_available_from_unix"] for x in g}, k
    assert len(tos) == 1, (k, tos)
    to = tos.pop()
    timed.append(dict(label=label, res=res, n=len(g), y0=min(froms), y1=year(to) if to else None,
                      span=f"{min(froms)}" if len(froms) == 1 else f"{min(froms)} to {max(froms)}"))
untimed = []
for k, label, res, _ in UNTIMED:
    g = groups[k]
    assert g, k
    kinds = {x["temporal_kind"] for x in g}
    assert all(x["history_available_from_unix"] is None for x in g), k
    untimed.append(dict(label=label, res=res, n=len(g), kind=kinds))
assert sum(r["n"] for r in timed) + sum(r["n"] for r in untimed) == len(items)
assert groups["now"] and {x["temporal_kind"] for x in groups["now"]} == {"now_only"}
assert {x["temporal_kind"] for x in groups["ovt"]} == {"per_release"}
n_timed = sum(r["n"] for r in timed)
n_now = sum(r["n"] for r in timed if r["y1"] is None)
oldest = min(r["y0"] for r in timed)
assert oldest == 1940

W = 800
X0, X1 = 1940, 2027
TRACK = 672


def x(y):
    return (y - X0) / (X1 - X0) * TRACK


NOW_X = x(2026 + 310 / 365)

rows = []
for r in timed:
    left = x(r["y0"])
    right = NOW_X if r["y1"] is None else x(r["y1"] + 1)
    cls = "on" if r["y1"] is None else "off"
    when = f"{r['y0']} to now" if r["y1"] is None else (
        f"{r['y0']}" if r["y0"] == r["y1"] else f"{r['y0']} to {r['y1']}")
    meta = f"{r['n']} name{'s' if r['n'] > 1 else ''}" + (f", {r['res']}" if r["res"] else "")
    rows.append(f"""<div class="row"><div class="lt"><span>{r['label']}</span><span class="when {cls}t">{when}</span></div>
<div class="meta">{meta}</div>
<div class="track"><div class="bar {cls}" style="left:{left:.1f}px;width:{max(right - left, 6):.1f}px"></div>
<div class="nowl" style="left:{NOW_X:.1f}px"></div></div></div>""")

ticks = "".join(f'<span style="left:{x(y):.1f}px">{y}</span>' for y in (1940, 1960, 1980, 2000, 2020))
KIND = {"now_only": "current value only", "per_release": "latest release", "static": "one value per place",
        "time_series": "no history window stated"}
urows = "".join(
    f"""<div class="urow"><div><div>{r['label']}</div><div class="meta">{r['n']} name{'s' if r['n'] > 1 else ''}"""
    f"""{', ' + r['res'] if r['res'] else ''}</div></div><div class="kind">{', '.join(KIND[k] for k in sorted(r['kind']))}</div></div>"""
    for r in untimed)

page = f"""<!doctype html><meta charset="utf-8"><style>
body{{margin:0;background:#0d1117;color:#e6edf3;font:26px/1.3 "DejaVu Sans",system-ui,sans-serif;width:{W}px}}
.wrap{{padding:40px}}
h1{{font-size:42px;line-height:1.2;margin:0 0 10px}}
.q{{color:#8b949e;font-size:24px;margin-bottom:8px}}
.mono{{font-family:"DejaVu Sans Mono",ui-monospace,monospace}}
.panel{{background:#161b22;border:1px solid #30363d;border-radius:10px;padding:20px 20px 16px;margin-top:22px}}
.ph{{font-size:28px;margin-bottom:6px}}
.axis{{position:relative;height:30px;width:{TRACK}px;color:#8b949e;font-size:22px;margin-top:6px}}
.axis span{{position:absolute;transform:translateX(-50%)}}
.axis span:first-child{{transform:none}}
.row{{margin-top:14px}}
.lt{{display:flex;justify-content:space-between;gap:12px;font-size:24px}}
.when{{white-space:nowrap;font-family:"DejaVu Sans Mono",ui-monospace,monospace;font-size:22px}}
.ont{{color:#79c0ff}} .offt{{color:#e6edf3}}
.meta{{color:#8b949e;font-size:22px}}
.track{{position:relative;height:16px;width:{TRACK}px;background:#0d1117;border-radius:3px;margin-top:4px}}
.bar{{position:absolute;top:0;height:16px;border-radius:3px}}
.bar.on{{background:#79c0ff}} .bar.off{{background:#8b949e}}
.nowl{{position:absolute;top:-4px;height:24px;border-left:2px dashed #e6edf3}}
.key{{display:flex;gap:24px;flex-wrap:wrap;font-size:22px;color:#8b949e;margin-top:14px}}
.sw{{display:inline-block;width:28px;height:14px;border-radius:3px;margin-right:8px;vertical-align:middle}}
.urow{{display:flex;justify-content:space-between;gap:16px;margin-top:12px;font-size:24px}}
.kind{{color:#8b949e;font-size:22px;text-align:right;white-space:nowrap}}
.foot{{color:#8b949e;font-size:22px;margin-top:22px;line-height:1.4}}
</style><div class="wrap">
<h1>emem fetches {len(items)} band names for a new place. The oldest history starts in {oldest}.</h1>
<div class="q">What can emem read the first time you ask about a cell, and how far back? One live call to
<span class="mono">GET /v1/materializers</span> on {CALL_DAY}, every row grouped by source.</div>

<div class="panel">
<div class="ph">{n_timed} names with a history window</div>
<div class="axis">{ticks}</div>
{''.join(rows)}
<div class="key"><span><span class="sw" style="background:#79c0ff"></span>still updated, to now</span>
<span><span class="sw" style="background:#8b949e"></span>closed window</span>
<span><span style="border-left:2px dashed #e6edf3;margin-right:8px"></span>{CALL_DAY}</span></div>
</div>

<div class="panel">
<div class="ph">{len(items) - n_timed} names with no history window</div>
{urows}
</div>

<div class="foot">Windows are what the responder declares. A read inside a window can still return a signed absence,
for example under cloud or over the sea. Sizes are the input pixels in the code
(<span class="mono">band_input_resolution_m</span>); emem stores each value on its own 10 m cell.
<span class="mono">GET /v1/agent_card</span> counts {AGENT_CARD_WIRED} (<span class="mono">band_taxonomy.materializer_wired.count</span>): two <span class="mono">opera_dist</span> names are wired but missing from this list.</div>
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
Image.open(io.BytesIO(png)).convert("RGB").save(out, "WEBP", quality=82, method=6)
print(out, out.stat().st_size, "bytes", f"timed={n_timed} to_now={n_now}")
