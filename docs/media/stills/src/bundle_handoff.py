"""Still: agent A hands over one bundle line; weeks later agent B resolves it and checks every fact itself.

Every value comes from the files in data/bundle/, which fetch_bundle.py saved from live reads of emem.dev on
7 November 2026. The bundle was minted on 23 September 2026; this still resolves it and does not mint one.
The bundle receipt and the four fact receipts are checked offline here with the plugin's verify.py and
emem_crypto.py, so the image shows only what those offline checks accept.
usage: python bundle_handoff.py OUT.webp    (needs Playwright for Python with Chromium and Pillow)
"""
import datetime as dt
import io
import json
import sys
from pathlib import Path

from PIL import Image
from playwright.sync_api import sync_playwright

HERE = Path(__file__).resolve().parent
D = HERE / "data" / "bundle"
sys.path.insert(0, str(HERE.parents[3] / "plugins/emem/skills/emem-verify-receipt/scripts"))
import verify as vr  # noqa: E402
from emem_crypto import blake3, ed25519_verify  # noqa: E402


def load(name):
    return json.loads((D / name).read_text())


def receipt_ok(r):
    return ed25519_verify(vr._pubkey_bytes(r), vr._sig_bytes(r), blake3(vr.build_preimage(r, 2)).digest())


bundle = load("bundle.json")
rc = bundle["receipt"]
assert bundle["members"] == bundle["resolved"] == len(bundle["citations"]) == 4
assert rc["preimage_version"] == 2 and receipt_ok(rc) and load("bundle_verify.json")["valid"]
assert rc["fact_cids"] == bundle["fact_cids"] == [c["fact_cid"] for c in bundle["citations"]]
digest = blake3(vr.build_preimage(rc, 2)).hexdigest()

# Change one character of the second member's fact id in the bundle receipt.
cids = list(rc["fact_cids"])
c1 = cids[1]
cids[1] = c1[:20] + ("a" if c1[20] != "a" else "b") + c1[21:]
bad = {**rc, "fact_cids": cids}
assert not receipt_ok(bad)
bad_digest = blake3(vr.build_preimage(bad, 2)).hexdigest()

facts = []
for i, c in enumerate(bundle["citations"]):
    f = load(f"fact_{i}.json")
    assert f["resolved"] and f["cell_matches"] and f["fact_cid"] == c["fact_cid"] and f["token"] == c["memory_token"]
    assert receipt_ok(f["receipt"]) and load(f"fact_{i}_verify.json")["valid"]
    assert f["receipt"]["fact_cids"] == [c["fact_cid"]]
    assert f["receipt"]["responder_pubkey_b32"] == rc["responder_pubkey_b32"]
    facts.append(f)


def day(t):
    return dt.datetime.fromisoformat(t.replace("Z", "+00:00")).strftime("%-d %b %Y")


LABEL = {
    "copdem30m.elevation_mean": ("elevation", lambda v: f"{v:.2f} m"),
    "indices.ndvi": ("NDVI", lambda v: f"{v:.2f}"),
    "cams.pm25": ("PM2.5", lambda v: f"{v:.1f} µg/m³"),
    "era5.t2m": ("air temperature", lambda v: f"{v:.1f} °C"),
}
rows = []
for f in facts:
    name, fmt = LABEL[f["band"]]
    seen = f["fact"]["sources"][0]["captured_at"]
    rows.append(
        f'<tr><td>{name}<div class="band mono">{f["band"].replace(".", ".<wbr>")}</div></td>'
        f'<td class="mono num">{fmt(f["value"])}</td>'
        f'<td>{day(seen)}<div class="band mono id">{f["fact_cid"][:8]}…</div></td>'
        f'<td class="mono ok">valid</td></tr>'
    )
lat, lng = facts[0]["fact"]["derivation"]["args"][:2]
token = bundle["bundle_token"]
pk = rc["responder_pubkey_b32"]
W = 800  # CSS px; rendered at 1080/W so the text stays readable on a 360 px phone

page = f"""<!doctype html><meta charset="utf-8"><style>
body{{margin:0;background:#0d1117;color:#e6edf3;font:32px/1.3 "DejaVu Sans",system-ui,sans-serif;width:{W}px}}
.wrap{{padding:40px}}
h1{{font-size:44px;line-height:1.2;margin:0 0 10px}}
.q{{color:#8b949e;font-size:26px;margin-bottom:8px}}
.mono{{font-family:"DejaVu Sans Mono",ui-monospace,monospace}}
.panel{{background:#161b22;border:1px solid #30363d;border-radius:10px;padding:20px 24px;margin-top:24px}}
.ph{{font-size:28px;margin-bottom:6px}}
.call{{font-family:"DejaVu Sans Mono",ui-monospace,monospace;font-size:22px;color:#ffa657;margin-bottom:12px;
  overflow-wrap:anywhere}}
.cards{{display:flex;gap:18px;margin-top:12px;align-items:stretch}}
.card{{background:#0d1117;border:1px solid #30363d;border-radius:10px;padding:14px 18px;flex:1;min-width:0}}
.lab{{color:#8b949e;font-size:22px;margin:0 0 4px}}
.ok{{color:#7ee787}} .bad{{color:#ff7b72}} .sig{{color:#d2a8ff}} .id{{color:#79c0ff}}
.val{{font-size:27px}}
.tok{{background:#0d1117;border-left:4px solid #ffa657;padding:10px 16px;font-size:25px;margin-top:4px;
  overflow-wrap:anywhere}}
table{{width:100%;border-collapse:collapse;font-size:25px;margin-top:8px}}
td{{padding:9px 6px;border-top:1px solid #30363d;vertical-align:top}}
th{{color:#8b949e;font-size:22px;font-weight:normal;text-align:left;padding:0 6px 4px}}
.num{{font-size:27px;white-space:nowrap}}
.band{{color:#8b949e;font-size:22px}}
.key{{color:#8b949e;font-size:23px;margin-top:10px}}
.foot{{color:#8b949e;font-size:22px;margin-top:24px;line-height:1.4}}
</style><div class="wrap">
<h1>One line between two agents: four signed facts, each checked by the reader</h1>
<div class="q">Agent A minted the bundle on {day(bundle['signed_at'])}. Agent B resolved and checked it
with live reads of emem.dev on 7 November 2026.</div>

<div class="panel">
<div class="ph">1. Agent A hands over one line</div>
<div class="tok mono">{token}</div>
<div class="key">{len(token)} characters. Four members, four resolved, one cell in Bengaluru
({lat:.3f}° N, {lng:.3f}° E).</div>
</div>

<div class="panel">
<div class="ph">2. Agent B resolves the bundle, then each fact token</div>
<div class="call">GET /v1/memory_bundle/&lt;token&gt;<br>POST /v1/memory_token/resolve, once per fact</div>
<table><tr><th>reading</th><th>value</th><th>observed, fact id</th><th>receipt</th></tr>
{''.join(rows)}</table>
<div class="key">Each token resolves to the fact id that A cited, at the same cell. Each fact receipt checks valid offline.</div>
</div>

<div class="panel">
<div class="ph">3. Check the bundle receipt offline</div>
<div class="cards">
<div class="card"><div class="lab">receipt as served</div>
<div class="lab" style="margin-top:8px">digest you rebuild</div><div class="mono id val">{digest[:8]}…</div>
<div class="lab" style="margin-top:8px">Ed25519 by <span class="mono sig">{pk[:8]}…</span></div>
<div class="mono ok val">valid</div></div>
<div class="card"><div class="lab">change 1 character of the NDVI fact id</div>
<div class="lab" style="margin-top:8px">digest you rebuild</div><div class="mono bad val">{bad_digest[:8]}…</div>
<div class="lab" style="margin-top:8px">same signature</div>
<div class="mono bad val">invalid</div></div>
</div>
<div class="key">B needs no account and no trust in A. POST /v1/verify_receipt also returns valid
for all five receipts.</div>
</div>

<div class="foot">Live calls: memory_bundle, memory_token/resolve, verify_receipt. Checked offline with verify.py
and emem_crypto.py from the emem plugin. A receipt shows that emem signed these fact ids. It does not show that
the values are correct. Each value has its own date; the bundle does not make them current.</div>
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
