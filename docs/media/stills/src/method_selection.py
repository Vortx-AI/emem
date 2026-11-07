"""Still: did choosing a method per forest beat the champion? Two registered selections, both undecided.

Every value comes from the files in data/selection/, two signed result notes of the methods lab (key mujzmxes)
saved from emem.dev on 7 November 2026 by fetch_selection.py. Each note's body hash and v2 signature are checked
offline with the plugin's verify_note.py. The decision rows, pooled scores and intervals are parsed from the signed
bodies, not typed in, and checked against the numbers that docs/research/method-selection.md prints.
usage: python method_selection.py OUT.webp    (needs Playwright for Python with Chromium and Pillow)
"""
import datetime as dt
import io
import re
import sys
from pathlib import Path

from PIL import Image
from playwright.sync_api import sync_playwright

HERE = Path(__file__).resolve().parent
D = HERE / "data" / "selection"
sys.path.insert(0, str(HERE.parents[3] / "plugins/emem/skills/emem-multi-agent-handoff/scripts"))
import verify_note as vn  # noqa: E402
from emem_crypto import blake3  # noqa: E402

# (note, section of method-selection.md, forests, registered, P0 MCC, P1 MCC, P2 MCC) as the page prints them
EXPS = [("first", "9", 9, "30 October", "0.145", "0.656", "0.145"),
        ("second", "10", 10, "5 November", "0.314", "0.261", "0.308")]


def load(name):
    v = vn.load_view(str(D / f"{name}.json"))
    a = v["authorship"]
    assert blake3(v["content"].encode()).hexdigest() == a["body_hash_hex"], name
    assert vn.signature_valid(a) == (True, 2) and a["signed_path"] == v["path"], name
    assert a["attester_pubkey_b32"].startswith("mujzmxes")
    return v


def parse(body):
    sec = body[body.index("## Decisions"):body.index("## Scored")]
    rows = []
    for line in sec.splitlines():
        m = re.match(r"\| ([^|`]+?) \| (\d+) \| `([^`]+)` \| `([^`]+)` \| `([^`]+)` \| ([^|]+?) \| (yes|no) \|", line)
        if m:
            rows.append({"forest": m.group(1).split(",")[0], "loss": int(m.group(2)), "p0": m.group(3),
                         "p1": m.group(4), "p2": m.group(5), "flag": m.group(7) == "yes"})
    m1 = re.search(r"Pooled MCC: local (-?[\d.]+), global (-?[\d.]+); local minus global, forests resampled: "
                   r"([-+][\d.]+) to ([-+][\d.]+)\.", body)
    m2 = re.search(r"pooled MCC (-?[\d.]+); context minus global, forests resampled: ([-+][\d.]+) to ([-+][\d.]+)\.",
                   body)
    signed = len(re.findall(r"^- [^:]+: `emem:fact:", body[body.index("## Signed decisions"):], re.M))
    return {"rows": rows, "p1": m1.group(1), "p0": m1.group(2), "i1": (float(m1.group(3)), float(m1.group(4))),
            "p2": m2.group(1), "i2": (float(m2.group(2)), float(m2.group(3))), "signed": signed}


exps = []
for name, sec, n, reg, p0, p1, p2 in EXPS:
    v = load(name)
    e = parse(v["content"])
    assert len(e["rows"]) == n and (e["p0"], e["p1"], e["p2"]) == (p0, p1, p2), (name, e)
    assert e["i1"][0] < 0 < e["i1"][1] and e["i2"][0] <= 0 <= e["i2"][1]
    assert len({r["p0"] for r in e["rows"]}) == 1
    e.update(name=name, sec=sec, n=n, reg=reg, v=v)
    exps.append(e)
assert exps[0]["signed"] == 0 and exps[1]["signed"] == 10
# Section 10 of the page: version 5 scored exactly as version 4, so choosing version 4 changed nothing.
v4 = sum(r["p1"] == "lab:memory_hls_emp@4" for r in exps[1]["rows"])
assert v4 == 2 and exps[1]["rows"][0]["p0"] == "lab:memory_hls_emp@5"
exps[1]["extra"] = (f" At {v4} of them it chose version 4, which scored exactly as version 5, so that was no change"
                    " in practice.")

W = 800  # CSS px; rendered at 1080/W so the text stays readable on a 360 px phone


def chip(r, col):
    same = r[col] == r["p0"]
    return f'<td><div class="sq {"same" if same else "diff"}" title="{r[col]}"></div></td>'


def grid(e):
    rows = "".join(
        f'<tr><td class="fn">{r["forest"]}</td><td class="mono n">{r["loss"]}</td>'
        f'{chip(r, "p0")}{chip(r, "p1")}{chip(r, "p2")}'
        f'<td class="fl">{"flagged" if r["flag"] else ""}</td></tr>'
        for r in e["rows"])
    return f"""<table class="g"><tr><th>forest</th><th>earlier loss cells</th><th>global</th><th>local</th>
<th>context</th><th>champion wrong here</th></tr>{rows}</table>"""


def bar(label, x, cls):
    return (f'<div class="pair"><span class="bl">{label}</span><div class="track"><div class="fill {cls}" '
            f'style="width:{float(x) * 100:.1f}%"></div></div><span class="mono pv">{x}</span></div>')


LO, HI = -0.4, 0.8


def xpos(x):
    return (x - LO) / (HI - LO) * 100


def interval(label, iv):
    lo, hi = iv
    return f"""<div class="pair"><span class="bl">{label}</span><div class="axis">
<div class="zero" style="left:{xpos(0):.1f}%"></div>
<div class="ci" style="left:{xpos(lo):.1f}%;width:{max(xpos(hi) - xpos(lo), 0.6):.1f}%"></div></div>
<span class="mono pv">{lo:+.3f}<br>{hi:+.3f}</span></div>"""


def panel(e):
    champ = e["rows"][0]["p0"]
    changed = sum(r["p1"] != r["p0"] for r in e["rows"])
    signed = (f'{e["signed"]} <span class="mono">decision@1</span> facts, signed before the later period'
              if e["signed"] else 'No <span class="mono">decision@1</span> signed. The choices are fixed only in '
              'the signed plan, which the log orders before the later period.')
    return f"""<div class="panel"><div class="ph">Selection {exps.index(e) + 1}, registered {e['reg']}:
{e['n']} forests (section {e['sec']})</div>
<div class="key">Champion at every forest: <span class="mono">{champ}</span>. The local policy chose another
method at {changed} of {e['n']}.{e.get('extra', '')}</div>
{grid(e)}
<div class="sub">Pooled MCC, 2024 and 2025 (later period)</div>
{bar('global', e['p0'], 'g0')}{bar('local', e['p1'], 'g1')}{bar('context', e['p2'], 'g2')}
<div class="sub">Difference from global, 95% interval, forests resampled</div>
{interval('local', e['i1'])}{interval('context', e['i2'])}
<div class="key">{signed}</div></div>"""


def hm(t):
    return dt.datetime.fromisoformat(t.replace("Z", "+00:00")).strftime("%d %b %H:%M UTC").lstrip("0")


notes = "".join(
    f'<tr><td>{exps.index(e) + 1}</td><td class="mono id">{e["v"]["file_cid"][:12]}…</td>'
    f'<td class="mono">{hm(e["v"]["signed_at"])}</td><td class="mono ok">valid</td></tr>'
    for e in exps)

page = f"""<!doctype html><meta charset="utf-8"><style>
body{{margin:0;background:#0d1117;color:#e6edf3;font:32px/1.3 "DejaVu Sans",system-ui,sans-serif;width:{W}px}}
.wrap{{padding:40px}}
h1{{font-size:40px;line-height:1.2;margin:0 0 10px}}
.q{{color:#8b949e;font-size:25px;margin-bottom:8px}}
.mono{{font-family:"DejaVu Sans Mono",ui-monospace,monospace}}
.panel{{background:#161b22;border:1px solid #30363d;border-radius:10px;padding:18px 24px;margin-top:22px}}
.ph{{font-size:27px;margin-bottom:4px}}
.sub{{font-size:23px;margin:18px 0 4px}}
.g{{border-collapse:collapse;width:100%;font-size:22px;margin-top:10px}}
.g th{{color:#8b949e;font-weight:normal;font-size:19px;text-align:left;padding:0 6px 6px 0;vertical-align:bottom}}
.g td{{padding:3px 6px 3px 0;border-top:1px solid #30363d}}
.n{{color:#8b949e;text-align:right;padding-right:24px!important}}
.sq{{width:30px;height:22px;border-radius:4px}}
.same{{background:#30363d}} .diff{{background:#ffa657}}
.fl{{color:#ff7b72;font-size:20px}}
.pair{{display:grid;grid-template-columns:120px 1fr 110px;align-items:center;gap:12px;margin:5px 0}}
.bl{{color:#8b949e;font-size:22px}}
.track{{background:#0d1117;border:1px solid #30363d;border-radius:4px;height:24px}}
.fill{{height:100%;border-radius:3px}}
.g0{{background:#79c0ff}} .g1{{background:#ffa657}} .g2{{background:#d2a8ff}}
.pv{{font-size:21px;text-align:right}}
.axis{{position:relative;height:30px;border-bottom:1px solid #30363d}}
.zero{{position:absolute;top:0;bottom:-6px;width:2px;background:#e6edf3}}
.ci{{position:absolute;top:9px;height:12px;background:#ffa657;border-radius:6px;opacity:.85}}
.ok{{color:#7ee787}} .id{{color:#79c0ff}} .sig{{color:#d2a8ff}}
table.t{{border-collapse:collapse;width:100%;font-size:23px;margin-top:6px}}
.t th{{color:#8b949e;font-weight:normal;text-align:left;padding:2px 8px 6px 0}}
.t td{{padding:4px 8px 4px 0;border-top:1px solid #30363d}}
.key{{color:#8b949e;font-size:22px;margin-top:10px}}
.lg{{display:inline-block;width:22px;height:16px;border-radius:3px;vertical-align:middle;margin:0 4px 0 10px}}
.foot{{color:#8b949e;font-size:22px;margin-top:22px;line-height:1.4}}
</style><div class="wrap">
<h1>Choosing a method per forest has not yet beaten the champion. Both intervals cross zero.</h1>
<div class="q">Each policy chose a canopy-loss method per forest from 2022 and 2023, before 2024 and 2025 were
read. Scored against Hansen Global Forest Change at 400 m cells. Higher MCC is better.</div>
<div class="key"><span class="lg same"></span>same method as the champion<span class="lg diff"></span>another
method</div>
{panel(exps[0])}
{panel(exps[1])}
<div class="panel">
<div class="ph">Read from two signed result notes</div>
<div class="key">Key <span class="mono sig">mujzmxes</span>, signature v2 checked offline</div>
<table class="t"><tr><th>selection</th><th>file_cid</th><th>signed</th><th>signature</th></tr>{notes}</table>
</div>
<div class="foot">Notes read with emem_memory_view on 7 November 2026. Values parsed from the signed bodies by
method_selection.py. The companion paper (section 5.19) finds that one forest, Sao Felix do Xingu, carries
the local gain in selection 1. A valid signature shows
which key wrote a note. It does not show that the result is true.</div>
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
Image.open(io.BytesIO(png)).convert("RGB").save(out, "WEBP", quality=80, method=6)
print(out, out.stat().st_size, "bytes")
