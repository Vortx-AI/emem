#!/usr/bin/env bash
# Prewarm the homepage showcase patches end-to-end so the panel (four-model
# consensus + engram + fingerprint + similar + scene) and the day-to-day ask
# examples (buy a home / air & health / livable / population) return instantly.
# Mirrors the atlas PATCHES array in web/index.html (cities first, nature after).
# Runs against the live responder; every read auto-materialises + signs + caches.
set -uo pipefail
BASE="${1:-https://emem.dev}"

PATCHES=(
  "19.07 72.87 Mumbai"
  "-23.55 -46.63 SaoPaulo"
  "41.88 -87.63 Chicago"
  "-6.20 106.85 Jakarta"
  "-17.70 -56.60 Pantanal"
  "-3.20 -54.10 Amazon"
  "0.50 113.90 Borneo"
  "42.00 -93.60 Iowa"
)
BANDS='["cams.pm25","cams.no2","cams.aod_550","modis.lst_day_8day","copdem30m.elevation_mean","surface_water.recurrence","indices.mndwi","indices.ndvi","indices.urban_canopy_index","overture.transportation.road_length_m","overture.buildings.count","overture.places.count"]'

post(){ curl -s -o /dev/null -w "%{http_code}" --max-time 200 -X POST "$BASE/$1" -H 'content-type: application/json' -d "$2"; }

for row in "${PATCHES[@]}"; do
  read -r lat lng name <<< "$row"
  t0=$(date +%s)
  cell=$(curl -s --max-time 30 -X POST "$BASE/v1/locate" -H 'content-type: application/json' -d "{\"lat\":$lat,\"lng\":$lng}" | python3 -c "import sys,json;print(json.load(sys.stdin).get('cell64',''))" 2>/dev/null)
  [ -z "$cell" ] && { echo "  $name: locate failed"; continue; }
  sm=$(post "v1/state_multi" "{\"cell\":\"$cell\",\"vectors\":true}")
  tc=$(post "v1/triple_consensus" "{\"cell\":\"$cell\"}")
  rc=$(post "v1/recall" "{\"lat\":$lat,\"lng\":$lng,\"bands\":$BANDS}")
  fs=$(post "v1/find_similar" "{\"key\":\"$cell\",\"k\":4}")
  curl -s -o /dev/null --max-time 60 "$BASE/v1/cells/$cell/scene.png" &
  echo "  $name $cell: state_multi=$sm consensus=$tc recall=$rc similar=$fs in $(( $(date +%s)-t0 ))s"
done
wait
echo "prewarm-consumer done"
