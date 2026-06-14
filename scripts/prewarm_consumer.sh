#!/usr/bin/env bash
# Prewarm the consumer-facing bands at the homepage showcase patches, so the
# day-to-day ask examples (buy a home / air & health / livable / population)
# return instantly instead of cold-fetching Overture + CAMS on first click.
# Runs against the live responder; recall auto-materialises + signs + caches.
set -uo pipefail
BASE="${1:-https://emem.dev}"

# showcase patches (must mirror the atlas PATCHES array in web/index.html)
PATCHES=(
  "-17.70 -56.60 Pantanal"
  "42.00 -93.60 Iowa"
  "0.60 21.00 Congo"
  "-3.20 -54.10 Amazon"
  "0.50 113.90 Borneo"
  "-0.50 101.50 Sumatra"
)
# the bands the consumer topics route to (air/health, property/flood, livability, population)
BANDS='["cams.pm25","cams.no2","cams.aod_550","modis.lst_day_8day","copdem30m.elevation_mean","surface_water.recurrence","indices.mndwi","indices.ndvi","indices.urban_canopy_index","overture.transportation.road_length_m","overture.buildings.count","overture.places.count"]'

for row in "${PATCHES[@]}"; do
  read -r lat lng name <<< "$row"
  t0=$(date +%s)
  code=$(curl -s -o /tmp/pw_$name.json -w "%{http_code}" --max-time 180 -X POST "$BASE/v1/recall" \
    -H 'content-type: application/json' \
    -d "{\"lat\":$lat,\"lng\":$lng,\"bands\":$BANDS}")
  n=$(python3 -c "import json;print(len(json.load(open('/tmp/pw_$name.json')).get('facts',[])))" 2>/dev/null || echo "?")
  echo "  $name ($lat,$lng): http=$code facts=$n in $(( $(date +%s)-t0 ))s"
done
echo "prewarm-consumer done"
