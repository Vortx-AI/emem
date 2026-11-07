"""Still: the glossary terms placed on the parts of one live recall answer.

Values come from data/glossary/ (fetch_glossary.py, live emem.dev reads on 7 November 2026, 16:03 UTC). The script
stops if a part does not agree with another part of the same answer.
usage: python glossary_map.py OUT.webp    (needs Playwright for Python with Chromium, and Pillow)
"""
import datetime as dt
import io
import json
import sys
from pathlib import Path

from PIL import Image
from playwright.sync_api import sync_playwright

D = Path(__file__).resolve().parent / "data" / "glossary"
rec = json.loads((D / "recall.json").read_text())
ver = json.loads((D / "verify.json").read_text())
sth = json.loads((D / "sth.json").read_text())["sth"]
f = rec["facts"][0]
r = rec["receipt"]

day = dt.datetime.fromtimestamp(f["tslot"] * 86_400, dt.timezone.utc).date()
assert day.isoformat() == f["observed_at"][:10], (day, f["observed_at"])
assert len(f["fact_cid"]) == 52 and f["fact_cid"] in r["fact_cids"]
assert f["memory_token"] == f"emem:fact:{f['cell']}:{f['fact_cid']}"
assert f["cell"] in r["cells"]
assert rec["bands_cid"] == r["source_versions"]["bands_cid"]
assert ver["valid"] and ver["signature_valid"] and ver["preimage_version"] == r["preimage_version"]
assert r["responder_pubkey_b32"] == f["signer_pubkey_b32"] == sth["responder_pubkey_b32"]
prov = f["band_metadata"]["provenance"]["class"]
assert prov == "deterministic_index"
n_facts = len(rec["facts"])
assert n_facts == len(r["fact_cids"])


def short(s, a=10, b=6):
    return s if len(s) <= a + b + 1 else s[:a] + "…" + s[-b:]


def row(term, value, gloss):
    return (f'<div class="row"><div class="term">{term}</div>'
            f'<div class="mono val">{value}</div><div class="gloss">{gloss}</div></div>')


groups = [
    ("Places and time", [
        row("cell, cell64", f["cell"], "One square of about 10 m. Here: Mount Fuji."),
        row("tslot", f'{f["tslot"]}', f"A whole number for a period. Here: the day {day:%d %B %Y}."),
    ]),
    ("Facts", [
        row("band", f["band"], "The kind of measurement."),
        row("fact", f'{f["value"]:.4f}', "One signed observation: cell, band, tslot, value."),
        row("fact_cid", short(f["fact_cid"]), "The BLAKE3 hash of the fact's bytes, 52 characters."),
        row("provenance class", prov, "A formula over a cited Sentinel-2 scene. No caution."),
        row("manifest", "bands_cid " + short(rec["bands_cid"]), "The band list this answer used, by content id."),
    ]),
    ("Trust", [
        row("responder", "emem.dev · key " + short(r["responder_pubkey_b32"], 8, 4),
            "The server that answers and signs."),
        row("receipt", f'served_at {r["served_at"][11:19]} UTC', f"One signature over all {n_facts} facts in the answer."),
        row("preimage", "BLAKE3 " + short(ver["preimage_blake3_hex"], 8, 4),
            f"The bytes hashed before signing (version {r['preimage_version']})."),
        row("STH", f'tree_size {sth["tree_size"]:,}', "A signed snapshot of the transparency log."),
    ]),
    ("Tokens", [
        row("token", f"emem:fact:{f['cell']}:<br>{short(f['fact_cid'])}",
            "Another agent resolves it to the same signed bytes."),
    ]),
]
body = "".join(f'<div class="card"><div class="h">{h}</div>{"".join(rows)}</div>' for h, rows in groups)

W = 800
page = f"""<!doctype html><meta charset="utf-8"><style>
body{{margin:0;background:#0d1117;color:#e6edf3;font:28px/1.3 "DejaVu Sans",system-ui,sans-serif;width:{W}px}}
.wrap{{padding:40px}}
h1{{font-size:42px;line-height:1.2;margin:0 0 10px}}
.q{{color:#8b949e;font-size:24px;margin-bottom:22px}}
.mono{{font-family:"DejaVu Sans Mono",ui-monospace,monospace}}
.card{{background:#161b22;border:1px solid #30363d;border-radius:10px;padding:16px 24px 8px;margin-bottom:20px}}
.h{{font-size:24px;color:#8b949e;text-transform:uppercase;letter-spacing:.06em;margin-bottom:6px}}
.row{{padding:12px 0;border-top:1px solid #30363d}}
.h + .row{{border-top:none}}
.term{{display:inline-block;font-size:26px;color:#d2a8ff;border:1px solid #30363d;border-radius:6px;padding:1px 10px;
background:#0d1117;margin-bottom:6px}}
.val{{font-size:27px;color:#79c0ff;word-break:break-all}}
.gloss{{font-size:24px;color:#c9d1d9;margin-top:2px}}
.foot{{color:#8b949e;font-size:22px;margin-top:6px;line-height:1.4}}
</style><div class="wrap">
<h1>The glossary on one live answer</h1>
<div class="q">One recall of NDVI at the Mount Fuji cell on emem.dev. Each term names one part of that answer.</div>
{body}
<div class="foot">Live reads from emem.dev on 7 November 2026 at 16:03 UTC: POST /v1/recall (indices.ndvi, first of
{n_facts} facts), POST /v1/verify_receipt (valid), GET /v1/log/sth. Definitions: docs/glossary.md. Checks:
docs/media/stills/src/glossary_map.py.</div>
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
