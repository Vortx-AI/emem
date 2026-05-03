# Changelog

All notable changes to the emem reference implementation are recorded
here. The format follows [Keep a Changelog](https://keepachangelog.com/)
and we use [Semantic Versioning](https://semver.org/) once we're past
0.1.

## [Unreleased]

### Added
- **`/v1/{ndvi,air,lst,soil,water,forest,weather,at}`** — convenience
  POST handlers accepting `{place: "..."}` (geocode→cell) or
  `{lat, lng}`; matching `?place=` and `?lat=&lng=` GET forms. Each
  returns a single signed scalar plus receipt. Closes the gap where
  only `/v1/elevation` was wired despite the eight sister endpoints
  being documented.
- **`GET /v1/cells/:cell64/scene.rgb`** — raw `application/octet-stream`
  RGB bytes (with `x-emem-scene-{format,width,height,channels,...}`
  headers), sibling to the existing `scene.png` route. Matches the MCP
  `emem_cell_scene_rgb` tool.
- **`GET /security`** — fourth policy page, served as
  `text/markdown` from `SECURITY.md` alongside the existing
  `/privacy`, `/terms`, `/support` routes; required for marketplace
  listings.
- **`POST /v1/fetch`** — REST mirror of the MCP `emem_fetch` tool.
  Accepts either `{cid}` (lookup-only) or `{cell, band, [tslot]}`
  (materialize-then-persist), returning the fact + signed receipt.
- **`bands_cid` in `/v1/bands` response root** — agents reading
  `/v1/bands` alone can now pin the manifest CID without a separate
  `/v1/manifests` call.
- **`open_meteo_copdem90m@1` and `met_no_locationforecast_compact@1`**
  registered in `/v1/functions` so the derivation-function registry
  matches the materializer registry. Function count: 17 → 19.
- **`place_not_found` and `geocoder_transport_down` error codes**
  exposed in `/v1/errors` alongside the legacy `no_geocoder_match`,
  documenting the runtime distinction wired in commit `02e4c5e`.
- **`/v1/query_region` accepts `bbox: [west, south, east, north]`**
  in addition to the cell64 `geometry` form, matching the documented
  examples in `examples/agent-walkthroughs.md`.
- **`emem verify <receipt-path|->`** subcommand on the `emem` CLI —
  offline ed25519 verification of a receipt against either the
  embedded responder pubkey, an explicit `--pubkey b32`, or
  `/.well-known/emem.json` via `--base-url`/`EMEM_BASE_URL`. Calls the
  same blake3 preimage path as `POST /v1/verify_receipt`. Exits 0 on
  valid, 1 on invalid.
- **`EMEM_BASE_URL` defaults to `http://localhost:5051`** in
  `emem-demo`, `emem-livedemo`, `emem-realdemo`, matching the
  emem-server default bind. Emits a one-line stderr note when the
  default fires.
- Updated `emem_grid_info` MCP tool description to the active
  `~9.54 m × 9.55 m` grid (was stale `~305 m × 611 m` text from the
  legacy v0.0.0 layout). The response payload itself was always
  correct; only the catalog description string was stale.

- **Parallel temporal recipes** — `/v1/temporal_route` and
  algorithm temporal-window materialization now fan out across
  windows via `tokio::task::JoinSet`, removing the previous serial
  60-s timeout bottleneck. Snapshot path (`try_materialize_bands`)
  remains serial; parallelization is currently temporal-only.
- **Transparent `algorithm_outcomes[]`** — algorithm responses now
  carry `formula`, `inputs: {band_key: value}`, and `citation`
  alongside the computed value, so any agent can recompute the
  outcome locally and verify it. Outcomes are unsigned JSON in
  0.0.x; signed `DerivativeFact` wrapping is targeted for 0.0.4.
- **Deeper `/v1/bands`** — band manifest extended with
  `description`, `units`, `value_range`, `interpretation`,
  `pitfalls`, `references`, `dimensions[]`, `scalar_keys[]` for
  the top 11 bands.
- **MCP resource templates** — `emem://band/{band_key}`,
  `emem://algorithm/{algorithm_key}`, `emem://fact/{fact_cid}`,
  `emem://cell/{cell64}/...` exposed via MCP resource discovery.

### Fixed
- **Dockerfile**: added `g++` to the build stage so the
  `model2vec-rs` C++ dependency compiles cleanly in CI.
- **0.0.3 sweep**: aligned `0.0.2 → 0.0.3` in user-agent strings,
  exposed `topicRouter` backend in introspection.

## [0.0.3] — 2026-05-01

This release closes the gaps surfaced by the Katihar (Bihar) man-made-lake
test report — placeholders, hardcodes, and silent fallbacks that turned
honest geospatial questions into wrong-but-confident answers. It also
tightens every protocol surface that an external agent touches: the
geocoder cascade, the temporal composition vocabulary, the algorithm
registry, the multimodal scene path, and the brand identity.

### Added
- **Topic registry + transformer router** —
  `crates/emem-core/data/topics-v0.json` is a content-addressed
  `TopicRegistry` (manifest topic `emem-topics`) of 25 hand-authored
  topics, each declaring `{ key, description, aliases[], bands[],
  algorithms[] }`. The new `topic_router` module in `emem-api-rest`
  embeds each topic's description + aliases with model2vec-rs
  (`minishlab/potion-base-8M`, ~32 MB, sub-ms inference, pure-Rust,
  no ONNX/C++) and routes a free-text question to topics by cosine
  similarity ≥ 0.35. If the model fails to load (no HF_HOME, no
  network on first run, etc.) the router transparently falls back to
  alias keyword matching — the surface contract is identical.
  Replaces ~639 lines of static `TOPIC_BANDS` / `TOPIC_ALGORITHMS` /
  `TOPIC_KEYWORDS` tables that were previously hardcoded in
  `lib.rs`. (Phase B of the "scientific routing" scan.)
- **Formula-AST evaluator + composite dispatcher** — `Expr` enum in
  `emem-core::algorithms` (`Band`, `Const`, `Add`, `Sub`, `Mul`,
  `Div`, `Linear`, `WeightedBlend`, `Clamp`, `Where`, `Abs`,
  `Sigmoid`, `Relu`, `Max`, `Min`) with `Expr::evaluate(samples) ->
  Option<f64>` and `Expr::referenced_bands()`. Algorithms now carry
  an optional `evaluation: Expr` field that turns a human-readable
  `formula: String` into a byte-stable executable AST. `flood_risk@2`
  is the proof-of-concept: its `0.55*(swr/100) +
  0.25*dem_agreement*(relu(50-cop)/50) + 0.20*sigmoid((-15-s1)/2)`
  formula round-trips through canonical-CBOR JSON to the dispatcher
  and produces 0.4836 byte-stably (test:
  `flood_risk_v2_evaluates_to_a_real_number_from_dispatcher`). The
  new `dispatch_algorithms(matched_keys, recall)` helper in
  `emem-api-rest` runs every matched algorithm whose evaluation block
  is satisfied by the recall samples and emits an
  `algorithm_outcomes[]` array on `/v1/ask` (additive sibling, empty
  when no algorithm has an `evaluation` block yet). (Phase C of the
  "scientific routing" scan; M-13 / M-14 carryforward to v0.0.4 to
  migrate the remaining 101 algorithms.)
- **Temporal composition** — `Algorithm.temporal_recipe { windows[], label,
  note }` in `emem-core::algorithms`. Each window declares
  `{ band, lookback_days, aggregator, purpose, trigger_threshold? }` so a
  composite score can express "antecedent rainfall (7 d, sum) → recent
  radar water (14 d, max) → optical water (30 d, baseline)" without
  hardcoding the cadence in the responder.
- `/v1/ask` and `/v1/intent` responses now carry an additive
  `temporal_composition[]` field (sibling to `facts`/`results`,
  not a replacement) — empty array when no matched algorithm declares
  a recipe. Each entry surfaces the algorithm key, the recipe label, and
  per-window fact CIDs + scalar values + an aggregator summary so the
  agent can compose a real flood / drought / wildfire answer in one
  round-trip instead of a hand-rolled fan-out.
- `flood_risk@2` algorithm — adds GMRT topo input and a
  `dem_agreement` weighting term (factor 0.5 when |Cop-DEM − GMRT| > 5 m)
  on top of `flood_risk@1`, plus a temporal_recipe for antecedent
  rainfall + recent radar water + optical water. `flood_risk@1` is
  retained for backwards compatibility.
- `temporal_recipe` blocks on `water_consensus@1`,
  `wildfire_exposure_score@1`, and `spi_meteorological_drought@1`.
- **Sentinel-2/Sentinel-1 fallback ladders** — `s2_search_with_fallback`
  (40 % cloud / 30 d → 60 % / 60 d → 80 % / 90 d) and
  `s1_search_with_fallback` (15 d → 30 d → 60 d) so cloudy / rainy
  regions still return a real scene rather than degrading to a
  placeholder. Used by the rainy-day flood path the Katihar report
  asked for.
- **Adaptive polygon density** — `RecallPolygonReq.cells_per_sqkm` and
  `drill_on_water` parameters; max-cells cap raised from 256 to 1024.
  Two-stage drill now adds `hot_centres` around recurrence > 25 % cells
  so a polygon recall over a water body returns the wet pixels at high
  density instead of being blurred by a uniform sweep.
- **Lake / pond / reservoir keywords** in `TOPIC_KEYWORDS["flood_water_event_window"]`
  — `lake`, `pond`, `reservoir`, `manmade lake`, `tank`, `water body`,
  `lagoon`, `wetland`, `marsh` — so a "is the lake flooded" question
  actually routes to the flood/water topic instead of falling through.
- **Photon (komoot.io) geocoder** as the primary live fallback when the
  embedded gazetteer and TTL cache miss. Fast (~100 ms typical),
  Elasticsearch-indexed OSM with strong recall on rural villages /
  tanks / water bodies (Katihar test: `Laliyahi` resolves via Photon
  but returned no results from Nominatim). Nominatim is now the
  secondary fallback. Configurable via `EMEM_PHOTON_BASE`.
- **Overture release auto-discovery** — `latest_release()` walks the
  Overture S3 bucket via ListObjectsV2 + XML parse, with a 24 h cached
  ReleaseCache and `EMEM_OVERTURE_RELEASE` env override so an operator
  can pin a specific release for repro builds.
- `/v1/materializers` now exposes `overture_release` so an agent can
  see which release will be served without an Overture round-trip.
- `temporal_recipe` is also surfaced inline on each
  `/v1/intent → composite_suggestions.applicable[]` entry so an agent
  planning a follow-up `/v1/ask` sees the lookback windows without a
  second `GET /v1/algorithms/<key>` round-trip.
- **Brand identity refresh** — new `/logo.png`, `/logo-mark.png`,
  `/logo-300w.png`, `/logo-600w.png`, `/logo-1200w.png`,
  `/favicon.png`, `/apple-touch-icon.png`, `/icon-192.png`,
  `/icon-512.png` (PNG variants of the new mark) plus a refreshed
  `favicon.svg` and `og-image.svg` carrying the new
  indigo→purple gradient palette. `index.html`, `agent.json`,
  `ai-plugin.json`, and `gemini-extension.json` now reference
  `/logo.png` for organization-logo schema.

### Changed
- Live geocoder priority is now Photon → Nominatim (was: Nominatim → Overpass).
  The `via` field in `/v1/locate` reports `embedded` / `cache` /
  `photon` / `nominatim` / `direct`.
- `build_cell_scene_rgb` percentile helper now returns `Option<f64>`
  and filters non-finite / non-positive samples up front, so a tile
  with no surface reflectance no longer renders as a black scene.
- `algorithms_for_topic[flood_*]` now points at `flood_risk@2`.
  `flood_risk@1` is retained in the registry so existing receipts
  still resolve.
- `/v1/recall_polygon`'s `max_cells` cap raised 256 → 1024 to support
  the new dense / drilled fan-outs without forcing the agent to
  manually slice the polygon.
- Bumped HTTP request timeouts to give cold materializers room to
  fan out on first hit (per `docs/CLIENTS.md` guidance).

### Removed
- **Static topic-routing tables** — the ~639-line
  `TOPIC_BANDS` / `TOPIC_ALGORITHMS` / `TOPIC_KEYWORDS`
  `LazyLock` blocks in `crates/emem-api-rest/src/lib.rs` are gone.
  The same data now lives in `topics-v0.json` and is consumed
  through the `TopicRouter` (transformer-backed cosine match,
  alias-keyword fallback). The four call sites
  (`live_bands_for_topic`, `algorithms_keys_for_topic`,
  `route_question_to_topics`, `matched_keywords`) are now thin
  registry queries.
- **Overpass geocoder fallback** — the public Overpass instance
  routinely returns 503 under rate limits and a global
  name-regex query takes ~30 s on a synchronous request path. Photon
  serves the same OSM corpus via Elasticsearch in ~100 ms with better
  recall, so Overpass has been removed from the live cascade.
  `EMEM_OVERPASS_BASE` is no longer consulted.

### Fixed
- `/v1/locate` for rural OSM places (e.g. `Laliyahi` / Katihar) — the
  embedded → cache → Photon → Nominatim cascade now resolves these
  reliably; the old chain timed out on Overpass.
- `/v1/cells/{cell}/scene.png` no longer returns a uniformly black
  PNG when a Sentinel-2 tile has scattered reflectance (percentile
  helper was returning 0 instead of `None`).
- Three Overture client `cli.release().to_string()` callsites were
  silently fire-and-forgetting an unawaited future; now `.await`ed
  with `map_err`.

### Migration notes
- **No breaking changes.** `temporal_composition` and
  `temporal_recipe` are additive sibling fields; existing readers
  that parse `facts[]` only continue to work.
- `flood_risk@1` is still in the registry, so receipts that cite the
  v1 algorithm key continue to verify. New flood questions
  automatically route to `flood_risk@2`.
- If you hardcoded `via == "overpass"` anywhere, change it to
  `via == "photon"`.

### Also added (carryover from the post-0.0.2 development cycle)
- `PRIVACY.md` "Your rights" section enumerating GDPR / CCPA / CPRA data-
  subject rights (access, erasure, rectification, objection, opt-out of
  sale/sharing, non-discrimination) and how to exercise them.
- Native HTTPS via in-process rustls + Let's Encrypt (TLS-ALPN-01).
- `/v1/locate` (lat/lng or place name → cell64; OSM Nominatim under the
  hood for place-name lookup).
- `/v1/cells/{cell64}/info` (cell64 → centre + bbox + approx size).
- `/v1/discover` (one-call agent bootstrap: agent_card + manifests +
  canonical places + next-call hints).
- `/api` — 308 redirect to `/v1/agent_card`.
- `/v1/contributors` and `/v1/contributors/{pubkey_b32}` — the
  Contributor-of-Intelligence Layer (CoIL) leaderboard.
- `/metrics` — Prometheus text-format counters.
- `/llms-full.txt` — the comprehensive single-call agent context dump.
- `/examples/agent-walkthroughs.md` — 8 worked end-to-end queries.
- Production middleware: 16 MiB body cap, 30 s timeout, per-IP token-
  bucket rate limit (60/min, 120 burst), HSTS / CSP / X-Content-Type-
  Options / X-Frame-Options / Referrer-Policy / Permissions-Policy,
  optional HTTP→HTTPS redirect via `EMEM_REDIRECT_HTTPS=1`,
  graceful shutdown on SIGTERM.
- Persistent ed25519 responder identity at
  `<EMEM_DATA>/identity.secret.b32` (mode 0600).
- `emem-livedemo` and `emem-realdemo` CLI binaries with full request +
  response + receipt traceability written to `var/demos/`.
- Daily systemd timer (`emem-daily-delta.timer`) capturing contributor
  + metrics + realdemo trace at 03:17 UTC.
- SEO surface: Open Graph + Twitter card meta, geo / ICBM / DC.coverage
  meta, JSON-LD `SoftwareApplication` + `Organization` + `WebSite`,
  GA4 (`G-RBLXX5LR9L`), favicon, OG image, IndexNow key endpoint,
  `/.well-known/security.txt`.

### Also changed (carryover from the post-0.0.2 development cycle)
- Cell64 codec now exposes a stable `cell_from_latlng` / `latlng_from_cell64`
  pair in `emem-codec::geo`, with a documented bit layout
  (`mode|res|base|hilbert_d`).
- `emem-realdemo` uses the canonical codec — its attested cells now
  match the cells `/v1/locate` returns for the same coordinates.
- Curl examples in `web/index.html` and `web/llms.txt` now reference a
  real, locatable cell that returns real Cop-DEM provenance facts.
- `serve_llms_full` actually serves a comprehensive LLM-targeted text
  rather than the whitepaper.

### Also removed (carryover from the post-0.0.2 development cycle)
- The third-party `r.jina.ai` external-probe dependency. We use
  `curl --resolve` for direct external connectivity tests now.

## [0.0.2] — 2026-04-26

Initial open-source release. The protocol surface, primitives, MCP
server, and reference responder are all functional. See README.md for
the workspace layout and DEPLOY.md for production deployment.
