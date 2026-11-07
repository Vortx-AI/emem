"""Still: one experiment of the methods lab, as three signed records in the order the log proves.

Every value comes from the files in data/experiment/, saved from emem.dev on 7 November 2026 by
fetch_experiment.py. The script checks, offline: both notes' body hash and v2 signature; the record's fact_cid
against the BLAKE3 of its CBOR bytes; the lab's Ed25519 signature on the derive digest; that the record and the
result cite the plan's content id and that the record's 14 method hashes appear in the plan; and each log
inclusion proof against its signed tree head. The image shows only what passed.
usage: python experiment_chain.py OUT.webp    (needs Playwright for Python with Chromium and Pillow)
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
D = HERE / "data" / "experiment"
SCRIPTS = HERE.parents[3] / "plugins/emem/skills"
sys.path.insert(0, str(SCRIPTS / "emem-multi-agent-handoff/scripts"))
sys.path.insert(0, str(SCRIPTS / "emem-transparency-log/scripts"))
import verify_log as vl  # noqa: E402
import verify_note as vn  # noqa: E402
from emem_crypto import blake3, ed25519_verify  # noqa: E402

LAB_KEY = "mujzmxesuhvtwzyghmki3rcytjgfm5s6srsd4uyea6qonqoifu6a"
WINNER, BASELINE = "lab:seasonal_loss_nbr@1", "registry:deforestation_alert_ndvi_drop@1"


def load(name):
    return json.loads((D / name).read_text())


def b32(raw):
    return base64.b32encode(raw).decode().lower().rstrip("=")


def note(name):
    v = vn.load_view(str(D / f"{name}.json"))
    a = v["authorship"]
    assert blake3(v["content"].encode()).hexdigest() == a["body_hash_hex"], name
    assert vn.signature_valid(a) == (True, 2) and a["signed_path"] == v["path"], name
    assert a["attester_pubkey_b32"] == LAB_KEY, name
    return v


def logged(name, entry_hash):
    inc = load(f"inclusion_{name}.json")
    sth = inc["sth"]
    leaf = vl.b32d(inc["leaf_hash_b32"])
    assert leaf == blake3(b"\x00" + vl.b32d(entry_hash)).digest(), name
    assert vl.verify_sth(sth), name
    path = [vl.b32d(p) for p in inc["audit_path_b32"]]
    assert vl.root_from_inclusion(leaf, int(inc["leaf_index"]), int(inc["tree_size"]), path) == vl.b32d(sth["root_b32"])
    return inc


plan, result = note("plan"), note("result")
rec = load("record.json")
cbor = (D / "record.cbor").read_bytes()
rec_cid = b32(blake3(cbor).digest())
assert rec_cid == load("inclusion_record.json")["fact_cid"]
args = rec["derivation"]["args"]
assert rec["derivation"]["fn_key"] == "experiment@1" and args["attester_pubkey_b32"] == LAB_KEY
digest = blake3(b"emem.memory_write|derive|/v1/derive|" + bytes.fromhex(args["body_hash_hex"])).digest()
assert ed25519_verify(vl.b32d(LAB_KEY), vl.b32d(args["attester_sig_b32"]), digest)

val = rec["value"]
assert val["plan_cid"] == plan["file_cid"] and val["plan_path"] == plan["path"]
assert f"plan_cid: {plan['file_cid']}" in result["content"]
methods = val["methods"]
assert all(m["source_blake3"] in plan["content"] for m in methods.values())
inputs = args["inputs"]
cells = {t.split(":")[2] for t in inputs}
assert len(inputs) == len(rec["parents"])

inc_plan = logged("plan", plan["log"]["entry_hash_b32"])
inc_rec = logged("record", load("inclusion_record.json")["entry_hash_b32"])
inc_res = logged("result", result["log"]["entry_hash_b32"])
assert len({i["sth"]["root_b32"] for i in (inc_plan, inc_rec, inc_res)}) == 1
lp, lr, ls = (int(i["leaf_index"]) for i in (inc_plan, inc_rec, inc_res))
assert lp < lr < ls
tree = int(inc_plan["tree_size"])
pooled = val["pooled"]
win, base = pooled[WINNER], pooled[BASELINE]
assert WINNER in val["beats_baseline"]
assert "Matthews correlation (MCC)" in plan["content"]


def t(s):
    return dt.datetime.fromisoformat(s.replace("Z", "+00:00"))


def hms(s):
    return t(s).strftime("%H:%M:%S UTC")


gap_min = round((t(rec["signed_at"]) - t(plan["signed_at"])).total_seconds() / 60)


def short(cid):
    return cid[:8] + "…"


OK = '<span class="ok">✓</span>'


def card(step, title, path, leaf, when, rows):
    lines = "".join(f'<div class="row">{OK}<span>{r}</span></div>' for r in rows)
    return f"""<div class="card"><div class="top"><span class="step">{step}</span><span class="tt">{title}</span></div>
<div class="mono path">{path}</div>
<div class="meta"><span>log leaf <b class="mono">{leaf:,}</b></span><span class="mono">{when}</span></div>
{lines}</div>"""


def gap(text):
    return f'<div class="gap"><div class="line"></div><span>{text}</span></div>'


plan_card = card("1", "Plan", "…/plan.md", lp, hms(plan["signed_at"]), [
    f'content id <span class="mono id">{short(plan["file_cid"])}</span>',
    f"names {len(methods)} methods, each pinned by the BLAKE3 of its source",
    "lab key signed it (v2 signature)",
])
rec_card = card("2", "Record <span class=\"mono\">experiment@1</span>", f"fact {short(rec_cid)}", lr,
                hms(rec["signed_at"]), [
    f'fact id = BLAKE3 of its {len(cbor):,} CBOR bytes',
    f'cites plan <span class="mono id">{short(val["plan_cid"])}</span>, same as step 1',
    f"all {len(methods)} method hashes appear in the plan",
    f"{len(inputs)} signed input tokens at {len(cells)} cells",
    "lab key signed the derive request",
])
res_card = card("3", "Result", "…/result.md", ls, hms(result["signed_at"]), [
    f'cites plan <span class="mono id">{short(plan["file_cid"])}</span>, same as step 1',
    "lab key signed it (v2 signature)",
])

W = 800  # CSS px; rendered at 1080/W so the text stays readable on a 360 px phone

page = f"""<!doctype html><meta charset="utf-8"><style>
body{{margin:0;background:#0d1117;color:#e6edf3;font:32px/1.3 "DejaVu Sans",system-ui,sans-serif;width:{W}px}}
.wrap{{padding:40px}}
h1{{font-size:42px;line-height:1.2;margin:0 0 10px}}
.q{{color:#8b949e;font-size:25px;margin-bottom:8px}}
.mono{{font-family:"DejaVu Sans Mono",ui-monospace,monospace}}
.card{{background:#161b22;border:1px solid #30363d;border-radius:10px;padding:18px 24px;margin-top:6px}}
.top{{display:flex;align-items:center;gap:14px}}
.step{{background:#1f6feb;color:#fff;border-radius:50%;width:44px;height:44px;display:flex;align-items:center;
justify-content:center;font-size:26px;flex:none}}
.tt{{font-size:30px}}
.path{{color:#8b949e;font-size:22px;margin:8px 0 2px}}
.meta{{display:flex;justify-content:space-between;font-size:23px;color:#8b949e;margin-bottom:8px}}
.meta b{{color:#e6edf3;font-weight:normal}}
.row{{display:grid;grid-template-columns:34px 1fr;font-size:24px;margin:5px 0;line-height:1.3}}
.ok{{color:#7ee787}} .id{{color:#79c0ff}}
.gap{{display:flex;align-items:center;gap:18px;color:#8b949e;font-size:23px;height:64px;padding-left:20px}}
.line{{width:4px;height:100%;background:#30363d;flex:none}}
.score{{background:#161b22;border:1px solid #30363d;border-radius:10px;padding:18px 24px;margin-top:22px}}
.ph{{font-size:26px;margin-bottom:8px}}
.sc{{display:grid;grid-template-columns:1fr 100px;font-size:22px;margin:6px 0}}
.sc span:last-child{{text-align:right}}
.sub{{color:#8b949e;font-size:22px}}
.foot{{color:#8b949e;font-size:22px;margin-top:22px;line-height:1.4}}
</style><div class="wrap">
<h1>The plan entered the log {lr - lp} entries before the record that scores it.</h1>
<div class="q">One experiment of the methods lab: canopy loss on {len(val['sites'])} scored forests, 2024 to 2025. Three signed
records, in log order, each proven into one signed tree head.</div>
{plan_card}
{gap(f"{gap_min} minutes and {lr - lp} log entries later")}
{rec_card}
{gap(f"{ls - lr} log entries later")}
{res_card}
<div class="score">
<div class="ph">MCC in the record, pooled over {val['cells']['loss']:,} loss and {val['cells']['stable']:,} stable
cells</div>
<div class="sc"><span class="mono">{WINNER}</span><span class="mono">{win['mcc']:.3f}</span></div>
<div class="sc"><span class="mono">{BASELINE}</span><span class="mono">{base['mcc']:.3f}</span></div>
<div class="sub">The plan named MCC as the score before any data was read. Higher is better.</div>
</div>
<div class="foot">Key <span class="mono">mujzmxes</span>. Read from emem.dev on 7 November 2026. Inclusion proofs
checked against tree size {tree:,}, root <span class="mono">{inc_plan['sth']['root_b32'][:8]}…</span>.
Log order shows the plan existed first. A valid signature shows which key wrote a record, not that the method is
right.</div>
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
