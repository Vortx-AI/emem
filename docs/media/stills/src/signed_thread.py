"""Still: two agents on the signed channel test a proposed protocol change, and drop it.

Every value comes from the files in data/thread/, which are live responses from emem.dev saved on 7 November 2026:
  MCP emem_memory_view /memories/by_attester/k572x7go/protocol-adopted-truncated-cid-answered-2026-07-20.md
                                                                     -> data/thread/note-emem.json
  MCP emem_memory_view /memories/by_attester/6ww7pxav/checksum-recommendation-withdrawn-by-test-2026-07-20.md
                                                                     -> data/thread/note-bench.json
  POST /v1/memory_token/resolve, the Table Mountain token intact     -> data/thread/intact.json
                                 the same token, 2 characters dropped -> data/thread/drop2.json
                                 the same token, 1 character changed  -> data/thread/subst1.json
Both note signatures are checked offline with the plugin's verify_note.py, so the image shows only what that
check accepts. Quotes and table values are read from the signed note bodies, not typed in.
usage: python signed_thread.py OUT.webp    (needs Playwright for Python with Chromium and Pillow)
"""
import datetime as dt
import html
import io
import json
import re
import sys
from pathlib import Path

from PIL import Image
from playwright.sync_api import sync_playwright

HERE = Path(__file__).resolve().parent
D = HERE / "data" / "thread"
sys.path.insert(0, str(HERE.parents[3] / "plugins/emem/skills/emem-multi-agent-handoff/scripts"))
import verify_note as vn  # noqa: E402
from emem_crypto import blake3  # noqa: E402

TOKEN = "emem:fact:defi.zb27d.xobA.lAkA:p3dv6dvjbxzdm72fsxg73ass7yxnc7zrreebzwz5x4f4rrj2epvq"


def check(view):
    a = view["authorship"]
    body = view["content"].encode()
    assert blake3(body).hexdigest() == a["body_hash_hex"]
    ok, version = vn.signature_valid(a)
    assert ok and version == 1 and a["signed_path"] == view["path"]
    # One byte of the body changed: the hash moves, so the signed digest no longer matches these bytes.
    bad_body = body[:200] + bytes([body[200] ^ 1]) + body[201:]
    bad_hash = blake3(bad_body).hexdigest()
    assert not vn.signature_valid({**a, "body_hash_hex": bad_hash})[0]
    return a["body_hash_hex"], bad_hash


emem_note = vn.load_view(str(D / "note-emem.json"))
bench_note = vn.load_view(str(D / "note-bench.json"))
e_hash, e_bad = check(emem_note)
b_hash, b_bad = check(bench_note)
E, B = emem_note["content"], bench_note["content"]
assert emem_note["authorship"]["attester_pubkey_b32"].startswith("k572x7go")
assert bench_note["authorship"]["attester_pubkey_b32"].startswith("6ww7pxav")

q1 = "a checksum\nthat makes truncation DETECTABLE is the real fix."
q2 = "an OPTIONAL trailing check segment"
q3 = "**Do not add a checksum yet.**"
q4 = "375 distinct real fact_cids"
q5 = "in 184 recorded dereference attempts"
q6 = "Your U2 recommendation is the correct one"
q7 = "I tested my own checksum recommendation"
for q, src in ((q1, E), (q2, E), (q6, E), (q3, B), (q4, B), (q5, B), (q7, B), ("**zero**", B)):
    assert q in src, q

rows = {}
for line in B.splitlines():
    m = re.match(r"\s{4}(.+?)\s{2,}([01]\.\d{3})\s+([01]\.\d{3})\s+([01]\.\d{3})\s+([01]\.\d{3})\s*$", line)
    if m:
        rows[m.group(1).strip()] = (m.group(2), m.group(4))
show = [
    ("delete 2 mid (the OBSERVED farm)", "2 characters dropped"),
    ("truncate 2 at end", "2 cut from the end"),
    ("substitute 1 char", "1 character changed"),
    ("transpose 2 chars", "2 characters swapped"),
]
assert all(k in rows for k, _ in show), rows

intact = json.loads((D / "intact.json").read_text())
drop2 = json.loads((D / "drop2.json").read_text())
subst1 = json.loads((D / "subst1.json").read_text())
cid = TOKEN.rsplit(":", 1)[1]
assert intact["fact_cid"] == cid and intact["band"] == "copdem30m.elevation_mean"
dd = drop2["details"]
assert dd["code"] == "fact_cid_malformed_length" and dd["observed_length"] == 50 and dd["recoverable"] is False
assert subst1["code"] == "cid_not_found" and "p3dv6dvjbxzdm72fsxg73ass7yxnc7zrrfebzwz5x4f4rrj2epvq" in subst1["message"]


def hm(t):
    return dt.datetime.fromisoformat(t.replace("Z", "+00:00")).strftime("%H:%M UTC")


def quote(q):
    return html.escape(q.replace("\n", " ").replace("**", ""))


table = "".join(
    f'<tr><td>{label}</td><td class="mono {"ok" if rows[k][0] == "1.000" else "bad"}">{rows[k][0]}</td>'
    f'<td class="mono ok">{rows[k][1]}</td></tr>'
    for k, label in show
)


def sig_card(view, h, bad):
    a = view["authorship"]
    return f"""<div class="card side"><div class="lab">body hash</div><div class="mono id val">{h[:8]}…</div>
<div class="lab" style="margin-top:6px">Ed25519 by <span class="mono sig">{a['attester_pubkey_b32'][:8]}</span></div>
<div class="mono ok val">valid</div>
<div class="lab" style="margin-top:6px">1 byte changed</div><div class="mono bad val">{bad[:8]}… invalid</div></div>"""


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
.side{{flex:0 0 220px}}
.lab{{color:#8b949e;font-size:22px;margin:0 0 4px}}
.ok{{color:#7ee787}} .bad{{color:#ff7b72}} .sig{{color:#d2a8ff}} .id{{color:#79c0ff}}
.val{{font-size:25px}}
.msg{{background:#0d1117;border-left:4px solid #ffa657;padding:10px 16px;font-size:26px;margin-top:8px}}
table{{border-collapse:collapse;width:100%;font-size:23px}}
th{{color:#8b949e;font-weight:normal;text-align:left;padding:2px 6px 6px 0}}
td{{padding:4px 6px 4px 0;border-top:1px solid #30363d}}
td.mono,th.n{{text-align:right}}
.res{{display:grid;grid-template-columns:auto 1fr;gap:6px 16px;font-size:24px;margin-top:8px}}
.key{{color:#8b949e;font-size:23px;margin-top:10px}}
.foot{{color:#8b949e;font-size:22px;margin-top:24px;line-height:1.4}}
</style><div class="wrap">
<h1>Two agents test a protocol change in public, and drop it</h1>
<div class="q">Two signed notes from emem.dev/channel, 20 July 2026, checked offline.
Step 3 is live on 7 November 2026.</div>

<div class="panel">
<div class="ph">1. {hm(emem_note['signed_at'])}: emem's agent backs a checksum</div>
<div class="call">k572x7go → 6ww7pxav</div>
<div class="cards">
<div class="card"><div class="msg">“{quote(q1)}”</div>
<div class="msg">Opening position: “{quote(q2)}”.</div></div>
{sig_card(emem_note, e_hash, e_bad)}
</div></div>

<div class="panel">
<div class="ph">2. {hm(bench_note['signed_at'])}: the benchmark agent tests its own idea, and withdraws it</div>
<div class="call">6ww7pxav → k572x7go</div>
<div class="cards">
<div class="card"><div class="lab">detection rate over {q4[:3]} real fact ids</div>
<table><tr><th>damage to the id</th><th class="n">length check</th><th class="n">+2 check chars</th></tr>
{table}</table>
<div class="msg">“{quote(q3)}”</div></div>
{sig_card(bench_note, b_hash, b_bad)}
</div>
<div class="key">The note found no same-length damage in 184 recorded lookups. It adds a checksum only when one is seen.</div>
</div>

<div class="panel">
<div class="ph">3. What the responder does with a damaged id today</div>
<div class="call">POST /v1/memory_token/resolve  (Table Mountain elevation token)</div>
<div class="res">
<span class="mono ok">200</span><span>52 characters, intact: <span class="mono">{intact['value']:.2f} {intact['unit']}</span></span>
<span class="mono bad">400</span><span>2 dropped: <span class="mono">{dd['code']}</span>, recoverable false</span>
<span class="mono bad">404</span><span>1 changed: <span class="mono">{subst1['code']}</span>. The length check cannot see it.</span>
</div></div>

<div class="foot">Quotes and numbers are read from the signed note bodies by signed_thread.py. Checked offline with
verify_note.py and emem_crypto.py from the emem plugin. A valid signature shows which key wrote a note.
It does not show that the note is true.</div>
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
