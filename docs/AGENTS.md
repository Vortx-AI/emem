# AGENTS — using emem from inside an agent loop

emem is built for AI agents. This guide tells the agent (or the human
wiring an agent) **what to call when**, **what to expect back**, and
**how to compose** the protocol's primitives into reliable spatial
reasoning. Every read returns a signed Receipt; every fact has a stable
content address (CID). Quote the CID and the responder pubkey in your
reply and the user can audit the answer offline.

---

## 1. The model in one paragraph

Every fact about every place is a content-addressed tuple
`(cell, band, tslot)` — signed by the responder's ed25519 key, hashable
with blake3, and recall-able by any client that knows the cell64. The
CID is `base32_nopad_lowercase(blake3(canonical_cbor(fact)))` and
survives copy-paste between conversations and between agents. There is
no "chat session"; emem is global, append-only memory of place.

---

## 2. The four addresses

| Address | Meaning           | Wire form              | Tokens |
|---------|-------------------|------------------------|--------|
| `cell`  | 64-bit cell ID    | `cell64` 4 base-1024 bigrams | ≤ 4 |
| `tslot` | u64 time slot     | `t.<base32>`           | ≤ 2    |
| `vec`   | 128-D (geotessera) or 1024-D (geotessera.multi_year) fp16 vector | `vec64` 12-byte prefix | ≤ 3    |
| `cid`   | 32-byte fact CID  | `cid64` 8-byte prefix  | ≤ 3    |

Reference any of these in chat using the short form. Full CIDs are for
canonical CBOR; the short forms are how agents talk to each other and
to users.

---

## 3. When to use emem (decision tree)

```
user mentions a place / lat-lng / cell64
   └─ POST /v1/locate {place|lat,lng}  →  cell64
   └─ POST /v1/recall {cell, bands?}    →  Facts + signed receipt

user says "how similar is X to Y"
   └─ POST /v1/compare {a, b, family?}  →  cosine + per-band deltas

user says "find places like X"
   └─ POST /v1/find_similar {key, band?, k?}  →  ranked neighbors

user says "what changed at X between t1 and t2"
   └─ POST /v1/diff {cell, band, tslot_a, tslot_b}  →  DerivativeFact

user says "show me the trajectory"
   └─ POST /v1/trajectory {cell, band, window:[t0,t1]}

user asks a yes/no with citable evidence
   └─ POST /v1/verify {cell, claim:{band, op, value}}

user wants a region summary
   └─ POST /v1/query_region {geometry, bands?, agg?}

user's ask is underspecified
   └─ POST /v1/intent {type:"what_is_here|where_is|is_like|...", ...}

user types a freeform question about a place
   └─ POST /v1/ask {q, lat, lng | cell | place}  →  facts + topic_routing
                                                    + algorithm_outcomes[]
                                                    + temporal_composition[]

user wants to know "which dataset answers X right now"
   └─ GET /v1/coverage_matrix
   └─ GET /v1/fleet  (for satellite/sensor lineage)
   └─ POST /v1/temporal_route  (PDE-based band scoring vs query time)

user wants a short-horizon LST forecast (urban heat island, etc)
   └─ POST /v1/heat_solve {cell, hours_ahead, diffusivity_m2_per_s}
        2-D explicit-FD heat solver (∂u/∂t = α∇²u) over a 3×3 cell stencil
        of MODIS LST_Day_1km. Signed forecast + CFL diagnostics.

user wants offshore swell propagation to a coast
   └─ POST /v1/wave_solve {coastal_cell, offshore_height_m, period_s, n_offshore_cells}
        1-D explicit-FD shallow-water wave solver (∂²u/∂t² = c²∂²u/∂x²,
        c² = g·h) along the GMRT bathymetric profile. Signed arrival.

user wants next-month NDVI prediction at an agriculture cell
   └─ POST /v1/jepa_predict {cell, band, lookback_months}
        Constrained JEPA-pattern AR(2) seasonal predictor (closed-form
        coefficients, NOT a learned MLP). Returns the prediction plus
        the cited monthly fact CIDs.
```

In every reply: cite `receipt.fact_cids[0]` (truncated 13-char `cid64`
prefix) and mention `responder_pubkey_b32` once per session.

### What `/v1/ask` (and `/v1/intent`) carry beyond raw facts

Two additive sibling arrays sit alongside `facts` and let an agent skip
a hand-rolled fan-out:

- **`algorithm_outcomes[]`** — one entry per matched algorithm whose
  registry entry carries an `evaluation: Expr` block. Each entry is
  `{ algorithm_key, evaluation_via: "ast", input_fact_cids[], value,
  inputs, inputs_with_provenance, formula, citation }` so the agent
  can recompute the value from the named inputs and verify each input
  fact independently. As of 0.0.4: `flood_risk@2`, `aqi_class@1` (PM2.5
  → 1–6 EPA category, look up label via `/v1/algorithms`'s
  `output.values[index-1]`); the rest still ship `formula: String`
  only.
- **`band_observations[]`** — fall-through path for when topic-router
  returns zero topics but the cell still owns signed bundle facts.
  Two-pass: topic-router first, then a second pass walks
  `recall.facts[]` and emits an entry per band not yet represented,
  tagged `routing_via: "inventory"`. This is the "raw bands fallback"
  that lets `/v1/ask {q:"air quality in Delhi"}` come back with 17
  cite-able CAMS observations even before any algorithm fires.
- **`temporal_composition[]`** — one entry per matched algorithm
  whose registry entry carries a `temporal_recipe`. Each entry is
  `{ algorithm_key, recipe_label, windows: [{ band, lookback_days,
  aggregator, fact_cid, value, ... }], aggregator_summary }`. Lets a
  single round-trip yield "antecedent rainfall (7 d sum) → recent
  radar water (14 d max) → optical water (30 d baseline)" without
  the agent issuing 3 follow-up calls.

Topic routing for the natural-language `q` field uses a content-
addressed `TopicRegistry` (`topics_cid` on `/v1/manifests`). Backend
since 2026-05-04 is direct `ort` 2.x + `tokenizers` BERT inference on
`BAAI/bge-base-en-v1.5` (110 M params, 768-D, MTEB ~63), CLS-pooled
+ L2-normalised — same backend an agent can verify per-call via
`topic_routing.routing.method` in the response. A keyword pre-pass
runs first so exact-noun matches always surface. Falls back to
`model2vec/potion-base-8M` (256-D, sub-µs, no ONNX) if ORT can't
load. Pin the `topics_cid` in your receipt if you need to reproduce
the routing decision later — same registry CID + same model file =
same matched topics.

---

## 4. Live materialized bands (one curl each)

Each band auto-materializes on a cache miss: the responder fetches
upstream, signs the resulting Fact under its identity, persists it, and
returns it. The next call hits the hot cache. Real cell64s below — copy
and run.

All examples below post to `https://emem.dev/v1/recall` with header
`content-type: application/json`. Body shown. A 200 response with an
empty `facts` list and `materialize_notes` is the honest signal that
the responder hasn't wired this band's upstream connector yet — the
response also carries `bands_already_attested_at_cell` listing what
*is* answerable at that cell. (This field was named `bands_available`
through 0.0.4; renamed in 0.0.5 because the old name read like a
global capability list when it's really a per-cell cache snapshot.)

```bash
# copdem30m.elevation_mean — Mount Fuji land DEM (Absence over water)
{"cell":"damO.zb000.xUti.zde78","bands":["copdem30m.elevation_mean"]}

# gmrt.topobathy_mean — Mount Everest, any-point-on-Earth elevation
{"cell":"damO.zb000.wapu.yAxe","bands":["gmrt.topobathy_mean"]}

# modis.ndvi_mean — Tokyo, 16-day MODIS Terra composite
{"cell":"damO.zb000.xUto.sisA","bands":["modis.ndvi_mean"]}

# indices.ndvi — Sentinel-2 L2A 10 m NDVI, Lagos
{"cell":"damO.zb000.tEkU.waxi","bands":["indices.ndvi"]}

# sentinel1_raw — Sentinel-1 GRD VV (dB), all-weather radar, São Paulo
{"cell":"damO.zb000.gihi.zbb17","bands":["sentinel1_raw"]}

# geotessera — Tessera 128-D embedding (HTTP range ~640 B/cell), Tokyo
{"cell":"damO.zb000.xUto.sisA","bands":["geotessera"]}

# weather.temperature_2m — Tokyo current 2-m air temp (geo-fed, 15-min)
{"cell":"damO.zb000.xUto.sisA","bands":["weather.temperature_2m"]}

# weather.cloud_cover — Sydney current cloud-cover percentage
{"cell":"damO.zb000.qiru.wUxi","bands":["weather.cloud_cover"]}

# weather.precipitation_mm — São Paulo last-15-min liquid-equivalent
{"cell":"damO.zb000.gihi.zbb17","bands":["weather.precipitation_mm"]}

# weather.wind_speed_10m — Reykjavík 10-m wind speed
{"cell":"damO.zb000.zce4f.jogI","bands":["weather.wind_speed_10m"]}
```

Full one-liner form:

```bash
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"damO.zb000.xUti.zde78","bands":["copdem30m.elevation_mean"]}'
```

Each response returns `facts: [...]` plus a `receipt` carrying
`fact_cids`, `responder` pubkey bytes, `signature` (64-byte ed25519),
`request_id`, `served_at`, and the manifest CIDs the responder used.

---

## 4½. Polygon-aware boring endpoints (regions, parks, airports)

Every `POST /v1/{ndvi,air,lst,soil,water,forest,weather,elevation,at}`
accepts `{place: "<text>"}`. When the place geocodes to a feature with
extent — a city, an admin boundary, a park, an airport, a lake, a
protected area — the responder fans out to `n_cells` (default 16,
cap 64) sample cells inside the bbox in parallel and returns:

| field | shape | what it is |
| --- | --- | --- |
| `polygon.bbox` | `{min_lat, max_lat, min_lng, max_lng}` | The OSM/Nominatim/wide-table bounding box used for sampling. |
| `polygon.area_km2` | number | Mid-latitude approx so an LLM can sanity-check. |
| `polygon.n_sample_cells` | int | How many cells were actually queried (deduped post-sampling). |
| `polygon.source` | string | `wide_bbox_table` / `nominatim_boundingbox` — where the bbox came from. |
| `polygon.geojson` | FeatureCollection | One Polygon Feature for the bbox outline; renders directly in geojson.io. |
| `polygon.scene_thumbs[]` | array | One entry per sampled cell with `{cell, lat, lng, scene_png, scene_rgb, geojson, info}` URLs — pick the first 9 for a 3×3 multimodal grid. |
| `polygon.scene_overlay_url` | URL | Server-rendered `image/svg+xml` viridis-painted heatmap of the band over the polygon, with the place label as caption. |
| `stats` (single-band) | `{mean, median, min, max, std, n, n_missing}` for numeric bands; `{mode, n_classes, n, n_missing}` + `class_distribution` for categorical (ESA WorldCover, JRC GSW). | Headline aggregate. |
| `bands.<key>.stats` (multi-band) | same | Per-band stats for the multi-band envelope (`/v1/at`, `/v1/soil`, `/v1/forest`, `/v1/water`, `/v1/lst`, `/v1/air`, `/v1/weather`). |
| `value_per_cell[]` | array | Per-cell `{cell, lat, lng, value, kind: "primary"|"absence", fact_cid}`. Cite individual `fact_cid` for the verifiable raw values. |
| `geojson` (single-band) / `bands.<key>.geojson` | FeatureCollection | Per-cell Polygon features each carrying the recalled value as a property — drop into Mapbox/Leaflet/Deck.gl/QGIS without further processing. |
| `partial`, `coverage_fraction` | bool, fraction in [0,1] | Honesty flags: `partial: true` flips when ANY band has a missing cell across the polygon sample (cell error / materializer skip / signed Absence). |

Force point behaviour with `n_cells: 1` — the response degrades to the
legacy single-cell shape (no `polygon` block, no `stats`).

Geocoder cascade for the polygon: embedded gazetteer + wide-bbox table
(zero network) → persistent TTL cache (sled) → Photon (komoot.io) live
→ Nominatim live. When any of these returns a centroid without a bbox
(common: dense-city Photon Point features, embedded-gazetteer entries
that pre-date polygon tracking), `locate_inner` enriches via a single
Nominatim `/search` lookup and re-caches the result.

```bash
curl -s -X POST https://emem.dev/v1/ndvi \
  -H 'content-type: application/json' \
  -d '{"place":"Miami International Airport","n_cells":9}' \
  | jq '{place_label, value, stats, polygon: (.polygon | {bbox, area_km2, n_sample_cells, scene_overlay_url, n_thumbs:(.scene_thumbs|length)})}'
```

```json
{
  "place_label": "Miami International Airport (aerodrome), Miami, FL, United States",
  "value": 0.184,
  "stats": {
    "mean": 0.184, "median": 0.137, "min": 0.035, "max": 0.409, "std": 0.139,
    "n": 9, "n_missing": 0
  },
  "polygon": {
    "bbox": {"min_lat":25.78, "max_lat":25.81, "min_lng":-80.32, "max_lng":-80.26},
    "area_km2": 14.8,
    "n_sample_cells": 9,
    "scene_overlay_url": "/v1/places/scene_overlay.svg?place=Miami%20International%20Airport%20(aerodrome)&band=indices.ndvi&n_cells=9",
    "n_thumbs": 9
  }
}
```

For raw per-cell facts (one signed receipt per cell, ≤256 cells),
use `POST /v1/recall_polygon` instead — it skips aggregation and
returns the full fact list.

---

## 5. Anatomy of a numeric response (everything an agent needs to cite)

As of 0.0.4 every signed fact in a `/v1/recall`, `/v1/ask`,
`/v1/cells/:cell64`, `/v1/recall_polygon`, or boring-endpoint response
carries the following sibling fields **in addition** to the core
`{cell, band, value, signed_at, signer, signature, sources}` an agent
already knows about. No second `/v1/bands` call is needed to interpret
or quote the value.

```json
{
  "band":               "cams.aod_550",
  "value":              0.87,
  "unit":               null,
  "signed_at":          "2026-05-04T06:00:29Z",
  "signer":             [231, 254, ...],
  "signer_pubkey_b32":  "777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka",
  "fact_cid":           "i2wrnw4rywqvliu3blzpz2hrgn33horfbb5twn6jo7xctzwlm6jq",
  "value_decoded":      "Built-up",
  "band_metadata": {
    "description":              "Surface air-quality scalars sourced from CAMS via Open-Meteo …",
    "interpretation":           "PM2.5 / PM10 / NO2 / O3 are the canonical four pollutants used by US EPA and EEA AQI calculators — see aqi_class@1 …",
    "pitfalls":                 "CAMS is a global model assimilating satellite + ground stations — local hot-spots …",
    "references":               "https://atmosphere.copernicus.eu / https://open-meteo.com/en/docs/air-quality-api",
    "units":                    "unitless",
    "value_range":              [0, 5],
    "dimension_description":    "Aerosol optical depth at 550 nm — column-integrated …",
    "dimension_index":          6,
    "inherited_from_cube_band": "air_quality"
  }
}
```

And on the receipt itself:

```json
{
  "responder":             [231, 254, ...],
  "responder_pubkey_b32":  "777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka",
  "fact_cids":             ["i2wrnw4rywqvliu3blzpz2hrgn33horfbb5twn6jo7xctzwlm6jq", ...],
  "signature":             [...]
}
```

Field-by-field, what an agent does with each:

| field                                | what to do                                                                                                                                                                  |
|--------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `value`                              | Quote the number. Pair with `unit` from `band_metadata.units` (or `band_metadata.dimension_description` for richer context).                                                    |
| `band_metadata.interpretation`       | Use to translate the raw number into user-facing language (e.g. "AOD 0.87 = hazy, > 1 would be severe smoke"). Don't re-invent thresholds — quote ours.                       |
| `band_metadata.pitfalls`             | Mention only when the user's question intersects a pitfall (e.g. "CAMS is a ~11 km global model so a single industrial stack won't show up").                                  |
| `band_metadata.references`           | Cite as the upstream source URL.                                                                                                                                              |
| `value_decoded`                      | For categorical bands (ESA WorldCover, JRC GSW transition class, S2 SCL): the human label. Quote this directly instead of the integer class ID.                              |
| `signer_pubkey_b32`                  | Quote (truncated to first 8 chars + ellipsis is fine). Lets the user paste it into `/v1/verify` to confirm the responder.                                                     |
| `fact_cid`                           | Quote (cid64 prefix — first 13 chars). Lets the user replay the exact same fact later via `GET /v1/cells/:cell/facts/:fact_cid`.                                                |
| `receipt.responder_pubkey_b32`       | Mention once per session. Same b32 string as `signer_pubkey_b32` when the responder produced the fact directly (vs. relayed from another attester).                            |

### Reply skeleton an agent can re-use verbatim

> AOD-550 over Delhi is **0.87** (hazy; CAMS via Open-Meteo, ~11 km
> resolution). Cite-able as `cid64 i2wrnw4rywqvl…` signed by
> responder `777er3yi…`. Caveat: CAMS is a global model so this is a
> regional, not point, value.

Three sentences, one number, one CID, one pubkey. The user can
verify offline via `POST /v1/verify_receipt`; the agent does not have
to re-fetch anything to write the reply.

---

## 6. Trust model

emem facts are content-addressed; receipts are signed. Verification is
deterministic and offline-capable.

- **Hash**: blake3 over canonical CBOR.
- **CID**: `base32_nopad_lowercase(blake3(canonical_cbor(fact)))`.
- **Signature preimage**:
  `blake3(request_id || "|" || served_at || "|" || primitive || "|" ||
  cell1,cell2,…|cid1,cid2,…)`.
- **Responder pubkey** (hosted instance):
  `777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka`. Available at
  `/health` and `/.well-known/emem.json`.
- **Manifest CIDs** — fetch live from `GET /v1/manifests` (or
  `GET /.well-known/emem.json`) and pin in the receipt. The CIDs evolve
  every release as bands / algorithms / functions land — anything
  pasted into a prompt goes stale within weeks. The current set is
  always in `/v1/manifests.{bands_cid, algorithms_cid, functions_cid,
  schema_cid, sources_cid, topics_cid}`.

Verify any responder's receipt offline:

```bash
curl -s -X POST https://emem.dev/v1/verify_receipt \
  -H 'content-type: application/json' \
  -d '{"receipt": <paste any receipt object from any prior call>}'
# → { valid: true|false, signer_pubkey_b32, preimage_blake3_hex }
```

Materialized facts are signed by the *responder*, not the upstream
provider. The fact's `derivation.fn_key` declares the recipe; an
external attester can re-run that recipe and submit a corroborating or
correcting Fact under their own ed25519 key. This is the
Contributor-of-Intelligence Layer (CoIL); see `/v1/contributors`.

---

## 7. How emem differs from a vector DB

| concern               | vector DB                  | emem                                   |
|-----------------------|----------------------------|----------------------------------------|
| key                   | opaque ID                  | `(cell, band, tslot)` — typed, stable  |
| value                 | embedding only             | scalar, vector, histogram, or signed Absence |
| identity across DBs   | none                       | identical canonical fact → identical CID |
| answer audit trail    | trust the operator         | ed25519 signature + offline verifier   |
| time semantics        | none                       | tslot maps to a real clock; trajectory + diff primitives |
| missing data          | null / empty result        | `Fact::Absence` is a signed first-class value |
| ontology              | none                       | `/v1/bands` — every band has a published key, dim, tempo, privacy class |
| similarity search     | the only operation         | one of eight primitives                |

When the user asks "find places like X" you want vector search. For
everything else (what's there, what changed, did this happen, which
satellite covers this) you want emem's typed primitives.

---

## 8. Reply formatting that doesn't waste tokens

When the agent answers with emem facts:

1. State the fact in plain language with units.
2. Quote the `cell64` and `tslot` text-form in backticks so the user
   (or the next agent) can copy them.
3. Cite `fact_cids[0]` from the receipt as a 13-char `cid64` prefix.
4. Mention `responder_pubkey_b32` (truncated) at most once per session.
5. If the response carries Absence facts, say so explicitly — Absence
   is "tried and got no answer", not null.

Example reply:

> Elevation at `damO.zb000.xUti.zde78` (Mount Fuji) is **3776 m**
> from `copdem30m.elevation_mean`. cid64 `oivxwgmenewlh` ·
> responder `777er3yi…`.

---

## 9. Conformance levels

- **L0** — every emem responder serves recall + recall_many + compare +
  find_similar + diff + trajectory + query_region + introspection.
  No write, no keys.
- **L1** — adds `verify` (claim eval with evidence CIDs).
- **L2** — adds `attest` (signed writes from any contributor with an
  ed25519 keypair). `challenge` and stake-based slashing are reserved
  in the wire format but are **not implemented in 0.0.x** — see
  `docs/SPEC.md` §6.3 and §8.4.

The `level` field on every tool descriptor at `/v1/tools` declares what
this responder serves.

---

## 10. Errors that mean something

The wire-stable error catalog at `/v1/errors` is what agents branch on:

- `cid_not_found` — recall hit a (cell, band) with no fact and no
  materializer; fall back to `query_region` aggregation or tell the
  user the cell is uncovered for that band.
- `band_not_in_registry` — the band key is not in the active manifest;
  call `/v1/bands` to enumerate.
- `bad_signature` — attestation failed verification; never retry blindly.
- `materialize_miss` — fact not in cache and no upstream connector for
  the band's source scheme; either contribute via `/v1/attest_cbor` or
  the operator must wire a connector.

Treat 5xx as transient (retry); treat 4xx as caller-side and surface
to the user.

---

## 11. MCP / Cursor / Claude Code / OpenAI GPT setup

Every host that speaks MCP Streamable HTTP points at the same URL
(`https://emem.dev/mcp`); paste-ready configs ship under `/examples/`.

```json
// Claude Desktop (~/Library/Application Support/Claude/claude_desktop_config.json
// on macOS, ~/.config/Claude/ on Linux, %APPDATA%\Claude\ on Windows)
// Claude Desktop ≥ 0.10 and Claude Code recent infer the transport
// from the https:// URL — no explicit transport field required.
{ "mcpServers": { "emem": { "url": "https://emem.dev/mcp" } } }
```

- **Cursor**: Settings → MCP → add Streamable-HTTP MCP server at
  `https://emem.dev/mcp` (HTTPS-only), or write `.cursor/mcp.json` at
  the project root. See `/examples/cursor.mcp.json`.
- **Cline (VS Code)**: Cline → MCP Servers → add Streamable-HTTP MCP
  server at the same HTTPS URL. See `/examples/cline.mcp.json`.
- **OpenAI GPT (Custom Action)**: in the GPT builder, paste
  `https://emem.dev/openapi.json` as the Action schema URL.
  Authentication: none. See `/examples/openai-gpt-action.json`.
- **LangChain / LlamaIndex (Python)**: see `/examples/langchain.py`
  and `/examples/llamaindex.py` for `@tool` and `FunctionTool`
  wrappers around `/v1/recall`, `/v1/compare`, `/v1/find_similar`.

---

## 12. Common mistakes

The failure modes that show up most often in agent traces, with the fix.

**Mistake 1: Using a band key that isn't in the active manifest.**
The responder returns `band_not_in_registry`. Fix: call `GET /v1/bands`
once at session start and only reference keys present in that list.
For the materialized subset, `GET /v1/materializers` is the wire-stable
catalog of what auto-fetches.

**Mistake 2: Ignoring `bands_already_attested_at_cell` on an empty recall.**
If `/v1/recall` returns an empty `facts` list, the response carries
`bands_already_attested_at_cell: [...]` listing the bands that DO
have data at this cell. Fix: re-query with one of those band keys,
or call `/v1/coverage_matrix` to see what the responder can answer
globally. (This field was named `bands_available` through 0.0.4 —
the old name implied global capability when it's really a per-cell
cache snapshot.)

**Mistake 3: Treating `Fact::Absence` as null.**
Absence is a signed statement that the responder tried and got no
answer (e.g. `copdem30m.elevation_mean` over open water — Cop-DEM uses
0 m as no-data marker, so emem signs Absence to disambiguate from
"sea level"). Fix: render Absence as "no land coverage at this cell"
and use `gmrt.topobathy_mean` for any-point-on-Earth elevation.

**Mistake 4: Not citing the receipt.**
Replies that just state the value lose the audit trail. Fix: include
`receipt.fact_cids[0]` in cid64 short form (13 chars) plus the
truncated `responder_pubkey_b32` so the user can verify with
`POST /v1/verify_receipt`.

**Mistake 5: Re-fetching the same cell on every turn.**
Recall responses are deterministic by `(cell, band, tslot)`. Use ETag
on `/v1/recall` (returns 304 on hit) and `/v1/recall_many` for
multi-cell fan-out (one round trip, per-cell receipts; max 256 cells).

**Mistake 6: Picking a band by name instead of by query time.**
The temporal router (POST `/v1/temporal_route`) scores every band
against query time, query intent, and last-attestation age using PDE
kernels (heat / wave / advection). Fix: when the user's question has a
clock ("right now", "yesterday", "last summer"), call the router first
and use its top-ranked band.

**Mistake 7: Calling `/v1/find_similar` on a band the cell has no
vector for.**
Returns `cid_not_found`. Fix: read the cell first via `/v1/recall`
with the target band; if the materializer is wired, the call
populates the vector; then run `find_similar`. Default vector band is
`geotessera` (128-D); `geotessera.multi_year` (1024-D) is also
available where the 8 annual vintages are reachable from the
Tessera v1 0.1° tile grid.

**Mistake 8: Trusting upstream provenance without checking the
derivation.**
Materialized facts are signed by the *responder*, not the upstream
provider. The fact's `derivation.fn_key` (e.g.
`gmrt_pointserver@1`, `open_meteo_forecast_current@1`,
`modis_ornl_subset@1`) declares the recipe. An external attester can
re-run the recipe and submit a corroborating or correcting Fact under
its own key. Surface the fn_key when accuracy matters.

---

## 13. What you get

Citable answers (`receipt.fact_cids[0]` + responder pubkey verify
offline), reproducible reads (same `(cell, band, tslot)` → same CID
on any responder), and cheap composition (locate → recall → verify →
diff is one chain of signed steps). Recall what was true yesterday —
the log is append-only. Use it whenever a question has a `where`.
