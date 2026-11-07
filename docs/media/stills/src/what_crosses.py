"""Still: for a file, a device and an archive, what stays at the source, the one token that crosses, and its check.

Every value comes from live responses from emem.dev saved on 7 November 2026:
  GET  /v1/tree/amii4tnpkfoofsyzrbbs2vxsj4?row=1      -> data/crosses/tree_row1.json  (fetch_crosses.py)
  POST /v1/trace_resolve, POST /v1/trace_verify     -> data/crosses/trace_*.json
  GET  /v1/devices                                  -> data/crosses/devices.json
  POST /v1/recall at Mount Fuji, GET /v1/log/inclusion -> data/log/recall.json, data/log/inclusion_fuji.json
The tree path and the fact receipt are checked offline with the plugin's scripts before anything is drawn. A
changed leaf and a changed fact id must fail. The trace is checked by the server (/v1/trace_verify), and the
image says so.
usage: python what_crosses.py OUT.webp [--phone]    (needs Playwright for Python with Chromium, and Pillow)
"""
import copy
import io
import json
import sys
from pathlib import Path

from PIL import Image
from playwright.sync_api import sync_playwright

HERE = Path(__file__).resolve().parent
D = HERE / "data"
PLUG = HERE.parents[3] / "plugins/emem/skills"
sys.path.insert(0, str(PLUG / "emem-verify-receipt/scripts"))
import verify as vr  # noqa: E402
from emem_crypto import blake3, ed25519_verify  # noqa: E402

sys.path.insert(0, str(PLUG / "emem-transparency-log/scripts"))
import verify_log as vl  # noqa: E402

sys.path.insert(0, str(PLUG / "emem-tokenise-files/scripts"))
import tree_proof as tp  # noqa: E402


def load(p):
    return json.loads((D / p).read_text())


def receipt_ok(r):
    v = r["preimage_version"]
    return ed25519_verify(vr._pubkey_bytes(r), vr._sig_bytes(r), blake3(vr.build_preimage(r, v)).digest())


def fold(leaf, path):
    h = leaf
    for s in path:
        x = tp.b32d(s["hash_b32"])
        h = tp.h(x + h) if s["side"] == "left" else tp.h(h + x)
    return tp.b32e(h)


# A file: one tensor of a published safetensors index.
row = load("crosses/tree_row1.json")
leaf = tp.b32d(row["leaf_b32"])
assert fold(leaf, row["path"]) == row["root_b32"] and row["note_root_matches"]
bad_leaf = bytes([leaf[0] ^ 1]) + leaf[1:]
assert fold(bad_leaf, row["path"]) != row["root_b32"]
tensor = row["row"]["label"].removeprefix("tensor ")

# A device: one execution trace of an enrolled Linux host.
tr, tv, dev = load("crosses/trace_resolve.json"), load("crosses/trace_verify.json"), load("crosses/devices.json")
t = tr["trace"]
assert tr["resolved"] and tv["verdict"] == "admit" and not tv["report"]["coverage"]["missing"]
assert tv["report"]["trace_cid"] == tr["trace_cid"]
assert dev["count"] == 1 and dev["devices"][0]["platform_id"] == t["device"]["platform"]
assurance = dev["devices"][0]["assurance"]
assert assurance == "operator_endorsed"
segs = t["segments"]
events = sum(s["event_count"] for s in segs)
trace_tok = "emem:trace:" + tr["trace_cid"]

# An archive: the Mount Fuji elevation fact.
rec, inc = load("log/recall.json"), load("log/inclusion_fuji.json")
fact = rec["facts"][0]
cid, fact_tok = fact["fact_cid"], fact["memory_token"]
assert receipt_ok(rec["receipt"]) and rec["receipt"]["fact_cids"] == [cid]
bad = copy.deepcopy(rec["receipt"])
bad["fact_cids"][0] = ("r" if cid[0] != "r" else "s") + cid[1:]
assert not receipt_ok(bad)
n, m = int(inc["tree_size"]), int(inc["leaf_index"])
lpath = [vl.b32d(p) for p in inc["audit_path_b32"]]
assert inc["fact_cid"] == cid and vl.verify_sth(inc["sth"])
assert vl.root_from_inclusion(vl.b32d(inc["leaf_hash_b32"]), m, n, lpath) == vl.b32d(inc["sth"]["root_b32"])


def tokhtml(tok):
    return tok.replace(":", ":<wbr>").replace("#", "<wbr>#")


PHONE = "--phone" in sys.argv[2:]
W, DSF = (540, 2) if PHONE else (1200, 1.5)


def col(colour, kind, stays, crosses_lab, tok, checks, status):
    rows = "".join(f'<div class="chk"><span>{a}</span><span class="{c}">{w}</span></div>' for a, w, c in checks)
    return f"""<div class="col" style="border-top-color:{colour}">
<div class="step">{kind}</div>
<div class="lab">stays at the source</div><div class="v">{stays}</div>
<div class="lab">{crosses_lab}</div><div class="mono tok">{tokhtml(tok)}</div>
{rows}
<div class="status">{status}</div>
</div>"""


file_col = col(
    "#79c0ff", "A file",
    f"The GPT-2 weights file (safetensors). Only an index of its {row['rows']} units is published.",
    f"crosses: one tree token, unit {row['row']['index']}, the tensor <span class=\"mono\">{tensor}</span>",
    row["token"],
    [(f"The leaf hash and {len(row['path'])} path hashes give the index root, offline", "valid", "ok"),
     ("One bit of the leaf hash changed", "fails", "bad")],
    "Shipped. The proof says the unit is in the file that was signed, not that the file is true.")
dev_col = col(
    "#d2a8ff", "A device",
    f"The raw OS event logs: {events:,} events in {len(segs)} layers. The trace holds their digests.",
    "crosses: one execution trace, signed by the device key",
    trace_tok,
    [(f"POST /v1/trace_verify, {len(segs)} of {len(segs)} required layers", tv["verdict"], "ok"),
     ("Who vouches for the key", assurance.replace("_", " "), "warn")],
    f"The device is a Linux host ({t['device']['platform']}), not a satellite or robot. No hardware anchor is "
    "admitted yet.")
arc_col = col(
    "#7ee787", "An open archive",
    "The Copernicus GLO-30 DEM tile N35 E138, in its public archive. Anyone can read it again.",
    f"crosses: one fact token, Mount Fuji, {fact['value']:,.2f}&nbsp;m",
    fact_tok,
    [("Receipt signature, offline", "valid", "ok"),
     (f"Log entry {m:,} of {n:,}, {len(lpath)} hashes", "valid", "ok"),
     ("One character of the fact id changed", "fails", "bad")],
    "Shipped. This is the first corpus.")

page = f"""<!doctype html><meta charset="utf-8"><style>
body{{margin:0;background:#0d1117;color:#e6edf3;font:24px/1.3 "DejaVu Sans",system-ui,sans-serif;width:{W}px}}
.wrap{{padding:{"24px 18px 20px" if PHONE else "34px 36px 28px"}}}
h1{{font-size:{34 if PHONE else 42}px;line-height:1.15;margin:0 0 6px}}
.q{{color:#8b949e;font-size:22px}}
.mono{{font-family:"DejaVu Sans Mono",ui-monospace,monospace}}
.cols{{display:grid;grid-template-columns:{"1fr" if PHONE else "1fr 1fr 1fr"};gap:{"18px" if PHONE else "22px"};margin-top:24px}}
.col{{background:#161b22;border:1px solid #30363d;border-top:6px solid;border-radius:10px;padding:16px 18px;
display:flex;flex-direction:column}}
.step{{font-size:30px;margin-bottom:4px}}
.lab{{color:#8b949e;font-size:20px;margin-top:12px}}
.v{{font-size:21px}}
.tok{{color:#ffa657;font-size:19px;overflow-wrap:anywhere;margin-top:2px}}
.chk{{display:flex;justify-content:space-between;gap:10px;border-top:1px solid #30363d;padding-top:8px;margin-top:10px;
font-size:20px}}
.ok{{color:#7ee787;white-space:nowrap}} .bad{{color:#ff7b72;white-space:nowrap}}
.warn{{color:#e3b341;text-align:right}}
.status{{color:#8b949e;font-size:19px;margin-top:auto;padding-top:14px}}
.foot{{color:#8b949e;font-size:18px;margin-top:18px;line-height:1.4}}
</style><div class="wrap">
<h1>The data stays where it is. One token crosses, and any reader checks it.</h1>
<div class="q">Three live examples from emem.dev, read on 7 Nov 2026.</div>
<div class="cols">{file_col}{dev_col}{arc_col}</div>
<div class="foot">Offline checks: tree_proof.py, verify.py and verify_log.py in plugins/emem. The device trace is
checked by the server. Elevation: Copernicus GLO-30 DEM.</div>
</div>"""

out = Path(sys.argv[1]).resolve()
html_path = out.with_suffix(".html")
html_path.write_text(page)
with sync_playwright() as p:
    b = p.chromium.launch()
    pg = b.new_page(viewport={"width": W, "height": 700}, device_scale_factor=DSF)
    pg.goto(html_path.as_uri(), wait_until="load")
    png = pg.screenshot(full_page=True)
    b.close()
html_path.unlink()
Image.open(io.BytesIO(png)).convert("RGB").save(out, "WEBP", quality=74 if PHONE else 86, method=6)
print(out, out.stat().st_size, "bytes", Image.open(out).size)
