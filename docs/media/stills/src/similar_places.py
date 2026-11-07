"""Still: the places whose stored 2024 surface vector is closest to Dubai's, with a Sentinel-2 crop of each.

Every value comes from data/similar/, live responses from emem.dev saved on 7 November 2026 by fetch_similar.py:
  POST /v1/find_similar {"key":"Dubai","k":8,"band":"geotessera","mode":"hamming_then_rerank"}
                                                       -> data/similar/find_similar.json
  GET  /v1/scene.png (one per place, 2024)             -> data/similar/<name>.png + <name>.headers.json
  POST /v1/verify_receipt {"receipt": <receipt above>} -> data/similar/verify_receipt.json
usage: python similar_places.py OUT.webp    (needs Playwright for Python with Chromium, and Pillow)
"""
import base64
import html
import io
import json
import sys
from pathlib import Path

from PIL import Image
from playwright.sync_api import sync_playwright

D = Path(__file__).resolve().parent / "data" / "similar"
fs = json.loads((D / "find_similar.json").read_text())
vr = json.loads((D / "verify_receipt.json").read_text())
rc = fs["receipt"]
seed = fs["resolved_from"]["key"]
W = 1080

# Short names from place_label_cached. Points with no cached label, or a label we do not shorten, show coordinates only.
NAMES = {"Kirdāsah, Giza, Egypt": "Giza, Egypt", "Al Ḩawāmidīyah, Giza, Egypt": "Giza, Egypt",
         "Lisbon, Portugal": "Lisbon, Portugal", "Chicago, Illinois, United States": "Chicago, USA"}

assert vr["valid"] and vr["signature_valid"] and vr["signer_pubkey_b32"] == rc["responder_pubkey_b32"]
assert all(n["band_used"] == "geotessera" for n in fs["neighbors"])


def coord(lat, lng):
    return f"{abs(lat):.2f}°{'N' if lat >= 0 else 'S'} {abs(lng):.2f}°{'E' if lng >= 0 else 'W'}"


LO, HI, GAMMA = 0, 6000, 0.6


def common_scale(im, hdr):
    """Undo the per-image stretch named in the x-emem-scene-stretch-* headers, then put every crop on one scale."""
    chans = []
    for ch, key in zip(im.split(), "rgb"):
        lo, hi = map(float, hdr[f"x-emem-scene-stretch-{key}"].split("-"))
        lut = []
        for v in range(256):
            dn = lo + v / 255 * (hi - lo)
            x = min(max((dn - LO) / (HI - LO), 0.0), 1.0)
            lut.append(round(255 * x ** GAMMA))
        chans.append(ch.point(lut))
    return Image.merge("RGB", chans)


def tile(name, title, sub, cls=""):
    hdr = json.loads((D / f"{name}.headers.json").read_text())
    buf = io.BytesIO()
    common_scale(Image.open(D / f"{name}.png").convert("RGB"), hdr).save(buf, "PNG")
    b64 = base64.b64encode(buf.getvalue()).decode()
    date = hdr["x-emem-scene-datetime"][:10]
    return f"""<div class="t {cls}">
  <div class="imgw"><img src="data:image/png;base64,{b64}"><div class="cross"></div></div>
  <div class="tt">{title}</div><div class="ts">{sub}</div><div class="td">Sentinel-2, {date}</div>
</div>"""


tiles = [tile("seed", "Dubai <span class='tag'>seed</span>", coord(seed["lat"], seed["lng"]), "seed")]
for i, n in enumerate(fs["neighbors"]):
    name = NAMES.get(n.get("place_label_cached") or "")
    title = f"<span class='rk'>{i + 1}</span> <span class='sc'>{n['score']:.2f}</span>"
    sub = coord(n["lat"], n["lng"]) + (f"<br>{html.escape(name)}" if name else "<br><span class='dim'>no name cached</span>")
    tiles.append(tile(f"n{i}", title, sub))
tiles.append(f"""<div class="t card">
  <div class="cl">verify_receipt</div><div class="ok mono">valid</div>
  <div class="cl">signer</div><div class="sig mono">{rc['responder_pubkey_b32'][:8]}…</div>
  <div class="cl">vector facts in the receipt</div><div class="id mono">{len(rc['fact_cids'])}</div>
  <div class="cl">asked for k = {fs['requested_k']}, returned</div><div class="id mono">{fs['returned_k']}</div>
</div>""")

page = f"""<!doctype html><meta charset="utf-8"><style>
body{{margin:0;background:#0d1117;color:#e6edf3;font:32px/1.3 "DejaVu Sans",system-ui,sans-serif;width:{W}px}}
.wrap{{padding:40px}}
h1{{font-size:44px;line-height:1.2;margin:0 0 10px}}
.q{{color:#8b949e;font-size:26px;margin-bottom:28px}}
.mono{{font-family:"DejaVu Sans Mono",ui-monospace,monospace}}
.grid{{display:grid;grid-template-columns:repeat(3,1fr);gap:26px}}
.t{{background:#161b22;border:1px solid #30363d;border-radius:10px;padding:12px}}
.seed{{border:3px solid #ffa657;padding:10px}}
.imgw{{position:relative}}
.imgw img{{width:100%;display:block;image-rendering:pixelated;border-radius:4px}}
.cross{{position:absolute;left:50%;top:50%;width:22px;height:22px;margin:-11px 0 0 -11px;border:2px solid #ffa657;border-radius:50%}}
.tt{{font-size:30px;margin-top:10px}}
.ts{{color:#e6edf3;font-size:24px;font-family:"DejaVu Sans Mono",ui-monospace,monospace}}
.td{{color:#8b949e;font-size:21px;margin-top:4px}}
.tag{{color:#ffa657;font-size:24px}}
.rk{{color:#8b949e}} .sc{{color:#79c0ff;font-family:"DejaVu Sans Mono",ui-monospace,monospace}}
.dim{{color:#8b949e;font-family:"DejaVu Sans",system-ui,sans-serif}}
.card{{display:flex;flex-direction:column;justify-content:center;padding:20px 22px}}
.cl{{color:#8b949e;font-size:22px;margin-top:10px}}
.ok{{color:#7ee787;font-size:30px}} .sig{{color:#d2a8ff;font-size:28px}} .id{{color:#79c0ff;font-size:28px}}
.panel{{background:#161b22;border:1px solid #30363d;border-radius:10px;padding:18px 24px;margin-top:30px;font-size:25px;color:#8b949e}}
.panel b{{color:#e6edf3;font-weight:normal}}
.foot{{color:#8b949e;font-size:22px;margin-top:26px;line-height:1.4}}
</style><div class="wrap">
<h1>The seven stored places whose surface vector is closest to Dubai</h1>
<div class="q">find_similar ranks stored 128-D Tessera vectors for 2024 by cosine score (blue). A vector sums up
Sentinel-1 radar and Sentinel-2 optical texture over one year.</div>
<div class="grid">{''.join(tiles)}</div>
<div class="panel"><b>Read the score as surface texture, not climate.</b> Most neighbours are dry, bare ground.
Lisbon and Chicago also rank, and neither is a desert. The answer says this itself: its
<span class="mono">interpretation</span> field says a high score does not mean climate or biome proximity.</div>
<div class="foot">Live calls to emem.dev on 7 November 2026: find_similar, scene.png, verify_receipt. Each crop is about
2 km wide, centred on the point (orange ring), from one 2024 scene (March to October, at most 5% cloud). All crops share one colour scale (Sentinel-2 L2A values 0 to 6000), so brightness compares across tiles.
A signature shows who ranked the places. It does not show that they are alike on the ground.<br>
Contains modified Copernicus Sentinel data 2024.</div>
</div>"""

out = Path(sys.argv[1]).resolve()
html_path = out.with_suffix(".html")
html_path.write_text(page)
with sync_playwright() as p:
    b = p.chromium.launch()
    pg = b.new_page(viewport={"width": W, "height": 800}, device_scale_factor=1)
    pg.goto(html_path.as_uri(), wait_until="load")
    png = pg.screenshot(full_page=True)
    b.close()
html_path.unlink()
Image.open(io.BytesIO(png)).convert("RGB").save(out, "WEBP", quality=80, method=6)
print(out, out.stat().st_size, "bytes")
