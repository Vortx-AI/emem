"""Fetch the live reads behind receipt-preimage.webp. Read-only calls to emem.dev.

One recall of the Copernicus DEM elevation band at the Mount Fuji cell, then /v1/verify_receipt on its
receipt, and GET /.well-known/emem.json for the published responder key.
usage: python fetch_receipt.py    (writes data/receipt/*.json)
"""
import json
import urllib.request
from pathlib import Path

D = Path(__file__).resolve().parent / "data" / "receipt"
D.mkdir(parents=True, exist_ok=True)
API = "https://emem.dev"
CELL = "defi.zb592.nUkO.zEzE"


def call(path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(f"{API}{path}", data=data, headers={"content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.load(r)


rec = call("/v1/recall", {"cell": CELL, "bands": ["copdem30m.elevation_mean"]})
(D / "recall.json").write_text(json.dumps(rec, indent=1))
(D / "verify.json").write_text(json.dumps(call("/v1/verify_receipt", {"receipt": rec["receipt"]}), indent=1))
(D / "well_known.json").write_text(json.dumps(call("/.well-known/emem.json"), indent=1))
print("wrote", sorted(p.name for p in D.glob("*.json")))
