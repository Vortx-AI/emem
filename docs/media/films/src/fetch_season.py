"""Fetch one Sentinel-2 true-colour crop per row of the saved field_series answer (read-only GETs).

Input: ../../stills/src/data/field_series.json (POST /v1/field_series near Ames, Iowa, saved 7 November 2026).
Output: data/season/<date>.png and <date>.headers. Each crop must come from the row's own scene; the script
checks the x-emem-scene-item-id header against the row and does not save a crop from another scene.
usage: python fetch_season.py
"""
import datetime as dt
import json
import time
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
FS = HERE.parent.parent / "stills" / "src" / "data" / "field_series.json"
OUT = HERE / "data" / "season"
OUT.mkdir(parents=True, exist_ok=True)
BBOX = "-93.706,41.996,-93.688,42.008"

for row in json.loads(FS.read_text())["rows"]:
    png, hdr = OUT / f"{row['date']}.png", OUT / f"{row['date']}.headers"
    if png.exists():
        continue
    d = dt.date.fromisoformat(row["date"])
    win = f"{d - dt.timedelta(days=1)}/{d + dt.timedelta(days=1)}"
    url = f"https://emem.dev/v1/scene.png?bbox={BBOX}&datetime={win}&max_cloud=100"
    req = urllib.request.Request(url, headers={"User-Agent": "emem-docs-media/1"})
    with urllib.request.urlopen(req, timeout=120) as r:
        body, headers = r.read(), r.headers
    item = headers.get("x-emem-scene-item-id")
    print(row["date"], item, len(body))
    if item != row["scene"]:
        print("  scene differs from the field_series row; not saved:", row["scene"])
        continue
    png.write_bytes(body)
    hdr.write_text("".join(f"{k}: {v}\n" for k, v in headers.items() if k.lower().startswith("x-emem-")))
    time.sleep(1.5)
