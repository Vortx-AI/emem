#!/usr/bin/env bash
# Bake the four 3-D world presets into $EMEM_WORLDS_DIR so /worlds and
# /v1/worlds serve finished, signed artifacts from disk.
#
# Building a splat world is the slow part: a cold preset materializes and
# signs up to 1,024 cells x several bands, minutes of work that has also
# been implicated in responder stalls when browsers drive it directly
# (2026-07-03, twice). Delivery must not repeat the build. This script is
# the only place the build happens: it runs make_splats.py per preset
# against the local responder in gentle mode (small batches, spaced out),
# verifies every stored receipt, and only then swaps the finished
# directory into place atomically. The server never serves a half-baked
# world; a failed preset leaves the previous bake untouched.
#
# Run it manually after a redeploy, or let emem-worlds-bake.timer run it
# weekly to pick up fresh vintages. Warm re-bakes take a couple of
# minutes; the first bake of a cold preset can take much longer.
set -euo pipefail

REPO=/home/ubuntu/emem
EXPORTER="$REPO/examples/3d-worlds/make_splats.py"
WORLDS_DIR="${EMEM_WORLDS_DIR:-$REPO/var/worlds}"
RESPONDER="${EMEM_BAKE_RESPONDER:-http://127.0.0.1:5051}"
# Small batches + a pause between them keep a cold bake from starving
# interactive traffic on a shared responder (recall_many holds 16
# concurrent cell recalls per call; 32-cell batches finish fast enough
# that /health and real agents keep getting scheduled).
BATCH="${EMEM_BAKE_BATCH:-32}"
SLEEP="${EMEM_BAKE_SLEEP:-1}"
PRESETS=(${EMEM_BAKE_PRESETS:-canyon interlaken semantic carbon})

TITLES_canyon="Grand Canyon · elevation"
TITLES_interlaken="Interlaken · elevation + vegetation + water"
TITLES_semantic="Cairo and Giza · what the foundation model sees"
TITLES_carbon="Rondônia · standing carbon and the year it fell"

mkdir -p "$WORLDS_DIR"

fail=0
for preset in "${PRESETS[@]}"; do
  echo "==> baking $preset ($RESPONDER, batch=$BATCH, sleep=${SLEEP}s)"
  stage="$(mktemp -d "$WORLDS_DIR/.bake-$preset-XXXXXX")"
  if ! python3 "$EXPORTER" --preset "$preset" --out "$stage/world" \
       --responder "$RESPONDER" --batch "$BATCH" --sleep "$SLEEP" --verify; then
    echo "FAIL: $preset bake or receipt verification failed; keeping previous bake" >&2
    rm -rf "$stage"
    fail=1
    continue
  fi

  # meta.json is the one file the /v1/worlds list endpoint reads: enough
  # to render a card without pulling the scene or the receipts.
  title_var="TITLES_$preset"
  python3 - "$stage" "$preset" "${!title_var}" <<'PY'
import json, os, sys, datetime
stage, preset, title = sys.argv[1], sys.argv[2], sys.argv[3]
prov = json.load(open(os.path.join(stage, "world.provenance.json")))
scene = json.load(open(os.path.join(stage, "world.scene.json")))
meta = {
    "preset": preset,
    "title": title,
    "baked_at": datetime.datetime.now(datetime.timezone.utc)
        .strftime("%Y-%m-%dT%H:%M:%SZ"),
    "cells": len(scene["cells"]),
    "facts": scene["fact_count"],
    "splats": len(prov["splats"]),
    "bands": prov["bands"],
    "bbox": prov["bbox"],
    "responder_pubkey_b32": prov.get("responder_pubkey_b32"),
    "artifacts": prov["artifacts"],
}
with open(os.path.join(stage, "meta.json"), "w") as f:
    json.dump(meta, f, indent=1)
PY

  # Atomic swap: rename the finished stage over the live directory. A
  # reader mid-request sees either the old bake or the new one, never a
  # mixture; the provenance hashes always match the files next to them.
  old="$WORLDS_DIR/.old-$preset-$$"
  [ -d "$WORLDS_DIR/$preset" ] && mv "$WORLDS_DIR/$preset" "$old"
  mv "$stage" "$WORLDS_DIR/$preset"
  rm -rf "$old"
  echo "==> $preset done: $(du -sh "$WORLDS_DIR/$preset" | cut -f1)"
done

exit $fail
