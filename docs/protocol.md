# emem protocol (v0.0.6)

## What this document promises

Bytes on the wire. Anyone implementing emem in another language reads this
document, follows it line by line, and produces byte-identical receipts to
a Rust responder running the same registry CIDs. Where the prose diverges
from `crates/emem-codec`, `crates/emem-fact`, `crates/emem-attest`, or
`crates/emem-storage`, the source is canonical and this document is the
bug. Every encoding rule cites the file and line that defines it.

---

## 1. Cell64 — the spatial primitive

A Cell64 is a 64-bit integer that addresses a quantised lat/lng bucket on
the WGS-84 ellipsoid. The wire form is four base-65,536 digits joined by
dots, e.g. `dedi.zaf00.bafi.baba`. The integer form is what gets hashed
and compared; the dotted text form is what shows up inside facts and
receipts.

![address algebra — cell + band + tslot → canonical CBOR → blake3 → 26-character base32 CID](/docs/diagrams/09-address-algebra.svg)
*The full address pipeline at a glance. Section 4 walks each box; the SVG also names every constant the encoder uses.*

### 1.1 Bit layout

Defined in `crates/emem-codec/src/geo.rs:38-45`.

```
 bit  63           60 59         52 51         44 43 42         22 21          0
      +-------------+--------------+-------------+--+-------------+--------------+
      |  mode (4)   | resolution(8)|   base (8)  |R |  lat_q (21) |  lng_q (22)  |
      |   0b0001    |     21       |     0xab    |0 |             |              |
      +-------------+--------------+-------------+--+-------------+--------------+
```

| Bits | Field | Value | Purpose |
|------|-------|-------|---------|
| 63..60 | mode | `0b0001` | distinguishes cell from edge/vertex/set |
| 59..52 | resolution | `21` | encoded as the lat-axis bit count |
| 51..44 | base | `0xab` | "geo aperture" marker, separates this layout from H3-style cells |
| 43..43 | reserved | `0` | must be zero on encode; pass-through on decode |
| 42..22 | lat_q | 21 bits | quantised latitude, `[0, 2^21)` over the 180° range |
| 21..00 | lng_q | 22 bits | quantised longitude, `[0, 2^22)` over the 360° range |

The lat axis carries one fewer bit than the lng axis on purpose:
180° / 2^21 ≈ 360° / 2^22, so the bucket is square at the equator
(~9.54 m × ~9.55 m). Equal bit counts would give 1:2-rectangular cells.

### 1.2 Encoding rule (lat/lng → Cell)

Defined in `crates/emem-codec/src/geo.rs:75-82`:

```rust
pub fn cell_from_latlng(lat_deg: f64, lng_deg: f64) -> Cell {
    let lat = lat_deg.clamp(-90.0, 90.0);
    let lng = ((lng_deg + 180.0).rem_euclid(360.0)) - 180.0;
    let lat_q = (((lat + 90.0) / 180.0) * GEO_LAT_MAX as f64).round() as u64 & GEO_LAT_MASK;
    let lng_q = (((lng + 180.0) / 360.0) * GEO_LNG_MAX as f64).round() as u64 & GEO_LNG_MASK;
    let path = (lat_q << GEO_LNG_BITS) | lng_q;
    Cell::from_raw(GEO_PREFIX | path)
}
```

Constants (geo.rs:50-71):
`GEO_LAT_BITS=21`, `GEO_LNG_BITS=22`,
`GEO_LAT_MAX=(1<<21)-1=2_097_151`,
`GEO_LNG_MAX=(1<<22)-1=4_194_303`,
`GEO_RES=21` (resolution tag),
`GEO_BASE=0xab` (aperture marker),
`GEO_PREFIX = (1 << 60) | (21 << 52) | (0xab << 44) = 0x1150_ab00_0000_0000`.

Lat clamps to `[-90, 90]`. Lng wraps via `rem_euclid` (`-181°` →
`+179°`). Quantisation is `f64::round`
(round-half-away-from-zero).

### 1.3 Text form: 4 bigrams + dots

Defined in `crates/emem-codec/src/cell64.rs:14-24`. The 64-bit integer
is split into four 16-bit lanes (`d0=raw>>48`, `d1=raw>>32`,
`d2=raw>>16`, `d3=raw`, each masked to 16 bits) and each lane indexes
a 65,536-entry alphabet built deterministically in
`crates/emem-codec/src/alphabet.rs:22-46`:

- Consonants: `b c d f g h j k l m n p q r s t v w x y z` (21).
- Vowels: `a e i o u A E I O U` (10).
- Bigrams: outer product `c1·v1·c2·v2`, in that exact loop order →
  21 × 10 × 21 × 10 = 44,100 bigrams covering indices 0..44,099.
- Indices 44,100..65,535 are filled with synthetic codepoints
  `z<hex4>`, where `<hex4>` is the four-digit lowercase hex of the
  index itself.

Index → bigram is `O(1)` via `ALPHABET[i]`; bigram → index is `O(1)`
via the precomputed reverse map `ALPHABET_INDEX`.

### 1.4 Worked example: lat=0.0, lng=0.0

Apply `cell_from_latlng`:

1. `lat_q = round(0.5 × 2_097_151) = 1_048_576` (round-half-away-from-zero).
2. `lng_q = round(0.5 × 4_194_303) = 2_097_152`.
3. `path = (1_048_576 << 22) | 2_097_152 = 0x0000_0400_0020_0000`.
4. `raw = GEO_PREFIX | path = 0x1150_ab00_0000_0000 | 0x0000_0400_0020_0000
   = 0x1150_af00_0020_0000`.

The four 16-bit lanes are
`d0=0x1150 (4432)`,
`d1=0xaf00 (44800)`,
`d2=0x0020 (32)`,
`d3=0x0000 (0)`.

Index `i` in the structured-bigram region (0..44,099) decomposes as
`i = c1·2100 + v1·210 + c2·10 + v2` from `alphabet.rs`. Indices ≥
44,100 fall into the synthetic `z<hex4>` region.

- `d0 = 4432 = 2·2100 + 1·210 + 2·10 + 2` → `d e d i` → **`dedi`**.
- `d1 = 44800` ≥ 44,100 → `"z" + "af00"` → **`zaf00`**.
- `d2 = 32 = 0 + 0 + 3·10 + 2` → `b a f i` → **`bafi`**.
- `d3 = 0` → `b a b a` → **`baba`**.

Cell64 for `(lat=0.0, lng=0.0)` is

```
dedi.zaf00.bafi.baba
```

### 1.5 Decode rule (cell64 → lat/lng)

`latlng_from_cell64` (geo.rs:96-117) inverts the encode: parse the
four bigrams via `from_cell64` (cell64.rs:41-54), reject if
`(raw & 0xFFFF_F000_0000_0000) != GEO_PREFIX` (guards against legacy;
see §1.7), then unpack
`lng_q = raw & GEO_LNG_MASK`, `lat_q = (raw >> 22) & GEO_LAT_MASK`,
and convert with `lat_deg = (lat_q/GEO_LAT_MAX)·180 - 90` and
`lng_deg = (lng_q/GEO_LNG_MAX)·360 - 180`. The bucket bbox extends
`±half_lat / ±half_lng` from the centre, clipped to `[-90, 90]` on lat.

### 1.6 Round-trip and edge cases

The tests in `geo.rs:139-241` pin the contract.

- **Sub-quantum collision** (geo.rs:198-202): two queries 9 µ° apart
  (~1 m) MUST produce the same cell — the cell's grain, not a bug.
- **12 m apart distinguishes** (geo.rs:187-193): two queries
  `1.08e-4°` apart (~12 m) MUST produce different cells.
- **Antimeridian** (geo.rs:178-180): `lng = 179.99` round-trips;
  `lng = -181` wraps to `+179`.
- **Polar clamp** (geo.rs:76): `lat = 95` clamps to `90`.
- **Square at equator** (geo.rs:207-225): bucket extent is 8–12 m on
  both axes; lat and lng agree to within 5%.

### 1.7 Legacy 16-bit grid: rejected

Pre-0.0.3 emem used a `GEO_RES = 12` (16-bit-per-axis, ~305 m) grid.
That encoding is **not** decodable by the current codec. The test at
`geo.rs:231-241` constructs a legacy-shaped raw word and confirms
`latlng_from_cell64` returns `Err(CodecError::NotGeoCell)` — the
resolution field changed (12 → 21), `GEO_PREFIX_MASK` keys on it, so
legacy strings fail closed instead of silently misplacing a fact by
hundreds of metres. Implementations MUST NOT serve, accept, or quietly
upgrade legacy cell64 strings.

---

## 2. Tslot — temporal addressing

A `Tslot` is a `u64` bucket index of the Unix timeline at a band's
declared tempo cadence. Defined in `crates/emem-core/src/tslot.rs:19-22`.

### 2.1 Anchor: Unix epoch, not emem epoch

Pre-0.0.3 emem anchored tslot at `2026-01-01T00:00:00Z` (`EMEM_EPOCH_UNIX
= 1_767_225_600`). That broke history: every pre-2026 observation
collapsed to `Tslot(0)`. The current code (tslot.rs:56-68) computes

```
Tslot(unix_seconds.max(0) / tempo.slot_seconds())
```

The constant `EMEM_EPOCH_UNIX` is retained as protocol metadata only —
nothing in the encode path subtracts it. Pre-1970 (negative Unix)
inputs clamp to `Tslot(0)`.

### 2.2 Tempo class

Defined in `tslot.rs:24-37, 43-54`. Five variants:

| Variant | `slot_seconds()` | Cadence | Sample bands |
|---------|------------------|---------|--------------|
| `Static` | 0 | never changes | DEM, Köppen, lcv-1 |
| `Slow` | 31_536_000 | 365 d | Tessera (2017–2024 vintages + `multi_year` 1024-D + `bin128`), soil |
| `Medium` | 2_592_000 | 30 d | NDVI composites |
| `Fast` | 86_400 | 1 d | raw S2 NDVI |
| `UltraFast` | 3_600 | 1 h | weather, traffic |

`Static` returns `Tslot(0)` regardless of input — the slot is
meaningless for a band that never refreshes. `to_unix_start` is the
inverse: the Unix second at which the slot opened.

### 2.3 Cadence overlap

```
seconds since 1970   ─────────────────────────────────────────────►
                     0                                            now

Static     [   one bucket forever                               ]
Slow       [365d][365d][365d][365d][365d][365d][365d][365d][365d]
Medium     |30d|30d|30d|30d|30d|30d|30d|30d|30d|30d|30d|30d|30d|
Fast       ||||||||||||||||||||||||||||||||||||||||||||||||||||||
UltraFast  ::::::::::::::::::::::::::::::::::::::::::::::::::::::

Sample-band cadences:
- Tessera annual:  one Slow slot per year         (Slow)
- MODIS NDVI:      one Fast slot per 8-day comp   (Fast)
- Open-Meteo:      one UltraFast slot per hour    (UltraFast)
```

### 2.4 Text form: `t.<base32-nopad-leb128>`

Defined in `crates/emem-codec/src/tslot_text.rs:9-13`. The integer is
encoded as little-endian LEB128 (varint), then the byte string is
base32-encoded with `data_encoding::BASE32_NOPAD` and lowercased.

```rust
fn to_tslot_text(t: Tslot) -> String {
    let mut buf = [0u8; 10];
    let n = encode_varint(t.0, &mut buf);
    format!("t.{}", BASE32_NOPAD.encode(&buf[..n]).to_lowercase())
}
```

Worked examples (matching the test at tslot_text.rs:70-77):

- `Tslot(0)` → varint `[0x00]` → base32 `"AA"` → text `t.aa`.
- `Tslot(1)` → varint `[0x01]` → base32 `"AE"` → text `t.ae`.
- `Tslot(26)` → varint `[0x1A]` → base32 `"DI"` → text `t.di`.
- `Tslot(1024)` → varint `[0x80, 0x08]` → base32 `"QAEA"` → text
  `t.qaea`.

Decode inverts the chain (tslot_text.rs:16-23): strip `t.`, uppercase
the body, base32-decode, LEB128-decode.

---

## 3. CID and FactCid

emem uses BLAKE3 over canonical CBOR. The hash bytes are encoded with
`data_encoding::BASE32_NOPAD` and lowercased. There are exactly two
durable lengths:

| Form | Bytes | Chars | Source | Use |
|------|-------|-------|--------|-----|
| `cid64` | 8 | 13 | `crates/emem-codec/src/cid64.rs:9-11` | short visible ID for inline text |
| `FactCid` | 16 | 26 | `crates/emem-fact/src/cbor.rs:38-41` (`base32_prefix(&hash, 16)`) | durable storage and signing |

The full 32-byte hash is computed once (`blake3_32`, cbor.rs:30-35);
the two encodings are prefixes.

### 3.1 cid64

```rust
pub fn to_cid64(cid: &[u8; 32]) -> String {
    BASE32_NOPAD.encode(&cid[..8]).to_lowercase()
}
```

8 bytes = 64 bits → `ceil(64/5) = 13` base32 characters. Decode-only
inversion (`from_cid64`, cid64.rs:15-25) returns the `[u8; 8]`
prefix; full collision resistance requires the full 32-byte CID.

### 3.2 FactCid

`FactCid` is a string newtype (cid.rs:25). The construction is

```text
FactCid = base32_nopad_lowercase( blake3( canonical_cbor(fact) )[..16] )
```

16 bytes = 128 bits → 26 base32 characters. Mutating any field of the
fact changes its CBOR bytes and therefore its FactCid; the round-trip
test at `crates/emem-fact/tests/round_trip.rs` (CBOR → decode →
re-encode → byte-equal) pins this.

The same recipe constructs the other newtypes from `cid.rs:25-34`:
`RegistryCid`, `SchemaCid`, `ReasonCid`, `BatchCid`, `CoverageCid`.

### 3.3 Manifest CID

For the eight registries (bands, algorithms, functions, sources,
topics, schema, lcv-1, alphabet) the recipe is identical:

```text
manifest_cid = base32_nopad_lowercase( blake3( canonical_cbor(manifest) )[..32] )
```

Full 32 bytes (52 chars) — the bigger size is acceptable here because a
manifest CID appears once per response in `registry_cid` / `schema_cid`,
not once per fact.

---

## 4. CBOR canonicalisation

Defined in `crates/emem-fact/src/cbor.rs`.

```rust
pub fn to_canonical_cbor<T: serde::Serialize>(v: &T) -> Result<Vec<u8>, ...> {
    let mut buf = Vec::new();
    ciborium::ser::into_writer(v, &mut buf)?;
    Ok(buf)
}
```

The encoder is `ciborium::ser::into_writer`. `ciborium` emits RFC 8949
deterministic encoding **when** the input traversal is deterministic.
For serde-derived structs that holds: fields serialise in declaration
order. For freeform maps (`ciborium::Value::Map`) callers MUST present
the map with already-sorted keys — emem does not re-sort silently.

### 4.1 emem CBOR tags

`crates/emem-fact/src/cbor.rs:6-13`.

| Tag | Meaning | Tagged value |
|-----|---------|--------------|
| 65000 | emem cell | u64 packed per §1.1 |
| 65001 | emem tslot | u64 |
| 65002 | emem vec64 | 32-byte vector CID |
| 42 | IPLD CID | base32 multibase string (`b...`) |

Two implementations MUST produce byte-identical CBOR for the same fact.
A round-trip test (encode → decode → encode → byte-compare) is the
gate; `crates/emem-fact/tests/round_trip.rs` enforces it.

### 4.2 What "canonical" means here, concretely

- Length encodings minimal (CBOR head byte is the shortest form
  `ciborium` emits).
- Field order = serde declaration order. For `PrimaryFact` that is
  `cell, band, tslot, value, unit?, confidence, uncertainty?, sources,
  derivation, privacy_class, schema_cid, signer, signed_at`
  (fact.rs:38-67).
- `Option::None` fields with `#[serde(skip_serializing_if =
  "Option::is_none")]` are absent from the CBOR map (not encoded as
  null).
- Floats serialise as f64 unless declared `f32`. `confidence: f32`
  emits CBOR major type 7 with f32 head.

---

## 5. Fact

Three variants. All carry `signer: AttesterKey` (32-byte ed25519
public key) and `signed_at: String` (ISO 8601 UTC), so any fact can be
attributed without referring to its enclosing attestation.

### 5.1 PrimaryFact

`crates/emem-fact/src/fact.rs:37-67`.

```rust
struct PrimaryFact {
    cell: String,                       // cell64 string
    band: String,                       // e.g. "indices.ndvi"
    tslot: u64,                         // bucket per band tempo
    value: ciborium::Value,             // band-typed (number, vector, enum)
    unit: Option<String>,               // SI unit when applicable
    confidence: f32,                    // 0..1
    uncertainty: Option<Uncertainty>,
    sources: Vec<Source>,               // ≥1
    derivation: Derivation,             // recipe for re-execution
    privacy_class: String,              // serialised at attest time
    schema_cid: SchemaCid,
    signer: AttesterKey,                // [u8; 32]
    signed_at: String,                  // ISO 8601 UTC
}
```

Worked example (Fast-tempo NDVI composite over a single Sentinel-2 capture):

```jsonc
{
  "kind": "primary",
  "cell": "dedi.zaf00.bafi.baba",
  "band": "indices.ndvi",
  "tslot": 19852,
  "value": 0.42, "unit": "dimensionless", "confidence": 0.97,
  "sources": [{ "scheme": "sentinel2.l2a",
                "id": "S2A_MSIL2A_20240315T101031_T43PFT",
                "captured_at": "2024-03-15T10:10:31Z" }],
  "derivation": { "fn_key": "indices.ndvi@1" },
  "privacy_class": "public",
  "schema_cid": "bn7c...",
  "signer": [/* 32 bytes */],
  "signed_at": "2024-03-15T11:02:14Z"
}
```

`tslot=19852` at Fast tempo (86_400 s) inverts to Unix
`19852 × 86_400 = 1_715_212_800 = 2024-05-09T00:00:00Z` (slot start).

### 5.2 DerivativeFact

`fact.rs:71-94`.

```rust
struct DerivativeFact {
    cell: String, band: String,
    tslot_window: [u64; 2],             // inclusive [start, end]
    op: String,                         // delta | mean | trend | rate | anomaly
    parents: Vec<FactCid>,              // input fact CIDs
    value: ciborium::Value,
    confidence: f32,
    derivation: Derivation,
    schema_cid: SchemaCid,
    signer: AttesterKey,
    signed_at: String,
}
```

Worked example (90-day NDVI mean over three monthly composites):

```jsonc
{
  "kind": "derivative",
  "cell": "dedi.zaf00.bafi.baba",
  "band": "indices.ndvi",
  "tslot_window": [665, 667],
  "op": "mean",
  "parents": ["fc6...26char", "ab2...26char", "9k4...26char"],
  "value": 0.39, "confidence": 0.92,
  "derivation": { "fn_key": "agg.mean@1" },
  "schema_cid": "bn7c...",
  "signer": [/* 32 bytes */],
  "signed_at": "2024-04-01T03:11:00Z"
}
```

### 5.3 NegativeFact

`fact.rs:98-117`.

```rust
struct NegativeFact {
    cell: String, band: String, tslot: u64,
    reason_cid: ReasonCid,              // evidence (e.g. an S1 scene CID)
    confidence: f32,
    sources: Vec<Source>,               // ≥1
    schema_cid: SchemaCid,
    signer: AttesterKey,
    signed_at: String,
}
```

A negative fact is **not** the same as a missing record. Missing means
"no responder has attested this (cell, band, tslot)". Negative means
"I looked and there was nothing — here is what I looked at
(`reason_cid`)". Per the no-silent-fallbacks rule, the API must
distinguish these states; see §10.

#### Signed Absence as a first-class protocol move

Every band that has no data at a cell returns a `NegativeFact` —
referred to throughout the codebase as a **signed Absence**. The
Absence itself is content-addressed (it has a `FactCid`), signed by
the responder, and citable on the same footing as a Primary or
Derivative fact. The `reason_cid` carries a typed enumeration:

| Reason | When the responder emits it |
|---|---|
| `outside_coverage` | The query falls outside the dataset's spatial or temporal window (DMSP-OLS post-2013, CHIRPS poleward of ±50°, Köppen pixel value 0 over open ocean). |
| `unavailable_capability` | A required upstream is reachable but does not expose the requested layer (Hansen 80°N tile genuinely not published; Overture release lacks the queried theme). |
| `gpu_unavailable` | A foundation-model band was requested while the Python sidecar UDS is down or VRAM-saturated. |
| `archetype_seed_unavailable` | A climate-archetype query landed in a Köppen-Geiger zone that the v1 centroid seed file does not yet cover. |
| `upstream_no_data` | Upstream returned an empty result with no error (WorldPop `total_population == 0`; FIRMS bulk CSV with no fire detection inside the window). |

A signed Absence is a working answer, not an error path. A verifier
holding the Absence's receipt can replay the same upstream call and
expect the same empty result; downstream agents can pin reasoning on
"the responder looked and confirmed nothing was there" instead of
guessing why a recall came back empty.

### 5.4 Tagged enum on the wire

`fact.rs:9-25`. The `Fact` enum serialises with `#[serde(tag = "kind",
rename_all = "snake_case")]`. CBOR shape: a single map with `"kind"`
keying the string discriminator `"primary" | "derivative" | "absence"`
plus the variant's fields flattened in.

---

## 6. Attestation envelope

`crates/emem-fact/src/attest.rs:10-28`.

```rust
struct Attestation {
    facts: Vec<Fact>,
    batch_root: [u8; 32],               // blake3 merkle root of CBOR(fact_i)
    attester: AttesterKey,
    attester_key_epoch: KeyEpoch,
    registry_cid: RegistryCid,
    schema_cid: SchemaCid,
    signature: Signature,               // [u8; 64]
    attested_at: String,                // ISO 8601 UTC
}
```

The `signature` is over

```text
ed25519_sign( blake3( batch_root || registry_cid_bytes || schema_cid_bytes ) )
```

where `registry_cid_bytes` and `schema_cid_bytes` are the **string
bytes** (UTF-8 of the lowercase base32 CID), not the raw hash bytes.
That is what `verify_attestation` (`crates/emem-storage/src/lib.rs:428-438`)
passes to BLAKE3:

```rust
let mut h = Hasher::new();
h.update(&att.batch_root);
h.update(att.registry_cid.as_str().as_bytes());
h.update(att.schema_cid.as_str().as_bytes());
let msg = h.finalize();
let pk = ed25519_dalek::VerifyingKey::from_bytes(&att.attester.0)?;
let sig = ed25519_dalek::Signature::from_bytes(&att.signature.0);
pk.verify_strict(msg.as_bytes(), &sig)?;
```

### 6.1 Merkle root construction

`crates/emem-attest/src/lib.rs:11-89`. Leaves are the
`blake3(canonical_cbor(fact))` hashes, sorted bytewise. The empty input
returns `[0u8; 32]`.

Every leaf is **promoted by self-hash** before pairing — the leaf
becomes `blake3(leaf || leaf)`. The self-hash separates the "leaf" and
"internal node" domains; without it, an attacker who knows a
`CBOR(fact)` could splice it in at an internal position. The test
`single_leaf_is_self_hashed` (lib.rs:128-139) pins the rule for the
1-leaf case: `merkle_root([leaf]) == blake3(leaf || leaf)`.

Once promoted, layers fold pairwise with `blake3(left || right)`. For
odd-cardinality layers the trailing element pairs **with itself**
(lib.rs:36-44, lib.rs:174-194).

ASCII tree for 4 facts with sorted CBOR-hashes `C0 ≤ C1 ≤ C2 ≤ C3`:

```
                          root
                       blake3(L01 || L23)
                      /                 \
              L01 = blake3(l0 || l1)  L23 = blake3(l2 || l3)
              /          \             /          \
       l0 = b3(C0||C0)  l1=b3(C1||C1)  l2=b3(C2||C2)  l3=b3(C3||C3)
            |               |               |              |
       CBOR(fact0)     CBOR(fact1)     CBOR(fact2)     CBOR(fact3)
```

### 6.2 Verify-on-write

`crates/emem-storage/src/lib.rs:407-440` (`verify_attestation`). Every
attestation re-checks the merkle root and the ed25519 signature
**before** it is persisted; no bypass:

1. CBOR-encode each fact, take `blake3(bytes)` → leaf.
2. Sort leaves bytewise.
3. `emem_attest::merkle_root(&leaves)` must equal `att.batch_root` —
   else `StorageError::AttestationInvalid("merkle root mismatch …")`.
4. Recover `VerifyingKey::from_bytes(&att.attester.0)`.
5. `vk.verify_strict(blake3(batch_root || registry_cid || schema_cid),
   sig)` must succeed — else `AttestationInvalid("bad signature …")`.

Failure → write rejected. The HTTP layer surfaces this as the
`BadSignature` error code from `crates/emem-core/src/error.rs`.

---

## 7. Receipt

![the trust plane — preimage, signature, merkle path, offline verify](/docs/diagrams/10-trust-plane.svg)
*The five-step trust pipeline. Section 7.2 specifies the preimage byte-by-byte; sections 8 and 9 cover the Merkle path and the append-only log.*

`crates/emem-fact/src/receipt.rs:11-58`.

| Field | Type | Notes |
|-------|------|-------|
| `request_id` | `String` | ULID generated per request |
| `served_at` | `String` | ISO 8601 UTC, second precision (`server.rs:194-211`) |
| `primitive` | `String` | namespaced wire form: `"emem.recall"`, `"emem.find_similar"`, `"emem.verify"`, `"emem.query_region"`, … (the bare `"recall"` form is internal-only; wire receipts always include the `emem.` prefix) |
| `intent` | `Option<String>` | populated when served via `/v1/intent`; omitted from JSON when None |
| `cells` | `Vec<String>` | cell64 strings cited in the response |
| `fact_cids` | `Vec<FactCid>` | every fact CID returned |
| `schema_cid` | `SchemaCid` | active CDDL profile |
| `merkle_proof` | `Option<MerkleProof>` | inclusion proof for `fact_cids[0]` when persisted; omitted from JSON when None |
| `responder` | `AttesterKey` | ed25519 pubkey, `[u8; 32]` |
| `responder_key_epoch` | `KeyEpoch` | `u32` rotation counter |
| `responder_pubkey_b32` | `String` | base32-nopad-lowercase of `responder`; appended at REST-serialization time so JSON consumers don't need to re-encode the bytes |
| `signature` | `Signature` | ed25519 `[u8; 64]` |
| `source_versions` | `BTreeMap<String, String>` | per-source version pins |
| `registry_cid` | `RegistryCid` | function registry CID in force |
| `cost` | `Cost` | self-declared (see below) |

`Cost` (`receipt.rs:46-58`):

```rust
struct Cost {
    credits: u64,
    latency_p50_ms: u32,
    latency_p99_ms: u32,
    source_freshness_s: u32,
    was_cached: bool,
}
```

### 7.1 Signature preimage

The exact preimage construction is `Server::sign_receipt`,
`crates/emem-storage/src/server.rs:119-148`. The bytes that go into
BLAKE3, in order:

```
<request_id> | <served_at> | <primitive> |
<cell_0> , <cell_1> , … <cell_{n-1}> , |
<fact_cid_0> , <fact_cid_1> , … <fact_cid_{m-1}> ,
```

Details that matter:

- Header-field separator is `|` (0x7C); list-element separator is
  `,` (0x2C).
- **Every** list element — including the last — is followed by a
  trailing `,`. The loop writes `c.as_bytes()` then `b","`
  unconditionally; there is no terminator-omit branch
  (server.rs:139-147).
- The `|` between the cells block and the fact_cids block appears
  exactly once, after the cells trailing comma and before the first
  fact_cid.
- An empty list contributes only the surrounding `|` bytes.

The signature is `ed25519_dalek::SigningKey::sign(blake3_digest)`
emitted as a 64-byte `Signature`.

   **What the preimage does NOT cover.** The five fields above
   (`request_id`, `served_at`, `primitive`, `cells`, `fact_cids`) are
   the complete signed surface. Notably **NOT** in the preimage:

   - The caller's free-text `place` / `q` string. A wrong-place
     geocode produces a clean signature for the wrong cell64; the
     trust chain attests *the responder claims these facts at these
     cells*, never *these cells were the right resolution of the
     question*. Agents bind the resolution decision themselves via
     `selected.is_high_confidence` from `POST /v1/locate`.
   - The caller's raw `lat` / `lng`. Quantisation collapses sub-cell
     precision into `cell_from_latlng` *before* signing; the receipt
     binds the cell, not the input coordinate.
   - The requested `bands[]`, `tslot`, `intent`. The responder returned
     what it returned; whether the returned facts answer the agent's
     question is the agent's interpretive responsibility.

   Echo the original query alongside the receipt if the downstream
   needs *"the user asked X and the responder agreed"* — the receipt
   alone does not testify to the resolution-of-intent step.

   **Per-replica fact identity.** Each Primary / Negative /
   Derivative fact body includes `signed_at` (ISO-8601 wall clock at
   materialisation time), which is part of the canonical CBOR hashed
   into `fact_cid`. Two responders materialising the same
   `(cell, band, tslot)` from byte-identical upstream pixels therefore
   produce **different `fact_cid`s** — this is intentional (each
   responder signs independently under its own identity). The
   cross-replica join key for "does any responder have this
   observation" is the tuple `(cell, band, tslot)`, not `fact_cid`.
   Aggregate fan-out endpoints — notably `POST /v1/recall_polygon` —
   emit one independently signed receipt per cell under
   `by_cell.<cell>.receipt`; the top-level `merged_facts[]` is
   convenience-only and is **not** covered by an aggregate signature.

### 7.2 Worked example: preimage layout

Given:

- `request_id = "01HZX0K9V3"` (ULID, 26 chars in practice; this short
  example is illustrative)
- `served_at = "2026-05-08T11:22:33Z"`
- `primitive = "emem.recall"` (every emitted primitive name is namespaced; `crates/emem-primitives/src/recall.rs:115` calls `sign_receipt("emem.recall", …)`)
- `cells = ["dedi.zaf00.bafi.baba", "dedi.zaf00.bafi.babe"]`
- `fact_cids = ["bn7cabcdefghij1234567890ab"]`

The preimage byte sequence is the concatenation, with no extra
whitespace:

```
01HZX0K9V3|2026-05-08T11:22:33Z|emem.recall|dedi.zaf00.bafi.baba,dedi.zaf00.bafi.babe,|bn7cabcdefghij1234567890ab,
```

Then `signature = ed25519_sign( blake3(preimage_bytes) )`.

The same logic with empty `cells` and one `fact_cids` would be:

```
01HZX0K9V3|2026-05-08T11:22:33Z|emem.recall||bn7cabcdefghij1234567890ab,
```

Two `|` characters in succession is the legal "empty list" shape.

### 7.3 Merkle proof attachment

`server.rs:163-165` attaches `merkle_proof` for `fact_cids[0]` only.
The receipt's signature already binds **all** cited CIDs together
(they appear in the preimage), so a single inclusion anchor to the
attestation tree is sufficient: the verifier checks the signature,
then checks the one inclusion proof, and is convinced the whole batch
came from the declared attester.

If the cited facts pre-date the proof tree (ephemeral run, or facts
written before `persist_fact_proofs` shipped), `merkle_proof` is
absent. The receipt is still a valid signed statement; only the
attestation-tree anchor is missing.

### 7.4 Offline verification (Python)

A self-contained verifier:

```python
import json, urllib.request
from blake3 import blake3
from nacl.signing import VerifyKey

receipt = json.load(open("receipt.json"))
well_known = json.load(urllib.request.urlopen(
    "https://your.emem.host/.well-known/emem.json"))
pk_bytes = bytes(well_known["responder"])    # [u8; 32]

# Reconstruct the preimage per server.rs:119-148.
parts  = receipt["request_id"].encode() + b"|"
parts += receipt["served_at"].encode()  + b"|"
parts += receipt["primitive"].encode()  + b"|"
for c in receipt["cells"]:     parts += c.encode() + b","
parts += b"|"
for c in receipt["fact_cids"]: parts += c.encode() + b","

digest = blake3(parts).digest()
VerifyKey(pk_bytes).verify(digest, bytes(receipt["signature"]))
```

A verifier that can reproduce the preimage and run `verify_strict` is
the entire trust-rebinding path — no other call to the responder is
required.

---

## 8. Merkle inclusion proof

`crates/emem-fact/src/receipt.rs:60-69`:

```rust
struct MerkleProof {
    leaf_index: u32,                    // position in sorted-leaves order
    path: Vec<[u8; 32]>,                // sibling hashes from leaf upward
    root: [u8; 32],                     // expected root
}
```

`leaf_index` is the position of the leaf in the **sorted** batch (the
same sort that produced `batch_root`); not the original fact index.
The conversion from "original fact index" to "sorted leaf index" is
done at write time by `persist_fact_proofs`
(`crates/emem-storage/src/lib.rs:360-400`).

### 8.1 verify_merkle_path

`crates/emem-attest/src/lib.rs:94-117`. The verifier walks the path
bottom-up: at each layer, `idx % 2 == 0` means the accumulator is the
left child (`acc := blake3(acc || sibling)`); odd means it is the
right child (`acc := blake3(sibling || acc)`); then `idx /= 2`. Final
`acc` must equal `root`.

Two preconditions a verifier must respect:

1. The `leaf` argument is the **promoted** leaf — `blake3(C || C)`,
   not the raw `C = blake3(CBOR(fact))`. The same self-hash that
   `merkle_root` applies internally must be done by the caller before
   `verify_merkle_path`. The test at `lib.rs:160-171` and `lib.rs:196-218`
   show the exact pattern.
2. The path is **bottom-up**: `path[0]` is the leaf's sibling at
   layer 0; `path[k]` is the sibling at layer `k`.

### 8.2 Worked path: leaf 1 of a 4-leaf tree

For four facts producing sorted leaves `[C0, C1, C2, C3]`, with
promoted forms `[l0, l1, l2, l3]`:

```
                  root = b3(L01 || L23)
                 /                     \
        L01 = b3(l0 || l1)         L23 = b3(l2 || l3)
         /          \                /          \
       l0          l1               l2          l3
```

Inclusion proof for fact at sorted index 1 (i.e. promoted-leaf `l1`):

- `leaf_index = 1`
- `path = [ l0, L23 ]`
- `root = b3(L01 || L23)`

Trace `verify_merkle_path(l1, 1, [l0, L23], root)`:

1. `acc = l1`, `idx = 1`.
2. Layer 0: `idx % 2 == 1` → `acc := b3(l0 || acc) = b3(l0 || l1) = L01`.
   `idx /= 2 → 0`.
3. Layer 1: `idx % 2 == 0` → `acc := b3(acc || L23) = b3(L01 || L23) =
   root`.
4. Loop ends. `acc == root` → `true`.

### 8.3 Single-leaf case

For a one-fact attestation, `path` is empty (no siblings to combine)
and the promoted leaf **is** the root (`lib.rs:160-171`). The verifier
short-circuits: `acc = leaf; return acc == root`.

### 8.4 Odd-cardinality case

When a layer has odd size, the trailing element pairs with itself
(`lib.rs:36-44`). The recorded sibling for that index at that level is
the leaf itself; `verify_merkle_path` reproduces the duplicate-pair
branch automatically because `idx % 2 == 0` for the last index, so
`acc := b3(acc || acc)`. Test: `lib.rs:174-194`.

---

## 9. Append-only attestation log

`crates/emem-storage/src/merkle_log.rs`. Every verified attestation
goes here before any per-cell index is updated. The on-disk format is
recoverable without the database.

### 9.1 Layout

Files live under `<EMEM_DATA>/log/` and are named
`merkle.log.<u64-segment-index>` (`merkle_log.rs:77`).

Per record (`merkle_log.rs:58-85`):

```
+--------+-----------------------------+--------------------+
| u32 LE | CBOR(Attestation)           | blake3(CBOR) [32B] |
| length |                             |                    |
+--------+-----------------------------+--------------------+
```

Per segment trailer (`merkle_log.rs:161-171`):

```
< all records >  || segment_hash = blake3(all_records) [32 B]
```

The trailer is written when the segment is sealed at rotation
(`merkle_log.rs:74-76`). The current/open segment has no trailer until
it rotates.

### 9.2 Append semantics

`AttestationLog::append` (`merkle_log.rs:58-91`): CBOR-encode the
attestation, hash it, build the `[len][cbor][hash]` record, rotate the
segment if the open one would exceed 1 GiB, append, then `sync_all()`.
Data is fsynced before `append` returns — receipts depend on the
durability claim.

`AppendOutcome` (`merkle_log.rs:142-150`) returns `segment_index`,
`offset_in_segment`, and `record_hash` — enough to rebuild a
record-level inclusion proof later.

### 9.3 Rotation

When the open segment exceeds 1 GiB, `seal_segment`
(`merkle_log.rs:161-171`) finalises the in-memory hasher, appends its
32-byte output as a trailer, fsyncs, and bumps `segment_index`. The
next append opens a fresh segment.

### 9.4 Verify

`AttestationLog::verify` (`merkle_log.rs:102-136`) walks every
`merkle.log.*` file, splits off the trailing 32 bytes, recomputes
`blake3(body)`, and counts segments where it matches. Mismatches are
returned in `VerifyReport.bad`; the log keeps writing to a fresh
segment so a corrupt sealed segment does not halt ingestion.

### 9.5 Snapshot / replication trait

`SegmentBackup` (`merkle_log.rs:230-246`) is the trait an operator
implements to push sealed segments to S3, IPFS, etc., and to pull them
back for replay-restore. The protocol does not mandate a backend; the
segment file format and trailer hash are the wire contract.

---

## 10. Privacy classes

`crates/emem-core/src/privacy.rs:18-41`. Every band declares a
`PrivacyClass`; responders enforce it before serving facts.

| Class | Wire form | Behaviour |
|-------|-----------|-----------|
| `Public` | `{"class":"public"}` | unrestricted at any resolution |
| `AggregateOnly { min_res }` | `{"class":"aggregate_only","min_res":11}` | snap up to coarser-than-or-equal-to `min_res`; responses MUST mark `privacy_snapped: true` |
| `L2OnlyWithModelCid` | `{"class":"l2_only_with_model_cid"}` | only at conformance L2; requires `Source.cid` of the model checkpoint |
| `Prohibited` | `{"class":"prohibited"}` | conforming responder MUST refuse |

The discriminator is `class` (`#[serde(tag = "class", rename_all =
"snake_case")]`), CBOR-encoded as the leading map key.

### 10.1 permits_resolution

`privacy.rs:43-55`:

```rust
pub fn permits_resolution(self, requested_res: u8, conformance_l2: bool) -> bool {
    match self {
        PrivacyClass::Public => true,
        PrivacyClass::AggregateOnly { min_res } => requested_res <= min_res,
        PrivacyClass::L2OnlyWithModelCid => conformance_l2,
        PrivacyClass::Prohibited => false,
    }
}
```

A request at finer resolution than `min_res` does not silently fall
through. The responder either snaps to `min_res` (and stamps
`privacy_snapped: true` in the response payload) or rejects the
request — the choice is the responder's, but it MUST announce which
happened. Silent fallthrough would violate the no-silent-fallbacks
contract: an agent seeing an empty result cannot tell whether the
band was prohibited or simply absent.

---

## 11. Claim algebra

`crates/emem-claim/src/lib.rs`.

```rust
struct Claim {
    band: String,
    op: Op,
    value: ciborium::Value,
    tslot: Option<u64>,            // one of tslot|window MUST be set
    window: Option<[u64; 2]>,
    agg: Option<String>,           // any|all|mean|min|max
}
```

Operators (`Op`, lib.rs:31-55):

| Op | Wire | Meaning |
|----|------|---------|
| `Eq` | `eq` | fact value equals RHS |
| `Ne` | `ne` | not equal |
| `Lt` | `lt` | less than |
| `Le` | `le` | less than or equal |
| `Gt` | `gt` | greater than |
| `Ge` | `ge` | greater than or equal |
| `In` | `in` | RHS is a set; value is a member |
| `Ni` | `ni` | non-member |
| `Exists` | `exists` | a fact exists for `(cell, band, tslot)` |
| `Absent` | `absent` | a confirmed-absence fact exists |

Aggregations over a window (`Claim.agg`): `any`, `all`, `mean`, `min`,
`max`. Either `tslot` or `window` MUST be set (`ClaimError::NoTime`).

A type mismatch between `Claim.value` and the fact's value type is
*decidable* depending on context: in `find_similar.filter` it returns
`false` (candidate filtered out); in `verify` it returns
`ClaimError::TypeMismatch` so the agent can distinguish a typo from a
mismatch. New ops ship under semver — an unknown op MUST surface as a
structured error, not `false`.

---

## 12. Conformance gates: the four CIDs

`/v1/manifests` returns four content-addressed identifiers that every
responder MUST be able to compute from the same JSON inputs:

| CID | Source manifest | Pinned shape |
|-----|-----------------|--------------|
| `bands_cid` | `bands-v0.json` | 35 cube slots summing to exactly 1792 dims; 118 materializer-wired band names route into those slots |
| `algorithms_cid` | `algorithms-v0.json` | 159 algorithms in three kinds (Solo, Combined, Embedding); each entry carries typed `parameters`, citation-bearing `learned_from`, and `prerequisites`, so every algorithm is re-executable against the receipt that cites it. See `docs/agents.md` for the catalog, including the six triple-encoder-consensus entries (`deforestation_triple@1`, `wetland_change_triple@1`, `urban_expansion_triple@1`, `disaster_anomaly_triple@1`, `climate_archetype_triple@1`, `coastal_erosion_triple@1`) |
| `sources_cid` | `sources-v0.json` | 43 source schemes; the majority route through the universal STAC + COG sampler (`cog.rs`), the remainder through HTTPS-JSON, Parquet S3, NCSS CSV, TAR/ZIP, Overpass QL, and PMTiles paths |
| `schema_cid` | `schema-v0.json` | CDDL bundle pinned to `hash="blake3"`, `signature="ed25519"`, `cid_encoding="base32-nopad-lowercase"` |

Recipe (identical for all four):

```
manifest_cid = base32_nopad_lowercase( blake3( canonical_cbor(manifest_json) )[..32] )
```

The conformance test before any test vectors land: an external
implementation reads the same JSON files, runs its own canonical CBOR
encoder + BLAKE3, and produces byte-identical CIDs. If it does not,
no other compatibility claim holds — every fact, every receipt, every
attestation cites these CIDs.

---

## 13. Test vectors

The directory `spec/test_vectors/` is the conformance fixture root.
0.0.6 ships the directory framework only; populating each
sub-directory with JSON-per-vector fixtures (extracted from the
existing crate tests) is coming soon:

- `cell64/`: `(lat, lng) → cell64 string` inputs and outputs.
- `tslot/`: `(unix_seconds, tempo) → tslot` and the text round-trip.
- `vec64/`: the 1792-D fp16 byte sequence and resulting vec64 short.
- `cbor/`: a Fact value in JSON plus its canonical CBOR bytes (hex).
- `cid/`: CBOR-bytes input and the FactCid output.
- `sig/`: receipt preimage bytes + ed25519 keypair seed +
  expected signature.
- `claim_eval/`: claim + cell-fact set → expected boolean / error.
- `derivation/`: parent FactCid set + Derivation recipe → expected
  derivative fact.

This document does not invent vectors. An external implementation
passes the conformance gate by producing byte-identical outputs against
the fixtures (once shipped), against the CIDs in §12, and against the
worked examples in §1.4 and §7.2.

---

## 14. Forward compatibility

Three rules govern how this protocol evolves without breaking deployed
verifiers.

1. **Manifests are content-addressed.** An operator who publishes a
   new `bands-v0.json` ships a new `bands_cid`. Existing facts under
   the old `bands_cid` stay valid forever — they never need to be
   re-signed. A verifier with the receipt's `registry_cid` /
   `schema_cid` knows exactly which manifest set was in force.
2. **Schema migrations live at the manifest level.** A CDDL change
   produces a new `schema_cid`. The CBOR encoder is intentionally not
   versioned in-band: there is no version field in a fact. Two facts
   with different `schema_cid` values describe themselves through
   their respective manifests, not through wire-level discriminators.
3. **Operators add ops/connectors under semver.** New `Claim` ops, new
   `Source` connector kinds, new `Tempo` variants ship in a new
   manifest CID. Old responders that don't recognise an op or a
   connector kind MUST surface a structured error, not silently
   evaluate as `false` or substitute a default.

End of spec.
