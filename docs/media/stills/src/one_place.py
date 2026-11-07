"""Still: five ways to name one mountain give four cells; the cell64 or the token is what to hand on.

Every value comes from live responses from emem.dev saved on 7 November 2026 by fetch_one_place.py
(data/names/). Each recall receipt is checked offline with the plugin's verify.py before anything is drawn.
usage: python one_place.py OUT.webp    (needs Playwright for Python with Chromium and Pillow)
"""
import io
import json
import math
import sys
from pathlib import Path

from PIL import Image
from playwright.sync_api import sync_playwright

HERE = Path(__file__).resolve().parent
D = HERE / "data" / "names"
sys.path.insert(0, str(HERE.parents[3] / "plugins/emem/skills/emem-verify-receipt/scripts"))
import verify as vr  # noqa: E402
from emem_crypto import blake3, ed25519_verify  # noqa: E402

BAND = "copdem30m.elevation_mean"


def load(name):
    return json.loads((D / f"{name}.json").read_text())


def receipt_ok(r):
    v = r["preimage_version"]
    return ed25519_verify(vr._pubkey_bytes(r), vr._sig_bytes(r), blake3(vr.build_preimage(r, v)).digest())


def metres(a, b):
    la1, lo1, la2, lo2 = map(math.radians, (a["lat"], a["lng"], b["lat"], b["lng"]))
    h = math.sin((la2 - la1) / 2) ** 2 + math.cos(la1) * math.cos(la2) * math.sin((lo2 - lo1) / 2) ** 2
    return 2 * 6371000 * math.asin(math.sqrt(h))


rows = load("locate")
recalls = load("recalls")
assert load("verify")["valid"] and load("verify")["signature_valid"]
fact = {}
for cell, rc in recalls.items():
    assert receipt_ok(rc["receipt"]), cell
    f = [x for x in rc["facts"] if x["band"] == BAND]
    assert len(f) == 1 and f[0]["cell"] == cell and rc["receipt"]["fact_cids"] == [f[0]["fact_cid"]]
    fact[cell] = f[0]

a, ja, fj, co, sh = rows
assert a["cell64"] == ja["cell64"] and len({r["cell64"] for r in rows}) == 4
assert fj["wikidata"] == "Q39231" and co["source"] == "direct"
assert sh["disambiguation_required"] is True and sh["is_high_confidence"] is False
assert not any(r["disambiguation_required"] for r in rows[:4])
served = recalls[a["cell64"]]["receipt"]["served_at"]
read_day = served[:10]
new_cells = [c for c, f in fact.items() if f["signed_at"][:10] == read_day]
assert set(new_cells) == {fj["cell64"], sh["cell64"]}, new_cells

d_fj = metres(a, fj)
d_co_fj = metres(fj, co)
d_sh = metres(a, sh)
hi = fact[fj["cell64"]]["value"]
lo = fact[a["cell64"]]["value"]

GROUP = {a["cell64"]: "#79c0ff", fj["cell64"]: "#d2a8ff", co["cell64"]: "#7ee787", sh["cell64"]: "#ffa657"}


def picked(r):
    if r["source"] == "direct":
        return "read as latitude, longitude"
    if r["wikidata"]:
        return f"Wikidata {r['wikidata']}, &ldquo;{r['label']}&rdquo;"
    if r["disambiguation_required"]:
        return "a shrine in Fujiyoshida (OpenStreetMap)"
    return "the OpenStreetMap volcano point"


def row(r, note):
    c = r["cell64"]
    f = fact[c]
    return f"""<div class="row" style="border-left-color:{GROUP[c]}">
<div class="pair"><div class="ph mono">&ldquo;{r['phrasing']}&rdquo;</div>
<div class="mono big">{f['value']:,.2f} m</div></div>
<div class="mono val" style="color:{GROUP[c]}">{c}</div>
<div class="lab">picked: {picked(r)}{note}</div>
</div>"""


# local metre frame centred between the three summit points; north up
lat0 = (a["lat"] + fj["lat"]) / 2
lng0 = (a["lng"] + fj["lng"]) / 2


def xy(r):
    x = (r["lng"] - lng0) * 111_320 * math.cos(math.radians(lat0))
    y = (r["lat"] - lat0) * 111_320
    return x, y


SV, HALF = 560, 260  # px box, metres from centre to edge
px = lambda m: SV / 2 + m * (SV / 2) / HALF  # noqa: E731
pts = ""
for r, lab, dx, dy, anchor in (
    (a, "Mount Fuji, mount fuji", 14, 44, "end"),
    (fj, "Fujisan", -26, -14, "end"),
    (co, "35.3606, 138.7274", 24, 34, "start"),
):
    x, y = xy(r)
    cx, cy = px(x), px(-y)
    tx = cx + dx if anchor == "start" else cx + (dx if dx < 0 else -dx)
    mark = (f'<circle cx="{cx:.1f}" cy="{cy:.1f}" r="15" fill="none" stroke-width="5" stroke="{GROUP[r["cell64"]]}"/>'
            if r is fj else f'<circle cx="{cx:.1f}" cy="{cy:.1f}" r="8" fill="{GROUP[r["cell64"]]}"/>')
    pts += (mark +
            f'<text x="{tx:.1f}" y="{cy + dy:.1f}" text-anchor="{anchor}" class="t">{lab}</text>')
sx, sy = xy(sh)
ang = math.atan2(-sy, sx)
ax, ay = SV / 2 + math.cos(ang) * (SV / 2 - 30), SV / 2 + math.sin(ang) * (SV / 2 - 30)
bar = px(-HALF + 40), px(-HALF + 140)
svg = f"""<svg viewBox="0 0 {SV} {SV}" width="100%" xmlns="http://www.w3.org/2000/svg">
<rect x="0" y="0" width="{SV}" height="{SV}" fill="#0d1117" stroke="#30363d"/>
<line x1="{SV/2 + math.cos(ang) * SV * 0.33:.1f}" y1="{SV/2 + math.sin(ang) * SV * 0.33:.1f}" x2="{ax:.1f}" y2="{ay:.1f}" stroke="#ffa657" stroke-width="3" stroke-dasharray="8 8"/>
<polygon points="{ax:.1f},{ay:.1f} {ax-22:.1f},{ay+2:.1f} {ax-8:.1f},{ay+20:.1f}" fill="#ffa657"/>
<text x="16" y="34" class="t" style="fill:#ffa657">&ldquo;Fuji-san, Japan&rdquo; is {d_sh/1000:.1f} km this way</text>
{pts}
<line x1="{bar[0]:.1f}" y1="{SV-30}" x2="{bar[1]:.1f}" y2="{SV-30}" stroke="#e6edf3" stroke-width="3"/>
<text x="{bar[0]:.1f}" y="{SV-42}" class="t">100 m</text>
<text x="{SV-16}" y="34" text-anchor="end" class="t">N up</text>
</svg>"""

W = 800
page = f"""<!doctype html><meta charset="utf-8"><style>
body{{margin:0;background:#0d1117;color:#e6edf3;font:32px/1.3 "DejaVu Sans",system-ui,sans-serif;width:{W}px}}
.wrap{{padding:40px}}
h1{{font-size:44px;line-height:1.2;margin:0 0 10px}}
.q{{color:#8b949e;font-size:26px;margin-bottom:8px}}
.mono{{font-family:"DejaVu Sans Mono",ui-monospace,monospace}}
.panel{{background:#161b22;border:1px solid #30363d;border-radius:10px;padding:20px 24px;margin-top:24px}}
.ph{{font-size:28px;margin-bottom:4px}}
.sub{{color:#8b949e;font-size:22px}}
.row{{background:#0d1117;border:1px solid #30363d;border-left:8px solid;border-radius:10px;padding:12px 18px;margin-top:14px}}
.pair{{display:flex;gap:18px;align-items:baseline;justify-content:space-between}}
.lab{{color:#8b949e;font-size:22px;margin:0 0 2px}}
.big{{font-size:36px}}
.val{{font-size:24px;overflow-wrap:anywhere}}
.tok{{color:#ffa657;font-size:23px;overflow-wrap:anywhere}}
.ok{{color:#7ee787}} .warn{{color:#ff7b72}}
.t{{fill:#e6edf3;font:22px "DejaVu Sans",sans-serif}}
.why{{background:#0d1117;border-left:4px solid #d2a8ff;padding:10px 16px;font-size:24px;margin-top:14px}}
.foot{{color:#8b949e;font-size:22px;margin-top:24px;line-height:1.4}}
</style><div class="wrap">
<h1>Five ways to name one mountain gave four cells.</h1>
<div class="q">Five live calls to POST /v1/locate, then one elevation recall at each cell,
on 7 Nov 2026 at {served[11:16]} UTC. emem gives one place one address. It does not make two
different phrasings agree.</div>

<div class="panel">
<div class="ph">1. What each phrasing resolved to</div>
<div class="sub">Each row: the phrasing, the elevation fact recalled at its cell, the cell64, and what locate picked. Same colour, same cell, same signed fact.</div>
{row(a, "")}
{row(ja, "")}
{row(fj, f", {d_fj:.0f} m from the first point")}
{row(co, f", {d_co_fj:.0f} m from the Wikidata point")}
{row(sh, f", {d_sh/1000:.1f} km away. <span class='warn'>disambiguation_required: true</span>")}
</div>

<div class="panel">
<div class="ph">2. Where the cells are</div>
<div class="sub">Each cell is about 10 m across. The rounded coordinates fell in the cell next to the
Wikidata point. The two summit points are {d_fj:.0f} m apart and {hi - lo:.0f} m apart in height.</div>
{svg}
</div>

<div class="panel">
<div class="ph">3. Hand on the cell64 or the token, not the name</div>
<div class="sub">Any agent that resolves this token reads the same signed bytes:</div>
<div class="mono tok" style="margin-top:8px">{fact[a['cell64']]['memory_token']}</div>
<div class="why">{lo:,.2f} m, Copernicus GLO-30 DEM. Receipt <span class="ok">valid</span>
offline with verify.py and at /v1/verify_receipt. The token names one cell and one fact id, so it
cannot point anywhere else.</div>
</div>

<div class="foot">Before you use a cell from locate, read disambiguation_required and
is_high_confidence in the answer. Two of the four elevation facts were first signed on {int(read_day[8:])} Nov 2026,
when recall fetched them for this image. All four receipts check valid offline.</div>
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
Image.open(io.BytesIO(png)).convert("RGB").save(out, "WEBP", quality=88, method=6)
print(out, out.stat().st_size, "bytes")
