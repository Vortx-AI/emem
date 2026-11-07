"""Still: one signed change ledger at Cubbon Park, Bengaluru. Evidence per term, and no numeric split.

Every value comes from live reads of emem.dev on 7 November 2026, saved by fetch_change.py:
  POST /v1/memory_token/resolve  the stored change_attribution.ledger fact -> data/change/ledger.json
  POST /v1/memory_token/resolve  the two NDVI facts the ledger cites        -> data/change/ndvi_{now,prev}.json
  POST /v1/verify_receipt        the resolve receipt                         -> data/change/ledger_verify.json
The resolve receipt is also checked offline with the plugin's verify.py.
usage: python change_ledger.py OUT.webp    (needs Playwright for Python with Chromium and Pillow)
"""
import io
import json
import re
import sys
from datetime import datetime
from pathlib import Path

from PIL import Image
from playwright.sync_api import sync_playwright

HERE = Path(__file__).resolve().parent
D = HERE / "data" / "change"
sys.path.insert(0, str(HERE.parents[3] / "plugins/emem/skills/emem-verify-receipt/scripts"))
import verify as vr  # noqa: E402
from emem_crypto import blake3, ed25519_verify  # noqa: E402


def load(name):
    return json.loads((D / name).read_text())


led = load("ledger.json")
now, prev, chk = load("ndvi_now.json"), load("ndvi_prev.json"), load("ledger_verify.json")
rc = led["receipt"]
assert ed25519_verify(vr._pubkey_bytes(rc), vr._sig_bytes(rc), blake3(vr.build_preimage(rc, 2)).digest())
assert chk["valid"] and chk["signature_valid"] and chk["merkle_proof_valid"]
assert led["resolved"] and led["cell_matches"] and led["band"] == "change_attribution.ledger"

L = led["value"]
T = L["terms"]
assert L["split"] is None and L["schema"] == "emem.change_attribution.v1"
assert not T["env"]["estimated"] and not T["sensor"]["estimated"] and not T["geo"]["estimated"]
assert T["encoder"]["changed"] is False and T["noise"]["class_changed"] is False
assert T["noise"]["class_now"] == T["noise"]["class_prev"] == 4.0  # Sentinel-2 SCL 4 = vegetation
ev = {e["band"]: e for e in T["env"]["evidence"]}
nd = ev["indices.ndvi"]
assert nd["fact_cids"] == [now["fact_cid"], prev["fact_cid"]]
assert now["value"] == nd["value_now"] and prev["value"] == nd["value_prev"]
assert all(not r["same_observation_path"] for r in T["sensor"]["records"])
lat, lng = now["fact"]["derivation"]["args"][:2]


def scene(fact):
    m = re.match(r"(S2[ABC])_MSIL2A_(\d{8})T\d+_R(\d+)_T(\w{5})_", fact["fact"]["derivation"]["args"][2])
    sat, day, orbit, tile = m.groups()
    return "Sentinel-" + sat[1:], datetime.strptime(day, "%Y%m%d"), tile, orbit


sat_now, day_now, tile, orb = scene(now)
sat_prev, day_prev, tile_p, orb_p = scene(prev)
assert (tile, orb) == (tile_p, orb_p) and sat_now != sat_prev


def d(dt):
    return f"{dt.day} {dt:%b %Y}"


def num(x):
    return f"{x:.3f}".replace("-", "−")


def sign(x):
    return ("+" if x >= 0 else "−") + f"{abs(x):.3f}"


signed = datetime.strptime(led["fact"]["signed_at"], "%Y-%m-%dT%H:%M:%SZ")
served = datetime.strptime(chk["served_at"], "%Y-%m-%dT%H:%M:%SZ")
emb = L["observed"]["embedding"]
cid = led["fact_cid"]

# NDVI scale for the two dots: 0.70 to 0.90 across the bar
lo, hi = 0.70, 0.90


def x(v):
    return 100 * (v - lo) / (hi - lo)


rows = []
for band, short in (("indices.ndvi", "NDVI"), ("indices.nbr", "NBR"), ("indices.ndwi", "NDWI")):
    e = ev[band]
    rows.append(
        f'<tr><td>{short}</td><td class="v">{num(e["value_prev"])}</td><td class="v">{num(e["value_now"])}</td>'
        f'<td class="v up">{sign(e["delta"])}</td></tr>'
    )


def term(name, sym, chip, cls, text):
    return (
        f'<div class="term"><div class="th"><span class="tn">{name}</span> <span class="sym">{sym}</span>'
        f'<span class="chip {cls}">{chip}</span></div><div class="tx">{text}</div></div>'
    )


terms = "".join(
    [
        term("World", "Δ_env", "evidence", "c-ev",
             f"Three index pairs from the same two visits, each with both fact ids. Raw deltas, not weighed."),
        term("Sensor", "Δ_sensor", "recorded", "c-rec",
             f"The path changed: {sat_prev} on {d(day_prev)}, {sat_now} on {d(day_now)}. "
             f"Same tile {tile}, same orbit {int(orb)}. Its share is not computed."),
        term("Geometry", "Δ_geo", "not estimated", "c-no",
             "This build has no measure of registration error. The ledger names the term and does not fold it into World."),
        term("Encoder", "Δ_encoder", "pinned", "c-pin",
             f"Both embedding years are slices of one signed fact under one recipe "
             f'(<span class="m">{T["encoder"]["pinned_by"]["fn_key"]}</span>). Changed: no.'),
        term("Noise", "ε", "no class flip", "c-ev",
             "Sentinel-2 scene class 4 (vegetation) on both visits. No switch to cloud or shadow."),
    ]
)

W = 800
page = f"""<!doctype html><meta charset="utf-8"><style>
body{{margin:0;background:#0d1117;color:#e6edf3;font:30px/1.3 "DejaVu Sans",system-ui,sans-serif;width:{W}px}}
.wrap{{padding:40px}}
h1{{font-size:42px;line-height:1.2;margin:0 0 12px}}
.q{{color:#8b949e;font-size:24px;line-height:1.35}}
.m,.v{{font-family:"DejaVu Sans Mono",ui-monospace,monospace}}
.panel{{background:#161b22;border:1px solid #30363d;border-radius:10px;padding:20px 24px;margin-top:24px}}
.ph{{font-size:28px;margin-bottom:12px}}
.lab{{color:#8b949e;font-size:22px}}
.bar{{position:relative;height:120px;margin:6px 10px 0}}
.axis{{position:absolute;left:0;right:0;top:58px;border-top:2px solid #30363d}}
.tick{{position:absolute;top:66px;color:#8b949e;font-size:20px;transform:translateX(-50%)}}
.dot{{position:absolute;top:44px;width:28px;height:28px;border-radius:50%;transform:translateX(-50%)}}
.dl{{position:absolute;top:0;font-size:24px;transform:translateX(-50%);white-space:nowrap}}
.arrow{{position:absolute;top:57px;height:4px;background:#7ee787}}
table{{border-collapse:collapse;width:100%;font-size:26px;margin-top:8px}}
td,th{{padding:6px 8px;text-align:right}} td:first-child,th:first-child{{text-align:left}}
th{{color:#8b949e;font-weight:normal;font-size:22px}}
.up{{color:#7ee787}}
.term{{border-top:1px solid #30363d;padding:12px 0}}
.term:first-of-type{{border-top:none}}
.th{{display:flex;align-items:baseline;gap:10px}}
.tn{{font-size:28px}} .sym{{color:#8b949e;font-size:24px;font-family:"DejaVu Sans Mono",monospace}}
.chip{{margin-left:auto;font-size:22px;padding:2px 12px;border-radius:14px;border:1px solid}}
.c-ev{{color:#7ee787;border-color:#2ea043}} .c-rec{{color:#79c0ff;border-color:#1f6feb}}
.c-no{{color:#ffa657;border-color:#9e6a03}} .c-pin{{color:#d2a8ff;border-color:#8957e5}}
.tx{{font-size:24px;color:#c9d1d9;margin-top:4px;line-height:1.35}}
.split{{font-size:44px;font-family:"DejaVu Sans Mono",monospace;color:#ffa657}}
.ok{{color:#7ee787}}
.tok{{font-size:22px;overflow-wrap:anywhere;color:#79c0ff}}
.foot{{color:#8b949e;font-size:21px;margin-top:22px;line-height:1.4}}
</style><div class="wrap">
<h1>NDVI went up at one cell. The signed ledger lists the evidence for each cause. It does not split the change.</h1>
<div class="q">Cubbon Park, Bengaluru: cell <span class="m">{led['cell']}</span> at {lat:.4f} N, {lng:.4f} E.
The ledger is a stored fact, signed {d(signed)} at {signed:%H:%M} UTC. Resolved and checked live on
{d(served)} at {served:%H:%M} UTC.</div>

<div class="panel">
<div class="ph">1. What moved</div>
<div class="bar"><div class="axis"></div>
<div class="arrow" style="left:{x(prev['value']):.1f}%;width:{x(now['value']) - x(prev['value']):.1f}%"></div>
<div class="dot" style="left:{x(prev['value']):.1f}%;background:#8b949e"></div>
<div class="dot" style="left:{x(now['value']):.1f}%;background:#7ee787"></div>
<div class="dl" style="left:{x(prev['value']):.1f}%">{d(day_prev)}</div>
<div class="dl" style="left:{x(now['value']):.1f}%">{d(day_now)}</div>
<div class="tick" style="left:0">{lo:.2f}</div><div class="tick" style="left:50%">NDVI</div>
<div class="tick" style="left:100%">{hi:.2f}</div></div>
<table><tr><th>index</th><th>{d(day_prev)}</th><th>{d(day_now)}</th><th>change</th></tr>{''.join(rows)}</table>
<div class="lab" style="margin-top:10px">Embedding change, latest year against the year before (1 minus cosine,
{emb['covered_vintages']} annual vintages): <span class="m">{emb['change']:.3f}</span></div>
</div>

<div class="panel">
<div class="ph">2. Five possible causes, one row each</div>
<div class="lab m" style="margin-bottom:6px">{L['decomposition']}</div>
{terms}
</div>

<div class="panel">
<div class="ph">3. How much of the change is the park itself?</div>
<div><span class="m">split:</span> <span class="split">null</span></div>
<div class="tx">emem does not answer this. A numeric split needs a calibrated model of sensor and encoder
stability. This build does not have one, so it does not invent the numbers. The split is roadmap.</div>
<div class="lab" style="margin-top:14px">Cite the ledger like any fact:</div>
<div class="m tok">emem:fact:{led['cell']}:{cid[:8]}…{cid[-4:]}</div>
<div class="tx" style="margin-top:8px">Receipt of the resolve: <span class="ok">signature valid</span> offline;
<span class="ok">signature and merkle proof valid</span> at /v1/verify_receipt.</div>
</div>

<div class="foot">The ledger names {len(led['fact']['parents'])} parent facts. The resolve gives it provenance class
model_output and says the responder did not re-run the derivation. Read it as a list of evidence, not as a measurement.
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
Image.open(io.BytesIO(png)).convert("RGB").save(out, "WEBP", quality=82, method=6)
print(out, out.stat().st_size, "bytes")
