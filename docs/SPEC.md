# emem Protocol — Specification v0.0.4

> Status: stable · 2026-05-06 · Editor: Vortx-AI Private Limited (avijeet@vortx.ai)
> Supersedes: `docs/SPEC.md@v0.0.3`
> Hosted responder: https://emem.dev · Privacy: https://emem.dev/privacy · Terms: https://emem.dev/terms · Support: https://emem.dev/support
> Conformance terms (MUST, SHOULD, MAY) follow [RFC 2119], [RFC 8174].
>
> Changes vs v0.0.3:
> - Foundation embedding stack reflects what the reference responder
>   actually serves: `geotessera` (Tessera v1, Cambridge, 128-D, vintage
>   2024) [TESSERA], `prithvi_eo2` (NASA/IBM Prithvi-EO-2.0-300M-TL,
>   1024-D) [PRITHVI], `galileo_base_v1` (NASA Harvest Galileo Base,
>   768-D) [GALILEO]. AlphaEarth Foundations [ALPHAEARTH] remains
>   informative only; DeepMind has not released open weights and the
>   per-pull GEE mirror requires authenticated access incompatible
>   with the L0/L1 read path.
> - Honest declaration of active grid: the reference responder serves
>   a 21×22-bit `cell64-geo` quantisation (~9.55 m square at the
>   equator), not the spec-target aperture-7 hex DGGS at res-13
>   (3.41 m hex). Both are documented; only the former is wired.
> - References split into Normative and Informative, with explicit
>   citations for every upstream data source the reference build
>   reads from.

## Abstract

**emem** is an open, content-addressed, agent-native protocol for representing, exchanging, and verifying claims about places on Earth. It is engineered exclusively for AI-agent consumption — it does not retrofit human cartography conventions, gazetteer norms, or browser-era APIs. emem operates as a **global lazy memory**: agents recall `(cell, band, tslot)` triples; the protocol either returns a cached fact or fetches the canonical upstream sources, computes the band value, attests, caches forever, and returns. Coverage is the whole Earth, not a precomputed corpus.

The protocol defines: (a) a hierarchical cell tessellation (`emem cells`) and a token-economical, locality-preserving, self-decoding cell codec (`cell64`); (b) an epoch-relative integer temporal grid (`tslot`) replacing ISO 8601 in the canonical channel; (c) a vector-as-address scheme (`vec64`) that makes embedding space directly dereferenceable; (d) a 1792-dimensional band ontology, published as a **content-addressed manifest** (not a hardcoded constant), that combines three open-weight geospatial foundation embeddings — Tessera v1 [TESSERA], Prithvi-EO-2.0 [PRITHVI], Galileo [GALILEO] — with raw Sentinel-1 [S1] / Sentinel-2 [S2] / Landsat / MODIS [MODIS] reflectance, terrain (Cop-DEM [COPDEM], GMRT [GMRT]), surface water (JRC GSW [JRC-GSW]), forest change (Hansen GFC [HANSEN]), land cover (ESA WorldCover [WORLDCOVER]), climate reanalysis (ERA5 [ERA5], NASA POWER [POWER]), nowcast weather (MET Norway [METNO]), atmospheric composition (Open-Meteo CAMS [OPENMETEO-CAMS]), marine (Open-Meteo Marine [OPENMETEO-MARINE]), soil (SoilGrids 2.0 [SOILGRIDS]), and human geography (Overture Maps Foundation [OVERTURE]); each band carries explicit provenance, tempo, and privacy class; (e) a deterministic content-addressed fact format `CID = blake3(canonical_cbor(fact))` supporting **primary**, **derivative**, and **negative** fact variants, each carrying a `schema_cid` for self-description; (f) signed attestation envelopes and proof-carrying receipts with cost / latency self-declaration; (g) a content-addressed function registry plus a swappable source-connector manifest that maps abstract source schemes to fetch templates (operators add mirrors, auth, regions without touching the protocol); and (h) MCP-first transport [MCP] with a normative tool inventory and self-describing introspection tools so agents discover the active manifests at runtime, with REST as the compatibility adapter the reference build also serves natively. IPLD blocks [IPLD] are used as the canonical CID format; every `fact_cid` is a valid IPLD CID for downstream interop, but no IPLD/IPFS retrieval client is wired in 0.0.x. The reference implementation is a Rust crate workspace.

The protocol is built on five constraints unique to agent consumption — **token economy, deterministic re-execution, append-only persistence, honest absence, zero-trust verification** — plus three architectural commitments that move the surface beyond what existing geospatial protocols offer: **lazy global materialization, vector-as-address, intent-routed planning**.

---

## §0 Quickstart for agents

Five things an agent needs to call any primitive. (Verbose explanation in §3, §11.)

1. **Discover the active manifests.** Call `emem.manifests` (or `GET /.well-known/emem.json`). You get back CIDs for the `bands`, `functions`, `sources`, `schema`, and `lcv1` registries. Cache them keyed by CID; they never change for a given CID.
2. **Address.** A point on Earth at a moment in time is `(cell, band, tslot)`:
   - `cell` is `cell64(lat, lng)` — 4 dot-separated bigrams, ≤4 tokens. Encodes a fixed ~9.54 m × 9.55 m square (21-bit latitude × 22-bit longitude quantisation, matching Sentinel-1/Sentinel-2 native pixel pitch). The cell64 bit layout reserves a resolution-tag field for future hierarchical refinement targeting H3-equivalent res-13 (~3.4 m) cells in v0.1; in 0.0.x all cells use the single active resolution. Reported live by `GET /v1/grid_info`.
   - `band` is one of the keys in the bands manifest (e.g. `"geotessera"`, `"prithvi_eo2"`, `"galileo_base_v1"`, `"indices"`, `"esa_worldcover.lc_2021"`, `"weather.temperature_2m"`).
   - `tslot` is an unsigned integer offset from the emem epoch (2026-01-01T00:00:00Z), in units determined by the band's tempo (year / month / day / hour). Static bands always use `tslot = 0`.
3. **Recall.** Call `emem.recall(cell, [bands], tslot?)`. You get back `{facts: [Fact], receipt: Receipt}`. The Receipt carries `cost.was_cached`, `cost.source_freshness_s`, `cost.credits` — read these to choose your next move (cheap cache hit vs. expensive lazy fetch).
4. **Verify, don't trust.** Every Fact carries `signer`, `signature`, `schema_cid`. Verify the signature against `attesters[].key` from `emem.manifests`. Dereference `schema_cid` once and cache; subsequent facts under the same CID are guaranteed to parse.
5. **Compose, address by similarity.** When you need "places like this place", use `emem.find_similar(key=<cell64 or vec64>, k=10)` — embedding space is part of the address space. When you don't know which primitive to call, send `emem.intent({type: ..., ...})` and the protocol returns a Plan you can execute (or ask the protocol to execute for you).

That is the entire surface for L0 reads. Verify and attest are §6–§8; the rest of the spec is reference material. Privacy and data protection posture (GDPR, UK GDPR, DPDP, CCPA) is §13; the canonical hosted instance publishes its policy at `/privacy` and its terms at `/terms`.

---

## §1 Motivation & gap

### 1.1 Why a new protocol

Existing options for grounding AI agents in spatial reality are inadequate in three structural ways:

**Transactional location APIs** (Mapbox MCP [MAPBOX-MCP], Google Maps Grounding Lite [GMAPS-AGENTIC], CARTO MCP [CARTO-MCP]) expose geocoding, routing, search. They are designed for *retrieval of places*, not *recall of facts about places*. They have no concept of an immutable, citeable "what was true at this cell on this date" unit, and no concept of fact-level content addressing.

**Geospatial knowledge graphs** (KnowWhereGraph [KNOWWHERE], 12B triples, GeoSPARQL) demonstrate the value of pre-integrated cross-domain spatial data. They are read-only, schema-rigid, and SPARQL-only, which is too high-friction for the agent inside-loop, and they have no cryptographic commitment to facts.

**Spatial foundation models** (Tessera v1 [TESSERA], Prithvi-EO-2.0 [PRITHVI], Galileo [GALILEO], Clay [CLAY], AlphaEarth Foundations [ALPHAEARTH], SatMAE) produce dense embeddings that are the right substrate for retrieval. Recent benchmarking [HARVEST] documents three concrete limits when agents try to use one foundation embedding alone: (i) limited spatial transferability, (ii) limited time sensitivity, (iii) low interpretability. The reference responder mitigates all three by carrying *multiple* foundation embeddings as separate bands (Tessera, Prithvi, Galileo) alongside raw Sentinel-1 / Sentinel-2 reflectance, terrain, climate, and soil — each addressable by name, each independently signed, each with declared provenance. AlphaEarth Foundations is cited as comparable prior art; it is not in the active band set because DeepMind has not released open weights and the GEE-hosted embedding requires per-pull authentication incompatible with anonymous L0/L1 reads.

The gap nobody fills: **a cryptographically verifiable, token-economical, agent-contributable, vector-addressable, lazily-materialized global memory layer for spatial facts.** That is the emem protocol. Coverage is the whole Earth at sub-meter scale, but storage cost scales with *demand*, not with *area* — facts are computed and cached only when an agent asks for them.

### 1.2 Why now

- **MCP momentum**: 1,412 servers as of Feb 2026, 232% growth in 6 months, 97M monthly downloads [BLOOMBERRY-MCP]. The agent ecosystem has converged on a discovery + invocation surface; new protocols can ride that rail.
- **zkML reaches production**: Lagrange DeepProve-1 cryptographically proved a full LLAMA inference [DEEPPROVE]; cost-of-proof is forecast to drop below $0.01 per call in 2026, the threshold at which proof-carrying inference becomes default.
- **Content-addressed scientific data is mature**: IPLD [IPLD] is the substrate for content-addressed blobs. The reference responder in 0.0.x ships a local sled hot cache + append-only Merkle log only; no IPFS / IPLD / Filecoin client is wired in this release. Cold-tier replication is a v0.1 design problem, not a current feature.
- **Foundation embeddings have stabilized**: Tessera v1 [TESSERA] shipped a 128-D global embedding under MIT in late 2025; Prithvi-EO-2.0-300M [PRITHVI] and Galileo Base [GALILEO] released open weights under Apache-2.0 / MIT in 2024-2025. The 1792-D fusion layout is the slot-allocation contract; specific foundation embeddings occupy slots and can be swapped under semver as upstream weights evolve.

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

### 3.1 emem cells — hierarchical tessellation (active and target)

The protocol defines two grids: a flat lat/lng quantisation that the reference responder serves today, and an aperture-7 hex DGGS that is the target for v0.1. Both are normative; the active grid is reported live by `GET /v1/grid_info` so agents know which one is wired without reading source.

**Active grid (v0.0.x): `cell64-geo-21x22`.** A square-at-equator quantisation with 21 bits of latitude and 22 bits of longitude, encoded as four base-1024 bigrams joined by `.` (e.g. `defi.zb592.nemu.zEvE`). Pitch is approximately 9.54 m × 9.55 m at the equator, matching Sentinel-1 / Sentinel-2 native pixel pitch. Longitude pitch narrows with `cos(lat)` so cells become taller than wide above the equator. This is what every fact CID in 0.0.x is computed against.

**Target grid (v0.1): aperture-7 hex DGGS at res-13.** The geometry below is the migration target. It is mathematically identical in structure to several existing icosahedral DGGS — this is intentional, so that high-quality math libraries (notably Uber H3 [H3]) can serve as permitted backends — but the addressing, encoding, naming, and operator algebra are normative to emem.

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

Hexagons over squares because (a) all neighbors are equidistant (path / navigation primitives are clean) and (b) hex2vec [HEX2VEC] showed hexagonal embeddings have lower sampling artifacts than rectangular grids when used as ML inputs, which matters because agents will request cell vectors as model inputs at scale.

**Target Cell ID** is a 64-bit integer with the bit layout (used by the target hex DGGS; the active `cell64-geo-21x22` codec packs lat/lng bits directly and does not use this layout):

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

Total: 1 + 4 + 3 + 4 + 7 + 45 = 64 bits. Resolution 0 is the bare base cell with no path digits; resolution `r` consumes the first `r` of the 15 path-digit slots. Reference implementations MAY use Uber H3 ≥ 4.0 [H3] as a backend if and only if their outputs pass the `cell.*` test vectors (§19). H3 is not normatively cited in the wire format.

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
| `slow` | 1 year | Tessera, Prithvi, Galileo, soil |
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

The v0 manifest contains 34 named bands and is shipped at `crates/emem-core/data/bands-v0.json`. The physical layout (key, offset, dims) is the source-of-truth for what occupies each slot in the 1792-D voxel and in what order; cubes computed under one manifest CID decode identically under any responder pinning the same CID. Family classification is editorial (not load-bearing); tempo and privacy class are normative per band. Live count and per-band materialiser status are reported by `GET /v1/bands`.

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
| foundation | 704 | `geotessera` [TESSERA] 128-D live (Tessera v1, Cambridge, vintage 2024; int8 + f32-scale upstream, decoded f32 over the wire); `prithvi_eo2` [PRITHVI] 1024-D live (Prithvi-EO-2.0-300M-TL, NASA/IBM, HLS V2 6-band ViT-L); `galileo_base_v1` [GALILEO] 768-D live (Galileo Base, NASA Harvest, S2 + masked-zero S1/DEM/climate); `alphaearth` [ALPHAEARTH] slot reserved at 576 = 9 yrs × 64-D (DeepMind has not released open weights; populated only when the per-cell GEE embedding becomes available without per-pull authentication) | slow | public |
| optical | 13 | `sentinel2_raw` (10) [S2], `indices` (3) | fast | public |
| radar | 2 | `sentinel1_raw` (2) [S1] | fast | public |
| terrain | 43 | `dem` (3), `terrain_derived` (32), `copdem30m` (8) [COPDEM]; bathymetry from `gmrt.topobathy_mean` [GMRT] | static | public |
| climate | 56 | `weather` (MET Norway nowcast) [METNO]; `era5` (Open-Meteo ECMWF reanalysis) [ERA5]; `power` (NASA POWER daily) [POWER]; `cams` (Open-Meteo CAMS air quality) [OPENMETEO-CAMS]; `marine` (Open-Meteo Marine) [OPENMETEO-MARINE]; `koppen` static climate classes; `terraclimate` decadal | static-to-medium | public |
| soil | 20 | `soilgrids` (ISRIC SoilGrids 2.0) [SOILGRIDS] | slow | public |
| vegetation | 160 | `temporal_diff` (64), `phenology` (32), `multiscale` (64); MODIS-derived inputs include `modis.ndvi_mean`, `modis.lst_day_8day`, `modis.et_8day`, `modis.gpp_8day`, `modis.lai_8day`, `modis.burned_area_monthly` [MODIS] | slow-to-medium | public |
| landcover | 44 | `esa_worldcover.lc_2021` (8) [WORLDCOVER]; `hansen.{tree_cover_2000, loss_year, gain}` (12) [HANSEN]; `mangrove` (4); `ecoregions` (20) | slow | public |
| water | 16 | `surface_water.recurrence` (JRC GSW v1.4, Landsat 1984-2021) [JRC-GSW]; `ocean_chl` (4) | fast | public |
| human | 38 | `overture.*` [OVERTURE], `nightlights` (8), `ghsl` (8), `population` (8), `protected` (4), `admin` (10) | static-to-fast | **aggregate_only at res ≥ 11** |
| vision | 384 | `sam3_visual` (192), `qwen_visual` (192) | slow | **L2-only, model-CID required** |
| topology | 32 | `topology` | static | public |
| encoding | 160 | `spatial_fourier` (96), `temporal_fourier` (64) | static-to-fast | public |
| reserved | 120 | future sensors | n/a | n/a |

**Privacy classes** (§13 normative):

- `public` — unrestricted at any resolution
- `aggregate_only at res ≥ N` — implementations MUST NOT serve at resolution finer than N; queries at finer res return aggregated values with `privacy_snapped: true` flag
- `L2-only` — admissible only at conformance level L2, requires `Source.cid` of model checkpoint
- `prohibited` — reserved; MUST NOT be served

The full normative manifest lives at `crates/emem-core/data/bands-v0.json`. Loaded and validated by `emem_core::bands::BandRegistry`. CID derivation: `base32(blake3(canonical_cbor(manifest)))[:32]`.

**lcv-1 land cover taxonomy.** The `landcover` band carries an 8-dim ESA WorldCover [WORLDCOVER] one-hot. The `lcv-1` *taxonomy* (64 leaves, 8 families) is a richer leaf index served as a separate Fact value (`band: "landcover.lcv1_leaf"`) when requested; it is also content-addressed via its own manifest. v0 ships placeholder names (`lcv-1.f0.l0` … `lcv-1.f7.l7`); v0.1 swaps in learned cluster centroids derived from a fusion of Tessera + Sentinel-2 monthly composites + Köppen climate + ecoregions at res-9 cell centroids (HDBSCAN), at which point each leaf gains a canonical 1792-D centroid embedding so that `landcover:lcv-1.43` is *also* a vector.

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
  scheme:      text,                  ; "sentinel2.l2a", "tessera.v1", "prithvi.eo2", "copernicus.dem.30m", ...
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
  stake:        ? uint,               ; passthrough only — see §6.3
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

### 6.3 Stake field — passthrough only

The `Attestation.stake` field is reserved space for an out-of-band economic commitment (bonded reputation, slashable deposit on a sidechain, x402 / LSP escrow id, etc.). The reference responder **stores it verbatim** in the merkle log and on the attester's record. The protocol itself does not mint, transfer, escrow, or slash anything based on this field — there is no protocol-issued credit, no on-chain anchor, no challenge-driven slashing logic in the 0.0.x reference build.

Operators who want to layer a payment / reputation / slashing economy on top can do so via x402, LSP, or any other rail and use this field to surface the commitment to other agents reading from `/v1/contributors`.

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
  credits:             uint,          ; reserved — see note below
  latency_p50_ms:      uint,          ; observed latency, this primitive class
  latency_p99_ms:      uint,
  source_freshness_s:  uint,          ; age of stalest source, seconds
  was_cached:          bool,
}
```

Receipts are byte-stable: two responders serving the same fact under the same protocol version produce signatures that differ only in `responder`, `responder_key_epoch`, `signature`, `served_at`, and `cost`. The agent can hand the receipt to its caller as cryptographic evidence — and the caller can independently re-verify against the protocol's published attester pubkeys.

**Why `cost` is in the receipt.** Agent planners need to make local decisions about which primitives to call. Surfacing real cost+latency+freshness in the receipt lets the agent build an empirical model of primitive costs without a separate metering API.

**What's actually populated in 0.0.x.** `was_cached` and `source_freshness_s` are real. `latency_p50_ms` and `latency_p99_ms` both echo the observed `elapsed_ms` of the single served call (the histogram naming anticipates a future per-primitive aggregate). `credits` is **always `0`** — no protocol credit ledger exists in the reference build (see §6.3 for the matching position on `Attestation.stake`). Agents should not branch on `credits`; treat it as a wire-stable placeholder.

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

For `mode=resolve` and `mode=zk`, the protocol uses an **optimistic ML** workflow modeled on zk-OPML [ZK-OPML]: the responder produces a result with cheap Merkle commitments to intermediate states; challengers may dispute by demanding ZKP for any operator. This achieves proof-carrying inference at near-opML cost in the common case, and zkML cost only on dispute.

### 8.4 Challenge — not implemented

The wire format reserves `challenge(attestation_id, counter_evidence)` as a future primitive for disputing a fact: the counter-attestation would carry a conflicting value plus sources, the responder would re-execute from sources, and a refuted attestation would be marked superseded.

The reference build in 0.0.x does **not** implement `challenge` — there is no `/v1/challenge` endpoint, no slashing logic, and no responder-side re-execution path. Disputes today happen out-of-band: a contributor submits a fresh attestation with the corrected value and the older fact is superseded by the canonical (cell × band × tslot) → cid index.

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
  "protocol":  "emem/v0.0.3",
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

MCP is the **primary** transport. REST is the compatibility adapter the reference build serves natively (`/v1/*`). gRPC and an IPLD storage adapter are reserved in the design — no gRPC server, no IPLD client are wired in 0.0.x; every fact CID is a valid IPLD CID for downstream interop, but the responder does not fetch from IPFS.

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
| **L2** | L1 + `attest` (third-party). `challenge`, `verify(mode=zk)`, on-chain anchoring, and vision-band attestation are reserved in the wire format but **not implemented in 0.0.x** (see §6.3, §8.4). | open contributor protocol |

A conforming implementation MUST publish its level in `/.well-known/emem.json` and MUST enforce per-band privacy class (§13) at every level.

---

## §13 Privacy and data protection

### 13.1 Per-band privacy class (protocol level)

Hex DGGS res-13 cells are ~3.4 m on a side. Some bands at that resolution are PII-loaded: population at building scale, nightlights at residence scale, future thermal at vehicle scale. The protocol enforces privacy at the band declaration level:

- **`public`** — unrestricted; default.
- **`aggregate_only at res ≥ N`** — implementations MUST NOT serve at resolution finer than N. Queries at finer resolution receive aggregated values from the res-N parent, with `privacy_snapped: true` and the parent cell ID in the response.
- **`L2-only`** — admissible only at conformance level L2; requires `Source.cid` of the model checkpoint; not available on hosted L0/L1 nodes.
- **`prohibited`** — reserved for future bands the protocol has chosen not to expose; serving such bands is a conformance violation.

The privacy class is part of the band registry and therefore content-addressed via `manifests.coverage_cid`. A privacy reclassification is a registry version bump.

### 13.2 What the canonical channel does and does not contain

The wire-canonical channel carries `(cell, band, tslot, value, source_cids, signer, signature)`. None of those fields is a personal identifier. `cell` is a quantised lat/lng anchored to public Earth observation pixels; `signer` is the responder's own ed25519 pubkey; `source_cids` reference public datasets. The canonical fact format **MUST NOT** include user identifiers, IP addresses, free-text questions, or any data that could re-identify the requesting party.

Recall, compare, find_similar, diff, trajectory, backfill, and verify_receipt are stateless from the user's perspective: they consume a `(cell, band[], tslot?)` triple and return facts plus a receipt. The protocol prescribes no user-account, session, cookie, or identifier mechanism.

### 13.3 Operator-side processing and lawful basis

The hosted responder at `https://emem.dev` is operated by Vortx AI Private Limited (India). Operator-level processing (request logs, geocoder query traces, abuse mitigation) is governed by the operator's published privacy policy at `/privacy` and terms at `/terms`. Self-hosted deployments are out of scope and are governed by their own operator's policy.

Lawful bases under EU/UK GDPR Art. 6 for the canonical hosted instance:

- **Art. 6(1)(f) legitimate interests** for serving public Earth observation facts, since the protocol returns answers about public places from public datasets and processes no personal data in the canonical channel.
- **Art. 6(1)(f) legitimate interests** for operator-side request logs (timestamp, blake3-hashed truncated IP, user-agent, path, GET query string, status, duration) for the limited purposes of service health, capacity planning, and abuse mitigation. Retention is enforced at 30 days by `MaxRetentionSec=30day` on systemd journald (see `ops/systemd/journald-30day-retention.conf` in the reference deployment). POST bodies are not captured.
- **Art. 6(1)(b) contract** for any future authenticated paid tier, when offered.

No `Art. 9` special-category data is processed.

### 13.4 GDPR / UK GDPR / DPDP / CCPA compliance

Implementations targeting EU/UK markets MUST conform with [GDPR] and [UK-GDPR]. Implementations targeting Indian markets MUST conform with [DPDP-2023]. Implementations targeting California consumers MUST conform with [CCPA-CPRA]. The hosted responder honours data subject / data principal rights under all four regimes:

| Right (GDPR Art.) | What the responder does |
|---|---|
| Access (Art. 15) | Operator returns any operational log line tied to a controllable IP within 30 days of request to `avijeet@vortx.ai`. |
| Rectification (Art. 16) | Same channel; corrections applied to retained logs. |
| Erasure (Art. 17) | Log lines purged ahead of the 30-day rotation on request. Signed attestations submitted to `/v1/attest` are content-addressed and cannot be retracted (this is the cryptographic property of the protocol, disclosed in `/terms` §4); the protocol does not allow erasure of public ledger entries. |
| Restriction (Art. 18) | Operator stops processing operational metadata associated with the IP for any purpose beyond fulfilling the request. |
| Portability (Art. 20) | Operational logs are exported in JSONL on request. Public attestations are already content-addressed and self-portable. |
| Object (Art. 21) | Same channel; objection honoured for legitimate-interest processing. |
| Lodge a complaint (Art. 77) | EU: local supervisory authority. UK: Information Commissioner's Office (ICO). India: Data Protection Board of India once operational. California: California Privacy Protection Agency. |

DPDP Act 2023 §11–§14 rights map to the GDPR rights above; California CCPA/CPRA rights to access / delete / correct / opt-out of sale or sharing map to the same channel. The responder does not sell or share personal data with third parties for advertising or cross-context behavioural purposes; there is nothing to opt out of in that regard.

### 13.5 Cookies, fingerprints, tracking

The hosted responder sets no cookies of its own. The HTML landing page at `/` (and **only** that page; not `/v1/*`, not `/mcp`, not `/openapi.json`, not the markdown surfaces, not `.well-known/*`) loads Google Analytics 4 (`G-RBLXX5LR9L`) under Consent Mode v2 with default-denied for `ad_storage`, `ad_user_data`, `ad_personalization`, `analytics_storage`, `functionality_storage`, and `personalization_storage`. Under that configuration GA4 emits only cookieless aggregated pings: no `_ga` or `_ga_*` cookies are set, no raw IP is transmitted, no profile is built. `curl -I https://emem.dev/` returns no `Set-Cookie` header. Full disclosure (vendor, measurement ID, consent defaults, transfer basis, opt-out URL) at `/privacy` and machine-readable in `/.well-known/agent-card.json` under `provider.data_protection.third_party_analytics[]`.

No localStorage / IndexedDB entries, no fingerprinting probes, no first-party cookies. Static assets (`/favicon.svg`, `/og-image.svg`) are served from the same origin.

### 13.6 IP handling

Raw client IPs are **not** stored. The access log middleware computes `agent_ip_hash = base32_nopad_lower(blake3(client_ip)[:8])` and stores only that 8-byte hash. The construction is one-way: a stored hash cannot be reverted to a raw IP without rainbow-tabling the IPv4 / IPv6 space. Source: `crates/emem-api-rest/src/lib.rs::hashed_ip`.

### 13.7 Data subject contact and policy

For data-subject-rights enquiries, breach reports, and any other privacy correspondence: **avijeet@vortx.ai**. The canonical privacy policy is `/privacy`; the canonical terms of service are `/terms`; support and security contacts are at `/support` and `/.well-known/security.txt`.

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
  emem-cache/        ; sled hot cache (warm parquet / cold content-addressed tiers reserved in trait, not wired)
  emem-fetch/        ; source-connector framework + dispatcher (HTTPS, GCS — IPLD/IPFS reserved, not wired)
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
- **Cold tier — design pending.** Content-addressed durable storage (IPLD blocks behind CIDs; Filecoin Saturn caching as one option) is on the roadmap but **not implemented in 0.0.x**. The reference build keeps everything in the local sled hot tier and the Merkle log on disk; operators back up by snapshotting the data directory.
- **Append-only Merkle log — segment files of 1 GiB**, format `[u32 LE: cbor_len][cbor_bytes][32 bytes: blake3(cbor_bytes)]`, trailing per-segment hash, fsync MUST happen before receipt is signed.
- **Backup / replication.** Sealed segment files can be copied to any object store (S3, GCS, B2, …) — restore = pull segments in order, verify trailing hash per segment, replay attestations. The IPFS / Filecoin replication path is design-pending and is not gated by this spec.
- **Multi-node.** Operators shard by H3 res-7 parent (≈1.22 km tiles). Cross-shard reads are stitched at the primitive layer; gossip-dedupe between responders is a v0.1 design problem and is **not implemented in 0.0.x**.
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
| `lc.esa@1` | `landcover.class` (`lcv-1`) | ESA WorldCover [WORLDCOVER] | manual 11-class → lcv-1 mapping (`crates/emem-core/data/lcv-mapping-v1.json`) |
| `pp.ghsl@1` | `human.population` | GHSL R2023A | direct read, density per km² |
| `nl.viirs@1` | `human.nightlights` | VIIRS DNB | monthly composite |
| `kg.koppen@1` | `climate.koppen` | Beck et al. 2018 | direct lookup, 32-class |
| `sr.soil@1` | `soil.*` (20 dims) | SoilGrids 2.0 [SOILGRIDS] | direct read at 250 m, bilinear to 10 m |
| `gt.slice@1` | `foundation.geotessera` (128) | Tessera v1 [TESSERA] | direct slice; vintage 2024 only upstream |
| `pr.embed@1` | `foundation.prithvi_eo2` (1024) | Prithvi-EO-2.0 [PRITHVI] | 224×224 HLS V2 6-band chip → ViT-L CLS token |
| `gl.embed@1` | `foundation.galileo_base_v1` (768) | Galileo Base [GALILEO] | 8×8 chip at 30 m equiv, 10 S2 bands; S1/DEM/climate masked-zero |
| `ae.slice@1` | `foundation.alphaearth` (576) | AlphaEarth Foundations [ALPHAEARTH] | RESERVED; not wired in 0.0.x (no open weights) |
| `s2.comp@1` | `optical.sentinel2_raw` (10) | Sentinel-2 L2A [S2] via STAC [STAC] | monthly median composite, 10 bands, ≤ 40% cloud |
| `s1.comp@1` | `radar.sentinel1_raw` (2) | Sentinel-1 GRD [S1] | monthly mean composite, VV + VH |
| `cd.elev@1` | `terrain.copdem30m` | Copernicus DEM 30 m [COPDEM] | direct read, signed Absence over water |
| `gm.topo@1` | `terrain.gmrt` | GMRT [GMRT] | global topo + bathy |
| `mod.lst@1` | `vegetation.modis_lst` | MOD11A2 [MODIS] | 8-day LST day / night, 1 km |
| `mod.ndvi@1` | `vegetation.modis_ndvi` | MOD13Q1 [MODIS] | 16-day NDVI composite |
| `jrc.gsw@1` | `water.surface_water` | JRC GSW v1.4 [JRC-GSW] | flood-recurrence climatology, Landsat 1984-2021 |
| `hgfc.loss@1` | `landcover.forest_change` | Hansen GFC v1.11 2023 [HANSEN] | tree_cover_2000, loss_year, gain |
| `wx.met@1` | `climate.weather` | MET Norway nowcast [METNO] | hourly, 15-min cadence |
| `wx.era5@1` | `climate.era5` | Open-Meteo / ECMWF ERA5 [ERA5] | reanalysis 1940-present, hourly |
| `wx.power@1` | `climate.power` | NASA POWER [POWER] | daily reanalysis (MERRA-2 + GEOS), 1981-present |
| `wx.cams@1` | `climate.cams` | Open-Meteo CAMS [OPENMETEO-CAMS] | air quality, hourly, 2013-08-01-present |
| `wx.marine@1` | `climate.marine` | Open-Meteo Marine [OPENMETEO-MARINE] | ECMWF WAM, hourly, 2022-08-01- |
| `ovt.places@1` | `human.overture` | Overture Maps [OVERTURE] | per-cell aggregates (buildings, places, road length) |
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

## §18 Reference implementation status

The canonical reference implementation is the **Rust workspace** (`crates/*`) at version 0.0.3. It is what `emem.dev` runs and what every conformance test gates against. The TypeScript playground (`src/`, `server/index.ts`, `sdks/emem-ts`) is legacy from the v1 design and is **not** kept in lock-step with the Rust core; an agent that needs to inter-op should read from the Rust REST adapter (`/v1/*`) or use the Python SDK shim (`sdks/emem-py`). The TS playground will be retired once it has no remaining demos that the Rust adapter doesn't already serve.

---

## §19 Test vectors

`spec/test_vectors/` is the conformance gate. Every test vector is a JSON file with a normative schema:

```json
{
  "id":       "cell.cell64.roundtrip.0001",
  "kind":     "cell64",
  "spec":     "v0.0.3",
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
| OQ-4 | on-chain anchoring | **Open.** Reference build does not anchor anywhere; receipt verification is pure ed25519 + Merkle. A v0.1 design will choose one rail (candidate: Base L2 for finality + EVM compat) once a concrete need lands. |
| OQ-5 | stake currency | **Open.** No protocol-issued credits in 0.0.x. The `Attestation.stake` field is a passthrough (§6.3) so external economies (x402, LSP) can layer on top without forcing one in-protocol. |
| OQ-6 | licence | SPEC: **CC-BY-SA-4.0** · Rust ref impl: **Apache-2.0** · SDKs (py/ts): **MIT**. |
| OQ-7 | vision band reproducibility | Vision bands admissible at L2 only. Source.cid of model checkpoint MUST be present. L0/L1 nodes do not serve vision bands. |

Open questions tracked at v0.0.4:

| # | Question | Status |
|---|---|---|
| OQ-8 | tslot epoch handling for pre-2026 historical data | Proposed: signed offset (negative tslot for pre-epoch). Revisit when first historical-archive ingest lands. |
| OQ-9 | lcv-1 learned taxonomy methodology | Proposed: HDBSCAN over (Tessera v1 ⊕ Sentinel-2 monthly composites ⊕ Köppen ⊕ ecoregions) at res-9 cell centroids. AlphaEarth dropped as input (no open weights). |
| OQ-10 | derivative fact composition limits | Proposed: max derivative depth = 4 to keep verification cost bounded. Revisit after derivative-of-derivative use cases emerge. |
| OQ-11 | intent grammar extensibility | Proposed: registry-style; new intent types ship under semver. v0.1 introduces a learned planner that can dispatch arbitrary structured intents. |
| OQ-12 | temporal_recipe canonicalisation | v0.0.3 ships per-algorithm `temporal_recipe { windows[], aggregator }` as additive metadata. Canonical receipt format for `temporal_composition[]` (candidate: Merkle root over per-window fact CIDs) is open. |
| OQ-13 | Photon hosted-instance dependency | v0.0.3 made Photon (komoot.io) the primary live geocoder. Open: self-host a Photon mirror to avoid soft-dependency on the public komoot endpoint? Trade-off is ~6 GB of OSM index storage per instance. |
| OQ-14 | Overture Places [OVERTURE] as third geocoder tier | Reserved slot below Nominatim. Open: add once Overture Places parquet partitions are S3-anonymous-readable for arbitrary `name LIKE` predicates. |
| OQ-15 | Migration from `cell64-geo-21x22` to aperture-7 hex DGGS | The active grid (§3.1) is the flat lat/lng quantisation. The hex DGGS at res-13 remains the spec target. New cell strings under the hex grid will ship under a new mode prefix; existing cell64-geo facts remain valid forever under their current CIDs. Open: cutover gating (test-vector parity vs Uber H3 [H3]; per-band materialiser readiness on hex inputs). |
| OQ-16 | Foundation embedding rotation policy | Three open-weight embeddings now occupy the foundation family (Tessera, Prithvi, Galileo). Open: when does an embedding band retire? Proposal: a band whose upstream model is unmaintained for 24 months is marked `deprecated` in the manifest; CIDs remain verifiable, materialiser stops auto-fetching new vintages. |

---

## §22 References

References are split into Normative (required for interpreting the
spec) and Informative (background and prior art). Citation keys are
stable; URLs are best-effort.

### 22.1 Normative references

Wire format and cryptography:

- [RFC 2119] Bradner, S. *Key words for use in RFCs to Indicate Requirement Levels.* BCP 14, RFC 2119, March 1997. https://www.rfc-editor.org/rfc/rfc2119
- [RFC 8174] Leiba, B. *Ambiguity of Uppercase vs Lowercase in RFC 2119 Key Words.* BCP 14, RFC 8174, May 2017. https://www.rfc-editor.org/rfc/rfc8174
- [RFC 8949] Bormann, C., and Hoffman, P. *Concise Binary Object Representation (CBOR).* STD 94, RFC 8949, December 2020. https://www.rfc-editor.org/rfc/rfc8949
- [RFC 8610] Birkholz, H., Vigano, C., and Bormann, C. *Concise Data Definition Language (CDDL).* RFC 8610, June 2019. https://www.rfc-editor.org/rfc/rfc8610
- [RFC 8032] Josefsson, S., and Liusvaara, I. *Edwards-Curve Digital Signature Algorithm (EdDSA).* RFC 8032, January 2017. https://www.rfc-editor.org/rfc/rfc8032
- [BLAKE3] O'Connor, J., Aumasson, J.-P., Neves, S., and Wilcox-O'Hearn, Z. *BLAKE3: One Function, Fast Everywhere.* 2020. https://github.com/BLAKE3-team/BLAKE3-specs
- [RFC 4648] Josefsson, S. *The Base16, Base32, and Base64 Data Encodings.* RFC 4648, October 2006. https://www.rfc-editor.org/rfc/rfc4648
- [IPLD] Protocol Labs. *IPLD: InterPlanetary Linked Data — Content-Addressed Data Structures.* https://ipld.io/specs/ . CID v1 binary form is RFC-aligned and used as the canonical fact CID format.

Transport and discovery:

- [MCP] Anthropic et al. *Model Context Protocol Specification.* 2024 onward. https://modelcontextprotocol.io/specification
- [A2A] Linux Foundation Agentic AI Foundation. *Agent-to-Agent (A2A) Protocol v0.2 Agent Card.* https://a2a-protocol.org/latest/topics/agent-discovery/
- [AGENTS-MD] Agentic AI Foundation. *AGENTS.md.* https://agents.md/
- [LLMS-TXT] Howard, J. et al. *llms.txt — A proposal to standardise on using an /llms.txt file.* https://llmstxt.org/
- [WELL-KNOWN] Nottingham, M. *Well-Known URIs.* RFC 8615, May 2019. https://www.rfc-editor.org/rfc/rfc8615

Data protection (normative for hosted responders in the named jurisdictions):

- [GDPR] European Parliament and Council. *Regulation (EU) 2016/679 (General Data Protection Regulation).* 27 April 2016. https://eur-lex.europa.eu/eli/reg/2016/679/oj
- [UK-GDPR] UK Information Commissioner's Office. *Guide to the UK General Data Protection Regulation.* https://ico.org.uk/for-organisations/uk-gdpr-guidance-and-resources/
- [DPDP-2023] Government of India. *The Digital Personal Data Protection Act, 2023.* https://www.meity.gov.in/data-protection-framework
- [CCPA-CPRA] California Privacy Protection Agency. *California Consumer Privacy Act, as amended by the California Privacy Rights Act.* https://oag.ca.gov/privacy/ccpa
- [RFC 9116] Foudil, E., and Shafranovich, Y. *A File Format to Aid in Security Vulnerability Disclosure (security.txt).* RFC 9116, April 2022. https://www.rfc-editor.org/rfc/rfc9116

### 22.2 Informative references — upstream data sources

Each band wired in the reference responder pulls from a published
open-data source. Sources are cited here so an agent that wants to
verify a fact's provenance independently has the canonical URL.

Foundation embeddings:

- [TESSERA] Cambridge Centre for Carbon Credits. *Tessera v1: Global 128-D representation derived from Sentinel-2 spectral-temporal manifolds.* 2025. https://www.cambridge-cccc.org/tessera . Live in the reference build under band `geotessera` (vintage 2024). Per-cell delivery is HTTPS range-read against published int8 + per-pixel f32-scale tiles, decoded to f32 over the wire.
- [PRITHVI] Jakubik, J. et al. *Prithvi-EO-2.0: A Versatile Multi-Temporal Foundation Model for Earth Observation Applications.* arXiv:2412.02732, 2024. NASA / IBM, Apache-2.0. Reference build runs the 300M-TL variant locally on CUDA via the GPU sidecar; band `prithvi_eo2`.
- [GALILEO] Tseng, G., Cresswell, J. C., et al. *Galileo: Learning Global and Local Features in Pretrained Remote Sensing Models.* NASA Harvest / Vector Institute, 2025. MIT licence. https://github.com/nasaharvest/galileo . Reference build runs Galileo Base (86.5 M params, 768-D) locally; band `galileo_base_v1`.
- [ALPHAEARTH] Brown, C. F. et al. *AlphaEarth Foundations: An embedding field model for accurate and efficient global mapping from sparse label data.* arXiv:2507.22291, 2025. Google DeepMind. https://deepmind.google/blog/alphaearth-foundations . Cited as comparable prior art; not in the active band set (no open weights, GEE delivery requires per-pull authentication).
- [CLAY] Clay Foundation. *Clay: An Open Foundation Model for Earth.* Development Seed, 2024. https://developmentseed.org/projects/clay/ . Cited as comparable prior art.
- [HARVEST] Ma, Y. et al. *Harvesting AlphaEarth: Benchmarking the Geospatial Foundation Model for Agricultural Downstream Tasks.* arXiv:2601.00857, 2026.

Optical and radar (live materialisers):

- [S2] European Space Agency / Copernicus. *Sentinel-2 Mission Specification.* Copernicus Open Access Hub. https://sentiwiki.copernicus.eu/web/sentinel-2 . Reference build reads L2A Cloud-Optimized GeoTIFFs via Element84 STAC [STAC] with anonymous AWS S3 range reads.
- [S1] European Space Agency / Copernicus. *Sentinel-1 Mission Specification.* https://sentiwiki.copernicus.eu/web/sentinel-1 . Reference build reads GRD VV (dB), all-weather radar.
- [STAC] Radiant Earth Foundation. *SpatioTemporal Asset Catalog (STAC) v1.0.* https://stacspec.org/ . Reference build queries the Element84 / Earth Search v1 endpoint for S1, S2, and Landsat scenes.

Terrain and bathymetry:

- [COPDEM] European Space Agency / Airbus. *Copernicus Digital Elevation Model 30 m (Cop-DEM GLO-30).* 2021. https://spacedata.copernicus.eu/collections/copernicus-digital-elevation-model . Reference build reads the public AWS mirror.
- [GMRT] Lamont-Doherty Earth Observatory. *Global Multi-Resolution Topography (GMRT) Synthesis.* https://www.gmrt.org/ . Reference build serves global topo + bathy.

Land surface and biophysics:

- [MODIS] NASA Land Processes DAAC. *MODIS Terra/Aqua Standard Products (MOD11A2 LST 8-day; MOD13Q1 NDVI 16-day; MOD16A2 ET; MOD17A2H GPP; MOD15A2H LAI; MCD64A1 Burned Area).* https://lpdaac.usgs.gov/ . Reference build pulls per-cell samples via the public AWS mirror.
- [JRC-GSW] Pekel, J.-F. et al. *High-resolution mapping of global surface water and its long-term changes.* Nature 540, 418-422, 2016. JRC Global Surface Water v1.4, Landsat 1984-2021. https://global-surface-water.appspot.com/ .
- [HANSEN] Hansen, M. C. et al. *High-Resolution Global Maps of 21st-Century Forest Cover Change.* Science 342, 850-853, 2013. v1.11 2023 release at 30 m. https://glad.umd.edu/dataset/global-2010-tree-cover-30-m .
- [WORLDCOVER] Zanaga, D. et al. *ESA WorldCover 10 m 2021 v200.* European Space Agency / VITO. https://esa-worldcover.org/ . CC BY 4.0.
- [SOILGRIDS] Poggio, L. et al. *SoilGrids 2.0: producing soil information for the globe with quantified spatial uncertainty.* Soil 7, 217-240, 2021. ISRIC. https://www.isric.org/explore/soilgrids . CC BY 4.0.

Climate, weather, atmosphere:

- [METNO] Norwegian Meteorological Institute. *MET Norway Locationforecast 2.0 / compact.* https://api.met.no/weatherapi/locationforecast/2.0/documentation . CC BY 4.0; ECMWF + EUMETSAT geostationary-fed nowcast.
- [ERA5] Hersbach, H. et al. *The ERA5 global reanalysis.* Q.J.R. Meteorol. Soc. 146, 1999-2049, 2020. ECMWF. Reference build accesses via Open-Meteo. https://www.ecmwf.int/en/forecasts/dataset/ecmwf-reanalysis-v5
- [POWER] NASA Langley Research Center. *POWER: Prediction Of Worldwide Energy Resources (MERRA-2 + GEOS).* US Government, public domain. https://power.larc.nasa.gov/
- [OPENMETEO-CAMS] Open-Meteo. *Air Quality API (Copernicus Atmosphere Monitoring Service).* CC BY 4.0. https://open-meteo.com/en/docs/air-quality-api . Surface-level pollutants (PM2.5, PM10, NO₂, O₃, SO₂, CO, AOD).
- [OPENMETEO-MARINE] Open-Meteo. *Marine Weather API (ECMWF WAM).* CC BY 4.0. https://open-meteo.com/en/docs/marine-weather-api . Wave height, swell period / height, SST.

Human geography:

- [OVERTURE] Overture Maps Foundation. *Overture Maps Data Schema.* https://overturemaps.org/ . Reference build aggregates buildings, places, transportation per cell from anonymous S3 partitions.

### 22.3 Informative references — agent platforms and prior protocols

- [MAPBOX-MCP] Mapbox. *Introducing the Mapbox Model Context Protocol Server.* Mapbox Blog, 2026. https://www.mapbox.com/blog/introducing-the-mapbox-model-context-protocol-mcp-server
- [GMAPS-AGENTIC] Google. *Powering the next era of agentic experiences — new grounding capabilities.* Google Maps Platform Blog, 2026. https://mapsplatform.google.com/resources/blog/powering-the-next-era-of-agentic-experiences-announcing-new-grounding-capabilities/
- [CARTO-MCP] CARTO. *CARTO MCP Server.* CARTO Blog, 2026. https://carto.com/blog/carto-mcp-server-turn-your-ai-agents-into-geospatial-experts/
- [KNOWWHERE] Janowicz, K. et al. *The KnowWhereGraph: A Large-Scale Geo-Knowledge Graph for Interdisciplinary Knowledge Discovery and Geo-Enrichment.* arXiv:2502.13874, 2025.
- [BLOOMBERRY-MCP] Bloomberry. *We analyzed 1,400 MCP servers — here's what we learned.* 2026. https://bloomberry.com/blog/we-analyzed-1400-mcp-servers-heres-what-we-learned/

### 22.4 Informative references — addressing and indexing

- [H3] Uber Engineering. *H3: A Hexagonal Hierarchical Geospatial Indexing System.* https://h3geo.org/ .
- [HEX2VEC] Wozniak, S., and Szymanski, P. *hex2vec: Context-Aware Embedding H3 Hexagons with OpenStreetMap Tags.* arXiv:2111.00970, 2021.
- [S2GEO] Google. *S2 Geometry Library.* https://s2geometry.io/ .

### 22.5 Informative references — verifiable inference

- [DEEPPROVE] Lagrange Labs. *DeepProve-1: The First zkML System to Prove a Full LLM Inference.* 2026. https://lagrange.dev/blog/deepprove-1
- [ZK-OPML] Vid201 et al. *zk-OPML: Using zero-knowledge proofs to optimize OPML.* J. King Saud Univ. CIS, 2026.

---

*End of emem Protocol Specification v0.0.4.*
