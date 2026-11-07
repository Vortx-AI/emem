# Registries

A registry is a JSON manifest that tells the responder what a band, an
algorithm, a function or a source is. Each registry has a content ID (its
manifest CID). A receipt pins the manifest CIDs that were in force when the
responder signed it. A verifier can then replay an algorithm against the same
manifests.

This page lists each registry, its fields and the rules its validator enforces
before the binary uses it.

## Summary

- Nine JSON manifests are in `crates/emem-core/data/`. Two more registries
  live in code: the land-cover taxonomy and the cell64 alphabet.
- `GET /v1/manifests` returns the CID of the nine JSON manifests, plus
  `registry_cid`. `registry_cid` is the CID of the functions manifest
  (`emem-server.rs` sets it from `functions_cid`), so the two are equal.
- Each receipt carries four of these CIDs in `source_versions`:
  `bands_cid`, `registry_cid`, `schema_cid` and `sources_cid`.
- The `emem-server` binary embeds the JSON files at compile time. To change a
  manifest, edit the JSON and rebuild. There is no runtime override.
- Band and algorithm keys are the names that agents send, so pin them.
  URLs, weights and thresholds inside a manifest can change with a new
  manifest CID.
- The tenth JSON file, `climate_archetype_centroids_v1.json`, is seed data
  for one algorithm. It is not a manifest and has no CID.

## How to compute a manifest CID

```text
manifest_cid = base32_nopad_lowercase( blake3( canonical_cbor(manifest) )[..32] )
```

The code is `manifest_cid` in `crates/emem-core/src/manifest.rs`.
`canonical_cbor` is the output of `ciborium::ser::into_writer` for the
parsed Rust struct. The bytes follow the struct field order, not the JSON key
order. A CBOR encoding of the JSON as written gives a different CID.
[Protocol §12](protocol.md#12-conformance-gates-the-four-cids) shows how to
reproduce `bands_cid`.

When a manifest changes (a new band, a new algorithm version, a new mirror),
its CID changes. Old facts still verify, because their receipts pin the old
CID.

Each registry type has a `Manifest::validate` function. The parser calls it
before anything takes the CID. The embedded registries load through
`LazyLock`. If an embedded file fails to parse or validate, the process
panics on first use (`"embedded bands-v0.json is malformed"`).

Another operator can publish different manifests (other weights, another
provider order). They edit the JSON files and build their own binary. Their
registry CIDs then differ from emem.dev. Fact CIDs and receipt formats stay
compatible.

## The registries

Counts are from `crates/emem-core/data/` on 7 November 2026. On the same day,
the `covers` object of the live `GET /v1/manifests` agreed. It gave
algorithms 172, bands 43, device platforms 17, sources 46, substrates 18 and
trace encodings 8.

| Identifier | Source | Struct in `emem-core` | Count | Role |
|---|---|---|---|---|
| `emem-bands` | `data/bands-v0.json` | `bands::BandRegistry` | 43 slots | 1792-D voxel layout: family, tempo, privacy per slot |
| `emem-algorithms` | `data/algorithms-v0.json` | `algorithms::AlgorithmRegistry` | 172 | composition recipes (solo, combined, embedding) |
| `emem-functions` | `data/functions-v0.json` | `functions::FunctionRegistry` | 23 | derivation functions (primary, derivative, negative) |
| `emem-sources` | `data/sources-v0.json` | `sources::SourceRegistry` | 46 schemes | ordered providers per scheme |
| `emem-topics` | `data/topics-v0.json` | `topics::TopicRegistry` | 27 topics | `/v1/ask` routing (description, aliases, bands) |
| `emem-schema` | `data/schema-v0.json` | `schema::SchemaRegistry` | 8 fragments | CDDL fragments and the pinned hash, signature and CID encoding |
| `emem-substrates` | `data/substrates-v0.json` | `substrates::SubstrateRegistry` | 18 profiles | per contributor class: admission rule, required trace layers, grain |
| `emem-device-platforms` | `data/device-platforms-v0.json` | `device_platforms::DevicePlatformRegistry` | 17 platforms, 7 families | admitted device platforms: family, root of trust, anchors, encodings |
| `emem-trace-encodings` | `data/trace-encodings-v0.json` | `trace_encodings::TraceEncodingRegistry` | 8 encodings | known capture toolchains: the layers each can capture, the tracer's own integrity |
| `emem-lcv1` | `src/taxonomy.rs` | `taxonomy::Lcv1`, `LcvFamily` | 64 leaves | 8 families x 8 leaves land-cover taxonomy, `u8` encoded |
| `emem-cell64-alphabet` | `crates/emem-codec/src/alphabet.rs` | `build_alphabet_v0()` (no struct) | 65 536 | CVCV bigrams, padded with a `z<hex4>` suffix |

The last two have no JSON file and are not in `GET /v1/manifests`. The binary
pins them in code.

### Substrates, device platforms and trace encodings

`emem-substrates` is the substrate profile registry behind the encoder trust
layer (`docs/plans/encoder-substrates.md`). On 7 November 2026 it had one
`active` profile and 17 `candidate` profiles. The active profile is
`earth.satellite.v0`: the open public archive, which anyone can recompute.
Each candidate profile names the OS trace layers that a device of that class
must present before the responder admits its output.

The satellite class appears twice on purpose. `earth.satellite.v0` is the
public archive. `orbital.satellite.v1` is an operator's own constellation
that writes under its own key. That makes it a device substrate, and the
trace rule applies to it.

`emem-device-platforms` says which hardware the trace rule accepts.
`emem-trace-encodings` says which tracers a trace can name.

## emem-bands (bands-v0.json)

The 1792-D voxel layout. The 43 bands sum to exactly 1792 dims. The validator
in `bands.rs` rejects a manifest when any of these is true:

- `bands[i].offset != sum(bands[0..i].dims)`
- two bands have the same `key`
- the sum of `dims` is not `total_dims`

Top-level fields:

| Field | Meaning |
|---|---|
| `manifest` | MUST equal `"emem-bands"` |
| `version` | `"v0"` |
| `total_dims` | `1792` (the validator checks the sum) |
| `tempo_classes` | `["static", "slow", "composite_16day", "composite_8day", "medium", "fast", "ultra_fast"]` |
| `privacy_classes` | `["public", "aggregate_only", "l2_only_with_model_cid", "prohibited"]` |
| `bands[]` | band records in physical layout order |

Per-band fields (struct `bands::Band`):

| Field | Required | Description |
|---|---|---|
| `key` | yes | stable wire band key (for example `"geotessera"`) |
| `family` | yes | one of 14 `BandFamily` variants |
| `offset` | yes | stable offset in the 1792-D cube |
| `dims` | yes | dimension count (for example 128 for `geotessera`) |
| `tempo` | yes | one of 7 `Tempo` variants |
| `provenance_class` | no | how the source makes the band's values (one of 7 `ProvenanceClass` variants); default `unclassified` |
| `privacy` | yes | one of 4 `PrivacyClass` variants |
| `description` | no | one paragraph for `/v1/bands` |
| `units` | no | physical units of a scalar band |
| `value_range` | no | `[min, max]` for a sanity check |
| `interpretation` | no | how to read the value |
| `pitfalls` | no | known traps for an agent |
| `references` | no | citation and document URLs, one per line |
| `dimensions[]` | no | per-slot breakdown for a multi-dim band |
| `scalar_keys[]` | no | dotted keys that agents send to `/v1/recall` (for example `indices.ndvi`) |
| `scalar_units` | no | map from a scalar key to its unit; left out of the CBOR when empty |

Every field in this table is part of the `Band` struct and serializes into
`bands_cid`. This includes `provenance_class`, `scalar_units` and the
optional text fields (`description`, `interpretation`, `pitfalls`,
`references`). A change to them changes
`bands_cid`. The `_note` in `bands-v0.json` says otherwise; the code is the
authority ([protocol §12](protocol.md#12-conformance-gates-the-four-cids)).

The 14 `BandFamily` variants are `Foundation`, `Optical`, `Radar`,
`Terrain`, `Climate`, `Soil`, `Vegetation`, `Landcover`, `Water`, `Human`,
`Vision`, `Topology`, `Encoding` and `Reserved`. On 7 November 2026 the
cube used 13 of them. No slot used `Vision`.

The 7 `Tempo` variants (`crates/emem-core/src/tslot.rs`, `slot_seconds`):

| Variant | slot_seconds | Cube slots | Example bands |
|---|---|---|---|
| `static` | 0 | 12 | `cop_dem`, `koppen`, `topology` |
| `slow` | 31 536 000 (365 days) | 19 | `geotessera`, `overture`, `nightlights` |
| `composite_16day` | 1 382 400 | 0 | MODIS 16-day NDVI composites |
| `composite_8day` | 691 200 | 0 | MODIS 8-day LST, ET, GPP, LAI composites |
| `medium` | 2 592 000 (30 days) | 2 | `terraclimate`, `climate` |
| `fast` | 86 400 | 10 | `sentinel2_raw`, `indices`, `air_quality` |
| `ultra_fast` | 3 600 | 0 | materializer bands `weather.temperature_2m`, `weather.cloud_cover` |

The 4 `PrivacyClass` variants:

| Variant | Rule in the type | Cube slots on 7 November 2026 |
|---|---|---|
| `public` | no limit | 38 |
| `aggregate_only { min_res }` | do not serve finer than `min_res` | 3 (`nightlights`, `ghsl`, `population`, all `min_res: 11`) |
| `l2_only_with_model_cid` | serve only at conformance L2 with a model CID | 2 (`clay_v1`, `prithvi_eo2`) |
| `prohibited` | do not serve | 0 |

emem.dev does not enforce these rules. The check function
`permits_resolution` exists, but no serving path calls it, and no receipt
carries `privacy_snapped`. See
[protocol §10.1](protocol.md#101-permits_resolution).

### Cube slots and materializer bands

The 43 cube slots are the stable layout that `total_dims = 1792` pins. The
band names that materializers serve are a different, larger set. A
multi-dim slot fans out to several keys. For example, the `indices` slot
gives `indices.ndvi`, `indices.ndwi` and `indices.evi`.

On 7 November 2026, emem.dev returned these totals:

| Endpoint | Field | Value |
|---|---|---|
| `GET /v1/materializers` | `pagination.total` | 113 (each entry has a different `band`) |
| `GET /v1/coverage_matrix` | `totals.bands_declared` | 94 |
| `GET /v1/coverage_matrix` | `totals.bands_with_materializer` | 69 |

The cube layout must stay stable. A change must keep every later offset.
For this reason, the AlphaEarth slot (512 dims at offset 192) became
`_reserved_512` when its no-key path closed. Nobody removed the slot. Later
bands took parts of it. `air_quality` took 7 dims at 192 and `clay_v1` took
384 dims at 199. `_reserved_128` (121 dims at 583) holds the rest, so
`sentinel2_raw` still starts at 704. The `bands.rs` tests assert these
offsets.

## emem-algorithms (algorithms-v0.json)

172 composition recipes in three kinds:

| Kind | Count | What it composes |
|---|---|---|
| `solo` | 29 | one band to a derived class or scalar |
| `combined` | 114 | a weighted composite of several bands (for example `flood_risk@2`, `water_consensus@1`) |
| `embedding` | 29 | operations on an embedding: a foundation encoder's, or a field's seasonal memory |

74 of the 172 carry an `evaluation` AST. 8 carry a `parameters` block.

### Per-algorithm fields

Struct `algorithms::Algorithm`:

| Field | Required | Description |
|---|---|---|
| `key` | yes | versioned ID, for example `"flood_risk@2"` |
| `kind` | yes | solo, combined or embedding |
| `domain` | no | editorial routing tag (`"water"`, `"vegetation"`) |
| `inputs[]` | yes | declared inputs (band, role, weight, transform, unit, explanation) |
| `formula` | yes | the formula in plain math; the source of truth |
| `output` | yes | `{kind: scalar, classification or vector, unit, range, values}` |
| `when_to_use` | yes | editorial routing guidance for agents |
| `primitive` | yes | the REST call (or local operation) that gathers the inputs |
| `deterministic` | no | default `true`; a `_deterministic_note` explains a `false` |
| `citation` | yes | preferred peer-reviewed source |
| `frequency_of_calculation` | no | recommended recompute interval |
| `accuracy_band` | no | editorial confidence note |
| `multimodal` | no | `Multimodal` declaration (anchor band and tier chain) |
| `temporal_recipe` | no | per-window backfill |
| `evaluation` | no | `Expr` AST for deterministic evaluation in the process |
| `inference` | no | `InferenceTier` declaration (algorithms that need a GPU) |
| `parameters` | no | typed tunable thresholds (see below) |
| `learned_from` | with `parameters` | where each parameter came from |
| `prerequisites` | no | seed registries or centroid tables that the algorithm needs |

### Parameters, learned_from and prerequisites

Some composite algorithms keep their tunable constants as data, not as
numbers inside `formula`. This is the `flood_risk@2` block, shortened:

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
      "basis": "literature",
      "rationale": "Matches the typical Cop-DEM CE90 vertical accuracy ...",
      "source": "ESA Copernicus DEM Product Specification Document, §5.4 ..."
    },
    "dem_agreement_penalty": {
      "value": 0.5,
      "basis": "operational_default",
      "rationale": "Down-weight applied to the elevation leg ..."
    }
  }
}
```

`parameters` is a `BTreeMap<String, serde_json::Value>`. A value can be a
number, a string, or a `{value, learned_from, rationale}` object.
`Algorithm::param_f64("consensus_threshold")` reads both shapes. It returns
a number directly, or it reads `.get("value").as_f64()` from an object.
`Algorithm::param_str` does the same for strings. `Algorithm::param`
returns the raw `Value`.

The validator requires a `learned_from` entry with a `basis` for each key in
`parameters`. These are the allowed values:

| `basis` | Meaning | Extra fields the validator requires |
|---|---|---|
| `literature` | the value comes from a cited source | none |
| `fitted` | someone fitted the value to data | `estimator`, `corpus_cid`, `held_out` |
| `operational_default` | a person chose the value | none |

A missing or unknown `basis` stops the manifest from loading.
`/v1/algorithms` returns both blocks unchanged. An auditor can trace each
number back to its source.

`prerequisites` names external seed data. For example,
`climate_archetype_triple@1` uses the 12-entry Köppen-Geiger archetype table in
`crates/emem-core/data/climate_archetype_centroids_v1.json`. When that
data is not available, the dispatcher returns a structured Absence
(`archetype_centroids_unavailable`). It does not crash.

### Other rules the validator enforces

- No two algorithms have the same `key`.
- `inputs[]` is not empty.
- `output.kind` is `scalar`, `classification` or `vector`.
- Rules R1 to R4 for a `multimodal` block, and I1 and I2 for an `inference`
  block. See the next sections.

The validator does not check that `deterministic` is true.

### Triple-encoder consensus algorithms

The registry still declares seven triple-encoder algorithms
(`clay_prithvi_tessera_triple_consensus@1`, `deforestation_triple@1` and
five more). The registry is content-addressed and old receipts cite it, so
they stay. They need the retired Clay and Prithvi encoders. None of them
runs on emem.dev.

### Expr AST

An `evaluation` block is an `Expr` AST. The responder reduces it to a scalar
when it has the input facts. `Expr` has 22 variants. The JSON tag is `op`
in snake case.

| Variant | Shape | Result |
|---|---|---|
| `Band` | `{op:"band", band:"<key>"}` | the sample for that band |
| `Const` | `{op:"const", value:<f64>}` | a literal |
| `Add` | `{op:"add", terms:[...]}` | sum |
| `Sub` | `{op:"sub", a:..., b:...}` | `a - b` |
| `Mul` | `{op:"mul", terms:[...]}` | product |
| `Div` | `{op:"div", a:..., b:...}` | `a / b`; `None` when `b == 0` |
| `Linear` | `{op:"linear", weights:{...}, bias:0.0}` | weighted sum plus `bias` |
| `Clamp` | `{op:"clamp", inner:..., lo:0.0, hi:1.0}` | `inner` limited to `[lo, hi]` |
| `Where` | `{op:"where", cond:..., gt:5.0, then_:..., else_:...}` | `then_` if `cond > gt`, else `else_` |
| `WeightedBlend` | `{op:"weighted_blend", primary:..., alt:..., alt_weight:...}` | `(1 - w) * primary + w * alt`, with `w = alt_weight` clamped to `[0, 1]` |
| `Abs` | `{op:"abs", inner:...}` | absolute value |
| `Sigmoid` | `{op:"sigmoid", inner:...}` | `1 / (1 + exp(-inner))` |
| `Relu` | `{op:"relu", inner:...}` | `max(0, inner)` |
| `Max` | `{op:"max", terms:[...]}` | largest term |
| `Min` | `{op:"min", terms:[...]}` | smallest term |
| `Tanh` | `{op:"tanh", inner:...}` | `tanh(inner)` |
| `Exp` | `{op:"exp", inner:...}` | `exp(inner)` |
| `Sqrt` | `{op:"sqrt", inner:...}` | `sqrt(inner)`; `None` when `inner < 0` |
| `Pow` | `{op:"pow", base:..., exp:...}` | `base ^ exp`; `None` when not finite |
| `Log` | `{op:"log", inner:...}` | `ln(inner)`; `None` when `inner <= 0` |
| `OrElse` | `{op:"or_else", primary:..., fallback:...}` | `primary` if it is a finite value, else `fallback` |
| `ClassTable` | `{op:"class_table", inner:..., classes:[[t, v], ...], default:...}` | `v` of the first pair with `t > inner`, else `default` |

Evaluation is pure. Given `samples: HashMap<String, f64>`, the AST reduces to
`Option<f64>`. A missing band makes the expression `None`, never a
silent zero. The one exception is `or_else`: when its `primary` has a
missing band, it returns `fallback`. Before evaluation, the dispatcher
checks the required bands. These are all bands except the bands inside an
`or_else` primary. When one is missing, the receipt shows
`algorithm_outcomes[].skip_reason: "missing_input:<band>"`.

The triple-consensus algorithms do not use `Expr`. They use a wider formula
language (cosine over embedding vectors, `slice_latest` and `slice_prev`,
`recall(...)` calls). For them, the `formula` string and the pinned
`parameters` block are what make the result repeatable.

### Sensor tiers and the 10 m delivery rule

`algorithms.rs` defines a `SourceTier` enum:

```text
S1 > S2 > Landsat > IoT > OtherSat > Static
```

`SourceTier::for_band` maps a band key to a tier by its prefix:

| Band key | Tier |
|---|---|
| `sentinel1_raw`, `s1.*` | `S1` |
| `s2.*`, `indices.*`, `geotessera*`, `prithvi_eo2*`, `galileo_base*`, `clay_v1*` | `S2` |
| `landsat.*` | `Landsat` (no materializer on emem.dev) |
| `iot.*` | `IoT` |
| `modis.*`, `cams.*`, `marine.*`, `viirs.*` | `OtherSat` |
| all other keys (`power.*`, `era5.*`, `soilgrids.*`) | `Static` |

The validator applies four rules to a `multimodal` block. A
`composite_inherit: true` block skips R1, R2 and R3.

| Rule | Requirement |
|---|---|
| R1 | `anchor_band` is one of the `inputs[]` bands, or a `<composite>` reference |
| R2 | if `delivery_resolution_m <= 10`, at least one `variance_sources` band is tier S1, S2 or Landsat |
| R3 | a non-empty `variance_sources` list has at least one band that is not `Static` (fusion method `none` is exempt) |
| R4 | `priority_chain[0]` is the tier of `anchor_band` |

R2 stops a "10 m flood risk" algorithm from using only `era5.*` inputs as
its variance sources. Coarse physics algorithms (SPI on POWER
precipitation, GDD on POWER temperature) declare their true, larger
resolution.

For an `inference` block, I1 requires that `tier` is not `Absence` and that
`tier_chain[0]` equals `tier`. I2 requires `accuracy_band` when a GPU
algorithm uses `degrade_with_tier_label` with a CPU step in its chain.

## emem-functions (functions-v0.json)

23 derivation functions. Each one names the upstream sources it needs (by
scheme), how it derives the band value, and the band and index it writes.

| Kind | Count | What it produces |
|---|---|---|
| `primary` | 20 | a `PrimaryFact` directly from upstream sources |
| `derivative` | 2 | a `DerivativeFact` from parent fact CIDs (`op` is `delta` or `trend` in the file) |
| `negative` | 1 | a `NegativeFact` (a signed Absence with a reason) |

Per-function fields:

| Field | When | Description |
|---|---|---|
| `key` | always | versioned ID, for example `"nv.l2a@1"` |
| `kind` | always | primary, derivative or negative |
| `out_band` | always | output band key |
| `out_index` | multi-dim `out_band` | slot index in `out_band.dims` |
| `out_unit` | always | physical unit string |
| `sources[]` | primary, negative | upstream `SourceRequirement` (scheme, channels, tempo) |
| `parents_required` | derivative | exact parent count, for an op like `delta` |
| `parents_min` | derivative | minimum parent count, for an op like `trend` |
| `op` | derivative | free string; the struct comment lists `delta`, `mean`, `trend`, `rate`, `anomaly` |
| `formula` | always | readable formula string |
| `deterministic` | always | MUST be `true` |
| `reason_template` | negative | template for the source pointer of the `ReasonCid` |

The validator in `functions.rs` rejects a manifest when any of these is true:

- two functions have the same `key`
- a function has `deterministic: false`
- a `primary` or `negative` function has an empty `sources[]`
- a `derivative` function has neither `parents_required` nor `parents_min`

The validator does not check that `out_band` exists in the band registry.

One example of each kind:

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

46 source schemes. Each scheme has an ordered `providers[]` list.
`Dispatcher::fetch` in `crates/emem-fetch/src/lib.rs` tries the providers in
that order and returns the first success. `emem-storage` holds a
`Dispatcher`, but no serving path calls it. The materializers in
`emem-api-rest` call their connectors directly.

A fact names what it read in its `sources[]` list (struct `Source` in
`crates/emem-fact/src/fact.rs`). Each entry has `scheme` and `id`, the
provider's tile or scene ID. It can also have `cid`, `hash`, `captured_at`
and `url`. The receipt has no provider field. On 7 November 2026, a recall
of `copdem30m.elevation_mean` at Mount Fuji gave scheme
`copernicus.dem.30m.aws`, `captured_at`, and the GLO-30 tile URL as both
`id` and `url`.

`ConnectorKind` (`sources.rs`) has 8 variants. The provider counts are from
`sources-v0.json` on 7 November 2026.

| Kind | Providers | What it handles |
|---|---|---|
| `gcs_cog` | 2 | `gs://...` Cloud-Optimized GeoTIFF |
| `https_cog_vsicurl` | 15 | HTTPS COG with `Range` reads |
| `https_geotiff` | 2 | plain HTTPS GeoTIFF download |
| `ipld_cid` | 0 | content-addressed IPLD bundle (no network) |
| `stac_pc` | 9 | STAC API (Element84 anonymous, Microsoft Planetary Computer with SAS) |
| `https_json_api` | 16 | plain JSON REST (Open-Meteo, NASA POWER, met.no, ORNL DAAC) |
| `parquet_s3` | 1 | anonymous S3 Parquet (Overture) |
| `gpu_sidecar` | 3 | retired foundation-model schemes only; no client ships |

Per-scheme fields (struct `sources::SourceScheme`):

| Field | Description |
|---|---|
| `scheme` | unique key; matches `SourceRequirement.scheme` in the function registry |
| `providers[]` | providers in failover order |
| `tempo` | natural cadence of the source, as a string |
| `native_resolution_m` | resolution at the source, in metres |

Per-provider fields (struct `sources::Provider`):

| Field | Description |
|---|---|
| `id` | provider ID (for example `"gcs.public"`, `"aws.opendata"`) |
| `kind` | one of the 8 `ConnectorKind` values |
| `url_template` | template with `{var}` interpolation |
| `cid` | static IPLD CID (only for the `ipld_cid` kind) |
| `auth` | string; the file uses `"anonymous"` (45), `"earthdata_login"` (2) and `"firms_map_key"` (1) |
| `rate_limit_qps` | a soft limit |
| `license` | licence string (`"CC-BY-4.0"`, `"Copernicus open data"`) |

The validator rejects two schemes with the same name. An empty
`providers[]` is valid. It declares the scheme with no provider. On
7 November 2026, one scheme had no provider: `openet.30m.daily`.

`template.rs` resolves these template variables: `{cell64}`, `{tslot}`,
`{year}`, `{month}`, `{day}`, `{channel}`, `{lat_band}`, `{lon_band}`,
`{lat_top10}`, `{lon_left10}`, `{hansen_lat_band}`, `{hansen_lon_band}`,
`{tile_id}`, `{bbox_csv}`, `{lat_center}` and `{lon_center}`. Values in the
caller's `vars` override the built-in values.

[Data sources](developers/data-sources.md) lists which schemes have a connector and which
do not.

## emem-topics (topics-v0.json)

27 topics. They route free-text questions through `/v1/ask` and the MCP
`emem_ask` tool. On 7 November 2026, 24 topics had a materializer for every
band they cite (from `GET /v1/materializers`). Two topics,
`nightlights` and `foundation_embedding`, had a materializer for none of
their bands.

Each topic has these fields:

| Field | Use |
|---|---|
| `description` | a paragraph; the source of the sentence-transformer embedding |
| `aliases[]` | short example phrases; they also feed the embedding pool and the substring fallback |
| `bands[]` | the bands `/v1/ask` recalls when the topic matches |
| `algorithms[]` | recipes from `algorithms-v0.json` to apply |

The validator rejects two topics with the same `key`, an empty
`description`, and a topic with no bands and no algorithms.

The backends that route a question (`topics-v0.json` `_routing._backends`):

| Backend | Behaviour |
|---|---|
| `ort` | Default. BAAI/bge-base-en-v1.5 (110 M parameters, 768-D, MTEB about 63), CLS-pooled, L2-normalised. Reads `tokenizer.json` and `model.onnx` from `EMEM_TOPIC_MODEL_DIR`. About 110 ms warm for `/v1/ask` end to end on CPU. |
| `model2vec` | Fallback. `minishlab/potion-base-8M`, a static token-lookup embedder (256-D, about 32 MB, no ONNX dependency). |
| `keyword` | Deterministic substring search over `aliases[]` and `key`. Select it with `EMEM_TOPIC_BACKEND=keyword`. It also runs as a precision pass before the transformer backends. |

`route` in `crates/emem-api-rest/src/topic_router.rs` picks the topics for
a question in these steps:

1. A question that matches the out-of-scope list (`OUT_OF_SCOPE_PATTERNS`)
   gets no topic.
2. The keyword pass finds each topic whose key or alias is a word in the
   question. These topics come first.
3. A transformer backend gives each topic a cosine score. It keeps a topic
   when the score is at least the threshold and within the margin of the
   best score.
4. It adds at most 2 of these topics after keyword matches. With no keyword
   match, it adds at most 4.
5. The result has at most 5 topics.

The `keyword` backend does steps 1, 2 and 5 only. It ignores the threshold
and the margin.

| Setting | Default | How to change it |
|---|---|---|
| Cosine threshold | `0.35` | `EMEM_TOPIC_THRESHOLD`, else `_routing.threshold` in `topics-v0.json` |
| Margin below the best score | `0.05` | `EMEM_TOPIC_MARGIN`, a value from 0 to 1 |
| Topics the transformer adds | 2 after keyword matches, 4 alone | `EMEM_TOPIC_EMBED_EXTRA="2,4"` |
| Topics per question | 5 | `_routing.max_topics_per_question` in `topics-v0.json` |
| Old rule (before 11 October 2026) | off | `EMEM_TOPIC_LEGACY=1` removes the margin and the limit on added topics |

The `_threshold_learned_from` block in `topics-v0.json` names the evaluation
set `tests/comprehensive/questions_v2.json` and says it has 105 questions. On
7 November 2026 the file had 132 questions. The block also says to sweep the
threshold with `EMEM_TOPIC_BACKEND=keyword`. That sweep changes nothing,
because the keyword backend ignores the threshold. To tune the threshold for
your own questions, use the `ort` or `model2vec` backend. Sweep
`EMEM_TOPIC_THRESHOLD` and pick the value with the highest precision at
recall >= 0.80.

The registry has two inverse lookups:

- `topics_for_band(band_key) -> Vec<&Topic>`
- `topics_for_algorithm(algo_key) -> Vec<&Topic>`

Both scan the 27 topics. The list is small, so there is no index.

## emem-schema (schema-v0.json)

The bundle of CDDL and JSON fragments that pin the wire shapes.

| Field | Required | Description |
|---|---|---|
| `manifest` | yes | MUST equal `"emem-schema"` |
| `version` | yes | `"v0"` |
| `fragments[]` | yes | one entry per wire shape |
| `hash` | yes | MUST equal `"blake3"` |
| `signature` | yes | MUST equal `"ed25519"` |
| `cid_encoding` | yes | `"base32-nopad-lowercase"` |

The 8 fragments:

```text
Cell, Tslot, PrimaryFact, DerivativeFact, NegativeFact, Attestation, Receipt, Claim
```

Each fragment has `name`, `cid_alg: "blake3-32"` and
`encoding: "canonical-cbor"`. The validator checks `hash == "blake3"` and
`signature == "ed25519"`. It rejects an empty `fragments[]`.

## emem-lcv1 (taxonomy.rs)

The land-cover taxonomy: 64 leaves in 8 families of 8 leaves. One class is
one `u8`:

```text
high 2 bits unused | family (3 bits) | leaf (3 bits)
```

The 8 families:

| Index | Variant | Description |
|---|---|---|
| 0 | `VegClosed` | vegetation, closed canopy |
| 1 | `VegOpen` | vegetation, open or shrub |
| 2 | `CropAnnual` | cropland, annual |
| 3 | `CropPerennial` | cropland, perennial or orchard |
| 4 | `Built` | built or sealed |
| 5 | `Bare` | bare or sparse |
| 6 | `Water` | water, inland and coastal |
| 7 | `Cryo` | snow, ice or wetland |

The class identifier is `"lcv-1.f<fam_idx>.l<leaf_idx>"`. An operator who
wants readable labels can publish a separate label manifest. They then cite
its CID next to the taxonomy CID.

## emem-cell64-alphabet

The 65 536 bigrams that turn a 64-bit cell ID into a text form of four
chunks. Each bigram is a CVCV pattern. There are 21 consonants and 10 vowels
(`aeiouAEIOU`), so `21 x 10 x 21 x 10 = 44 100` patterns. A `z<hex4>`
suffix fills the rest of the 65 536.

`build_alphabet_v0()` in `crates/emem-codec/src/alphabet.rs` builds the
alphabet in code. The build is deterministic from the consonant and vowel
constants in the same file. A change to this file changes every cell64.

## Counts and validator rules

| Manifest | Count on 7 November 2026 | Rules the validator enforces |
|---|---|---|
| `emem-bands` | 43 slots | contiguous offsets; no duplicate keys; sum of `dims` == 1792 |
| `emem-algorithms` | 172 | no duplicate keys; inputs not empty; output kind; `basis` per parameter; R1 to R4; I1, I2 |
| `emem-functions` | 23 | no duplicate keys; `deterministic == true`; sources for primary and negative; parent count for derivative |
| `emem-sources` | 46 schemes | no duplicate schemes |
| `emem-topics` | 27 topics | no duplicate keys; description not empty; bands or algorithms not both empty |
| `emem-schema` | 8 fragments | `hash == "blake3"`; `signature == "ed25519"`; fragments not empty |
| `emem-substrates` | 18 profiles | see the list below the table |
| `emem-device-platforms` | 17 platforms | see the list below the table |
| `emem-trace-encodings` | 8 encodings | no duplicate ids; at least one layer; toolchain not empty |
| `emem-lcv1` | 64 leaves | 8 families x 8 leaves; `u8` encoding (Rust enum, no validator) |
| `emem-cell64-alphabet` | 65 536 | built in code, deterministic |

The substrate validator (`substrates.rs`) refuses a manifest that breaks one
of these rules:

- Each profile id is unique.
- A profile states its grain one way only: `grain_min_m` and `grain_max_m`,
  or `grain` with a unit. In both forms, 0 < min <= max.
- This build knows the address space and the provenance class.
- An `active` profile declares bands and has a write path in this build.
- Only an `os_trace_required` profile names trace layers. It names at least
  one.
- A trace-admitted or custodial profile is not a drift anchor.
- A custodial profile does not use the `direct_sensor` provenance class.
- At least one `active` profile is a drift anchor.

The device platform validator (`device_platforms.rs`) refuses a manifest that
breaks one of these rules:

- Each family id and each platform id is unique. Each platform names a known
  family.
- A platform serves one or more contributor classes and recognises one or
  more encodings.
- A platform does not use the `raw_ed25519` key kind.
- A platform has a trust anchor, unless its root of trust is `software_only`.
  A `software_only` platform is never `active`.
- A non-provisional anchor is a 32-byte digest.
- An `active` platform has one or more non-provisional anchors.

## Publish a new manifest

1. Edit the JSON file in `crates/emem-core/data/`.
2. Run `cargo test -p emem-core`. The crate tests parse each embedded
   manifest, which runs its `Manifest::validate`. A malformed manifest (an
   offset gap, a duplicate key, a non-deterministic function) fails the
   tests.
3. Change the `version` field in the file if the change is not byte-stable.
   The validator does not check this field.
4. Rebuild the binary. It takes the new CID when it first parses the
   manifest. The registry data needs no code change.
5. Old facts still verify, because their receipts pin the old CID. New
   receipts cite the new CID. There is no migration step.

An operator who runs their own responder does the same steps with their own
JSON files. Fact CIDs and receipt CIDs stay compatible. The registry CIDs
differ, which is the purpose of content addressing.

## Get the CIDs at runtime

```sh
curl -s https://emem.dev/v1/manifests
```

This ran on 7 November 2026. It returned ten digests: `algorithms_cid`,
`bands_cid`, `device_platforms_cid`, `functions_cid`, `registry_cid`,
`schema_cid`, `sources_cid`, `substrates_cid`, `topics_cid` and
`trace_encodings_cid`. It also returned a `covers` object with entry counts.
`registry_cid` had the same value as `functions_cid`, as the server code
sets it.

Each receipt pins four of these in `source_versions`: `bands_cid`,
`registry_cid`, `schema_cid` and `sources_cid`. The signature covers them
through the `0x06` segment ([protocol §7](protocol.md#7-receipt)). The
receipt does not pin `algorithms_cid`.

### Compare a receipt with the live manifests

Use this check to see if the responder signed a receipt against the manifests
that are in force now. Save a recall response and the manifests:

```sh
curl -s -X POST https://emem.dev/v1/recall -H 'content-type: application/json' \
  -d '{"cell":"defi.zb592.nUkO.zEzE","bands":["copdem30m.elevation_mean"]}' -o recall.json
curl -s https://emem.dev/v1/manifests -o manifests.json
```

The cell is Mount Fuji, from `POST /v1/locate {"place":"Mount Fuji"}`. Then
compare the four pinned CIDs:

```python
import json

recall = json.load(open("recall.json"))
live = json.load(open("manifests.json"))
for name, cid in recall["receipt"]["source_versions"].items():
    state = "same" if live.get(name) == cid else "DIFFERENT"
    print(f"{name:13} {state:9} {cid}")
```

On 7 November 2026 all four lines said `same`. A `DIFFERENT` line does not
make the receipt invalid. It tells you that the manifest changed after the
responder signed. To replay the result, use the manifest JSON from the commit
that gave the pinned CID. The server has no route that returns an old
manifest. The signature check itself is in
[protocol §7.4](protocol.md#74-offline-verification-python).

A receipt that cites an algorithm and its input fact CIDs gives a third party
what they need to repeat the result. With the same algorithm manifest, they
read the algorithm's `parameters`, walk the `evaluation` AST (or read
`formula`), recall the cited bands, and compute again. If they get the same
number, the composition reproduces.

## Where to read the code

| Concern | File |
|---|---|
| `Manifest` trait and CID rule | `crates/emem-core/src/manifest.rs` |
| `BandRegistry` | `crates/emem-core/src/bands.rs` |
| `AlgorithmRegistry` and `Expr` AST | `crates/emem-core/src/algorithms.rs` |
| `Algorithm::param_f64` | `crates/emem-core/src/algorithms.rs` (`impl Algorithm`) |
| Climate archetype seed data | `crates/emem-core/data/climate_archetype_centroids_v1.json` |
| `FunctionRegistry` | `crates/emem-core/src/functions.rs` |
| `SourceRegistry` | `crates/emem-core/src/sources.rs` |
| `TopicRegistry` | `crates/emem-core/src/topics.rs` |
| `SchemaRegistry` | `crates/emem-core/src/schema.rs` |
| `SubstrateRegistry` | `crates/emem-core/src/substrates.rs` |
| `DevicePlatformRegistry` | `crates/emem-core/src/device_platforms.rs` |
| `TraceEncodingRegistry` | `crates/emem-core/src/trace_encodings.rs` |
| `Lcv1` and `LcvFamily` | `crates/emem-core/src/taxonomy.rs` |
| `PrivacyClass` | `crates/emem-core/src/privacy.rs` |
| `Tempo` and `Tslot` | `crates/emem-core/src/tslot.rs` |
| Alphabet builder | `crates/emem-codec/src/alphabet.rs::build_alphabet_v0()` |
