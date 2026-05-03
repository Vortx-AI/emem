# Milestone — v0.0.4

> Status: planning · 2026-05-01 · curated by harvesting the leftover
> commitments and "lands in v0.0.X" placeholders that survived the
> 0.0.3 doc audit. This file is the next-milestone guide. It is the
> only place where forward-looking promises live; once a feature
> ships, its entry moves to `CHANGELOG.md` under the corresponding
> release.

The 0.0.3 release closed the Katihar (Bihar) man-made-lake
test-report gaps: temporal_composition + temporal_recipe,
flood_risk@2 with DEM agreement weighting, Sentinel-2/-1 fallback
ladders, adaptive polygon density, Photon-primary geocoder. The
items below are everything else the codebase or docs still owe.

---

## Tier 1 — protocol-shaped commitments still outstanding

### M1. H3-equivalent 3.4 m-class fine grid

- **Today (0.0.3):** `cell64` is a square ~10 m × ~10 m grid at the
  equator (lat 21 bits × lng 22 bits), matching Sentinel-1 / -2
  native pitch. Documented honestly via `/v1/grid_info`.
- **Spec target (`docs/SPEC.md` §0):** `res=13`-class equal-area
  hex (≈3.4 m), so two queries 4 m apart land in distinct cells.
- **Why it matters:** ≤10 m sub-pitch features (roof footprints,
  pavement vs. lawn, a single canal vs. its bank) cannot be
  disambiguated by the current grid even when the underlying
  satellite supports it.
- **Concrete plan:** vendor the H3 v4 reference indexer (or
  `h3o` Rust crate, MIT) under `emem-codec::h3compat`, add a
  `cell64.mode = HexAperture7` variant with bit layout `mode (1
  bit) | res (4 bits) | base_cell (7 bits) | path (52 bits)`,
  switch the default `LocateReq.fidelity = "h3_res13"` on a
  feature flag, and dual-write attestations until the cache
  has both keys. Keep the square 10 m mode as `mode =
  Square10mEquator` so 0.0.3 receipts continue to verify.

### M2. lcv-1 learned land-cover taxonomy (currently placeholder leaves)

- **Today (0.0.3):** `landcover.lcv1_leaf` returns placeholder names
  `lcv-1.f0.l0` … `lcv-1.f7.l7` (SPEC §13).
- **Target:** HDBSCAN over (AlphaEarth-9yr ⊕ S2-monthly ⊕ Köppen ⊕
  ecoregions) at res-9 cell centroids; each leaf gains a 1792-D
  centroid vector so `landcover:lcv-1.43` is *also* a vec64
  address.
- **Open questions:** OQ-9 in `docs/SPEC.md` (clustering
  methodology), OQ-10 (max derivative-of-derivative depth).

### M3. Primitives 6–8 (SPEC §20.6–§20.8)

- **(6)** `intent_routed_plan` — heuristic dispatcher landed in
  0.0.3 via `/v1/intent`. The **learned planner** variant is
  still v0.1 work (SPEC §20.6).
- **(7)** `region_attest` — sign over a `RecallPolygon` rather
  than each cell separately. Requires Merkle root over per-cell
  fact CIDs; receipt schema TBD.
- **(8)** Shared planner traces (SPEC §20.8) — agents cite
  prior plan CIDs to amortise route re-discovery cost.

### M4. Temporal composition canonical receipt

- **Today (0.0.3):** `temporal_composition[]` is an additive sibling
  on `/v1/ask` and `/v1/intent`. Each entry surfaces per-window
  fact CIDs but the **composition CID** itself (a Merkle root
  over the per-window CIDs + recipe CID + aggregator output) is
  not yet computed or signed.
- **Target:** add `composition_cid` per entry and a top-level
  `temporal_composition_root_cid` so a downstream verifier can
  re-execute the recipe and check the aggregator output without
  re-fetching every input. (OQ-12 in SPEC.)

---

## Tier 2 — surface gaps

### M5. Overture Places third-tier geocoder

- **Today:** `/v1/locate` cascade is embedded → cache → Photon →
  Nominatim. Photon and Nominatim both index OSM; if a name only
  exists in Overture (e.g. some India POIs), neither resolves it.
- **Target:** Overture Places parquet `name LIKE` lookup as a
  third tier, gated on `EMEM_OVERTURE_RELEASE` (already
  auto-discovered by `latest_release()`). Expose `via =
  "overture"` in `/v1/locate` responses. (OQ-14 in SPEC.)

### M6. Self-hosted Photon mirror option

- **Today:** Photon depends on `https://photon.komoot.io`. Soft
  dependency on a third-party endpoint.
- **Target:** add `EMEM_PHOTON_MIRROR_DIR` so an operator can
  point at a local Photon Elasticsearch index. Document the
  ~6 GB index footprint per regional shard. (OQ-13 in SPEC.)

### M7. Photon polygon_bbox persistence

- **Today:** Nominatim's `boundingbox` is stored in the geocoder
  cache so a `recall_polygon` after a cached `locate` keeps
  polygon fan-out. Photon's `extent` field is parsed but
  cached-bbox round-trip on Photon-served entries is not yet
  symmetric with the Nominatim path.
- **Target:** mirror the Nominatim cache layout for Photon hits
  so `recall_polygon` after a `via=photon` lookup retains the
  polygon instead of degrading to a single-cell fan-out.

### M8. Landsat path

- **Today:** mentioned as a "target" sensor in the multimodal
  policy (S1 > S2 > **Landsat** > IoT > OtherSat > Static) and in
  several algorithm input lists.
- **Target:** a real `materialize_landsat_*` family backed by
  USGS / AWS Open Data Landsat-8/-9 Collection 2 L2 products,
  same shape as `materialize_s2_*`.

### M9. Tessera embedding completeness

- **Today:** `materialize_geotessera_embedding` (Pure Rust, HTTP
  range, ~640 B/cell) is wired. Multi-year fused Tessera
  embeddings are referenced in the bands manifest but the
  materializer for the *fused* variant is not yet wired.
- **Target:** add `materialize_geotessera_fused_2017_2024` that
  composes per-year embeddings into the fused vector at L2 and
  attests with a `Source.cid` for each input year.

---

## Tier 3 — operator polish

### M10. Hosted Nominatim-mirror option

- Same shape as M6 but for Nominatim. Several deployments cannot
  reach the public Nominatim instance from inside their VPC.

### M11. tslot epoch handling for pre-2026 historical data (OQ-8)

- Today the emem epoch is 2026-01-01T00:00:00Z. Anything before
  that has no canonical `tslot`. Proposal: signed offset (negative
  `tslot` for pre-epoch). Revisit when the first historical-archive
  ingest lands.

### M12. Vision-band L2 attestation flow

- SPEC §11/§13 say vision bands are L2-only and `Source.cid` of
  the model checkpoint MUST be present. The attestation flow for
  a vision band (e.g. Tessera2 visual encoder) is not yet
  wired end-to-end — checkpoint CIDs need to land in `Source`
  and an attester needs to publish them.

### M13. Migrate every algorithm in the registry to carry `evaluation: Expr`

- **0.0.3 shipped** the formula-AST evaluator (`Expr` in
  `emem-core::algorithms`) and the dispatcher
  (`dispatch_algorithms` in `emem-api-rest`); `flood_risk@2` is
  the proof-of-concept that round-trips through it (test:
  `flood_risk_v2_evaluates_to_a_real_number_from_dispatcher`).
  The other 101 algorithms still ship only the human-readable
  `formula: String`.
- **What's left:** add an `evaluation` block to every algorithm
  whose formula fits the existing AST primitives (linear sums,
  weighted blends, sigmoid / relu, clamps, threshold branches).
  Algorithms that need a new primitive (polynomial regression,
  elementwise vector ops, classification trees) extend the AST
  under semver — the AST kind enum is `serde(tag = "op")` so
  new variants are forward-compatible.
- **Bottom-up order:** start with single-input algorithms
  (`vegetation_class_from_ndvi@1` → classification, needs
  AST extension for "case-when"), then weighted-mean
  composites (`water_consensus@1`, `walkability_score@1`,
  `parametric_trigger@1`), then complex composites with
  thresholds and DEM-agreement (`flood_risk@2` is already
  done; `wildfire_exposure_score@1` next).

### M14. Sign composite outcomes as `DerivativeFact` and surface their CID

- **0.0.3 shipped** byte-stable composite values via
  `algorithm_outcomes[]` on `/v1/ask`, but the values are
  **not yet attested**. A downstream verifier can re-execute the
  AST and check the number, but there's no canonical CID for
  the composite itself.
- **Target:** wrap each successful evaluation in a
  `Fact::Derivative` (`emem-fact::DerivativeFact`) carrying
  `algorithm_cid`, `input_fact_cids[]`, the evaluator
  `expr_cid` (BLAKE3 over the canonical-CBOR Expr), and the
  scalar value. Sign and emit alongside the snapshot facts so
  `/v1/ask` returns *signed* composites that downstream agents
  can quote in receipts.
- **Open question:** OQ-12 (is `composition_cid` a Merkle root
  over per-input + recipe CIDs? Or a flat CID over the whole
  derived fact?). The `temporal_composition[]` block has the
  same question — both should land at the same answer.

### M15. Multimodal upload surface — POST /v1/contribute

- **Where we are today (0.0.3):** every fact in the corpus is materialized
  by the responder from open upstream sources (Sentinel-2, Cop-DEM, JRC
  GSW, …). Agents *cannot contribute* their own observations: there's no
  `/v1/contribute`, no GeoJSON upload, no signed-claim ingestion path.
  `/v1/agent_card` already declares this honestly under
  `multimodal_io._status_summary`: "INGESTION is JSON/CBOR only — no
  image upload or binary-vector-as-payload endpoint yet."
- **Target:** add `POST /v1/contribute` accepting one of:
  1. **Tabular observation** — `{cell, band, value, observed_at_unix,
     attester_pubkey, attester_sig, evidence_uri?}`. The responder
     verifies the ed25519 signature, validates `band` is in the
     manifest + `value` is in `value_range`, persists as a
     `Fact::Primary` with the *attester's* signature (not the
     responder's), and adds to the per-cell append-log.
  2. **GeoJSON polygon batch** — `{features: [{geometry, properties:
     {band, value, observed_at_unix}}]}`. Each feature's centroid
     resolves to a cell64; the responder fans out one signed fact per
     cell × per band. Cap at 1024 features per call.
  3. **Image upload** — `multipart/form-data` with `file` (PNG/JPG/
     GeoTIFF) + `cell` + `tag`. The responder stores the bytes under
     a content-addressed CID, returns the CID, and emits a
     `Fact::Evidence` referencing it — *not* a Primary fact (image
     interpretation is left to downstream tooling). 50 MB per upload
     cap.
- **Why this matters for agents:** lets them turn user-provided GeoJSON
  ("here's the field boundary") or photo evidence ("here's what the
  flood looked like") into signed facts the next call can recall + cite.
  Closes the loop — emem stops being read-only.
- **Open question:** rate-limiting and abuse model for an unauthenticated
  `/v1/contribute`. Likely answer: require ed25519-signed envelope from
  day one, treat any unsigned upload as a 401. The signed claim then
  carries the attester's identity into the receipt; downstream readers
  can choose whether to trust that pubkey.

### M16. Snapshot-band materialization parallelism

- **Where we are today (0.0.3):** `dispatch_temporal_recipes` and
  `run_temporal_window` parallelise temporal samples and per-window
  fetches via `tokio::task::JoinSet` (added 2026-05-03 after observing
  flood-risk /v1/ask hit the 60 s gateway timeout). The bumped 180 s
  default keeps cold calls successful.
- **Remaining bottleneck:** `try_materialize_bands` (the snapshot-mode
  per-band materializer used by `recall_with_auto_materialize`) is
  still a `for b in bands` serial loop covering ~30 match arms across
  Open-Meteo / STAC / SoilGrids / Hansen / NASA POWER. A 15-band
  flood-risk question pays serial 15 × 1–5 s on cold cells. Net
  observable: a warm /v1/ask still lands at ~10 s; a cold one at
  60 s+ (the temporal recipes now go fast, but the snapshot is the
  long pole).
- **Target:** refactor `try_materialize_bands` so each iteration spawns
  via `JoinSet`. Either extract a `materialize_one_band(cell, band, s)`
  helper containing the ~660-line match (mechanical), or unify with
  `materialize_band_at` (already a parallel-safe dispatcher used by
  the temporal layer) by routing snapshot mode through it with
  `target_unix = now`. The unification cuts the dispatcher count from
  two to one and avoids the ~659-line copy.
- **Risk:** `materialize_band_at` currently returns
  `"no historical materializer registered"` for some bands the
  snapshot path handles directly (e.g. Overture themes that only
  have a "now" semantic). A test sweep across all bands listed in
  `/v1/materializers` is the gate.

---

## Pointers
- Active version: `0.0.3` (workspace `Cargo.toml`).
- Next milestone: `0.0.4` (this file).
- Spec: `docs/SPEC.md` (currently `v0.0.3-draft`).
- Whitepaper: `docs/WHITEPAPER.md`.
- Open questions: SPEC.md §"Open Questions" — OQ-1 through OQ-14.
