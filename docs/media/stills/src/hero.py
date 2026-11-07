"""Still: one place, one signed fact, one token that any agent resolves and checks. A real-data README hero.

Every value comes from live responses from emem.dev saved on 7 November 2026:
  POST /v1/locate   {"place":"Mount Fuji"}                                -> data/log/locate.json
  POST /v1/recall   {"cell":"defi.zb592.nUkO.zEzE","bands":["elevation"]} -> data/log/recall.json
  GET  /v1/log/inclusion?fact_cid=<fact_cid>                              -> data/log/inclusion_fuji.json
  POST /v1/memory_token/resolve {"token":"<memory_token of the fact>"}    -> data/hero/resolve.json
  GET  /v1/scene.png?bbox=138.711,35.347,138.751,35.379&datetime=2025-11-01/2025-12-31&max_cloud=10
                                                                          -> data/hero/scene.png (+ .headers)
The receipts and the log path are checked offline with the plugin's verify.py and verify_log.py before anything
is drawn. A copy of the recall receipt with one changed character in its fact id must fail.
usage: python hero.py OUT.webp    (needs Playwright for Python with Chromium, Pillow and pyproj)
"""
import base64
import copy
import io
import json
import sys
from pathlib import Path

from PIL import Image
from playwright.sync_api import sync_playwright
from pyproj import Transformer

HERE = Path(__file__).resolve().parent
D = HERE / "data"
PLUG = HERE.parents[3] / "plugins/emem/skills"
sys.path.insert(0, str(PLUG / "emem-verify-receipt/scripts"))
import verify as vr  # noqa: E402
from emem_crypto import blake3, ed25519_verify  # noqa: E402

sys.path.insert(0, str(PLUG / "emem-transparency-log/scripts"))
import verify_log as vl  # noqa: E402


def load(p):
    return json.loads((D / p).read_text())


def receipt_ok(r):
    v = r["preimage_version"]
    return ed25519_verify(vr._pubkey_bytes(r), vr._sig_bytes(r), blake3(vr.build_preimage(r, v)).digest())


loc, rec, inc, res = load("log/locate.json"), load("log/recall.json"), load("log/inclusion_fuji.json"), load(
    "hero/resolve.json")
fact = rec["facts"][0]
cid, tok = fact["fact_cid"], fact["memory_token"]
assert loc["cell64"] == fact["cell"] and fact["band"] == "copdem30m.elevation_mean"
assert res["token"] == tok and res["resolved"] and res["fact_cid"] == cid and res["value"] == fact["value"]
assert res["fact"]["signed_at"] == fact["signed_at"] and res["fact"]["sources"] == fact["sources"]
assert receipt_ok(rec["receipt"]) and rec["receipt"]["fact_cids"] == [cid]
assert receipt_ok(res["receipt"])
bad = copy.deepcopy(rec["receipt"])
bad["fact_cids"][0] = ("r" if cid[0] != "r" else "s") + cid[1:]
assert not receipt_ok(bad)

sth = inc["sth"]
n, m = int(inc["tree_size"]), int(inc["leaf_index"])
path = [vl.b32d(p) for p in inc["audit_path_b32"]]
assert inc["fact_cid"] == cid and vl.verify_sth(sth)
assert vl.root_from_inclusion(vl.b32d(inc["leaf_hash_b32"]), m, n, path) == vl.b32d(sth["root_b32"])

hdr = {}
for line in (D / "hero/scene.headers").read_text().splitlines():
    if line.lower().startswith("x-emem-scene-"):
        k, v = line.split(":", 1)
        hdr[k.lower()[len("x-emem-scene-"):]] = v.strip()
x0, y0, x1, y1 = map(float, hdr["bbox-crs"].split(","))
img = Image.open(D / "hero/scene.png").convert("RGB")
lat, lng = res["fact"]["derivation"]["args"][:2]
ex, ny = Transformer.from_crs(4326, int(hdr["epsg"]), always_xy=True).transform(lng, lat)
assert x0 < ex < x1 and y0 < ny < y1
mx, my = (ex - x0) / (x1 - x0) * 100, (y1 - ny) / (y1 - y0) * 100
km = (x1 - x0) / 1000
scene_day = hdr["datetime"][:10]
buf = io.BytesIO()
img.save(buf, "JPEG", quality=90)
scene_uri = "data:image/jpeg;base64," + base64.b64encode(buf.getvalue()).decode()


def day(iso):
    y, mo, d = iso[:10].split("-")
    return f"{int(d)} {['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'][int(mo) - 1]} {y}"


key = fact["signer_pubkey_b32"]
read_at = rec["receipt"]["served_at"]
W = 1200
page = f"""<!doctype html><meta charset="utf-8"><style>
body{{margin:0;background:#0d1117;color:#e6edf3;font:26px/1.3 "DejaVu Sans",system-ui,sans-serif;width:{W}px}}
.wrap{{padding:34px 36px 28px}}
h1{{font-size:42px;line-height:1.15;margin:0 0 6px}}
.q{{color:#8b949e;font-size:23px}}
.mono{{font-family:"DejaVu Sans Mono",ui-monospace,monospace}}
.cols{{display:grid;grid-template-columns:1fr 34px 1fr 34px 1fr;align-items:stretch;margin-top:24px}}
.arrow{{display:flex;align-items:center;justify-content:center;color:#8b949e;font-size:34px}}
.col{{background:#161b22;border:1px solid #30363d;border-top:6px solid;border-radius:10px;padding:16px 18px}}
.step{{font-size:30px;margin-bottom:10px}}
.lab{{color:#8b949e;font-size:20px;margin-top:10px}}
.v{{font-size:22px;overflow-wrap:anywhere}}
.big{{font-size:46px;line-height:1.1}}
.pic{{position:relative;margin-top:8px}}
.pic img{{width:100%;display:block;border-radius:6px}}
.ring{{position:absolute;width:26px;height:26px;margin:-13px 0 0 -13px;border:4px solid #ffa657;border-radius:50%}}
.chk{{display:flex;justify-content:space-between;gap:10px;border-top:1px solid #30363d;padding-top:8px;margin-top:8px;font-size:21px}}
.ok{{color:#7ee787;white-space:nowrap}} .bad{{color:#ff7b72;white-space:nowrap}}
.tok{{color:#ffa657;font-size:19px;overflow-wrap:anywhere}}
.foot{{color:#8b949e;font-size:18px;margin-top:18px;line-height:1.4}}
</style><div class="wrap">
<h1>A place becomes one signed fact, and any agent can check its token.</h1>
<div class="q">Live reads from emem.dev on {day(read_at)}. The signature and log checks ran offline and do not trust the server.</div>
<div class="cols">

<div class="col" style="border-top-color:#79c0ff">
<div class="step">1. Name a place</div>
<div class="mono v">&ldquo;Mount Fuji&rdquo;</div>
<div class="lab">POST /v1/locate gives one 10 m cell</div>
<div class="mono v" style="color:#79c0ff">{fact['cell']}</div>
<div class="pic"><img src="{scene_uri}"><div class="ring" style="left:{mx:.1f}%;top:{my:.1f}%"></div></div>
<div class="lab">Ring: the cell. Sentinel-2, {day(scene_day)}, {km:.1f} km across.</div>
</div>
<div class="arrow">&#9654;</div>

<div class="col" style="border-top-color:#d2a8ff">
<div class="step">2. emem signs one fact</div>
<div class="mono big">{fact['value']:,.2f} m</div>
<div class="lab">band</div><div class="mono v">{fact['band'].replace('.', '.<wbr>')}</div>
<div class="lab">source file, read in place</div><div class="mono v">Copernicus GLO-30 DEM tile N35 E138</div>
<div class="lab">fact id: BLAKE3 of the canonical fact bytes</div><div class="mono v" style="color:#d2a8ff">{cid}</div>
<div class="lab">signed {day(fact['signed_at'])} by the Ed25519 key</div><div class="mono v">{key[:8]}&hellip;{key[-6:]}</div>
</div>
<div class="arrow">&#9654;</div>

<div class="col" style="border-top-color:#7ee787">
<div class="step">3. Any agent checks it</div>
<div class="lab">hand on this {len(tok)}-character token</div>
<div class="mono tok">{tok}</div>
<div class="chk" style="margin-top:14px"><span>Resolve on {day(res["receipt"]["served_at"])}: same fact id and value</span><span class="ok">same</span></div>
<div class="chk"><span>Receipt signature, checked offline</span><span class="ok">valid</span></div>
<div class="chk"><span>Log entry {m:,} of {n:,}, {len(path)} hashes to the signed head</span><span class="ok">valid</span></div>
<div class="chk"><span>One character of the fact id changed</span><span class="bad">fails</span></div>
</div>
</div>
<div class="foot">The source file stays in its archive. The token carries the place and the fact id, so it cannot
point anywhere else. Offline checks: verify.py and verify_log.py in plugins/emem. Elevation: Copernicus GLO-30 DEM.
Contains modified Copernicus Sentinel data 2025.</div>
</div>"""

out = Path(sys.argv[1]).resolve()
html_path = out.with_suffix(".html")
html_path.write_text(page)
with sync_playwright() as p:
    b = p.chromium.launch()
    pg = b.new_page(viewport={"width": W, "height": 700}, device_scale_factor=1.5)
    pg.goto(html_path.as_uri(), wait_until="load")
    png = pg.screenshot(full_page=True)
    b.close()
html_path.unlink()
Image.open(io.BytesIO(png)).convert("RGB").save(out, "WEBP", quality=86, method=6)
print(out, out.stat().st_size, "bytes", Image.open(out).size)
