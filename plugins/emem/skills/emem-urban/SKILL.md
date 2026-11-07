---
name: emem-urban
description: Answers urban-analysis questions with signed emem facts and says what each number measures. It covers Overture buildings, places, road length and road bearing, and WorldPop population density. It also covers the Sentinel-2 built-up index (NDBI), the ESA WorldCover built-up class, JRC surface-water recurrence and DMSP night lights. For each band it gives the source, release, resolution and failure modes. Use when the user asks how built-up, dense, connected or lit a neighbourhood is, or compares districts. Also use it when the user needs street and building evidence that another agent can verify, or an honest area total from sampled cells.
---

# emem-urban

| | |
|---|---|
| Use it when | The user asks how built-up, dense, connected or lit a place is, compares districts, or needs an area total for roads or buildings. |
| You need | A cell64 for one place, or a bounding box for an area. Get a cell64 with [`emem-locate-and-recall`](../emem-locate-and-recall/SKILL.md). No key. |
| You get | One signed fact for each band, each with an `emem:fact:` token and the source release. An area call also gives the sample size and the coverage fraction. |
| Calls | `POST /v1/recall` (MCP `emem_recall`), `POST /v1/recall_polygon` (MCP `emem_recall_polygon`). |

> **Network use.** The commands in this skill send JSON to `https://emem.dev` and receive JSON from it. `https://emem.dev` is the service that the MCP server of the plugin connects to. The commands do not execute what they download. This skill ships no scripts. Its commands read no environment variable, key or credential. The only local files that they read are files that an earlier step saved from `https://emem.dev`.

emem answers for each cell. A cell is about 10 m across. Most urban
sources are much coarser than that. Thus, for each urban number, first
find the area that the source measured. The band table gives that area.
Step 2 shows how to go from cells to an area and give the true
coverage.

## The bands

| Band | Unit | Measured over | Source | Watch for |
|---|---|---|---|---|
| `overture.buildings.count` | count | The bbox of the cell. A building counts when its **centroid** is inside. | Overture buildings. The release is in `derivation.args`. | 0 can mean "not mapped". Coverage is not equal everywhere, and is best in North America and Europe. |
| `overture.places.count` | count | Points inside the bbox of the cell. | Overture places | Points of interest. Not people and not floor area. |
| `overture.transportation.road_length_m` | m | Carriageway segments (class `motorway` through `service`), **clipped** to the bbox of the cell. Footways, paths, rail and water do not count. | Overture transportation | A 10 m cell next to a street reads 0. |
| `overture.transportation.road_bearing_deg` | deg_axial, 0 to 180 from north | The nearest carriageway segment within 50 m. The distance, the `class` and the row groups read are in `derivation.args`. | Overture transportation | An Absence when no carriageway is within 50 m. A footway or a plaza edge does not count. |
| `population` | people_per_km2 | One 1 km pixel, sampled at the cell. | WorldPop UNadj 1 km, 2020 | The density of the pixel, not the people in the cell. |
| `indices.ndbi` | ratio, -1 to 1 | One 10 m Sentinel-2 pixel, (B11 - B08) / (B11 + B08). | Sentinel-2 L2A. The scene id is in `derivation.args`. | Read `surface_class`. A pixel flagged as cloud shadow or water is not built-up evidence. |
| `esa_worldcover.lc_2021` | class | One 10 m pixel. | ESA WorldCover v200, 2021 | Class 50 means built-up. |
| `surface_water.recurrence` | percent | One 30 m pixel. | JRC Global Surface Water, 1984 to 2021 | Covers 60 N to 60 S only. |
| `nightlights.dmsp_ols_avg_dn` | DN, 0 to 63 | DMSP-OLS stable lights. | DMSP-OLS V4, 1992 to 2013 | Retired in this code, because the upstream moved behind a login. Stored facts still recall and verify. A cell with no stored fact gets a `materialize_notes` entry with `band_retired_at_this_responder`. Saturates at 63 in a city core. There is no VIIRS band. |

emem.dev does not serve the GHSL built-surface bands. A request for
`ghsl.built_surface_m2` returns HTTP 400 with `band_not_in_registry`.
That error means "this responder has no such band". It does not mean
"no data here". `GET https://emem.dev/v1/bands` gives the full catalog.
Find a band there before you build an analysis on it.

## Step 1: read one place

Send this body to `POST https://emem.dev/v1/recall`, or call
`emem_recall` with it:

```sh
curl -s -X POST https://emem.dev/v1/recall -H 'content-type: application/json' \
  -d '{"cell":"defi.zb595.zbaf8.zd63c","bands":["overture.buildings.count","overture.places.count","overture.transportation.road_length_m","overture.transportation.road_bearing_deg","population","indices.ndbi","esa_worldcover.lc_2021","surface_water.recurrence","nightlights.dmsp_ols_avg_dn"]}' \
  -o urban.json
jq -c '.facts[] | {band, value, unit, observed_at, surface_class, args: .derivation.args, token: .memory_token}' urban.json
```

The cell is in Shibuya, Tokyo (35.65988 N, 139.70737 E). This call
returned these values on 7 November 2026 at 01:04 UTC:

| Band | Value | Note |
|---|---|---|
| `esa_worldcover.lc_2021` | 50 | The class for built-up. |
| `indices.ndbi` | 0.024 (24 August 2026), -0.109 (28 October 2026) | Two scenes. `surface_class` flags the first as cloud shadow and the second as water. Neither is built-up evidence. |
| `nightlights.dmsp_ols_avg_dn` | 63 | A stored fact. The code retires the band, so it makes no new fact. Saturated. Data year 2013, satellite F18. |
| `population` | 16403 people_per_km2 | The 1 km pixel, data year 2020. |
| `overture.places.count` | 2 | Release `2026-09-23.1`. |
| `overture.buildings.count` | 0 | Release `2026-09-23.1`. |
| `overture.transportation.road_length_m` | 0 | Release `2026-10-21.0`. |
| `overture.transportation.road_bearing_deg` | 55.63 | Class `unclassified`, 11.1 m from the cell. Release `2026-10-21.0`. |
| `surface_water.recurrence` | 0 | |

Each fact came back with an `emem:fact:` token in `memory_token`. The
call fetched three bands on a miss, so the result also had a
`materialize_notes` list. A repeat call reads the stored facts and
can show no `materialize_notes`.

The cell reads "no buildings, no road" in the middle of Shibuya. This
is correct. The cell is 10 m across and sits between features. The
counts give exactly what is inside the cell. For an area question,
use an area call.

## Step 2: read an area

Send this body to `POST https://emem.dev/v1/recall_polygon`, or call
`emem_recall_polygon` with it:

```sh
curl -s -X POST https://emem.dev/v1/recall_polygon -H 'content-type: application/json' \
  -d '{"polygon_bbox":{"min_lat":35.6585,"max_lat":35.6605,"min_lng":139.6995,"max_lng":139.7015},"bands":["overture.transportation.road_length_m","overture.buildings.count"],"budget_ms":60000}' \
  -o area.json
jq '{area_km2, cells_sampled, native_grid_cells_in_polygon, coverage_fraction, is_exhaustive}' area.json
jq -c '[.by_cell | .. | objects | select(.band? == "overture.transportation.road_length_m") | .value]
  | {n: length, nonzero: (map(select(. > 0)) | length), sum: add}' area.json
jq -c '[.by_cell | .. | objects | select(.band? == "overture.buildings.count") | .value]
  | {n: length, sum: add}' area.json
```

`emem_recall_polygon` is not in the short tool list that the MCP server
of the plugin shows. The server runs it by name through `tools/call`.
If your host calls only listed tools, use the REST route. An MCP result
has a wire budget of about 24 KB. A larger result comes back slimmed,
and `_emem_truncation` names what it left out. The REST route returns
the full response. On 7 November 2026 it was about 400 KB for this box.

This call returned these values on 7 November 2026 at 01:06 UTC:

| Field | Value |
|---|---|
| `area_km2` | 0.040 |
| `cells_sampled` | 64 |
| `native_grid_cells_in_polygon` | about 400 |
| `coverage_fraction` | 0.16 |
| `is_exhaustive` | `false` |
| Road length | Nonzero in 14 of 64 cells, sum 126.5 m |
| Building centroids | 3 in the 64 cells |

The 126.5 m is a sample. It is not the road length of the box. An
estimate from the mean for each cell is 126.5 / 64 x 400, about 790 m.
That is about 20 km of carriageway for each km2. When you give this
number, say that it is an estimate from a 16 percent sample. Cite the
facts that it came from.

Do not scale the building count in the same way without a warning. A
centroid count in 10 m cells is sparse. A count of 3 in 64 cells has a
wide interval. To get street grids and building totals over a full
district, sample more cells with `cells_per_sqkm`. Or list the full
grid with `POST /v1/cells_in_bbox` and read it with
`POST /v1/recall_many` (at most 256 cells for each call).

## Read the evidence correctly

- Overture facts name their release in `sources`, with a hash of the
  file listing of that release. Thus a figure stays checkable after
  Overture removes the release.
- `population`, `nightlights.dmsp_ols_avg_dn` and
  `esa_worldcover.lc_2021` give the value of the source pixel at the
  cell. Two adjacent cells can be the same pixel. Do not subtract
  them and call the result a gradient. The `spatial_basis` object in
  the response gives the same warning.
- The data years are different. Population is 2020, WorldCover is
  2021, DMSP is 2013, and Overture is a 2026 release. Give each date.
  Do not give one "as of" date for all of them.
- The built-up signals can disagree for real reasons. NDBI can read
  bare soil as roofs. WorldCover is the output of a 2021 classifier.
  Overture gives mapped footprints. Say which signal you used.

To cite a figure in a report, see
[`emem-research-grade-citation`](../emem-research-grade-citation/SKILL.md).
