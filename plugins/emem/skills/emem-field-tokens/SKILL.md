---
name: emem-field-tokens
description: Fetches a native-resolution raster field over an area from emem, or the same field over several dates, as a signed artifact that anyone can re-derive, not a set of per-cell values. Use when the user needs the grid of values over an area, not one number at one point. Examples are a world-model input, a band or NDVI layer, change over a scene window, or pixels a third party can re-hash. Returns content-addressed grid artifacts, a signed derivation record, and an emem:raster, emem:cube or emem:rasterset token. Reads need no key.
---

# emem-field-tokens

| | |
|---|---|
| Use it when | The user needs a grid of pixel values over an area, at one date or at several dates. |
| You need | A bounding box (`min_lat`, `min_lng`, `max_lat`, `max_lng`) and a band. Dates are optional. No key. |
| You get | A grid artifact with its BLAKE3 name (`artifact_cid`), a signed derivation record, a receipt, and an `emem:raster:` or `emem:cube:` token. |
| Calls | `POST /v1/band_raster`, `/v1/band_composite`, `/v1/band_cube`, then `/v1/raster/resolve` or `/v1/cube/resolve` to check a token. |

> **Network use.** The commands in this skill send JSON to `https://emem.dev` and receive JSON or a raster file from it. `https://emem.dev` is the service that the MCP server of the plugin connects to. The commands do not execute what they download. The files under `scripts/` read local files and make no network calls.

A world model reads a field over an area, often across time. This
skill gets that field from emem as a signed artifact. Another party can
fetch the same bytes again, or compute them again, and get the same
hash.

## Calls in this skill

| Call | What it returns |
|---|---|
| `POST /v1/band_raster` | One field over a bbox at one date, and an `emem:raster:` token. |
| `POST /v1/band_composite` | A cloud-free median of one band over a date window. Same shape as `band_raster`. |
| `POST /v1/band_cube` | The same field at several dates: a signed list of raster slices, and an `emem:cube:` token. |
| `POST /v1/raster_bundle` | 2 to 64 raster tokens as one set, and an `emem:rasterset:` token. |
| `POST /v1/raster/resolve`, `/v1/cube/resolve`, `/v1/raster_bundle/resolve` | The signed record behind a token. With `"spot_check": true`, the raster route also reads sample pixels back. |
| `POST /v1/cells_in_bbox` | The cell64 addresses in a bbox, in pages. Use it when you want addresses, not a grid. |

Each route has an MCP tool that takes the same JSON body:
`emem_band_raster`, `emem_band_composite`, `emem_band_cube`,
`emem_raster_bundle`, `emem_raster_resolve`, `emem_cube_resolve`,
`emem_raster_bundle_resolve` and `emem_cells_in_bbox`. These tools are
not in the short tool list of the plugin. The MCP server runs them by
name through `tools/call`. If your host calls only listed tools, or has
no MCP, send the body to the REST route. The steps below use REST.

The receipt does not sign a pixel value. It signs a derivation: this
responder made the artifact named `artifact_cid` from a pinned
Sentinel-2 scene, over an area named `aoi_cid`, at one time slot.

## When to use a different skill

| Question | Use |
|---|---|
| One value at one place | [`emem-locate-and-recall`](../emem-locate-and-recall/SKILL.md) |
| Signed per-cell facts inside an area | [`emem-recall-polygon`](../emem-recall-polygon/SKILL.md) |
| A picture to look at | `GET /v1/scene.png?bbox=w,s,e,n`, see [`emem-field-signals`](../emem-field-signals/SKILL.md). It is a view, not a signed artifact. |

## Step 1: fetch one field

`/v1/band_raster` accepts these bands:

| Band | Source | Window cap |
|---|---|---|
| `s2.B02`, `s2.B03`, `s2.B04`, `s2.B08`, `s2.B11`, `s2.B12` | Sentinel-2 L2A reflectance | 512 px a side, about 5.1 km at 10 m |
| `copdem30m.elevation` | Copernicus GLO-30 DEM. Open ocean has no tile. | 512 px a side |
| `forest_change.lossyear`, `forest_change.treecover2000`, `forest_change.gain`, `jrc_gfc2020.forest_2020`, `jrc_tmf.deforestation_year` | Static forest layers | 1024 px a side |

The band list and the caps come from `crates/emem-api-rest/src/band_raster.rs`.
A larger window gets a refusal that gives the size and the cap. Split
the area into smaller boxes. To ask for a capture date, add
`"observed_on": "YYYY-MM-DD"`. The responder pins the scene that it
used in both cases.

```sh
curl -s -X POST https://emem.dev/v1/band_raster -H 'content-type: application/json' \
  -d '{"bbox":{"min_lat":32.5699,"min_lng":77.0328,"max_lat":32.5727,"max_lng":77.0362},"band":"s2.B04"}' \
  -o raster.json
jq '{raster: .tokens.raster, artifact: .artifact.url, cid: .artifact.artifact_cid,
     grid: .grid, scene: .derivation.sources[0].id}' raster.json
```

On 7 November 2026 this call returned in 0.6 s:

| Field | Value |
|---|---|
| `grid` | 32 x 32 px, 10 m, EPSG:32643 |
| `scene` | `S2A_MSIL2A_20261104T054241_R005_T43SFS_20261104T085202` |
| `artifact_cid` | `77zgrf4hiv4cnzqblgkttrg4jssvh5sghhvdqjjq3wnj77nmgpma` |
| `tokens.raster` | `emem:raster:tllufj6j...:s2.B04:20761:2w5daw7i...` |

Without a date, the responder picks a recent usable scene. The scene,
the cid and the token change when a newer scene arrives.

## Step 2: download the bytes and hash them again

Download the file at `artifact.url`. No MCP tool returns the bytes.

```sh
curl -s -o artifact.bin https://emem.dev/v1/artifacts/77zgrf4hiv4cnzqblgkttrg4jssvh5sghhvdqjjq3wnj77nmgpma
python3 "${CLAUDE_SKILL_DIR}/scripts/rehash.py" 77zgrf4hiv4cnzqblgkttrg4jssvh5sghhvdqjjq3wnj77nmgpma artifact.bin
```

Use your own `artifact_cid` from step 1 in both lines.
`${CLAUDE_SKILL_DIR}` is the directory of this skill. Claude Code sets
it.

| Output of `rehash.py` | Exit code | Meaning |
|---|---|---|
| The computed cid, then `MATCH` | 0 | The bytes are the bytes that the record names. |
| The computed cid, then `MISMATCH` | 1 | The bytes changed, or you used a different cid. |
| `emem_crypto.py not found` | 2 | The script is not beside `emem_crypto.py`. |

On 7 November 2026 the file was 4160 bytes and `rehash.py` printed
`MATCH`. With a different cid, it printed `MISMATCH`. `rehash.py`
gets BLAKE3 from `scripts/emem_crypto.py`, which ships with this skill.
It is plain Python with no third-party packages.
`rehash.py --self-test` checks it against the official test vectors.

The file format is `application/x.emem-grid-f32.v1`. It has a 64-byte
header, then a little-endian f32 array. The header starts with the
magic `EMEMGRD1`, then `width`, `height`, `epsg`, `nodata`, the origin
and the steps. NaN is nodata. A 32 x 32 grid is 64 + 32 x 32 x 4 = 4160
bytes.

## Step 3 (optional): get a cloud-free composite

`/v1/band_composite` takes the median of one Sentinel-2 band over a
date window. It removes cloud pixels by their scene class first.

```sh
curl -s -X POST https://emem.dev/v1/band_composite -H 'content-type: application/json' \
  -d '{"bbox":{"min_lat":32.5699,"min_lng":77.0328,"max_lat":32.5727,"max_lng":77.0362},"band":"s2.B04","start_date":"2026-06-01","end_date":"2026-06-30","max_scenes":4}' \
  -o composite.json
jq '{raster: .tokens.raster, members: .member_count, window, grid}' composite.json
```

`max_scenes` is 2 to 16, and the default is 12. `min_valid_count`
(default 1) sets how many clear samples a pixel needs. On
7 November 2026 this call used 4 scenes and returned in 1.4 s. The
result has the same shape as a `band_raster` result. Check it with
steps 2 and 5.

## Step 4 (optional): fetch the field at several dates

```sh
curl -s -X POST https://emem.dev/v1/band_cube -H 'content-type: application/json' \
  -d '{"bbox":{"min_lat":32.5699,"min_lng":77.0328,"max_lat":32.5727,"max_lng":77.0362},"band":"s2.B08","observed_on":["2026-05-15","2026-06-15","2026-07-15"]}' \
  -o cube.json
jq '{cube: .tokens.cube, count: .member_count,
     members: [.derivation.members[] | {tslot, scene: .scene_id, requested_dates,
               distance: .requested_date_distance_days, raster: .raster_token}]}' cube.json
```

A cube contains no new pixels. It is a signed list of `member_count`
raster slices, one for each scene. You can resolve each slice alone.
If two dates map to the same scene, they become one member. Each
member gives its `requested_dates` and the distance in days to its
scene. `cube_cid` is the BLAKE3 of the ordered member derivation cids.

On 7 November 2026 this call returned three members:

| Requested date | Scene date | `requested_date_distance_days` |
|---|---|---|
| 2026-05-15 | 2026-06-10 | 26 |
| 2026-06-15 | 2026-06-30 | 15 |
| 2026-07-15 | 2026-08-03 | 19 |

A cube member is not a reading on the requested day. When you report
a member, give the requested date and the distance.

## Step 5: check a token that you received

For a raster token, send it with `spot_check`:

```sh
curl -s -X POST https://emem.dev/v1/raster/resolve -H 'content-type: application/json' \
  -d '{"token":"emem:raster:tllufj6jxg3cpuucznj4h75ng3lelg2ki3yb5whw6ggdkzt2gocq:s2.B04:20761:2w5daw7ir2r53u7to6rrz7pcuwa2aj6g2n2ujmkkmw4iwihkj5bq","spot_check":true}' \
  | jq -c '{resolved, url: .artifact.url, field: .receipt.field, passed: .spot_check.passed, anchors: .spot_check.anchors}'
```

On 7 November 2026 this returned `resolved: true`, `passed: true` and
`anchors: 5`.

| Field | Meaning |
|---|---|
| `resolved` | The token matches the signed record. |
| `artifact.url` | Where to download the bytes for step 2. |
| `receipt.field` | The `aoi_cid` and `derivation_cid` that the receipt signature covers. |
| `spot_check.passed` | The sampled pixels in the artifact agree with the values in the signed record. |
| `spot_check.anchors` | The number of sampled pixels. |
| `spot_check.fact_matches_grid` | Sampled pixels that also agree with a signed per-cell fact. It is 0 when the store holds no such fact. |

For a cube token, send `{"token": "emem:cube:..."}` to
`/v1/cube/resolve`. Read `resolved` and `member_tokens`. On
7 November 2026 the cube from step 4 resolved with 3 member tokens.

The responder checks each part of the token against the signed record
before it reads the artifact. If the area, band or date does not
match, you get HTTP 409. For example, a raster token with `s2.B08` in
place of `s2.B04` returned:

```json
{"code":"invalid_argument","message":"token band `s2.B08` contradicts the signed record's `s2.B04`; refusing to dereference a forged or corrupt handle","schema":"emem.error.v1"}
```

To check the receipt signature offline, use
[`emem-verify-receipt`](../emem-verify-receipt/SKILL.md) on
`raster.json`. On 7 November 2026 it printed `VALID`, primitive
`emem.band_raster`, with the FIELD segment bound.

## Step 6 (optional): put several rasters in one set

Send 2 to 64 raster tokens to `POST /v1/raster_bundle`:

```json
{"tokens": ["emem:raster:<aoi>:s2.B04:<tslot>:<cid>", "emem:raster:<aoi>:s2.B08:<tslot>:<cid>"],
 "purpose": "NDVI inputs"}
```

The result has an `emem:rasterset:` token. Check it with
`/v1/raster_bundle/resolve`. This call stores a new bundle record on
the responder, so this page does not show a live run.

## List the cells in an area

```sh
curl -s -X POST https://emem.dev/v1/cells_in_bbox -H 'content-type: application/json' \
  -d '{"bbox":{"min_lat":32.5699,"min_lng":77.0328,"max_lat":32.5727,"max_lng":77.0362},"page_size":1024}' \
  | jq -c '{total, count, next_cursor, first: .cells[0]}'
```

On 7 November 2026 this box had `total: 1394` cells. The first page
had 1024 cells and `next_cursor: 1024`. Send `"cursor": <next_cursor>`
to get the next page. Stop when `next_cursor` is null. You can send
each page to `POST /v1/recall_many` with a `budget_ms`. This call reads
no facts and returns no receipt.

## Limits and failures

| Case | What happens | What to do |
|---|---|---|
| Window larger than the cap | HTTP 400 with the size in pixels and the cap | Use a smaller bbox, or split it. |
| DEM over open ocean | Refusal: the archive has no tile there | No elevation field exists there. |
| Cold area under load | `compute_timeout` after the 40 s transport budget | Retry once. |
| Artifact evicted from the store | The derivation record stays. It pins scene, recipe and area. | Send the same body again to rebuild the same bytes. |
| Token parts do not match the record | HTTP 409 `invalid_argument` | Do not use the token. |

To get NDVI from rasters, fetch `s2.B08` and `s2.B04` for the same
scene. Compute (B08 - B04) / (B08 + B04) for each pixel. For one cell,
the cloud-gated `indices.ndvi` fact from recall is already computed.

## What the token proves

An `emem:raster:` token has the form
`emem:raster:<aoi_cid>:<band>:<tslot>:<derivation_cid>`. The
`derivation_cid` names the signed derivation record. That record pins
the scene, the recipe and the BLAKE3 of the artifact. So the token
binds the derivation, and through it, the bytes. `emem:cube:` does the
same over a tslot range and its member slices. `emem:rasterset:` names
a bundle cid and its derivation.

None of these tokens signs a pixel as a measurement. When one reading
matters, also cite the `emem:fact:` token of that cell.
[`emem-multi-agent-handoff`](../emem-multi-agent-handoff/SKILL.md) has
the full token table.
