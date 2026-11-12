"""Still: two canopy-loss rules find about as much lost forest, but one flags far more stable forest.

Every value comes from the files in data/canopy/, three signed result notes of the methods lab (key mujzmxes)
saved from emem.dev on 7 November 2026 by fetch_canopy.py. Each note's body hash and v2 signature are checked
offline with the plugin's verify_note.py, and the table rows are parsed from the signed bodies, not typed in.
usage: python canopy_rules.py OUT.webp    (needs Playwright for Python with Chromium and Pillow)
"""
import datetime as dt
import io
import re
import sys
from pathlib import Path

from PIL import Image
from playwright.sync_api import sync_playwright

HERE = Path(__file__).resolve().parent
D = HERE / "data" / "canopy"
sys.path.insert(0, str(HERE.parents[3] / "tools/verifiers/emem-multi-agent-handoff"))
import verify_note as vn  # noqa: E402
from emem_crypto import blake3  # noqa: E402

REG = "registry:deforestation_alert_ndvi_drop@1"
LAB = "lab:seasonal_loss_nbr@1"
EXPS = [("scaled", "5.5", 19), ("mcc-replication", "5.6", 7), ("third-replication", "5.8", 10)]
DOC_MCC = {"5.5": ("0.295", "0.381"), "5.6": ("0.145", "0.457"), "5.8": ("0.098", "0.262")}


def load(name):
    v = vn.load_view(str(D / f"{name}.json"))
    a = v["authorship"]
    assert blake3(v["content"].encode()).hexdigest() == a["body_hash_hex"], name
    assert vn.signature_valid(a) == (True, 2) and a["signed_path"] == v["path"], name
    assert a["attester_pubkey_b32"].startswith("mujzmxes")
    return v


def rows(body):
    out = {}
    for line in body.splitlines():
        m = re.match(r"\| `([^`]+)` \| \w+ \| (-?[01]\.\d+) \| [\d.]+ \| [\d.]+ \| ([\d.]+) \| ([\d.]+) \|", line)
        if m:
            out[m.group(1)] = {"mcc": m.group(2), "recall": float(m.group(3)), "far": float(m.group(4))}
    return out


exps = []
for name, sec, n_sites in EXPS:
    v = load(name)
    body = v["content"]
    m = re.search(r"(\d+) cells labelled loss and (\d+) stable", body)
    # Sites that could not be read are listed as "not scored"; count only the scored ones.
    sites = body[body.index("## Per site"):body.index("## Reading it")].count(": labels ")
    r = rows(body)
    assert sites == n_sites and REG in r and LAB in r, name
    assert (r[REG]["mcc"], r[LAB]["mcc"]) == DOC_MCC[sec], (sec, r[REG]["mcc"], r[LAB]["mcc"])
    exps.append({"sec": sec, "sites": sites, "loss": int(m.group(1)), "stable": int(m.group(2)),
                 "reg": r[REG], "lab": r[LAB], "v": v})


def pct(x):
    return f"{round(x * 100)}%"


def bar(x, cls):
    return f'<div class="track"><div class="fill {cls}" style="width:{x * 100:.1f}%"></div></div>'


def rule_row(name, r):
    title, code = name
    return f"""<div class="rule"><div class="rn"><span>{title}</span><span class="mono mcc">MCC {r['mcc']}</span></div>
<div class="mono nm">{code}</div>
<div class="pair"><span class="bl">finds lost forest</span>{bar(r['recall'], 'find')}<span class="mono pv">{pct(r['recall'])}</span></div>
<div class="pair"><span class="bl">flags stable forest</span>{bar(r['far'], 'flag')}<span class="mono pv">{pct(r['far'])}</span></div></div>"""


panels = "".join(
    f"""<div class="panel"><div class="ph">Section {e['sec']}: {e['sites']} forests, {e['loss']:,} lost and
{e['stable']:,} stable cells</div>
{rule_row(('Registry rule', 'registry:deforestation_alert_ndvi_drop@1'), e['reg'])}
{rule_row(('Seasonal rule', 'lab:seasonal_loss_nbr@1'), e['lab'])}
</div>"""
    for e in exps
)


def hm(t):
    return dt.datetime.fromisoformat(t.replace("Z", "+00:00")).strftime("%H:%M UTC")


notes = "".join(
    f'<tr><td>{e["sec"]}</td><td class="mono id">{e["v"]["file_cid"][:12]}…</td>'
    f'<td class="mono">{hm(e["v"]["signed_at"])}</td><td class="mono ok">valid</td></tr>'
    for e in exps
)

def span(key):
    lo, hi = sorted((e[key]["far"] for e in exps))[0], max(e[key]["far"] for e in exps)
    return f"{round(lo * 100)} to {round(hi * 100)}%"


flag_reg, flag_lab = span("reg"), span("lab")
assert all(e["reg"]["recall"] > e["lab"]["recall"] for e in exps)

W = 800  # CSS px; rendered at 1080/W so the text stays readable on a 360 px phone

page = f"""<!doctype html><meta charset="utf-8"><style>
body{{margin:0;background:#0d1117;color:#e6edf3;font:32px/1.3 "DejaVu Sans",system-ui,sans-serif;width:{W}px}}
.wrap{{padding:40px}}
h1{{font-size:42px;line-height:1.2;margin:0 0 10px}}
.q{{color:#8b949e;font-size:25px;margin-bottom:8px}}
.mono{{font-family:"DejaVu Sans Mono",ui-monospace,monospace}}
.panel{{background:#161b22;border:1px solid #30363d;border-radius:10px;padding:18px 24px;margin-top:22px}}
.ph{{font-size:26px;margin-bottom:4px}}
.rule{{margin-top:14px}}
.rn{{font-size:25px;display:flex;justify-content:space-between}}
.nm{{color:#8b949e;font-size:22px;margin-bottom:6px}}
.mcc{{color:#8b949e;font-size:22px;align-self:center}}
.pair{{display:grid;grid-template-columns:230px 1fr 70px;align-items:center;gap:12px;margin:4px 0}}
.bl{{color:#8b949e;font-size:22px}}
.track{{background:#0d1117;border:1px solid #30363d;border-radius:4px;height:24px}}
.fill{{height:100%;border-radius:3px}}
.find{{background:#79c0ff}} .flag{{background:#ff7b72}}
.pv{{font-size:24px;text-align:right}}
.ok{{color:#7ee787}} .id{{color:#79c0ff}} .sig{{color:#d2a8ff}}
table{{border-collapse:collapse;width:100%;font-size:23px;margin-top:6px}}
th{{color:#8b949e;font-weight:normal;text-align:left;padding:2px 8px 6px 0}}
td{{padding:4px 8px 4px 0;border-top:1px solid #30363d}}
.key{{color:#8b949e;font-size:22px;margin-top:10px}}
.foot{{color:#8b949e;font-size:22px;margin-top:22px;line-height:1.4}}
</style><div class="wrap">
<h1>The registry rule finds more lost forest, but flags {flag_reg} of stable forest. The seasonal rule flags
{flag_lab}.</h1>
<div class="q">Three pre-registered experiments on disjoint forests, scored against Hansen Global Forest Change
at 400 m cells. MCC is the score each plan named in advance. Higher is better.</div>
{panels}
<div class="panel">
<div class="ph">Read from three signed result notes</div>
<div class="key">Key <span class="mono sig">mujzmxes</span>, written 29 October 2026, signature v2 checked offline</div>
<table><tr><th>section</th><th>file_cid</th><th>signed</th><th>signature</th></tr>{notes}</table>
</div>
<div class="foot">Notes read with emem_memory_view on 7 November 2026. Values parsed from the signed bodies by
canopy_rules.py. A valid signature shows which key wrote a note. It does not show that the result is true.</div>
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
