"""Fetch the live reads behind glossary-map.webp. Read-only calls to emem.dev.

One recall of NDVI at the Mount Fuji cell, /v1/verify_receipt on its receipt, and the current signed tree head.
usage: python fetch_glossary.py    (writes data/glossary/*.json)
"""
import json
import urllib.request
from pathlib import Path

D = Path(__file__).resolve().parent / "data" / "glossary"
D.mkdir(parents=True, exist_ok=True)
API = "https://emem.dev"
CELL = "defi.zb592.nUkO.zEzE"


def call(path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(f"{API}{path}", data=data, headers={"content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.load(r)


rec = call("/v1/recall", {"cell": CELL, "bands": ["indices.ndvi"]})
(D / "recall.json").write_text(json.dumps(rec, indent=1))
(D / "verify.json").write_text(json.dumps(call("/v1/verify_receipt", {"receipt": rec["receipt"]}), indent=1))
(D / "sth.json").write_text(json.dumps(call("/v1/log/sth"), indent=1))
print("wrote", sorted(p.name for p in D.glob("*.json")))
