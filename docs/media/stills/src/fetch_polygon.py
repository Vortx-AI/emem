"""Fetch the live read behind area-receipts.webp. Read-only calls to emem.dev.

One call to POST /v1/recall_polygon: the Copernicus DEM elevation band at 16 cells in a box of about 7.5 km2
over the Mount Fuji summit, with budget_ms 20000. On 7 November 2026 this exact call was made twice:
  data/polygon/first.json   16:08:38 UTC, the cells were cold: 0 facts, 16 pending entries
  data/polygon/second.json  16:09:28 UTC, 16 facts and 16 receipts
The cells are warm now, so a new call writes only data/polygon/again.json and leaves both files as they are.
The receipts are checked offline by area_receipts.py.
usage: python3 fetch_polygon.py
"""
import json
import urllib.request
from pathlib import Path

D = Path(__file__).resolve().parent / "data" / "polygon"
D.mkdir(parents=True, exist_ok=True)
BODY = {"polygon_bbox": {"min_lat": 35.35, "max_lat": 35.375, "min_lng": 138.715, "max_lng": 138.745},
        "bands": ["copdem30m.elevation_mean"], "max_cells": 16, "budget_ms": 20000}

req = urllib.request.Request("https://emem.dev/v1/recall_polygon", data=json.dumps(BODY).encode(),
                             headers={"content-type": "application/json", "user-agent": "emem-docs-still"})
with urllib.request.urlopen(req, timeout=120) as r:
    out = json.load(r)
(D / "again.json").write_text(json.dumps(out, indent=1))
print(out["cells_sampled"], "cells,", out["facts_returned"], "facts,", len(out.get("pending") or []),
      "pending, converged", out["converged"])
