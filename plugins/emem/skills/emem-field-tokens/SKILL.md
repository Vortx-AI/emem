---
name: emem-field-tokens
description: Fetches a native-resolution raster field over an area from emem, or the same field over several dates, as a signed and re-derivable artifact rather than a set of per-cell scalars. Use when the user needs the actual grid of values over an area of interest (a world-model input, a band or NDVI drape, change analysis over a scene window, pixels a third party can re-hash) rather than one number at one point. Returns content-addressed grid artifacts, a signed derivation record, and an emem:raster, emem:cube or emem:rasterset token. Reads need no key.
---

# emem-field-tokens

A world model reads a **field over an area across time**, not a set of
points. This skill fetches that field from emem as a signed artifact
anyone can re-derive.

| Call | Returns |
|---|---|
| `POST /v1/band_raster` | one native-resolution field over a bbox at one time, plus an `emem:raster:` token |
| `POST /v1/band_composite` | a cloud-free median composite over a date window (`s2_median_composite@1`), same shape |
| `POST /v1/band_cube` | the same field over several dates: a signed manifest over raster slices, as an `emem:cube:` token |
| `POST /v1/raster_bundle` | several rasters as one set, as an `emem:rasterset:` token |
| `POST /v1/raster/resolve`, `/v1/cube/resolve`, `/v1/raster_bundle/resolve` | walk a token back to its record; `{"spot_check": true}` also re-hashes the pixels for you |
| `POST /v1/cells_in_bbox` | the cell64 addresses in a bbox, paged, when you want addresses rather than a grid |

The receipt does not attest a value. It attests a **derivation**: this
responder computed the artifact whose blake3 is `artifact_cid`, from a
pinned Sentinel-2 scene, over a content-addressed area (`aoi_cid`) and
time. Re-fetch the bytes and re-hash them, or re-run the recipe from the
pinned scene, and you get the same artifact.

## When to invoke

- "Give me the 10 m red-band grid over this farm, not per-cell scalars."
- "Build a time series of near-infrared over this AOI across the summer."
- "I need pixels over this area that a stranger can re-derive."

For one value at one place use
[`emem-locate-and-recall`](../emem-locate-and-recall/SKILL.md). For
per-cell facts inside an area use
[`emem-recall-polygon`](../emem-recall-polygon/SKILL.md). For a picture
to look at, `GET /v1/scene.png?bbox=w,s,e,n` (see
[`emem-field-signals`](../emem-field-signals/SKILL.md)); that is a view,
not a signed artifact.

## One field (a raster)

Bands: `s2.B02`, `s2.B03`, `s2.B04`, `s2.B08`, `s2.B11`, `s2.B12`, or
`copdem30m.elevation` for terrain (one 1-degree DEM tile, no open
ocean). Window cap: 512 px a side, about 5.1 km at 10 m; page larger
areas. `observed_on` (YYYY-MM-DD) targets a capture date; the chosen
scene is pinned either way.

```sh
curl -sf -X POST https://emem.dev/v1/band_raster \
  -H 'content-type: application/json' \
  -d '{"bbox":{"min_lat":32.5699,"min_lng":77.0328,"max_lat":32.5727,"max_lng":77.0362},
       "band":"s2.B04"}' > raster.json
jq '{raster: .tokens.raster, artifact: .artifact.url, cid: .artifact.artifact_cid,
     grid: .grid, scene: .derivation.sources[0].id}' raster.json
```

Fetch the bytes and re-hash them:

```sh
CID=$(jq -r '.artifact.artifact_cid' raster.json)
curl -sf "https://emem.dev/v1/artifacts/$CID" \
  | python3 "${CLAUDE_SKILL_DIR}/rehash.py" "$CID"   # prints MATCH or MISMATCH
```

Recorded on 2026-09-28: a 32 x 32 px B04 grid (EPSG:32643, scene
`S2A_MSIL2A_20260925T054251_R005_T43SFS_20260925T090015`), token
`emem:raster:tllufj6j...:s2.B04:20721:ucosdg5p...`, artifact
`sipykc4ewzt72gwa7ep2ginzxuxwp5eh6ei5e4szxfspopi2f2ca`. `rehash.py`
printed `MATCH`, the receipt verified with its FIELD segment bound, and
`/v1/raster/resolve` with `spot_check` re-read five anchors and passed.

`${CLAUDE_SKILL_DIR}` is this skill's directory, filled in by Claude
Code; `rehash.py` ships beside this file and needs `pip install blake3`.
Outside Claude Code, take it from the repository
(`plugins/emem/skills/emem-field-tokens/rehash.py`) and read it before
running it.

The grid bytes are a little-endian f32 array behind a 64-byte header
(`application/x.emem-grid-f32.v1`): magic `EMEMGRD1`, then `width`,
`height`, `epsg`, `nodata`, and the origin and steps. NaN is nodata.

## A field over time (a cube)

```sh
curl -sf -X POST https://emem.dev/v1/band_cube \
  -H 'content-type: application/json' \
  -d '{"bbox":{"min_lat":32.5699,"min_lng":77.0328,"max_lat":32.5727,"max_lng":77.0362},
       "band":"s2.B08",
       "observed_on":["2026-05-15","2026-06-15","2026-07-15"]}' > cube.json
jq '{cube: .tokens.cube, count: .member_count,
         members: [.derivation.members[] | {tslot, scene: .scene_id,
                    requested_dates, distance: .requested_date_distance_days,
                    raster: .raster_token}]}' cube.json
```

A cube is not new pixels. It is a signed manifest over `member_count`
independently resolvable `emem:raster:` slices, one per scene. Dates
that hit the same scene collapse into one member, and each member says
which `requested_dates` mapped to it and how far away the scene was.
`cube_cid` is the blake3 of the ordered member derivation cids.

Recorded on 2026-09-28: three members, and the nearest usable scene for
the `2026-05-15` request was 10 June, 26 days away
(`requested_date_distance_days: 26`). Report that distance with the
date the user asked for; a cube member is not a reading on the
requested day.

```sh
jq '{token: .tokens.cube}' cube.json \
  | curl -sf -X POST https://emem.dev/v1/cube/resolve \
      -H 'content-type: application/json' -d @- \
  | jq '.resolved, .member_tokens'
```

## Enumerate the cells in an area

```sh
curl -sf -X POST https://emem.dev/v1/cells_in_bbox \
  -H 'content-type: application/json' \
  -d '{"bbox":{"min_lat":32.5699,"min_lng":77.0328,"max_lat":32.5727,"max_lng":77.0362},
       "page_size":1024}' \
  | jq '{total, count, next_cursor, first: .cells[0]}'
```

Page with `next_cursor` until it is null, and feed each page to
`POST /v1/recall_many` with a `budget_ms`. Pure geometry: no facts read,
no receipt.

## Verify a token you were handed

```sh
jq '{token: .tokens.raster, spot_check: true}' raster.json \
  | curl -sf -X POST https://emem.dev/v1/raster/resolve \
      -H 'content-type: application/json' -d @- \
  | jq '{bound: .resolved, artifact: .artifact.url, field: .receipt.field,
         spot_check_passed: .spot_check.passed}'
```

Every claim in the token binds to the signed record before anything
dereferences: a mismatched area, band or date is a typed 409, never a
silent wrong answer. Then check the receipt with
[`emem-verify-receipt`](../emem-verify-receipt/SKILL.md); its FIELD
segment binds `(aoi_cid, derivation_cid)`.

## Notes

- The artifact is evictable by design. The derivation record persists
  and pins scene, recipe and geometry, so an evicted artifact is a
  rebuild recipe (re-POST the same body), never a broken citation.
- NDVI from rasters: fetch B08 and B04 and compute
  (B08 - B04) / (B08 + B04) per pixel. For one cell, the cloud-gated
  `indices.ndvi` fact from recall is already computed.
- A cold area costs one scene read. Under load the first call can pass
  the 40 s transport budget and answer `compute_timeout`; retry once.

## What the token proves, and what it does not

An `emem:raster:` token spells out its claims:
`emem:raster:<aoi_cid>:<band>:<tslot>:<derivation_cid>`. The
`derivation_cid` names the signed derivation record, which pins the
scene, the recipe and the artifact's blake3, so the token binds the
derivation and, through it, the bytes. `emem:cube:` does the same over a
tslot range and its member slices; `emem:rasterset:` names a bundle cid
plus its derivation. None of them signs a pixel as a measurement. When
one reading matters, cite that cell's `emem:fact:` token too.
[`emem-agent-handoff`](../emem-agent-handoff/SKILL.md) has the whole
token table.
