"""Still: one /v1/ask answer and the four stages that reached its number, each with an address you can recompute.

Every value comes from live responses from emem.dev saved on 7 November 2026 by fetch_ask.py:
  POST /v1/ask {"q": "what is the NDVI in Central Park, New York?"}   -> data/ask/ask.json
  POST /v1/verify_receipt on its receipt                               -> data/ask/ask_verify.json
  GET  /v1/state/<cid> for each reasoning stage                        -> data/ask/states.json
  GET  /v1/facts/<cid> for each fact the answer rests on               -> data/ask/facts.json
The script checks, offline, the receipt signature (the plugin's verify.py) and every stage address (BLAKE3 over
the stage's canonical CBOR, base32). It also changes one byte of one stage and checks that its address moves.
usage: python ask_trail.py OUT.webp    (needs Playwright for Python with Chromium and Pillow)
"""
import base64
import html
import io
import json
import sys
from pathlib import Path

from PIL import Image
from playwright.sync_api import sync_playwright

HERE = Path(__file__).resolve().parent
D = HERE / "data" / "ask"
sys.path.insert(0, str(HERE.parents[3] / "plugins/emem/skills/emem-verify-receipt/scripts"))
import verify as vr  # noqa: E402
from emem_crypto import blake3, ed25519_verify  # noqa: E402


def load(name):
    return json.loads((D / f"{name}.json").read_text())


ans, srv, states, facts = load("ask"), load("ask_verify"), load("states"), load("facts")


def addr(b):
    return base64.b32encode(blake3(b).digest()).decode().lower().rstrip("=")


rc = ans["receipt"]
assert ed25519_verify(vr._pubkey_bytes(rc), vr._sig_bytes(rc), blake3(vr.build_preimage(rc, rc["preimage_version"])).digest())
assert srv["valid"] and srv["signature_valid"]
assert rc["fact_cids"] == ans["fact_cids"] and len(facts) == 3

stages = ans["reasoning"]["states"]
assert [s["stage"] for s in stages] == ["located", "routed", "recalled", "scored"]
for i, s in enumerate(stages):
    cid = s["state"].split(":")[-1]
    st = states[s["stage"]]
    assert st["address_holds"] and addr(bytes.fromhex(st["canonical_cbor_hex"])) == cid, s["stage"]
    if i:
        assert s["derived_from"][0] == {"as": "own_state", "cid": stages[i - 1]["state"].split(":")[-1]}
rec = stages[2]
assert [x["cid"] for x in rec["derived_from"] if x["as"] == "fact"] == ans["fact_cids"]
cbor = bytearray(bytes.fromhex(states["recalled"]["canonical_cbor_hex"]))
cbor[-1] ^= 1
tampered = addr(bytes(cbor))
assert tampered != rec["state"].split(":")[-1]

flist = sorted(facts.items(), key=lambda kv: kv[1]["sources"][0]["captured_at"])
assert all(f["band"] == "indices.ndvi" for _, f in flist)
quoted = flist[-1]
assert "greenness (NDVI) 0.79" in ans["answer"] and round(quoted[1]["value"], 2) == 0.79

MONTHS = "January February March April May June July August September October November December".split()


def day(iso):
    y, m, d = iso[:10].split("-")
    return f"{int(d)} {MONTHS[int(m) - 1]} {y}"


def short(cid):
    return html.escape(cid[:8]) + "…"


W = 800  # CSS px; rendered at 1080/W so the text stays readable on a 360 px phone

stage_rows = []
for s in stages:
    says = s["says"]
    if s["stage"] == "located":
        says = f"Central Park, New York at cell {ans['place_resolved']['cell64']}"
    elif s["stage"] == "scored":
        n = len([x for x in ans["algorithm_outcomes_summary"] if x["value"] is not None])
        says = f"Computed {n} values from the NDVI, such as a vegetation class of 4"
    inputs = "previous stage" if s["derived_from"] else "the question"
    if s["stage"] == "recalled":
        inputs = "previous stage + 3 facts"
    stage_rows.append(
        f'<div class="stage"><div class="sn">{s["stage"]}</div>'
        f'<div class="ss">{html.escape(says)}</div>'
        f'<div class="sm"><span class="id mono">{short(s["state"].split(":")[-1])}</span>'
        f' · from {inputs} · {s["at_ms"]} ms</div></div>'
    )

bars = []
for cid, f in flist:
    v = f["value"]
    q = cid == quoted[0]
    bars.append(
        f'<div class="fr"><div class="fd">{day(f["sources"][0]["captured_at"])}</div>'
        f'<div class="track"><div class="bar{" q" if q else ""}" style="width:{v * 100:.1f}%"></div></div>'
        f'<div class="fv mono{" ok" if q else ""}">{v:.2f}</div><div class="fc id mono">{short(cid)}</div></div>'
    )

page = f"""<!doctype html><meta charset="utf-8"><style>
body{{margin:0;background:#0d1117;color:#e6edf3;font:32px/1.3 "DejaVu Sans",system-ui,sans-serif;width:{W}px}}
.wrap{{padding:40px}}
h1{{font-size:44px;line-height:1.2;margin:0 0 10px}}
.q{{color:#8b949e;font-size:26px;margin-bottom:8px}}
.mono{{font-family:"DejaVu Sans Mono",ui-monospace,monospace}}
.panel{{background:#161b22;border:1px solid #30363d;border-radius:10px;padding:20px 24px;margin-top:24px}}
.ph{{font-size:28px;margin-bottom:10px}}
.msg{{background:#0d1117;border-left:4px solid #ffa657;padding:10px 16px;font-size:26px}}
.big{{font-size:44px;font-family:"DejaVu Sans Mono",ui-monospace,monospace}}
.ok{{color:#7ee787}} .bad{{color:#ff7b72}} .id{{color:#79c0ff}}
.stage{{background:#0d1117;border:1px solid #30363d;border-radius:10px;padding:10px 16px;margin-top:10px;position:relative}}
.stage+.stage:before{{content:"";position:absolute;left:30px;top:-11px;height:10px;border-left:3px solid #30363d}}
.sn{{font-size:26px;color:#ffa657}}
.ss{{font-size:24px}}
.sm{{font-size:22px;color:#8b949e;margin-top:2px}}
.fr{{display:grid;grid-template-columns:230px 1fr 80px 150px;gap:12px;align-items:center;margin-top:10px;font-size:23px}}
.fd{{color:#8b949e}}
.track{{background:#0d1117;border:1px solid #30363d;border-radius:6px;height:24px}}
.bar{{background:#30363d;height:100%;border-radius:5px}} .bar.q{{background:#7ee787}}
.fc{{font-size:22px}}
.chk{{font-size:24px;margin-top:6px}}
.foot{{color:#8b949e;font-size:22px;margin-top:24px;line-height:1.4}}
</style><div class="wrap">
<h1>An emem answer shows the steps that reached its number</h1>
<div class="q">One live call to POST /v1/ask on 7 November 2026, served at {rc['served_at'][11:16]} UTC.</div>

<div class="panel">
<div class="msg">what is the NDVI in Central Park, New York?</div>
<div style="margin-top:12px">Answer: greenness (NDVI) <span class="big ok">0.79</span></div>
</div>

<div class="panel">
<div class="ph">Four stages. Each has an address that commits to the stage before it.</div>
{''.join(stage_rows)}
</div>

<div class="panel">
<div class="ph">The three signed Sentinel-2 NDVI facts the recall found</div>
{''.join(bars)}
<div class="sm" style="margin-top:10px">The answer quotes the newest reading (green).</div>
</div>

<div class="panel">
<div class="ph">Checks you can run without trusting emem</div>
<div class="chk"><span class="ok">valid</span> receipt over the 3 fact ids: offline and at /v1/verify_receipt</div>
<div class="chk"><span class="ok">4 of 4</span> stage addresses recomputed: BLAKE3 over the stored bytes</div>
<div class="chk"><span class="bad">moved</span> one changed byte in the recalled stage:
<span class="id mono">{short(rec['state'].split(':')[-1])}</span> becomes <span class="bad mono">{html.escape(tampered[:8])}…</span></div>
</div>

<div class="foot">The receipt signs the fact ids. The stage addresses are content addresses, not signatures:
GET /v1/state/&lt;cid&gt; returns the bytes. The scored values are computed when the answer is made. They are
not signed facts. Contains modified Copernicus Sentinel data 2026.</div>
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
