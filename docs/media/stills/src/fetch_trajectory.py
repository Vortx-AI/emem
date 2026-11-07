"""Fetch the live reads that trajectory.py draws.

Read-only calls to emem.dev:
  POST /v1/trajectory      the harvester call in docs/robots.md (Bengaluru cell, indices.ndvi,
                           window [20325, 20690]), and Central Park with a window that ends before
                           its only stored NDVI fact
  GET  /v1/facts/<cid>     the body of one point in the Bengaluru series
  POST /v1/verify_receipt  both trajectory receipts
Usage: python3 fetch_trajectory.py   (writes data/trajectory/*.json)
"""

import json
import pathlib
import urllib.request

BASE = "https://emem.dev"
OUT = pathlib.Path(__file__).parent / "data" / "trajectory"
HARVEST = {"cell": "defi.zb493.xuqA.zcb5f", "band": "indices.ndvi", "window": [20325, 20690]}
EMPTY = {"place": "Central Park", "band": "indices.ndvi", "from_date": "2025-01-01", "to_date": "2026-11-01"}


def call(path, body=None):
    req = urllib.request.Request(
        BASE + path,
        data=None if body is None else json.dumps(body).encode(),
        headers={"content-type": "application/json", "user-agent": "emem-docs-still"},
        method="GET" if body is None else "POST",
    )
    with urllib.request.urlopen(req, timeout=90) as r:
        return json.load(r)


def save(name, obj):
    (OUT / f"{name}.json").write_text(json.dumps(obj, indent=2))


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    tr = call("/v1/trajectory", HARVEST)
    save("harvest", tr)
    save("harvest_verify", call("/v1/verify_receipt", {"receipt": tr["receipt"]}))
    s = tr["series"]
    print("harvest", len(s), "points", s[0]["slot_start"], "to", s[-1]["slot_start"])
    pick = max(s, key=lambda r: r["value"] if r["value"] < 0.99 else -1)
    save("harvest_point", call("/v1/facts/" + pick["fact_cid"]))
    em = call("/v1/trajectory", EMPTY)
    save("empty", em)
    save("empty_verify", call("/v1/verify_receipt", {"receipt": em["receipt"]}))
    print("empty", len(em["series"]), em.get("empty_series_diag", {}).get("stored_tslots"))


if __name__ == "__main__":
    main()
