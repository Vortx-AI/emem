"""Write season-data.js for emem-ndvi-season.html from saved live responses (no network).

Inputs, all saved from emem.dev on 7 November 2026:
  ../../stills/src/data/field_series.json   POST /v1/field_series, bbox -93.700,42.000,-93.694,42.004 (Ames, Iowa)
  ../../stills/src/data/verify_receipt.json POST /v1/verify_receipt on that answer's receipt
  data/season/<date>.png + .headers          GET /v1/scene.png, one per row (fetch_season.py)
The script checks that every crop comes from its row's scene and shares one UTM grid, then projects the box and
the 6 x 6 block quads of the answer into crop pixels.
usage: python season_data.py      (needs pyproj)
"""
import json
from pathlib import Path

from pyproj import Transformer

HERE = Path(__file__).resolve().parent
STILL = HERE.parent.parent / "stills" / "src" / "data"
SEASON = HERE / "data" / "season"
fs = json.loads((STILL / "field_series.json").read_text())
vr = json.loads((STILL / "verify_receipt.json").read_text())
assert vr["signature_valid"] is True and vr["merkle_proof_valid"] is True, vr


def headers(date):
    out = {}
    for line in (SEASON / f"{date}.headers").read_text().splitlines():
        k, v = line.split(":", 1)
        out[k.strip().lower()] = v.strip()
    return out


grid = None
rows = []
for r in fs["rows"]:
    h = headers(r["date"])
    assert h["x-emem-scene-item-id"] == r["scene"], (r["date"], h["x-emem-scene-item-id"])
    g = (h["x-emem-scene-epsg"], h["x-emem-scene-bbox-crs"])
    assert grid in (None, g), (grid, g)
    grid = g
    rows.append({"date": r["date"], "median": r["median"], "n_clear": r["n_clear"], "quality": r["quality"],
                 "blocks": r["block_medians"], "sat": r["scene"][:3],
                 "img": f"data/season/{r['date']}.png"})

epsg, bb = grid
x0, y0, x1, y1 = map(float, bb.split(","))
W, H = 149, 133
to_utm = Transformer.from_crs(4326, int(epsg), always_xy=True)


def px(lng, lat):
    e, n = to_utm.transform(lng, lat)
    return [round((e - x0) / (x1 - x0) * W, 2), round((y1 - n) / (y1 - y0) * H, 2)]


box = [px(-93.700, 42.000), px(-93.694, 42.000), px(-93.694, 42.004), px(-93.700, 42.004)]
quads = [[px(*p) for p in q[0][:4]] for q in fs["blocks"]["quads"]]
assert fs["blocks"]["per_side"] == 6 and len(quads) == 36

data = {
    "w": W, "h": H, "box": box, "quads": quads, "rows": rows,
    "excluded": [{"date": e["date"], "reason": e["reason"]} for e in fs["excluded"]],
    "token": fs["token"],
    "candidates": fs["selection"]["candidate_dates"],
    "preimage": vr["preimage_blake3_hex"],
}
(HERE / "season-data.js").write_text("window.SEASON = " + json.dumps(data, separators=(",", ":")) + ";\n")
print("rows", len(rows), "excluded", len(data["excluded"]), "grid", grid)
