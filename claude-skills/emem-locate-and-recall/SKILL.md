---
name: emem-locate-and-recall
description: Resolve a free-form place name to an emem cell64 and recall signed Earth-observation facts at that location. Use when the user asks about current weather, vegetation index, elevation, soil properties, or any other geospatial measurement at a named place ("what's the temperature in Bengaluru", "how high is Denali", "what's the NDVI in the Sundarbans"). Returns content-addressed facts with Ed25519 receipts that can be verified offline. Reads are public — no auth required.
allowed-tools: Bash(curl:*) Bash(jq:*) Bash(python3:*) Read
---

# emem-locate-and-recall

This skill turns a place name into signed Earth-observation facts in
two REST calls.

## When to invoke

The user asks about a measurable geospatial fact at a named place:

- "What's the current 2 m air temperature in Bengaluru?"
- "Show me the NDVI in the Sundarbans."
- "How does the soil pH look in central Iowa?"
- "What's the elevation of Mount Kilimanjaro?"

If the user has a `cell64` already (a string like
`defi.zb493.xoso.zcb6a`), skip the locate step and go straight to
`/v1/recall`. If they want a *thumbnail* (visual preview) instead of
numeric facts, use `GET /v1/cells/{cell64}/scene.png` directly.

## How to invoke

The endpoint is `https://emem.dev`. Reads are public.

### Step 1 — resolve the place name

```sh
curl -sf -X POST https://emem.dev/v1/locate \
  -H 'content-type: application/json' \
  -d '{"q":"Bengaluru, India"}' | jq '.cell64, .place_label, .via'
```

`via` reports how the name was resolved (`embedded`, `cache`,
`photon`, `nominatim`, `fallback`). Treat `via=fallback` as
low-confidence and ask the user to clarify.

### Step 2 — recall the band(s)

```sh
curl -sf -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"<CELL64_FROM_STEP_1>",
       "bands":["weather.temperature_2m","indices.ndvi"]}' \
  | jq '.facts[] | {band, value, unit, signed_at}, .receipt.fact_cids[]'
```

The response carries `.facts[]` (with `value`, `unit`, `signed_at`,
`signer_pubkey_b32`) and `.receipt` (Ed25519 signature, fact_cids,
schema_cid, registry_cid). The `fact_cids[i]` is a durable handle —
re-fetching that CID from any responder in any year returns the same
bytes.

## Picking bands

The full band list is at `https://emem.dev/v1/bands` (35 bands, 1792
total dims). Common picks:

- **Weather**: `weather.temperature_2m`, `weather.precipitation_mm`,
  `weather.relative_humidity_2m`, `weather.wind_speed_10m`
- **Air quality**: `cams.pm25`, `cams.no2`, `cams.aod_550`
- **Climate**: `era5.t2m`, `era5.precip` (1940→present hourly)
- **Vegetation**: `indices.ndvi`, `indices.evi`, `modis.ndvi_mean`
- **Elevation**: `copdem30m.elevation_mean`, `gmrt.topobathy_mean`
- **Land cover**: `esa_worldcover.lc_2021`
- **Embeddings**: `geotessera` (128-D, default 2024 vintage); the
  responder also ships `geotessera.{2017..2024}` for per-year and
  `geotessera.multi_year` (1024-D = 8×128 stacked).

If unsure which band fits the user's question, call `/v1/bands` and
filter by family (`weather`, `vegetation`, `terrain`, `landcover`,
`climate`).

## Recovering from errors

- **HTTP 404 `cid_not_found`** — band has no fact at this cell on
  this responder. With `EMEM_AUTO_MATERIALIZE=on` (default at
  emem.dev) the recall already attempted to materialise; the band
  has no upstream connector wired. Use `/v1/data_availability` to
  check which bands have a materialiser.
- **HTTP 400 `unknown cell64 symbol`** — typo in the cell64. Re-run
  step 1 with the place name.
- **HTTP 422** — request body malformed; the responder's structured
  error message names the missing field.

## Verifying offline

Pass the returned receipt to the `emem-verify-receipt` skill (or to
`POST /v1/verify_receipt` server-side) to confirm the signature
without re-trusting the responder.

## Worked example

```
USER: What's the current temperature in Bengaluru, and the NDVI?

CLAUDE invokes this skill:
  POST /v1/locate {"q":"Bengaluru, India"}
    → cell64=defi.zb493.xoso.zcb6a
  POST /v1/recall {"cell":"defi.zb493.xoso.zcb6a",
                   "bands":["weather.temperature_2m","indices.ndvi"]}
    → facts:
       weather.temperature_2m = 28.4 degC
       indices.ndvi = 0.42
    → fact_cids: [qi3jo4..., bn7c4d...]

CLAUDE replies: "Bengaluru is at 28.4 °C right now (signed by
emem.dev at 2026-05-08T13:22Z). The vegetation index there
is 0.42 — middling, consistent with a dry urban core. Both
facts are content-addressed: qi3jo4...l2hgjtwm and
bn7c4d...kkb73a respectively."
```
