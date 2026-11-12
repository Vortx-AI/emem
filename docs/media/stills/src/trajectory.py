"""Still: a trajectory is a list of signed facts, each with its own id, and an empty one is not a zero.

Every value comes from live responses from emem.dev saved on 7 November 2026 by fetch_trajectory.py
(data/trajectory/). The Bengaluru call is the harvester call in docs/robots.md. Each receipt is checked
offline with the plugin's verify.py before anything is drawn.
usage: python trajectory.py OUT.webp    (needs Playwright for Python with Chromium and Pillow)
"""
import datetime as dt
import io
import json
import sys
from pathlib import Path

from PIL import Image
from playwright.sync_api import sync_playwright

HERE = Path(__file__).resolve().parent
D = HERE / "data" / "trajectory"
sys.path.insert(0, str(HERE.parents[3] / "tools/verifiers/emem-verify-receipt"))
import verify as vr  # noqa: E402
from emem_crypto import blake3, ed25519_verify  # noqa: E402

CELL = "defi.zb493.xuqA.zcb5f"
WINDOW = (20325, 20690)
MON = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]


def load(name):
    return json.loads((D / f"{name}.json").read_text())


def receipt_ok(r):
    v = r["preimage_version"]
    return ed25519_verify(vr._pubkey_bytes(r), vr._sig_bytes(r), blake3(vr.build_preimage(r, v)).digest())


def day(n):
    d = dt.date(1970, 1, 1) + dt.timedelta(days=n)
    return f"{d.day} {MON[d.month - 1]} {d.year}"


def iso_day(iso):
    y, m, d = iso[:10].split("-")
    return f"{int(d)} {MON[int(m) - 1]} {y}"


tr, em, pt = load("harvest"), load("empty"), load("harvest_point")
assert receipt_ok(tr["receipt"]) and load("harvest_verify")["valid"]
assert receipt_ok(em["receipt"]) and load("empty_verify")["valid"]
s = tr["series"]
assert tr["receipt"]["cells"] == [CELL]
assert tr["receipt"]["fact_cids"] == [r["fact_cid"] for r in s]
assert all(WINDOW[0] <= r["tslot"] <= WINDOW[1] for r in s)
assert all(r["token"] == f"emem:fact:{CELL}:{r['fact_cid']}" for r in s)
p = next(r for r in s if r["tslot"] == pt["tslot"])
assert (pt["cell"], pt["band"], pt["value"]) == (CELL, "indices.ndvi", p["value"])
scene = pt["derivation"]["args"][2]
assert scene in pt["sources"][0]["id"] and pt["sources"][0]["captured_at"][:10] == p["slot_start"]
assert em["series"] == [] and tr["receipt"]["primitive"] == em["receipt"]["primitive"] == "emem.trajectory"
diag, fill = em["empty_series_diag"], em["fill"]
assert len(diag["stored_tslots"]) == 1 and diag["stored_tslots"][0] > diag["your_window"][1]
assert fill["call"]["path"] == "/v1/backfill"

gaps = [b["tslot"] - a["tslot"] for a, b in zip(s, s[1:])]
lo, hi = min(r["value"] for r in s), max(r["value"] for r in s)
served = tr["receipt"]["served_at"]

W = 800
CW, CH, PL, PR, PT, PB = 712, 360, 58, 14, 16, 58


def x(t):
    return PL + (t - WINDOW[0]) / (WINDOW[1] - WINDOW[0]) * (CW - PL - PR)


def y(v):
    return PT + (1 - v) * (CH - PT - PB)


svg = [f'<svg width="{CW}" height="{CH}" viewBox="0 0 {CW} {CH}" font-family="DejaVu Sans" font-size="22">']
for v in (0.0, 0.2, 0.4, 0.6, 0.8, 1.0):
    svg.append(f'<line x1="{PL}" x2="{CW - PR}" y1="{y(v):.1f}" y2="{y(v):.1f}" stroke="#30363d"/>')
    svg.append(f'<text x="{PL - 8}" y="{y(v) + 7:.1f}" fill="#8b949e" text-anchor="end">{v:.1f}</text>')
for t, lab in ((WINDOW[0], day(WINDOW[0])), (WINDOW[1], day(WINDOW[1]))):
    anchor = "start" if t == WINDOW[0] else "end"
    svg.append(f'<line x1="{x(t):.1f}" x2="{x(t):.1f}" y1="{PT}" y2="{CH - PB}" stroke="#8b949e" stroke-dasharray="4 4"/>')
    svg.append(f'<text x="{x(t):.1f}" y="{CH - PB + 26}" fill="#8b949e" text-anchor="{anchor}">{lab}</text>')
svg.append(f'<text x="{(PL + CW - PR) / 2:.1f}" y="{CH - 6}" fill="#8b949e" text-anchor="middle">window [20325, 20690], days since 1 Jan 1970</text>')
for r in s:
    svg.append(f'<line x1="{x(r["tslot"]):.1f}" x2="{x(r["tslot"]):.1f}" y1="{CH - PB}" y2="{CH - PB - 8}" stroke="#79c0ff"/>')
    svg.append(f'<circle cx="{x(r["tslot"]):.1f}" cy="{y(r["value"]):.1f}" r="6" fill="#79c0ff"/>')
svg.append(f'<circle cx="{x(p["tslot"]):.1f}" cy="{y(p["value"]):.1f}" r="13" fill="none" stroke="#ffa657" stroke-width="3"/>')
svg.append("</svg>")
chart = "".join(svg)

page = f"""<!doctype html><meta charset="utf-8"><style>
body{{margin:0;background:#0d1117;color:#e6edf3;font:32px/1.3 "DejaVu Sans",system-ui,sans-serif;width:{W}px}}
.wrap{{padding:40px}}
h1{{font-size:44px;line-height:1.2;margin:0 0 10px}}
.q{{color:#8b949e;font-size:26px;margin-bottom:8px}}
.mono{{font-family:"DejaVu Sans Mono",ui-monospace,monospace}}
.panel{{background:#161b22;border:1px solid #30363d;border-radius:10px;padding:20px 24px;margin-top:24px}}
.ph{{font-size:28px;margin-bottom:4px}}
.sub{{color:#8b949e;font-size:22px}}
.lab{{color:#8b949e;font-size:22px;margin:10px 0 2px}}
.val{{font-size:23px;overflow-wrap:anywhere}}
.id{{color:#79c0ff}} .tok{{color:#ffa657}} .ok{{color:#7ee787}}
.why{{background:#0d1117;border-left:4px solid #d2a8ff;padding:10px 16px;font-size:24px;margin-top:14px}}
.row{{display:flex;gap:18px}} .row div{{flex:1;min-width:0}}
.big{{font-size:46px;font-family:"DejaVu Sans Mono",ui-monospace,monospace}}
.foot{{color:#8b949e;font-size:22px;margin-top:24px;line-height:1.4}}
</style><div class="wrap">
<h1>A trajectory is a list of signed facts, each with its own id.</h1>
<div class="q">The harvester call in docs/robots.md: NDVI at the Bengaluru city point
(cell <span class="mono">{CELL}</span>), read on {iso_day(served)} at {served[11:16]} UTC.</div>

<div class="panel">
<div class="ph">1. {len(s)} points in a window of {WINDOW[1] - WINDOW[0] + 1} days</div>
<div class="sub">Each dot is one signed NDVI fact. The marks on the axis show its date.
emem holds no value between two dots, so the dots are not joined.</div>
{chart}
<div class="row">
<div><div class="lab">first point</div><div class="val">{iso_day(s[0]['slot_start'])}</div></div>
<div><div class="lab">last point</div><div class="val">{iso_day(s[-1]['slot_start'])}</div></div>
<div><div class="lab">gaps</div><div class="val">{min(gaps)} to {max(gaps)} days</div></div>
</div>
<div class="lab">NDVI from {lo:.2f} to {hi:.2f}. The series does not say why a point drops.
Open the fact to see the scene it came from.</div>
</div>

<div class="panel">
<div class="ph">2. Open one point (ringed above)</div>
<div class="lab">token</div><div class="mono val tok">{p['token']}</div>
<div class="row">
<div><div class="lab">NDVI</div><div class="big">{p['value']:.3f}</div></div>
<div><div class="lab">captured</div><div class="val">{iso_day(pt['sources'][0]['captured_at'])},
{pt['sources'][0]['captured_at'][11:16]} UTC</div>
<div class="lab">signed</div><div class="val">{iso_day(pt['signed_at'])}</div></div>
</div>
<div class="lab">scene</div><div class="mono val id">{scene}</div>
<div class="lab">method</div><div class="mono val">{pt['derivation']['fn_key']}</div>
<div class="lab">receipt</div><div class="val">binds all {len(tr['receipt']['fact_cids'])} fact ids,
<span class="ok">valid</span> at /v1/verify_receipt and offline</div>
</div>

<div class="panel">
<div class="ph">3. An empty series is not a zero</div>
<div class="sub">Central Park, NDVI, {day(diag['your_window'][0])} to {day(diag['your_window'][1])}.
Receipt <span class="ok">valid</span>.</div>
<div class="row">
<div><div class="lab">series</div><div class="big">[ ]</div></div>
<div><div class="lab">stored NDVI facts at this cell</div><div class="val">1, on {day(diag['stored_tslots'][0])},
after the window</div></div>
</div>
<div class="why">"{fill['why']}"</div>
<div class="lab">The response names the next call: <span class="mono">POST {fill['call']['path']}</span>.
A backfill fetches and signs new facts, so this still does not call it.</div>
</div>

<div class="foot">Live read-only calls to emem.dev on {iso_day(served)}: two trajectories, one fact body,
two receipts, each checked twice.
<span style="white-space:nowrap">NDVI = (B08 - B04) / (B08 + B04).</span>
Contains modified Copernicus Sentinel data 2025-2026.</div>
</div>"""

out = Path(sys.argv[1]).resolve()
html_path = out.with_suffix(".html")
html_path.write_text(page)
with sync_playwright() as pw:
    b = pw.chromium.launch()
    pg = b.new_page(viewport={"width": W, "height": 800}, device_scale_factor=1080 / W)
    pg.goto(html_path.as_uri(), wait_until="load")
    png = pg.screenshot(full_page=True)
    b.close()
html_path.unlink()
Image.open(io.BytesIO(png)).convert("RGB").save(out, "WEBP", quality=88, method=6)
print(out, out.stat().st_size, "bytes")
