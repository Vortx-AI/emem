"""Fetch the live reads behind time-filters-live.webp. Read-only calls to emem.dev.

Four recalls of NDVI at the Mount Fuji cell: no filter, as_of_tslot only, as_of_signed_at only, both.
Then /v1/verify_receipt on the receipt of the read with both filters.
usage: python fetch_asof.py    (writes data/asof/*.json)
"""
import json
import urllib.request
from pathlib import Path

D = Path(__file__).resolve().parent / "data" / "asof"
D.mkdir(parents=True, exist_ok=True)
API = "https://emem.dev/v1"
CELL = "defi.zb592.nUkO.zEzE"
TSLOT = 20589
SIGNED = "2026-10-01T00:00:00Z"


def post(path, body):
    req = urllib.request.Request(f"{API}/{path}", data=json.dumps(body).encode(),
                                 headers={"content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.load(r)


base = {"cell": CELL, "bands": ["indices.ndvi"]}
reads = {
    "none": base,
    "tslot": {**base, "as_of_tslot": TSLOT},
    "signed": {**base, "as_of_signed_at": SIGNED},
    "both": {**base, "as_of_tslot": TSLOT, "as_of_signed_at": SIGNED},
}
for name, body in reads.items():
    (D / f"{name}.json").write_text(json.dumps(post("recall", body), indent=1))
both = json.loads((D / "both.json").read_text())
(D / "verify_both.json").write_text(json.dumps(post("verify_receipt", {"receipt": both["receipt"]}), indent=1))
print("wrote", sorted(p.name for p in D.glob("*.json")))
