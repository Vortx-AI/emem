# emem protocol (v1.2.1)

> The wire spec below is the concrete encoding; the formal object, the
> property table, and the memory algebra live in [the memory model](model.md).

## What this document promises

Bytes on the wire. Anyone implementing emem in another language reads this
document, follows it line by line, and produces byte-identical receipts to
a Rust responder running the same registry CIDs. Where the prose diverges
from `crates/emem-codec`, `crates/emem-fact`, `crates/emem-attest`, or
`crates/emem-storage`, the source is canonical and this document is the
bug. Every encoding rule cites the file and line that defines it.

---

## 1. Cell64: the spatial primitive

A Cell64 is a 64-bit integer that addresses a quantised lat/lng bucket on
the WGS-84 ellipsoid. The wire form is four base-65,536 digits joined by
dots, e.g. `dedi.zaf00.bafi.baba`. The integer form is what gets hashed
and compared; the dotted text form is what shows up inside facts and
receipts.

![address algebra: cell + band + tslot → canonical CBOR → blake3 → 52-character base32 CID](/docs/diagrams/09-address-algebra.svg)
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
  (~1 m) MUST produce the same cell: the cell's grain, not a bug.
- **12 m apart distinguishes** (geo.rs:187-193): two queries
  `1.08e-4°` apart (~12 m) MUST produce different cells.
- **Antimeridian** (geo.rs:178-180): `lng = 179.99` round-trips;
  `lng = -181` wraps to `+179`.
- **Polar clamp** (geo.rs:76): `lat = 95` clamps to `90`.
- **Square at equator** (geo.rs:207-225): bucket extent is 8 to 12 m on
  both axes; lat and lng agree to within 5%.

### 1.7 Legacy 16-bit grid: rejected

Pre-0.0.3 emem used a `GEO_RES = 12` (16-bit-per-axis, ~305 m) grid.
That encoding is **not** decodable by the current codec. The test at
`geo.rs:231-241` constructs a legacy-shaped raw word and confirms
`latlng_from_cell64` returns `Err(CodecError::NotGeoCell)`: the
resolution field changed (12 → 21), `GEO_PREFIX_MASK` keys on it, so
legacy strings fail closed instead of silently misplacing a fact by
hundreds of metres. Implementations MUST NOT serve, accept, or quietly
upgrade legacy cell64 strings.

---

## 2. Tslot: temporal addressing

A `Tslot` is a `u64` bucket index of the Unix timeline at a band's
declared tempo cadence. Defined in `crates/emem-core/src/tslot.rs:19-22`.

### 2.1 Anchor: Unix epoch, not emem epoch

Pre-0.0.3 emem anchored tslot at `2026-01-01T00:00:00Z` (`EMEM_EPOCH_UNIX
= 1_767_225_600`). That broke history: every pre-2026 observation
collapsed to `Tslot(0)`. The current code (tslot.rs:56-68) computes

```
Tslot(unix_seconds.max(0) / tempo.slot_seconds())
```

The constant `EMEM_EPOCH_UNIX` is retained as protocol metadata only;
nothing in the encode path subtracts it. Pre-1970 (negative Unix)
inputs clamp to `Tslot(0)`.

### 2.2 Tempo class

Defined in `tslot.rs:24-37, 43-54`. Five variants:

| Variant | `slot_seconds()` | Cadence | Sample bands |
|---------|------------------|---------|--------------|
| `Static` | 0 | never changes | DEM, Köppen, lcv-1 |
| `Slow` | 31_536_000 | 365 d | Tessera (2017..2024 vintages + `multi_year` 1024-D + `bin128`), soil |
| `Medium` | 2_592_000 | 30 d | NDVI composites |
| `Fast` | 86_400 | 1 d | raw S2 NDVI |
| `UltraFast` | 3_600 | 1 h | weather, traffic |

`Static` returns `Tslot(0)` regardless of input; the slot is
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
FactCid = base32_nopad_lowercase( blake3( canonical_cbor(fact) ) )
```

The **full 32-byte digest**, which is 52 base32 characters. It is not
truncated, and the difference is not cosmetic: a reader who truncates
computes an identifier that can never match a real `fact_cid`, so every
lookup misses and every signature check fails, in a way that looks like a
corrupt corpus rather than a wrong rule.

This page said `[..16]` until 2026-08-13, and so did the agent guide. The
whitepaper's own errata already called that the most consequential error
in the v1 draft, and it was still live in two other documents, which is
the failure this repo keeps finding: a correction lands where it was
noticed and not where it is read. `dpwotikn` reported the widths as
inconsistent across our documents while building a telescope pipeline,
and they were right.

The truncated rule is real but belongs to two other identifiers:
`entity_cid` and `bundle_cid` are `blake3(...)[..16]`, 26 characters,
because they anchor a reference rather than binding a whole body.

Mutating any field of the fact changes its CBOR bytes and therefore its
FactCid; the round-trip test at `crates/emem-fact/tests/round_trip.rs`
(CBOR → decode → re-encode → byte-equal) pins this.

The same recipe constructs the other newtypes from `cid.rs:25-34`:
`RegistryCid`, `SchemaCid`, `ReasonCid`, `BatchCid`, `CoverageCid`.

### 3.3 Manifest CID

For the eight registries (bands, algorithms, functions, sources,
topics, schema, lcv-1, alphabet) the recipe is identical:

```text
manifest_cid = base32_nopad_lowercase( blake3( canonical_cbor(manifest) )[..32] )
```

Full 32 bytes (52 chars). The bigger size is acceptable here because a
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
the map with already-sorted keys; emem does not re-sort silently.

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

#### Caller-registered derivations (`POST /v1/derive`)

A responder computes derivatives of its own. A caller can also register
one: a value it computed itself over facts the responder holds. The
result is an ordinary `DerivativeFact` with an ordinary
`emem:fact:<cell64>:<fact_cid>` token, which is the point. A world model
built out of emem facts can hand a stranger one string that resolves,
verifies, and names its parents, instead of asserting a conclusion and
asking to be believed. A verifier walks `parents` down to the
responder's own signed measurements.

Three rules make that safe to expose.

**1. Every parent must resolve here.** Each `inputs[]` entry is parsed,
fetched, and checked against the cell it claims. A token naming a fact
this responder does not hold is refused with `404
derive_parent_unresolved` naming the index that failed; a real
`fact_cid` under a false cell is refused with `409
derive_parent_cell_mismatch`. Unvalidated parents would be fake lineage,
which is worse than none: it would let a token's ancestry *look* like it
terminates in signed measurements when it terminates in nothing.

**2. The provenance class must be a caller class.** `model_output` or
`human_curated`, declared by the caller and validated by the responder.
`direct_sensor` and `deterministic_index` are refused with `400
derive_provenance_class_refused`. Those two classes assert
tamper-evidence (a sensor produced this, or anyone can recompute it
from the cited raw source) and the responder can assert neither about
arithmetic it never saw.

**3. It must be attested, and the signature means something narrow.**

> **What the responder's signature attests:** that this attester
> submitted this derivation, over these parent facts, at this time, and
> that the responder stored it. It does **not** attest that the value is
> true. The responder did not compute it and cannot recompute it.

`signer` on the fact is the **responder's** key, because the responder's
own identity is the only one it can sign with, and minting a fact that
claimed the caller's key had signed the fact's canonical CBOR would be a
forgery: the caller signed the derive preimage, not the fact body. The
caller's binding is recorded inside `derivation.args`, hence inside the
`fact_cid`, hence covered by the responder's signature:

```jsonc
"derivation": {
  "fn_key": "same_doy_ndvi_delta@1",     // the CALLER's recipe; never executed here
  "args": {
    "attester_pubkey_b32": "...",        // whose claim this is
    "attester_sig_b32": "...",           // the caller's own ed25519 signature
    "body_hash_hex": "...",              // what that signature covers
    "inputs": ["emem:fact:...", "..."],  // the parent tokens as submitted
    "provenance_class": "model_output",
    "code_cid": null,
    "preimage": "blake3(\"emem.memory_write|derive|/v1/derive|\" || body_hash)",
    "submitted_via": "emem.derive.v1"
  }
}
```

Every field of the signed body is recoverable from the stored fact, so a
third party holding only the fact can rebuild `body_hash`, rebuild the
preimage, and re-check the **caller's** signature without the original
HTTP request. That is the difference between recording "some key signed
something" and proving "this key signed *this* derivation".

##### The preimage

The binding reuses the memory-write attester scheme unchanged (§ see
`crates/emem-primitives/src/memory_acl.rs`), with `verb = "derive"` and
`path = "/v1/derive"`:

```text
sig       = ed25519(blake3("emem.memory_write|derive|/v1/derive|" || body_hash))
body_hash = blake3(cbor(DeriveBody))
```

One scheme, one verifier. The verb is what separates the domains: no
memory-file verb is spelled `derive`, so a signature minted for a file
write cannot be replayed as a derivation or the reverse. The literal
`emem.memory_write` prefix is the domain for every caller-attested write
at this responder, not only file writes; it is spelled that way because
renaming it would invalidate every memory-write signature already
issued.

`cbor(DeriveBody)` is a **definite-length 10-entry map whose keys appear
in declaration order**:

| # | key | encoding |
|---|---|---|
| 1 | `fn_key` | text |
| 2 | `inputs` | array of text, in caller order (order is signed) |
| 3 | `cell` | text |
| 4 | `band` | text |
| 5 | `tslot_window` | array of two unsigned ints |
| 6 | `op` | text |
| 7 | `value` | RFC 8949 §4.2 deterministic; floats canonicalised first |
| 8 | `confidence` | **CBOR float32** (`0xfa` + IEEE-754 binary32) |
| 9 | `provenance_class` | text |
| 10 | `code_cid` | text, or CBOR null when omitted |

Two traps worth stating plainly, because a generic encoder gets both
wrong:

- The outer map is **not** RFC 8949 §4.2.1 key-sorted. It is declaration
  order. A canonical-CBOR library pointed at this map will sort the keys
  and compute the wrong digest.
- `confidence` is a **float32**, not a float64, because
  `DerivativeFact::confidence` is an `f32` and the signed body has to
  match the stored fact bit-for-bit or the reconstruction above breaks.

Nested maps *inside* `value` are sorted by their encoded key bytes, per
§4.2.1, so the digest does not depend on the caller's JSON key order.

**You do not have to implement any of this.** Send the request with no
`attester` block. The `401 derive_attestation_required` response carries
`details.how_to_sign.sign_this.digest_hex`: the exact 32 bytes the
responder will verify against, plus the full byte-level rules and a
worked example. Sign that digest, re-send the byte-identical body with
the signature attached. The digest is a pure function of the request, so
nothing about it is secret and nothing about it expires. The rules above
are published for verifiers rebuilding a digest from a stored fact, and
for clients that would rather sign in one round-trip.

##### Tenancy: derived facts are attester-scoped

A caller-registered derivative is untrusted input, so it must never
reach another agent's default read. It does not, and not because of a
filter:

> `fact_canonical_key` returns `None` for the `Derivative` variant
> (`crates/emem-cache/src/sled_hot.rs`), so a derivative is stored
> content-addressed but is **never written to the canonical index**, the
> multi-attester index, or the scope index.

Every keyed read walks one of those indexes. So `recall`,
`recall_polygon`, `state`, `query_region`, `memory_search` and
`find_similar` cannot return a caller's derivation at all. The absence
is structural rather than six filters someone has to remember to apply.
This is the point rather than a limitation: registering a derivation
buys citation and resolution, not an assertion into the commons.

Two reads reach a derivation, and both require naming it:

1. **Its token.** `POST /v1/memory_token/resolve` (or `GET
   /v1/facts/<fact_cid>`). A derivation is not a new token family, it
   is a fact.
2. **`POST /v1/derived`**, which requires `attester_pubkey_b32`. There is
   no all-attesters form. A required pubkey is a stronger guarantee than
   an optional filter on `recall`, where a future `attester: None` could
   silently come to mean "everyone".

`POST /v1/derive` is **idempotent per `(attester, body_hash)`**. A
repeat submission returns the token already minted for it, flagged
`deduplicated: true`, so retrying a timed-out call is safe. This is an
explicit index rather than a consequence of content-addressing: because
`signed_at` rides on the fact, two identical submissions inside one
second would collapse to a single CID while two a second apart would
not, and "idempotent if you retry fast enough" is not a contract a
caller can code against. Dedup is scoped to the attester: two keys
making the same claim are two claims, and each gets its own token.

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
"I looked and there was nothing; here is what I looked at
(`reason_cid`)". Per the no-silent-fallbacks rule, the API must
distinguish these states; see §10.

#### Signed Absence as a first-class protocol move

Every band that has no data at a cell returns a `NegativeFact`,
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

Every leaf is **promoted by self-hash** before pairing: the leaf
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
3. `emem_attest::merkle_root(&leaves)` must equal `att.batch_root`,
   else `StorageError::AttestationInvalid("merkle root mismatch …")`.
4. Recover `VerifyingKey::from_bytes(&att.attester.0)`.
5. `vk.verify_strict(blake3(batch_root || registry_cid || schema_cid),
   sig)` must succeed, else `AttestationInvalid("bad signature …")`.

Failure → write rejected. The HTTP layer surfaces this as the
`BadSignature` error code from `crates/emem-core/src/error.rs`.

---

## 7. Receipt

![the trust plane: preimage, signature, merkle path, offline verify](/docs/diagrams/10-trust-plane.svg)
*The five-step trust pipeline. Section 7.2 specifies the preimage byte-by-byte; sections 8 and 9 cover the Merkle path and the append-only log.*

`crates/emem-fact/src/receipt.rs:11-58`.

| Field | Type | Notes |
|-------|------|-------|
| `request_id` | `String` | ULID generated per request |
| `served_at` | `String` | ISO 8601 UTC, second precision (`server.rs:194-211`) |
| `primitive` | `String` | namespaced wire form: `"emem.recall"`, `"emem.find_similar"`, `"emem.verify"`, `"emem.query_region"`, `"emem.memory_file"`, `"emem.memory_bundle"`, `"emem.memory_contradictions"`, … (the bare form without `emem.` is internal-only; wire receipts always carry the prefix) |
| `intent` | `Option<String>` | populated when served via `/v1/intent`; omitted from JSON when None |
| `cells` | `Vec<String>` | cell64 strings cited in the response. For `emem.memory_file` primitives, `cells[0]` is the memory path; when an attester block was supplied, `cells[0] = "pubkey:<b32>"` and `cells[1]` is the path |
| `fact_cids` | `Vec<FactCid>` | every fact CID returned. For `emem.memory_file`, `fact_cids[0]` is the new `file_cid`; for `emem.memory_bundle`, the spatial `fact_cids` cited inside the bundle in citation order |
| `as_of` | `Option<AsOfReceipt>` | `{valid_time?: u64, transaction_time?: ISO8601}`. Present only when at least one bi-temporal bound was set on the read; absent for current-state reads so pre-bi-temporal receipts deserialise byte-identically |
| `schema_cid` | `SchemaCid` | active CDDL profile |
| `merkle_proof` | `Option<MerkleProof>` | inclusion proof for `fact_cids[0]` when persisted; omitted from JSON when None. Bound into the signature from `preimage_version: 2` on, so do not strip it (see §7.3.1) |
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
    // Age of the STALEST source cited, seconds. `None` when nothing in
    // the response carries a dated source; never 0 as a stand-in.
    source_freshness_s: Option<u32>,
    was_cached: bool,
}
```

### 7.1 Signature preimage

The exact preimage construction is one function per version in
`crates/emem-attest/src/lib.rs`: `receipt_preimage_v1`, and
`receipt_preimage_v2` for everything signed after 2026-08-05. The signer
(`crates/emem-storage/src/server.rs`) and the verifier (`POST
/v1/verify_receipt`) call the same function you would.

**Current receipts carry `preimage_version: 2`.** Read the version off
the receipt and rebuild under THAT rule; do not assume the current one.
A verifier that hardcodes v1 rejects every receipt signed after the
cutover, and one that hardcodes v2 rejects every receipt signed before
it. Both remain valid and both must verify. `GET /v1/verifier_spec` is
generated from the same constants the signer uses and is the source of
truth if this prose ever disagrees with it.

The signed digest is `blake3` over a domain-separated, tagged,
length-prefixed segment stream:

1. The stream opens with the 17 bytes `emem.preimage.v1\x00`, then
   `u32-LE(len(domain))`, then the domain string. For receipts the
   domain is `receipt`.
2. A scalar segment is `tag:u8 || u32-LE len || bytes`.
3. A list segment is `tag:u8 || u32-LE count || (u32-LE len || bytes)*`.
4. Optional scalar segments are omitted entirely when absent; their tag
   simply does not appear, so presence and absence are unambiguous. An
   empty list is still written as its tag with count 0.

The receipt segments, in tag order (constants live in
`emem_attest::receipt_tag`, `crates/emem-attest/src/lib.rs`):

| Tag | Segment | Kind | Presence |
|-----|---------|------|----------|
| `0x01` | `request_id` | scalar | required |
| `0x02` | `served_at` | scalar | required |
| `0x03` | `scope_hex` | scalar | optional |
| `0x04` | `as_of_hex` | scalar | optional |
| `0x05` | `edges_hex` | scalar | optional |
| `0x06` | `manifest_hex` | scalar | optional |
| `0x07` | `primitive` | scalar | required |
| `0x08` | `cells` | list | required |
| `0x09` | `fact_cids` | list | required |

Each optional digest is the blake3-hex of the canonical CBOR of its
receipt field: `scope_hex` over the non-empty `Scope`, `as_of_hex` over
the bounded `as_of`, `edges_hex` over the sorted `edge_cids`, and
`manifest_hex` over the sorted `source_versions{registry_cid,
schema_cid, bands_cid, sources_cid}`. `bands_cid` rides inside the
manifest digest, so the active band set is transitively attested.

The value that ed25519 signs is `blake3(stream)`, the 32-byte digest,
not the raw stream. The signature is
`ed25519_dalek::SigningKey::sign(digest)` emitted as a 64-byte
`Signature`.

The canonical, code-generated segment table is served at
`GET /v1/verifier_spec` (also `/.well-known/emem-verifier.json`); it is
emitted from the same compiled `emem_attest` constants the signer uses,
so it cannot drift from the wire. Treat it as the source of truth over
any prose here.

**Legacy v0.** Receipts with `preimage_version` absent or `0` were
signed under the pre-cutover untagged pipe rule
`blake3(request_id|served_at|[scope|][as_of|][edges|][manifest|]primitive|cell,*|cid,*)`;
`POST /v1/verify_receipt` still verifies those, but no responder emits
v0 anymore.

   **What the preimage does NOT cover.** The segments above are the
   complete signed surface. Notably **NOT** in the preimage:

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
   needs *"the user asked X and the responder agreed"*: the receipt
   alone does not testify to the resolution-of-intent step.

   **The `as_of` block enters the preimage.** When a read carried a
   bi-temporal bound (`as_of_tslot` and/or `as_of_signed_at`), the
   receipt body carries an `as_of: {valid_time?, transaction_time?}`
   block, and its blake3-hex over canonical CBOR is hashed in as the
   optional `0x04` segment (`as_of_hex`). A verifier recomputes that
   digest from the `as_of` block to reproduce the signed bytes; a
   current-state read omits the block and the segment alike. The bound
   is therefore bound both directly (via `as_of_hex`) and transitively
   (it selects which `fact_cids` the preimage also binds).

   **Per-replica fact identity.** Each Primary / Negative /
   Derivative fact body includes `signed_at` (ISO-8601 wall clock at
   materialisation time), which is part of the canonical CBOR hashed
   into `fact_cid`. Two responders materialising the same
   `(cell, band, tslot)` from byte-identical upstream pixels therefore
   produce **different `fact_cid`s**; this is intentional (each
   responder signs independently under its own identity). The
   cross-replica join key for "does any responder have this
   observation" is the tuple `(cell, band, tslot)`, not `fact_cid`.
   Aggregate fan-out endpoints (notably `POST /v1/recall_polygon`)
   emit one independently signed receipt per cell under
   `by_cell.<cell>.receipt`; the top-level `merged_facts[]` is
   convenience-only and is **not** covered by an aggregate signature.

### 7.2 Worked example: preimage layout

Given a current-state recall receipt (no scope, `as_of`, edges, or
`source_versions`, so all four optional segments are absent):

- `request_id = "01HZX0K9V3"` (ULID, 26 chars in practice; this short
  example is illustrative)
- `served_at = "2026-05-08T11:22:33Z"`
- `primitive = "emem.recall"` (every emitted primitive name is namespaced; `crates/emem-primitives/src/recall.rs:115` calls `sign_receipt("emem.recall", …)`)
- `cells = ["dedi.zaf00.bafi.baba", "dedi.zaf00.bafi.babe"]`
- `fact_cids = ["bn7cabcdefghij1234567890ab"]`

The blake3 stream is the domain prefix followed by the required
segments, each tagged and length-prefixed (bytes shown hex):

```
emem.preimage.v1\x00                    656d656d2e707265696d6167652e763100
u32-LE(7) "receipt"                     07000000 72656365697074
0x01 u32-LE(10) "01HZX0K9V3"            01 0a000000 3031485a58304b395633
0x02 u32-LE(20) "2026-05-08T11:22:33Z"  02 14000000 …
0x07 u32-LE(11) "emem.recall"           07 0b000000 …
0x08 u32-LE(2) (20,cell0)(20,cell1)     08 02000000 14000000 … 14000000 …
0x09 u32-LE(1) (26,fact_cid0)           09 01000000 1a000000 …
```

The signed value is `blake3(stream)`:

```
eea12d3ae157cd40fb427ca0937e8f71a0af8faed3779ace0869764e872fefe3
```

Then `signature = ed25519_sign( blake3(stream) )`. The same receipt
with empty `cells` still writes the `0x08` segment with count 0
(`08 00000000`); its digest is
`da65c4ee7514a370b8158f103a085461850cb5776761781a38d2aeaa278885c0`.
When a read carries a scope, bi-temporal bound, edges, or
`source_versions`, the matching optional segment
(`0x03`/`0x04`/`0x05`/`0x06`) is inserted before `0x07` in tag order,
each the blake3-hex of the field's canonical CBOR (see §7.1).

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
attestation-tree anchor is missing. Under v2 that absence is itself
signed: `merkle_binding_v2(None)` hashes an explicit ABSENT marker, so "I have no proof" is a statement the responder made, not a gap an
intermediary can create.

### 7.3.1 A receipt is byte-for-byte or nothing

The v2 binding has a cost worth stating plainly, because integrators
meet it as a bug report rather than as a design note.

Since v2 covers the proof, and v1 already covered every other field
listed in §7.1, **a receipt cannot be reshaped and still verify.** An
SDK that drops `merkle_proof` because it looks redundant, re-keys a
field, summarises the envelope, or round-trips it through anything
lossy produces `signature_valid: false` on data nobody tampered with.
That is not a defect in the verifier; it is the same property that
stops an intermediary stripping a proof in transit, seen from the
other side. Store and forward the responder's exact bytes.

Only two omissions actually reach a signature failure. Dropping any of
the other fourteen receipt fields is refused by the deserialiser with a
400 `invalid_argument`, and dropping `responder_pubkey_b32` or
`signature_b32` changes nothing because each duplicates a byte-array
field. The two that fall through are v2's own:

- `merkle_proof`, which v2 binds, so its removal changes the preimage.
- `preimage_version`, which deserialises to `0` when absent and so
  silently selects the v0 rule. This is the worse of the two: the
  inclusion proof is untouched and still walks to its root, so the
  response reports `merkle_proof_valid: true` beside a signature it
  calls invalid. That pairing is this failure, not a contradiction.

The failure is worth distinguishing because it is indistinguishable on
the wire from real tampering, and a false "forged" is more expensive
than a missed one: it teaches an agent to distrust the one thing that
was provable. `POST /v1/verify_receipt` names which it is *where it can
prove the difference*. When the responder still holds the receipt's
first fact, it rebuilds the receipt under v2 with the inclusion proof
it recorded, re-checks the same signature, and if that verifies it
reports `reason:
receipt_reshaped_after_signing` with a `failure_detail` instead of
`signature_invalid`. It never returns `valid: true` for such a receipt,
and a body altered anywhere else fails the restored rebuild too, so the
distinction is not a way back to the v1 downgrade
(`crates/emem-attest/src/lib.rs::restoring_a_proof_rescues_only_an_untouched_body`
pins that). An offline verifier holds no recorded proof and cannot make
the distinction at all; `web/verify.html` says so rather than guessing.

### 7.4 Offline verification (Python)

A self-contained verifier that mirrors `emem_attest::receipt_preimage_v1`
byte-for-byte. The segment table it encodes is the one served at
`GET /v1/verifier_spec`.

```python
import json
from blake3 import blake3
from nacl.signing import VerifyKey

receipt = json.load(open("receipt.json"))

# The signing key travels ON the receipt, so verifying needs no second call.
# (`responder` under /.well-known/emem.json is an object describing the
# encoding, not the key itself; `responder_pubkey_b32` there is the key.)
pk_bytes = bytes(receipt["responder"])             # [u8; 32]

def le32(n): return n.to_bytes(4, "little")
def seg(tag, s):
    # str for the text segments, bytes for the merkle sub-preimage below.
    b = s.encode() if isinstance(s, str) else bytes(s)
    return bytes([tag]) + le32(len(b)) + b
def seg_list(tag, items):
    out = bytes([tag]) + le32(len(items))
    for s in items:
        b = s.encode(); out += le32(len(b)) + b
    return out

# Minimal canonical CBOR (mirrors ciborium) for the optional digests.
def cbor_head(m, n):
    mt = m << 5
    if n < 24: return bytes([mt | n])
    if n < 0x100: return bytes([mt | 24, n])
    if n < 0x10000: return bytes([mt | 25]) + n.to_bytes(2, "big")
    if n < 0x100000000: return bytes([mt | 26]) + n.to_bytes(4, "big")
    return bytes([mt | 27]) + n.to_bytes(8, "big")
def cbor_text(s): b = s.encode(); return cbor_head(3, len(b)) + b
def cbor_uint(n): return cbor_head(0, n)
def b3hex(b): return blake3(b).hexdigest()

def manifest_hex(r):
    sv = r.get("source_versions") or {}
    ks = sorted(sv)                                # BTreeMap key order
    if not ks: return None
    body = cbor_head(5, len(ks)) + b"".join(cbor_text(k) + cbor_text(str(sv[k])) for k in ks)
    return b3hex(body)
def edges_hex(r):
    e = sorted(map(str, r.get("edge_cids") or []))
    if not e: return None
    return b3hex(cbor_head(4, len(e)) + b"".join(cbor_text(x) for x in e))
def scope_hex(r):
    sc = r.get("scope") or {}
    p = [(k, str(sc[k])) for k in ("user_id","agent_id","run_id","org_id") if sc.get(k) is not None]
    if not p: return None
    return b3hex(cbor_head(5, len(p)) + b"".join(cbor_text(k) + cbor_text(v) for k, v in p))
def as_of_hex(r):
    a = r.get("as_of") or {}
    body, n = b"", 0
    if a.get("valid_time") is not None:
        body += cbor_text("valid_time") + cbor_uint(a["valid_time"]); n += 1
    if a.get("transaction_time") is not None:
        body += cbor_text("transaction_time") + cbor_text(str(a["transaction_time"])); n += 1
    if n == 0: return None
    return b3hex(cbor_head(5, n) + body)

def merkle_hex(r):
    """v2 sub-preimage over the inclusion proof, or an explicit ABSENT marker.

    Absence is HASHED rather than expressed by omitting the segment. If it
    were omitted, a receipt whose proof was stripped in transit would hash
    identically to one that never had a proof, which is exactly the v1
    behaviour v2 replaces.
    """
    inner = b"emem.preimage.v1\x00" + le32(len("merkle")) + b"merkle"
    p = r.get("merkle_proof")
    if not p:
        return b3hex(inner + seg(0x05, b""))        # ABSENT
    path = b"".join(bytes(h) for h in p["path"])
    inner += seg(0x01, bytes(p["root"]))
    inner += seg(0x02, int(p["leaf_index"]).to_bytes(4, "little"))
    inner += seg(0x03, path)
    # `version` is omitted from the wire when 0 (legacy unprefixed hashing),
    # so the default here has to be 0. Defaulting to 1 rebuilds a digest the
    # responder never signed.
    inner += seg(0x04, bytes([p.get("version", 0)]))
    return b3hex(inner)

# Rebuild under the receipt's OWN version. A verifier that hardcodes either
# version is wrong: v1 receipts are still valid and v2 receipts are current.
version = receipt.get("preimage_version", 0)
if version not in (1, 2):
    raise SystemExit(f"v{version} receipts use the legacy pipe rule, see 7.1")

stream  = b"emem.preimage.v1\x00" + le32(len("receipt")) + b"receipt"
stream += seg(0x01, receipt["request_id"])
stream += seg(0x02, receipt["served_at"])
for tag, h in ((0x03, scope_hex(receipt)), (0x04, as_of_hex(receipt)),
               (0x05, edges_hex(receipt)), (0x06, manifest_hex(receipt))):
    if h: stream += seg(tag, h)
stream += seg(0x07, receipt["primitive"])
stream += seg_list(0x08, receipt.get("cells") or [])
stream += seg_list(0x09, [str(c) for c in (receipt.get("fact_cids") or [])])
if receipt.get("field") is not None:
    stream += seg(0x0a, receipt["field"])
if version >= 2:
    # ALWAYS written in v2, never conditional. Omitting it here is what makes
    # a broken verifier compute the same digest a downgrade attacker does.
    stream += seg(0x0b, merkle_hex(receipt))

digest = blake3(stream).digest()                   # 32 bytes; NOT the raw stream
VerifyKey(pk_bytes).verify(digest, bytes(receipt["signature"]))
```

Legacy receipts (`preimage_version` absent or `0`) instead use the v0
pipe rule from §7.1; `POST /v1/verify_receipt` handles both versions and
is the reference verifier. A verifier that can reproduce this preimage
and run ed25519 verify is the entire trust-rebinding path; no other call
to the responder is required.

### 7.5 Capability binding for memory writes

Memory file writes can carry an `attester: {pubkey_b32, sig_b32}`
block where the signature is computed by the *caller* (not the
responder) over a separate preimage. The responder checks this
signature before persisting the write, and rejects it if the path
namespace belongs to a different attester.

```
attester_preimage = blake3(
    "emem.memory_write|" || verb || "|" || path || "|" || body_hash
)
attester_sig = ed25519_sign(caller_signing_key, attester_preimage)
```

where:

- `verb ∈ {create, str_replace, insert, delete, rename}`
- `path` is the canonical memory path beginning with `/memories/`, the
  path the verb writes *to*: for `rename` that is the `new_path`
- `body_hash = blake3(canonical body bytes)`, where the canonical body
  is defined per verb:

| verb | canonical body |
|---|---|
| `create` | the `file_text` string's UTF-8 bytes |
| `str_replace` | the whole file's UTF-8 bytes *after* the replacement |
| `insert` | the whole file's UTF-8 bytes *after* the insertion |
| `delete` | the empty string: `blake3("")` |
| `rename` | the `old_path` string's UTF-8 bytes |

The body is the *content*, never the JSON envelope the caller POSTs.
For the edit verbs the caller must apply the edit locally and hash the
resulting file, which makes the signature commit to the state the
responder will persist rather than to the patch that produced it.

`rename` binds both ends of the move with one signature: the
destination is the preimage's `path`, the source is its `body_hash`.
Verifying the same signature a second time against a preimage built
from `old_path` would check one signature against two messages, which
no ed25519 signature satisfies, so a responder MUST treat the source as
a namespace question: if `old_path` is under `by_attester`, the
attester key MUST own it, else 403 `memory_namespace_violation`
(`reason: source_namespace`).

The reference implementation is `crates/emem-primitives/src/memory_acl.rs`:

```rust
pub fn attester_preimage(verb: &str, path: &str, body_hash: &[u8; 32]) -> [u8; 32]
pub fn verify_attester(verb: &str, path: &str, body_hash: &[u8; 32],
                       attester: &MemoryAttester) -> AttestationVerdict
```

Namespace ownership: paths under `/memories/by_attester/<pubkey_short>/`
are write-restricted, where `pubkey_short` is the first 8 chars of
`base32_nopad_lower(pubkey_bytes)`. A write with a valid signature
but the wrong namespace returns 403 `memory_namespace_violation`. A
write with an invalid signature returns 401
`memory_attestation_invalid`.

Bare `/memories/...` paths (no `by_attester` segment) are governed by
the responder's memory-write policy, not by the namespace rule. A
release build defaults to refusing every unattested write with 401
`memory_attestation_required`; an operator re-opens the bare namespace
for the unsigned Anthropic memory-tool form with `EMEM_MEMORY_OPEN=1`,
or admits unattested `create` / `str_replace` / `insert` while gating
`delete` and `rename` with `EMEM_MEMORY_HARDEN_DESTRUCTIVE=1`. A
conforming client MUST NOT assume the bare namespace accepts unattested
writes; it MUST treat 401 `memory_attestation_required` as the expected
response and sign. The policy never relaxes `by_attester`, which
requires a valid attester under every setting.

For attested writes, the responder's own receipt (§7.1) carries
`cells = ["pubkey:<b32>", path]`, so the path → attester binding
is reproducible from the receipt body. Two signatures cover the
write: the caller's over `attester_preimage`, and the responder's
over §7.1. The first proves authority over the namespace, the
second proves the responder persisted the bytes.

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

1. The `leaf` argument is the **promoted** leaf `blake3(C || C)`,
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
Data is fsynced before `append` returns; receipts depend on the
durability claim.

`AppendOutcome` (`merkle_log.rs:142-150`) returns `segment_index`,
`offset_in_segment`, and `record_hash`, enough to rebuild a
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

### 9.6 Transparency log {#transparency-log}

The append-only log above proves *durability*; the transparency layer
proves *append-only-ness to a third party*. It uses the
[RFC 6962](https://www.rfc-editor.org/rfc/rfc6962) *tree construction*,
with BLAKE3-256 substituted for SHA-256, over the
log's per-record hashes (each record's trailing `blake3(attestation_cbor)`,
in append order), implemented in `crates/emem-attest/src/translog.rs`.
RFC 6962 §2.1 mandates SHA-256, so this log is **not RFC 6962
conformant** and no Certificate Transparency client, auditor or monitor
interoperates with it; what is borrowed is the construction. The STH
self-reports `"hash": "blake3-256"` for exactly this reason.

It is a **different tree from the batch-root construction** in section
6.1: a lone node is *promoted* to its parent unchanged rather than paired
with itself, so `mth([A,B,C]) != mth([A,B,C,C])` and the hash of the
first `m` entries is a genuine prefix of the hash of the first `n >= m`.
That prefix property is what makes consistency provable; the batch-root
construction (which pairs the odd node with itself, the CVE-2012-2459
shape it dedups to avoid) cannot answer consistency queries.

- Leaf: `blake3(0x00 || entry_hash)`. Node: `blake3(0x01 || left || right)`.
- Leaves come from `AttestationLog::leaf_hashes()` (`merkle_log.rs`), the
  natural append-ordered source.

Three read-only endpoints, all strictly additive (they never mutate state
and the responder is never trusted to self-certify; every proof verifies
offline):

- `GET /v1/log/sth` returns a **Signed Tree Head**: `{tree_size, root_b32,
  signed_at, responder_pubkey_b32, signature_b32}`. The signature is
  ed25519 over `PreimageV1("emem.translog.sth.v1")` with segments
  `1:u64_be(tree_size), 2:root, 3:signed_at, 4:responder_pubkey` (same
  preimage discipline as receipts, section 7.1).
- `GET /v1/log/inclusion?leaf_index=<i>` (or `?entry_hash=<base32>`)
  returns an audit path proving entry `i` is committed under the STH.
  Verify with `translog::verify_inclusion`.
- `GET /v1/log/consistency?first=<m>&second=<n>` returns a proof that the
  tree of size `m` is a prefix of the tree of size `n` (append-only).
  Verify with `translog::verify_consistency` against the `first_root` in
  the STH you pinned at size `m`; a mismatch means the log rewrote
  history.

Usage: pin an STH, then re-request `/v1/log/consistency` later to prove
the log only grew. That is the whole of what it
proves: append-only-ness, to a client that pinned an earlier head and came
back. A first-contact client has no baseline and the endpoints prove it
nothing.

Witness co-signing of STHs is available: an external party counter-signs a
`(tree_size, root)` head via `POST /v1/log/witness` (listed at
`GET /v1/log/witnesses`); the responder records a co-signature only when
the signature verifies and the root matches its own history at that size.
**This does not make split-view equivocation detectable today.** The STH,
the proofs and the witness list are all served by the party a client would
be checking, so an equivocating responder serves each client a
self-consistent view of all three; there is no gossip channel between
clients, and detecting a split view without one is not possible. There is
also no witness allowlist or trust anchor: `POST /v1/log/witness` accepts
any well-formed ed25519 key, so a co-signature proves only that *some* key
signed a `(size, root)` pair, not that the key is independent of the
responder. Treat witness co-signing as a mechanism awaiting an operating
network. A
`fact_cid -> leaf_index` index, so an inclusion proof can be requested by
fact rather than by log position, is the next increment on this substrate.

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
request. The choice is the responder's, but it MUST announce which
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
mismatch. New ops ship under semver; an unknown op MUST surface as a
structured error, not `false`.

---

## 12. Conformance gates: the four CIDs

`/v1/manifests` returns four content-addressed identifiers that every
responder MUST be able to compute from the same JSON inputs:

| CID | Source manifest | Pinned shape |
|-----|-----------------|--------------|
| `bands_cid` | `bands-v0.json` | 43 cube slots summing to exactly 1792 dims; 129 materializer-wired band names (42 user-callable today) route into those slots |
| `algorithms_cid` | `algorithms-v0.json` | 168 algorithms in three kinds (Solo, Combined, Embedding); each entry carries typed `parameters`, citation-bearing `learned_from`, and `prerequisites`, so every algorithm is re-executable against the receipt that cites it. See `docs/agents.md` for the catalog, including the six triple-encoder-consensus entries (`deforestation_triple@1`, `wetland_change_triple@1`, `urban_expansion_triple@1`, `disaster_anomaly_triple@1`, `climate_archetype_triple@1`, `coastal_erosion_triple@1`) |
| `sources_cid` | `sources-v0.json` | 46 source schemes; the majority route through the universal STAC + COG sampler (`cog.rs`), the remainder through HTTPS-JSON, Parquet S3, NCSS CSV, TAR/ZIP, Overpass QL, and PMTiles paths |
| `schema_cid` | `schema-v0.json` | CDDL bundle pinned to `hash="blake3"`, `signature="ed25519"`, `cid_encoding="base32-nopad-lowercase"` |

Recipe (identical for all four):

```
manifest_cid = base32_nopad_lowercase( blake3( canonical_cbor(manifest_json) )[..32] )
```

The conformance test before any test vectors land: an external
implementation reads the same JSON files, runs its own canonical CBOR
encoder + BLAKE3, and produces byte-identical CIDs. If it does not,
no other compatibility claim holds: every fact, every receipt, every
attestation cites these CIDs.

---

## 13. Test vectors

The directory `spec/test_vectors/` is the conformance fixture root.
The 1.x line ships the directory framework only; populating each
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
   the old `bands_cid` stay valid forever; they never need to be
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
