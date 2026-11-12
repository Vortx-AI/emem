"""Still: an agent with no key and no account asks emem one question over A2A and checks the answer itself.

Every value comes from the files in data/a2a/, which are live responses from emem.dev saved on 7 November 2026:
  GET  /.well-known/agent-card.json                                        -> data/a2a/card.json
  POST /a2a/tasks  JSON-RPC message/send, one text part
       "what is the elevation of Table Mountain?", metadata.verbosity=full  -> data/a2a/task.json
  GET  /v1/facts/<the one fact_cid in the task's receipt>                  -> data/a2a/fact.json
  POST /v1/verify_receipt {"receipt": <the task artifact's receipt>}       -> data/a2a/verify_receipt.json
  GET  /v1/cells/<cell>/scene.png?datetime=2024-01-01/2024-03-31&max_cloud=5
                                                                           -> data/a2a/scene.png, scene.headers
The card's JWS and the receipt are checked offline here with the plugin's emem_crypto.py and verify.py, so the
image shows only what those offline checks accept.
usage: python a2a_exchange.py OUT.webp    (needs Playwright for Python with Chromium and Pillow)
"""
import base64
import datetime as dt
import io
import json
import sys
from pathlib import Path

from PIL import Image
from playwright.sync_api import sync_playwright

HERE = Path(__file__).resolve().parent
D = HERE / "data" / "a2a"
sys.path.insert(0, str(HERE.parents[3] / "tools/verifiers/emem-verify-receipt"))
import verify as vr  # noqa: E402
from emem_crypto import blake3, ed25519_verify  # noqa: E402


def load(name):
    return json.loads((D / name).read_text())


card, task, fact, srv = (load(f) for f in ("card.json", "task.json", "fact.json", "verify_receipt.json"))
res = task["result"]
data = res["artifacts"][0]["parts"][0]["data"]
rc = data["receipt"]
pubkey_b32 = card["responder"]["pubkey_b32"]


def b64u(b):
    return base64.urlsafe_b64encode(b).rstrip(b"=")


def jcs(obj):
    return json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()


# Card: JWS over the card with `signatures` removed, canonicalized per RFC 8785.
body = {k: v for k, v in card.items() if k != "signatures"}
jws = card["signatures"][0]
hdr = json.loads(base64.urlsafe_b64decode(jws["protected"] + "=="))
pk = vr.b32_nopad_decode(pubkey_b32)
jws_sig = base64.urlsafe_b64decode(jws["signature"] + "==")
assert hdr["alg"] == "EdDSA" and hdr["kid"] == pubkey_b32
assert ed25519_verify(pk, jws_sig, jws["protected"].encode() + b"." + b64u(jcs(body)))
assert not ed25519_verify(pk, jws_sig, jws["protected"].encode() + b"." + b64u(jcs({**body, "version": "2.4.5"})))
listed, total = card["emem"]["skills"]["listed"], card["emem"]["skills"]["total"]
assert listed == len(card["skills"])

# Task and fact.
assert res["status"]["state"] == "TASK_STATE_COMPLETED" and res["metadata"]["skill"] == "emem_ask"
assert rc["fact_cids"] == [fact_cid := rc["fact_cids"][0]] and fact["band"] == "copdem30m.elevation_mean"
assert data["place_resolved"]["cell64"] == fact["cell"] == rc["cells"][0]


# Receipt: rebuild the v2 preimage offline, then change one character of the fact id.
def receipt_ok(r):
    return ed25519_verify(vr._pubkey_bytes(r), vr._sig_bytes(r), blake3(vr.build_preimage(r, 2)).digest())


assert rc["preimage_version"] == 2 and receipt_ok(rc)
digest = blake3(vr.build_preimage(rc, 2)).hexdigest()
assert digest == srv["preimage_blake3_hex"] and srv["signature_valid"] and srv["merkle_proof_valid"]
bad_cid = fact_cid[:20] + ("a" if fact_cid[20] != "a" else "b") + fact_cid[21:]
bad = {**rc, "fact_cids": [bad_cid], "merkle_proof": {**rc["merkle_proof"], "fact_cid": bad_cid}}
assert not receipt_ok(bad)
bad_digest = blake3(vr.build_preimage(bad, 2)).hexdigest()

hdrs = dict(line.split(": ", 1) for line in (D / "scene.headers").read_text().splitlines() if ": " in line)
hdrs = {k.lower(): v for k, v in hdrs.items()}
x0, y0, x1, y1 = map(float, hdrs["x-emem-scene-bbox-crs"].split(","))
km = (x1 - x0) / 1000
scene_day = dt.datetime.fromisoformat(hdrs["x-emem-scene-datetime"].replace("Z", "+00:00")).strftime("%-d %B %Y")
buf = io.BytesIO()
Image.open(D / "scene.png").convert("RGB").save(buf, "PNG")
scene_b64 = base64.b64encode(buf.getvalue()).decode()


def day(t):
    return dt.datetime.fromisoformat(t.replace("Z", "+00:00")).strftime("%-d %b %Y")


def hm(t):
    return dt.datetime.fromisoformat(t.replace("Z", "+00:00")).strftime("%H:%M:%S")


lat, lng = data["place_resolved"]["lat"], data["place_resolved"]["lng"]
question = data["question"]
W = 800  # CSS px; rendered at 1080/W so the text stays readable on a 360 px phone

page = f"""<!doctype html><meta charset="utf-8"><style>
body{{margin:0;background:#0d1117;color:#e6edf3;font:32px/1.3 "DejaVu Sans",system-ui,sans-serif;width:{W}px}}
.wrap{{padding:40px}}
h1{{font-size:44px;line-height:1.2;margin:0 0 10px}}
.q{{color:#8b949e;font-size:26px;margin-bottom:8px}}
.mono,.v{{font-family:"DejaVu Sans Mono",ui-monospace,monospace}}
.panel{{background:#161b22;border:1px solid #30363d;border-radius:10px;padding:20px 24px;margin-top:24px}}
.ph{{font-size:28px;margin-bottom:6px}}
.call{{font-family:"DejaVu Sans Mono",ui-monospace,monospace;font-size:22px;color:#ffa657;margin-bottom:12px;
  overflow-wrap:anywhere}}
.cards{{display:flex;gap:18px;margin-top:12px;align-items:stretch}}
.card{{background:#0d1117;border:1px solid #30363d;border-radius:10px;padding:14px 18px;flex:1;min-width:0}}
.lab{{color:#8b949e;font-size:22px;margin:0 0 4px}}
.ok{{color:#7ee787}} .bad{{color:#ff7b72}} .sig{{color:#d2a8ff}} .id{{color:#79c0ff}}
.val{{font-size:27px}}
.big{{font-size:44px;font-family:"DejaVu Sans Mono",ui-monospace,monospace}}
.msg{{background:#0d1117;border-left:4px solid #ffa657;padding:10px 16px;font-size:26px;margin-top:4px}}
.scene{{position:relative;width:256px;height:256px;flex:none}}
.scene img{{width:256px;height:256px;display:block;border-radius:6px}}
.ring{{position:absolute;left:116px;top:116px;width:24px;height:24px;border:3px solid #ffa657;border-radius:50%}}
.key{{color:#8b949e;font-size:23px;margin-top:10px}}
.foot{{color:#8b949e;font-size:22px;margin-top:24px;line-height:1.4}}
</style><div class="wrap">
<h1>An agent with no key asks emem over A2A, then checks the answer itself</h1>
<div class="q">Three steps. Every value here is from live calls to emem.dev on 7 November 2026.</div>

<div class="panel">
<div class="ph">1. Read the agent card</div>
<div class="call">GET /.well-known/agent-card.json</div>
<div class="cards">
<div class="card"><div class="lab">agent</div><div class="val">{card['name']} {card['version']}</div>
<div class="lab" style="margin-top:8px">A2A protocol</div><div class="val">{card['protocolVersion']}</div></div>
<div class="card"><div class="lab">skills</div><div class="val">{listed} listed</div>
<div class="val">{total} in all</div></div>
<div class="card"><div class="lab">card signature, {hdr['alg']}</div>
<div class="mono sig val">{pubkey_b32[:8]}…</div><div class="mono ok val">valid</div></div>
</div></div>

<div class="panel">
<div class="ph">2. Send one message, get a finished task</div>
<div class="call">POST /a2a/tasks  message/send</div>
<div class="msg">{question}</div>
<div class="cards">
<div class="scene"><img src="data:image/png;base64,{scene_b64}" alt=""><div class="ring"></div></div>
<div class="card"><div class="lab">task state</div><div class="mono ok val">COMPLETED</div>
<div class="lab" style="margin-top:8px">answered by skill</div><div class="mono val">{res['metadata']['skill']}</div>
<div class="lab" style="margin-top:8px">elevation at the cell</div><div class="big">{fact['value']:.2f} m</div></div>
</div>
<div class="key">Copernicus DEM, 30 m pixel at {abs(lat):.3f}° S, {lng:.3f}° E (orange ring).
Fact <span class="mono id">{fact_cid[:8]}…</span>, signed {day(fact['signed_at'])}.</div>
</div>

<div class="panel">
<div class="ph">3. Check the receipt in the task's artifact, offline</div>
<div class="cards">
<div class="card"><div class="lab">receipt as served, {hm(rc['served_at'])} UTC</div>
<div class="lab" style="margin-top:8px">digest you rebuild</div><div class="mono id val">{digest[:8]}…</div>
<div class="lab" style="margin-top:8px">Ed25519 by <span class="mono sig">{rc['responder_pubkey_b32'][:8]}…</span></div>
<div class="mono ok val">valid</div></div>
<div class="card"><div class="lab">change 1 character of the fact id</div>
<div class="lab" style="margin-top:8px">digest you rebuild</div><div class="mono bad val">{bad_digest[:8]}…</div>
<div class="lab" style="margin-top:8px">same signature</div>
<div class="mono bad val">invalid</div></div>
</div>
<div class="key">The same signer key signed the card and the receipt. POST /v1/verify_receipt also returns
signature valid and Merkle proof valid.</div>
</div>

<div class="foot">Live calls: agent-card.json, a2a/tasks, facts, verify_receipt, scene.png. Checked offline with
verify.py and emem_crypto.py from the emem plugin. The receipt shows that emem signed this fact id.
It does not show that the elevation is correct. The value is the pixel where the name resolves, not the summit.
The image is the Sentinel-2 scene of {scene_day}, about {km:.1f} km wide.
Contains modified Copernicus Sentinel data 2024.</div>
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
