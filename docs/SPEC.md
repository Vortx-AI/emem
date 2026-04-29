# emem Protocol — Specification v0.0.2-draft

> Status: draft · 2026-04-27 · Editor: Vortx-AI
> Supersedes: `docs/SPEC.md@v0.0.1-draft`, `docs/PROPOSAL_v2.archive.md`, `docs/product-memory.md`

## Abstract

**emem** is an open, content-addressed, agent-native protocol for representing, exchanging, and verifying claims about places on Earth. It is engineered exclusively for AI-agent consumption — it does not retrofit human cartography conventions, gazetteer norms, or browser-era APIs. emem operates as a **global lazy memory**: agents recall `(cell, band, tslot)` triples; the protocol either returns a cached fact or fetches the canonical upstream sources, computes the band value, attests, caches forever, and returns. Coverage is the whole Earth, not a precomputed corpus.

The protocol defines: (a) a custom recursive cell tessellation (`emem cells`) and a token-economical, locality-preserving, self-decoding cell codec (`cell64`); (b) an epoch-relative integer temporal grid (`tslot`) replacing ISO 8601 in the canonical channel; (c) a vector-as-address scheme (`vec64`) that makes embedding space directly dereferenceable; (d) a 1792-dimensional band ontology — published as a **content-addressed manifest**, not a hardcoded constant — that fuses leading geospatial foundation embeddings (AlphaEarth, Sentinel-2/1, terrain, climate, soil, vision) with explicit per-band provenance, tempo, and privacy class; (e) a deterministic content-addressed fact format (`blake3(canonical_cbor(fact)) → CID`) supporting **primary**, **derivative**, and **negative** fact variants, each carrying a `schema_cid` for self-description; (f) signed attestation envelopes and proof-carrying receipts with cost/latency self-declaration; (g) a content-addressed function registry plus a swappable source-connector manifest that maps abstract source schemes to fetch templates (operators add mirrors, auth, regions without touching the protocol); and (h) MCP-first transport with a normative tool inventory and self-describing introspection tools so agents discover the active manifests at runtime, with REST and IPLD as compatibility adapters. The reference implementation is a Rust crate workspace.

The protocol is built on five constraints unique to agent consumption — **token economy, deterministic re-execution, append-only persistence, honest absence, zero-trust verification** — plus three architectural commitments that move the surface beyond what existing geospatial protocols offer: **lazy global materialization, vector-as-address, intent-routed planning**.

---

## §0 Quickstart for agents

Five things an agent needs to call any primitive. (Verbose explanation in §3, §11.)

1. **Discover the active manifests.** Call `emem.manifests` (or `GET /.well-known/emem.json`). You get back CIDs for the `bands`, `functions`, `sources`, `schema`, and `lcv1` registries. Cache them keyed by CID; they never change for a given CID.
2. **Address.** A point on Earth at a moment in time is `(cell, band, tslot)`:
   - `cell` is `cell64(lat, lng, res)` — 4 dot-separated bigrams, ≤4 tokens. Default `res=13` (~3.4 m).
   - `band` is one of the keys in the bands manifest (e.g. `"indices"`, `"alphaearth"`, `"landcover"`).
   - `tslot` is an unsigned integer offset from the emem epoch (2026-01-01T00:00:00Z), in units determined by the band's tempo (year / month / day / hour). Static bands always use `tslot = 0`.
3. **Recall.** Call `emem.recall(cell, [bands], tslot?)`. You get back `{facts: [Fact], receipt: Receipt}`. The Receipt carries `cost.was_cached`, `cost.source_freshness_s`, `cost.credits` — read these to choose your next move (cheap cache hit vs. expensive lazy fetch).
4. **Verify, don't trust.** Every Fact carries `signer`, `signature`, `schema_cid`. Verify the signature against `attesters[].key` from `emem.manifests`. Dereference `schema_cid` once and cache; subsequent facts under the same CID are guaranteed to parse.
5. **Compose, address by similarity.** When you need "places like this place", use `emem.find_similar(key=<cell64 or vec64>, k=10)` — embedding space is part of the address space. When you don't know which primitive to call, send `emem.intent({type: ..., ...})` and the protocol returns a Plan you can execute (or ask the protocol to execute for you).

That is the entire surface for L0 reads. Verify and attest are §6–§8; the rest of the spec is reference material.

---

## §1 Motivation & gap

### 1.1 Why a new protocol

Existing options for grounding AI agents in spatial reality are inadequate in three structural ways:

**Transactional location APIs** (Mapbox MCP, Google Maps Grounding Lite, CARTO MCP) [1, 2, 3] expose geocoding, routing, search. They are designed for *retrieval of places*, not *recall of facts about places*. They have no concept of an immutable, citeable "what was true at this cell on this date" unit, and no concept of fact-level content addressing.

**Geospatial knowledge graphs** (KnowWhereGraph [4], 12B triples, GeoSPARQL) demonstrate the value of pre-integrated cross-domain spatial data. They are read-only, schema-rigid, SPARQL-only — too high-friction for the agent inside-loop, and have no cryptographic commitment to facts.

**Spatial foundation models** (AlphaEarth [5, 6], Clay [7], Prithvi-EO-2.0 [8], SatMAE) produce dense embeddings that are the right substrate for retrieval. Recent benchmarking [9] documents three concrete limits when agents try to use AE alone: (i) limited spatial transferability, (ii) limited time sensitivity, (iii) low interpretability. emem mitigates all three by *fusing* AE with 32 other bands (raw S2/S1 for time, named per-band attribution for interpretability, terrain/climate/soil for transfer).

The gap nobody fills: **a cryptographically verifiable, token-economical, agent-contributable, vector-addressable, lazily-materialized global memory layer for spatial facts.** That is the emem protocol. Coverage is the whole Earth at sub-meter scale, but storage cost scales with *demand*, not with *area* — facts are computed and cached only when an agent asks for them.

### 1.2 Why now

- **MCP momentum**: 1,412 servers as of Feb 2026, 232% growth in 6 months, 97M monthly downloads [10]. The agent ecosystem has converged on a discovery+invocation surface; new protocols can ride that rail.
- **zkML reaches production**: Lagrange DeepProve-1 cryptographically proved a full LLAMA inference [11]; cost-of-proof is forecast to drop below $0.01/call in 2026, the threshold at which proof-carrying inference becomes default.
- **Content-addressed scientific data is mature**: IPLD + Filecoin Saturn provides a verifiable, cached retrieval substrate for content-addressed blobs [12]. emem facts can be IPLD blocks, inheriting persistence and CDN for free.
- **Foundation embeddings have stabilized**: AlphaEarth's 64D × 9 years format has held since mid-2025, and the 1792D fusion has been validated across 56 sites and three biomes.

The window to define the protocol is now, before incumbents (Google, Mapbox, ESA) ossify proprietary layers.

---

## §2 Design principles

The protocol obeys 14 principles. Implementations conforming to a level (§12) MUST satisfy all principles for that level.

1. **Content-addressed, not endpoint-addressed.** A fact's identifier is `blake3(canonical_cbor(fact))`. Endpoints are caches, not sources of truth.
2. **Agent-native CRS.** The default coordinate encoding is optimized for LLM tokenizers and ML inputs, not for human cartography.
3. **Token economy is a hard SLO.** Wire-format text MUST stay under documented token budgets per primitive (§9).
4. **Deterministic re-execution.** Every attestation includes a recipe that, given the same source CIDs, deterministically reproduces the value.
5. **Honest absence beats confident default.** If a band is unknown, the response is `null` with a `reason_cid`. Procedural placeholders are prohibited in the canonical channel.
6. **Append-only.** Frozen facts are never overwritten. Versioned facts chain Merkle-style.
7. **Provenance is mandatory.** Every fact carries source CIDs, signer pubkey, signature, and inclusion proof if part of a Merkle batch.
8. **Tempo-aware caching.** Bands declare a tempo class; clients SHOULD use it to set TTLs.
9. **One canonical key per fact.** No client-side joins.
10. **Transport-agnostic.** MCP, REST, gRPC, IPLD are adapters. Canonical CBOR round-trips through any of them with byte-level fidelity.
11. **Vector-addressable.** State is reachable by similarity (`emem:vec/<vec64>`), not just by ID. Embedding space is part of the address space.
12. **Self-describing.** Every response carries `schema_cid`. Fresh agents resolve unknown schemas once and cache.
13. **Intent-routable.** Agents MAY submit a typed `Intent` and receive an executable tool plan in lieu of selecting primitives manually.
14. **Privacy-snapping.** Implementations MUST enforce per-band `privacy_class` and snap responses to the coarsest permitted resolution before serving.

---

## §3 Addressing

### 3.1 emem cells — recursive icosahedral hex tessellation

The protocol defines its own hierarchical cell system. The geometry is identical in mathematical structure to several existing icosahedral discrete global grid systems (DGGS) — **this is intentional**, so that high-quality math libraries can serve as permitted backends — but the addressing, encoding, naming, and operator algebra are normative to emem.

**Construction.**

- Project the WGS84 sphere onto the 20 faces of a regular icosahedron.
- Each face is tiled with hexagons (with 12 pentagonal cells at icosahedron vertices, an unavoidable Euler-characteristic artifact).
- Subdivide recursively with **aperture-7 hexagonal subdivision**: each parent cell at resolution `r` has exactly 7 children at resolution `r+1` (one centered child + six neighbors), with edge length scaling by `1/√7 ≈ 0.378`.

**Resolution table** (16 levels; default fact resolution is **13**):

| Res | Avg edge length | Avg cell area | Use |
|---|---|---|---|
| 0 | 1108 km | 4.36×10⁶ km² | continent |
| 5 | 9.85 km | 2.52×10² km² | region |
| 7 | 1.22 km | 3.86 km² | tile |
| 9 | 174 m | 0.079 km² | neighborhood |
| 11 | 24.9 m | 1620 m² | sub-field |
| **13** | **3.41 m** | **30.2 m²** | **default fact resolution** |
| 15 | 0.50 m | 0.65 m² | reserved (sub-meter sensors, v0.2) |

Hexagons over squares because (a) all neighbors are equidistant (path/navigation primitives are clean) and (b) hex2vec [14] showed hexagonal embeddings have lower sampling artifacts than rectangular grids when used as ML inputs — relevant because agents will request cell vectors as model inputs at scale.

**Cell ID** is a 64-bit integer with the bit layout:

```
[63]      reserved (MUST be 0)
[62..59]  mode (4 bits, 16 modes; cell|directed_edge|undirected_edge|vertex|set|...)
[58..56]  edge/vertex disambiguation (3 bits)
[55..52]  resolution (4 bits, 0..=15)
[51..45]  base cell (7 bits, 0..=121 valid; 110 hexagons + 12 pentagons across the 20 icosahedron faces)
[44..0]   path: 15 × 3-bit child digits, level 1 (highest 3 bits) to level 15 (lowest 3 bits).
          Unused trailing levels (for resolutions < 15) are filled with the sentinel digit
          0b111 (=7), which is never a valid child.
```

Total: 1 + 4 + 3 + 4 + 7 + 45 = 64 bits. Resolution 0 is the bare base cell with no path digits; resolution `r` consumes the first `r` of the 15 path-digit slots. Reference implementations MAY use Uber H3 ≥4.0 [13] as a backend if and only if their outputs pass the `cell.*` test vectors (§19). H3 is not normatively cited in the wire format.

### 3.2 cell64 — token-economical, locality-preserving, self-decoding cell codec

`cell64` encodes a 64-bit emem cell ID as a 4-symbol string in a 65,536-symbol bigram alphabet. The alphabet is constructed by:

1. Compute the intersection of single-token strings across `cl100k_base`, `o200k_base`, `llama-3-bpe`, and the Claude tokenizer.
2. Filter to BPE-friendly bigrams (consonant-vowel or vowel-consonant patterns) with mean rank < 100,000 across all four tokenizers.
3. Order the alphabet so that adjacent bigram indices map to spatially adjacent cells (Hilbert-curve traversal of children at each resolution). Spatially proximate cells therefore share string prefixes.

**Locality property:** for any two cells `a, b` whose great-circle distance is `d`, the longest common `cell64` prefix is monotonically non-increasing in `d`. This makes cell IDs themselves usable as approximate spatial keys.

**Self-decoding property:** the codec includes a deterministic `cell64 → (lat, lng)` decoder that runs in O(resolution) without external lookup. Agents do not require a network round-trip to reason about an emem address.

**Token budget:** ≤ 4 tokens per cell ID under cl100k/o200k; ≤ 6 tokens worst case under non-aligned tokenizers. The full alphabet is pinned in `crates/emem-codec/data/cell64-alphabet-v0.bin` after empirical measurement (`tools/measure_alphabet.py`).

```
cell64:  ento.bria.calo.tris       # 4 bigrams, 17 chars, ~4 tokens
hex H3:  8d2a1072b59afff            # 15 chars, ~5 tokens
lat/lng: 12.971600, 77.594600        # 19 chars, ~12 tokens
```

A canonical address always validates round-trip: `from_cell64(to_cell64(cell)) == cell` MUST hold.

**Note:** the v0.0.1 hybrid namespace `@gazetteer:cell` is **dropped**. Gazetteers are a human-cartography convention with provenance, encoding, and licensing problems (GeoNames messy, OSM heavy, both centralizing). Agents do not need "Bengaluru, JP Nagar" — they need cells. A learned, agent-derived region naming scheme (clustered from nightlights + GHSL + admin signal, addressed by cell-set CID) is deferred to v0.2 and will not block v0 ratification.

### 3.3 tslot — token-economical temporal addressing

ISO 8601 is replaced in the canonical channel by `tslot`: an unsigned integer offset from the **emem epoch (2026-01-01T00:00:00Z UTC)** in tempo-class-implied units.

| Tempo class | Slot duration | Example |
|---|---|---|
| `static` | (none — `tslot = 0` always) | DEM, Köppen |
| `slow` | 1 year | AlphaEarth, soil |
| `medium` | 1 month | NDVI composites |
| `fast` | 1 day | S2 NDVI raw |
| `ultra_fast` | 1 hour | weather, traffic |

Encoded as a CBOR unsigned integer (≤3 bytes for any post-epoch time within 1000 years at hour granularity). Token-economical text rendering uses a base-32 short form (`t.k7q` ≈ 2 tokens).

**ISO 8601 interop:** ISO 8601 timestamps remain valid at the *ingest boundary* (sources, attestation inputs from upstream providers), but MUST be snapped to the band's tempo class on attestation. The canonical fact carries `tslot`, never an ISO string.

### 3.4 vec64 — vector-as-address

A 1792D float16 vector `v` is addressable as an emem URI:

```
vec64(v) = base32(blake3(canonical_f16_le(v))[:12])    # ~20 chars, ~5 tokens
emem:vec/<vec64>                                         # dereferenceable URI
```

The 12-byte (96-bit) prefix puts birthday collision at √(2⁹⁶) ≈ 8×10¹⁴ vectors, comfortably above the global emem fact-vector population (~10¹³ at full coverage). The full 32-byte CID is still the storage key; vec64 is the token-economical short form for inline reference.

Dereferencing `emem:vec/<vec64>` MUST return the top-k cells whose attested 1792D embedding has highest cosine similarity to the addressed vector (default `k=10`, capped at 1000), each with their similarity score and a Receipt. This makes embedding space a first-class part of the address space, alongside cells.

**Why this matters for agents.** Today, every spatial API returns vectors as values; emem also accepts vectors as keys. An agent that has computed a query embedding (from text, image, or another fact) can address state by similarity in a single primitive call — without orchestrating an external vector store.

Vectors of arbitrary dimension are supported; the protocol commits to the 1792D ontology as the **canonical** vector for the foreseeable future, with `Source.scheme = "emem.cube.v1"` identifying the canonical 1792D layout.

### 3.5 URI scheme

All emem addresses are dereferenceable URIs. There is no host-relative form in the canonical channel.

```
emem:cell/<cell64>                                       # cell at default res 13
emem:cell/<cell64>?res=11                                # explicit resolution
emem:cell/<cell64>?tslot=t.k7q                           # cell at a temporal slot
emem:fact/<cid>                                          # single fact
emem:vec/<vec64>?k=10                                    # vector address → top-k cells
emem:attest/<batch_cid>                                  # attestation batch
emem:receipt/<receipt_id>                                # signed receipt
emem:registry/<registry_cid>                             # function registry version
emem:schema/<schema_cid>                                 # CDDL schema version
emem:verify?cell=<cell64>&claim=<claim_b64>              # one-shot verify
emem:intent?type=<intent_type>&...                       # typed intent → tool plan
```

Conforming agents that emit emem URIs in chain-of-thought or output train downstream agents to follow them. URIs are the protocol's organic discovery vector.

---

## §4 Bands — the 1792D ontology (content-addressed manifest)

The 1792-dimensional band ontology is **NOT a protocol-level constant**. It is a **content-addressed manifest** (`emem-bands` kind, identifier `bands_cid`) that any operator can publish, version, and supersede. The protocol pins a manifest CID per attestation; facts attested under different manifests remain valid forever under their original CID.

The v0 manifest contains 32 named bands and is shipped at `crates/emem-core/data/bands-v0.json`. Its physical layout (key, offset, dims) is byte-identical to AgriSynth's `BAND_OFFSETS` registry — that registry is the upstream source-of-truth for *what to fuse and in what order*; emem inherits it so cubes computed by either codebase decode under either lib. Family classification is editorial (not load-bearing); tempo and privacy class are normative per band.

The manifest schema:

```json
{
  "manifest": "emem-bands",
  "version":  "v0",
  "total_dims": 1792,
  "bands": [
    { "key": "geotessera", "family": "foundation", "offset": 0, "dims": 128,
      "tempo": "slow", "privacy": {"class": "public"} },
    ...
  ]
}
```

Validation invariants (enforced by `BandRegistry::validate`):

- `offset`s are contiguous starting at 0
- `Σ dims == total_dims` (=1792 in v0)
- Band keys are unique
- Tempo and privacy class strings are in the declared enum sets

Top-level summary of v0 (full manifest at `data/bands-v0.json`; CID at `/.well-known/emem.json#manifests.bands_cid`):

| Family | Dims | Bands | Tempo | Privacy class |
|---|---|---|---|---|
| foundation | 704 | geotessera (128), alphaearth (576, 9 yrs × 64) | medium | public |
| optical | 13 | sentinel2_raw (10), indices (3) | fast | public |
| radar | 2 | sentinel1_raw | fast | public |
| terrain | 43 | dem (3), terrain_derived (32), cop_dem (8) | static | public |
| climate | 56 | climate (4), koppen (32), terraclimate (20) | static-to-medium | public |
| soil | 20 | soilgrids | slow | public |
| vegetation | 160 | temporal_diff (64), phenology (32), multiscale (64) | slow-to-medium | public |
| landcover | 44 | landcover (8), forest_change (12), mangrove (4), ecoregions (20) | slow | public |
| water | 16 | surface_water (12), ocean_chl (4) | fast | public |
| human | 38 | nightlights (8), ghsl (8), population (8), protected (4), admin (10) | static-to-fast | **aggregate_only at res ≥ 11** |
| vision | 384 | sam3_visual (192), qwen_visual (192) | slow | **L2-only, model-CID required** |
| topology | 32 | topology | static | public |
| encoding | 160 | spatial_fourier (96), temporal_fourier (64) | static-to-fast | public |
| reserved | 120 | reserved | (future sensors) | — |

**Privacy classes** (§13 normative):

- `public` — unrestricted at any resolution
- `aggregate_only at res ≥ N` — implementations MUST NOT serve at resolution finer than N; queries at finer res return aggregated values with `privacy_snapped: true` flag
- `L2-only` — admissible only at conformance level L2, requires `Source.cid` of model checkpoint
- `prohibited` — reserved; MUST NOT be served

The full normative manifest lives at `crates/emem-core/data/bands-v0.json`. Loaded and validated by `emem_core::bands::BandRegistry`. CID derivation: `base32(blake3(canonical_cbor(manifest)))[:32]`.

**lcv-1 land cover taxonomy.** The `landcover` band carries an 8-dim ESA-WorldCover one-hot (matching agri's layout). The `lcv-1` *taxonomy* (64 leaves, 8 families) is a richer leaf index served as a separate Fact value (`band: "landcover.lcv1_leaf"`) when requested; it is also content-addressed via its own manifest. v0 ships placeholder names (`lcv-1.f0.l0` … `lcv-1.f7.l7`); v0.1 swaps in learned cluster centroids derived from AlphaEarth + S2 + climate, at which point each leaf gains a canonical 1792D centroid embedding so that `landcover:lcv-1.43` is *also* a vector.

---

## §5 Facts — the immutable unit

### 5.1 Fact variants

emem defines three fact variants, all content-addressed and signed identically:

```cddl
Fact = PrimaryFact / DerivativeFact / NegativeFact

PrimaryFact = {
  kind:        "primary",
  cell:        text,                  ; cell64 string
  band:        text,                  ; band key, e.g. "indices.ndvi"
  tslot:       uint,                  ; epoch-relative slot (§3.3)
  value:       any,                   ; band-defined; numeric, vector, enum
  unit:        ? text,                ; SI unit if applicable
  confidence:  float,                 ; 0..1
  uncertainty: ? Uncertainty,         ; distribution, not just point estimate
  sources:     [+ Source],
  derivation:  Derivation,
  privacy_class: text,
  schema_cid:  text,                  ; CID of the CDDL fragment this conforms to
  signer:      bytes .size 32,
  signed_at:   text,                  ; ISO 8601 (signing wall clock — not data time)
}

DerivativeFact = {
  kind:        "derivative",
  cell:        text,
  band:        text,
  tslot_window: [uint, uint],         ; [start, end] inclusive
  op:          text,                  ; "delta" | "mean" | "trend" | "rate" | "anomaly"
  parents:     [+ text],              ; CIDs of input facts
  value:       any,
  confidence:  float,
  derivation:  Derivation,
  schema_cid:  text,
  signer:      bytes .size 32,
  signed_at:   text,
}

NegativeFact = {
  kind:        "absence",
  cell:        text,
  band:        text,
  tslot:       uint,
  reason_cid:  text,                  ; CID of the evidence that confirmed absence
  confidence:  float,
  sources:     [+ Source],
  schema_cid:  text,
  signer:      bytes .size 32,
  signed_at:   text,
}

Source = {
  scheme:      text,                  ; "sentinel2", "alphaearth", "srtm", ...
  id:          text,                  ; provider-defined ID
  cid:         ? text,                ; IPLD CID if available
  hash:        ? bytes .size 32,      ; SHA-256 of source bytes if known
  captured_at: ? text,                ; ISO 8601
}

Derivation = {
  fn:    text,                        ; function registry key, e.g. "nv.l2a@1"
  args:  ? map,                       ; deterministic args
}

Uncertainty = {
  family: text,                       ; "gaussian" | "interval" | "categorical"
  params: map,                        ; family-specific
}
```

### 5.2 Canonical encoding

Facts are encoded with **canonical CBOR** (RFC 8949 deterministic encoding: smallest int representation, sorted map keys, no indefinite-length items). emem additionally requires the **emem-CBOR profile**: mandatory CBOR tags for cells (tag 65000), tslot (65001), vec64 (65002), and CIDs (tag 42, IPLD-standard). Two implementations MUST produce byte-identical CBOR for the same fact.

### 5.3 Content ID (CID)

```
fact_cid = base32(blake3(canonical_cbor(fact))[:32])
```

`fact_cid` is the protocol's primary key. Two agents that derive the same fact from the same sources via the same function converge on the same `fact_cid`. The CID is also a valid IPLD CID (multihash prefix `1e20`, multibase `b`) for IPLD interop.

A token-economical short form `cid64` (first 8 bytes, base32) is defined for inline reference in token-budgeted text; full CIDs MUST be used in canonical CBOR.

### 5.4 Determinism contract

A `Derivation.fn` listed in the protocol's function registry MUST be deterministic: given identical source CIDs and identical args, it MUST produce identical `value`. Non-deterministic functions are prohibited from the canonical channel; they may only be exposed via the `predict/` URI namespace, which is explicitly probabilistic and out of scope for v0.

The function registry is **content-addressed** (§16), not URL-addressed: `Attestation.registry_cid` pins which registry version was in force at attestation time. A breaking change to `nv.l2a` ships as `nv.l2a@2`; old facts continue to be valid under their original `Derivation.fn`.

---

## §6 Attestations — the write protocol

### 6.1 Attest envelope

```cddl
Attestation = {
  facts:        [+ Fact],
  batch_root:   bytes .size 32,       ; blake3 Merkle root over fact_cids
  attester:     bytes .size 32,       ; ed25519 pubkey
  attester_key_epoch: uint,           ; key rotation epoch
  registry_cid: text,                 ; CID of function registry version in force
  schema_cid:   text,                 ; CID of the CDDL profile in force
  stake:        ? uint,               ; credits committed (v2.5)
  signature:    bytes .size 64,       ; ed25519(blake3(batch_root || registry_cid || schema_cid))
  attested_at:  text,                 ; ISO 8601
}
```

### 6.2 Submission semantics

1. Implementation verifies CBOR canonicalization of every fact.
2. Recomputes each `fact_cid` and the `batch_root`.
3. Verifies `signature` against `attester` at the declared `attester_key_epoch`.
4. Verifies `registry_cid` and `schema_cid` are recognized.
5. For each new `fact_cid`: if absent, store; if present, deduplicate (and credit attester for novelty == 0).
6. Returns per-fact CID list, batch acceptance receipt, attester credit delta.

### 6.3 Stake & slashing — v2.5

In v2.0 only protocol-issued attester keys (operated by Vortx) may successfully attest. In v2.5, third-party attesters stake protocol credits; a successful `challenge` (§8.4) slashes the attester's stake and rewards the challenger. Stake economics, slashing fractions, and challenge windows are deferred to a separate `STAKE.md` companion spec.

---

## §7 Receipts — proof of recall, with cost

```cddl
Receipt = {
  request_id:        text,            ; ULID
  served_at:         text,            ; ISO 8601
  primitive:         text,            ; "recall" | "verify" | "find_similar" | ...
  intent:            ? text,          ; if served via emem.intent
  cells:             [* text],
  fact_cids:         [* text],
  schema_cid:        text,            ; CID of the response schema
  merkle_proof:      ? MerkleProof,
  responder:         bytes .size 32,
  responder_key_epoch: uint,
  signature:         bytes .size 64,
  source_versions:   { * text => text },
  registry_cid:      text,            ; CID of registry used to serve
  cost:              Cost,
}

Cost = {
  credits:             uint,          ; protocol credits charged
  latency_p50_ms:      uint,          ; observed latency, this primitive class
  latency_p99_ms:      uint,
  source_freshness_s:  uint,          ; age of stalest source, seconds
  was_cached:          bool,
}
```

Receipts are byte-stable: two responders serving the same fact under the same protocol version produce signatures that differ only in `responder`, `responder_key_epoch`, `signature`, `served_at`, and `cost`. The agent can hand the receipt to its caller as cryptographic evidence — and the caller can independently re-verify against the protocol's published attester pubkeys.

**Why `cost` is in the receipt.** Agent planners need to make local decisions about which primitives to call. Surfacing real cost+latency+freshness in the receipt lets the agent build an empirical model of primitive costs without a separate metering API.

**Key rotation.** Each attester/responder publishes pubkeys with `epoch` numbers. Receipts cite the epoch used. Compromised keys are revoked by publishing `revoked_at` for that epoch in `/.well-known/emem.json`; receipts signed pre-revocation remain valid; receipts signed post-revocation are invalid.

---

## §8 Verification — the wedge

### 8.1 Custom claim grammar

A claim is a structured predicate over `(band, op, value, tslot_window)`. The grammar is **custom-but-structural** — no human-mnemonic predicate names (`LandCoverIs`, `DeforestedSince`); instead, predicates compose from a small algebra:

```cddl
Claim = {
  band:     text,                     ; band key, e.g. "indices.ndvi"
  op:       Op,                       ; comparison or membership
  value:    any,                      ; band-typed
  tslot:    ? uint,                   ; specific slot
  window:   ? [uint, uint],           ; or a range of slots (one of tslot|window)
  agg:      ? text,                   ; "any" | "all" | "mean" | "min" | "max" over window
}

Op = "eq" / "ne" / "lt" / "le" / "gt" / "ge" / "in" / "ni" / "exists" / "absent"
```

Examples:

```cbor
{band: "landcover.class",     op: "eq", value: "lcv-1.43",       tslot: 26}
{band: "indices.ndvi",        op: "gt", value: 0.7,              window: [12, 23], agg: "mean"}
{band: "human.protected",     op: "exists",                       tslot: 0}
{band: "water.surface",       op: "absent",                       window: [10, 22], agg: "all"}
{band: "human.population",    op: "gt", value: 1000.0,            tslot: 26}
```

The grammar is extensible; new ops and band-typed value validators ship under semver and degrade gracefully.

### 8.2 Verification workflow & resolution modes

```
verify(claim, cell, mode)
  ├── mode = "fast"      → look up canonical fact_cid; agree/disagree+evidence; no inference
  ├── mode = "resolve"   → if fact missing, trigger self-attestation, return result + new CID
  └── mode = "zk"        → run claim eval inside a zkML circuit (DeepProve-style); ZKP receipt
```

Default mode is `"fast"`. `"resolve"` is metered (triggers compute). `"zk"` is premium and only available for high-value claim types.

### 8.3 opML by default

For `mode=resolve` and `mode=zk`, the protocol uses an **optimistic ML** workflow modeled on zk-OPML [15]: the responder produces a result with cheap Merkle commitments to intermediate states; challengers may dispute by demanding ZKP for any operator. This achieves proof-carrying inference at near-opML cost in the common case, and zkML cost only on dispute.

### 8.4 Challenge

A `challenge(attestation_id, counter_evidence)` primitive (L2 only) disputes a fact. `counter_evidence` is itself an attestation with a conflicting value + sources. Triggers protocol-level re-execution from sources; if the original attestation is refuted, its stake is slashed and challenger is rewarded. Stubbed in v2.0; activated in v2.5 alongside the staking economy.

---

## §9 Wire formats

### 9.1 Canonical CBOR (machine ↔ machine)

The normative wire format. Profile: RFC 8949 deterministic encoding, plus the emem-CBOR tag set (§5.2). Used between conforming implementations, for IPLD storage, for Merkle hashing.

### 9.2 Token-economical text (LLM consumption)

For LLM ingestion the protocol defines a compact text rendering:

```
@cell:ento.bria.calo.tris  t:k7q
  landcover:lcv-1.43
  ndvi:0.31±0.04
  slope:2.1°
  pop:8400/km²[snapped:res9]
  surface_water:absent[reason:r4w...]
  #r:k7q3v
```

Constraints (cl100k tokens, measured, MUST hold):

- `recall(cell)` minimum payload (1 cell, 5 bands): ≤ 80 tokens
- `query_region(bbox, ≤100 cells)`: ≤ 600 tokens
- `verify(claim, cell)` (fast mode): ≤ 30 tokens
- `find_similar(cell, k=10)`: ≤ 200 tokens
- `intent(intent)` plan output: ≤ 150 tokens

Implementations MUST emit token-economical text for any `Accept: text/emem-tx` request and MUST include the `#r:<cid64>` receipt reference. Full Receipts dereference at `emem:receipt/<id>`.

### 9.3 SSE streaming with progressive refinement

Used for `recall`, `query_region`, `trajectory`. Each `data:` chunk carries one or more facts in canonical CBOR (base64-wrapped) or token-economical text. **Refinement order is normative:** chunks MUST be emitted from coarsest resolution to finest — res-9 facts first (cheap, immediate), then res-11, then res-13 — so agents can act on partial state before the stream completes. The terminal chunk carries the Receipt.

---

## §10 Discovery

The primary discovery mechanism is **emem URIs in agent context**. A conforming agent SHOULD attempt to dereference any `emem:` URI it observes.

The fallback discovery mechanism is HTTP `.well-known`:

```
GET /.well-known/emem.json     ; protocol version, supported levels, attesters, manifests
GET /.well-known/llms.txt      ; AI-readable summary
GET /.well-known/mcp.json      ; MCP transport announcement
GET /openapi.json              ; OpenAPI 3.1, utoipa-generated (compat layer)
```

A `GET /.well-known/emem.json` example:

```json
{
  "protocol":  "emem/v0.0.2",
  "levels":    ["L0", "L1"],
  "attesters": [
    { "key": "ed25519:base32...", "epoch": 1, "operator": "Vortx-AI",
      "since": "2026-04-27", "revoked_at": null }
  ],
  "transports": {
    "mcp":  { "stdio": "npx @emem/mcp", "sse": "https://mcp.emem.dev/sse" },
    "rest": "https://api.emem.dev/v0",
    "ipld": { "gateway": "https://ipfs.io/ipfs/", "saturn": true }
  },
  "manifests": {
    "bands_cid":     "b...",
    "functions_cid": "b...",
    "sources_cid":   "b...",
    "schema_cid":    "b...",
    "lcv1_cid":      "b...",
    "alphabet_cid":  "b...",
    "coverage_cid":  "b..."
  }
}
```

**Key change vs v0.0.1:** the registry, coverage, schema, and codec alphabet are all **content-addressed CIDs** in this manifest. They are not operator-mutable URLs. An operator that wants to publish a new registry must publish a new CID; old CIDs remain valid forever. This removes the silent-mutation backdoor in v0.0.1.

---

## §11 Transport — MCP-first tool inventory

MCP is the **primary** transport. REST and gRPC are compatibility adapters; IPLD is a storage adapter.

### 11.1 MCP tools (normative)

Conforming MCP servers MUST expose this tool set with these exact names and parameter schemas:

| Tool | Inputs | Outputs |
|---|---|---|
| `emem.recall` | `cell: cell64, bands?: [str], tslot?: uint` | `{facts: [Fact], receipt: Receipt}` |
| `emem.query_region` | `geometry: cell64\|bbox\|geojson, bands?: [str], agg?: str` | `{facts: [Fact], receipt}` |
| `emem.compare` | `a: cell64, b: cell64, family?: str` | `{cosine: float, per_band: map, receipt}` |
| `emem.find_similar` | `key: cell64\|vec64, k?: uint, filter?: ClaimAlgebra` | `{neighbors: [{cell, score}], receipt}` |
| `emem.verify` | `claim: Claim, cell: cell64, mode?: "fast"\|"resolve"\|"zk"` | `{verdict: bool, evidence: [cid], receipt}` |
| `emem.trajectory` | `cell: cell64, band: str, window: [tslot, tslot]` | `{series: [(tslot, value)], receipt}` |
| `emem.diff` | `cell: cell64, band: str, tslot_a: uint, tslot_b: uint` | `{delta_fact: DerivativeFact, receipt}` |
| `emem.attest` | `facts: [Fact]` (L2 / authorized only) | `{cids: [str], batch_root, receipt}` |
| `emem.challenge` | `attestation_id: str, counter: Attestation` (L2 only) | `{verdict, slashed?, receipt}` |
| `emem.bands` | `cid?: str` (default: active manifest) | `{manifest: BandRegistry, cid}` |
| `emem.functions` | `cid?: str` | `{manifest: FunctionRegistry, cid}` |
| `emem.sources` | `cid?: str` | `{manifest: SourceRegistry, cid}` |
| `emem.schema` | `cid: str` | `{cddl: str, json_schema: object, cid}` |
| `emem.errors` | — | `{codes: [{code, description}]}` |
| `emem.manifests` | — | `{bands_cid, functions_cid, sources_cid, schema_cid, lcv1_cid, alphabet_cid, coverage_cid}` |
| `emem.intent` | `intent: Intent` | `{plan: [ToolCall], cost_estimate: Cost}` |

### 11.2 REST compatibility surface

Resource-style paths, OpenAPI 3.1, generated by `utoipa`:

```
GET    /v0/cells/{cell64}                            → recall
GET    /v0/cells/{cell64}/bands/{band}               → recall (single band)
POST   /v0/query                                     → query_region
POST   /v0/find_similar                              → find_similar
POST   /v0/verify                                    → verify
GET    /v0/facts/{cid}                               → fact dereference
GET    /v0/vec/{vec64}                               → vec64 dereference
GET    /v0/registry/{registry_cid}                   → registry dereference
GET    /v0/schema/{schema_cid}                       → schema dereference
POST   /v0/attest                                    → attest (L2 / authorized)
POST   /v0/intent                                    → intent
```

REST is a strict subset of MCP capability and is intended for human developers exploring with curl, not for in-loop agent use.

### 11.3 Error code catalog (normative)

All MCP and REST responses on failure carry `{error: {code, message, offending?}}` where `code` is one of the wire-stable strings below. New codes ship under semver and degrade gracefully (`internal` is the always-valid fallback).

| Group | Code | Meaning |
|---|---|---|
| address | `invalid_cell` | cell64 round-trip failed |
| address | `invalid_resolution` | resolution out of [0, 15] |
| address | `tslot_mismatch` | tslot did not match the band's tempo grain |
| lookup | `band_not_in_registry` | band key not in active manifest |
| lookup | `function_not_in_registry` | function key not in active manifest |
| lookup | `source_scheme_unknown` | source scheme not in active sources manifest |
| lookup | `cid_not_found` | CID could not be dereferenced |
| lookup | `registry_cid_unknown` | referenced registry CID is not known to this responder |
| lookup | `schema_cid_unknown` | referenced schema CID is not known to this responder |
| privacy | `privacy_refused` | privacy class refuses serving at requested resolution |
| auth | `level_too_low` | operation requires a higher conformance level |
| auth | `attester_revoked` | attester key has been revoked at the cited epoch |
| auth | `unauthorized` | caller lacks authorization for L2/staked op |
| verify | `claim_undecidable` | claim cannot be decided; switch to `mode=resolve` |
| verify | `bad_signature` | signature verification failed |
| verify | `bad_merkle_proof` | Merkle inclusion proof did not validate |
| verify | `canonical_encoding_divergence` | upstream produced byte-different canonical CBOR |
| compute | `source_fetch_failed` | upstream source fetch failed (network, auth, rate-limit) |
| compute | `source_format_mismatch` | source response did not match expected format |
| compute | `compute_timeout` | compute deadline exceeded |
| compute | `compute_quota_exceeded` | per-caller compute quota exhausted |
| compute | `rate_limited` | per-caller QPS rate limit exceeded |
| internal | `cache_error` | cache backend reported an error |
| internal | `internal` | catch-all; responder MUST include a free-form message |

Servers MUST also expose this catalog at `emem.errors` (MCP) and `GET /v0/errors` (REST) so agents can fetch the full code set at runtime.

---

## §12 Conformance levels

| Level | What it covers | Use case |
|---|---|---|
| **L0** | Read-only: `recall`, `query_region`, `compare`, `find_similar`, `diff`, `trajectory`, `intent`. Receipts MUST be served. Privacy-snapping enforced. opML/zkML not required. | hosted Vortx node; agent SDKs |
| **L1** | L0 + `verify` (fast & resolve modes), full Merkle inclusion proofs, signed attestation log replay, content-addressed registry+coverage manifests | self-hosted nodes; auditors |
| **L2** | L1 + `attest` (third-party), `challenge`, `verify(mode=zk)`, on-chain anchoring, vision bands admissible | the open protocol; v2.5+ |

A conforming implementation MUST publish its level in `/.well-known/emem.json` and MUST enforce per-band privacy class (§13) at every level.

---

## §13 Privacy

H3-equivalent res-13 cells are ~3.4m on a side. Some bands at that resolution are PII-loaded — population at building scale, nightlights at residence scale, future thermal at vehicle scale. The protocol enforces privacy at the band declaration level:

- **`public`** — unrestricted; default.
- **`aggregate_only at res ≥ N`** — implementations MUST NOT serve at resolution finer than N. Queries at finer resolution receive aggregated values from the res-N parent, with `privacy_snapped: true` and the parent cell ID in the response.
- **`L2-only`** — admissible only at conformance level L2; requires `Source.cid` of the model checkpoint; not available on hosted L0/L1 nodes.
- **`prohibited`** — reserved for future bands the protocol has chosen not to expose; serving such bands is a conformance violation.

The privacy class is part of the band registry and therefore content-addressed via `manifests.coverage_cid`. A privacy reclassification is a registry version bump.

---

## §14 Versioning policy

The protocol uses semantic versioning at three levels: protocol, registry, schema. All three are content-addressed.

**Protocol version (`emem/vMAJOR.MINOR.PATCH`).** PATCH for editorial. MINOR for additive changes that don't break old facts (new bands in `reserved` slots, new function registry entries, new claim ops). MAJOR for breaking changes (band ontology rearrangement, CBOR profile changes, fact schema changes).

**Registry version (CID).** Adding a function is a new registry CID. Removing a function requires a MAJOR protocol bump. Bumping a function (`nv.l2a@1` → `nv.l2a@2`) is additive; old facts under `@1` remain valid forever.

**Schema version (CID).** Each schema CID is a complete CDDL profile. Old schema CIDs remain dereferenceable; receipts cite the schema CID in force at serve time.

**Append-only invariant.** No primary, derivative, or negative fact may be edited. New attestations MAY contradict old ones; the protocol surfaces both via `verify` and resolves via challenge (§8.4). Old facts remain queryable and citeable forever.

---

## §15 Reference implementation

Cargo workspace at the repository root:

```
crates/
  emem-core/         ; cell algebra, tslot, manifest loader (bands/functions/sources), keys, errors
  emem-codec/        ; cell64 + tslot text + vec64 + cid64 codecs, alphabet data
  emem-fact/         ; Fact/Attestation/Receipt types, canonical CBOR, typed CIDs
  emem-claim/        ; claim algebra + evaluator
  emem-cache/        ; multi-tier cache (Hot sled / Warm parquet / Cold IPLD)
  emem-fetch/        ; source-connector framework + dispatcher (HTTPS, GCS, IPLD)
  emem-storage/      ; composite: cache + fetch + materializer + Merkle log
  emem-cubes/        ; AgriSynth cube loader (reference; not on the hot path)
  emem-primitives/   ; recall, query_region, compare, find_similar, verify, diff, trajectory
  emem-attest/       ; attestation envelope, Merkle batching
  emem-intent/       ; intent grammar + heuristic planner (v0); learned planner v0.2
  emem-mcp/          ; rmcp transport adapter (primary)
  emem-api-rest/     ; axum + utoipa REST compat layer
  emem-cli/          ; `emem serve|materialize|keygen|verify|manifests|bands|functions|sources|errors`
sdks/
  emem-py/           ; thin Python client (ctypes over emem-core)
  emem-ts/           ; thin TypeScript client (rebuilt against new spec)
spec/
  test_vectors/      ; canonical CBOR + cell + tslot + vec64 + claim eval test vectors
tools/
  measure_alphabet.py  ; cell64 alphabet derivation from tokenizer corpora
```

**Storage decisions for the default backend.** emem is a global lazy memory; "storage" is really *cache + log*, not a precomputed dataset.

- **Hot tier — `sled`** (~30 days, sub-ms point lookups). The CID → fact KV plus the `(cell, band, tslot) → fact_cid` canonical index.
- **Warm tier — Parquet** (~90 days). Columnar scans for `query_region`; promoted out of Hot on age.
- **Cold tier — IPLD/IPFS** (forever). Content-addressed; addressed by CID; Filecoin Saturn caching.
- **Append-only Merkle log — segment files of 1 GiB**, format `[u32 LE: cbor_len][cbor_bytes][32 bytes: blake3(cbor_bytes)]`, trailing per-segment hash, fsync MUST happen before receipt is signed.
- **Backup / replication.** Sealed segment files snapshot to S3/IPFS every N segments; `SegmentManifest{index, hash, bytes}` is published into the coverage manifest. Restore = pull segments in order, verify trailing hash per segment, replay attestations.
- **Multi-node.** Operators shard by H3 res-7 parent (≈1.22 km tiles). Cross-shard reads are stitched at the primitive layer; gossip-dedupe between nodes is deferred to v2.5 federation.
- **Vector index — Lance** (hierarchical: index res-7 cell *centroids* of attested cubes, drill down on demand to bound RAM).

Choice of Rust is normative for the reference implementation but NOT for conforming implementations; alternative implementations (Go, TypeScript, C++, Python) are welcome and MUST pass the test vector suite.

---

## §16 Function registry — content-addressed manifest

The v0 function registry is a **content-addressed manifest** (`emem-functions` kind), shipped at `crates/emem-core/data/functions-v0.json`. All functions are deterministic. Registry CID is computed over the canonical CBOR of the validated registry and pinned in `/.well-known/emem.json#manifests.functions_cid`.

A function entry binds: `(key, kind, out_band, [out_index], sources[], formula, deterministic)`. `sources[]` references abstract source schemes (resolved against the **sources manifest**, §16a) — never raw URLs. This separation lets operators publish their own sources manifest with regional mirrors / auth credentials without forking the function registry.

Summary of v0 (full manifest in `data/functions-v0.json`):

| `fn` key | Output band | Input sources | Notes |
|---|---|---|---|
| `nv.l2a@1` | `indices.ndvi` | Sentinel-2 L2A B04, B08 | `(B08 - B04) / (B08 + B04)` |
| `ev.l2a@1` | `indices.evi` | S2 L2A B02, B04, B08 | `2.5 * (B08 - B04) / (B08 + 6*B04 - 7.5*B02 + 1)` |
| `nw.l2a@1` | `indices.ndwi` | S2 L2A B03, B08 | `(B03 - B08) / (B03 + B08)` |
| `sl.dem@1` | `terrain_derived.slope` | DEM (Copernicus 30m) | Horn 1981 |
| `as.dem@1` | `terrain_derived.aspect` | DEM | Horn 1981 |
| `tp.dem@1` | `terrain_derived.tpi` | DEM | mean of 3×3 neighborhood diff |
| `lc.esa@1` | `landcover.class` (`lcv-1`) | ESA WorldCover | manual 11-class → lcv-1 mapping (`crates/emem-core/data/lcv-mapping-v1.json`) |
| `pp.ghsl@1` | `human.population` | GHSL R2023A | direct read, density per km² |
| `nl.viirs@1` | `human.nightlights` | VIIRS DNB | monthly composite |
| `kg.koppen@1` | `climate.koppen` | Beck et al. 2018 | direct lookup, 32-class |
| `sr.soil@1` | `soil.*` (20 dims) | SoilGrids 2.0 | direct read at 250m, bilinear to 10m |
| `ae.slice@1` | `foundation.alphaearth` (576) | AlphaEarth Foundations | year × 64 = 576 dims |
| `gt.slice@1` | `foundation.geotessera` (128) | GeoTessera | direct slice |
| `s2.comp@1` | `optical.sentinel2_raw` (10) | S2 L2A | monthly median composite, 10 bands |
| `s1.comp@1` | `radar.sentinel1_raw` (2) | S1 GRD | monthly mean composite, VV+VH |
| `nd.delta@1` | `indices.ndvi` (DerivativeFact) | two NDVI primary CIDs | `value_b - value_a` |
| `nd.trend@1` | `indices.ndvi` (DerivativeFact) | window of NDVI primary CIDs | OLS slope per month |
| `abs.s1.water@1` | `water.surface` (NegativeFact) | S1 backscatter scene | absence confirmed if max VV < threshold |

Adding a function is a registry CID bump (§14). New functions MUST ship with golden test vectors and a deterministic implementation; no test vectors, no merge.

### §16a Source-connector manifest

Functions name source *schemes* (e.g. `"sentinel2.l2a"`, `"copernicus.dem.30m"`); the source-connector manifest (`emem-sources`) maps schemes to fetch templates. Manifest at `crates/emem-core/data/sources-v0.json`; CID at `/.well-known/emem.json#manifests.sources_cid`.

Per scheme: ordered list of providers (failover-aware), each with `connector_kind ∈ {gcs_cog, https_cog_vsicurl, https_geotiff, ipld_cid}`, `url_template` (with `{cell64,year,month,day,channel,tile_id,...}` interpolation), `auth`, `rate_limit_qps`, and `license`. Operators publish their own sources manifest CID to add mirrors, regional routes, or paid feeds without changing the function registry. Source schemes are wire-stable across manifests; URLs and auth are not.

Validation invariants: schemes unique; each scheme has ≥1 provider; provider declares either `url_template` or `cid` (for IPLD bundles). Loaded by `emem_core::sources::SourceRegistry`.

---

## §17 Lazy materialization

emem is a **global memory**, not a precomputed dataset. Coverage is the whole Earth (~10¹⁴ res-13 cells × 90+ bands × tslots; the universe of addressable facts is far larger than any storage budget). The protocol fetches the canonical upstream sources, computes facts, and caches them: only on demand, only what's needed, forever.

The materialization pipeline (run from any primitive that touches a fact):

```text
recall(cell, band, tslot)
  ├── cache.lookup_canonical((cell, band, tslot)) → Some(cid) → cache.get_fact(cid) → return
  └── miss:
        1. registry.functions.lookup(band → producing fn)
        2. for each src in fn.sources:
             dispatcher.fetch(SourceRegistry, FetchRequest{cell, tslot, src.scheme, src.channels})
             → returns bytes + source_cid
        3. compute = registry.functions[fn].executor(fetched_bytes, args)
        4. fact = PrimaryFact { cell, band, tslot, value: compute, sources: [...source_cids],
                                derivation: {fn_key, args}, schema_cid, signer, signed_at }
        5. attestation = Attestation { facts: [fact], batch_root, attester, registry_cid,
                                       schema_cid, signature }
        6. storage.put_attestation(attestation)
             ├── cache.put_many([fact])    (Hot tier, persists)
             └── log.append(attestation)   (Merkle log, fsync before return)
        7. return fact + receipt(cost, was_cached: false, source_freshness: now-captured_at)
```

Key properties:

- **Idempotent.** A second agent recalling the same (cell, band, tslot) gets a cache hit and the same `fact_cid` — the attestation is not re-created.
- **Deterministic.** Two responders that fetch the same upstream source bytes (via different mirrors / providers) produce byte-identical `value` and therefore the same `fact_cid`. Source dedupe across responders is automatic at the CID layer.
- **Cost-aware.** Cache miss returns an `Receipt.cost` with `was_cached: false`, `source_freshness_s: <age of upstream capture>`, and a higher `credits` charge. Agents see lazy fetch *as cost*, not as failure.
- **Shareable.** Attestations are content-addressed; an L2 federation node can publish its newly-materialized facts to a gossip channel, populating peer caches without re-fetching upstream.
- **Backstop-able.** A node that never sees the relevant upstream provider can still serve a cell *if* a peer attestation for it exists — the protocol is "eventually coherent" over the federation.

**Workload shaping.** "Bootstrap" becomes "warm the cache by pre-recalling high-probability cells" — exactly the same `recall()` code path, just driven by an offline workload (popular cities, recent news regions, agricultural zones). The agri 56-farm cubes are useful **as reference test fixtures** (golden CIDs against which any responder validates), not as the bootstrap corpus.

Throughput targets:

- Cache hit: p50 < 5 ms, p99 < 50 ms (sled read + signature verify).
- Cache miss with a single upstream: p50 < 2 s, p99 < 30 s (depends on upstream provider).
- Bootstrap warm-up (concurrent miss workload): ≥ 1×10³ new facts/sec/node, gated by upstream rate limits.

---

## §18 TS → Rust migration plan

The repository currently contains a v1 TypeScript implementation (`src/`, `server/index.ts`, `tests/`). Migration is staged so the TS layer never silently rots:

1. **Phase A — Spec freeze.** v0.0.2-draft (this document) ratifies the new band+codec+fact decisions.
2. **Phase B — Rust core.** `crates/emem-core`, `emem-codec`, `emem-fact`, `emem-fetch`, `emem-cache`, `emem-storage` ship with golden test vectors. The TS `src/lib/bands.ts` becomes a thin loader over the same `data/bands-v0.json` manifest the Rust core consumes — both implementations validate against the same manifest CID; neither hand-edits the band table.
3. **Phase C — TS clients port.** `tests/bands.test.ts`, `tests/geotessera.test.ts`, `tests/providers.test.ts` are ported into `spec/test_vectors/` so they keep gating CI against both implementations.
4. **Phase D — Server cutover.** `server/index.ts` is replaced by `crates/emem-cli serve`; the TS dev playground in `src/App.tsx` is repointed at the Rust server's REST adapter.
5. **Phase E — SDK rebuild.** `sdks/emem-ts` is published as a thin wrapper over the Rust REST adapter; `sdks/emem-py` ships ctypes bindings to `emem-core`.

The TS code is not deleted in v0.0.2 — it remains the developer playground until the Rust REST adapter is at parity.

---

## §19 Test vectors

`spec/test_vectors/` is the conformance gate. Every test vector is a JSON file with a normative schema:

```json
{
  "id":       "cell.cell64.roundtrip.0001",
  "kind":     "cell64",
  "spec":     "v0.0.2",
  "input":    { "lat": 12.9716, "lng": 77.5946, "res": 13 },
  "expected": { "cell64": "ento.bria.calo.tris", "h3_equivalent": "8d2a1072b59afff" },
  "notes":    "Bengaluru anchor; verifies round-trip and H3 backend equivalence."
}
```

Vector kinds:

- `cbor` — canonical CBOR encoding of a Fact/Attestation/Receipt
- `cid` — fact CID computation
- `sig` — ed25519 attestation signature
- `cell64` — cell encode/decode round-trip + locality property
- `tslot` — time-slot encode/decode
- `vec64` — vector address derivation
- `claim_eval` — claim evaluation against a fact bundle
- `derivation` — function registry entry produces expected output

A conforming implementation MUST pass all vectors marked `level: "L0"` for L0 conformance, plus `L1` for L1, etc.

---

## §20 Revolutionary primitives — agent-native surface

The eight primitives below distinguish emem from a "less-anthropocentric geospatial API" and make it qualitatively useful inside an agent loop. v0 specifies (1)–(5); (6)–(8) are scheduled for v0.1.

### 20.1 Vector-as-address (v0)

`emem:vec/<vec64>` is a first-class dereferenceable address. An agent that has a query embedding (from text, image, or another fact) can address state by similarity in a single primitive call. No external vector store, no separate `find_similar` orchestration. See §3.4.

### 20.2 Derivative facts (v0)

`DerivativeFact` is a fact-of-facts: content-addressed, signed, walkable. An agent asking for "NDVI change over July" does not subtract two values — it reads a single derivative fact whose `parents: [cid_a, cid_b]` and `op: "delta"` make the derivation auditable. The fact is itself attestable, so derivative facts dedupe across agents the same way primary facts do. See §5.1.

### 20.3 Negative facts (v0)

`NegativeFact` carries `reason_cid` — the CID of evidence (typically a sensor scene CID) that confirmed the absence. An agent gets a typed answer to "is there water at this cell?" — `present | absent (reason: <scene>) | unknown` — instead of three-valued ambiguity over `null`. See §5.1.

### 20.4 Cost/latency self-declaration (v0)

Every Receipt carries `cost: { credits, latency_p50_ms, latency_p99_ms, source_freshness_s, was_cached }`. Agent planners build empirical cost models without a separate metering API and can route around slow or stale endpoints. See §7.

### 20.5 Schema-CID self-description (v0)

Every Fact, Attestation, and Receipt carries `schema_cid` — the content hash of the CDDL fragment it conforms to. Fresh agents that encounter an unknown schema dereference once and cache forever; the protocol cannot silently mutate behind an agent's back. Eliminates the entire class of "did the API change?" failures. See §5.1, §7, §10.

### 20.6 Intent-routed primitive (v0; planner is heuristic in v0, learned in v0.1)

```cddl
Intent =
    WhereIs       = { type: "where_is",       description: text }
  / WhatIsHere    = { type: "what_is_here",   cell: text }
  / IsLike        = { type: "is_like",        a: text, b: text }
  / DidChange     = { type: "did_change",     cell: text, band: text, window: [uint, uint] }
  / FindLike      = { type: "find_like",      key: text, k: ? uint, filter: ? Claim }
  / Confirm       = { type: "confirm",        claim: Claim, cell: text }
```

`emem.intent(intent)` returns `{ plan: [ToolCall], cost_estimate: Cost }`. The agent executes the plan or asks the protocol to execute it. Closes the gap between *what the agent wants* and *which primitive to call*.

### 20.7 Progressive-refinement SSE (v0)

`recall` and `query_region` over SSE MUST stream from coarsest resolution to finest (§9.3). Agents can act on res-9 state immediately, refine to res-13 in subsequent chunks. Matches the cheap-first / refine-on-demand reasoning pattern.

### 20.8 Shared planner traces (v0.1)

When an agent satisfies an intent via emem, the resolved plan + tool outputs may be attested as a `PlannerTrace` — a fact whose CID is content-addressed by `(intent_canonical_cbor, registry_cid)`. The next agent submitting the same intent gets the trace in O(1) and skips planning. Agents teaching agents.

---

## §21 Open questions — resolution log

All seven OQs from v0.0.1 are resolved in this draft.

| # | Question | Resolution |
|---|---|---|
| OQ-1 | cell64 alphabet source | Empirical intersection of cl100k/o200k/llama-3/claude tokenizers; Hilbert-ordered for spatial locality. Implemented by `tools/measure_alphabet.py`; pinned in `crates/emem-codec/data/cell64-alphabet-v0.bin`. |
| OQ-2 | gazetteer authority | **Dropped.** No gazetteer in v0. Agent-derived region naming deferred to v0.2; will not block ratification. |
| OQ-3 | function registry governance | IETF-style RFC + reference Rust impl + golden test vectors required at merge. Registry is content-addressed (CID, not URL). |
| OQ-4 | on-chain anchoring | **Base L2** (cheapest finality, EVM-compatible). Trait surface activated v2.0; on-chain writes deferred to v2.5. |
| OQ-5 | stake currency | Native off-chain protocol credit ledger with redemption hook. Avoids token-regulatory exposure through v2.0/v2.5; trait-swappable. |
| OQ-6 | licence | SPEC: **CC-BY-SA-4.0** · Rust ref impl: **Apache-2.0** · SDKs (py/ts): **MIT**. |
| OQ-7 | vision band reproducibility | Vision bands admissible at L2 only. Source.cid of model checkpoint MUST be present. L0/L1 nodes do not serve vision bands. |

New open questions for v0.0.3:

| # | Question | Status |
|---|---|---|
| OQ-8 | tslot epoch handling for pre-2026 historical data | Proposed: signed offset (negative tslot for pre-epoch); revisit when first historical-archive ingest lands. |
| OQ-9 | lcv-1 learned taxonomy methodology | Proposed: HDBSCAN over (AlphaEarth-9yr ⊕ S2-monthly ⊕ Köppen ⊕ ecoregions) at res-9 cell centroids. |
| OQ-10 | derivative fact composition limits | Proposed: max derivative depth = 4 to keep verification cost bounded; revisit after derivative-of-derivative use cases emerge. |
| OQ-11 | intent grammar extensibility | Proposed: registry-style; new intent types ship under semver; v0.1 introduces a learned planner that can dispatch arbitrary structured intents. |

---

## References

[1] Mapbox MCP Server. Mapbox Blog, 2026. https://www.mapbox.com/blog/introducing-the-mapbox-model-context-protocol-mcp-server
[2] Google Maps Platform — agentic experiences. Google, 2026. https://mapsplatform.google.com/resources/blog/powering-the-next-era-of-agentic-experiences-announcing-new-grounding-capabilities/
[3] CARTO MCP Server. CARTO Blog, 2026. https://carto.com/blog/carto-mcp-server-turn-your-ai-agents-into-geospatial-experts/
[4] Janowicz et al. *The KnowWhereGraph: A Large-Scale Geo-Knowledge Graph for Interdisciplinary Knowledge Discovery and Geo-Enrichment.* arXiv:2502.13874, 2025.
[5] Brown et al. *AlphaEarth Foundations: An embedding field model for accurate and efficient global mapping from sparse label data.* arXiv:2507.22291, 2025.
[6] Google DeepMind. AlphaEarth Foundations announcement. 2025. https://deepmind.google/blog/alphaearth-foundations-helps-map-our-planet-in-unprecedented-detail/
[7] Clay Foundation. *An Open Foundation Model for Earth.* Development Seed, 2024. https://developmentseed.org/projects/clay/
[8] *Prithvi-EO-2.0: A Versatile Multi-Temporal Foundation Model for Earth Observation Applications.* arXiv:2412.02732, 2024.
[9] Ma et al. *Harvesting AlphaEarth: Benchmarking the Geospatial Foundation Model for Agricultural Downstream Tasks.* arXiv:2601.00857, 2026.
[10] Bloomberry. *We analyzed 1400 MCP servers — here's what we learned.* 2026. https://bloomberry.com/blog/we-analyzed-1400-mcp-servers-heres-what-we-learned/
[11] Lagrange Labs. *DeepProve-1: The First zkML System to Prove a Full LLM Inference.* 2026. https://lagrange.dev/blog/deepprove-1
[12] IPLD Foundation. *IPLD: 2025 In Review.* IPFS Foundation, 2026. https://ipfsfoundation.org/ipld-2025-in-review/
[13] Uber. *H3: Hexagonal Hierarchical Geospatial Indexing System.* https://h3geo.org/
[14] *hex2vec: Context-Aware Embedding H3 Hexagons with OpenStreetMap Tags.* arXiv:2111.00970.
[15] Vid201 et al. *zk-OPML: Using zero-knowledge proofs to optimize OPML.* J. King Saud Univ. CIS, 2026.

---

*End of emem Protocol Specification v0.0.2-draft.*
