# Non-geographic spatial memory — research direction

> The cell64 codec encodes a Hilbert-ordered position in a unit
> square `[0,1]²`. Earth coordinates project into that square via
> equirectangular `(lng, lat) → ((lng+180)/360, (lat+90)/180)`. But
> the codec itself doesn't care about Earth — it cares about a 2D
> manifold with a known bijection to the unit square.
>
> So the same mechanism gives **locality-preserving short codes for
> any 2D space an agent can name**: a building floor plan, a software
> architecture diagram, a relational-schema dependency map, a
> warehouse layout, a chess-game game-tree projection, or the
> embedding plane of a latent space after a 2D projection.

This is a stretch direction prompted by external feedback (Gemini's
review of emem.dev, 2026-04). It is **not shipped** — this file
documents the math and what the protocol-level extension would look
like so a future implementer doesn't reinvent it.

---

## What "spatial" means without Earth

A *space* is a tuple `(name, axes, bbox, projection)` where:

- `name` — human label and its CID (e.g. `space:floorplan/vortx-hq-v1`,
  `space:repo/emem.git`, `space:latent/clip-32d-tsne`).
- `axes` — names + units for each of the two coordinates. For
  geography these are `(lng_deg, lat_deg)`. For a floor plan they
  might be `(metres_x, metres_y)`.
- `bbox` — `(min, max)` per axis. For geography it's the WGS-84
  global box. For a floor plan it's the building outline.
- `projection` — the map from the user's coordinates into `[0,1]²`.
  For geography we use equirectangular. For a floor plan, an affine
  fit. For a software architecture diagram, the layout coordinates
  produced by graphviz/sfdp.

Once you have these, the cell64 codec applies unchanged: encode the
point's `(u, v) ∈ [0,1]²` with the same Hilbert curve at the same
resolution, prefix with the space CID, and you have a content-
addressed cell64-equivalent for that space.

```
geo:    "defi.zb493.xoso.zcb6a"                  (current)
floor:  "space:floorplan/vortx-hq-v1#defi.zb493.xoso.zcb6a"
arch:   "space:repo/emem.git#defi.zb493.xoso.zcb6a"
latent: "space:clip-tsne-v3#defi.zb493.xoso.zcb6a"
```

The `#` is the namespace separator — old cells keep their bare form
for backwards compatibility; new spaces are explicitly prefixed.

---

## Why locality preservation still matters off-Earth

The whole reason the geo cell64 is useful for LLMs is that *spatially
close points have lexically close strings*. A model sees
`defi.zb592.nemu.zEvE` and `defi.zb493.xoso.zcb6a` and the attention
mechanism can spot they share a 12-character prefix; it doesn't need
explicit math to know they're nearby.

That property comes from the Hilbert curve, not from the Earth. The
moment you have any 2D space mapped to `[0,1]²`, you get the same
benefit:

- **Building floor plan**: rooms in the same wing share a prefix.
  An agent reasoning about routing through the building "knows"
  which rooms are physically adjacent without an explicit graph.
- **Software architecture diagram**: services placed by graph layout
  near each other in the picture share a prefix. An agent reading a
  trace can spot related modules by token-prefix similarity.
- **Latent embedding plane (2D-projected)**: vectors that t-SNE /
  UMAP placed near each other share a cell-prefix. An agent can
  cite "the cluster around `defi.zb493.*`" without dumping
  hundreds of vectors.

The math is the same; the only thing that differs is which `(u, v)`
point you encode.

---

## What the protocol extension would look like

1. **New endpoint `POST /v1/spaces`**
   Register a space with `name`, `axes`, `bbox`, `projection_kind`,
   and (for affine projections) the 6-parameter matrix. Returns a
   space CID. Spaces are content-addressed, immutable, and indexed
   in a sled tree `emem.spaces`.

2. **New endpoint `GET /v1/spaces`**
   List spaces. Lets agents discover what 2D spaces this responder
   knows about and what bands (band manifest) are defined for each.

3. **Cell encoding update**
   `cell64::encode(space_cid, u, v) → "<space_cid>#<bigrams>"`,
   `cell64::decode("<space_cid>#<bigrams>") → (space, u, v)`.
   Existing geographic cells (no `#`) get an implicit
   `space:earth-wgs84/v1` prefix.

4. **Band registry per-space**
   Each space declares its own band manifest. For a floor plan,
   bands might be `floorplan.room_temp_c`, `floorplan.occupancy_count`.
   For a repo, `repo.module_loc`, `repo.test_coverage_pct`. The
   responder serves `/v1/bands?space=<cid>` so an agent can ask
   "what data exists in *this* space?"

5. **Recall + attest unchanged**
   Once a cell is `(space, bigrams)`, every existing primitive
   (`recall`, `compare`, `find_similar`, `query_region`) works the
   same way. The cube algebra and content-addressing scheme don't
   care what "space" means.

---

## Reference points in the literature

- **FloorplanQA** ([arxiv.org/abs/2507.07644](https://arxiv.org/abs/2507.07644))
  — benchmark for spatial reasoning over indoor layouts; an agent
  with locality-preserving room codes does better than one parsing
  raw `(x, y)` coordinates.
- **AlphaEarth Foundations** ([arxiv.org/abs/2507.22291](https://arxiv.org/abs/2507.22291))
  — embeds Earth into 64 dims globally. The same encoder family,
  retrained over a non-geographic 2D space, gives non-geographic
  embeddings with the same locality property.
- **State of AI Agent Memory 2026** ([mem0.ai](https://mem0.ai/blog/state-of-ai-agent-memory-2026))
  — production agent memory now combines vector retrieval with
  graph relationships. cell64-style locality codes are the geometric
  third leg of the same stool.
- **OGC Cloud-Native Geospatial Concept Study** ([docs.ogc.org/per/21-023.html](https://docs.ogc.org/per/21-023.html))
  — establishes that "spatial" + "cloud-optimized" is a generic
  storage problem, not an Earth-specific one. Validates the
  decoupling proposed above.

---

## What this *doesn't* solve

- 3D / volumetric memory. The codec is fundamentally 2D. A 3D
  variant would need a 3D space-filling curve (Morton-3D or
  Hilbert-3D) and a different bigram encoding.
- High-cadence ephemeral memory ("I just saw a truck at cell X").
  That's a separate event-bus problem; cell64 gives you the *where*
  but the *when* + *propagation* require a sub-second pub/sub layer
  that doesn't exist in emem yet.
- Indoor positioning accuracy. The geographic codec produces square
  ~10 m × ~10 m cells at the equator; rescaled to a 100 m × 100 m
  floor plan, the same 43-bit lat/lng budget gives ~3 mm cells, which
  is enough for furniture-level reasoning. The full 64-bit budget
  (lifting the mode/resolution prefix for the indoor mode word)
  pushes that into the sub-mm regime, sufficient for robotic-grasp
  precision when paired with a global-to-local frame anchor.

---

## When to do this

When emem.dev has a paying customer or a clear research use case
that needs a non-geographic space. Until then, this file is the
contract: anyone implementing it should preserve the four-part
extension above so future spaces are interoperable across responders.
