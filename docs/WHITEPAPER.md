# emem — the Earth Memory Protocol

**Version 0.0.4** · Vortx-AI · Apache-2.0
github.com/Vortx-AI/emem · live at https://emem.dev

emem is an agent-native, content-addressed, lazy-materialization protocol
for spatial memory at planetary scale. Every fact about every place is a
signed, hashable, recall-able tuple of `(cell, band, tslot)` — and every
read is a signed receipt that downstream agents can audit, compose, and
verify offline.

This whitepaper summarises the math, the address algebra, the canonical
encoding rules, and the agent-facing surfaces. The wire-stable protocol
spec lives in `docs/SPEC.md`; this document is the *reasonable shorthand*
for engineers integrating emem.

---

## 1. Vision: memory, not a service

LLMs hold language models, not Earth models. When an agent answers a
question about a place, it confabulates because it has no shared,
cite-able memory of *what is true at that place at that time*. emem is
that memory: a global, content-addressed log of attested facts that any
agent can query, verify, and extend.

emem is intentionally **not** a SaaS:

- Every fact is content-addressed. The CID is `base32(blake3(canonical_cbor(fact)))`.
- Every attestation is signed (`ed25519`) and merkle-rooted.
- Every read is a signed receipt. Agents can prove provenance offline.
- The protocol is the loader, the validator, the CID rule, and the
  primitive semantics. The data — bands, functions, sources — lives in
  content-addressed manifests that operators publish and replicate.

The protocol's unit of value is the receipt, not the API call.

---

## 2. Address algebra

Every fact is keyed by **`(cell, band, tslot)`** — three orthogonal axes.

### 2.1 Cell

The wire format is a 64-bit packed integer. Two grids share the same
codec and bit-prefix locality property; **GET `/v1/grid_info` is the
authoritative declaration of which is active in a given build**, so
clients never have to guess what a cell64 string means.

**Spec target — aperture-7 hex DGGS** (per `docs/SPEC.md` §3,
H3-equivalent geometry). Default fact resolution 13 (≈ 3.4 m hex
edge), maximum 15. Bit layout:

```
[63]      reserved (must be 0)
[62..59]  mode (4 bits, 16 modes — Cell, DirectedEdge, UndirectedEdge, Vertex, Set, …)
[58..56]  edge/vertex disambiguation (3 bits)
[55..52]  resolution (4 bits, 0..=15)
[51..45]  base cell (7 bits, 0..=121 valid)
[44..0]   path: 15 × 3-bit child digits
```

**Ships today (active in 0.0.x) — `cell64-geo-21x22`**, a packed
lat/lng quantisation: 21 lat bits × 22 lng bits, square ~10 m × ~10 m
at the equator (matching Sentinel-1 / Sentinel-2 native pitch).
Above the equator, longitude pitch narrows with cos(lat) so cells
become taller than wide — H3 fixes this with equal-area hexagons
once the migration lands. Until then, `/v1/grid_info.honest_warnings[]`
makes the geometry mismatch explicit, and migration will issue new
strings under a new mode prefix so historical receipts pin
unambiguously to the manifest CIDs in force at attest time.

The bit layout is *itself* locality-preserving in both grids: cells
with shared bit-prefix share string-prefix in the cell64 codec.

### 2.2 Band

A logical channel within a 1792-dimensional embedding contract. Bands
are not Rust constants — they load from the **band-ontology manifest**
(`emem-bands` v0). Each band declares: key, family (Optical, Radar,
Terrain, Climate, etc.), offset within the 1792D layout, dim count,
tempo class, and privacy class. Operators publish their own band
manifest CIDs to extend or restrict the contract.

### 2.3 Tslot

An unsigned u64 offset from the emem epoch (2026-01-01T00:00:00Z) in
units determined by the band's tempo class:

| Tempo       | Slot duration | Example bands         |
| ----------- | ------------- | --------------------- |
| `static`    | n/a (slot 0)  | DEM, Köppen           |
| `slow`      | 1 year        | Tessera, Prithvi, Galileo, soil |
| `medium`    | 30 days       | NDVI composites       |
| `fast`      | 1 day         | raw S2 NDVI           |
| `ultra_fast`| 1 hour        | weather, traffic      |

---

## 3. Codecs (token-economical, locality-preserving, round-trippable)

emem is designed for AI agents that reason in tokens. Four codecs
trade between density and locality so chat-window references stay
cheap *and* spatially meaningful.

| Codec    | Purpose                                            | Token target |
| -------- | -------------------------------------------------- | ------------ |
| `cell64` | 64-bit cell → 4-bigram string, locality-preserving | ≤ 4 tokens   |
| `tslot`  | u64 time slot → base32 short form                  | ≤ 2 tokens   |
| `vec64`  | 1792D fp16 vector → 8-byte blake3 prefix, base32   | ≤ 3 tokens   |
| `cid64`  | 32-byte fact CID → 8-byte prefix, base32           | ≤ 3 tokens   |

The cell64 alphabet is the deterministic CVCV product (21 consonants ×
10 vowels in two passes, padded to 65,536 with `z<hex4>` synthetic
suffixes). Operators may publish a measured BPE-optimal alphabet
manifest; the protocol is alphabet-neutral provided the manifest CID
matches between responder and replica.

**Round-trip invariant**: every codec round-trips losslessly for every
input — `from_cell64(to_cell64(c)) == c` for any 64-bit `c`.

---

## 4. Facts

Three variants, each content-addressed via canonical CBOR:

- **Primary** — direct attested observation about `(cell, band, tslot)`.
- **Derivative** — function over parent fact CIDs (`delta`, `mean`,
  `trend`, `rate`, `anomaly`).
- **Absence** — confirmed negative fact with a `reason_cid` evidence
  pointer (distinct from `null` / `unknown`).

The CID rule is identical for all three:

```
fact_cid = base32_nopad_lowercase(blake3(canonical_cbor(fact)))
```

Two implementations parsing the same JSON or CBOR MUST produce
byte-identical canonical CBOR — that is the protocol's primary
soundness guarantee.

---

## 5. Attestations

Facts ship in signed batches. An `Attestation` envelope carries:

- `facts: Vec<Fact>` — one or more facts.
- `batch_root: [u8; 32]` — blake3 Merkle root over the **canonically-sorted** fact CIDs.
- `attester: AttesterKey` — ed25519 pubkey.
- `attester_key_epoch: u32` — supports key rotation + revocation.
- `registry_cid` / `schema_cid` — CIDs of registry + schema in force.
- `signature: [u8; 64]` — `ed25519(blake3(batch_root || registry_cid || schema_cid))`.

Verification is total: the responder recomputes the merkle root from
the received facts and the signature is verified against that root.
A mismatched root or invalid signature returns `BadSignature` (HTTP
422), never silent acceptance.

---

## 6. Receipts (the unit of value)

Every read primitive returns a signed `Receipt`:

- `request_id` (ULID), `served_at` (ISO 8601), `primitive` name.
- `cells` and `fact_cids` cited.
- `responder` pubkey + epoch + ed25519 `signature` over `request_id ||
  served_at || primitive || cells || fact_cids`.
- `cost: { credits, latency_p50_ms, latency_p99_ms, source_freshness_s, was_cached }`.
- `registry_cid` / `schema_cid` in force.
- Optional `MerkleProof` for inclusion proofs against the attestation log.

Agents can compose receipts in chains (recall → verify → diff) and the
chain is independently auditable with only the responder's epoch
pubkey.

---

## 7. Lazy materialization

Storage is a single facade composing cache + fetch + log:

```
Storage::materialize_many(keys) →
    cache hit?     → return CIDs
    cache miss?    → fetch upstream → compute (function registry)
                   → attest → cache forever → return CIDs
```

**Bootstrap == recall**. Pre-warming the cache for popular cells uses
the *exact same code path* as agent-driven recall. There is no
separate ingest pipeline.

The hot tier is a sled DB with two trees:

- `emem.canonical_index` — `(cell ‖ 0x00 ‖ band ‖ 0x00 ‖ tslot_be8)` → fact CID.
- `emem.facts` — fact CID → canonical CBOR bytes.

The `Cache` trait reserves space for warm (parquet) and cold
(content-addressed, IPLD-style) tiers, but the 0.0.x reference build
ships only the sled hot tier and the on-disk Merkle log — no
parquet, no IPFS, no Filecoin client. Multi-tier eviction is part of
the v0.1 roadmap; until then, operators back up by snapshotting the
data directory.

The Merkle attestation log is append-only with 1 GiB segments and
trailing per-segment blake3 hashes. Replay-restore is "for each
segment, re-hash and verify trailing hash."

---

## 8. Open-data fetch (vsicurl, no keys)

The default emem build serves recall against open Earth-observation
data without operator credentials, via anonymous HTTPS Range reads
(vsicurl-equivalent COG window fetch). Default no-auth providers:

- Sentinel-2 L2A via Element84 / Earth Search v1 STAC (anonymous AWS)
- Sentinel-1 GRD via the same STAC endpoint
- Copernicus DEM 30m (anonymous AWS open data)
- GMRT global topo + bathy (Lamont-Doherty)
- MODIS Terra/Aqua products via NASA LP DAAC (LST, NDVI, ET, GPP, LAI, burned area)
- ESA WorldCover 2021 v200 (anonymous AWS)
- JRC Global Surface Water v1.4 (Landsat 1984-2021)
- Hansen Global Forest Change v1.11 2023 release
- SoilGrids 2.0 (ISRIC REST API)
- MET Norway Locationforecast 2.0 nowcast
- Open-Meteo (ERA5 reanalysis, CAMS air quality, Marine wave / SST)
- NASA POWER (MERRA-2 + GEOS) daily reanalysis
- Overture Maps Foundation (anonymous S3 partitions)
- Tessera v1 foundation embedding (Cambridge, vintage 2024)
- Prithvi-EO-2.0-300M-TL (NASA / IBM, Apache-2.0, run locally on CUDA)
- Galileo Base (NASA Harvest, MIT, run locally on CUDA)
- AlphaEarth Foundations v1 (slot reserved; not auto-materialized in 0.0.x — DeepMind has not released open weights and the GEE delivery requires per-pull authentication incompatible with anonymous L0/L1 reads)

The default dispatcher is `emem_fetch::connectors::open_data_dispatcher()`.
Authenticated providers (Earthdata, Sentinel Hub, Mapbox) are wired by
operators registering additional `SourceConnector` implementations.

A 1 GB COG with a 5 × 5 km AOI window touches only a few hundred KB
through HTTP `Range` headers — that is what makes lazy materialization
viable at planetary scale.

---

## 9. Agent surfaces

emem ships *three* agent-facing surfaces on a single binary
(`emem-server`, default port 5051):

### 9.1 REST (developer-facing)

The active route table is generated from `crates/emem-api-rest/src/lib.rs`
and exposed verbatim via `GET /openapi.json`. Grouped by purpose:

```
discovery / health
  GET    /health
  GET    /.well-known/{emem,mcp,openai-mcp,agent-card,agent,ai-plugin,security}.json
  GET    /openapi.json | /v1/openapi.action.json
  GET    /v1/discover                     bootstrap manifest for new agents
  GET    /v1/quickstart | /v1/agent_card | /v1/agent_quickref | /v1/agent_stats
  GET    /v1/grid_info                    declare active cell-grid geometry
  GET    /v1/manifests | /v1/bands | /v1/functions | /v1/sources | /v1/errors | /v1/tools
  GET    /v1/schema | /v1/algorithms | /v1/algorithms/:key | /v1/materializers
  GET    /v1/contributors | /v1/contributors/:pubkey_b32 | /v1/fleet

place / cell I/O
  POST   /v1/locate                       free-text place → cell64 (via embedded → cache → photon → nominatim)
  GET    /v1/cells/:cell64
  GET    /v1/cells/:cell64/info | /geojson | /scene.png | /scene.rgb

recall family (the seven read primitives + bulk + polygon)
  POST   /v1/recall | /v1/recall_many | /v1/recall_polygon
  POST   /v1/query_region | /v1/compare | /v1/compare_bands
  POST   /v1/find_similar | /v1/diff | /v1/trajectory | /v1/verify
  POST   /v1/intent
  POST   /v1/temporal_route               decay-scoring band ranker — see §10.7 and docs/TEMPORAL.md
  POST   /v1/ask                          single-shot free-text answer with signed evidence (§9.4)
  POST   /v1/backfill                     extend tslot history for a (cell, band)
  POST   /v1/data_availability | /v1/coverage_matrix
  GET    /v1/coverage_map.svg

physics primitives (real PDE / forecast solvers — §10.8)
  POST   /v1/heat_solve | /v1/wave_solve | /v1/jepa_predict

attestation, receipts, reviews
  POST   /v1/attest                       signed JSON
  POST   /v1/attest_cbor                  signed canonical CBOR — preferred for byte-exact merkle
  POST   /v1/verify_receipt               offline receipt verification
  POST   /v1/reviews                      attest task-outcome review keyed by subject (fact/cell/band)
  GET    /v1/reviews/:subject_id          read aggregated reviews
  GET    /v1/facts/:cid

domain shortcuts (named bands wired to `/v1/recall`-equivalent semantics)
  GET/POST /v1/{air,at,elevation,forest,lst,ndvi,soil,water,weather}
```

Total live REST surface as of 0.0.4: **65 routes** under `/v1/*` plus
the well-known + static-doc set. The OpenAPI manifest is the
machine-readable source of truth — clients should generate their SDK
from `/openapi.json`, not from this list.

### 9.2 MCP Streamable HTTP (in-loop agent)

```
POST   /mcp
   method: initialize              → { protocolVersion, serverInfo, capabilities }
   method: tools/list              → ~77 tools spanning recall, physics,
                                     multimodal, attest, introspection
   method: tools/call              → invoke any primitive
```

The MCP surface mirrors every REST primitive and adds introspection
tools (`emem_bands`, `emem_manifests`, `emem_errors`, …). The
authoritative count and names come from `tools/list` at runtime — the
0.0.4 build registers approximately 77 distinct `emem_*` tools (the
sharp jump from 0.0.3's 28 reflects the addition of physics solvers,
domain shortcuts, polygon recall, place locate, reviews, GeoJSON /
RGB scene rendering, and per-route GET/POST variants for HTTP-only
clients). `docs/AGENTS.md §10` has paste-ready configs for every
supporting host.

### 9.3 OpenAPI 3.1 (LLM tool discovery)

`GET /openapi.json` returns a hand-rolled OpenAPI manifest covering
every REST route, with JSON schemas for every request body. Agents
that consume OpenAPI tool descriptions (Claude, GPT) can wire emem
without bespoke glue.

### 9.4 Topic routing (the brain behind `/v1/ask`)

`POST /v1/ask` collapses *locate → topic-route → recall → algorithm
hint* into a single signed turn. Routing is content-addressed: every
topic in `crates/emem-core/data/topics-v0.json` (25 topics in the
0.0.4 registry) carries a `description + aliases[]` block that is
embedded once at startup, and a query is matched by cosine similarity
to per-topic centroids. The same question routes the same way as
long as the topic registry CID and the model file are pinned —
receipts stay reproducible.

| backend | model | dim | role |
| --- | --- | --- | --- |
| `ort` (default) | `BAAI/bge-base-en-v1.5` | 768 | direct ONNX via `ort` 2.0.0-rc.12 + `tokenizers` 0.22; cold load ≈ 6.2 s, warm inference ≈ 110 ms |
| `model2vec`     | `minishlab/potion-base-8M` | 256 | distilled, ~1 ms inference, used as a CPU-light fallback |
| `keyword`       | substring over `aliases[]` | — | always-available offline backend, keeps the protocol bootable without any ML toolchain |

The `ort` backend replaced `fastembed-rs` outright in 0.0.4 because
fastembed's session initialiser deadlocked under axum's request
runtime; ort runs inside a `tokio::task::spawn_blocking` warmup with
a 90 s timeout *before* the listener binds, so the first incoming
request is never the one that pays the model-load cost. `EMEM_TOPIC_BACKEND`
selects backend; legacy aliases `fastembed → ort` and
`transformer → model2vec` keep older configs working unchanged.

Topics are returned in descending similarity order, capped at the
registry's `max_topics_per_question` (default 5). For every matched
topic, `/v1/ask` looks up the algorithm registry and surfaces the
applicable algorithm keys (formula, inputs, citation, fetch URL) in
the response's `algorithms_for_question[]` array. **Note: `/v1/ask`
does not auto-dispatch the physics primitives (§10.8) today — it
hands the agent a typed hint that names `heat_equation_2d@1`,
`wave_equation_1d@1`, or `jepa_temporal_predictor@1`, and the agent
must follow up with a separate POST.** Closing this coupling gap is
the next item on the routing roadmap.

`/v1/ask` was evaluated end-to-end in 2026-05 against a 51-question
agent suite spanning health, property, calamity, carbon, ESG,
agriculture, forest, water, and energy. Routing accuracy was 90 %
(46/51 questions matched the expected topic set); 50/51 returned
HTTP 200; mean latency 41.5 s including cold materialisation, ~110 ms
warm. Full report in `docs/EVAL_2026_05_04.md`.

---

## 10. Mathematics

### 10.1 Hashing

- **`blake3`** for content addressing, merkle trees, and signature
  preimages. blake3 is faster than SHA-256 by an order of magnitude on
  modern CPUs and is parallel-tree-friendly.
- **CID** = `base32_nopad_lowercase(blake3(canonical_cbor(fact)))`,
  always 52 chars (256 bits).

### 10.2 Signatures

- **ed25519** (curve25519 EdDSA) for both attestations and receipts.
- 32-byte secret, 32-byte pubkey, 64-byte signature.
- Key epochs allow rotation; revocation is by publishing
  `revoked_at` in `/.well-known/emem.json`.

### 10.3 Merkle batching

Binary merkle tree, blake3 leaves over canonical CBOR. The merkle
root is computed over **canonically sorted** fact CIDs so any
re-ordering of a batch produces the same root.

### 10.4 Cosine similarity

For `compare(a, b)` and `find_similar(key)` over vector-valued bands:

```
cos(u, v) = (Σ uᵢ vᵢ) / (‖u‖₂ · ‖v‖₂)
```

Computed in f64 for accumulation, returned as f32. Zero-vector
handling returns 0.0 (not NaN).

### 10.5 vec64

Vector-as-address: the first 12 bytes of `blake3(canonical_fp16(v))`,
base32-rendered. 96 bits ≈ √(2⁹⁶) = 8 × 10¹⁴ collisions, safely
above the global vector population at full coverage. Full CIDs are
the storage key; vec64 is the inline reference.

### 10.6 Locality

The cell64 alphabet is constructed so adjacent codepoints are spatial
neighbours through the cell ID's own bit structure. Cells in a
sub-tree share string-prefix in cell64, which is exactly what an LLM
sees when an agent quotes a cell in chat — adjacent cells share
adjacent tokens.

### 10.7 Temporal-routing kernels (decay scores, not PDE solvers)

`POST /v1/temporal_route` ranks bands at a cell by a per-band
*staleness score* `Q ∈ [0, 1]` evaluated at the temporal lag
`Δt = |τ_query − t_obs|`. Each band's `tempo` class picks one of
five closed-form kernels, each motivated by the analytical solution
of a PDE in time:

| tempo        | kernel name        | formula                                | inspiration                                 |
| ------------ | ------------------ | -------------------------------------- | ------------------------------------------- |
| `static`     | `identity`         | `Q = 1`                                | constant in time                            |
| `slow`       | `linear_ar1`       | `Q = max(0, 1 − Δt/T)`                 | AR-1 / first-order Markov                   |
| `medium`     | `heat_gaussian`    | `Q = exp(−(Δt/σ)²)`                    | 1-D heat Green's function                   |
| `fast`       | `wave_seasonal`    | `Q = max(0, ½ + ½·cos(2π·Δt/T))`       | sinusoidal traveling-wave solution          |
| `ultra_fast` | `advection_linear` | `Q = max(0, 1 − Δt/H)`                 | 1-D advection over a short horizon          |

The Gaussian kernel *is* the analytical fundamental solution of
`∂u/∂t = D∇²u` collapsed to time only — that math is correct. The
half-cosine kernel is *motivated* by the wave equation's traveling-
wave family but is not a wave-equation solver: there is no second-
order time derivative, no spatial Laplacian, no characteristic-line
solver. The same caveat applies to `advection_linear` (no `v·∇u`
term, no velocity field, no grid).

**What `/v1/temporal_route` is**: a constant-time per-band staleness
ranker over already-attested facts, plus a small (≤ 1.5×) intent-
keyword multiplier for usability. Code is in `quality_kernel`,
`crates/emem-api-rest/src/lib.rs`. The kernels are honest about this
in their `derivation` strings; this section makes it explicit at the
spec level too.

**What it is not**: a 2-D spatiotemporal predictor. The router is
the staleness-ranker. The actual PDE solvers ship as separate
endpoints — see §10.8.

### 10.8 Real PDE primitives (shipped 2026-05)

Three real explicit-finite-difference solvers run alongside the
router. Each evaluates an actual PDE discretisation under a CFL
stability check, signs the result with the responder identity, and
cites every input fact CID in the receipt.

| primitive | endpoint | equation | inputs | scheme |
| --- | --- | --- | --- | --- |
| `heat_equation_2d@1` | `POST /v1/heat_solve` | `∂u/∂t = α∇²u` | 9 × `modis.lst_day_8day` (3×3 stencil) | explicit FTCS, CFL `α·Δt/Δx² ≤ 0.20` |
| `wave_equation_1d@1` | `POST /v1/wave_solve` | `∂²u/∂t² = c²∂²u/∂x²`, `c² = g·h` | N × `gmrt.topobathy_mean` along seaward gradient | explicit CTCS, sinusoidal forcing offshore + hard-wall coast, CFL `c·Δt/Δx ≤ 0.5` |
| `jepa_temporal_predictor@1` | `POST /v1/jepa_predict` | `y_{t+1} = α·(lag-12 NDVI ∨ recent_mean) + β·(last + slope) + γ·recent_mean` | N × `indices.ndvi` (monthly) | constrained AR(2) seasonal predictor, closed-form coefficients (NOT learned) |

The first two are full PDE rollouts on a real cell-grid stencil; the
third is the honest constrained version of the JEPA pattern —
closed-form coefficients (α=0.6, β=0.3, γ=0.1) calibrated from the
agricultural-NDVI literature, not a learned encoder. A learned
`jepa_temporal_predictor@2` is on the design roadmap but is not
shipped, not benchmarked, and is deliberately omitted from the
algorithms manifest until it exists in code.

All three primitives accept either a `cell` (cell64 string) or a
free-text `place` (geocoded via `crate::resolve_cell_field()`) on
the request body, and surface `resolved_from.{kind, label, lat,
lng, via}` so the caller can audit which input path produced the
operating cell.

Each primitive is also registered in the algorithms manifest
(`/v1/algorithms`) with its formula, citation, and CFL bound; an
agent that wants the full math can read the registry entry instead
of inventing thresholds itself. The Rust math is in
`crates/emem-api-rest/src/physics.rs` (pure functions
`heat_solve_3x3_centre`, `wave_step_1d`, `jepa_predict_ar2_seasonal`
are unit-tested without storage).

**Honest limitations of the 0.0.x rollout** (each surfaces as an
explicit response field — none silently degrades):

- **Heat.** When the lazy materialiser populates the 3×3 stencil
  from a single point sample (Open-Meteo style), all nine
  `neighborhood_initial_k[]` entries are equal, the discrete
  Laplacian is exactly zero, and `delta_k = 0`. The agent can
  detect this from the uniform stencil array and either skip the
  call or pre-fetch a real MODIS LST tile crop so the perimeter
  cells have spatial variation.
- **Wave.** The seaward bathymetric profile is generated by
  walking N cells in a fixed heading from the input. If that walk
  lands on continental crust the depth floor (0.01 m) clamps the
  profile, phase speed pancakes to √(g · 0.01) ≈ 0.31 m/s, and the
  arrival height becomes a near-constant placeholder. The
  `depth_profile_m[]` and `phase_speed_profile_m_per_s[]` arrays
  expose this directly; the response also returns a
  `next.longer_profile` hint so an agent can retry with a deeper
  walk or a different `coastal_cell`.
- **JEPA.** When only one fact exists in history (cold cell, no
  prior backfill) `lookback_months_used = 1`, the AR(2) collapses
  to identity, and `forecast_value = history_values[0]`. The
  response surfaces `lookback_months_requested` vs
  `lookback_months_used` so the agent can detect this and call
  `/v1/backfill` for a longer history before relying on the
  forecast.

These three modes are documented in detail with live request/response
samples in `docs/PHYSICS_ENDPOINTS_2026_05_04.md`. The protocol
contract is "every limitation is a visible field, never a silent
fallback" — that is what makes the receipts safe for downstream
agents to compose without hidden assumptions.

---

## 11. Conformance levels

- **L0** — read-only, public-band recall (every emem build serves L0).
- **L1** — verified claims (`/v1/verify mode=fast`).
- **L2** — write (`/v1/attest`). Any contributor with an ed25519
  keypair can attest; the responder accepts on canonical-CBOR +
  signature verification. `/v1/challenge` and stake-based slashing
  are reserved in the wire format but are **not implemented in
  0.0.x** — see SPEC §6.3 and §8.4.
- **L2-reviews** — agents and humans can attest task-outcome
  *reviews* of any subject (fact / cell / band / algorithm) via
  `POST /v1/reviews`. Each review is canonical-CBOR signed,
  content-addressed (`review_cid`), and aggregated under
  `/v1/reviews/:subject_id`. This closes the feedback loop without
  introducing a separate moderation API.

Every receipt declares the active level and registry/schema CIDs.

### 11.1 Lazy materialisation, gated

Behind every read primitive is the materialisation switch
`EMEM_AUTO_MATERIALIZE`. When enabled, an empty `/v1/recall` for a
known band on a known cell triggers a synchronous upstream fetch
(Open-Meteo, Copernicus, JRC, …), the responder signs the
freshly-fetched fact as itself, the fact is persisted to the
content-addressed store, and the receipt comes back with the new
`fact_cid`. The agent sees no difference between a warm cache hit
and a cold materialisation other than latency. When the switch is
off, the responder returns an empty recall plus `bands_already_attested_at_cell[]`
so the agent can distinguish "wrong query" from "place is
genuinely empty" — never a silent fallback.

---

## 12. Why this shape (first principles)

The protocol's surface is not a feature list — it is the smallest closed
set that satisfies five hard constraints. Anything outside that set
is policy; anything missing breaks one of the constraints.

**1. Identity follows from content, not from servers.**
A memory of Earth that any party can host must address its own data.
Hash-derived identity (blake3 over canonical CBOR) is the only way two
independent responders can agree on what a fact *is* without a registry
lookup. Everything addressable in the system — facts, manifests, schemas,
sources — is named by what it contains. Servers become caches; trust
moves to math.

**2. Every answer must carry its own proof.**
Agents reason across contexts they did not generate. An answer without
an artifact is a rumor. Every read therefore returns a receipt: an
ed25519 signature over the canonical preimage of the request, the bound
manifest CIDs, and the answer digest. Verification is offline and key-free
for the verifier — anyone with the responder's pubkey can recompute the
preimage from the receipt alone. This is non-negotiable; it is what makes
the system *cite-able*.

**3. Address space must be cheap to type, parse, and remember.**
Agents pay tokens per character. A 64-bit cell encoded as four
1024-symbol bigrams (`damO.zb000.xUti.zde79`) sits at the entropy limit
of human-and-machine-friendly addressing: 18 ASCII characters, exactly
one BPE token in mainstream tokenizers per bigram, lossless round-trip
with `(lat, lng)` to ≈30 m, total order under Hilbert traversal so
spatial neighborhoods are token-adjacent. A longer address would burn
context; a shorter one would lose precision or order.

**4. Time and space are independent axes.**
The same cell answers different questions in 2014 and 2024. Co-mingling
them in a single index forces either re-indexing or stale joins. emem
keeps `tslot` orthogonal to `cell64`, makes both first-class in the wire
format, and lets every primitive scope time independently. This is why
`compare`, `diff`, and `trajectory` are distinct primitives, not flags
on `recall` — they each impose a different time-axis topology
(snapshot, two-sided difference, ordered sequence).

**5. Public Earth memory cannot depend on private keys.**
A protocol that requires Sentinel Hub credentials to answer "how high is
Mt. Fuji" is not public. The default data plane therefore leans only on
no-auth open-data sources that accept anonymous HTTP `Range` reads —
Copernicus DEM (S3, requester-anonymous), JRC GSW, Hansen GFC, ESA
WorldCover, GHSL, OSM. Key-gated providers can be plugged in by self-hosters,
but the public responder must never need them. Cite-ability without
licensing friction is a prerequisite, not a feature.

### What the read surface must contain, and why exactly seven

The seven read primitives are the closure of "questions an agent asks
about a place" under composition:

- `recall` answers *what is here, now*.
- `query_region` answers *what is across this area* — `recall` over a
  `cell64` set instead of a single cell.
- `compare` answers *how does this place differ from that one* — the
  minimal binary form of inter-place reasoning.
- `find_similar` answers *what else looks like this* — the inverse of
  `compare` over an embedding band.
- `diff` answers *what changed here between these times* — the temporal
  analogue of `compare`.
- `trajectory` answers *what was the path through these places at these
  times* — the joint product of the spatial and temporal axes.
- `verify` answers *is this receipt still authentic* — the closure
  operator that makes the other six self-checking.

Adding an eighth primitive would either duplicate one of these under a
different name, or violate constraint 2 by returning a result that the
client cannot independently re-derive from the receipt and its
manifests. Removing one would force agents to fake it client-side, which
defeats the cite-ability guarantee because the synthesized step would
not be signed.

### Why content-addressed manifests, not a versioned API

A dataset is a moving target — Hansen ships a new GFC year, Copernicus
re-tiles, OSM rolls forward continuously. If the protocol's contract is
"v1 returns elevation" the contract drifts every time the upstream
changes. emem instead binds each receipt to four manifest CIDs
(`schema_cid`, `bands_cid`, `sources_cid`, `functions_cid`). Two answers
agree if and only if their manifests agree. This makes "what does this
mean" a hash comparison, not an English-language SLA.

### Why three storage tiers and not one

Cost-of-access varies by five orders of magnitude across (RAM hot →
local SSD warm → remote object store cold). A single tier optimizes
for one regime and pays for it in the others. The 0.0.x reference
build ships only the hot tier (sled, content-addressed,
sub-millisecond reads of the working set); a warm parquet tier for
month-scale `tslot` sweeps and a cold content-addressed tier for
inter-responder durability are described in the design but are not
yet wired. The tiers are not features — they are what falls out of
treating storage as a function of access pattern.

### What this implies for the agent

An agent that internalizes these five constraints will use emem
correctly without reading the SDK: it will discover via content-hashed
manifests, scope time and space independently, keep receipts as
provenance, and refuse to fabricate a primitive that the protocol does
not provide. The shape of the API is the shape of the problem.

---

## 13. References

Specifications and operating documents:

- `docs/SPEC.md` — wire-stable protocol specification.
- `docs/AGENTS.md` — paste-ready MCP / OpenAPI configs for every supporting host.
- `docs/TEMPORAL.md` — derivation of the five `/v1/temporal_route` decay kernels.
- `docs/PHYSICS_ENDPOINTS_2026_05_04.md` — live request/response samples and known-limitations matrix for `/v1/heat_solve`, `/v1/wave_solve`, `/v1/jepa_predict`.
- `docs/EVAL_2026_05_04.md` — 51-question agent evaluation across 9 domains; per-domain pass/fail and routing accuracy.

Topic + algorithm registries (canonical, content-addressed):

- `crates/emem-core/data/topics-v0.json` — 25-topic registry with descriptions, aliases, and per-topic algorithm bindings.
- `crates/emem-core/data/bands-v0.json` — 1792-D voxel layout; band offsets and tempo classes.
- `crates/emem-core/data/algorithms-v0.json` — algorithm manifest with formulas, citations, and CFL bounds.

Workspace crates:

- `crates/emem-core` — type identities, manifest loaders, topic + algorithm registries.
- `crates/emem-codec` — agent-native string codecs (cell64, tslot, vec64, cid64).
- `crates/emem-fact` — fact / attestation / receipt types.
- `crates/emem-attest` — attestation envelopes and Merkle batching.
- `crates/emem-claim` — claim structures for the `/v1/verify` primitive.
- `crates/emem-cache`, `crates/emem-storage` — hot cache + materializer + log.
- `crates/emem-cubes` — 1792-D voxel cube layout and band-cube indexing.
- `crates/emem-primitives` — read primitives over `&Server`.
- `crates/emem-fetch` — anonymous HTTPS / GCS / vsicurl connectors.
- `crates/emem-intent` — intent-routed planner backing the `emem_intent` tool.
- `crates/emem-api-rest` — HTTP surface (REST + MCP + OpenAPI), topic router, physics primitives.
- `crates/emem-mcp` — MCP transport adapter (JSON-RPC 2.0 streamable HTTP).
- `crates/emem-cli` — `emem` (introspection) and `emem-server` (HTTP) binaries.
