---
name: emem-urban
description: Answers urban-analysis questions with signed emem facts and says what each number measures. Covers Overture buildings, places, road length and road bearing, WorldPop population density, the Sentinel-2 built-up index (NDBI), ESA WorldCover built-up class, JRC surface-water recurrence and DMSP night lights, with each band's source, release, resolution and failure modes. Use when the user asks how built-up, dense, connected or lit a neighbourhood is, compares districts, or needs street and building evidence another agent can verify, and when an area total must be estimated from sampled cells honestly.
---

# emem-urban

emem answers per cell. A cell is about 10 m across, much smaller than
most urban sources, so the first question for every urban number is
what it was measured over. This skill gives the answer per band and
shows how to go from cells to an area without overstating coverage.

## The bands

| Band | Unit | Measured over | Source | Watch for |
|---|---|---|---|---|
| `overture.buildings.count` | count | the cell's own bbox: buildings whose **centroid** falls inside | Overture buildings, release named in `derivation.args` | 0 can mean unmapped; coverage is uneven outside NA and EU |
| `overture.places.count` | count | points inside the cell bbox | Overture places | POIs, not people or floor area |
| `overture.transportation.road_length_m` | m | road segments **clipped** to the cell bbox | Overture transportation | a 10 m cell off the street reads 0 |
| `overture.transportation.road_bearing_deg` | deg_axial, 0 to 180 from north | the nearest segment within 50 m; distance in `derivation.args` | Overture transportation | an Absence when no segment is within 50 m |
| `population` | people_per_km2 | one 1 km pixel, sampled at the cell | WorldPop UNadj 1 km, 2020 | the pixel's density, not people in the cell |
| `indices.ndbi` | ratio, -1 to 1 | one 10 m Sentinel-2 pixel, (B11 - B08) / (B11 + B08) | Sentinel-2 L2A, scene id in args | read `surface_class`: a cloud-shadow pixel is not built-up evidence |
| `esa_worldcover.lc_2021` | class | one 10 m pixel | ESA WorldCover v200, 2021 | 50 is built-up |
| `surface_water.recurrence` | percent | one 30 m pixel | JRC Global Surface Water 1984 to 2021 | 60 N to 60 S only |
| `nightlights.dmsp_ols_avg_dn` | DN 0 to 63 | DMSP-OLS stable lights | DMSP-OLS V4, 1992 to 2013 | saturates at 63 in any city core; no VIIRS |

GHSL built-surface bands are not served (`unknown_band`, which is not
"no data here"). `GET /v1/bands` is the full list; check a band there
before building an analysis on it.

## One place

```sh
curl -sf -X POST https://emem.dev/v1/recall -H 'content-type: application/json' \
  -d '{"cell":"defi.zb595.zbaf8.zd63c","bands":["overture.buildings.count",
       "overture.places.count","overture.transportation.road_length_m",
       "overture.transportation.road_bearing_deg","population","indices.ndbi",
       "esa_worldcover.lc_2021","surface_water.recurrence","nightlights.dmsp_ols_avg_dn"]}' \
  -o urban.json
jq -c '.facts[] | {band, value, unit, observed_at, args: .derivation.args, token: .memory_token}' urban.json
```

Get the cell with `POST /v1/locate` first
([`emem-locate-and-recall`](../emem-locate-and-recall/SKILL.md)).

Recorded on 2026-09-28 at a Shibuya cell (35.65988 N, 139.70737 E):
WorldCover 50 (built-up), NDBI 0.024 on a pixel flagged cloud shadow,
DMSP 63 (saturated, 2013), population 16403 people/km2 (the 1 km pixel),
2 places, 0 buildings and 0 m of road inside the cell, and a road
bearing of 55.63 deg_axial from a segment 11.1 m away. Overture facts
named release `2026-09-23.1`. Each value came back with an
`emem:fact:` token.

That one cell reads as "no buildings, no road" in the middle of Shibuya.
Nothing is wrong: the cell is 10 m, it sits between features, and the
counts are exactly what fell inside it. Area questions need area calls.

## An area

```sh
curl -sf -X POST https://emem.dev/v1/recall_polygon -H 'content-type: application/json' \
  -d '{"polygon_bbox":{"min_lat":35.6585,"max_lat":35.6605,"min_lng":139.6995,"max_lng":139.7015},
       "bands":["overture.transportation.road_length_m","overture.buildings.count"],
       "budget_ms":60000}' -o area.json
jq '{area_km2, cells_sampled, native_grid_cells_in_polygon, coverage_fraction, is_exhaustive}' area.json
jq '[.by_cell | .. | objects | select(.band? == "overture.transportation.road_length_m") | .value]
    | {n: length, nonzero: (map(select(. > 0)) | length), sum: add}' area.json
```

Recorded on 2026-09-28: 0.040 km2, 64 of about 400 cells sampled
(`coverage_fraction` 0.16, `is_exhaustive: false`), road length nonzero
in 45 cells, summing to 598.6 m; 3 building centroids in the sample.

The 598.6 m is a sample, not the road length of the box. A mean-per-cell
estimate is 598.6 / 64 x 400, about 3.7 km, or roughly 93 km of road per
km2; say it is an estimate from a 16 percent sample and cite the facts
it came from. Do not scale building counts the same way without saying
so: a centroid count in 10 m cells is sparse, and 3 in 64 cells has a
wide interval. For street grids and building totals over a whole
district, sample more cells (`cells_per_sqkm`) or page the full grid
with `POST /v1/cells_in_bbox` and `POST /v1/recall_many`.

## Reading the evidence

- **Release pinning.** Overture facts name their release and a hash of
  its file listing in `sources`, so a figure stays checkable after
  Overture drops that release.
- **Point samples of coarse pixels.** `population`, `nightlights` and
  WorldCover are the value of the source pixel at the cell. Two
  neighbouring cells can be the same pixel; never difference them as a
  gradient. The response's `spatial_basis` says so too.
- **Years differ.** Population is 2020, WorldCover 2021, DMSP 2013,
  Overture this month. State each date rather than one "as of".
- **Built-up signals disagree for real reasons.** NDBI confuses bare
  soil with roofs; WorldCover is a 2021 classifier output; Overture is
  mapped footprints. Report which one you used.

To cite a figure in a report, see
[`emem-research-grade-citation`](../emem-research-grade-citation/SKILL.md).
