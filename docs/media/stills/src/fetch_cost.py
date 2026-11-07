"""Fetch the live reads behind citation-cost.webp. Read-only calls to emem.dev.

Input: data/glossary/recall.json (19 NDVI facts at the Mount Fuji cell, fetch_glossary.py).
For each fact, POST /v1/memory_token with band and observed_on. The handler composes strings and never reads
or writes storage. Then POST /v1/memory_token/resolve on the first descriptor token, to show that it resolves.
usage: python fetch_cost.py    (writes data/cost/*.json)
"""
import json
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
D = HERE / "data" / "cost"
D.mkdir(parents=True, exist_ok=True)
API = "https://emem.dev"


def call(path, body):
    req = urllib.request.Request(f"{API}{path}", data=json.dumps(body).encode(),
                                 headers={"content-type": "application/json", "user-agent": "emem-docs-still"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.load(r)


rec = json.loads((HERE / "data" / "glossary" / "recall.json").read_text())
tokens = []
for f in rec["facts"]:
    tokens.append(call("/v1/memory_token", {"cell": f["cell"], "fact_cid": f["fact_cid"], "band": f["band"],
                                            "observed_on": f["observed_at"][:10]}))
(D / "tokens.json").write_text(json.dumps(tokens, indent=1))
res = call("/v1/memory_token/resolve", {"token": tokens[0]["descriptor_token"]})
(D / "resolve.json").write_text(json.dumps(res, indent=1))
print("wrote", len(tokens), "tokens; resolve keys:", sorted(res)[:12])
