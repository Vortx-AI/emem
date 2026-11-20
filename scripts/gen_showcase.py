#!/usr/bin/env python3
"""The site's imagery: real Sentinel-2 scenes read by emem itself, each with its provenance.

    python3 scripts/gen_showcase.py            fetch every scene from /v1/scene.png and write web/img/
    python3 scripts/gen_showcase.py --check    every image the manifest names exists with its hash (no network)

Why this exists
---------------
The owner asked for a light, imagery-led site where the world comes first and the proof sits beside it. A stock
photo of a field proves nothing; a scene emem read, named by its Sentinel-2 item and capture time, is the same
evidence the product serves. So the images are generated, never edited by hand: this script asks
GET /v1/scene.png for each bbox below (native 10 m, up to 1024 px a side), records the scene headers the responder
returns (item id, capture time, cloud cover), writes a JPEG whose file name carries its content hash (so it can be
cached forever), and writes web/img/showcase.json. Pages cite the scene by that manifest.

Places are public and chosen on purpose. Not a military site (the first Punjab box held an air force station), not
a place whose honest number reads as an error on a hero (Fuji's 30 m grid mean is 3,768 m, the summit 3,776 m).

Needs Pillow (`/home/ubuntu/emem-ops/venv-img/bin/python` has it); --check needs nothing but the standard library.
"""
import argparse, hashlib, io, json, sys, time, urllib.parse, urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
IMG = ROOT / "web" / "img"
MANIFEST = IMG / "showcase.json"
ORIGIN = "https://emem.dev"

# name, what the picture shows, bbox w,s,e,n, datetime window (or None for the latest), max cloud %
SCENES = [
    ("punjab-sep", "Field mosaic in Punjab at the height of the kharif season",
     "75.30,30.42,75.39,30.50", "2026-08-20/2026-09-30", 10),
    ("punjab-nov", "The same Punjab fields after harvest, with burnt stubble",
     "75.30,30.42,75.39,30.50", "2026-11-10/2026-11-20", 5),
    ("rondonia", "Forest edge and cleared land in Rondonia, Brazil",
     "-63.21,-10.10,-63.13,-10.03", "2026-09-01/2026-09-30", 2),
    ("bhakra", "Gobind Sagar reservoir behind the Bhakra dam",
     "76.40,31.38,76.50,31.46", "2026-11-10/2026-11-20", 5),
    ("sundarbans", "Mangrove channels of the Sundarbans",
     "89.06,21.85,89.14,21.92", "2026-11-10/2026-11-20", 5),
    ("delhi", "Delhi along the Yamuna",
     "77.19,28.58,77.29,28.66", "2026-11-05/2026-11-20", 5),
]
QUALITY = 84


def fetch(name, bbox, window, max_cloud):
    q = {"bbox": bbox, "max_cloud": str(max_cloud)}
    if window:
        q["datetime"] = window
    url = f"{ORIGIN}/v1/scene.png?{urllib.parse.urlencode(q)}"
    req = urllib.request.Request(url, headers={"user-agent": "emem-gen-showcase/1"})
    with urllib.request.urlopen(req, timeout=180) as r:
        h = {k.lower(): v for k, v in r.headers.items()}
        return r.read(), {
            "item_id": h.get("x-emem-scene-item-id"),
            "datetime": h.get("x-emem-scene-datetime"),
            "cloud_cover_pct": float(h.get("x-emem-scene-cloud-cover") or "nan"),
            "pixel_size_m": h.get("x-emem-scene-pixel-size"),
            "epsg": h.get("x-emem-scene-epsg"),
            "request": url,
        }


def build():
    from PIL import Image  # only the build needs it
    IMG.mkdir(parents=True, exist_ok=True)
    old = {p.name for p in IMG.glob("*.jpg")}
    out = {"generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), "origin": ORIGIN,
           "note": "Sentinel-2 L2A true colour, read by emem at native 10 m via GET /v1/scene.png", "scenes": {}}
    for name, caption, bbox, window, mc in SCENES:
        png, meta = fetch(name, bbox, window, mc)
        im = Image.open(io.BytesIO(png)).convert("RGB")
        buf = io.BytesIO()
        im.save(buf, "JPEG", quality=QUALITY, optimize=True, progressive=True)
        data = buf.getvalue()
        digest = hashlib.sha256(data).hexdigest()[:10]
        fname = f"{name}-{digest}.jpg"
        (IMG / fname).write_bytes(data)
        out["scenes"][name] = {"file": fname, "sha256_10": digest, "caption": caption, "bbox": bbox,
                               "width": im.width, "height": im.height, "bytes": len(data), **meta}
        print(f"{name}: {fname} {im.width}x{im.height} {len(data) // 1024} KB, {meta['item_id']} {meta['datetime']}")
    MANIFEST.write_text(json.dumps(out, indent=1) + "\n")
    keep = {s["file"] for s in out["scenes"].values()}
    for stale in old - keep:
        (IMG / stale).unlink()
        print(f"removed stale {stale}")


def check():
    if not MANIFEST.exists():
        print("no web/img/showcase.json; run scripts/gen_showcase.py"); return 1
    m = json.loads(MANIFEST.read_text())
    bad = 0
    for name, s in m["scenes"].items():
        f = IMG / s["file"]
        if not f.exists():
            print(f"{name}: {s['file']} missing"); bad += 1; continue
        d = hashlib.sha256(f.read_bytes()).hexdigest()[:10]
        if d != s["sha256_10"] or not s["file"].endswith(f"-{d}.jpg"):
            print(f"{name}: {s['file']} does not hash to its name ({d})"); bad += 1
    names = {s["file"] for s in m["scenes"].values()}
    for p in IMG.glob("*.jpg"):
        if p.name not in names and not p.name.startswith("poster-"):
            print(f"{p.name}: on disk but not in the manifest"); bad += 1
    print(f"showcase: {len(m['scenes'])} scenes, {'all present and named by their hash' if not bad else f'{bad} problem(s)'}")
    return 1 if bad else 0


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true")
    a = ap.parse_args()
    sys.exit(check() if a.check else (build() or 0))
