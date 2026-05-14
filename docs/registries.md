# Registries

Eight JSON manifests under `crates/emem-core/data/` form the content-addressed
backbone of an emem responder. Each manifest's blake3 hash (over canonical
CBOR) is the registry CID. Every signed receipt cites those CIDs, so a
verifier can replay an algorithm against the exact manifest set in force at
attestation time.

This document describes each manifest and the contract the validator enforces
before the binary will start.

## Why content-addressed

Eight manifests live under `crates/emem-core/data/`. They load via
`include_str!()` at process start and their CIDs are derived from the
canonical CBOR encoding of the parsed structures. When a manifest changes
(new band, new algorithm version, new mirror), the CID changes. Old facts
attested under the old CID still verify against the old manifest because
the receipt pins the CID inline.

A new responder can publish its own manifest CIDs (its own algorithm weights,
its own provider ordering) without recompiling. Manifest keys are stable
across publishers; URLs, weights, and thresholds are not.

## Manifest CID rule

```text
manifest_cid = base32_nopad_lowercase( blake3( canonical_cbor(manifest) )[..32] )
```

Implemented at `crates/emem-core/src/manifest.rs` (`fn manifest_cid`).
`canonical_cbor` is what `ciborium::ser::into_writer` produces from the
deserialised struct, so two implementations parsing the same JSON converge
on the same CID as long as their structs share the same in-memory shape.
The per-manifest `Manifest::validate` impl runs before the CID is taken, so
a structurally invalid manifest never gets a CID — the loader panics at
startup.

## The eight manifests

| Identifier              | File                                       | Struct (in `emem-core`)             | Count today | Role                                                        |
|-------------------------|--------------------------------------------|-------------------------------------|-------------|-------------------------------------------------------------|
| `emem-bands`            | `data/bands-v0.json`                       | `bands::BandRegistry`               | 35 slots    | 1792-D voxel layout: family, tempo, privacy per slot        |
| `emem-algorithms`       | `data/algorithms-v0.json`                  | `algorithms::AlgorithmRegistry`     | 155         | composition recipes (solo / combined / embedding)           |
| `emem-functions`        | `data/functions-v0.json`                   | `functions::FunctionRegistry`       | 20          | derivation functions (primary / derivative / negative)      |
| `emem-sources`          | `data/sources-v0.json`                     | `sources::SourceRegistry`           | 43 schemes  | ordered providers per scheme                                |
| `emem-topics`           | `data/topics-v0.json`                      | `topics::TopicRegistry`             | 26 topics   | `/v1/ask` routing (description + aliases + bands)           |
| `emem-schema`           | `data/schema-v0.json`                      | `schema::SchemaRegistry`            | 8 fragments | CDDL fragments + pinned hash/sig/cid encoding               |
| `emem-lcv1`             | `src/taxonomy.rs`                          | `taxonomy::Lcv1` + `LcvFamily`      | 64 leaves   | 8 families x 8 leaves land-cover taxonomy, u8 encoded       |
| `emem-cell64-alphabet`  | `crates/emem-codec/src/alphabet.rs`        | `build_alphabet_v0()` (no struct)   | 65 536      | CVCV bigrams padded with `z<hex4>` synthetic suffix         |

The first six are JSON+struct pairs validated against `crate::manifest::Manifest`.
`emem-lcv1` is a Rust enum. `emem-cell64-alphabet` is synthesised in code.

## emem-bands (bands-v0.json)

The 1792-D voxel layout. 35 bands sum to exactly 1792 dims, validated at
load. Each band declares an `offset` and `dims`; the validator at
`bands.rs` rejects any manifest where
`bands[i].offset != sum(bands[0..i].dims)` or the total deviates from
`total_dims`.

Top-level fields:

| Field             | Meaning                                                                |
|-------------------|------------------------------------------------------------------------|
| `manifest`        | MUST equal `"emem-bands"`                                              |
| `version`         | `"v0"`                                                                 |
| `total_dims`      | `1792` (validator enforces sum-equality)                               |
| `tempo_classes`   | `["static", "slow", "medium", "fast", "ultra_fast"]`                   |
| `privacy_classes` | `["public", "aggregate_only", "l2_only_with_model_cid", "prohibited"]` |
| `bands[]`         | physical-layout-ordered band records                                   |

Per-band fields (struct `bands::Band`):

| Field             | Required | Description                                                     |
|-------------------|----------|-----------------------------------------------------------------|
| `key`             | yes      | stable wire band key (e.g. `"geotessera"`)                      |
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
| `dimensions[]`    | no       | per-slot breakdown for multi-dim bands                          |
| `scalar_keys[]`   | no       | dotted keys agents pass to `/v1/recall` (e.g. `indices.ndvi`)   |

The 14 `BandFamily` variants:
`Foundation`, `Optical`, `Radar`, `Terrain`, `Climate`, `Soil`,
`Vegetation`, `Landcover`, `Water`, `Human`, `Vision`, `Topology`,
`Encoding`, `Reserved`.

The 5 `Tempo` variants and their slot duration:

| Variant      | slot_seconds | Example bands                              |
|--------------|--------------|--------------------------------------------|
| `static`     | 0            | `cop_dem`, `koppen`, `topology`            |
| `slow`       | 31_536_000   | `geotessera`, `overture`, `nightlights`    |
| `medium`     | 2_592_000    | `terraclimate`, `climate`                  |
| `fast`       | 86_400       | `sentinel2_raw`, `indices`, `air_quality`  |
| `ultra_fast` | 3_600        | `weather` (MET Norway), `open_meteo`       |

The 4 `PrivacyClass` variants:

- `public` — unrestricted
- `aggregate_only { min_res }` — must snap to >= `min_res` before serving
- `l2_only_with_model_cid` — admissible only at conformance L2 with a model CID
- `prohibited` — refuse to serve

### Cube vs materializer surface

The 35 cube slots are the byte-stable layout pinned by `total_dims = 1792`.
Materializer-wired band names — what shows up under `/v1/coverage_matrix`
and `/v1/materializers` — are denser: 118 distinct keys today, because
multi-dim cube slots fan out to several materializable subkeys (the
`indices` slot expands to `indices.ndvi`, `indices.ndwi`, `indices.evi`,
and so on; `geotessera` expands to per-year vintages plus `bin128` and
`multi_year`).

The cube layout is byte-stable: any change must preserve subsequent offsets.
That is why the AlphaEarth slot was renamed `_reserved_512` rather than
removed when its no-key path closed.

## emem-algorithms (algorithms-v0.json)

155 composition recipes split across three kinds:

| Kind        | Count | What it composes                                                       |
|-------------|-------|------------------------------------------------------------------------|
| `solo`      | 23    | single band -> derived classification or scalar                        |
| `combined`  | 107   | multi-band weighted composite (flagship: `flood_risk@2`, `water_consensus@1`) |
| `embedding` | 25    | operations on a foundation embedding (cosine, novelty, change)         |

### Per-algorithm fields

Struct `algorithms::Algorithm`:

| Field                       | Required | Description                                                        |
|-----------------------------|----------|--------------------------------------------------------------------|
| `key`                       | yes      | versioned id, e.g. `"flood_risk@2"`                                |
| `kind`                      | yes      | solo / combined / embedding                                        |
| `domain`                    | no       | editorial routing tag (`"water"`, `"vegetation"`, ...)             |
| `inputs[]`                  | yes      | declared inputs (band, role, weight, transform, unit, explanation) |
| `formula`                   | yes      | plain-math source-of-truth string                                  |
| `output`                    | yes      | `{kind: scalar|classification|vector, unit, range, values}`        |
| `when_to_use`               | yes      | editorial agent-routing guidance                                   |
| `primitive`                 | yes      | the REST call (or local op) that gathers inputs                    |
| `deterministic`             | no       | default `true`; explicit `_deterministic_note` when `false`        |
| `citation`                  | yes      | preferred peer-reviewed source                                     |
| `frequency_of_calculation`  | no       | recommended re-computation cadence                                 |
| `accuracy_band`             | no       | editorial confidence note                                          |
| `multimodal`                | no       | `Multimodal` declaration (anchor band + tier chain)                |
| `temporal_recipe`           | no       | per-window backfill                                                |
| `evaluation`                | no       | `Expr` AST for in-process deterministic evaluation                 |
| `inference`                 | no       | `InferenceTier` declaration (GPU-required algorithms)              |
| `parameters`                | no       | typed tunable thresholds (see below)                               |
| `learned_from`              | no       | citation provenance for tuned constants                            |
| `prerequisites`             | no       | seed registries / centroid tables the algorithm depends on         |

### The `parameters` + `learned_from` + `prerequisites` contract

The flagship composite algorithms (`flood_risk@2`,
`clay_prithvi_tessera_triple_consensus@1`, the six new triple-consensus
algorithms below) carry their tunable constants as data, not as numeric
literals inside the `formula` string. Three optional blocks make those
constants discoverable, citable, and replaceable:

```jsonc
{
  "key": "flood_risk@2",
  "parameters": {
    "dem_agreement_threshold_m": 5.0,
    "dem_agreement_penalty": 0.5
  },
  "learned_from": {
    "dem_agreement_threshold_m": {
      "value": 5.0,
      "rationale": "Matches the typical Cop-DEM CE90 vertical accuracy ...",
      "source": "ESA Copernicus DEM Product Specification Document, par 5.4 ..."
    }
  }
}
```

`parameters` is a `BTreeMap<String, serde_json::Value>` so a value can be a
bare number, a string, or a `{value, learned_from, rationale}` sub-object.
The Rust accessor `Algorithm::param_f64("consensus_threshold")` unwraps
either shape: a bare number returns directly, an object falls back to
`.get("value").as_f64()`. `Algorithm::param_str` does the same for strings;
`Algorithm::param` returns the raw `Value` for callers that want the full
provenance.

`learned_from` is a free-form object that holds the citation, dataset, or
PR-curve sweep the value came from. The validator does not require it; the
audit value is voluntary. `/v1/algorithms` surfaces both blocks verbatim so
an auditor can trace any number that is not physically obvious back to its
source.

`prerequisites` declares external seed data the algorithm depends on (the
12-cell Köppen-Geiger archetype registry seeds `climate_archetype_triple@1`;
see `crates/emem-core/data/climate_archetype_centroids_v1.json`). When the
prerequisite is unavailable at request time, the dispatcher emits a
structured Absence (`archetype_centroids_unavailable`) rather than crashing.

### Triple-encoder consensus algorithms

Seven algorithms compose Clay v1.5, Prithvi-EO-2.0, and Tessera (and in some
cases Hansen GFC, Overture buildings, or SWIR) into a single agreement
verdict over a 365-day window:

| Key                                          | Domain     | Anchor                                    |
|----------------------------------------------|------------|-------------------------------------------|
| `clay_prithvi_tessera_triple_consensus@1`    | embedding  | three-encoder cosine-change consensus     |
| `deforestation_triple@1`                     | vegetation | three-encoder + Hansen lossyear uplift    |
| `wetland_change_triple@1`                    | water      | three-encoder + JRC GSW recurrence        |
| `urban_expansion_triple@1`                   | human      | three-encoder + Overture buildings + SWIR |
| `disaster_anomaly_triple@1`                  | climate    | three-encoder + FIRMS active fires        |
| `climate_archetype_triple@1`                 | climate    | three-encoder + Köppen-Geiger centroids   |
| `coastal_erosion_triple@1`                   | water      | three-encoder + JRC GSW shoreline mask    |

Every triple algorithm uses the same shape: per-encoder cosine change vs a
365-day lookback, gated by a `consensus_threshold` parameter, fused into an
ensemble score plus a discrete agreement label (`one_or_none`,
`two_of_three`, `all_three`, or a domain-specific uplift like
`hansen_confirmed`). See `protocol.md` for the formula DSL at the wire level.

### Expr AST

Some algorithms carry an in-process `evaluation` block — an `Expr` AST that
the responder can reduce to a scalar once it has the input facts. 15 `Expr`
variants cover every composition pattern in the registry that uses the AST:

| Variant         | Shape                                                          |
|-----------------|----------------------------------------------------------------|
| `Band`          | `{op:"band", band:"<key>"}` — leaf lookup                      |
| `Const`         | `{op:"const", value:<f64>}` — leaf literal                     |
| `Add`           | `{op:"add", terms:[...]}` — pointwise sum                      |
| `Sub`           | `{op:"sub", a:..., b:...}`                                     |
| `Mul`           | `{op:"mul", terms:[...]}`                                      |
| `Div`           | `{op:"div", a:..., b:...}` — `b==0` collapses to `None`        |
| `Linear`        | `{op:"linear", weights:{...}, bias:0.0}`                       |
| `Clamp`         | `{op:"clamp", inner:..., lo:0.0, hi:1.0}`                      |
| `Where`         | `{op:"where", cond:..., gt:5.0, then_:..., else_:...}`         |
| `WeightedBlend` | `{op:"weighted_blend", primary:..., alt:..., alt_weight:...}`  |
| `Abs`           | `{op:"abs", inner:...}`                                        |
| `Sigmoid`       | `{op:"sigmoid", inner:...}`                                    |
| `Relu`          | `{op:"relu", inner:...}`                                       |
| `Max`           | `{op:"max", terms:[...]}`                                      |
| `Min`           | `{op:"min", terms:[...]}`                                      |

Evaluation is pure: given `samples: HashMap<String, f64>`, the AST reduces
to `Option<f64>`. A missing band collapses the whole expression to `None`
(no silent zeros). Receipts surface this as
`algorithm_outcomes[].skip_reason: "missing_input:<band>"`. The triple-
consensus algorithms use the wider formula DSL (cosine over embedding
vectors, `slice_latest`/`slice_prev`, `recall(...)` calls) rather than the
scalar `Expr` AST — re-executability there comes from the formula string and
the pinned `parameters` block, not from an inline AST.

### Sensor tier and the 10 m delivery rule

`algorithms.rs` defines a `SourceTier` enum:

```text
S1 > S2 > Landsat > IoT > OtherSat > Static
```

The validator enforces: any algorithm declaring
`multimodal.delivery_resolution_m <= 10` MUST have at least one S1, S2, or
Landsat band in `multimodal.variance_sources`. Coarse-physics algorithms
(SPI on POWER precip, GDD on POWER temperature) declare honest large
resolutions instead. This stops a `"10 m flood risk"` algorithm from
cheating its anchor by swapping in an `era5.*` input.

The `for_band` rules (string-prefix matching):

| Prefix                                              | Tier       |
|-----------------------------------------------------|------------|
| `sentinel1_raw`, `s1.*`                             | `S1`       |
| `s2.*`, `indices.*`, `geotessera*`                  | `S2`       |
| `landsat.*`                                         | `Landsat`  |
| `iot.*`                                             | `IoT`      |
| `modis.*`, `cams.*`, `marine.*`, `viirs.*`          | `OtherSat` |
| everything else (`power.*`, `era5.*`, `soilgrids.*`, ...) | `Static`   |

## emem-functions (functions-v0.json)

20 derivation functions (17 primary, 2 derivative, 1 negative). Each
declares which upstream sources it requires (by canonical scheme), how to
derive the band value (formula, deterministically), and which band/index
it writes.

Three kinds (`functions.rs`):

| Kind         | Count | What it produces                                                 |
|--------------|-------|------------------------------------------------------------------|
| `primary`    | 17    | A `PrimaryFact` directly from upstream sources                   |
| `derivative` | 2     | A `DerivativeFact` from N parent fact CIDs (`op` in delta\|trend\|...)|
| `negative`   | 1     | A `NegativeFact` (signed Absence with reason)                    |

Per-function fields:

| Field              | When                | Description                                                 |
|--------------------|---------------------|-------------------------------------------------------------|
| `key`              | always              | versioned id, e.g. `"nv.l2a@1"`                             |
| `kind`             | always              | primary / derivative / negative                             |
| `out_band`         | always              | output band key (must exist in band registry)               |
| `out_index`        | multi-dim out_band  | slot index within `out_band.dims`                           |
| `out_unit`         | always              | physical unit string                                        |
| `sources[]`        | primary, negative   | upstream `SourceRequirement` (scheme + channels + tempo)    |
| `parents_required` | derivative          | required parent count for ops like `delta`                  |
| `parents_min`      | derivative          | minimum parent count for ops like `trend`                   |
| `op`               | derivative          | one of `delta`, `mean`, `trend`, `rate`, `anomaly`          |
| `formula`          | always              | human-readable formula string                               |
| `deterministic`    | always              | MUST be `true` — non-deterministic functions are rejected   |
| `reason_template`  | negative            | template for the `ReasonCid`'s source pointer               |

The validator refuses non-deterministic entries outright. The canonical
channel of emem is deterministic by construction.

Examples (one per kind):

```jsonc
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

## emem-sources (sources-v0.json)

43 source-scheme entries. Each scheme has an ordered `providers[]` list; the
dispatcher walks them in order and returns on the first 2xx. The receipt
records which provider actually answered.

Seven `ConnectorKind` variants (`sources.rs`):

| Kind                 | What it handles                                                   |
|----------------------|-------------------------------------------------------------------|
| `gcs_cog`            | `gs://...` Cloud-Optimized GeoTIFF                                |
| `https_cog_vsicurl`  | HTTPS COG with `Range` reads                                      |
| `https_geotiff`      | plain HTTPS GeoTIFF download                                      |
| `ipld_cid`           | content-addressed IPLD bundle (no network)                        |
| `stac_pc`            | STAC API (Element84 anonymous, MS PC anonymous-with-SAS)          |
| `https_json_api`     | plain JSON REST (Open-Meteo, NASA POWER, met.no, ORNL DAAC, ...)  |
| `parquet_s3`         | anonymous S3 Parquet (Overture)                                   |

Per-source fields (struct `sources::SourceScheme`):

| Field                  | Description                                                |
|------------------------|------------------------------------------------------------|
| `scheme`               | unique key matching the function registry's `SourceRequirement.scheme` |
| `providers[]`          | failover-ordered list; first 2xx wins                      |
| `tempo`                | source's natural cadence string                            |
| `native_resolution_m`  | metres at the source                                       |

Per-provider fields (struct `sources::Provider`):

| Field             | Description                                                |
|-------------------|------------------------------------------------------------|
| `id`              | provider id (e.g. `"gcs.public"`, `"aws.opendata"`)        |
| `kind`            | one of the 7 `ConnectorKind` values                        |
| `url_template`    | template with `{var}` interpolation                        |
| `cid`             | static IPLD CID (only for `ipld_cid` kind)                 |
| `auth`            | `"anonymous"`, `"earthdata_login"`, `"oauth2"`, `"firms_map_key"` |
| `rate_limit_qps`  | soft hint                                                  |
| `license`         | licence string (`"CC-BY-4.0"`, `"Copernicus open data"`, ...) |

The validator rejects duplicate schemes and any scheme with an empty
`providers[]`. Template variables resolved by `template.rs` include
`{cell64}`, `{tslot}`, `{year}`, `{month}`, `{day}`, `{channel}`,
`{lat_band}`/`{lon_band}`, `{tile_id}`, `{bbox_csv}`,
`{lat_center}`/`{lon_center}`. Caller-supplied `vars` override built-ins.

See `data-sources.md` for the full inventory of which schemes have a wired
connector and which are declared but deferred.

## emem-topics (topics-v0.json)

26 topics for routing free-text questions through `/v1/ask` and the MCP
`emem_ask` tool. 11 are fully wired today (every band the topic cites has
a live materializer); the rest carry partial wiring or aspirational bands
deferred for connector work.

Each topic carries:

- `description` — paragraph used to build a sentence-transformer embedding
- `aliases[]` — short example phrases (also feed the embedding pool and
  serve as substring fallback when the transformer is offline)
- `bands[]` — the canonical bands `/v1/ask` recalls when this topic matches
- `algorithms[]` — composition recipes from `algorithms-v0.json` to apply

Routing policy (`topics-v0.json._routing`):

| Backend       | Behaviour                                                                |
|---------------|--------------------------------------------------------------------------|
| `ort`         | default. BAAI/bge-base-en-v1.5 (110 M params, 768-D, MTEB ~63), CLS-pooled, L2-normalised. Reads `tokenizer.json + model.onnx` from `EMEM_TOPIC_MODEL_DIR`. ~110 ms warm `/v1/ask` end-to-end on CPU. The crate uses `ort` + `tokenizers` directly; the older fastembed-rs wrapper has been removed. |
| `model2vec`   | fallback. `minishlab/potion-base-8M` static-distillation token-lookup embedder (256-D, ~32 MB, sub-microsecond per query, no ONNX dep). |
| `keyword`     | deterministic substring search over `aliases[]` + `key`. Selected by `EMEM_TOPIC_BACKEND=keyword`. Also runs as a precision pre-pass in front of the transformer paths. |

Cosine threshold: `0.35` (override via `EMEM_TOPIC_THRESHOLD`). Max topics
returned per question: 5.

The 0.35 threshold carries a `_threshold_learned_from` block in
`topics-v0.json` that names the eval corpus
(`tests/comprehensive/questions_v2.json`, 105 questions) and the
PR-curve-sweep procedure that picked it. Re-derive against your own corpus
by running the sweep with `EMEM_TOPIC_BACKEND=keyword
EMEM_TOPIC_THRESHOLD=<x>` and picking the `x` that maximises precision at
recall >= 0.80. This is the same provenance pattern as algorithm
`parameters.learned_from` (see above).

Inverse queries the registry exposes:

- `topics_for_band(band_key) -> Vec<&Topic>`
- `topics_for_algorithm(algo_key) -> Vec<&Topic>`

Both are O(N) over the 26-topic list — small enough that no index is needed.

## emem-schema (schema-v0.json)

The CDDL/JSON-fragment bundle the protocol's wire shapes are pinned against.
Top-level fields:

| Field           | Required | Description                                                |
|-----------------|----------|------------------------------------------------------------|
| `manifest`      | yes      | MUST equal `"emem-schema"`                                 |
| `version`       | yes      | `"v0"`                                                     |
| `fragments[]`   | yes      | one entry per wire shape                                   |
| `hash`          | yes      | MUST equal `"blake3"`                                      |
| `signature`     | yes      | MUST equal `"ed25519"`                                     |
| `cid_encoding`  | yes      | `"base32-nopad-lowercase"`                                 |

The 8 fragments:

```text
Cell, Tslot, PrimaryFact, DerivativeFact, NegativeFact, Attestation, Receipt, Claim
```

Per-fragment fields: `name`, `cid_alg: "blake3-32"`, `encoding:
"canonical-cbor"`. The validator enforces `hash == "blake3"` and
`signature == "ed25519"` and refuses an empty `fragments[]`.

## emem-lcv1 (taxonomy.rs)

The land-cover taxonomy. 64 leaves arranged as 8 families x 8 leaves,
encoded in a `u8`:

```text
high 5 bits unused | family (3 bits) | leaf (3 bits)
```

The 8 families:

| Index | Variant         | Description                          |
|-------|-----------------|--------------------------------------|
| 0     | `VegClosed`     | Vegetation (closed canopy)           |
| 1     | `VegOpen`       | Vegetation (open / shrub)            |
| 2     | `CropAnnual`    | Cropland (annual)                    |
| 3     | `CropPerennial` | Cropland (perennial / orchard)       |
| 4     | `Built`         | Built / sealed                       |
| 5     | `Bare`          | Bare / sparse                        |
| 6     | `Water`         | Water (inland + coastal)             |
| 7     | `Cryo`          | Snow / ice / wetland                 |

Canonical class identifier: `"lcv-1.f<fam_idx>.l<leaf_idx>"`. Operators that
prefer mnemonic labels publish a separate label manifest and reference its
CID alongside the taxonomy CID.

## emem-cell64-alphabet

The 65 536 bigrams that turn a 64-bit cell ID into a 4-chunk human-typeable
text form. Each bigram is a CVCV pattern from
`21 consonants x 10 vowels x 21 x 10 = 44 100` combinations, padded to the
full 65 536 with `z<hex4>` synthetic suffixes.

The alphabet is synthesised in code by `build_alphabet_v0()` at
`crates/emem-codec/src/alphabet.rs`. The rebuild is deterministic from the
consonant/vowel string constants in the same file.

## Numbers at a glance

| Manifest                 | Count today  | Invariant the validator enforces                                  |
|--------------------------|--------------|-------------------------------------------------------------------|
| `emem-bands`             | 35 slots     | sum of `dims` == 1792; offsets contiguous; no dup keys            |
| `emem-algorithms`        | 155          | no dup keys; deterministic flag honest; tier rule for <=10 m      |
| `emem-functions`         | 20           | no dup keys; `deterministic == true` always; sources non-empty for primary/negative |
| `emem-sources`           | 43 schemes   | no dup schemes; `providers[]` non-empty                           |
| `emem-topics`            | 26 topics    | no dup keys                                                       |
| `emem-schema`            | 8 fragments  | `hash == "blake3"` and `signature == "ed25519"`                   |
| `emem-lcv1`              | 64 leaves    | 8 families x 8 leaves; u8 encoding                                |
| `emem-cell64-alphabet`   | 65 536       | in-code CVCV builder, deterministic                               |

For materializer surface (118 distinct band names), see
`/v1/coverage_matrix` and `/v1/materializers` on a running responder.

## Publishing a new manifest

1. Edit the JSON in `crates/emem-core/data/`.
2. `cargo test -p emem-core` — every `Manifest::validate` impl runs in the
   crate tests. A malformed shape (offset gap, duplicate key,
   non-deterministic function, empty providers) is rejected before the
   build emits.
3. Bump the in-file `version` field if the change is not byte-stable. The
   validator does not gate on this; it is editorial.
4. Recompile. The new CID is computed at process start. No code change is
   required for the registry data itself.
5. Existing facts under the old CID still verify because their attestations
   pin the old CID. New attestations cite the new CID. There is no global
   migration step.

Operators running their own responder follow the same steps with their own
JSON files. Fact CIDs and Receipt CIDs remain interoperable; the registry
CID diverges, which is exactly what content-addressing is for.

## Looking up CIDs at runtime

```text
GET /v1/manifests
```

Returns a JSON map of `{ identifier -> cid }` for all eight manifests. The
four headline CIDs (`emem-bands`, `emem-algorithms`, `emem-sources`,
`emem-schema`) are also embedded in every Receipt as `registry_cid` and
`schema_cid`, so a verifier can replay an algorithm's evaluation against
the same band manifest the responder used.

A receipt that cites `algorithm_cid: "<cid>"` plus the input fact CIDs is
self-contained: a third party with the same algorithm manifest can fetch
the algorithm's `parameters`, walk the `evaluation` AST (or read the
`formula` string), recall the cited bands, and re-execute. If they get the
same number, the composition reproduces. These are re-executable
algorithms.

## Where to read the code

| Concern                          | File                                              |
|----------------------------------|---------------------------------------------------|
| `Manifest` trait + CID rule      | `crates/emem-core/src/manifest.rs`                |
| `BandRegistry`                   | `crates/emem-core/src/bands.rs`                   |
| `AlgorithmRegistry` + `Expr` AST | `crates/emem-core/src/algorithms.rs`              |
| `Algorithm::param_f64` accessor  | `crates/emem-core/src/algorithms.rs` (`impl Algorithm`) |
| Climate archetype seed           | `crates/emem-core/data/climate_archetype_centroids_v1.json` |
| `FunctionRegistry`               | `crates/emem-core/src/functions.rs`               |
| `SourceRegistry`                 | `crates/emem-core/src/sources.rs`                 |
| `TopicRegistry`                  | `crates/emem-core/src/topics.rs`                  |
| `SchemaRegistry`                 | `crates/emem-core/src/schema.rs`                  |
| `Lcv1` + `LcvFamily`             | `crates/emem-core/src/taxonomy.rs`                |
| `PrivacyClass`                   | `crates/emem-core/src/privacy.rs`                 |
| `Tempo` + `Tslot`                | `crates/emem-core/src/tslot.rs`                   |
| Alphabet builder                 | `crates/emem-codec/src/alphabet.rs::build_alphabet_v0()` |
