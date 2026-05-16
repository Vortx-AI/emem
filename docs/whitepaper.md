# emem: a content-addressed protocol for verifiable Earth observation

**Version 0.0.6 / 2026-05-14**

---

## Abstract

emem is a protocol for AI agents and analysts that need a stable,
citation-carrying place to ground spatial answers. Three primitives —
`locate`, `recall`, `find_similar` — operate over an open-data corpus
addressed by `(cell, band, tslot)`. Every response carries an Ed25519
receipt over the canonical CBOR of the cited facts. A downstream
verifier reproduces the canonical preimage from the receipt fields
and checks the signature without trusting the issuer.

The protocol defines the loader, the validator, the CID rule, and the
primitive semantics. It is never the data itself. Any conforming
implementation must produce byte-identical CIDs from byte-identical
inputs; the conformance set is a content-addressed manifest pinning
bands, algorithms, sources, schema, and the function registry. The
reference responder is a single Rust binary at
`github.com/Vortx-AI/emem`, running at `https://emem.dev`.

Three foundation encoders sit GPU-pinned inside the same tenant as
the responder — Clay v1.5 (1024-D), Prithvi-EO-2.0-300M-TL (1024-D),
and Tessera (128-D annual stack via `geotessera.multi_year`). Their
receptive fields are independent (Clay ~2.56 km, Prithvi ~6.7 km,
Tessera per-pixel), and the protocol surfaces a triple-consensus
change algorithm that votes across all three. Consensus across the
three is strong signal; one-or-none flags receptive-field aliasing
rather than land-surface change.

This document specifies the math and architecture that 0.0.6 ships.
Items not in 0.0.6 are listed under "Honest limits" and not discussed
elsewhere.

![emem architecture — one Rust binary, two wire surfaces, one optional sidecar](/docs/diagrams/01-architecture.svg)
*Figure 1 — the entire stack. REST and MCP share handlers; sled holds the hot cache, the append-only Merkle log holds the trust state. Open the SVG for the labelled variant.*

---

## 1. Motivation

LLM agents asked "what is at this place" sample fragmented, undated,
unattributed scrapes. Two failure modes recur: conflicting answers
across runs (no canonical address for "the patch of land at
lat=12.97°, lng=77.59° on 2024-09-01"), and no way to cite (the
underlying tile, timestamp, and algorithm get smeared together).

The wider context is memory. Long-term assistants and agent systems
need to accumulate, update, and reuse historical information across
sessions; current practice splits along three lines. Textual stores
inject prior history through the context window. Parametric stores
fold knowledge into adapter or prefix weights. Outside-channel stores
keep state in a separate module reached by retrieval. Each of these
operates over a *single agent's* history, scoped to a conversation
or a tenant. emem occupies a different layer: a shared, cross-session,
cross-tenant working memory of *Earth itself*, where the addresses
are places rather than token positions, the state is persistent
rather than per-conversation, and the bytes are reproducible across
replicas rather than fuzzily retrieved. Section 19.2 places this
layer alongside the in-agent memory patterns and addresses the
standard objections to outside-channel state.

emem's response to the citation failure is a small set of address
rules plus one signing rule. Every fact is keyed by
`(cell64, band, tslot)`. Every fact's CID is
`base32_nopad_lower(blake3(canonical_cbor)[..16])`. Every response
carries an Ed25519 receipt over a deterministic preimage naming the
cited CIDs. An offline verifier with the responder's pubkey
reproduces the preimage and checks the signature.

The surface stays small on purpose: three core primitives, one
verify call, seven derived primitives (`compare`, `compare_bands`,
`diff`, `trajectory`, `query_region`, `recall_polygon`,
`field_boundaries`). New bands, algorithms, and sources extend the
registries without changing the primitive surface.

---

## 2. Spatial primitive: cell64

A cell64 is a 64-bit packed identifier for a square lat/lng bucket on
WGS-84. The encoding lives in `crates/emem-codec/src/geo.rs`.

![cell + band + tslot → canonical CBOR → blake3 → 26-character base32 CID](/docs/diagrams/09-address-algebra.svg)
*Figure 2 — address algebra. Three integers become one 26-character handle the rest of the protocol cites.*

### 2.1 Bit layout

```text
  bit:  63    60 59      52 51      44 43            22 21            0
        +-------+-----------+-----------+----------------+----------------+
        | mode  | resolution|   base    |     lat_q      |     lng_q      |
        | 0001  |    21     |   0xab    |    21 bits     |    22 bits     |
        +-------+-----------+-----------+----------------+----------------+
            4         8           8            21              22
```

- `mode = 0b0001` marks this as a geo cell.
- `resolution = 21` distinguishes the active 10 m grid from the
  pre-0.0.3 305 m grid (`resolution = 12`). A legacy cell64 fails
  decoding with `NotGeoCell` rather than silently misplacing facts.
- `base = 0xab` is the geo aperture marker.
- `lat_q ∈ [0, 2²¹)` is the lat axis bucket.
- `lng_q ∈ [0, 2²²)` is the lng axis bucket.

### 2.2 Encoder

```text
  lat = clamp(lat_deg, -90, +90)
  lng = ((lng_deg + 180) mod 360) - 180
  lat_q = round((lat + 90) / 180 · (2²¹ − 1))
  lng_q = round((lng + 180) / 360 · (2²² − 1))
  cell64.raw = (1 << 60) | (21 << 52) | (0xab << 44) | (lat_q << 22) | lng_q
```

The asymmetric bit count (21 lat × 22 lng) keeps cells square at the
equator: `Δlat ≈ Δlng ≈ 8.583e-5°`, equator extent ~9.55 m. Poleward
the lng pixel narrows by cos(lat); cells become taller than wide.
The eventual migration target is an H3-style hex DGGS at resolution
13 (~3.4 m equal-area cells); cell64 is the active grid in 0.0.6.

### 2.3 Why a square 10 m grid

Sentinel-2 and Sentinel-1 RTC native pitch is 10 m. Cop-DEM 30 m
mean-pools cleanly to 10 m. A 10 m grid lets a fact be materialised
per pixel without aggregation loss. A coarser grid forces every
optical ingest to pre-aggregate or pre-resample — an opinion the
protocol does not need to take.

### 2.4 Text form

A cell64 renders as four CVCV bigrams from a 65 536-entry alphabet
(21 consonants × 10 vowels × 21 × 10 = 44 100 natural pairs, padded
with `z<hex4>` synthetic suffixes), separated by dots:

```text
  damO.zb000.xUti.zde78
```

The alphabet is Hilbert-ordered (`tools/measure_alphabet.py`), so
adjacent codepoints tend to map to nearby cells. For exact spatial
neighbourhoods, agents call `/v1/locate.neighborhood_cells` rather
than relying on string prefixes.

---

## 3. Temporal primitive: tslot

A tslot is a `u64` bucket of the Unix timeline at a band's tempo
cadence. The encoding lives in `crates/emem-core/src/tslot.rs`.

### 3.1 Anchor

```text
  tslot = floor(unix_seconds / tempo.slot_seconds())
```

The anchor is the Unix epoch (1970-01-01T00:00:00Z). Pre-1970
timestamps clamp to `tslot(0)`.

### 3.2 Tempo classes

```text
  Tempo       slot_seconds   typical bands
  ----------  -------------  --------------------------------------
  Static      0              copdem30m, gmrt, koppen
  Slow        31_536_000     geotessera.{2017..2024}, multi_year,
                             bin128, soilgrids
  Medium      2_592_000      ndvi_monthly, modis composites
  Fast        86_400         s2_raw, s1_raw, modis lst_day_8day
  UltraFast   3_600          weather, air_quality, traffic
```

Tempo is declared once per band in `bands-v0.json`. A band cannot be
served at a tempo finer than its declared cadence.

### 3.3 Why tempo

Bands have natural cadences — Tessera annual, MODIS 8-day, Open-Meteo
hourly. Snapping to tempo aligns the index across heterogeneous
sources. "Compare NDVI now versus a year ago" maps to two specific
tslot values without the responder reasoning about source-specific
cadences at query time.

### 3.4 Text form and recovery

Text form: `t.<base32-nopad-leb128>` — `t.aaaaagy` is the tslot
literal for the unsigned integer `1234`. Round-trippable through
`tslot_text.rs` in `emem-codec`. A receipt that cites
`(cell, band, tslot)` lets a verifier recover the wall-clock window:
multiply tslot by `tempo.slot_seconds()` (read from `bands-v0.json`)
to recover the Unix start.

---

## 4. Content addressing

### 4.1 Canonical CBOR

emem-CBOR is RFC 8949 deterministic encoding plus four mandatory tags.
It uses `ciborium` with serde-derived structs; field declaration order
in the struct decides serialisation order. Two implementations that
share the struct definition produce byte-identical CBOR for the same
fact.

```text
  Tag 65000  emem cell        (u64 packed per §2.1)
  Tag 65001  emem tslot       (u64)
  Tag 65002  emem vec64-CID   (32 bytes)
  Tag 42     IPLD CID         (multibase 'b' base32, RFC 9090)
```

Free-form maps must arrive with pre-sorted keys.

### 4.2 BLAKE3 + base32-nopad-lowercase

```text
  FactCid = base32_nopad_lower( blake3( canonical_cbor(fact) )[..16] )
          → 26 lowercase characters

  cid64   = base32_nopad_lower( blake3( ... )[..8] )
          → 13 lowercase characters
```

`FactCid` is the durable form referenced in attestations and receipts.
`cid64` is the visible short form for logs and inline text; it is a
prefix, not a separate hash, and its collision domain is 2⁶⁴.

Manifest CIDs use 32-byte BLAKE3 prefixes
(`base32_nopad_lower(...)[..32]`), giving longer strings appropriate
for content-addressing the band ontology, algorithm registry, source
catalog, and CDDL schema bundle.

### 4.3 Choice rationale

BLAKE3 ships a `derive_key` API used by the binary embedding rotation
(§9.3.1). base32-nopad-lowercase is URL-safe, case-insensitive,
padding-free, and has no slash collisions inside path segments.
128-bit truncation at the FactCid level: a birthday collision needs
~2⁶⁴ facts; the canonical responder is at ~10⁵ today.

---

## 5. Trust: receipts, attestations, in-browser verification

![receipt preimage → ed25519 signature → merkle path → offline verify](/docs/diagrams/10-trust-plane.svg)
*Figure 3 — the trust plane. Five steps to accept a fact without ever calling the issuer back. The `/verify` page recomputes these in the browser using `@noble/curves`.*

### 5.1 Receipt anatomy

```text
  field                   meaning
  ----------------------  ----------------------------------------------
  request_id              ULID; sortable + unique per call
  served_at               ISO 8601 UTC
  primitive               "emem.recall" | "emem.find_similar" | ...
  intent                  optional natural-language hint
  cells                   list of cell64 strings the call touched
  fact_cids               list of FactCid the response cited
  schema_cid              CID of the CDDL bundle used
  merkle_proof            optional inclusion proof for fact_cids[0]
  responder               32-byte ed25519 pubkey
  responder_key_epoch     u32; bumps when the operator rotates keys
  signature               64 bytes
  source_versions         per-source freshness map
  registry_cid            CID of the function registry version
  cost                    {credits, latency_p50_ms, latency_p99_ms,
                           source_freshness_s, was_cached}
```

### 5.2 Signature preimage

The preimage is deterministic in field order; fields join with the
literal `|` byte, list elements with the literal `,` byte. The
implementation lives in `crates/emem-storage/src/server.rs:119-189`.

```text
  preimage_hash = blake3(
      request_id  ||  "|" ||
      served_at   ||  "|" ||
      primitive   ||  "|" ||
      cells[0] || "," || cells[1] || "," || ... || "|" ||
      fact_cids[0] || "," || fact_cids[1] || "," || ...
  )
  signature = ed25519_sign(signing_key, preimage_hash)
```

Both empty `cells` and empty `fact_cids` lists emit their trailing
field separator, so a verifier reproduces the exact byte string from
the receipt fields without ambiguity.

Verification uses `verify_strict` on `ed25519_dalek::VerifyingKey`.
The strict variant rejects malleable signatures.

### 5.3 In-browser receipt verification

`/verify` and `/verify/<fact_cid>` serve a single HTML page
(`web/verify.html`) that recomputes the canonical preimage and runs
ed25519 signature verification entirely in the browser.

```text
  noble-curves@1.x   ed25519 verify_strict-equivalent
  noble-hashes@1.x   blake3, sha-256
  module loader      esm.sh (sub-resource CSP'd)
  fallback           POST /v1/verify_receipt (server-side) when noble
                     bundles are blocked; the page labels itself as
                     "server-assisted" in that case.
```

Idle `/verify` is a landing page; `/verify/<fact_cid>` auto-fetches
the cited fact and its enclosing receipt and runs the math. The page
reads `location.pathname` to decide which mode it is in.
`/verify?receipt=<base64>` is supported for direct paste of a receipt
the agent already holds.

The pubkey the page checks against is read from
`/.well-known/emem.json`. A verifier that holds an older pubkey
detects rotation through `responder_key_epoch`.

### 5.4 Attestation envelope

A primary fact reaches the index through an Attestation, which the
storage layer re-checks before persisting.

```text
  Attestation {
      facts: Vec<Fact>,
      batch_root: [u8; 32],          // merkle_root over sorted leaves
      attester: AttesterKey,         // 32-byte ed25519 pubkey
      attester_key_epoch: u32,
      registry_cid: RegistryCid,
      schema_cid: SchemaCid,
      signature: Signature,
      attested_at: i64,              // Unix seconds
  }

  preimage = blake3(
      batch_root  ||
      registry_cid_bytes  ||
      schema_cid_bytes
  )
```

`verify_attestation` re-CBOR-encodes every fact, hashes each to a
32-byte leaf, sorts bytewise, folds via `merkle_root`, compares to
`batch_root`, re-hashes `(batch_root || registry_cid || schema_cid)`,
and `verify_strict`s the signature. Failure raises
`StorageError::AttestationInvalid`. No bypass paths.

### 5.5 Merkle math and inclusion proofs

Every leaf is self-hashed once before folding
(`blake3(leaf || leaf)`), giving domain separation between leaves and
inner nodes. Folding pairs left-then-right; odd-cardinality layers
pair the last element with itself.

```text
  layer 0:    L0=h(l0||l0)  L1=h(l1||l1)  L2=h(l2||l2)  L3=h(l3||l3)
  layer 1:    P0=h(L0||L1)  P1=h(L2||L3)
  layer 2:    root=h(P0||P1)
```

The inclusion path for leaf 1 is `[L0, P1]`. `verify_merkle_path`
walks the path bottom-up: when `leaf_index` is even, hash
`acc || sibling`; when odd, hash `sibling || acc`; halve per step;
compare `acc == root`.

`merkle_root_and_paths(leaves)` returns `(root, Vec<path>)` in one
pass. `MaterializingStorage::put_attestation` persists per-fact
`MerkleProof` records to a sled tree `emem.fact_proofs`, keyed by
FactCid. `Server::sign_receipt` populates `Receipt.merkle_proof`
from the first cited fact's stored proof; a verifier re-derives
every other CID from the signed preimage. Receipts that pre-date
the proof tree carry `merkle_proof = None`; the signature still
binds the CIDs.

### 5.6 Append-only Merkle log

Every accepted Attestation appends to a per-segment file under
`var/emem/log/merkle.log.{0,1,...}`.

```text
  per-record:    [u32 LE record_len] [CBOR(attestation)]
                 [32 byte blake3(CBOR)]
  per-segment:   [32 byte segment_hash = blake3(all_records)]
```

`append()` calls `fsync_all()` before returning. Segments rotate at
1 GiB. `verify()` re-hashes every sealed segment and reports
mismatches; the current segment is verified up to the last
fully-written record.

### 5.7 Identity

A 32-byte Ed25519 secret stored at `var/emem/identity.secret.b32` in
base32-nopad-lowercase, mode 0600. Load order: `EMEM_SECRET_B32` (env)
> file > fresh keypair. `EMEM_DATA=:memory:` produces an ephemeral
key. The `u32` key epoch bumps on rotation.

---

## 6. Signed Absence

"We don't have this here" is a citable receipt, not a 404. A
NegativeFact carries the same `(cell, band, tslot)` key as a
PrimaryFact would, plus a `ReasonCid` pointing at the upstream
evidence that confirmed the absence. Typed reasons in 0.0.6:

| reason                       | meaning                                            |
|------------------------------|----------------------------------------------------|
| `unavailable_capability`     | responder does not implement the requested function |
| `outside_coverage`           | upstream product's documented coverage excludes the cell |
| `gpu_unavailable`            | sidecar `cuda.available=false` for a GPU-tier algorithm |
| `archetype_seed_unavailable` | type-locality centroid table missing for a classifier |
| `materialize_timeout`        | connector exceeded the per-fact budget             |
| `materialize_miss`           | no connector registered for this band              |
| `over_water` / `over_land`   | DEM vs bathymetry boundary refusal                 |

Every NegativeFact is signed and content-addressed. An agent asking
the same question twice gets the same absence CID and skips the
upstream call.

---

## 7. Bands — the 1792-D voxel

The band ontology loads from `bands-v0.json`. **Thirty-five band
cube slots** sum to exactly **1792 dims**. Offsets are contiguous;
reserved slots leave room for new bands without breaking existing
offsets. **118 materializer-wired band names** answer recall today —
the gap between 35 cube slots and 118 names is the parametric
expansion (every Sentinel-2 reflectance band, every spectral index,
every Tessera vintage, every Open-Meteo variant) under a fixed
underlying slot.

```text
  offset  dims  key                family       tempo   privacy
  ------  ----  -----------------  -----------  ------  --------
       0   128  geotessera         vision       slow    public
     128    64  overture           human        slow    public
     192     7  air_quality        climate      ultra   public
     199   505  _reserved_512      reserved     static  public
     704    10  sentinel2_raw      optical      fast    public
     ...   ...  ...                ...          ...     ...
                                                        (35 slots)
                                                        total = 1792
```

The 14-family enum (Foundation, Optical, Radar, Terrain, Climate,
Soil, Vegetation, Landcover, Water, Human, Vision, Topology,
Encoding, Reserved) routes display behaviour and documentation but
is not load-bearing for the CID rule.

Privacy classes (see §17 for usage):

```text
  Public                          unrestricted at any resolution
  AggregateOnly { min_res }       snap to coarser res; receipt
                                  carries privacy_snapped: true
  L2OnlyWithModelCid              admissible only at conformance L2
  Prohibited                      conforming responders MUST refuse
```

Manifest CID:
`bands_cid = base32_nopad_lower(blake3(canonical_cbor(BandsManifest))[..32])`,
exposed at `/v1/manifests` and on the `/v1/bands` response root.

---

## 8. Algorithms

The algorithm registry (`algorithms-v0.json`) holds **155 entries** in
three kinds:

```text
  solo        single input band → derived value
              (e.g. NDVI → vegetation_class)

  combined    multi-band composite score / classification
              (e.g. flood_risk@2 = 0.55·(swr/100)
                                 + 0.25·dem_agreement·(relu(50-cop)/50)
                                 + 0.20·sigmoid((-15-s1)/2))

  embedding   operates on a foundation embedding vector
              (cosine similarity, novelty, neighborhood consistency)
```

Each entry carries `inputs[]`, `formula` (plain math), `output`,
`when_to_use`, `citation`, an optional `evaluation: Expr` AST, and the
provenance trio added in 0.0.4:

- `parameters` — typed tunable thresholds (`consensus_threshold`,
  `k_neighbors`, `ask_timeout_ms`, `intent_cosine_threshold`, ...).
  Values are `serde_json::Value`; numerics resolve through
  `Algorithm::param_f64(key)`. `param_str` and a raw `param` accessor
  cover the other shapes.
- `learned_from` — citation provenance for every tuned number. Every
  gate threshold traces to its referee.
- `prerequisites` — registries, centroid tables, or seed datasets the
  algorithm depends on. Lets the dispatcher emit
  `archetype_seed_unavailable` Absence rather than a runtime crash.

### 8.1 The Expr AST

`Expr` is 15 variants for in-process deterministic evaluation:

```text
  Band(key)               Const(f64)
  Add(l, r)               Sub(l, r)
  Mul(l, r)               Div(l, r)
  Linear { weights[], bias }      Σ wᵢ·sampleᵢ + bias
  Clamp { lo, hi, x }
  Where { cond, lhs, op, rhs, then, else }
  WeightedBlend { entries[(weight, expr)] }
  Abs(x)        Sigmoid(x)        Relu(x)        Max(l, r)    Min(l, r)
```

`Expr::evaluate(samples) -> Option<f64>` runs the AST against a recall
snapshot; `Expr::referenced_bands()` enumerates required inputs. The
dispatcher walks the registry, runs every algorithm whose evaluation
block has all inputs present, and emits an `algorithm_outcomes[]`
array on `/v1/ask`.

### 8.2 Triple-encoder consensus

The differentiator of 0.0.6 is `clay_prithvi_tessera_triple_consensus@1`
plus six domain variants. Three foundation encoders with independent
receptive fields vote on a per-cell change index over a 365-day
window.

```text
  encoder      receptive field    output    role in the vote
  -----------  -----------------  --------  ----------------------------
  Clay v1.5    ~2.56 km           1024-D    chip-scale optical context
  Prithvi-EO   ~6.7 km            1024-D    multi-temporal HLS V2
  Tessera      per-pixel          128-D     pixel-scale annual stack
```

The base algorithm formula:

```text
  dc = clamp(1 - cos(clay_now, clay_year_ago), 0, 1)
  dp = clamp(1 - cos(prithvi_now, prithvi_year_ago), 0, 1)
  dt = clamp(1 - cos(tessera_latest, tessera_prev), 0, 1)
  ensemble = sqrt((dc² + dp² + dt²) / 3)
  agreement =
    if  dc > gate ∧ dp > gate ∧ dt > gate  then 'all_three'
    elif (dc > gate) + (dp > gate) + (dt > gate) ≥ 2  then 'two_of_three'
    else 'one_or_none'
```

`gate` is `parameters.consensus_threshold` (default 0.15, learned from
Healey et al. 2018, RSE 204:717-728, LandTrendr ensemble convention).

**Why independent receptive fields matter.** A single encoder
generates spurious change scores when chip boundaries intersect a
real edge. Clay's 2.56 km RF aliases differently from Prithvi's
6.7 km RF, and Tessera operates pixel-by-pixel. A cell where all
three encoders agree shifted is land-surface change; a cell where
only one encoder fires is almost certainly receptive-field artifact.

### 8.3 Six domain variants

```text
  algorithm key                    gate   uplift / extra leg
  -------------------------------  -----  ----------------------------------------
  deforestation_triple@1           0.20   Hansen GFC lossyear mask elevates 2+
                                          votes to 'hansen_confirmed'.
                                          EUDR pre-screen.
  wetland_change_triple@1          0.10   JRC GSW recurrence delta replaces the
                                          Tessera leg; abs gate 15 occurrence pts.
  urban_expansion_triple@1         0.20   Overture buildings.count delta + s2.B11
                                          SWIR corroboration tag 'swir_corroborated'
                                          on 1-vote cells.
  disaster_anomaly_triple@1        —      Spatial (no temporal recipe).
                                          2-σ neighbour z-score; single-pass
                                          discovery.
  climate_archetype_triple@1       —      12-class Köppen-Geiger classifier seeded
                                          from Beck et al. 2018 type-locality
                                          centroids (`climate_archetype_centroids_v1.json`).
  coastal_erosion_triple@1         0.12   Bathymetry-clamped to cells where
                                          gmrt.topobathy_mean ∈ [-5, +5] m.
```

Every gate threshold has a `_threshold_learned_from` block citing the
paper it was estimated against. Re-tune happens at registry-CID time
through the parameters block, not by recompiling.

### 8.4 Sensor tier rule

An algorithm that claims delivery resolution ≤ 10 m must have at
least one S1, S2, or Landsat input in `variance_sources`. Coarser
inputs (POWER, ERA5, SoilGrids, Hansen, WorldCover, JRC GSW,
Cop-DEM, GMRT) are baseline / context, never the sole variance
source for a fine-resolution claim. Order:
`S1 > S2 > Landsat > IoT > OtherSat > Static`; an algorithm whose
declared anchor is below a higher-tier input fails to load.

### 8.5 Worked example: flood_risk@2

```text
  inputs:   surface_water.recurrence, copdem30m.elevation_mean,
            gmrt.topobathy_mean, sentinel1_raw.vv
  formula:  0.55·(swr/100)
          + 0.25·dem_agreement·(relu(50-cop)/50)
          + 0.20·sigmoid((-15-s1)/2)
  output:   [0.0, 1.0] flood-risk score
  citation: Pekel 2016 (JRC GSW), Schumann 2018 (SAR flood)
```

The Expr round-trips through canonical-CBOR JSON and produces
`0.4836` byte-stably (test
`flood_risk_v2_evaluates_to_a_real_number_from_dispatcher`).

---

## 9. Primitives

Every primitive returns a signed receipt. Empty results are labelled,
not zeroed.

### 9.1 recall(cell, bands?, tslot?)

Index lookup over `(cell, band, tslot)`. Implementation in
`emem-primitives/src/recall.rs`.

When `bands` is supplied and matches no facts, the response includes
`bands_already_attested_at_cell: [...]` — the actual band keys present
on the cell. An agent asking for `band="alphaearth"` at a cell that
holds geotessera + soilgrids learns immediately that its band name is
wrong, not that the cell is empty.

### 9.2 Auto-materialize on miss

When the entire cell is empty for the requested band and a connector
is registered, the recall path triggers materialisation:

```text
  recall(cell, band, tslot) → miss
    → function-registry lookup (fn_key)
    → connector dispatch
    → upstream Range read (vsicurl COG, STAC, JSON API)
    → compute fact value
    → Fact::Primary → sign as responder → put_attestation → return
```

Gates: `EMEM_AUTO_MATERIALIZE` (default **on** — set `0`/`false` to
disable), 30 s materialiser timeout, 180 s gateway timeout, 16 MiB
body cap. A miss with no registered connector returns
`MaterializeMiss` as a typed Absence, never a silent empty.

Any cell on Earth answers without pre-seeding. The responder signs
the materialised fact as itself; trust delegation is "the same key
that signs receipts also signs the value, with `derivation.fn_key`
declaring exactly how it was produced." **20 live materializer
registrations** cover the wired band set.

### 9.3 find_similar(key, k?, band?, filter?, mode)

Brute-force k-NN over the canonical-key index for the configured band
(default `geotessera`).

```text
  Mode::Cosine             fp32 cosine over the full vector
  Mode::Hamming            popcount over the binary sibling band
  Mode::HammingThenRerank  adaptive triage + cosine rerank
```

Per-cell deduplication keeps the highest-scoring vintage; without it
multi-vintage bands return k near-duplicates of the same place.

The optional `filter: Claim` is evaluated per cell with memoisation —
a verdict for `(cell, claim.band, claim.op, claim.value)` computes
once and reuses across repeated tslots. Cells with no fact for the
filter band are dropped (undecidable, not "false").

`requested_k` vs `returned_k` are both surfaced. When
`returned_k < requested_k` after dedup, the corpus has fewer distinct
cells than the caller asked for; the responder returns what it has
rather than padding.

#### 9.3.1 TurboQuant binary rotation

`Mode::Hamming` operates over the binary sibling band
(`geotessera.bin128`); encoder in `binary_embedding.rs`.

```text
  ROT_SEED_TEXT = "emem.binary_embedding.turboquant.v1"
  BIN_DIMS = 128, BIN_BYTES = 16

  ROTATION (built once, cached):
    seed CSPRNG with blake3(ROT_SEED_TEXT) →
    128² Gaussian samples (Box-Muller) →
    classical Gram-Schmidt → orthonormal 128×128 matrix

  pack_bin128(vec):
    rotated[i] = Σⱼ ROTATION[i][j] · vec[j]
    bit i      = (rotated[i] >= 0)            # MSB-first per byte
```

Hamming distance is XOR + popcount, roughly 10⁹ scored pairs/sec per
x86 core. The rotation redistributes upstream variance across all
128 dims so a single bit per dim carries information.
Hamming-to-cosine bridge: `score = 1 − 2·dist/128 ∈ [−1, +1]`. The
matrix's content address is `rotation_cid()`; a verifier rebuilds
the matrix from the seed text and re-packs the source vector to
byte-compare.

#### 9.3.2 Inline auto-derive

When the binary sibling band is absent at a cell but the cosine band
is present, the find_similar path now inline-derives `bin128` from
cosine via the TurboQuant rotation rather than returning
`CidNotFound`. The derivation seed is the same `ROT_SEED_TEXT`, so
the result is byte-identical to a cached `bin128` at the same cell.

#### 9.3.3 HammingThenRerank adaptive oversampling

The triage path observes `|hamming_top_k ∩ cosine_top_k| / k` over an
EWMA (decay α = 0.05) backed by lock-free `AtomicU64` storage. After
~50 calls warm the gate, the oversampling factor adapts to corpus
binary↔cosine agreement instead of staying nailed to 4×. Cold-start
keeps the historical 4× multiplier so the first 50 calls match the
pre-EWMA behaviour byte-for-byte.

### 9.4 verify(claim, cell, mode)

A `Claim` is `{ band, op, value, tslot? | window? }` where `op` is
one of `eq`, `ne`, `lt`, `le`, `gt`, `ge`. `verify` evaluates the
claim against the index.

```text
  Mode::Fast      look up canonical fact_cid; agree/disagree+evidence;
                  no inference
  Mode::Resolve   when the band has no fact at the targeted tslot,
                  call storage.materialize_many(...) and re-scan
```

A `MaterializeMiss` (no upstream connector) surfaces to the caller
rather than collapsing to `verdict=false`. Open-ended windows (no
tslot, no single-point window) cannot pick a target tslot, so they
fall back to Fast over whatever is already in the index.

`Mode::Zk` was removed in 0.0.4 — Rust enum, MCP schema, OpenAPI
VerifyReq schema. It returned 500 on every call. ZK is not in 0.0.6.

### 9.5 compare / compare_bands

Two Primary facts side by side, with a structured difference object.
`compare_bands(cell, a, b, tslot_a?, tslot_b?)` resolves omitted
tslots to the latest tslot for that band at the cell. A caller who
omits both tslots gets `tslot_resolution.reason = "auto_picked_latest"`;
a caller who supplies tslots gets `"caller_supplied"`. A band with no
history at the cell surfaces as `bands_with_no_history[]` and the
response carries an empty-cite receipt — labelled empty, not zeroed.

### 9.6 diff / trajectory / query_region / recall_polygon

- `diff(cell, band, t0, t1)` — change between two tslots for a
  single band. Non-numeric bands return a structured error.
- `trajectory(cell, band, [tslots])` — ordered series; missing
  tslots surface as gaps with explicit reasons.
- `query_region(geometry, bands?, agg?)` — geometry is `<cell64>`,
  `cells:c1,c2,...`, or `bbox:lon_min,lat_min,lon_max,lat_max`.
  Bbox synthesis caps at `MAX_BBOX_CELLS = 4096` and
  `MAX_REGION_FACTS = 65 536`. Default `max_cells` is
  bbox-area-derived (target 1 cell per (10 km)², clamped `[64, 1024]`).
  Beyond the caps the responder aggregates over what it has;
  `receipt.fact_cids` reflects exactly what contributed.
- `recall_polygon(polygon_bbox, n_cells)` — fans out across up to
  1024 sample cells; returns mean / median / min / max / std per
  band plus per-cell `scene_thumbs[]`, `scene_overlay_url`,
  `geojson`. An `include: ["ftw_fields"]` flag attaches the
  field-boundary block from §9.7 inline.

### 9.7 field_boundaries

Per-field agricultural polygons from Fields of The World (FTW), a
global product of ~3.17 billion field polygons across 241 countries
at 10 m, CC-BY-4.0. The connector reads the upstream PMTiles archive
(2.14 TB, hosted on source.coop) over anonymous HTTP range requests;
MVT tiles decode and reproject from Web-Mercator to WGS-84 in-process.
Auto-zoom shrinks the request when a bounding box exceeds the 16-tile
cap.

```text
  POST /v1/field_boundaries
  body  { place: "Patiala, India", zoom?: u8 }
        | { polygon_bbox: [w,s,e,n], zoom?: u8 }

  response {
      count: u32,
      total_area_m2: f64,
      zoom_used: u8,
      geojson: FeatureCollection,
      source_cid: FactCid,
      provider_url, license, attribution
  }
```

The place-name path reuses the locate cascade (§11): GeoNames
cities-5000 → Overture divisions → Photon → Nominatim, with polygon
enrichment from Overture's `divisions/division_area`.

---

## 10. /v1/ask: foundation-embedding fan-out

`/v1/ask` carries a `foundation_embeddings` envelope when the
question matches Similarity or Change intent. A keyword classifier
(`ask_foundation::classify_intent`) pre-screens; matched questions
fan out across `clay_v1` + `prithvi_eo2` + `geotessera` concurrently
via `tokio::join!`.

| intent     | trigger phrases                                        | fan-out                                                                 |
|------------|--------------------------------------------------------|-------------------------------------------------------------------------|
| Similarity | "find places like", "similar to", "looks like", "analog of" | find_similar k-NN against each encoder; report per-encoder hit counts and whether the triple corroborated |
| Change     | "what changed", "year over year", "deforestation", "urban expansion", "anomaly" | recall of `clay_v1` + `prithvi_eo2` + `geotessera.multi_year` so the agent or in-process AST evaluator computes the triple-consensus index |

Budget is `ask_timeout_ms` (default 4000), read from the
`clay_prithvi_tessera_triple_consensus@1` algorithm's parameters
block. On timeout the envelope carries
`degraded_reason: "foundation_embedding_timeout"`; the
topic-router-driven `ask_inner` path still ships a useful answer.
The receipt remains the one signed by the standard recall path;
encoder fact CIDs merge into the same `fact_cids` list.

---

## 11. Locate cascade

`/v1/locate` walks a six-layer cascade so common queries never reach
a rate-limited upstream. The `via` field in the response names which
layer answered:

```text
  1. wide_bbox_lookup     embedded country / region polygons      ~5 ms
  2. embedded_gazetteer   in-binary place atlas                   ~5 ms
  3. geonames             cities-5000 (68 581 entries, CC-BY-4.0,
                          5.5 MB gzipped, include_bytes!)         ~10 ms
  4. sled cache           memoised result from prior resolve      ~2 ms
  5. photon               Komoot-hosted long tail                 ~80 ms
  6. nominatim            hard rate-limited fallback
```

Polygon enrichment in branches 2-4 reads Overture's
`divisions/division_area` theme (ODbL, anonymous S3 parquet with
row-group bbox pruning, exact-name match plus subtype rank
country > region > county > locality > borough > ... > microhood).
Mumbai, São Paulo, Tokyo, Patiala, Manhattan all resolve with
`polygon_bbox.source = overture_division_area`; the canonical
responder does not hit OSM for ~99% of city queries.

---

## 12. Inference

Four GPU-pinned encoders co-resident on a 20 GB VRAM budget. Latency
warm numbers measured on RTX 4090; cold numbers include weight load
from `<EMEM_DATA>/hf_cache/`.

```text
  model                    input shape          output  cold    warm    role
  -----------------------  -------------------  ------  ------  ------  ----------------------------
  Clay v1.5                [B, C, 256, 256]     1024-D  ~6 s    ~18 ms  pixel-scale wavelength-
                           (S1/S2/Landsat/                              conditioned ViT-L/8 MAE
                            NAIP/multi-sensor)                          + DINOv2 teacher
  Prithvi-EO-2.0-300M-TL   [B, 1, 224, 224, 6]  1024-D  ~10 s   ~20 ms  multi-temporal HLS V2 MAE,
                           (HLS V2 6-band)                              chip-scale receptive field
  Galileo (var. base)      [1, 1, 8, 8, 10]     D*      ~4 s    ~14 ms  S2-only modality; S1/ERA5/
                           (10 S2 bands @ 30 m)                         TC/VIIRS/SRTM/DW/WC/LandScan
                                                                        modalities zero-masked
  JEPA v2 (untrained)      3 × 128-D Tessera    128-D   —       ~50 µs  Dynamics predictor.
                           lags                                         Untrained baseline today;
                                                                        short-circuits to last
                                                                        attested vintage; receipt
                                                                        carries untrained_baseline
                                                                        warning.
```

`D*` for the Galileo row is variant-dependent: `EMEM_GALILEO_VARIANT`
defaults to `base` in the deployed responder; `tiny` and `nano` are
also selectable. The advertised capability becomes `galileo-<variant>`
in `/v1/capabilities.extensions[]` so an agent can read which dim
ships at request time.

All three trained encoders serve frozen embeddings; receipts carry
`frozen_pretrained_encoder`. Clay v1.5 loads from
`made-with-clay/Clay/v1.5/clay-v1.5.ckpt`. Galileo's non-S2 modalities
are zero-masked; the encoder accepts the multimodal shape but only S2
chips are wired today. The FastAPI shape — `POST /predict/<name>`
with `{cell, scene_url?, band_indices?}` request — is a public
contract; a customer drops in their own encoder under the same call.

JEPA v2 architecture: 3 × 128-D lags → flatten `[B, 384]` → 128-D
projection → 4 pre-LN residual blocks → zero-init head → `last_vintage
+ delta`. With delta starting at zero the baseline is identity. The
v2 handler short-circuits ONNX/sidecar inference when
`is_trained() == false` (which it is in 0.0.6) and returns
`last_input_vintage` directly, attaching `untrained_baseline` and
`upstream_geotessera_single_vintage` honesty warnings on the receipt.

`docs/developers/inference.md` carries the per-encoder chip-fetcher details,
sidecar protocol, VRAM partitioning, and the trained-checkpoint
loader contract.

### 12.1 Physics solvers

Three explicit-method solvers run in-process (no sidecar):
`/v1/heat_solve` (FTCS 2D, 3×3 MODIS `lst_day_8day` stencil, CFL
safety 0.20, horizon ≤ 168 h, Dirichlet boundary, default
α = 1e-6 m²/s per Oke 2017), `/v1/wave_solve` (CTCS 1D shallow water
along a seaward profile from a coastal cell; land-locked rejection
returns 422 with profile + suggestion), `/v1/jepa_predict`
(closed-form NDVI AR(2) seasonal with fixed α = 0.6, β = 0.3,
γ = 0.1). `/v1/jepa_predict_v2` is the sidecar Tessera-dynamics
route described above; its receipt carries `untrained_baseline`
until §12 changes.

---

## 13. Sources and connectors

`emem-fetch` ships **12 data connectors + 6 utility modules**.
**43 source schemes** are declared in `sources-v0.json`; the wired
subset answers recall today.

```text
  category       connectors
  -------------  ---------------------------------------------------------
  imagery        Sentinel-2 L2A, Sentinel-1 RTC, Landsat 8/9, MODIS
                 (MOD13/MOD11/MOD15A2H/MOD17A2H/MCD64A1)
  terrain        Cop-DEM 30 m, GMRT topobathy
  landcover      ESA WorldCover, Hansen GFC, Dynamic World*, FTW
  hydrology      JRC GSW occurrence + recurrence
  weather        NASA POWER, Open-Meteo (ERA5 + CAMS + Marine + Now)
  climatology    Köppen-Geiger, TerraClimate, CHIRPS
  soil           SoilGrids v2 (SOC, pH, clay, sand, BDOD, N · 0-30 cm)
  fire / nrt     FIRMS MODIS+VIIRS, VIIRS DNB*
  population     WorldPop, GHSL population + built-up, DMSP-OLS
  divisions      Overture (places, buildings, transportation,
                           division_area)
  gazetteer      GeoNames cities-5000 (68 581 places ≥ 5 000 pop)
  vector         FTW field polygons (~3.17 B fields), OSM Overpass
                 (WDPA fallback)
  embeddings     Tessera annual vintages (2017-2024 + multi_year + bin128)
  * declared, materialiser not wired in 0.0.6
```

Utility modules: `cog` (universal pure-Rust COG range sampler —
Deflate, LZW, Predictor 1/2/3, 8/16/32-bit LE), `cache_window`
(in-flight fetch coalescing), `connectors` (dispatcher), `proj`
(WGS84↔UTM), `stac` (Element84 + MPC search), `template` (URL
templating).

---

## 14. Topics

`topics-v0.json` declares **26 topics** routing free-text questions to
the right `(bands, algorithms)` pair. **11 topics are fully wired
live** — every declared band has a registered materialiser.
Routing is by cosine over a 768-D BAAI/bge-base-en-v1.5 embedding
served by `ort` 2.x + `tokenizers` directly (no third-party wrapper).
The model loads from `<EMEM_DATA>/models/bge-base-en-v1.5/` (CPU);
`EMEM_TOPIC_USE_GPU=1` switches to `CUDAExecutionProvider` when
`libonnxruntime.so` is GPU-enabled. Substring fallback over
`aliases[]` + `key` runs as a precision pre-pass so exact-match nouns
surface even below the cosine threshold.

Threshold is **0.35**. `topics-v0.json._threshold_learned_from` documents
the provenance:

> Set to balance recall on the 105-question
> `tests/comprehensive/questions_v2.json` corpus against false-positive
> topic suggestions on out-of-scope queries. Re-derive against your own
> eval corpus by running a PR-curve sweep over questions_v2.json with
> `EMEM_TOPIC_BACKEND=keyword EMEM_TOPIC_THRESHOLD=<x>`; pick the x
> that maximises precision at recall ≥ 0.80.

`EMEM_TOPIC_THRESHOLD` env overrides the JSON value.

---

## 15. Agent-discoverable surface

`emem-server` serves both HTTP/REST and MCP JSON-RPC on one port
(default `0.0.0.0:5051`). **169 REST routes** total, **79 under
`/v1/*`**, **49 MCP tools**. Discovery chain on first contact:

```text
  1. GET  /.well-known/emem.json         responder pubkey + capabilities
  2. GET  /.well-known/mcp.json          MCP transport advertisement
  3. GET  /.well-known/agent-card.json   metadata, recommended tool order
  4. GET  /v1/manifests                  bands_cid, algorithms_cid,
                                         sources_cid, schema_cid,
                                         registry_cid, topics_cid
  5. GET  /v1/grid_info                  cell pitch ~10 m square
  6. GET  /v1/data_availability          which bands have history
  7. POST /v1/locate {q:"<place>"}       → cell64
  8. POST /v1/recall, /v1/find_similar,
          /v1/verify, /v1/diff           primitives
```

MCP tools are a strict read-only subset of REST; writes (`attest`,
`backfill`, reviews POST) go through REST only.

---

## 16. Conformance

Two implementations conform when, given byte-identical inputs, they
produce byte-identical CIDs over the manifest set at
`/v1/manifests`:

```text
  bands_cid        BLAKE3 over canonical_cbor(BandsManifest)
                   (1792 dims, 35 cube slots)
  algorithms_cid   BLAKE3 over canonical_cbor(AlgorithmsManifest)
                   (155 entries)
  sources_cid      BLAKE3 over canonical_cbor(SourcesManifest)
                   (43 schemes)
  topics_cid       BLAKE3 over canonical_cbor(TopicsManifest)
                   (26 topics)
  schema_cid       BLAKE3 over canonical_cbor(SchemaBundle)
  registry_cid     BLAKE3 over canonical_cbor(FunctionRegistry)
                   (17 primary / 2 derivative / 1 negative)
```

A receipt binds `schema_cid` and `registry_cid` as struct fields.
The other four are exposed at `/v1/manifests` and
`/.well-known/emem.json`; conformance is verified by re-pulling the
manifests at receipt time and checking the CIDs match.

`cargo test --workspace` is the de-facto conformance check today;
the crate-internal tests at `crates/emem-codec/src/geo.rs`,
`tslot_text.rs`, and `emem-attest/src/lib.rs` are the de-facto
fixtures. Per-vector fixtures under `spec/test_vectors/` are not yet
extracted.

---

## 17. Privacy

The four `PrivacyClass` variants from `emem-core/src/privacy.rs`:
**Public** (open-data bands), **AggregateOnly { min_res }**
(population-density bands snap to coarser resolution; receipt carries
`privacy_snapped: true`), **L2OnlyWithModelCid** (fine-resolution
embeddings tied to a specific model checkpoint; L1 responders must
refuse), **Prohibited** (reserved; no band declares it today).

Legal surface (cited in SPEC.md §13): GDPR (Reg 2016/679), UK-GDPR,
DPDP-2023, CCPA-CPRA, RFC 9116. The canonical responder logs
`agent_ip_hash = base32_nopad_lower(blake3(client_ip)[..8])`, not
the raw IP; POST bodies are not captured; GET query strings are
captured for the 30-day journald retention window.

---

## 18. Honest limits

- **Sub-meter imagery.** No commercial high-resolution imagery
  pipeline. Sentinel-2 10 m is the finest pitch the responder
  serves.
- **Edge / onboard inference.** All inference is in-tenant; no
  spacecraft-bus encoder firmware ships with the protocol. The
  receipt schema accepts whatever `model_id` and `sensor_id` a
  customer attests under their own ed25519 key.
- **Federation.** Single primary; replicas are read-only. No
  multi-host clustering, no SOC 2 attestation.
- **JEPA v2 untrained.** The on-disk artifact is a residual-zero
  identity baseline. Training is gated on upstream Tessera
  publishing ≥ 3 vintages per cell; training pipeline is ready, the
  candidate-pool backfill is the bottleneck.
- **Tessera multi-vintage upstream-rate-limited.** The
  `dl2.geotessera.org` bucket ships 2017-2024 annual vintages; most
  cells in `/v1/coverage` have only the latest year attested
  locally.
- **12 wired data connectors.** The catalog count (43 declared
  schemes) is not the pitch. Five schemes remain declared-but-
  unwired: `openet.30m.daily`, `dynamic_world.v1`,
  `tropomi.s5p.ch4`, `tropomi.s5p.no2`, `viirs.dnb.monthly`.
- **Zero-knowledge verifier.** `verify Mode::Zk` was advertised in
  0.0.3 and returned 500; removed in 0.0.4.
- **Stake / economics.** `Attestation.stake` was reserved; removed
  from the struct and its call sites.
- **Filecoin / IPFS bridge.** `IpldConnector` is a stub.
- **Python / TypeScript SDKs.** Placeholder directories; agents use
  REST or MCP directly.

A request that names a missing surface returns a typed Absence (§6)
or a structured `ErrorCode`. No surface returns `verdict=false` for
an absent capability.

---

## 19. Comparison with adjacent work

### 19.1 Geospatial adjacents

- **STAC** describes scenes; emem describes per-pixel facts with
  provenance. STAC catalogs are an upstream connector kind, not the
  protocol.
- **GeoParquet** is a data format; emem is an addressing rule plus a
  receipt schema. A GeoParquet column can hold an emem fact's value,
  but the file carries neither the responder signature nor the
  Merkle path.
- **IPLD** is a CID layer; emem composes a CID rule on top of IPLD's
  CBOR tag 42 base32 encoding plus a domain-specific fact ontology.

### 19.2 emem in the memory-mechanism landscape

Memory mechanisms for LLM agents fall along three established lines,
each of which scopes to a single agent's history:

- **Textual stores** inject prior turns through the input context.
  Strength: flexible, no architecture change. Weakness:
  context-window limits, retrieval noise, compaction loss.
- **Parametric stores** fold prior interactions into adapter or
  prefix weights. Strength: zero retrieval cost at inference time.
  Weakness: static; cannot adapt to evolving information without
  retraining.
- **Outside-channel stores** keep state in a separate module reached
  by retrieval or encoding on a side channel. Strength: modular.
  Weaknesses: integration overhead, drift between the retrieval
  index and the backbone, and the silent-empty problem (a missing
  entry returns nothing rather than an attestable absence).

Recent work on compact, in-attention memory states (online
associative-memory matrices updated by gated delta-rule learning)
sharpens the per-session story further. None of these layers
operate at the scope emem operates at, because none of them
address the same question: *what is at this place on Earth, right
now, that any agent can cite later?*

emem is a persistent, planet-keyed, content-addressed memory layer.
It inherits the outside-channel virtues (modular, additive, LLM-
and runtime-agnostic — call it from any agent, in any host, with no
SDK install) and addresses the classical outside-channel failure
modes directly:

| Classical weakness          | emem's design response                                                                                                                                                                                                       |
|-----------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| *Integration complexity*    | Single Streamable-HTTP MCP endpoint plus mirrored REST. No SDK install, no auth, no per-tenant provisioning. Idempotent reads.                                                                                               |
| *Backbone misalignment*     | emem does not return embeddings the backbone has to fuse. It returns typed scalar facts in named units with a content-addressed CID. The agent paraphrases at its own attention; emem's contract ends at the bytes.          |
| *Retrieval drift / noise*   | The address is the place, not a similarity query. Two agents asking for `copdem30m.elevation_mean @ defi.zb4d9.pefa.zf619` get byte-identical CBOR back. No fuzzy ranking, no recall@k surprise.                              |
| *Per-session / per-tenant*  | emem's state is the planet, persisted on disk and content-addressed. A receipt minted by responder A in 2026-05 verifies offline against the same pubkey in 2030, on a self-hosted replica B that never spoke to A.          |
| *Silent empty*              | A missing band at a cell returns a signed `Absence` fact with a typed reason, not an empty array. The absence itself is content-addressed and citable as evidence.                                                            |

The relationship to in-agent memory is complementary, not
competitive. In-agent memory compresses a conversation's recent
history so the backbone stops spending quadratic attention on it;
emem compresses the planet's history so the backbone never loads
the raw scenes in the first place. An agent that uses both gets
compact internal memory of the chat plus shared external memory of
the world; the receipt CID is the bridge between them.

The properties emem holds that the in-agent layers structurally
cannot:

- **Reproducibility.** `fact_cid` dereferences to the same bytes on
  any conforming responder. Retrieval-style stores are reproducible
  only when the index is frozen, and only up to similarity, not
  byte equality.
- **Verifiability.** Ed25519 over a deterministic BLAKE3 preimage.
  Any party with the issuer's pubkey can verify a receipt without
  calling back. Browser-side verification ships at `/verify`.
- **Citation.** A 26-character CID an agent can quote verbatim to a
  user, a regulator, a competing agent, or its future self. The
  CID is the bibliographic primitive of this memory layer.
- **Cross-agent sharing.** Two agents on different runtimes, in
  different processes, in different timezones, with no shared
  state, paste the same CID and pull the same bytes. The protocol's
  job is to make this property hold across time and replicas.

The closing technical point worth importing from the in-attention
memory literature is *compact state*. There, an 8×8 matrix is
shown to be enough to retain useful historical signal once it is
addressed associatively rather than positionally. emem's analogue
is the 64-bit cell address: the entire knowledge graph for a place
collapses to one handle a downstream tool can quote, share, and
verify. The encoding scheme differs; the principle (address
memory by what it is *about*, not by where it sat in a stream) is
the same.

---

## 20. Open questions

- **H3 hex migration.** Spec target is H3-equivalent DGGS at
  resolution 13. cell64 is square at the equator, progressively
  non-square poleward. Migration requires a new manifest CID for
  the band ontology and an in-flight-fact story.
- **Trained JEPA v2.** Gated on upstream Tessera publishing
  multi-vintage history per cell.
- **WorldPop latency.** 2-4 s/cell at request time. Pre-baking the
  global 1 km² raster locally amortises the fetch.
- **Multi-modal Galileo.** S1 / ERA5 / TC / VIIRS / SRTM / DW / WC /
  LandScan / location modalities are zero-masked. Each needs a
  connector + chip fetcher.
- **Cross-modal alignment at scale.** Ridge-regression bridges
  between encoder spaces (Clay ↔ Prithvi, Prithvi ↔ Tessera) need
  labelled overlap sets per customer.

---

## References

- Beck, H.E., et al. "Present and future Köppen-Geiger climate
  classification maps at 1-km resolution." *Scientific Data* 5,
  180214 (2018). (`climate_archetype_triple` seed centroids.)
- Clay Foundation. "Clay v1.5." `made-with-clay/Clay` (Apache-2.0).
- Corley, I. "TerraBit — sign-bit rotation for binary k-NN."
  geospatialml.com/posts/terrabit. (TurboQuant rotation, §9.3.1.)
- Fields of The World (FTW). "Global agricultural field polygons,
  241 countries, 10 m, CC-BY-4.0." source.coop pmtiles archive.
- GeoNames. "cities-5000 — 68 581 populated places ≥ 5 000
  population, CC-BY-4.0." Vendored at
  `crates/emem-fetch/data/cities5000.txt.gz`.
- Hansen, M.C., et al. "High-resolution global maps of 21st-century
  forest cover change." *Science* 342, 850-853 (2013).
  (`deforestation_triple` uplift.)
- Healey, S.P., et al. "Mapping forest change using stacked
  generalization." *RSE* 204:717-728 (2018). (Consensus threshold
  provenance.)
- IBM / NASA / Jakubik et al. "Prithvi: a geospatial foundation
  model." `ibm-nasa-geospatial/Prithvi-EO-2.0-300M-TL`.
- O'Connor, J., Aumasson, J.-P., Neves, S., Wilcox-O'Hearn, Z.
  "BLAKE3: one function, fast everywhere."
- Oke, T.R. *Boundary Layer Climates*, 2nd ed. Methuen (1987); §2.3
  table 2.4 (urban α ≈ 1e-6 m²/s for `/v1/heat_solve`).
- Overture Maps Foundation. "Places, buildings, transportation,
  divisions themes, ODbL."
- Pekel, J.-F., et al. "High-resolution mapping of global surface
  water and its long-term changes." *Nature* 540, 418-422 (2016).
- Schumann, G.J.-P., et al. "The need for a high-accuracy, open-
  access global DEM." *Frontiers in Earth Science* 6:225 (2018).
- Snyder, J.P. "Map projections — a working manual." USGS PP 1395
  (1987). (UTM in `emem-fetch::proj`.)
- Tseng, G., et al. "Galileo — a multimodal geospatial foundation
  model." `nasaharvest/galileo`.
- RFC 8949 (CBOR §4.2 deterministic encoding), RFC 8032 (Ed25519),
  RFC 4648 (base32-nopad), RFC 9090 (multibase 'b'),
  RFC 9116 (security.txt).
