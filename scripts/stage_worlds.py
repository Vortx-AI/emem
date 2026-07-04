#!/usr/bin/env python3
"""Stage the repo's committed splat scenes into EMEM_WORLDS_DIR.

`examples/3d-worlds/scenes/` ships each README world's fetched scene,
exported .ply/.splat, and provenance receipts. This script (stdlib only)
lays them out the way the /v1/worlds handlers serve them, and writes the
meta.json the list endpoint and the /worlds page read:

    var/worlds/<preset>/world.ply
    var/worlds/<preset>/world.splat
    var/worlds/<preset>/world.scene.json
    var/worlds/<preset>/world.provenance.json
    var/worlds/<preset>/meta.json

It is the offline complement to scripts/bake_worlds.sh: an operator who
just cloned the repo gets a working /worlds without a single upstream
fetch, then the weekly bake (or a manual one) refreshes the scenes in
place. Artifact hashes in meta.json are computed from the staged bytes,
so the page's browser-side sha256 check works even for scenes exported
before the provenance carried a scene hash.

Usage: python3 scripts/stage_worlds.py [--worlds-dir var/worlds]
"""

import argparse
import datetime
import hashlib
import json
import os
import shutil
import sys
import tempfile

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SCENES = os.path.join(REPO, "examples", "3d-worlds", "scenes")

TITLES = {
    "canyon": "Grand Canyon · elevation",
    "interlaken": "Interlaken · elevation + vegetation + water",
    "semantic": "Cairo and Giza · what the foundation model sees",
    "carbon": "Rondônia · standing carbon and the year it fell",
}


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def stage(preset, worlds_dir):
    src = {
        "world.ply": os.path.join(SCENES, preset + ".ply"),
        "world.splat": os.path.join(SCENES, preset + ".splat"),
        "world.scene.json": os.path.join(SCENES, preset + ".scene.json"),
        "world.provenance.json": os.path.join(SCENES, preset + ".provenance.json"),
    }
    for p in src.values():
        if not os.path.exists(p):
            print("skip %s: missing %s" % (preset, p), file=sys.stderr)
            return False

    stage_dir = tempfile.mkdtemp(prefix=".stage-%s-" % preset, dir=worlds_dir)
    for name, p in src.items():
        shutil.copyfile(p, os.path.join(stage_dir, name))

    prov = json.load(open(src["world.provenance.json"]))
    scene = json.load(open(src["world.scene.json"]))
    artifacts = {}
    for kind, name in (("ply", "world.ply"), ("splat", "world.splat"),
                       ("scene", "world.scene.json")):
        staged = os.path.join(stage_dir, name)
        artifacts[kind] = {
            "file": name,
            "sha256": sha256_file(staged),
            "bytes": os.path.getsize(staged),
        }
    meta = {
        "preset": preset,
        "title": TITLES.get(preset, preset),
        "baked_at": datetime.datetime.now(datetime.timezone.utc)
        .strftime("%Y-%m-%dT%H:%M:%SZ"),
        "staged_from": "examples/3d-worlds/scenes",
        "cells": len(scene["cells"]),
        "facts": scene["fact_count"],
        "splats": len(prov.get("splats", [])),
        "bands": prov.get("bands", []),
        "bbox": prov.get("bbox"),
        "responder_pubkey_b32": prov.get("responder_pubkey_b32"),
        "artifacts": artifacts,
    }
    with open(os.path.join(stage_dir, "meta.json"), "w") as f:
        json.dump(meta, f, indent=1)

    # Same atomic swap discipline as bake_worlds.sh: readers see the old
    # world or the new one, never a mixture.
    live = os.path.join(worlds_dir, preset)
    old = os.path.join(worlds_dir, ".old-%s-%d" % (preset, os.getpid()))
    if os.path.isdir(live):
        os.rename(live, old)
    os.rename(stage_dir, live)
    shutil.rmtree(old, ignore_errors=True)
    print("staged %s: %d splats, scene %d KB" %
          (preset, meta["splats"], artifacts["scene"]["bytes"] // 1024))
    return True


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--worlds-dir",
                    default=os.environ.get("EMEM_WORLDS_DIR",
                                           os.path.join(REPO, "var", "worlds")))
    args = ap.parse_args()
    os.makedirs(args.worlds_dir, exist_ok=True)
    ok = [p for p in sorted(TITLES) if stage(p, args.worlds_dir)]
    print("staged %d/%d worlds into %s" % (len(ok), len(TITLES), args.worlds_dir))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
