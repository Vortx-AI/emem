"""Still: emem-guard reads three drafts that cite (or do not cite) one signed fact, and answers allow or deny.

Every value comes from live responses from emem.dev saved on 7 November 2026 by fetch_guard.py:
  POST /v1/guard/verdict                     draft with the signed value       -> data/guard/allow.json
  POST /v1/guard/verdict                     same citation, another number     -> data/guard/deny.json
  POST /v1/guard/verdict                     a number with no citation         -> data/guard/nocite_default.json
  POST /v1/guard/verdict?claim_gating=true   the same sentence                 -> data/guard/nocite_gated.json
  POST /v1/guard/verdict?claim_gating=true   "Table Mountain rises to 1250 m." -> data/guard/nocite.json
  POST /v1/verify_receipt for each verdict's receipt                           -> data/guard/*_verify.json
The cited fact is the Table Mountain elevation in data/a2a/fact.json. Each receipt is also checked offline with
the plugin's verify.py, so the image shows only what that offline check accepts.
usage: python guard_check.py OUT.webp    (needs Playwright for Python with Chromium and Pillow)
"""
import html
import io
import json
import sys
from pathlib import Path

from PIL import Image
from playwright.sync_api import sync_playwright

HERE = Path(__file__).resolve().parent
D = HERE / "data" / "guard"
sys.path.insert(0, str(HERE.parents[3] / "tools/verifiers/emem-verify-receipt"))
import verify as vr  # noqa: E402
from emem_crypto import blake3, ed25519_verify  # noqa: E402


def load(name):
    return json.loads((D / name).read_text())


fact = json.loads((HERE / "data" / "a2a" / "fact.json").read_text())
v = {n: load(f"{n}.json") for n in ("allow", "deny", "nocite_default", "nocite_gated")}
srv = {n: load(f"{n}_verify.json") for n in v}


def receipt_ok(r):
    return ed25519_verify(vr._pubkey_bytes(r), vr._sig_bytes(r), blake3(vr.build_preimage(r, 2)).digest())


for n, body in v.items():
    rc = body["receipt"]
    assert rc["preimage_version"] == 2 and receipt_ok(rc), n
    assert srv[n]["valid"] and srv[n]["signature_valid"], n
    assert body["advisory"] is True, n

cid = fact["fact_cid"] if "fact_cid" in fact else v["allow"]["receipt"]["fact_cids"][0]
token = v["allow"]["citations"][0]["token"]
assert token.endswith(cid) and fact["band"] == "copdem30m.elevation_mean"
assert v["allow"]["action"] == "allow" and v["allow"]["citations"][0]["state"] == "verified"
assert v["deny"]["action"] == "deny" and v["deny"]["code"] == "PROV_VALUE" and v["deny"]["fix"] == "correct_value"
assert v["allow"]["receipt"]["fact_cids"] == v["deny"]["receipt"]["fact_cids"] == [cid]
assert v["nocite_default"]["action"] == "allow" and v["nocite_default"]["checked"] == 0
g = v["nocite_gated"]
assert g["action"] == "deny" and g["code"] == "CLAIM_UNGROUNDED" and g["fix"] == "cite_observation"
assert v["nocite_default"]["receipt"]["fact_cids"] == g["receipt"]["fact_cids"] == []
rises = load("nocite.json")
assert rises["_draft"] == "Table Mountain rises to 1250 m." and rises["action"] == "allow" and receipt_ok(rises["receipt"])

short = f"emem:fact:…:{cid[:8]}…"


def draft(name):
    text = v[name]["_draft"].replace(token, short)
    return html.escape(text)


W = 800  # CSS px; rendered at 1080/W so the text stays readable on a 360 px phone

page = f"""<!doctype html><meta charset="utf-8"><style>
body{{margin:0;background:#0d1117;color:#e6edf3;font:32px/1.3 "DejaVu Sans",system-ui,sans-serif;width:{W}px}}
.wrap{{padding:40px}}
h1{{font-size:44px;line-height:1.2;margin:0 0 10px}}
.q{{color:#8b949e;font-size:26px;margin-bottom:8px}}
.mono,.v{{font-family:"DejaVu Sans Mono",ui-monospace,monospace}}
.panel{{background:#161b22;border:1px solid #30363d;border-radius:10px;padding:20px 24px;margin-top:24px}}
.ph{{font-size:28px;margin-bottom:10px}}
.cards{{display:flex;gap:18px;margin-top:12px;align-items:stretch}}
.card{{background:#0d1117;border:1px solid #30363d;border-radius:10px;padding:14px 18px;flex:1;min-width:0}}
.lab{{color:#8b949e;font-size:22px;margin:0 0 4px}}
.ok{{color:#7ee787}} .bad{{color:#ff7b72}} .id{{color:#79c0ff}}
.val{{font-size:27px;overflow-wrap:anywhere}}
.big{{font-size:44px;font-family:"DejaVu Sans Mono",ui-monospace,monospace}}
.msg{{background:#0d1117;border-left:4px solid #ffa657;padding:10px 16px;font-size:26px;overflow-wrap:anywhere}}
.key{{color:#8b949e;font-size:23px;margin-top:10px}}
.foot{{color:#8b949e;font-size:22px;margin-top:24px;line-height:1.4}}
</style><div class="wrap">
<h1>emem-guard checks the numbers in a draft before it is sent</h1>
<div class="q">One sentence, three ways. Each answer is one live call to POST /v1/guard/verdict on 7 November 2026.
The cited fact: Table Mountain elevation, <span class="mono">{fact['value']:.2f} m</span>, Copernicus DEM.</div>

<div class="panel">
<div class="ph">1. The draft states the signed value and cites it</div>
<div class="msg">{draft('allow')}</div>
<div class="cards">
<div class="card"><div class="lab">citation</div><div class="mono ok val">verified</div></div>
<div class="card"><div class="lab">verdict</div><div class="big ok">allow</div></div>
</div></div>

<div class="panel">
<div class="ph">2. Same citation, a different number</div>
<div class="msg">{draft('deny')}</div>
<div class="cards">
<div class="card"><div class="lab">signed value</div><div class="mono val">{fact['value']:.2f} m</div>
<div class="lab" style="margin-top:8px">fix</div><div class="mono val">{v['deny']['fix']}</div></div>
<div class="card"><div class="lab">verdict</div><div class="big bad">deny</div>
<div class="mono bad val">{v['deny']['code']}</div></div>
</div></div>

<div class="panel">
<div class="ph">3. A number with no citation</div>
<div class="msg">{draft('nocite_gated')}</div>
<div class="cards">
<div class="card"><div class="lab">default call</div><div class="big ok">allow</div>
<div class="lab" style="margin-top:8px">claim rule off, citations checked: {v['nocite_default']['checked']}</div></div>
<div class="card"><div class="lab">?claim_gating=true</div><div class="big bad">deny</div>
<div class="mono bad val">{g['code']}</div>
<div class="lab" style="margin-top:8px">fix</div><div class="mono val">{g['fix']}</div></div>
</div></div>

<div class="foot">Each of the four verdicts carries a signed receipt. Each one checks valid offline (verify.py
from the emem plugin) and at /v1/verify_receipt. The receipt lists the fact the guard read: one fact for
drafts 1 and 2, none for draft 3.
On emem.dev the guard is advisory: it blocks nothing. A node you run can enforce.
The claim rule is narrow. It did not flag "Table Mountain rises to 1250 m.": it finds a place name
only after a word such as "at" or "in".</div>
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
