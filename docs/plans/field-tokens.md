# Field tokens: what the receipt attests when the answer is an array

**Status: design for owner sign-off, 2026-07-16. Nothing here ships;
this is the deliberate answer the roadmap's raster item required before
any of it may ship.**

## The question

`POST /v1/band_raster` will return a native-resolution window for an
area of interest: tens of thousands of pixel values as one artifact. A
2 km AOI holds about 40,040 cells at 10 m. Every receipt emem has ever
signed binds `(cell, fact_cid)` pairs, one per signed record; a raster
is the first response that is bulk bytes with no cell and no fact_cid.
Stapling a signature onto a byte pipe would make "signed" mean nothing.
So: what exactly does the responder attest?

## The answer in one sentence

The receipt attests a DERIVATION, not an array: "this responder
computed the artifact whose blake3 is `artifact_cid`, from these pinned
upstream sources, via this versioned recipe, over this
content-addressed geometry and this time slot," and every one of those
five claims is independently checkable.

## The design

### 1. The artifact is content-addressed like everything else

`artifact_cid = blake3(artifact_bytes)`, full 32 bytes, base32 (52
characters, the fact_cid convention). The bytes are one canonical
encoding per media type, stated in the derivation record (first target:
a deterministic little-endian `f32` row-major grid with a fixed 64-byte
header carrying dims, nodata, and CRS; GeoTIFF rendering is a lossy
VIEW of the artifact, never the signed thing, because compressors are
not byte-stable across versions).

### 2. The signed object is a derivation record, not the pixels

A `FieldDerivation` record (canonical CBOR, hashed and signed like any
fact) carries:

- `aoi_cid`: blake3 over the canonical CBOR of the request geometry,
  so the same area always names the same field.
- `band`, `tslot`: the observable and its time bucket.
- `fn_key`: the versioned recipe (`band_raster@1`) in the algorithm
  registry.
- `sources[]`: the pinned upstream, scene id plus byte hash where the
  upstream provides one, the same pinning recall provenance uses.
- `artifact_cid`, `byte_len`, `media_type`, `grid`: dims, pixel pitch,
  CRS, nodata.
- `anchors[]`: between 4 and 16 sampled cell64s inside the AOI, each
  with the pixel value the artifact holds at that cell and, where the
  store holds one, the fact_cid of the ordinary per-cell fact for the
  same `(cell, band, tslot)`.

The receipt binds `(aoi_cid, derivation_cid)` through a new tagged,
length-prefixed preimage segment (append-only tag constants, so
`/v1/verifier_spec` serializes it automatically and the offline
verifier learns it from the spec, not from prose).

### 3. Verification has a cheap tier and a total tier

- **Spot-check (milliseconds, no upstream):** recompute
  `blake3(artifact_bytes)` against `artifact_cid`, then check the
  `anchors[]`: read the artifact's pixel at each anchor cell and
  compare against the anchor value, and where an anchor carries a
  fact_cid, resolve that ordinary fact and compare again. The anchors
  bridge the new artifact trust to the existing per-cell trust: a
  forged artifact must now also forge signed per-cell facts to pass.
- **Recompute (the full audit):** fetch the pinned sources, run
  `band_raster@1` (deterministic by construction: pinned inputs, fixed
  resampling, canonical encoding), and compare digests. This is the
  same replay story every derived fact already tells, at field size.

### 4. The token grammar extension

- `emem:raster:<aoi_cid>:<band>:<tslot>:<derivation_cid>` names one
  field. Dereference returns the derivation record plus the artifact
  bytes (or a 404 with the recompute recipe if the artifact was
  evicted; see 5). Anchor-binding rule as for fact tokens: every claim
  in the token must match the signed record or the resolve refuses.
- `emem:cube:<aoi_cid>:<band>:<tslot_a>..<tslot_b>:<cube_cid>` names a
  stack; `cube_cid` is the blake3 of the ordered list of the member
  derivation cids, the bundle construction reused.

### 5. Storage is evictable because everything is recomputable

Artifacts live in a content-addressed blob store keyed by
`artifact_cid`, size-capped with LRU eviction. Evicting bytes never
breaks a citation: the derivation record is small, persists like any
fact, and carries everything needed to rebuild the identical bytes.
This is the cell-build-graph promise ("derived layers are evictable
without breaking citations") applied to its first artifact-typed value.

### 6. What this deliberately does not do

- No per-pixel signatures and no merkle-chunked partial reads in v1;
  a window that wants independent verification is its own AOI. A
  chunk-merkle root can replace `artifact_cid` later without changing
  the token grammar (the cid stays "the digest the receipt binds").
- No generative fill. A pixel with no source data is nodata, never an
  interpolation; densified or synthesized fields keep the splat rule
  (labelled, separate, never inside a `measured` artifact).
- No new trust class: the artifact inherits the band's provenance
  class, and the derivation record says so.

## Build order, once signed off

1. The preimage segment tags plus verifier-spec rows (emem-attest),
   with test vectors.
2. `FieldDerivation` record and CAS blob store (emem-fact,
   emem-storage).
3. `band_raster@1` recipe, the canonical `f32` grid encoder, and the
   executor over `cog::sample_window` (whose native-resolution output
   every current call site destroys).
4. The two token forms in the grammar, resolve and refuse rules.
5. MCP tool and OpenAPI surface; counts move; docs flip in the same
   commit, as always.

## The open question that stays open

Whether `anchors[]` should be REQUIRED to include at least one cell
with an existing per-cell fact (strongest bridge, but a cold AOI would
then materialize one fact per raster call and pay its latency), or
best-effort (weaker bridge, no forced materialization). Owner's call at
build time; the record shape carries both.
