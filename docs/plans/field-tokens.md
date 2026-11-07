# Field tokens: what the receipt attests when the answer is an array

This page is a design record. The owner signed off the design on
16 July 2026. All five build steps ship, and the owner closed the open question.
The current token table is in [docs/protocol.md](../protocol.md). To use
field tokens from an agent, see the `emem-field-tokens` skill in
[plugins/emem/](../../plugins/emem/).

## Status on 7 November 2026

The code is `crates/emem-api-rest/src/band_raster.rs`. The worked example
below ran on emem.dev on 7 November 2026.

| Item | Status | Where |
|---|---|---|
| FIELD preimage segment | Ships | `/v1/verifier_spec`, scalar `field_hex`, tag 10, optional |
| Canonical grid encoding | Ships | `crates/emem-codec/src/grid.rs`, media type `application/x.emem-grid-f32.v1` |
| Evictable artifact store | Ships | `crates/emem-storage/src/artifacts.rs`, `GET /v1/artifacts/{cid}` |
| One field over an area | Ships | `POST /v1/band_raster`, MCP `emem_band_raster` |
| Resolve a field token | Ships | `POST /v1/raster/resolve`, MCP `emem_raster_resolve` |
| Anchors spot-check | Ships, opt-in | `"spot_check": true` on the raster resolve |
| One field over time | Ships | `POST /v1/band_cube` and `POST /v1/cube/resolve`, MCP `emem_band_cube` and `emem_cube_resolve` |
| A set of field tokens | Ships | `POST /v1/raster_bundle` and `/v1/raster_bundle/resolve`, MCP `emem_raster_bundle` and `emem_raster_bundle_resolve` |
| Per-pixel signatures, chunk Merkle reads | Not built | Out of scope for version 1 (see below) |

Three points differ from the original design:

- The derivation record is the value of a `DerivativeFact`. This reuses the
  attribution-ledger envelope, so the record gets persistence, the Merkle
  log and `emem:fact:` resolution with no new code.
- A `band_raster@1` record has 5 anchors: the four corners inset by 2% of the box, and
  the centre. The design said 4 to 16.
- `sources[]` pins the scene id, asset URL, catalogue and capture time.
  It has no byte hash of the upstream file.

The OpenAPI request schema for `/v1/raster/resolve` lists only `token`.
The server and the MCP tool also accept `spot_check`.

## The question

`POST /v1/band_raster` returns a native-resolution window over an area of
interest. A 2 km area holds about 40,040 cells at 10 m. Every earlier
receipt binds `(cell, fact_cid)` pairs, one pair for each signed record.
A raster is bulk bytes with no cell and no `fact_cid`. A signature on a
byte stream alone proves nothing about the content. So what does the
responder attest?

## The answer

The receipt attests a derivation, not an array. It says: "this responder
computed the artifact whose BLAKE3 is `artifact_cid`. It used these
pinned upstream sources, this versioned recipe, this content-addressed
geometry and this time slot." A reader can check each of these five
claims independently.

## The design

### The artifact has a content address

`artifact_cid = blake3(artifact_bytes)`. It is the full 32 bytes in
base32, 52 characters, the same form as a `fact_cid`. Each media type has
one canonical encoding, and the derivation record names it.

The first encoding is a little-endian `f32` grid in row-major order. A
fixed 64-byte header holds the magic `EMEMGRD1`, the dimensions, nodata
and the CRS. A GeoTIFF is only a lossy view of the artifact. The
signature never covers a GeoTIFF, because compressors do not give the same
bytes across versions.

### The signed object is a derivation record

The record has schema `emem.field_derivation.v1` and is canonical CBOR.
The responder hashes and signs it like any fact. It holds these fields:

| Field | Meaning |
|---|---|
| `aoi`, `aoi_cid` | The request geometry and the BLAKE3 of its canonical CBOR. The same area always gets the same `aoi_cid`. |
| `band`, `tslot` | The observable and its time slot (days since 1970-01-01). |
| `fn_key` | The versioned recipe in the algorithm registry, for example `band_raster@1`. |
| `sources[]` | The pinned upstream scene. |
| `artifact` | `artifact_cid`, `byte_len`, `media_type` and `grid` (dimensions, pixel pitch, EPSG code, origin). |
| `anchors[]` | Sample cells inside the area. Each has its row, column and grid value, and the `fact_cid` of the per-cell fact when the store holds one. |
| `anchor_policy` | The rule for anchors (best effort, see the last section). |

The receipt binds `(aoi_cid, derivation_cid)` through a tagged,
length-prefixed preimage segment. The tag constants are append-only.
`/v1/verifier_spec` lists the segment, so an offline verifier learns it
from the spec and not from prose. A receipt with no field binding hashes
to the same bytes as before the segment existed.

### Verification has two tiers

| Tier | What the reader does | Needs upstream access |
|---|---|---|
| Spot-check | Hash the artifact and compare with `artifact_cid`. Read the pixel at each anchor and compare with the anchor value. If an anchor has a `fact_cid`, resolve that fact and compare again. | No |
| Recompute | Fetch the pinned sources, run the recipe, and compare digests. | Yes |

The anchors connect the artifact to the per-cell facts. To pass the
spot-check, a forged artifact must also forge signed per-cell facts. The
recompute tier is the same replay that every derived fact supports.

The responder runs the spot-check when the resolve request has
`"spot_check": true`. The answer gets a `spot_check` block with a
verdict for each anchor and a `passed` flag. If the store evicted the artifact,
the block says `checked: false` and gives the reason.

### The token grammar

| Token | Names | Resolve route |
|---|---|---|
| `emem:raster:<aoi_cid>:<band>:<tslot>:<derivation_cid>` | One field | `POST /v1/raster/resolve` |
| `emem:cube:<aoi_cid>:<band>:<tslot_lo>..<tslot_hi>:<derivation_cid>` | One field over 2 to 24 dates | `POST /v1/cube/resolve` |
| `emem:rasterset:<bundle_cid>:<derivation_cid>` | 2 to 64 field tokens | `POST /v1/raster_bundle/resolve` |

Each token claim must match the signed record, or the resolve refuses
with HTTP 409. This is the same rule as for fact tokens.

The design gave the cube token `cube_cid` as its last segment. As built,
the last segment is the derivation cid, and `cube_cid` is a field in the
record. `cube_cid` is the BLAKE3 of the canonical CBOR of the member
derivation cids in ascending time-slot order. The cube resolve
recomputes it and refuses a changed membership.

### Storage can evict because the record can rebuild

Artifacts live in a content-addressed blob store under
`$EMEM_DATA/artifacts`. `EMEM_ARTIFACTS_MAX_BYTES` sets the cap, and the
default is 4 GiB. When the store is full, it removes the blobs with the
oldest file time (mtime). A read or a repeated write updates that time.

Eviction never breaks a citation. The derivation record is small and
persists like any fact. It holds what a reader needs to rebuild the same
bytes. If the artifact is gone, the resolve says so in
`artifact.if_evicted`, and `GET /v1/artifacts/{cid}` returns a typed 404.
This applies the cell-build-graph rule: the store can evict derived layers,
and the citations stay valid.

### What this design does not do

- It has no per-pixel signatures and no Merkle-chunked partial reads in
  version 1. To verify a smaller window on its own, request it as its own
  area. A chunk Merkle root can replace `artifact_cid` later with no change
  to the token grammar.
- It does not fill gaps. A pixel with no source data is nodata, never an
  interpolation. Densified or synthesized fields stay labelled, separate,
  and outside any `measured` artifact.
- It adds no trust class. The artifact gets the provenance class of its
  band, and the record says so.

## A worked example

These calls ran on emem.dev on 7 November 2026, over a box of about
550 m on the side of Mount Fuji. The `band_raster` call reads a scene and
persists a derivation record. It needs no key.

```sh
curl -s https://emem.dev/v1/band_raster -H 'content-type: application/json' \
  -d '{"bbox":{"min_lat":35.36,"min_lng":138.72,"max_lat":35.365,"max_lng":138.726},"band":"s2.B04"}'
```

The answer named Sentinel-2 scene
`S2C_MSIL2A_20261028T012741_R074_T54STE_20261028T043307` and tslot 20754.
The grid was 56 by 55 pixels at 10 m in EPSG:32654. The artifact had
12,384 bytes: 64 header bytes plus 4 bytes for each of 3,080 pixels. The checks gave
these results:

| Check | Result |
|---|---|
| BLAKE3 of the bytes from `GET /v1/artifacts/{cid}` | Equal to `artifact_cid` |
| Grid value at each of the 5 anchors | Equal to the anchor value |
| Resolve with `"spot_check": true` | `passed: true`, `grid_matches_record: 5`, `anchors_with_fact: 0` |
| Resolve with the tslot changed to 20753 | HTTP 409, "token tslot 20753 contradicts the signed record" |
| `band_cube` for 2026-09-15 and 2026-10-28 | 2 members. The nearest scenes were on 13 and 28 October 2026 (tslots 20739 and 20754). |

No anchor had a per-cell fact, because nothing had read those cells
before. This is the best-effort policy at work.

The MCP form of the resolve:

```json
{"name": "emem_raster_resolve",
 "arguments": {"token": "emem:raster:<aoi_cid>:s2.B04:20754:<derivation_cid>", "spot_check": true}}
```

## Build order

| Step | Work | Status |
|---|---|---|
| 1 | Preimage segment tags and verifier-spec rows (emem-attest), with test vectors | Ships |
| 2 | Derivation record and content-addressed blob store (emem-fact, emem-storage) | Ships, record as a `DerivativeFact` |
| 3 | `band_raster@1` recipe, `f32` grid encoder, executor over `cog::sample_window` | Ships |
| 4 | The token forms, with resolve and refuse rules | Ships, raster, cube and rasterset |
| 5 | MCP tools and OpenAPI surface, counts and docs in the same commit | Ships |

Before step 3, every call site of `cog::sample_window` discarded its
native-resolution output.

## The open question, now closed

The question was: must `anchors[]` include at least one cell with an
existing per-cell fact? A required anchor gives the strongest bridge.
But a cold area would then materialize one fact for each raster call and
pay its latency. That is the gateway-timeout trap that
[partial-results.md](partial-results.md) describes.

The owner chose best effort. An anchor cites an existing per-cell fact
when the store holds one, and the responder never materializes a fact to
make the bridge. The record shape carries both cases.
