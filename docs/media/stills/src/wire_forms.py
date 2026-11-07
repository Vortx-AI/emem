"""Still: the three ids in one live fact (cell64, tslot, fact_cid), each decoded to what it names.

Values come from wire_data.decode(), which reads data/wire/ (live emem.dev reads on 7 November 2026, 15:40 UTC)
and stops if a decoded part does not agree with the response.
usage: python wire_forms.py OUT.webp    (needs Playwright for Python with Chromium, and Pillow)
"""
import io
import sys
from pathlib import Path

from PIL import Image
from playwright.sync_api import sync_playwright

sys.path.insert(0, str(Path(__file__).resolve().parent))
from wire_data import decode  # noqa: E402

v = decode()
f = v["fields"]
lanes = "".join(
    f'<div class="lane"><div class="mono id bg">{p}</div><div class="mono small">{d:,}</div></div>'
    + ('<div class="dot">.</div>' if i < 3 else "")
    for i, (p, d) in enumerate(v["lanes"]))
fields = "".join(
    f'<div class="fld"><div class="lab">{k}</div><div class="mono">{val}</div></div>'
    for k, val in (("mode", f'{f["mode"]} (a cell)'), ("resolution", f["resolution"]), ("base", f["base"]),
                   ("reserved", f["reserved"]), ("lat_q (21 bits)", f'{f["lat_q"]:,}'),
                   ("lng_q (22 bits)", f'{f["lng_q"]:,}')))
cid, hx = v["cid"], v["digest_hex"]
hx_lines = " ".join(hx[i:i + 8] for i in range(0, 32, 8)) + "<br>" + " ".join(hx[i:i + 8] for i in range(32, 64, 8))

W = 800
page = f"""<!doctype html><meta charset="utf-8"><style>
body{{margin:0;background:#0d1117;color:#e6edf3;font:28px/1.3 "DejaVu Sans",system-ui,sans-serif;width:{W}px}}
.wrap{{padding:40px}}
h1{{font-size:42px;line-height:1.2;margin:0 0 10px}}
.q{{color:#8b949e;font-size:24px;margin-bottom:22px}}
.mono{{font-family:"DejaVu Sans Mono",ui-monospace,monospace}}
.card{{background:#161b22;border:1px solid #30363d;border-radius:10px;padding:20px 24px;margin-bottom:20px}}
.tag{{font-size:30px;margin-bottom:12px}} .tag span{{color:#8b949e;font-size:24px}}
.lab{{color:#8b949e;font-size:22px;margin:12px 0 4px}}
.id{{color:#79c0ff}} .ok{{color:#7ee787}} .muted{{color:#8b949e}}
.lanes{{display:flex;align-items:flex-start;gap:6px}}
.lane{{text-align:center}} .lane .bg{{font-size:34px;background:#0d1117;border:1px solid #30363d;border-radius:6px;padding:2px 10px}}
.dot{{font-size:34px;color:#8b949e;padding-top:4px}}
.small{{font-size:22px;color:#8b949e;margin-top:4px}}
.flds{{display:grid;grid-template-columns:1fr 1fr 1fr;gap:6px 14px}}
.fld .lab{{margin:4px 0 0}} .fld .mono{{font-size:26px}}
.arrow{{color:#8b949e;font-size:24px;margin:10px 0 2px}}
.big{{font-size:30px}}
.foot{{color:#8b949e;font-size:22px;margin-top:6px;line-height:1.4}}
</style><div class="wrap">
<h1>Each id in a fact decodes to what it names</h1>
<div class="q">One NDVI fact at Mount Fuji, read from emem.dev. Each line below was decoded offline and checked
against the live response.</div>

<div class="card">
<div class="tag">cell64 <span>· where</span></div>
<div class="lanes">{lanes}</div>
<div class="arrow">each part is one 16-bit lane of a 64-bit word</div>
<div class="mono big">0x{v["word_hex"]}</div>
<div class="arrow">the word holds six fields</div>
<div class="flds">{fields}</div>
<div class="arrow">lat_q and lng_q give the cell centre</div>
<div class="mono big">{v["lat"]:.6f}° N, {v["lng"]:.6f}° E</div>
<div class="ok" style="font-size:24px;margin-top:4px">same as the centre that /v1/locate returns for "Mount Fuji"</div>
</div>

<div class="card">
<div class="tag">tslot <span>· when</span></div>
<div class="mono big">{v["tslot"]:,}</div>
<div class="arrow">{v["band"]} has a one-day slot (86,400 s)</div>
<div class="mono big">{v["tslot"]:,} × 86,400 s</div>
<div class="big">= slot opens {v["slot_start"]}</div>
<div class="ok" style="font-size:24px;margin-top:4px">the Sentinel-2 scene was captured {v["observed"]}, inside the slot</div>
<div class="arrow">text form: LEB128 bytes <span class="mono">{v["varint_hex"]}</span>, then base32</div>
<div class="mono big id">{v["tslot_text"]}</div>
</div>

<div class="card">
<div class="tag">fact_cid <span>· which bytes</span></div>
<div class="mono big id">{cid[:26]}<br>{cid[26:]}</div>
<div class="arrow">52 base32 characters are the 32 bytes of a BLAKE3 digest</div>
<div class="mono" style="font-size:24px">{hx_lines}</div>
<div class="arrow">the digest is over the {v["cbor_bytes"]} canonical CBOR bytes of the fact</div>
<div class="ok" style="font-size:24px">hashed again offline: the same fact_cid</div>
</div>

<div class="foot">Live reads from emem.dev on 7 November 2026 at 15:40 UTC: GET /v1/facts/&lt;fact_cid&gt; as JSON
and as CBOR, and POST /v1/locate. Rules: docs/protocol.md sections 1 to 3. Decoder and checks:
docs/media/stills/src/wire_data.py.</div>
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
print(out, out.stat().st_size, "bytes", Image.open(out).size)
