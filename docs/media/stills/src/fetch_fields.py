"""Live reads for field_outlines.py. Read-only calls to emem.dev; writes data/fields/.

usage: python3 fetch_fields.py
"""
import json
import subprocess
from pathlib import Path

OUT = Path(__file__).resolve().parent / "data" / "fields"
API = "https://emem.dev"
BOX = {"min_lat": 41.996, "max_lat": 42.008, "min_lng": -93.706, "max_lng": -93.688}
SCENE = "2025-06-22"


def post(route, body, timeout=280):
    r = subprocess.run(["curl", "-sf", "-m", str(timeout), "-X", "POST", API + route, "-H",
                        "content-type: application/json", "--data-binary", "@-"],
                       input=json.dumps(body), capture_output=True, text=True, check=True)
    return json.loads(r.stdout)


OUT.mkdir(parents=True, exist_ok=True)
fb = post("/v1/field_boundaries", {"polygon_bbox": BOX})
big = [f for f in fb["geojson"]["features"] if f["properties"]["area_m2"] > 10000]
fs = post("/v1/field_series", {
    "fields": [{"id": f["properties"]["polygon_cid"][:8], "geometry": f["geometry"]} for f in big],
    "start_date": "2025-01-01", "end_date": "2025-12-31", "index": "ndvi"})
vr = post("/v1/verify_receipt", {"receipt": fs["receipt"]}, 60)

xs = [x for f in big for x, _ in f["geometry"]["coordinates"][0]]
ys = [y for f in big for _, y in f["geometry"]["coordinates"][0]]
pad = 0.0015
bbox = f"{min(xs) - pad:.4f},{min(ys) - pad:.4f},{max(xs) + pad:.4f},{max(ys) + pad:.4f}"
day = int(SCENE[8:])
window = f"{SCENE[:8]}{day - 1:02d}/{SCENE[:8]}{day + 1:02d}"
subprocess.run(["curl", "-sf", "-m", "120", "-D", str(OUT / f"scene-{SCENE}.headers"),
                "-o", str(OUT / f"scene-{SCENE}.png"),
                f"{API}/v1/scene.png?bbox={bbox}&datetime={window}&max_cloud=50"], check=True)

for name, obj in (("field_boundaries", fb), ("field_series", fs), ("verify_receipt", vr)):
    (OUT / f"{name}.json").write_text(json.dumps(obj))
print(len(fb["geojson"]["features"]), "outlines,", len(big), "over 1 ha; verify_receipt valid:", vr["valid"])
print("scene bbox", bbox)
