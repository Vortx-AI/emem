"""Still: one signed fact proven into emem's append-only log, and the log proven to only grow.

Every value comes from the files in data/log/, which are live responses from emem.dev saved on 7 November 2026:
  POST /v1/locate   {"place":"Mount Fuji"}                                      -> data/log/locate.json
  POST /v1/recall   {"cell":"defi.zb592.nUkO.zEzE","bands":["elevation"]}       -> data/log/recall.json
  GET  /v1/log/sth                                                              -> data/log/sth.json
  GET  /v1/log/inclusion?fact_cid=<fact_cid of the recall>                      -> data/log/inclusion_fuji.json
  GET  /v1/log/consistency?first=<sth.json size>&second=<inclusion_fuji size>   -> data/log/consistency.json
  GET  /v1/log/witnesses                                                        -> data/log/witnesses.json
The checks use plugins/emem/skills/emem-transparency-log/scripts/verify_log.py, so the image shows only what that
offline verifier accepts.
usage: python log_proof.py OUT.webp    (needs Playwright for Python with Chromium and Pillow)
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
DATA = HERE / "data" / "log"
sys.path.insert(0, str(HERE.parents[3] / "plugins/emem/skills/emem-transparency-log/scripts"))
import verify_log as vl  # noqa: E402


def load(name):
    return json.loads((DATA / name).read_text())


loc, rec, sth0, inc, cons, wit = (load(f) for f in (
    "locate.json", "recall.json", "sth.json", "inclusion_fuji.json", "consistency.json", "witnesses.json"))
fact = rec["facts"][0]
sth1 = inc["sth"]
n, m = int(inc["tree_size"]), int(inc["leaf_index"])
path = [vl.b32d(p) for p in inc["audit_path_b32"]]
leaf = vl.b32d(inc["leaf_hash_b32"])
root = vl.b32d(sth1["root_b32"])

assert inc["fact_cid"] == fact["fact_cid"] and inc["matched"] == "fact_cid"
assert loc["cell64"] == fact["cell"] and fact["band"] == "copdem30m.elevation_mean"
assert vl.verify_sth(sth0["sth"]) and vl.verify_sth(sth1)
assert vl.root_from_inclusion(leaf, m, n, path) == root
assert cons["first_size"] == sth0["sth"]["tree_size"] and cons["second_size"] == n
assert cons["first_root_b32"] == sth0["sth"]["root_b32"] and cons["second_root_b32"] == sth1["root_b32"]
proof = [vl.b32d(p) for p in cons["consistency_proof_b32"]]
assert vl.consistent(int(sth0["sth"]["tree_size"]), vl.b32d(sth0["sth"]["root_b32"]), n, root, proof)


def ranges(lo, hi, i):
    """RFC 6962 audit path as leaf ranges, bottom first: the sibling subtree each path hash stands for."""
    if hi - lo == 1:
        return []
    k = 1
    while k * 2 < hi - lo:
        k *= 2
    if i < lo + k:
        return ranges(lo, lo + k, i) + [(lo + k, hi)]
    return ranges(lo + k, hi, i) + [(lo, lo + k)]


sib = ranges(0, n, m)
assert len(sib) == len(path) and sum(b - a for a, b in sib) == n - 1

# One character changed in one path hash must break the proof.
bad_i = 9
s = inc["audit_path_b32"][bad_i]
bad_s = s[:20] + ("a" if s[20] != "a" else "b") + s[21:]
bad_path = list(path)
bad_path[bad_i] = vl.b32d(bad_s)
bad_root = vl.root_from_inclusion(leaf, m, n, bad_path)
assert bad_root is not None and bad_root != root
bad_root_b32 = base64.b32encode(bad_root).decode().lower().rstrip("=")

W = 800  # CSS px; rendered at 1080/W so the text stays readable on a 360 px phone


def hm(t):
    return dt.datetime.fromisoformat(t.replace("Z", "+00:00")).strftime("%H:%M:%S")


def ladder():
    """One row per path hash, hash 1 at the bottom. Bar length is log2 of the entries it covers."""
    cw, top, row = W - 130, 50, 27
    cx = cw // 2
    maxb = cx - 30
    ch = top + row * len(sib) + 60
    g = [f'<text x="{cx}" y="30" class="ax" text-anchor="middle" style="fill:#e6edf3">root</text>']
    big = max(b - a for a, b in sib).bit_length()
    for i, (a, b) in enumerate(sib):
        y = top + (len(sib) - 1 - i) * row
        size = b - a
        w = 16 + (size.bit_length() - 1) / (big - 1) * (maxb - 16)
        left = b <= m
        x = cx - 6 - w if left else cx + 6
        col = "#ff7b72" if i == bad_i else "#79c0ff"
        g.append(f'<rect x="{x:.1f}" y="{y}" width="{w:.1f}" height="{row - 7}" rx="3" fill="{col}" '
                 f'fill-opacity="{0.55 if i == bad_i else 0.3}" stroke="{col}"/>')
        tx, anchor = (cx + 18, "start") if left else (cx - 18, "end")
        unit = "entry" if size == 1 else "entries"
        g.append(f'<text x="{tx}" y="{y + 16}" class="ax" text-anchor="{anchor}">{i + 1}: {size:,} {unit}</text>')
    yb = top + row * len(sib)
    g.append(f'<line x1="{cx}" x2="{cx}" y1="40" y2="{yb + 4}" stroke="#ffa657" stroke-width="3"/>')
    g.append(f'<circle cx="{cx}" cy="{yb + 10}" r="8" fill="#ffa657"/>')
    g.append(f'<text x="{cx}" y="{yb + 46}" class="ax" text-anchor="middle" style="fill:#ffa657">'
             f'entry {m:,}: the batch with the Fuji fact</text>')
    return f'<svg viewBox="0 0 {cw} {ch}" width="{cw}" height="{ch}">{"".join(g)}</svg>'


page = f"""<!doctype html><meta charset="utf-8"><style>
body{{margin:0;background:#0d1117;color:#e6edf3;font:32px/1.3 "DejaVu Sans",system-ui,sans-serif;width:{W}px}}
.wrap{{padding:40px}}
h1{{font-size:44px;line-height:1.2;margin:0 0 10px}}
.q{{color:#8b949e;font-size:26px;margin-bottom:24px}}
.mono,.v{{font-family:"DejaVu Sans Mono",ui-monospace,monospace}}
.panel{{background:#161b22;border:1px solid #30363d;border-radius:10px;padding:20px 24px;margin-top:24px}}
.ph{{font-size:28px;margin-bottom:12px}}
.ax{{fill:#8b949e;font:22px "DejaVu Sans",system-ui,sans-serif}}
.key{{color:#8b949e;font-size:23px;margin-top:6px}}
.cards{{display:flex;gap:18px;margin-top:14px;align-items:stretch}}
.card{{background:#0d1117;border:1px solid #30363d;border-radius:10px;padding:14px 18px;flex:1;min-width:0}}
.card .mono.val{{white-space:nowrap;overflow:hidden;text-overflow:clip}}
.lab{{color:#8b949e;font-size:22px;margin:0 0 4px}}
.arrow{{align-self:center;color:#8b949e;font-size:34px}}
.ok{{color:#7ee787}} .bad{{color:#ff7b72}} .sig{{color:#d2a8ff}} .id{{color:#79c0ff}}
.val{{font-size:27px}}
.foot{{color:#8b949e;font-size:22px;margin-top:24px;line-height:1.4}}
</style><div class="wrap">
<h1>Anyone can prove offline that this fact is in the log, and that the log only grew</h1>
<div class="q">The Mount Fuji elevation fact, <span class="v" style="color:#e6edf3">{fact['value']:.2f} m</span>
(<span class="mono id">{fact['fact_cid'][:12]}…</span>), was signed in a batch of one fact. That batch is entry {m:,} of {n:,} in emem's log.</div>

<div class="panel">
<div class="ph">1. Inclusion: {len(path)} hashes stand in for the other {n - 1:,} entries</div>
{ladder()}
<div class="key">Each bar is one hash in the proof. It stands for a block of entries to the left or right of the
path. Bar length is on a log scale. Hash the entry up through all {len(path)} and you get the root.</div>
<div class="cards">
<div class="card"><div class="lab">root you compute</div><div class="mono id val">{sth1['root_b32'][:8]}…</div>
<div class="lab" style="margin-top:8px">inclusion</div><div class="mono ok val">valid</div></div>
<div class="card"><div class="lab">signed tree head, {hm(sth1['signed_at'])} UTC</div>
<div class="val">size <span class="mono">{n:,}</span></div>
<div class="lab" style="margin-top:8px">Ed25519 by <span class="mono sig">{sth1['responder_pubkey_b32'][:8]}…</span></div>
<div class="mono ok val">valid</div></div>
<div class="card"><div class="lab">change 1 character in hash {bad_i + 1} (red bar)</div>
<div class="mono bad val">{bad_root_b32[:8]}…</div>
<div class="lab" style="margin-top:8px">inclusion</div><div class="mono bad val">invalid</div></div>
</div></div>

<div class="panel">
<div class="ph">2. Consistency: the later head contains the earlier one unchanged</div>
<div class="cards">
<div class="card"><div class="lab">head you pinned, {hm(sth0['sth']['signed_at'])} UTC</div>
<div class="val">size <span class="mono">{int(sth0['sth']['tree_size']):,}</span></div>
<div class="mono id val">{sth0['sth']['root_b32'][:8]}…</div></div>
<div class="arrow">→</div>
<div class="card" style="flex:.8"><div class="lab">consistency proof</div><div class="val">{len(proof)} hashes</div>
<div class="mono ok val">valid</div></div>
<div class="arrow">→</div>
<div class="card"><div class="lab">later head, {hm(sth1['signed_at'])} UTC</div>
<div class="val">size <span class="mono">{n:,}</span></div>
<div class="mono id val">{sth1['root_b32'][:8]}…</div></div>
</div>
<div class="key">{n - int(sth0['sth']['tree_size'])} entries were added in between. No earlier entry changed, or the proof would fail.</div>
</div>

<div class="foot">Live calls to emem.dev on 7 November 2026: locate, recall, log/sth, log/inclusion, log/consistency, log/witnesses.
Checked offline with verify_log.py from the emem plugin. The log shows that emem recorded the fact and did not remove it.
It does not show that the elevation is correct. The newest co-signature by an independent witness was
{wit['freshest_independent_witness_entries_behind']} entries behind the later head.</div>
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
