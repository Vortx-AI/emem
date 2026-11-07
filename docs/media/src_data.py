"""Pack, unpack and refetch the saved emem.dev responses that the stills, films and diagrams draw.

The renderers in stills/src/, films/src/ and diagrams/src/ read plain files under their data/ folders. Those folders are not
in git. The repo keeps one gzip archive per folder in src-packs/, and a list of the Sentinel-2 scene PNGs with
the emem.dev URL and SHA-256 of each, in src-packs/scenes.json. The PNGs are not in git.

usage (from any directory, Python 3.9 or later, Pillow only for `scenes`):
  python3 docs/media/src_data.py unpack     # src-packs/*.tar.gz -> the three data/ folders
  python3 docs/media/src_data.py scenes     # GET each scene PNG from emem.dev, compare its SHA-256
  python3 docs/media/src_data.py pack       # data/ folders -> src-packs/ (maintainers, after a refetch)
  python3 docs/media/src_data.py manifest   # rebuild scenes.json from the PNGs in data/ (maintainers)
`scenes` makes read-only GET calls, one per PNG (31 calls), with a pause between calls.
"""
import datetime as dt
import gzip
import hashlib
import io
import json
import sys
import tarfile
import time
import urllib.request
from pathlib import Path

MEDIA = Path(__file__).resolve().parent
ROOTS = {"stills": MEDIA / "stills" / "src" / "data", "films": MEDIA / "films" / "src" / "data",
         "diagrams": MEDIA / "diagrams" / "src" / "data"}
PACKS = MEDIA / "src-packs"
MANIFEST = PACKS / "scenes.json"
API = "https://emem.dev"


def groups(root):
    """One group per subfolder; loose files at the top level form the group '_top'."""
    out = {}
    for path in sorted(root.rglob("*")):
        if path.is_file() and path.suffix != ".png":
            rel = path.relative_to(root)
            out.setdefault(rel.parts[0] if len(rel.parts) > 1 else "_top", []).append(rel)
    return out


def pack():
    PACKS.mkdir(exist_ok=True)
    for kind, root in ROOTS.items():
        for name, files in groups(root).items():
            raw = io.BytesIO()
            with tarfile.open(fileobj=raw, mode="w", format=tarfile.PAX_FORMAT) as tar:
                for rel in files:
                    data = (root / rel).read_bytes()
                    info = tarfile.TarInfo(rel.as_posix())
                    info.size, info.mode, info.mtime = len(data), 0o644, 0
                    tar.addfile(info, io.BytesIO(data))
            out = PACKS / f"{kind}-{name}.tar.gz"
            with open(out, "wb") as f, gzip.GzipFile(fileobj=f, mode="wb", compresslevel=9, mtime=0) as gz:
                gz.write(raw.getvalue())
            print(f"{out.name}: {len(files)} files, {out.stat().st_size:,} bytes")


def members():
    """Yield (path under docs/media, bytes) for every file in every pack. check_media.py uses this."""
    for archive in sorted(PACKS.glob("*.tar.gz")):
        kind = archive.name.split("-", 1)[0]
        with tarfile.open(archive, "r:gz") as tar:
            for m in tar.getmembers():
                if m.isfile():
                    yield ROOTS[kind] / m.name, tar.extractfile(m).read()


def unpack():
    count = 0
    for path, data in members():
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        count += 1
    print(f"wrote {count} files into the data/ folders")


def scene_urls():
    """The request behind each saved scene PNG, as the fetch scripts and renderer docstrings record it."""
    s, f = ROOTS["stills"], ROOTS["films"]
    iowa = "-93.706,41.996,-93.688,42.008"
    cell = json.loads((s / "a2a" / "fact.json").read_text())["cell"]
    yield s / "a2a/scene.png", f"/v1/cells/{cell}/scene.png?datetime=2024-01-01/2024-03-31&max_cloud=5", None
    yield s / "hero/scene.png", ("/v1/scene.png?bbox=138.711,35.347,138.751,35.379"
                                 "&datetime=2025-11-01/2025-12-31&max_cloud=10"), None
    yield s / "recall/scene-420.png", ("/v1/scene.png?bbox=6.49,46.437,6.62,46.527"
                                       "&datetime=2026-07-01/2026-10-31&max_cloud=5"), 420
    for day, win in (("2024-08-01", "2024-07-31/2024-08-02"), ("2024-10-22", "2024-10-21/2024-10-23")):
        yield s / f"scene-{day}.png", f"/v1/scene.png?bbox={iowa}&datetime={win}&max_cloud=50", None
    fb = json.loads((s / "fields" / "field_boundaries.json").read_text())
    big = [g for g in fb["geojson"]["features"] if g["properties"]["area_m2"] > 10000]
    xs = [x for g in big for x, _ in g["geometry"]["coordinates"][0]]
    ys = [y for g in big for _, y in g["geometry"]["coordinates"][0]]
    bbox = f"{min(xs) - 0.0015:.4f},{min(ys) - 0.0015:.4f},{max(xs) + 0.0015:.4f},{max(ys) + 0.0015:.4f}"
    yield s / "fields/scene-2025-06-22.png", (f"/v1/scene.png?bbox={bbox}"
                                              "&datetime=2025-06-21/2025-06-23&max_cloud=50"), None
    for hdr in sorted((s / "similar").glob("*.headers.json")):
        box = json.loads(hdr.read_text())["request-bbox"]
        name = hdr.name.split(".")[0]
        yield s / f"similar/{name}.png", f"/v1/scene.png?bbox={box}&datetime=2024-03-01/2024-10-31&max_cloud=5", None
    for png in sorted((f / "season").glob("*.png")):
        day = dt.date.fromisoformat(png.stem)
        win = f"{day - dt.timedelta(days=1)}/{day + dt.timedelta(days=1)}"
        yield png, f"/v1/scene.png?bbox={iowa}&datetime={win}&max_cloud=100", None


def manifest():
    rows = []
    for path, url, resize in scene_urls():
        data = path.read_bytes()
        row = {"path": path.relative_to(MEDIA).as_posix(), "url": API + url,
               "sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)}
        if resize:
            row["resize_lanczos"] = resize
        rows.append(row)
    PACKS.mkdir(exist_ok=True)
    MANIFEST.write_text(json.dumps(rows, indent=1) + "\n")
    print(f"{MANIFEST.name}: {len(rows)} scenes")


def scenes():
    from PIL import Image
    same = 0
    rows = json.loads(MANIFEST.read_text())
    for row in rows:
        req = urllib.request.Request(row["url"], headers={"User-Agent": "emem-docs-media/1"})
        with urllib.request.urlopen(req, timeout=180) as r:
            data = r.read()
        if "resize_lanczos" in row:
            n = row["resize_lanczos"]
            buf = io.BytesIO()
            Image.open(io.BytesIO(data)).convert("RGB").resize((n, n), Image.LANCZOS).save(buf, "PNG")
            data = buf.getvalue()
        path = MEDIA / row["path"]
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        match = hashlib.sha256(data).hexdigest() == row["sha256"]
        same += match
        print(("same   " if match else "differs"), row["path"])
        time.sleep(1.0)
    print(f"{same} of {len(rows)} scenes have the saved SHA-256")


if __name__ == "__main__":
    cmds = {"pack": pack, "unpack": unpack, "manifest": manifest, "scenes": scenes}
    if len(sys.argv) != 2 or sys.argv[1] not in cmds:
        sys.exit(__doc__)
    cmds[sys.argv[1]]()
