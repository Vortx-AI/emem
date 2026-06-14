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
# the FULL band union the consumer topics fan out to (real_estate, flood,
# urban_livability, public_health, built_up, weather, vegetation, topography,
# fire, air_quality, reflectance) — so /v1/ask completes inside its budget.
BANDS='["cams.aod_550","cams.co","cams.no2","cams.o3","cams.pm10","cams.pm25","cams.so2","copdem30m.elevation_mean","indices.evi","indices.nbr","indices.ndbi","indices.ndmi","indices.ndvi","indices.savi","indices.urban_canopy_index","modis.lst_day_8day","modis.lst_night_8day","modis.ndvi_mean","overture.buildings.count","overture.places.count","overture.transportation.road_length_m","s2.B02","s2.B03","s2.B04","s2.B08","s2.B11","s2.B12","sentinel1_raw","surface_water.recurrence","weather.cloud_cover","weather.precipitation_mm","weather.relative_humidity_2m","weather.temperature_2m","weather.wind_speed_10m"]'

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
