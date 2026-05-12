# emem registries

## Why these are content-addressed

Eight JSON manifests live under `crates/emem-core/data/`, are loaded once via
`include_str!()` at process start, and have a CID computed from the canonical
CBOR encoding of the parsed structure. Every receipt the responder signs cites
those CIDs as `registry_cid` and `schema_cid`. When a manifest changes — a new
band, a new algorithm version, a new mirror in the source registry — the file's
CID changes too. Old facts attested under the old CID still verify against the
old manifest; new facts cite the new one. There is no in-band version number
fighting with the JSON for source-of-truth status: the CID *is* the version.

A new responder can publish its own manifest CID (its own algorithm weights,
its own ordering of mirrors) without recompiling any code in this repo. Manifest
keys are stable across publishers; URLs, weights, thresholds are not.

## The manifest CID rule

```text
manifest_cid = base32_nopad_lowercase( blake3( canonical_cbor(manifest) )[..32] )
```

Implemented at `crates/emem-core/src/manifest.rs:90-97` (`fn manifest_cid`).
`canonical_cbor` is whatever `ciborium::ser::into_writer` produces from the
deserialised Rust struct, which means **two implementations parsing the same
JSON converge on the same CID** as long as their structs deserialise to the
same in-memory shape. The validator (the per-manifest `Manifest::validate`
impl) runs before the CID is taken, so a structurally invalid manifest never
gets a CID — the loader panics at startup.

## The 8 manifests

The eight identifiers are pinned as `pub const` strings in
`crates/emem-core/src/manifest.rs`:

| Identifier              | File                                       | Struct (in `emem-core`)                      | Role                                                                |
|-------------------------|--------------------------------------------|----------------------------------------------|---------------------------------------------------------------------|
| `emem-bands`            | `crates/emem-core/data/bands-v0.json`      | `bands::BandRegistry`                        | 1792-D voxel layout: 35 bands, family + tempo + privacy per slot    |
| `emem-algorithms`       | `crates/emem-core/data/algorithms-v0.json` | `algorithms::AlgorithmRegistry`              | 149 composition recipes (solo / combined / embedding)               |
| `emem-functions`        | `crates/emem-core/data/functions-v0.json`  | `functions::FunctionRegistry`                | 20 derivation functions (17 primary / 2 derivative / 1 negative)    |
| `emem-sources`          | `crates/emem-core/data/sources-v0.json`    | `sources::SourceRegistry`                    | 43 source schemes, ordered providers per scheme                     |
| `emem-topics`           | `crates/emem-core/data/topics-v0.json`     | `topics::TopicRegistry`                      | 26 topics for `/v1/ask` routing (description + aliases + bands)     |
| `emem-schema`           | `crates/emem-core/data/schema-v0.json`     | `schema::SchemaRegistry`                     | 8 CDDL fragments + pinned hash/sig/cid encoding                     |
| `emem-lcv1`             | `crates/emem-core/src/taxonomy.rs`         | `taxonomy::Lcv1` + `LcvFamily`               | 64-leaf land-cover taxonomy (8 families × 8 leaves), u8 encoded     |
| `emem-cell64-alphabet`  | `crates/emem-codec/src/alphabet.rs` (in-code CVCV builder) | (no struct; `build_alphabet_v0()`)  | 65,536 CVCV bigrams (21 consonants × 10 vowels × 21 × 10) padded with `z<hex4>` synthetic suffix |

The first six are JSON+struct pairs validated against `crate::manifest::Manifest`.
`emem-lcv1` is a taxonomy enum — small enough that it lives entirely in code.
`emem-cell64-alphabet` is a binary asset shipped in `emem-codec`.

---

### 1. emem-bands (bands-v0.json)

The 1792-D voxel layout. 35 bands sum to exactly 1792 dims, validated at load.
Each band declares an `offset` and `dims`; the validator at
`bands.rs:163-180` rejects any manifest where `bands[i].offset !=
sum(bands[0..i].dims)` or the total ≠ `total_dims`.

Top-level fields:

| field             | meaning                                                                |
|-------------------|------------------------------------------------------------------------|
| `manifest`        | MUST equal `"emem-bands"`                                              |
| `version`         | `"v0"` today                                                           |
| `total_dims`      | `1792` (validator enforces sum-equality)                               |
| `tempo_classes`   | `["static", "slow", "medium", "fast", "ultra_fast"]`                   |
| `privacy_classes` | `["public", "aggregate_only", "l2_only_with_model_cid", "prohibited"]` |
| `bands[]`         | physical-layout-ordered band records                                   |

Per-band fields (struct: `bands::Band` at `bands.rs:79-130`):

| field             | required | what                                                            |
|-------------------|----------|-----------------------------------------------------------------|
| `key`             | yes      | stable wire-format band key (e.g. `"geotessera"`)               |
| `family`          | yes      | one of 14 `BandFamily` variants                                 |
| `offset`          | yes      | byte-stable offset in the 1792-D cube                           |
| `dims`            | yes      | dimension count (variable per band, e.g. 128 for `geotessera`)  |
| `tempo`           | yes      | one of 5 `Tempo` variants                                       |
| `privacy`         | yes      | one of 4 `PrivacyClass` variants                                |
| `description`     | no       | one paragraph for `/v1/bands` self-explanation                  |
| `units`           | no       | scalar bands' physical units                                    |
| `value_range`     | no       | `[min, max]` for sanity-check                                   |
| `interpretation`  | no       | how to read the value                                           |
| `pitfalls`        | no       | gotchas an agent should know                                    |
| `references`      | no       | newline-joined citation/doc URLs                                |
| `dimensions[]`    | no       | per-slot breakdown for multi-dim bands (`BandDimension`)        |
| `scalar_keys[]`   | no       | dotted keys agents pass to `/v1/recall` (e.g. `indices.ndvi`)   |

The 14 `BandFamily` variants (`bands.rs:23-52`):
`Foundation`, `Optical`, `Radar`, `Terrain`, `Climate`, `Soil`, `Vegetation`,
`Landcover`, `Water`, `Human`, `Vision`, `Topology`, `Encoding`, `Reserved`.

The 5 `Tempo` variants (`tslot.rs:25-37`) come with a fixed slot duration:

| variant      | slot_seconds  | example bands                       |
|--------------|---------------|-------------------------------------|
| `static`     | 0             | `cop_dem`, `koppen`, `topology`     |
| `slow`       | 31_536_000    | `geotessera`, `overture`, `nightlights` |
| `medium`     | 2_592_000     | `terraclimate`, `climate`           |
| `fast`       | 86_400        | `sentinel2_raw`, `indices`, `air_quality` |
| `ultra_fast` | 3_600         | source schemes `met_no`, `open_meteo` |

The 4 `PrivacyClass` variants (`privacy.rs:18-41`):

- `public` — unrestricted
- `aggregate_only { min_res }` — must snap to ≥ `min_res` before serving
- `l2_only_with_model_cid` — admissible only at conformance L2 with a model CID
- `prohibited` — refuse to serve

#### First five bands (head of the layout)

| key             | family     | offset | dims | tempo  | privacy   | what                                                               |
|-----------------|------------|--------|------|--------|-----------|--------------------------------------------------------------------|
| `geotessera`    | foundation | 0      | 128  | slow   | public    | Tessera annual fp16 embedding; default cosine surface              |
| `overture`      | human      | 128    | 64   | slow   | public    | Per-cell aggregate of Overture buildings/places/transportation     |
| `air_quality`   | climate    | 192    | 7    | fast   | public    | CAMS PM2.5/PM10/NO2/O3/SO2/CO + AOD-550 via Open-Meteo             |
| `_reserved_512` | foundation | 199    | 505  | slow   | public    | Forward-compat reservation; recall returns Absence                 |
| `sentinel2_raw` | optical    | 704    | 10   | fast   | public    | Sentinel-2 L2A reflectance × 10 000, ten canonical bands B02..B12  |

The cube layout is byte-stable: any change must preserve subsequent offsets.
That's why the AlphaEarth slot was renamed `_reserved_512` rather than removed
when Google's Requester-Pays gating closed off the no-key path.

The `geotessera` band's `dims = 128` does NOT mean Tessera was published as
128-D — Tessera publishes as int8 + per-pixel f32 scale; the recall side
decodes to f32 before cosine scoring. The responder ships eight annual
vintages addressed as `geotessera.{2017..2024}` (each 128-D), plus
`geotessera.bin128` (sign-bit binarised) and `geotessera.multi_year`
(1024-D = 8×128 stacked, zero-padded for missing years).

---

### 2. emem-algorithms (algorithms-v0.json)

149 composition recipes. Each is one of three kinds:

| kind        | count | what it composes                                                  |
|-------------|-------|-------------------------------------------------------------------|
| `solo`      | 16    | single band → derived classification or scalar                    |
| `combined`  | 78    | multi-band weighted composite (flagship: `flood_risk`, `water_consensus`) |
| `embedding` | 13    | operations on the geotessera vector (cosine, novelty, change)     |

Per-algorithm fields (struct `algorithms::Algorithm` at `algorithms.rs:258-336`):

| field                       | required | what                                                              |
|-----------------------------|----------|-------------------------------------------------------------------|
| `key`                       | yes      | versioned id, e.g. `"flood_risk@2"`                               |
| `kind`                      | yes      | solo / combined / embedding                                        |
| `domain`                    | no       | editorial routing tag (`"water"`, `"vegetation"`, …)              |
| `inputs[]`                  | yes      | declared inputs (band, role, weight, transform, unit, explanation)|
| `formula`                   | yes      | plain-math source-of-truth string                                 |
| `output`                    | yes      | `{kind: scalar|classification|vector, unit, range, values}`       |
| `when_to_use`               | yes      | editorial agent-routing guidance                                  |
| `primitive`                 | yes      | the REST call (or local op) that gathers inputs                   |
| `deterministic`             | no       | default `true`; explicit `_deterministic_note` when `false`       |
| `citation`                  | yes      | preferred peer-reviewed source                                    |
| `frequency_of_calculation`  | no       | recommended re-computation cadence                                |
| `accuracy_band`             | no       | editorial confidence note (e.g. `"R²~0.4-0.7 (S2 alone)"`)        |
| `multimodal`                | no       | `Multimodal` declaration (anchor band + tier chain)               |
| `temporal_recipe`           | no       | per-window backfill (`flood_event_window` style)                  |
| `evaluation`                | no       | `Expr` AST for in-process deterministic evaluation                |

#### The Expr AST

`algorithms.rs:397-516` defines 15 `Expr` variants. Together they cover every
composition pattern in the registry:

| variant         | shape                                                          |
|-----------------|----------------------------------------------------------------|
| `Band`          | `{op:"band", band:"<key>"}` — leaf lookup                      |
| `Const`         | `{op:"const", value:<f64>}` — leaf literal                     |
| `Add`           | `{op:"add", terms:[…]}` — pointwise sum                        |
| `Sub`           | `{op:"sub", a:…, b:…}`                                         |
| `Mul`           | `{op:"mul", terms:[…]}`                                        |
| `Div`           | `{op:"div", a:…, b:…}` — `b==0` collapses to `None`            |
| `Linear`        | `{op:"linear", weights:{…}, bias:0.0}` — Σ wᵢ·xᵢ + b           |
| `Clamp`         | `{op:"clamp", inner:…, lo:0.0, hi:1.0}`                        |
| `Where`         | `{op:"where", cond:…, gt:5.0, then_:…, else_:…}`               |
| `WeightedBlend` | `{op:"weighted_blend", primary:…, alt:…, alt_weight:…}`        |
| `Abs`           | `{op:"abs", inner:…}`                                          |
| `Sigmoid`       | `{op:"sigmoid", inner:…}` — `1/(1+exp(-x))`                    |
| `Relu`          | `{op:"relu", inner:…}` — `max(0, x)`                           |
| `Max`           | `{op:"max", terms:[…]}`                                        |
| `Min`           | `{op:"min", terms:[…]}`                                        |

Evaluation is pure: given a `samples: HashMap<String, f64>`, the AST reduces
to `Option<f64>`. A missing band collapses the whole expression to `None`
(no silent zeros). Receipts surface this as
`algorithm_outcomes[].skip_reason: "missing_input:<band>"`.

#### Example: flood_risk@2 evaluation tree

```json
{
  "op": "clamp", "lo": 0.0, "hi": 1.0,
  "inner": {
    "op": "add",
    "terms": [
      {"op":"mul","terms":[
        {"op":"const","value":0.55},
        {"op":"div",
         "a":{"op":"band","band":"surface_water.recurrence"},
         "b":{"op":"const","value":100.0}}]},
      {"op":"mul","terms":[
        {"op":"const","value":0.25},
        {"op":"where",
         "cond":{"op":"abs","inner":{
           "op":"sub",
           "a":{"op":"band","band":"copdem30m.elevation_mean"},
           "b":{"op":"band","band":"gmrt.topobathy_mean"}}},
         "gt":5.0,
         "then_":{"op":"const","value":0.5},
         "else_":{"op":"const","value":1.0}},
        {"op":"div",
         "a":{"op":"relu","inner":{
           "op":"sub",
           "a":{"op":"const","value":50.0},
           "b":{"op":"band","band":"copdem30m.elevation_mean"}}},
         "b":{"op":"const","value":50.0}}]},
      {"op":"mul","terms":[
        {"op":"const","value":0.20},
        {"op":"sigmoid","inner":{
          "op":"div",
          "a":{"op":"sub",
               "a":{"op":"const","value":-15.0},
               "b":{"op":"band","band":"sentinel1_raw"}},
          "b":{"op":"const","value":2.0}}}]}
    ]
  }
}
```

The `Where` arm halves the elevation term when Cop-DEM and GMRT disagree by
more than 5 m — a real bug the unweighted v1 had at Katihar (Cop-DEM 31 m vs
GMRT 25 m at the same point).

#### Sensor tier and the 10 m delivery rule

`algorithms.rs:60-115` defines the `SourceTier` enum and a `for_band` mapping
that runs at registry-validate time. Tier ordering:

```text
S1  >  S2  >  Landsat  >  IoT  >  OtherSat  >  Static
```

The validator enforces: any algorithm declaring `multimodal.delivery_resolution_m
<= 10` MUST have at least one S1, S2, or Landsat band in `multimodal.variance_sources`.
Coarse-physics algorithms (SPI on POWER precip, GDD on POWER temperature)
declare honest large resolutions instead. This stops a `"10 m flood risk"`
algorithm from cheating its anchor by swapping in an `era5.*` input.

The `for_band` rules (string-prefix matching):

- `sentinel1_raw`, `s1.*`           → `S1`
- `s2.*`, `indices.*`, `geotessera*` → `S2` (Tessera anchors at the S2 grid)
- `landsat.*`                       → `Landsat`
- `iot.*`                           → `IoT`
- `modis.*`, `cams.*`, `marine.*`, `viirs.*` → `OtherSat`
- everything else (`power.*`, `era5.*`, `weather.*`, `soilgrids.*`, `hansen.*`, `cop_dem.*`, `gmrt.*`, …) → `Static`

---

### 3. emem-functions (functions-v0.json)

20 derivation functions (17 primary / 2 derivative / 1 negative). Each declares:

- which upstream sources it requires (by canonical scheme)
- how to derive the band value (formula, deterministically)
- which band/index it writes

Three kinds (`functions.rs:18-25`):

| kind         | count today | what it produces                                                 |
|--------------|-------------|------------------------------------------------------------------|
| `primary`    | 16          | A `PrimaryFact` directly from upstream sources                   |
| `derivative` | 2           | A `DerivativeFact` from N parent fact CIDs (`op` ∈ delta\|trend) |
| `negative`   | 1           | A `NegativeFact` (signed Absence with reason)                    |

Per-function fields (struct `functions::Function` at `functions.rs:42-74`):

| field              | when                      | what                                                        |
|--------------------|---------------------------|-------------------------------------------------------------|
| `key`              | always                    | versioned id, e.g. `"nv.l2a@1"`                             |
| `kind`             | always                    | primary / derivative / negative                              |
| `out_band`         | always                    | output band key (must exist in band registry)               |
| `out_index`        | when band is multi-dim    | slot index within `out_band.dims`                           |
| `out_unit`         | always                    | physical unit string                                        |
| `sources[]`        | primary, negative         | upstream `SourceRequirement` (scheme + channels + tempo)    |
| `parents_required` | derivative                | required parent count for ops like `delta`                  |
| `parents_min`      | derivative                | minimum parent count for ops like `trend`                   |
| `op`               | derivative                | one of `delta`, `mean`, `trend`, `rate`, `anomaly`           |
| `formula`          | always                    | human-readable formula string                               |
| `deterministic`    | always                    | MUST be `true` — non-deterministic functions are rejected   |
| `reason_template`  | negative                  | template for the `ReasonCid`'s source pointer               |

The validator (`functions.rs:93-134`) refuses non-deterministic entries
outright. The "canonical channel" of emem is deterministic by construction.

#### Examples (one per kind)

```json
{
  "key": "nv.l2a@1", "kind": "primary",
  "out_band": "indices", "out_index": 0, "out_unit": "ratio",
  "sources": [{"scheme":"sentinel2.l2a","channels":["B04","B08"],"tempo":"fast"}],
  "formula": "(b08 - b04) / (b08 + b04)",
  "deterministic": true
}

{
  "key": "nd.delta@1", "kind": "derivative",
  "out_band": "indices", "out_index": 0,
  "op": "delta", "parents_required": 2,
  "formula": "value_b - value_a", "deterministic": true
}

{
  "key": "abs.s1.water@1", "kind": "negative",
  "out_band": "surface_water",
  "sources": [{"scheme":"sentinel1.grd.iw","channels":["VV"],"tempo":"fast"}],
  "formula": "vv_max_below_threshold(-17_db)_implies_absence",
  "deterministic": true,
  "reason_template": "absence.confirmed_by:s1_scene_cid:{scene_cid}"
}
```

---

### 4. emem-sources (sources-v0.json)

43 source-scheme entries. Each scheme has an ordered `providers[]` list — the
dispatcher walks them in order, returning on the first 2xx; the receipt records
which provider actually answered.

The 7 `ConnectorKind` variants (`sources.rs:17-36`):

| kind                 | what it handles                                                   |
|----------------------|-------------------------------------------------------------------|
| `gcs_cog`            | `gs://...` Cloud-Optimized GeoTIFF                                |
| `https_cog_vsicurl`  | HTTPS COG with `Range` reads (Sentinel-2 + Cop-DEM + WorldCover)  |
| `https_geotiff`      | plain HTTPS GeoTIFF download                                      |
| `ipld_cid`           | content-addressed IPLD bundle (no network)                        |
| `stac_pc`            | STAC API (Element84 anonymous, MS PC anonymous-with-SAS)          |
| `https_json_api`     | plain JSON REST (Open-Meteo, NASA POWER, met.no, ORNL DAAC, GMRT, ISRIC) |
| `parquet_s3`         | anonymous S3 Parquet (Overture)                                   |

Per-source fields (struct `sources::SourceScheme` at `sources.rs:69-80`):

| field                  | what                                                |
|------------------------|-----------------------------------------------------|
| `scheme`               | unique key matching the function-registry's `SourceRequirement.scheme` |
| `providers[]`          | failover-ordered list; first 2xx wins               |
| `tempo`                | source's natural cadence string                     |
| `native_resolution_m`  | metres at the source (e.g. 10 for S2, 30 for Cop-DEM, 11_000 for Open-Meteo) |

Per-provider fields (struct `sources::Provider` at `sources.rs:39-60`):

| field             | what                                                       |
|-------------------|------------------------------------------------------------|
| `id`              | provider id (e.g. `"gcs.public"`, `"aws.opendata"`)        |
| `kind`            | one of the 7 `ConnectorKind` values                        |
| `url_template`    | template with `{var}` interpolation (see `template.rs`)    |
| `cid`             | static IPLD CID (only for `ipld_cid` kind)                 |
| `auth`            | `"anonymous"`, `"earthdata_login"`, `"oauth2"`, `"firms_map_key"` |
| `rate_limit_qps`  | soft hint                                                  |
| `license`         | licence string (`"CC-BY-4.0"`, `"Copernicus open data"`, …) |

The validator (`sources.rs:96-123`) rejects duplicate schemes and any scheme
with an empty `providers[]`.

Template variables `template.rs:1-21` resolves: `{cell64}`, `{tslot}`,
`{year}`, `{month}`, `{day}`, `{channel}`, `{lat_band}`/`{lon_band}` (Cop-DEM
1°), `{lat_top10}`/`{lon_left10}` (JRC GSW 10°), `{tile_id}` (ESA WorldCover
3°), `{bbox_csv}` (STAC), `{lat_center}`/`{lon_center}`. Caller-supplied
`vars` override built-ins.

---

### 5. emem-topics (topics-v0.json)

26 topics for routing free-text questions through `/v1/ask` and the MCP
`emem_ask` tool. Each carries:

- `description` — paragraph used to build a sentence-transformer embedding
- `aliases[]` — short example phrases (also feed the embedding pool, also
  serve as substring fallback when transformer is offline)
- `bands[]` — the canonical bands `/v1/ask` recalls when this topic matches
- `algorithms[]` — composition recipes from `algorithms-v0.json` to apply

Routing policy (`topics-v0.json` `_routing` block):

| backend       | what                                                                    |
|---------------|-------------------------------------------------------------------------|
| `ort`         | default. `BAAI/bge-base-en-v1.5` (110 M params, 768-D, MTEB ~63), CLS-pooled, L2-normalised. Reads `tokenizer.json + model.onnx` from `EMEM_TOPIC_MODEL_DIR`. ~110 ms warm /v1/ask end-to-end on CPU. |
| `model2vec`   | fallback. `minishlab/potion-base-8M` static-distillation token-lookup embedder (256-D, ~32 MB, sub-µs per query, no ONNX dep). |
| `keyword`     | deterministic substring search over `aliases[]` + `key`. Selected by `EMEM_TOPIC_BACKEND=keyword`. Also runs as a precision pre-pass in front of the transformer paths. |

Cosine threshold: `0.35` (override via `EMEM_TOPIC_THRESHOLD`).
Max topics returned per question: 5.

Inverse queries the registry exposes:
- `topics_for_band(band_key) → Vec<&Topic>`
- `topics_for_algorithm(algo_key) → Vec<&Topic>`

Both are O(N) over the 25-topic list — small enough that no index is needed.

---

### 6. emem-schema (schema-v0.json)

The CDDL/JSON-fragment bundle the protocol's wire shapes are pinned against.
Top-level fields:

| field           | required | what                                                            |
|-----------------|----------|-----------------------------------------------------------------|
| `manifest`      | yes      | MUST equal `"emem-schema"`                                      |
| `version`       | yes      | `"v0"` today                                                    |
| `fragments[]`   | yes      | one entry per wire shape                                        |
| `hash`          | yes      | MUST equal `"blake3"`                                           |
| `signature`     | yes      | MUST equal `"ed25519"`                                          |
| `cid_encoding`  | yes      | `"base32-nopad-lowercase"`                                      |

The 8 fragments (`schema-v0.json` lines 6-13):

```text
Cell, Tslot, PrimaryFact, DerivativeFact, NegativeFact, Attestation, Receipt, Claim
```

Per-fragment fields:

| field      | required | what                                       |
|------------|----------|--------------------------------------------|
| `name`     | yes      | fragment name (e.g. `"PrimaryFact"`)       |
| `cid_alg`  | yes      | `"blake3-32"`                              |
| `encoding` | yes      | `"canonical-cbor"`                         |

The validator (`schema.rs:48-73`) enforces `hash == "blake3"` and `signature ==
"ed25519"` and refuses an empty `fragments[]`.

---

### 7. emem-lcv1 (taxonomy.rs)

The land-cover taxonomy. 64 leaves arranged as 8 families × 8 leaves, encoded
in a `u8`:

```text
high 5 bits unused | family (3 bits) | leaf (3 bits)
```

The 8 families (`taxonomy.rs:13-30`):

| index | variant         | description                          |
|-------|-----------------|--------------------------------------|
| 0     | `VegClosed`     | Vegetation (closed canopy)           |
| 1     | `VegOpen`       | Vegetation (open / shrub)            |
| 2     | `CropAnnual`    | Cropland (annual)                    |
| 3     | `CropPerennial` | Cropland (perennial / orchard)       |
| 4     | `Built`         | Built / sealed                       |
| 5     | `Bare`          | Bare / sparse                        |
| 6     | `Water`         | Water (inland + coastal)             |
| 7     | `Cryo`          | Snow / ice / wetland                 |

Canonical class identifier: `"lcv-1.f<fam_idx>.l<leaf_idx>"`, e.g.
`"lcv-1.f3.l5"` is leaf 5 of `CropPerennial`. Operators that prefer mnemonic
labels publish a separate label manifest and reference its CID alongside the
taxonomy CID.

---

### 8. emem-cell64-alphabet

The 65,536 bigrams that turn a 64-bit cell ID into a 4-chunk human-typeable
text form (spec §3.2). Each bigram is a CVCV (consonant-vowel-consonant-
vowel) pattern from `21 consonants × 10 vowels × 21 × 10 = 44,100`
combinations, padded to the full 65,536 with `z<hex4>` synthetic suffixes
where the natural alphabet runs out.

The alphabet is **synthesised in code** by `build_alphabet_v0()` at
`crates/emem-codec/src/alphabet.rs`. There is no external binary asset
to ship; the rebuild is deterministic from the consonant/vowel string
constants in the same file (`b/c/d/f/g/h/j/k/l/m/n/p/q/r/s/t/v/w/x/y/z`
× `a/e/i/o/u/A/E/I/O/U`).

---

## Numbers at a glance

| manifest         | count today | invariant the validator enforces                      |
|------------------|-------------|-------------------------------------------------------|
| `emem-bands`     | 35 bands    | sum of `dims` == 1792; offsets contiguous; no dup keys |
| `emem-algorithms`| 149         | no dup keys; deterministic flag honest; tier rule for ≤10 m |
| `emem-functions` | 20          | no dup keys; `deterministic == true` always; sources non-empty for primary/negative; parents_required\|parents_min for derivative |
| `emem-sources`   | 43          | no dup schemes; `providers[]` non-empty                |
| `emem-topics`    | 26          | no dup keys                                            |
| `emem-schema`    | 8 fragments | `hash == "blake3"` and `signature == "ed25519"`         |
| `emem-lcv1`      | 64 leaves   | 8 families × 8 leaves; u8 encoding                     |
| `emem-cell64-alphabet` | 65 536 | in-code CVCV builder, deterministic                    |

---

## How to publish a new manifest

1. **Edit the JSON** in `crates/emem-core/data/`.
2. **Run `cargo test`** — every `Manifest::validate` impl runs as part of the
   crate's tests (`bands::tests::default_loads_and_validates`,
   `functions::tests::default_loads_and_validates`, etc.). A malformed shape
   (offset gap, duplicate key, non-deterministic function, empty providers) is
   rejected before the build emits.
3. **Bump the in-file version field** if the change is not byte-stable. The
   validator does not gate on this; it's purely editorial.
4. **Recompile** — the new CID is computed at process start. No code change is
   required for the registry data itself.
5. **Existing facts under the old CID still verify** because their attestations
   pin the old CID. New attestations cite the new CID. There is no
   global migration step.

Operators running their own responder follow the same steps with their own JSON
files. Fact CIDs and Receipt CIDs remain interoperable; the registry CID
diverges, which is exactly what content-addressing is for.

## How to look up a CID at runtime

```text
GET /v1/manifests
```

Returns a JSON map of `{ identifier → cid }` for all eight manifests. The four
headline CIDs (`emem-bands`, `emem-algorithms`, `emem-sources`, `emem-schema`)
are also embedded in every Receipt as `registry_cid` and `schema_cid` so a
verifier can replay the algorithm CID's evaluation against the same band
manifest the responder used.

A receipt that cites `algorithm_cid: "<cid>"` plus the input fact CIDs is
self-contained: a third party with the same algorithm manifest CID can fetch
the algorithm's `evaluation` AST, recall the cited bands, and re-execute the
expression. If they get the same number, the composition reproduces.

## Where to read the code

| concern                    | file                                              |
|----------------------------|---------------------------------------------------|
| `Manifest` trait + CID rule| `crates/emem-core/src/manifest.rs`                |
| `BandRegistry`             | `crates/emem-core/src/bands.rs`                   |
| `AlgorithmRegistry` + `Expr` AST | `crates/emem-core/src/algorithms.rs`        |
| `FunctionRegistry`         | `crates/emem-core/src/functions.rs`               |
| `SourceRegistry`           | `crates/emem-core/src/sources.rs`                 |
| `TopicRegistry`            | `crates/emem-core/src/topics.rs`                  |
| `SchemaRegistry`           | `crates/emem-core/src/schema.rs`                  |
| `Lcv1` + `LcvFamily`       | `crates/emem-core/src/taxonomy.rs`                |
| `PrivacyClass`             | `crates/emem-core/src/privacy.rs`                 |
| `Tempo` + `Tslot`          | `crates/emem-core/src/tslot.rs`                   |
| Alphabet builder           | `crates/emem-codec/src/alphabet.rs::build_alphabet_v0()` |
