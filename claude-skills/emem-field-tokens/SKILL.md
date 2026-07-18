---
name: emem-field-tokens
description: Get a native-resolution raster field over an area from emem, or a field over time, as a signed, verifiable artifact rather than a set of per-cell scalars. Use when the user needs the actual grid of values over an area of interest (a world model input, an NDVI/band drape, change analysis over a scene window, exportable pixels a third party can re-derive) rather than one number at one point. Returns content-addressed grid artifacts plus a signed derivation record and an emem:raster: or emem:cube: token. Reads are public — no auth required.
allowed-tools: Bash(curl:*) Bash(jq:*) Bash(python3:*) Read
---

# emem-field-tokens

A world model is a **field over an area across time**, not a set of points.
This skill fetches that field from emem as a signed, re-derivable artifact:

- `POST /v1/band_raster` — one native-resolution Sentinel-2 band over a bbox
  at one time, as a content-addressed grid plus an `emem:raster:` token.
- `POST /v1/band_cube` — the same band over several dates, a signed manifest
  over the raster slices, as an `emem:cube:` token (the 4D world token).
- `POST /v1/cells_in_bbox` — enumerate the cell64s in a bbox, paged, when you
  want the per-cell address list instead of the packed grid.

The receipt does not attest a value, it attests a **derivation**: this
responder computed the artifact whose blake3 is `artifact_cid`, from a pinned
Sentinel-2 scene, over a content-addressed area (`aoi_cid`) and time. Anyone
can re-fetch the bytes and re-hash them, or re-run the recipe from the pinned
scene, and get the same artifact.

## When to invoke

The user wants an area's field of values, not a point reading:

- "Give me the 10 m NDVI grid over this farm, not per-cell scalars."
- "Get the red band (B04) as a raster over this bbox."
- "Build a time series of the near-infrared band over this AOI across the summer."
- "I need the actual pixels over this area that a stranger can re-derive."

For a single value at one place use **emem-locate-and-recall**. For similar
places use **emem-find-similar**. For per-cell facts inside a polygon use
**emem-recall-polygon**.

## How to invoke

The endpoint is `https://emem.dev`. Reads are public. Bands: `s2.B02`,
`s2.B03`, `s2.B04`, `s2.B08`, `s2.B11`, `s2.B12`. Window cap: 512 px per side.

### One field (a raster)

```sh
curl -sf -X POST https://emem.dev/v1/band_raster \
  -H 'content-type: application/json' \
  -d '{"bbox":{"min_lat":32.5699,"min_lng":77.0328,"max_lat":32.5727,"max_lng":77.0362},
       "band":"s2.B04"}' \
  | jq '{raster: .tokens.raster, artifact: .artifact.url, grid: .grid,
         scene: .derivation.sources[0].id, docs: .docs}'
```

The response carries the `emem:raster:` token, the artifact URL
(`GET /v1/artifacts/{cid}`, immutable), the grid georeferencing, and the
pinned scene. Fetch the bytes and re-hash to verify:

```sh
CID=$(curl -sf ... | jq -r '.artifact.artifact_cid')     # from the response
curl -sf "https://emem.dev/v1/artifacts/$CID" \
  | python3 -c "import sys,base64;from blake3 import blake3;
d=sys.stdin.buffer.read();
print('re-hash matches:', blake3(d).digest().hex())"   # compare to artifact_cid (base32)
```

The grid bytes are a canonical little-endian f32 array with a 64-byte header
(`application/x.emem-grid-f32.v1`): magic `EMEMGRD1`, then `width`, `height`,
`epsg`, `nodata`, and the lat/lng origin + steps. A NaN is nodata.

### A field over time (a cube)

```sh
curl -sf -X POST https://emem.dev/v1/band_cube \
  -H 'content-type: application/json' \
  -d '{"bbox":{"min_lat":32.5699,"min_lng":77.0328,"max_lat":32.5727,"max_lng":77.0362},
       "band":"s2.B08",
       "observed_on":["2026-05-15","2026-06-15","2026-07-15"]}' \
  | jq '{cube: .tokens.cube, count: .member_count,
         members: [.derivation.members[] | {tslot, scene: .scene_id,
                    requested_dates, distance: .requested_date_distance_days,
                    raster: .raster_token}]}'
```

A cube is **not** new pixels: it is a signed manifest over `member_count`
independently resolvable `emem:raster:` slices, one per date (dates that hit
the same scene collapse; each member echoes which `requested_dates` mapped to
it). `cube_cid` is the blake3 of the ordered member derivation cids. Resolve a
member independently, or the whole cube:

```sh
curl -sf -X POST https://emem.dev/v1/cube/resolve \
  -H 'content-type: application/json' -d '{"token":"emem:cube:..."}' | jq '.resolved, .member_tokens'
```

### Enumerate the cells in an area (paged)

When you want the address list rather than the packed grid — a dense recall,
a sample frame, a world build:

```sh
curl -sf -X POST https://emem.dev/v1/cells_in_bbox \
  -H 'content-type: application/json' \
  -d '{"bbox":{"min_lat":32.5699,"min_lng":77.0328,"max_lat":32.5727,"max_lng":77.0362},
       "page_size":1024}' \
  | jq '{total, count, next_cursor, first: .cells[0]}'
```

Page with `next_cursor` until it is null, and feed each page's `cells` to
`POST /v1/recall_many` with a `budget_ms` to read them under the
partial-results contract. Pure geometry: no facts read, no receipt.

## Verify a token you were handed

Resolve first, then re-hash the artifact and check its receipt with the
**emem-verify-receipt** skill:

```sh
curl -sf -X POST https://emem.dev/v1/raster/resolve \
  -H 'content-type: application/json' -d '{"token":"emem:raster:..."}' \
  | jq '{bound: .resolved, artifact: .artifact.url, receipt: .receipt.field}'
```

Every claim in the token binds to the signed record before anything
dereferences: a mismatched area, band, or date is a typed `409`, never a
silent wrong answer.

## Notes

- The artifact is **evictable by design**: the small derivation record
  persists and pins the scene, recipe, and geometry, so an evicted artifact is
  a rebuild recipe (re-POST with the same bbox/band/date), never a broken
  citation.
- For NDVI specifically, `POST /v1/band_raster` gives you B08 and B04 as two
  rasters; NDVI = (B08 − B04) / (B08 + B04) per pixel. Or use
  **emem-locate-and-recall** for the pre-computed, cloud-gated `indices.ndvi`
  scalar at one cell.
- Reads are public; no key. The reference for the whole token family is
  [`https://emem.dev/reference#tokens`](https://emem.dev/reference#tokens).
