#!/usr/bin/env bash
# Pre-warm the homepage atlas showcase patches so their four-encoder
# state_multi + triple_consensus verdict is instant for the first visitor.
# Each patch is materialized + signed + persisted on the first hit; a second
# pass confirms the warm read. Run against the local responder after a deploy
# (the box can't hairpin to emem.dev). Idempotent.
set -uo pipefail
BASE="${1:-http://127.0.0.1:5051}"

# lng lat name — must match PATCHES[] in web/index.html
PATCHES=(
  "-54.10 -3.20 Amazon-Para"
  "113.90 0.50 Borneo-Kalimantan"
  "21.00 0.60 Congo-Basin"
  "-93.60 42.00 Iowa-cropland"
  "-56.60 -17.70 Pantanal"
  "101.50 -0.50 Sumatra-peat"
)

jq_cell() { python3 -c 'import sys,json;print(json.load(sys.stdin).get("cell64",""))'; }
sm_sum() { python3 -c 'import sys,json;d=json.load(sys.stdin);print("encoders="+str(len(d.get("encoders",[])))+" missing="+str(len(d.get("missing",[]))))' 2>/dev/null || echo "encoders=? missing=?"; }
tc_sum() { python3 -c 'import sys,json;d=json.load(sys.stdin);print("verdict="+str(d.get("verdict"))+" agreement="+str(d.get("agreement")))' 2>/dev/null || echo "verdict=?"; }

for row in "${PATCHES[@]}"; do
  read -r lng lat name <<<"$row"
  cell=$(curl -s -m 60 -X POST "$BASE/v1/locate" -H 'content-type: application/json' \
         -d "{\"lat\":$lat,\"lng\":$lng}" | jq_cell)
  echo "== $name  ($lat,$lng) -> ${cell:-UNRESOLVED}"
  [ -z "$cell" ] && continue
  for pass in 1 2; do
    t0=$(date +%s)
    sm=$(curl -s -m 150 -X POST "$BASE/v1/state_multi"      -H 'content-type: application/json' -d "{\"cell\":\"$cell\"}" | sm_sum)
    tc=$(curl -s -m 150 -X POST "$BASE/v1/triple_consensus" -H 'content-type: application/json' -d "{\"cell\":\"$cell\"}" | tc_sum)
    t1=$(date +%s)
    echo "   pass$pass  $sm | $tc | $((t1-t0))s"
  done
done
echo "== prewarm done"
