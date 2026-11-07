"""Still: the signed preimage of one live recall receipt, rebuilt segment by segment and checked offline.

Values come from receipt_data.build(), which reads data/receipt/ (live emem.dev reads on 7 November 2026,
15:44 UTC) and stops if the offline digest or signature does not agree with the live response.
usage: python receipt_preimage.py OUT.webp    (needs Playwright for Python with Chromium, and Pillow)
"""
import io
import sys
from pathlib import Path

from PIL import Image
from playwright.sync_api import sync_playwright

sys.path.insert(0, str(Path(__file__).resolve().parent))
from receipt_data import build  # noqa: E402

v = build()


def split(s, n):
    return "<br>".join(s[i:i + n] for i in range(0, len(s), n))


def short(s):
    return s if len(s) <= 26 else s[:12] + "…" + s[-8:]


segs = "".join(
    f'<div class="seg"><div class="mono tagc">{r["tag"]}</div>'
    f'<div><div class="name">{r["name"]} <span class="muted">· {r["kind"]}, {r["len"]} bytes</span></div>'
    f'<div class="mono val">{short(r["value"])}</div></div></div>'
    for r in v["rows"])
dig = " ".join(v["digest_hex"][i:i + 8] for i in range(0, 64, 8))
dig = dig[:35] + "<br>" + dig[36:]
tdig = " ".join(v["tampered_digest_hex"][i:i + 8] for i in range(0, 64, 8))
tdig = tdig[:35] + "<br>" + tdig[36:]

W = 800
page = f"""<!doctype html><meta charset="utf-8"><style>
body{{margin:0;background:#0d1117;color:#e6edf3;font:28px/1.3 "DejaVu Sans",system-ui,sans-serif;width:{W}px}}
.wrap{{padding:40px}}
h1{{font-size:42px;line-height:1.2;margin:0 0 10px}}
.q{{color:#8b949e;font-size:24px;margin-bottom:22px}}
.mono{{font-family:"DejaVu Sans Mono",ui-monospace,monospace}}
.card{{background:#161b22;border:1px solid #30363d;border-radius:10px;padding:20px 24px;margin-bottom:20px}}
.tag{{font-size:30px;margin-bottom:12px}} .tag span{{color:#8b949e;font-size:24px}}
.id{{color:#79c0ff}} .ok{{color:#7ee787}} .bad{{color:#ff7b72}} .muted{{color:#8b949e}}
.seg{{display:flex;gap:16px;align-items:flex-start;padding:8px 0;border-top:1px solid #30363d}}
.seg:first-of-type{{border-top:none}}
.tagc{{font-size:26px;color:#d2a8ff;background:#0d1117;border:1px solid #30363d;border-radius:6px;padding:2px 8px;min-width:72px;text-align:center}}
.name{{font-size:26px}} .name span{{font-size:22px}}
.val{{font-size:24px;color:#79c0ff;word-break:break-all}}
.arrow{{color:#8b949e;font-size:24px;margin:10px 0 2px}}
.big{{font-size:28px}}
.note{{color:#8b949e;font-size:22px;margin-top:8px}}
.foot{{color:#8b949e;font-size:22px;margin-top:6px;line-height:1.4}}
</style><div class="wrap">
<h1>What a receipt signs, rebuilt from a live read</h1>
<div class="q">One recall of elevation at Mount Fuji ({v["value"]:.2f} m) on emem.dev. Each segment below was
rebuilt offline from the JSON receipt.</div>

<div class="card">
<div class="tag">1 · the preimage stream <span>· {v["stream_len"]} bytes</span></div>
<div class="mono" style="font-size:24px">emem.preimage.v1\\x00 · u32 7 · "receipt"</div>
<div class="note">then each segment: tag, u32 length, bytes</div>
<div style="margin-top:10px">{segs}</div>
<div class="note">Not present in this read, so not written: {", ".join(v["absent"])}.</div>
</div>

<div class="card">
<div class="tag">2 · hash and sign</div>
<div class="arrow">BLAKE3 of the {v["stream_len"]} bytes</div>
<div class="mono big id">{dig}</div>
<div class="ok" style="font-size:24px;margin-top:4px">same digest as /v1/verify_receipt reports</div>
<div class="arrow">Ed25519 signature on the receipt (64 bytes)</div>
<div class="mono" style="font-size:24px">{short(v["sig_b32"])}</div>
<div class="arrow">checked offline with the key from <span style="white-space:nowrap">/.well-known/emem.json</span></div>
<div class="mono" style="font-size:24px">{short(v["pub_b32"])}</div>
<div class="ok big" style="margin-top:6px">signature valid</div>
</div>

<div class="card">
<div class="tag">3 · change one character</div>
<div class="mono big">served_at {v["rows"][1]["value"][11:]} → <span class="bad">{v["tampered_served_at"][11:]}</span></div>
<div class="arrow">BLAKE3 of the changed stream</div>
<div class="mono big bad">{tdig}</div>
<div class="bad big" style="margin-top:6px">the same signature fails</div>
</div>

<div class="foot">Live reads from emem.dev on 7 November 2026 at 15:44 UTC: POST /v1/recall, POST /v1/verify_receipt,
<span style="white-space:nowrap">GET /.well-known/emem.json</span>. Rules: docs/protocol.md section 7.1. Rebuild and checks:
docs/media/stills/src/receipt_data.py.</div>
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
