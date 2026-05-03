# emem — the Earth Memory Protocol

**Version 0.0.3** · Vortx-AI · Apache-2.0
github.com/Vortx-AI/emem

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

A 64-bit packed integer over a recursive icosahedral aperture-7 hex
tessellation (H3-equivalent geometry, with the wire format defined
without referencing H3). Bit layout:

```
[63]      reserved (must be 0)
[62..59]  mode (4 bits, 16 modes)
[58..56]  edge/vertex disambiguation (3 bits)
[55..52]  resolution (4 bits, 0..=15)
[51..45]  base cell (7 bits, 0..=121 valid)
[44..0]   path: 15 × 3-bit child digits
```

Default fact resolution is 13 (≈3.4 m hex edge); maximum is 15. Modes
include `Cell`, `DirectedEdge`, `UndirectedEdge`, `Vertex`, and `Set`.
The bit layout is *itself* locality-preserving: cells with shared
cell-bit-prefix share string-prefix in the cell64 codec.

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
| `slow`      | 1 year        | AlphaEarth, soil      |
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
(content-addressed, IPLD-style) tiers, but the 0.0.3 reference build
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

- Copernicus DEM 30m (AWS open data)
- ESA WorldCover v2.00 (AWS open data)
- JRC Global Surface Water v1.4 (GCS public)
- Hansen Global Forest Change v1.12 (GCS public)
- GHSL Built-up & Population R2023A (JRC EC)
- WorldPop 1km (worldpop.org)
- AlphaEarth Foundations v1 (GCS public, when enabled)
- OSM tile servers (rate-limited)

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

```
GET    /health
GET    /.well-known/emem.json
GET    /openapi.json
GET    /v1/manifests | /v1/bands | /v1/functions | /v1/sources | /v1/errors | /v1/tools
GET    /v1/cells/{cell64}
POST   /v1/recall | /v1/query_region | /v1/compare | /v1/find_similar
POST   /v1/diff   | /v1/trajectory  | /v1/verify   | /v1/intent
POST   /v1/attest        (signed JSON)
POST   /v1/attest_cbor   (signed canonical CBOR — preferred for byte-exact merkle)
GET    /v1/facts/{cid}
```

### 9.2 MCP Streamable HTTP (in-loop agent)

```
POST   /mcp
   method: initialize              → { protocolVersion, serverInfo, capabilities }
   method: tools/list              → 28 tools spanning recall, multimodal, introspection
   method: tools/call              → invoke any primitive
```

The MCP surface mirrors the REST primitives one-for-one and adds
introspection tools (`emem_bands`, `emem_manifests`, `emem_errors`, …).
Authoritative count + names come from `tools/list`; `docs/AGENTS.md §10`
has paste-ready configs for every supporting host.

### 9.3 OpenAPI 3.1 (LLM tool discovery)

`GET /openapi.json` returns a hand-rolled OpenAPI manifest covering
every REST route, with JSON schemas for every request body. Agents
that consume OpenAPI tool descriptions (Claude, GPT) can wire emem
without bespoke glue.

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

---

## 11. Conformance levels

- **L0** — read-only, public-band recall (every emem build serves L0).
- **L1** — verified claims (`/v1/verify mode=fast`).
- **L2** — write (`/v1/attest`). Any contributor with an ed25519 keypair
  can attest; the responder accepts on canonical-CBOR + signature
  verification. `/v1/challenge` and stake-based slashing are reserved
  in the wire format but are **not implemented in 0.0.x** — see
  SPEC §6.3 and §8.4.

Every receipt declares the active level and registry/schema CIDs.

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
for one regime and pays for it in the others. The 0.0.3 reference
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

- `docs/SPEC.md` — wire-stable protocol specification.
- `crates/emem-core` — type identities, manifest loaders.
- `crates/emem-codec` — agent-native string codecs.
- `crates/emem-fact` — fact / attestation / receipt types.
- `crates/emem-cache`, `crates/emem-storage` — hot cache + materializer + log.
- `crates/emem-primitives` — read primitives over `&Server`.
- `crates/emem-fetch` — anonymous HTTPS / GCS / vsicurl connectors.
- `crates/emem-api-rest` — HTTP surface (REST + MCP + OpenAPI).
- `crates/emem-cli` — `emem` (introspection) and `emem-server` (HTTP) binaries.
