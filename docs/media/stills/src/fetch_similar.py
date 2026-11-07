"""Fetch the data for similar_places.py: one Sentinel-2 crop per place, and a receipt check.

Reads data/similar/find_similar.json, a live answer saved on 7 November 2026 from
  POST https://emem.dev/v1/find_similar {"key":"Dubai","k":8,"band":"geotessera","mode":"hamming_then_rerank"}
Then, for the seed and each neighbour, it calls
  GET /v1/scene.png?bbox=<about 2 km around the point>&datetime=2024-03-01/2024-10-31&max_cloud=5
and keeps the PNG and its x-emem-scene-* headers. It also calls POST /v1/verify_receipt once.
These are read-only calls. usage: python3 fetch_similar.py
"""
import json
import math
import urllib.request
from pathlib import Path

D = Path(__file__).resolve().parent / "data" / "similar"
BASE = "https://emem.dev"
fs = json.loads((D / "find_similar.json").read_text())
seed = fs["resolved_from"]["key"]
points = [("seed", seed["lat"], seed["lng"])]
points += [(f"n{i}", n["lat"], n["lng"]) for i, n in enumerate(fs["neighbors"])]

for name, lat, lng in points:
    dlat = 0.009
    dlng = dlat / math.cos(math.radians(lat))
    bbox = f"{lng - dlng:.5f},{lat - dlat:.5f},{lng + dlng:.5f},{lat + dlat:.5f}"
    url = f"{BASE}/v1/scene.png?bbox={bbox}&datetime=2024-03-01/2024-10-31&max_cloud=5"
    with urllib.request.urlopen(url, timeout=120) as r:
        (D / f"{name}.png").write_bytes(r.read())
        hdr = {k.lower(): v for k, v in r.headers.items() if k.lower().startswith("x-emem-scene-")}
    hdr["request-bbox"] = bbox
    (D / f"{name}.headers.json").write_text(json.dumps(hdr, indent=1))
    print(name, hdr.get("x-emem-scene-item-id"), hdr.get("x-emem-scene-datetime"))

req = urllib.request.Request(
    f"{BASE}/v1/verify_receipt",
    data=json.dumps({"receipt": fs["receipt"]}).encode(),
    headers={"content-type": "application/json"},
)
with urllib.request.urlopen(req, timeout=60) as r:
    (D / "verify_receipt.json").write_bytes(r.read())
print(json.loads((D / "verify_receipt.json").read_text()).get("valid"))
