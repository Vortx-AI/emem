"""Still: two signed facts at one address disagree; emem keeps both, says what changed, and reads the newest.

Every value comes from live responses from emem.dev saved on 7 November 2026 by fetch_contradictions.py
(data/contra/). Each receipt is checked offline with the plugin's verify.py before anything is drawn.
The "why" lines restate the code: SUPERSEDED_FN_KEYS and PIXEL_FIX_AT in crates/emem-api-rest/src/lib.rs.
usage: python contradictions.py OUT.webp    (needs Playwright for Python with Chromium and Pillow)
"""
import io
import json
import math
import sys
from pathlib import Path

from PIL import Image
from playwright.sync_api import sync_playwright

HERE = Path(__file__).resolve().parent
D = HERE / "data" / "contra"
sys.path.insert(0, str(HERE.parents[3] / "plugins/emem/skills/emem-verify-receipt/scripts"))
import verify as vr  # noqa: E402
from emem_crypto import blake3, ed25519_verify  # noqa: E402


def load(name):
    return json.loads((D / f"{name}.json").read_text())


def receipt_ok(r):
    v = r["preimage_version"]
    return ed25519_verify(vr._pubkey_bytes(r), vr._sig_bytes(r), blake3(vr.build_preimage(r, v)).digest())


scan = load("scan")
assert receipt_ok(scan["receipt"]) and load("scan_verify")["valid"]
assert scan["scopes"] == ["same_attester_provider_substitution"]
assert "PARTIAL" in scan["agent_hint"] and scan["returned"] == 40

k = {}
for name in ("road", "tree"):
    rec = scan["kept"][name]
    old, new, rc = load(f"{name}_old"), load(f"{name}_new"), load(f"{name}_recall")
    atts = sorted(rec["attestations"], key=lambda a: a["signed_at"])
    assert old["cell"] == new["cell"] == rec["cell"] and old["band"] == new["band"] == rec["band"]
    assert old["tslot"] == new["tslot"] == rec["tslot"]
    assert old["signer"] == new["signer"] and len({a["attester_pubkey_b32"] for a in atts}) == 1
    assert (old["value"], old["signed_at"]) == (atts[0]["value"], atts[0]["signed_at"])
    assert (new["value"], new["signed_at"]) == (atts[-1]["value"], atts[-1]["signed_at"])
    assert receipt_ok(rc["receipt"]) and load(f"{name}_recall_verify")["valid"]
    assert rc["receipt"]["fact_cids"] == [atts[-1]["fact_cid"]] and rc["facts"][0]["value"] == new["value"]
    k[name] = dict(rec=rec, old=old, new=new, atts=atts, rc=rc)

asof = load("road_recall_asof")
try:
    receipt_ok(asof["receipt"])
    raise SystemExit("verify.py now rebuilds as_of receipts: check offline and update the foot line")
except vr.Unsupported:
    pass
assert load("road_recall_asof_verify")["valid"] and load("road_recall_asof_verify")["signature_valid"]
assert asof["receipt"]["as_of"]["transaction_time"] == "2026-09-01T00:00:00Z"
assert asof["receipt"]["fact_cids"] == [k["road"]["atts"][0]["fact_cid"]]

r, t = k["road"], k["tree"]
assert r["old"]["derivation"]["fn_key"] == "overture_road_length_m@1"
assert r["new"]["derivation"]["fn_key"] == "overture_road_length_m@2"
assert "GFC-2024-v1.12" in t["old"]["sources"][0]["id"] and "GFC-2025-v1.13" in t["new"]["sources"][0]["id"]
assert "reader=cog-pixel-floor@2" in t["new"]["derivation"]["args"]
assert t["old"]["signed_at"] < "2026-09-28T04:09:26Z" < t["new"]["signed_at"]
assert all(a["value"] == t["old"]["value"] for a in t["atts"][:-1])

a = r["old"]["derivation"]["args"]
lat_m = (a[1] - a[0]) * 111_320
lon_m = (a[3] - a[2]) * 111_320 * math.cos(math.radians((a[0] + a[1]) / 2))
mid = ((a[0] + a[1]) / 2, (a[2] + a[3]) / 2)
ta = t["old"]["derivation"]["args"]


def day(iso):
    y, m, d = iso[:10].split("-")
    return f"{int(d)} {['Jan','Feb','Mar','Apr','May','Jun','Jul','Aug','Sep','Oct','Nov','Dec'][int(m)-1]} {y}"


def rel(src):
    return src["captured_at"] if "overture" in src["scheme"] else src["id"].split("/")[-2]


served = scan["receipt"]["served_at"]
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
.cards{{display:flex;gap:18px;margin-top:12px;align-items:stretch}}
.card{{background:#0d1117;border:1px solid #30363d;border-radius:10px;padding:14px 18px;flex:1;min-width:0}}
.lab{{color:#8b949e;font-size:22px;margin:0 0 2px}}
.big{{font-size:46px;font-family:"DejaVu Sans Mono",ui-monospace,monospace}}
.val{{font-size:23px;overflow-wrap:anywhere;word-break:keep-all}}
.old{{color:#ffa657}} .new{{color:#79c0ff}} .ok{{color:#7ee787}}
.why{{background:#0d1117;border-left:4px solid #d2a8ff;padding:10px 16px;font-size:24px;margin-top:14px}}
.read{{display:flex;gap:18px;margin-top:12px;font-size:24px}}
.read div{{flex:1;min-width:0}}
.foot{{color:#8b949e;font-size:22px;margin-top:24px;line-height:1.4}}
</style><div class="wrap">
<h1>Two signed facts at one address disagree. emem keeps both.</h1>
<div class="q">One live call to POST /v1/memory_contradictions with limit 40 ({day(served)}, {served[11:16]} UTC)
returned {scan['returned']} records from a partial scan of {scan['corpus_scanned']:,} keys. Each record has one
signer that changed its source or its method. Two of them:</div>

<div class="panel">
<div class="ph">1. Road length in one cell</div>
<div class="sub">Sydney, {abs(mid[0]):.4f} S {mid[1]:.4f} E, a cell of about {lat_m:.0f} m by {lon_m:.0f} m.
Band <span class="mono">overture.transportation.road_length_m</span>, same tslot.</div>
<div class="cards">
<div class="card"><div class="lab">signed {day(r['old']['signed_at'])}</div>
<div class="big old">{r['old']['value']:.2f} m</div>
<div class="lab">method</div><div class="mono val">{r['old']['derivation']['fn_key'].replace('_', '_<wbr>', 1)}</div>
<div class="lab">Overture release</div><div class="mono val">{rel(r['old']['sources'][0])}</div></div>
<div class="card"><div class="lab">signed {day(r['new']['signed_at'])}</div>
<div class="big new">{r['new']['value']:.2f} m</div>
<div class="lab">method</div><div class="mono val">{r['new']['derivation']['fn_key'].replace('_', '_<wbr>', 1)}</div>
<div class="lab">Overture release</div><div class="mono val">{rel(r['new']['sources'][0])}</div></div>
</div>
<div class="why">Why: @1 counted footways, pedestrian areas, rail and waterways as road. @2 counts only roads
a vehicle can use. The release changed too.</div>
<div class="read">
<div><div class="lab">recall today</div><span class="new mono">{r['rc']['facts'][0]['value']:.2f} m</span>,
receipt <span class="ok">valid</span></div>
<div><div class="lab">recall as_of_signed_at 1 Sep 2026</div><span class="old mono">{asof['facts'][0]['value']:.2f} m</span>,
receipt <span class="ok">valid</span></div>
</div></div>

<div class="panel">
<div class="ph">2. Tree cover in the year 2000</div>
<div class="sub">Brazil, {abs(ta[0]):.2f} S {abs(ta[1]):.2f} W, the Hansen pixel under the cell (about 30 m).
Band <span class="mono">forest_change.treecover2000</span>.</div>
<div class="cards">
<div class="card"><div class="lab">signed {len(t['atts']) - 1} times, {day(t['atts'][0]['signed_at'])} to
{day(t['atts'][-2]['signed_at'])}</div>
<div class="big old">{t['old']['value']} %</div>
<div class="lab">release</div><div class="mono val">GFC v1.12</div>
<div class="lab">pixel reader</div><div class="mono val">before the fix</div></div>
<div class="card"><div class="lab">signed {day(t['new']['signed_at'])}</div>
<div class="big new">{t['new']['value']} %</div>
<div class="lab">release</div><div class="mono val">GFC v1.13</div>
<div class="lab">pixel reader</div><div class="mono val">cog-pixel-floor@2</div></div>
</div>
<div class="why">Why: a pixel read signed before 28 Sep 2026 can carry the value of the pixel to its
south-east. The release changed at the same time, so this record alone does not say which change moved
the value.</div>
<div class="read"><div><div class="lab">recall today</div><span class="new mono">{t['rc']['facts'][0]['value']} %</span>,
receipt <span class="ok">valid</span></div></div>
</div>

<div class="foot">The old facts stay in the log and still resolve by fact id. A plain read returns the
newest fact; a read with as_of_signed_at returns what emem held on that date. Every receipt here checks valid at
/v1/verify_receipt. All but the as_of read also check valid offline with verify.py from the emem plugin,
which does not rebuild an as_of receipt.
The scan finds a signer that changed its source or method, and two signers that disagree.
A re-signing with the same source and method is not counted.</div>
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
