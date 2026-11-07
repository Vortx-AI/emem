"""Save the live reads behind change_ledger.py. Read-only: it resolves tokens and checks a receipt.

The ledger itself is a stored derivative fact (band change_attribution.ledger). It was signed when
POST /v1/change_attribution first ran at this cell. That call stores a fact, so this script does not
make it. It resolves the stored ledger by its token, then resolves the two NDVI facts the ledger cites.
usage: python fetch_change.py
"""
import json
import urllib.request
from pathlib import Path

BASE = "https://emem.dev"
CELL = "defi.zb493.zezo.zcb35"
LEDGER = "ssd2ueqbvtnpfya4qhy6fsl4bamj5qap3mm5ll4vkirmwsd2m5jq"
D = Path(__file__).resolve().parent / "data" / "change"


def post(path, body):
    req = urllib.request.Request(
        BASE + path, json.dumps(body).encode(), {"content-type": "application/json", "user-agent": "emem-docs-media"}
    )
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.loads(r.read())


def resolve(cid):
    return post("/v1/memory_token/resolve", {"token": f"emem:fact:{CELL}:{cid}"})


D.mkdir(parents=True, exist_ok=True)
ledger = resolve(LEDGER)
(D / "ledger.json").write_text(json.dumps(ledger, indent=1))
ndvi = ledger["value"]["terms"]["env"]["evidence"][0]
assert ndvi["band"] == "indices.ndvi"
# change_attribution.rs writes each pair as [now, prev].
for name, cid in zip(("ndvi_now", "ndvi_prev"), ndvi["fact_cids"]):
    (D / f"{name}.json").write_text(json.dumps(resolve(cid), indent=1))
check = post("/v1/verify_receipt", {"receipt": ledger["receipt"]})
(D / "ledger_verify.json").write_text(json.dumps(check, indent=1))
print("saved", sorted(p.name for p in D.iterdir()))
