"""Fetch the live reads that contradictions.py draws.

Read-only calls to emem.dev:
  POST /v1/memory_contradictions {"limit": 40}   whole-corpus scan, keeps the two keys drawn
  GET  /v1/facts/<cid>                           the old and the new fact at each key
  POST /v1/recall                                which fact a read returns today, at each cell,
                                                 and at the road cell with as_of_signed_at 2026-09-01
  POST /v1/verify_receipt                        each receipt above
Usage: python3 fetch_contradictions.py   (writes data/contra/*.json)
"""

import json
import pathlib
import urllib.request

BASE = "https://emem.dev"
OUT = pathlib.Path(__file__).parent / "data" / "contra"
KEYS = {
    "road": ("defi.zb27e.qEfa.ze033", "overture.transportation.road_length_m"),
    "tree": ("defi.zb317.cUyi.dAqO", "forest_change.treecover2000"),
}


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
    scan = call("/v1/memory_contradictions", {"limit": 40})
    keep = {}
    for name, (cell, band) in KEYS.items():
        rec = next(c for c in scan["contradictions"] if c["cell"] == cell and c["band"] == band)
        keep[name] = rec
        atts = sorted(rec["attestations"], key=lambda a: a["signed_at"])
        save(f"{name}_old", call("/v1/facts/" + atts[0]["fact_cid"]))
        save(f"{name}_new", call("/v1/facts/" + atts[-1]["fact_cid"]))
        rc = call("/v1/recall", {"cell": cell, "bands": [band]})
        save(f"{name}_recall", rc)
        save(f"{name}_recall_verify", call("/v1/verify_receipt", {"receipt": rc["receipt"]}))
        print(name, [a["value"] for a in atts], "recall", [f["value"] for f in rc["facts"]])
    cell, band = KEYS["road"]
    asof = {"cell": cell, "bands": [band], "as_of_signed_at": "2026-09-01T00:00:00Z"}
    rc = call("/v1/recall", asof)
    save("road_recall_asof", rc)
    save("road_recall_asof_verify", call("/v1/verify_receipt", {"receipt": rc["receipt"]}))
    print("road as_of 2026-09-01", [f["value"] for f in rc["facts"]])
    scan_summary = {
        "agent_hint": scan["agent_hint"],
        "corpus_scanned": scan["corpus_scanned"],
        "returned": len(scan["contradictions"]),
        "scopes": sorted({c["disagreement_scope"] for c in scan["contradictions"]}),
        "receipt": scan["receipt"],
        "kept": keep,
    }
    save("scan", scan_summary)
    save("scan_verify", call("/v1/verify_receipt", {"receipt": scan["receipt"]}))
    print("scanned", scan["corpus_scanned"], "scopes", scan_summary["scopes"])


if __name__ == "__main__":
    main()
