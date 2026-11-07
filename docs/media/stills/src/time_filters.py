"""Still: the two time filters on one real cell. NDVI facts at Mount Fuji, by ground date and by signed date.

Every value comes from data/asof/, live read-only responses from emem.dev saved on 7 November 2026 by
fetch_asof.py: four recalls of indices.ndvi at defi.zb592.nUkO.zEzE (no filter, as_of_tslot, as_of_signed_at,
both) and /v1/verify_receipt on the receipt of the read with both filters. The unfiltered receipt is also
checked offline with the plugin's verify.py; that script refuses receipts with an as_of segment.
usage: python time_filters.py OUT.webp    (needs Playwright for Python with Chromium and Pillow)
"""
import datetime as dt
import io
import json
import sys
from pathlib import Path

from PIL import Image
from playwright.sync_api import sync_playwright

HERE = Path(__file__).resolve().parent
D = HERE / "data" / "asof"
sys.path.insert(0, str(HERE.parents[3] / "plugins/emem/skills/emem-verify-receipt/scripts"))
import verify as vr  # noqa: E402
from emem_crypto import blake3, ed25519_verify  # noqa: E402

CELL = "defi.zb592.nUkO.zEzE"
TSLOT = 20589
SIGNED = "2026-10-01T00:00:00Z"
MON = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]


def load(name):
    return json.loads((D / f"{name}.json").read_text())


def receipt_ok(r):
    v = r["preimage_version"]
    return ed25519_verify(vr._pubkey_bytes(r), vr._sig_bytes(r), blake3(vr.build_preimage(r, v)).digest())


def days(iso):
    t = dt.datetime.fromisoformat(iso.replace("Z", "+00:00"))
    return (t - dt.datetime(1970, 1, 1, tzinfo=dt.timezone.utc)).total_seconds() / 86400


def iso_day(iso):
    y, m, d = iso[:10].split("-")
    return f"{int(d)} {MON[int(m) - 1]} {y}"


def tday(n):
    d = dt.date(1970, 1, 1) + dt.timedelta(days=n)
    return f"{d.day} {MON[d.month - 1]} {d.year}"


none, ts, sg, both, vb = load("none"), load("tslot"), load("signed"), load("both"), load("verify_both")
assert receipt_ok(none["receipt"])
for r in (none, ts, sg, both):
    assert r["receipt"]["cells"] == [CELL]
    assert r["receipt"]["fact_cids"] == [f["fact_cid"] for f in r["facts"]]
    assert all(f["band"] == "indices.ndvi" for f in r["facts"])
assert vb["valid"] and vb["signature_valid"] and vb["as_of_bound"]
assert ts["receipt"]["as_of"] == {"valid_time": TSLOT}
assert sg["receipt"]["as_of"] == {"transaction_time": SIGNED}
assert both["receipt"]["as_of"] == {"valid_time": TSLOT, "transaction_time": SIGNED}
assert "as_of" not in none["receipt"] or none["receipt"]["as_of"] is None

facts = {f["fact_cid"]: f for f in none["facts"]}
assert len(facts) == len({f["tslot"] for f in none["facts"]})
(ft,), (fs,), (fb,) = ts["facts"], sg["facts"], both["facts"]
for f in (ft, fs):
    assert f["fact_cid"] in facts
assert fb["fact_cid"] not in facts and fb["tslot"] == ft["tslot"] == TSLOT
facts[fb["fact_cid"]] = fb

# The filters as the responder applies them (recall.rs: tslot <= as_of_tslot, signed_at <= as_of_signed_at);
# each filtered read returned the newest ground date inside its filters.
def inside(f, t=None, s=None):
    return (t is None or f["tslot"] <= t) and (s is None or f["signed_at"] <= s)


assert ft["tslot"] == max(f["tslot"] for f in facts.values() if inside(f, t=TSLOT))
assert fs["tslot"] == max(f["tslot"] for f in facts.values() if inside(f, s=SIGNED))
assert inside(fb, TSLOT, SIGNED) and not inside(ft, TSLOT, SIGNED)

src_host = {}
for f in (ft, fb):
    host = f["sources"][0]["id"].split("/")[2]
    src_host[f["fact_cid"]] = host.split(".")[0]
assert src_host[ft["fact_cid"]] == "sentinel2l2a01" and src_host[fb["fact_cid"]] == "sentinel-cogs"
assert "S2B" in ft["sources"][0]["id"] and "S2B" in fb["sources"][0]["id"]
assert "54STE" in ft["sources"][0]["id"].replace("T54STE", "54STE") and "54STE" in fb["sources"][0]["id"]

served = both["receipt"]["served_at"]
X0, X1 = days("2025-10-01T00:00:00Z"), days("2026-11-08T00:00:00Z")
Y0, Y1 = days("2026-07-01T00:00:00Z"), days("2026-11-08T00:00:00Z")
CW, CH, PL, PR, PT, PB = 712, 470, 80, 18, 14, 66


def x(d):
    return PL + (d - X0) / (X1 - X0) * (CW - PL - PR)


def y(d):
    return CH - PB - (d - Y0) / (Y1 - Y0) * (CH - PT - PB)


xt, yc = x(TSLOT + 0.999), y(days(SIGNED))
svg = [f'<svg width="{CW}" height="{CH}" viewBox="0 0 {CW} {CH}" font-family="DejaVu Sans" font-size="22">']
svg.append(f'<rect x="{PL}" y="{yc:.1f}" width="{xt - PL:.1f}" height="{CH - PB - yc:.1f}" fill="#7ee787" fill-opacity="0.13"/>')
for lab, iso in (("1 Jul", "2026-07-01"), ("1 Aug", "2026-08-01"), ("1 Sep", "2026-09-01"),
                 ("1 Oct", "2026-10-01"), ("1 Nov", "2026-11-01")):
    yy = y(days(iso + "T00:00:00Z"))
    svg.append(f'<line x1="{PL}" x2="{CW - PR}" y1="{yy:.1f}" y2="{yy:.1f}" stroke="#30363d"/>')
    svg.append(f'<text x="{PL - 8}" y="{yy + 7:.1f}" fill="#8b949e" text-anchor="end">{lab}</text>')
for lab, iso in (("Oct 2025", "2025-10-01"), ("Jan", "2026-01-01"), ("Apr", "2026-04-01"),
                 ("Jul", "2026-07-01"), ("Oct 2026", "2026-10-01")):
    xx = x(days(iso + "T00:00:00Z"))
    svg.append(f'<line x1="{xx:.1f}" x2="{xx:.1f}" y1="{CH - PB}" y2="{CH - PB + 6}" stroke="#8b949e"/>')
    anchor = "start" if lab.startswith("Oct 2025") else ("end" if lab == "Oct 2026" else "middle")
    svg.append(f'<text x="{xx:.1f}" y="{CH - PB + 30}" fill="#8b949e" text-anchor="{anchor}">{lab}</text>')
svg.append(f'<line x1="{PL}" x2="{CW - PR}" y1="{CH - PB}" y2="{CH - PB}" stroke="#8b949e"/>')
svg.append(f'<text x="{(PL + CW - PR) / 2:.1f}" y="{CH - 4}" fill="#8b949e" text-anchor="middle">ground date (tslot)</text>')
svg.append(f'<line x1="{xt:.1f}" x2="{xt:.1f}" y1="{PT}" y2="{CH - PB}" stroke="#ffa657" stroke-width="3" stroke-dasharray="8 5"/>')
svg.append(f'<line x1="{PL}" x2="{CW - PR}" y1="{yc:.1f}" y2="{yc:.1f}" stroke="#d2a8ff" stroke-width="3" stroke-dasharray="8 5"/>')
for f in facts.values():
    svg.append(f'<circle cx="{x(f["tslot"] + 0.5):.1f}" cy="{y(days(f["signed_at"])):.1f}" r="7" fill="#79c0ff"/>')
for f, col in ((ft, "#ffa657"), (fs, "#d2a8ff"), (fb, "#7ee787")):
    svg.append(f'<circle cx="{x(f["tslot"] + 0.5):.1f}" cy="{y(days(f["signed_at"])):.1f}" r="15" fill="none" stroke="{col}" stroke-width="4"/>')
svg.append("</svg>")
chart = "".join(svg)


def row(label, col, f, note):
    return f"""<tr><td><span style="color:{col}">{label}</span></td>
<td>{iso_day(f['observed_at'])}<div class="sub">{note}</div></td><td>{iso_day(f['signed_at'])}</td>
<td class="mono">{f['value']:.3f}</td></tr>"""


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
.lab{{color:#8b949e;font-size:22px;margin:10px 0 2px}}
.val{{font-size:23px;overflow-wrap:anywhere}}
.ok{{color:#7ee787}}
table{{border-collapse:collapse;width:100%;font-size:23px;margin-top:8px}}
th{{color:#8b949e;font-weight:normal;text-align:left;font-size:22px;padding:6px 8px 6px 0}}
td{{border-top:1px solid #30363d;padding:8px 8px 8px 0;vertical-align:top}}
.why{{background:#0d1117;border-left:4px solid #7ee787;padding:10px 16px;font-size:24px;margin-top:14px}}
.foot{{color:#8b949e;font-size:22px;margin-top:24px;line-height:1.4}}
</style><div class="wrap">
<h1>Two time filters on one real cell</h1>
<div class="q">NDVI at the Mount Fuji cell <span class="mono">{CELL}</span>, read on {iso_day(served)}
at {served[11:16]} UTC.</div>

<div class="panel">
<div class="ph">1. {len(facts)} signed NDVI facts, placed by two dates</div>
<div class="sub">Across: the day of the scene. Up: the day emem signed the fact. The orange line is
<span class="mono">as_of_tslot</span> {TSLOT} ({tday(TSLOT)}). The purple line is
<span class="mono">as_of_signed_at</span> 1 Oct 2026. Both filters keep the green area.</div>
{chart}
<div class="lab">A read with no filter returned {len(none['facts'])} facts, one for each ground date.
The 20th fact appeared only in the read with both filters.</div>
</div>

<div class="panel">
<div class="ph">2. Four reads, four answers</div>
<table><tr><th>filter</th><th>ground date</th><th>signed</th><th>NDVI</th></tr>
<tr><td>none</td><td colspan="3">{len(none['facts'])} facts, {iso_day(none['facts'][0]['observed_at'])} to
{iso_day(none['facts'][-1]['observed_at'])}</td></tr>
{row("ground only", "#ffa657", ft, "newest scene on or before 16 May")}
{row("signed only", "#d2a8ff", fs, "newest scene emem held on 1 Oct")}
{row("both", "#7ee787", fb, "what emem knew on 1 Oct about 16 May")}
</table>
<div class="why">16 May 2026 has two facts. Both read the same Sentinel-2B pass over tile 54STE, from two
archive copies: <span class="mono">sentinel-cogs</span> (signed 11 Sep) and
<span class="mono">sentinel2l2a01</span> (signed 7 Nov). They differ by
{abs(ft['value'] - fb['value']):.3f}. A plain read shows only the newer fact. The older one is still stored and signed.</div>
</div>

<div class="panel">
<div class="ph">3. The receipt signs the filters</div>
<div class="val">The receipt of the read with both filters carries
<span class="mono">as_of</span> = {{valid_time: {TSLOT}, transaction_time: "{SIGNED}"}}.
<span class="ok">valid</span> at /v1/verify_receipt, with <span class="mono">as_of_bound: true</span>.
Another node can replay the same read. The plugin's offline verify.py does not yet rebuild the
<span class="mono">as_of</span> segment and refuses such a receipt.</div>
</div>

<div class="foot">Live read-only calls to emem.dev on {iso_day(served)}: four recalls of
<span class="mono">indices.ndvi</span> and one receipt check. The unfiltered receipt was also checked offline.
NDVI is near zero: ESA WorldCover gives this cell class 60, bare or sparse vegetation.
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
