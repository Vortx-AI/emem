"""Still: how many characters it costs to carry N facts in a context window, measured on live strings.

Values come from data/glossary/recall.json (19 NDVI facts at the Mount Fuji cell, 7 November 2026, 16:03 UTC),
data/cost/ (fetch_cost.py, 19 token mints and one resolve, 7 November 2026) and data/bundle/bundle.json (a bundle
minted on 23 September 2026). Counts are characters, not LLM tokens. The script stops if a string does not match
its grammar.
usage: python citation_cost.py OUT.webp    (needs Playwright for Python with Chromium, and Pillow)
"""
import io
import json
import math
import re
import sys
from pathlib import Path

from PIL import Image
from playwright.sync_api import sync_playwright

D = Path(__file__).resolve().parent / "data"
facts = json.loads((D / "glossary" / "recall.json").read_text())["facts"]
toks = json.loads((D / "cost" / "tokens.json").read_text())
res = json.loads((D / "cost" / "resolve.json").read_text())
bundle = json.loads((D / "bundle" / "bundle.json").read_text())

assert len(facts) == len(toks) == 19
CELL_TOK = re.compile(r"^emem:fact:[A-Za-z0-9.]{19,23}:[a-z2-7]{52}$")
DESC_TOK = re.compile(r"^emem:fact:-?\d+\.\d{5},-?\d+\.\d{5}@\d{4}-\d\d-\d\d@[a-z0-9_~]+:[a-z2-7]{52}$")
for f, t in zip(facts, toks):
    assert t["memory_token"] == f["memory_token"] and CELL_TOK.match(t["memory_token"]), t
    assert DESC_TOK.match(t["descriptor_token"]), t["descriptor_token"]
    assert t["descriptor_token"].endswith(":" + f["fact_cid"])
    assert float(f["value_verbatim"]) == f["value"]
assert res["resolved"] and res["cell_matches"] and res["fact_cid"] == facts[0]["fact_cid"]
assert res["canonical_token"] == facts[0]["memory_token"] and res["value"] == facts[0]["value"]
btok = bundle["bundle_token"]
assert btok == "emem:bundle:" + bundle["bundle_cid"] and bundle["members"] == bundle["resolved"] == 4


def mean(xs):
    return sum(xs) / len(xs)


shown = [f"{f['value']:.6f}" for f in facts]
kinds = [
    # label, colour, chars per fact, example string, note
    ("Value, 6 decimals", "#e6edf3", mean([len(s) for s in shown]), shown[0], "Rounded. Not the signed value."),
    ("Value as signed", "#79c0ff", mean([len(f["value_verbatim"]) for f in facts]), facts[0]["value_verbatim"],
     "Full precision, as the fact holds it."),
    ("Fact token, cell64 form", "#ffa657", mean([len(t["memory_token"]) for t in toks]), toks[0]["memory_token"],
     "Resolves to the signed bytes."),
    ("Fact token, descriptor form", "#d2a8ff", mean([len(t["descriptor_token"]) for t in toks]),
     toks[0]["descriptor_token"], "Readable place, date and band. Longer."),
]
SHORT = dict(zip([k[0] for k in kinds], ["value, 6 dp", "value, signed", "cell64 token", "descriptor token"]))
B = len(btok)
LIMIT = 256


def bundle_chars(n):
    return math.ceil(n / LIMIT) * B


# Smallest N where one bundle line is shorter than N copies of each string.
cross = {k[0]: next(n for n in range(1, 300) if bundle_chars(n) < n * k[2]) for k in kinds}
assert cross["Fact token, cell64 form"] == 1

# Chart: x is N on a log2 axis (1..1024), y is characters on a log10 axis (1..1e6).
W, H, L, R, T, Bm = 1000, 760, 130, 340, 30, 100
pw, ph = W - L - R, H - T - Bm


def X(n):
    return L + pw * math.log2(n) / 10


def Y(c):
    return T + ph * (1 - math.log10(c) / 6)


svg = [f'<svg viewBox="0 0 {W} {H}" width="100%" font-family="DejaVu Sans" font-size="34">']
for e in range(0, 7):
    y = Y(10 ** e)
    svg.append(f'<line x1="{L}" x2="{L + pw}" y1="{y}" y2="{y}" stroke="#30363d"/>')
    svg.append(f'<text x="{L - 12}" y="{y + 12}" fill="#8b949e" text-anchor="end">'
               f'{["1", "10", "100", "1k", "10k", "100k", "1M"][e]}</text>')
for n in (1, 4, 16, 64, 256, 1024):
    svg.append(f'<line x1="{X(n)}" x2="{X(n)}" y1="{T}" y2="{T + ph}" stroke="#30363d"/>')
    svg.append(f'<text x="{X(n)}" y="{T + ph + 44}" fill="#8b949e" text-anchor="middle">{n}</text>')
svg.append(f'<text x="{L + pw / 2}" y="{H - 6}" fill="#8b949e" text-anchor="middle">N facts carried</text>')
ends = []
for label, col, per, _, _ in kinds:
    svg.append(f'<line x1="{X(1)}" y1="{Y(per)}" x2="{X(1024)}" y2="{Y(1024 * per)}" stroke="{col}" '
               f'stroke-width="5"/>')
    ends.append((Y(1024 * per), col, SHORT[label]))
pts = []
for n in range(1, 1025):
    pts.append(f"{X(n):.1f},{Y(bundle_chars(n)):.1f}")
svg.append(f'<polyline points="{" ".join(pts)}" fill="none" stroke="#7ee787" stroke-width="5"/>')
ends.append((Y(bundle_chars(1024)), "#7ee787", "bundle"))
ends.sort()
last = -1e9
for y, col, lab in ends:
    y = max(y, last + 44)
    last = y
    svg.append(f'<text x="{X(1024) + 14}" y="{y + 12}" fill="{col}">{lab}</text>')
svg.append("</svg>")

rows = "".join(
    f'<div class="row"><div class="lab" style="color:{c}">{lab}</div>'
    f'<div class="num">{per:.1f}<span> characters per fact</span></div>'
    f'<div class="mono ex">{ex}</div><div class="note">{note} One bundle line is shorter from N = {cross[lab]}.</div>'
    f'</div>'
    for lab, c, per, ex, note in kinds)

page = f"""<!doctype html><meta charset="utf-8"><style>
body{{margin:0;background:#0d1117;color:#e6edf3;font:28px/1.3 "DejaVu Sans",system-ui,sans-serif;width:800px}}
.wrap{{padding:40px}}
h1{{font-size:42px;line-height:1.2;margin:0 0 10px}}
.q{{color:#8b949e;font-size:24px;margin-bottom:22px}}
.mono{{font-family:"DejaVu Sans Mono",ui-monospace,monospace}}
.card{{background:#161b22;border:1px solid #30363d;border-radius:10px;padding:16px 24px 12px;margin-bottom:20px}}
.h{{font-size:24px;color:#8b949e;text-transform:uppercase;letter-spacing:.06em;margin-bottom:6px}}
.row{{padding:12px 0;border-top:1px solid #30363d}}
.h + .row{{border-top:none}}
.lab{{font-size:26px}}
.num{{font-size:34px}} .num span{{font-size:22px;color:#8b949e}}
.ex{{font-size:22px;color:#c9d1d9;word-break:break-all;margin:4px 0}}
.note{{font-size:22px;color:#8b949e}}
.big{{font-size:34px;color:#7ee787}}
.foot{{color:#8b949e;font-size:22px;margin-top:6px;line-height:1.4}}
</style><div class="wrap">
<h1>What a citation costs in context</h1>
<div class="q">Characters needed to carry N facts, measured on live strings for {len(facts)} NDVI facts at the
Mount Fuji cell.</div>
<div class="card"><div class="h">One fact, four ways</div>{rows}</div>
<div class="card"><div class="h">One bundle line for up to {LIMIT} facts</div>
<div class="big">{B} characters</div><div class="mono ex">{btok}</div>
<div class="note">A real bundle minted on 23 September 2026, {bundle['members']} members, {bundle['resolved']}
resolved. Above {LIMIT} facts you need one more line per {LIMIT}. Minting a bundle stores it on the responder,
and the reader needs one round trip to resolve it.</div></div>
<div class="card"><div class="h">Characters for N facts (both axes log)</div>{"".join(svg)}</div>
<div class="foot">Live reads from emem.dev on 7 November 2026: POST /v1/recall (indices.ndvi, 16:03 UTC),
POST /v1/memory_token with band and observed_on ({len(toks)} calls), POST /v1/memory_token/resolve (the descriptor
token resolves to the same fact). Counts are characters, not LLM tokens. Checks:
docs/media/stills/src/citation_cost.py.</div>
</div>"""

out = Path(sys.argv[1]).resolve()
html_path = out.with_suffix(".html")
html_path.write_text(page)
with sync_playwright() as p:
    b = p.chromium.launch()
    pg = b.new_page(viewport={"width": 800, "height": 800}, device_scale_factor=1080 / 800)
    pg.goto(html_path.as_uri(), wait_until="load")
    png = pg.screenshot(full_page=True)
    b.close()
html_path.unlink()
Image.open(io.BytesIO(png)).convert("RGB").save(out, "WEBP", quality=80, method=6)
for lab, _, per, _, _ in kinds:
    print(f"{lab}: {per:.2f} chars, bundle shorter from N={cross[lab]}")
print(out, out.stat().st_size, "bytes", Image.open(out).size)
