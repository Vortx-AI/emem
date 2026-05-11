# Changelog

emem follows the [Keep a Changelog](https://keepachangelog.com/) format.
CIDs are content-addressed; minor version bumps may roll bands /
algorithms / sources manifests, but old facts under old CIDs continue
to verify. 

## [Unreleased]

### `/humans` rebuild — 2026-05-08

Public interactive surface at `https://emem.dev/humans` — the page is its
own API console. Every visible cell carries `data-emem-cell`, `band`,
`fact-cid`, `tslot` attributes; every interactive control carries
`data-emem-action`. A scraping LLM extracts everything from the rendered
DOM. A live console pane prints every `/v1/*` call the page makes with
copy-as-curl / copy-as-python / copy-as-MCP pivots and a replay button.

#### Added
- `web/humans.html` (~3.2 K LoC, single self-contained file) replaces
  the v1 dashboard. Constellation field, Verlet force-graph, Poincaré
  registry view, Sigstore-Rekor-style attestation log, lasso →
  `/v1/recall_polygon`, embedding-PCA reprojection over 128-D Tessera
  vectors fetched via `/v1/recall_many`, command palette, hash chips
  with click-to-copy, focus mode, collapsible rails, mobile bottom-sheet,
  touch-lasso path, URL state encoding (`?cell=…&proj=embed&mode=log&layout=…`)
  so a tweeted link reproduces the exact view.
- Sibling routes wired in `crates/emem-api-rest/src/lib.rs`:
  `/humans.json` (JSON twin, `schema=emem.humans.v1`),
  `/humans/llms.txt` (page-scoped llms.txt convention),
  `/humans-og.svg` (1200×630 OpenGraph card).
- Pinned offline-verify libs from `esm.sh`: `@noble/curves@1.6.0/ed25519`
  + `@noble/hashes@1.5.0/blake3`. Preimage builder mirrors
  `crates/emem-storage/src/server.rs:132-148` byte-for-byte; verifies
  receipts in the browser without re-contacting the responder.

#### Fixed
- CSP header was blocking `https://esm.sh`, so the page silently fell
  back to server-side `/v1/verify_receipt` while labelling itself "CDN
  libs unavailable for offline path" — read as a verify failure.
  `crates/emem-api-rest/src/lib.rs` CSP now lists esm.sh in `script-src`
  and `connect-src`. Offline verify actually runs offline.
- `find_similar` 404 on cold cells (no `geotessera` attested) now
  auto-materialises via `/v1/recall` and retries instead of swallowing
  the responder's hint into a bare "HTTP 404".
- `installChips` was reading `textContent` after `setMan` had ellipsised
  it, so manifest-CID chips and the rail pubkey chip copied
  `"abc...123…"` instead of the full base32. Now prefers `el.title`.
- Family filter no longer blanks the canvas — cells derive a real
  dominant family from `/v1/coverage_matrix` instead of all defaulting
  to `'foundation'`.
- Lasso auto-exits after polygon submission; touch-lasso path added so
  the chip works on mobile (was unreachable — single-finger drag always
  routed through pan).
- rAF twinkle loop honours `prefers-reduced-motion`.
- Console `aria-live=off` (was announcing every API call to screen
  readers) + `CONSOLE_MAX_ROWS=250` cap so DOM doesn't grow unbounded.
- `--fg-mute` lifted from `#5A5C66` (2.96:1, fails WCAG AA) to
  `#7A7D87` (4.5:1).
- Five top-edge absolute clusters consolidated into a single bottom
  dock (`modes | projection | zoom | focus`); top of the canvas is
  now empty so hover tooltips and the centred hero have room.

#### Doc-only
- README's "Foundation embeddings" line corrected: ships 8 annual
  Tessera vintages 2017–2024 plus `bin128` and `multi_year`, not
  "vintage 2024" only.
- Deferred-section claim "upstream is 2024-only" rewritten — the
  upstream has all 8 vintages; the JEPA-v2 training blocker is
  candidate-pool selection (most cells need backfill before they
  carry the multi-year stack), not upstream availability.

### Sweep — 2026-05-08

Fresh memory rebuild from code, full P0+P1+P2 fix sweep, then docs
redo. The summary: every honesty gap surfaced by the parallel audit
is closed in code or removed from the surface; nothing is left as a
stub or "lands in v0.1".

#### Added
- `verify mode=Resolve` actually resolves on miss. Previously
  degraded silently to Fast. Now calls
  `storage.materialize_many(&[CanonicalKey])` with the targeted
  tslot (or single-point window) and re-scans. Open-ended windows
  with no targetable tslot fall back to Fast — documented in the
  doc comment, not silently swallowed. `MaterializeMiss` (no
  upstream connector) bubbles to the caller.
  (`crates/emem-primitives/src/verify.rs:92-111`.)
- `find_similar.filter` honours structured `Claim` predicates with
  per-cell verdict memoisation, applied in both cosine and binary
  scoring paths. Cells with no fact for the filter band are dropped
  (undecidable, not "false") so an agent asking "find places like X
  where NDVI > 0.5" does not get silent inclusion of cells with no
  NDVI history.
- `Receipt.merkle_proof` populated end-to-end:
  - `emem_attest::merkle_root_and_paths(leaves) -> (root,
    Vec<path>)` returns root + per-leaf bottom-up sibling paths in
    one pass.
  - `emem_attest::verify_merkle_path(leaf, idx, path, root)` rebuilds
    the root from a single proof.
  - `MaterializingStorage::put_attestation` persists per-fact
    `MerkleProof` records to a sled tree `emem.fact_proofs`, keyed
    by FactCid string. Leaves are sorted by their 32-byte leaf
    hash; `MerkleProof.leaf_index` is the sorted-order position.
  - `Server::sign_receipt` populates `Receipt.merkle_proof` from
    the first cited fact's stored proof.
- `query_region` accepts `bbox:lon_min,lat_min,lon_max,lat_max`
  geometry. Synthesis caps at `MAX_BBOX_CELLS = 4096` (~6.4 km ×
  6.4 km at the equator) and `MAX_REGION_FACTS = 65_536`. Beyond
  either cap the responder stops scanning and aggregates over what
  it has; `receipt.fact_cids` reflects exactly what contributed.
  GeoJSON polyfill returns a structured error.
- JEPA-v2 trained-checkpoint loader at
  `python/jepa_v2_sidecar/server.py:_Registry.load_dynamics`:
  `torch.load(weights_only=True)` → `load_state_dict(strict=True)`
  (architecture-name drift fails at load, not at prediction) →
  optional `blake2b_hex(state_dict_bytes) == declared_hash`. The
  pre-existing `RuntimeError("loader not shipped yet")` guard is
  gone; its concern (silent garbage outputs) is preserved by the
  strict load + hash check.
- Terraclimate failover: `crates/emem-fetch/src/terraclimate.rs`
  defines `NCSS_BASES = [UI primary, NCAR RDA secondary]`.
  `fetch_terraclimate_normal` tries each in order; the receipt's
  `Source.url` records which mirror answered.
- Documented the user-vs-system-mode `cap_net_bind_service` story in
  `ops/systemd/emem-server.service.example` and `docs/operating.md`.
  An earlier attempt to add `AmbientCapabilities=CAP_NET_BIND_SERVICE`
  for the user unit failed in production: the kernel does not honour
  that directive for user-mode systemd (no UID transition for the
  user manager to prime), so the unit crash-looped with
  `status=218/CAPABILITIES`. The directive only works for system-mode
  units. User-mode deployments stay on `setcap cap_net_bind_service=+ep`
  re-applied by `scripts/redeploy.sh` after every release build.
- 4 query_region bbox lock-in tests (round-trip, oversized cap,
  malformed, inverted).
- 2 Merkle path lock-in tests (single-leaf empty path,
  odd-cardinality self-pair).
- 14 fresh docs files: `README.md`, `CONTRIBUTING.md`,
  `CHANGELOG.md` (this), `docs/{agents, protocol, architecture,
  registries, data-sources, inference, developing, operating,
  whitepaper}.md`.
- 12 fresh memory files capturing code-verified ground truth
  (`project_codec`, `project_registries`, `project_trust_layer`,
  `project_fetch_inventory`, `project_primitives`,
  `project_api_surface`, `project_cli_binaries`, `project_intent`,
  `project_inference`, `project_external_surface`,
  `project_integration_gaps`, `feedback_parallel_audits`).
- `chirps.daily.v2` connector wired end-to-end. New module
  `crates/emem-fetch/src/chirps.rs` (~310 LoC, 7 unit tests + 1 live
  test). Materializer `materialize_chirps_daily_precip` in
  `crates/emem-api-rest/src/lib.rs` signs Primary on real readings,
  Absence with structured `reason_text` on out-of-bounds (±50° lat),
  before-record (pre-1981), no-data (-9999.0 sentinel). New band
  `chirps.precip_daily_mm` at offset 1672 (1 dim); `reserved` shifted
  to 1673 with dims=119 (Σ=1792 preserved). Function `chirps.precip@1`
  registered. Live verification: Mumbai cell 2023-07-26 returns
  76.2 mm/day, 2023-07-27 returns 304.8 mm/day — heavy-monsoon ground
  truth, signed with populated `receipt.merkle_proof`.
- `/humans` interactive map at `https://emem.dev/humans`. Knowledge
  constellation of the corpus: every attested cell64 is a star
  positioned by Hilbert-ordered (lat,lng) projection (not Mercator),
  brightness scaled by fact density, colour by dominant band family.
  Click a star → right-pane shows facts + signed receipt + verify
  button; verify runs Ed25519 + BLAKE3 in-browser (`@noble/ed25519`
  + `@noble/hashes/blake3` via ESM CDN, falls back to
  `/v1/verify_receipt` if the imports fail). A `find_similar` graph
  view reveals the embedding topology. Console pane prints every
  `/v1/*` call so an LLM watching the page learns the agent API by
  observation. Single self-contained `web/humans.html`, 1101 LoC,
  served via `include_str!` on the new `/humans` route.

#### Changed
- HuggingFace Space `Dockerfile` pinned to
  `ghcr.io/vortx-ai/emem:0.0` (was `:latest`). A `:latest`
  deletion or upstream regression no longer dark-blacks the Space.
  SHA pin recommended in the comment for the next bump.
- `query_region` total-fact cap (`MAX_REGION_FACTS = 65_536`)
  added to defend against pathological dense-corpus + 4096-cell
  bbox combinations.
- `crates/emem-core/src/sources.rs` validator now accepts
  `providers: []` for a declared scheme. Replaces the older "no
  providers" hard-error that forced fake URLs into the manifest.
  An empty `providers[]` means the scheme name is recognised but
  no anonymous open-data path exists today — operators register
  their own key-bearing providers locally.
- `sources-v0.json:openet.30m.daily` providers list cleared with
  a `_note` documenting the blocker (the public S3 mirror returns
  `NoSuchBucket`; OpenET REST API and the GEE asset are both
  key-gated). Replaces the broken URL that previously made
  `/v1/sources` advertise a path that 404'd.

#### Removed
- `Mode::Zk` variant from `verify` — Rust enum
  (`emem-primitives/src/verify.rs`), MCP tool schema
  (`emem-mcp/src/lib.rs`), OpenAPI VerifyReq schema
  (`emem-api-rest/src/lib.rs`). The variant was advertised but had
  zero implementation; `mode=zk` returned 500 on every call. v0.1+
  may revisit.
- `Attestation.stake` field from `crates/emem-fact/src/attest.rs`
  and 9 call sites. Was reserved-for-v2.5; v2.5 will add a
  properly-named field if and when economics is designed.
- `find_similar.filter` Internal-error guard. Replaced with the
  actual evaluator above.
- 14 stale docs (`docs/{AGENTS, ATTESTING, CLIENTS, CONTRIBUTORS,
  DEPLOY, GO_LIVE, MATERIALIZERS, MILESTONE_v0.0.4, MULTIMODAL,
  PUBLISHING, SPACES, SPEC, TEMPORAL, WHITEPAPER}.md`) — replaced
  by the lowercase set above.

#### Fixed
- `verify mode=Resolve` no longer silently behaves as Fast.
- Production deploys no longer require manual `setcap` after every
  release rebuild.
- Receipts now carry Merkle inclusion proofs (was always `None`).

### Audit (parallel, 8 subsystems) — 2026-05-08

Eight Explore agents audited core+codec, fact / claim / attest /
storage, fetch / connectors, primitives, REST+MCP (live),
CLI+intent, GPU sidecar + JEPA / Prithvi / Galileo, and
SDKs+web+deploy. Findings:

- 73 REST endpoints + 34 MCP tools live and schema-aligned, tested
  on `127.0.0.1:5051`.
- 244+ workspace tests pass.
- Sources audit correction: original "11 unwired schemes" claim was
  wrong. Six are wired inline in `emem-api-rest/src/lib.rs`
  materialiser functions (gmrt, ornl_modis, nasa_power, open_meteo
  4-variant, soilgrids.v2, viirs.fire.nrt). Five are genuinely
  unwired: openet.30m.daily, dynamic_world.v1, tropomi.s5p.ch4,
  tropomi.s5p.no2, viirs.dnb.monthly.

## [0.0.4] — 2026-05-XX

Polygon-aware boring endpoints, real physics primitives (heat /
wave PDE solvers + AR(2) NDVI predictor), agent-first homepage,
production SPEC.md, GDPR / UK-GDPR / DPDP-2023 / CCPA-CPRA
compliance surface.

### Added
- **Three real physics primitives.**
  - `POST /v1/heat_solve` — explicit FTCS 2D for `∂u/∂t = α∇²u`
    over a 9-cell stencil at the cell64 10 m pitch. Reads
    `modis.lst_day_8day` at the centre and 8 neighbours, integrates
    forward under `α·Δt/Δx² ≤ 0.20`, returns Kelvin forecast +
    initial condition + chosen `(n_steps, dt_seconds)`. Default
    α=1e-6 m²/s (Oke 2017 §2.3 table 2.4); horizon ≤168 h.
  - `POST /v1/wave_solve` — explicit CTCS 1D shallow-water for
    `∂²u/∂t² = c²∂²u/∂x²` along the seaward bathymetric gradient
    from `gmrt.topobathy_mean`, `c² = g·h`, `c` floored at 0.01 m.
    Sinusoidal forcing at the offshore boundary; hard wall at the
    coast; CFL safety 0.5. Land-locked rejection: offshore
    boundary ≥5 m AND ≥50 % of profile >1 m, else 422 + suggestion.
  - `POST /v1/jepa_predict` — closed-form AR(2) seasonal NDVI
    (`α=0.6, β=0.3, γ=0.1`, lookback ≤24 months). Surfaces
    `lag_12_used` so an agent can audit which terms drove the
    prediction. NOT a learned MLP.
- All three wired through MCP and OpenAPI; receipts verify offline
  via `POST /v1/verify_receipt`. Pure math (`heat_step_2d`,
  `wave_step_1d`, `jepa_predict_ar2_seasonal`) unit-tested without
  storage.
- `POST /v1/jepa_predict_v2` — pulls 3 latest Tessera vintages,
  routes to GPU sidecar, returns 128-D prediction. Receipt carries
  `untrained_baseline` warning until Tessera publishes
  multi-vintage history.
- **Polygon-aware boring endpoints** — `POST /v1/{ndvi, elevation,
  air, lst, soil, water, forest, weather, at}` resolve a place to
  an OSM polygon, fan out to up to 64 sample cells in parallel
  (`tokio::task::JoinSet`), return mean / median / min / max / std
  per band (mode + class distribution for categorical bands;
  centroid for vector embeddings). Knob: `n_cells` (default 16, max
  64, `1` forces point mode at the centroid).
- Visual + structured deliverables on polygon responses:
  `polygon.geojson` outline FeatureCollection, `polygon.scene_thumbs[]`,
  `polygon.scene_overlay_url` pointer, top-level `value_per_cell[]`
  + per-cell `geojson` FeatureCollection.
- `GET /v1/places/scene_overlay.svg?place=&band=&n_cells=&...` —
  server-rendered viridis SVG of the resolved polygon, cells
  coloured across the actual recalled min/max.
- `GET /v1/cells/:cell64/scene.rgb` — raw octet-stream RGB bytes
  with `x-emem-scene-{format,width,height,channels,...}` headers.
- `POST /v1/fetch` — REST mirror of MCP `emem_fetch`. Accepts
  either `{cid}` (lookup) or `{cell, band, [tslot]}` (materialise +
  persist).
- `POST /v1/elevation` cross-band coherent — recalls Cop-DEM (land),
  GMRT (ocean topobathy), ESA WorldCover (LC veto); reports
  `validity ∈ {land, ocean, coastline, unknown}`. Open ocean
  surfaces `elevation_m: null` + signed `bathymetry_m`, eliminating
  the `0.0` ambiguity.
- **Embedded band metadata** in `/v1/recall`, `/v1/cells/:cell`,
  `/v1/recall_polygon`, `/v1/ask`, boring endpoints. Every fact
  carries sibling `band_metadata` (description, units, value_range,
  interpretation, pitfalls, references) + `value_decoded` for
  categorical bands (ESA WorldCover LCCS, JRC Surface Water
  transition class, S2 SCL). Materialiser scalars
  (`copdem30m.elevation_mean`, `surface_water.*`, `s2.scl`) inherit
  metadata from their cube band and surface
  `inherited_from_cube_band`.
- `signer_pubkey_b32` + `responder_pubkey_b32` sibling fields on
  receipts. Raw 32-byte arrays remain intact for byte-for-byte
  verification; the base32-nopad string is for paste-into-`/v1/verify`
  ergonomics.
- `aqi_class@1` algorithm (chained `Where` ops on `cams.pm25` →
  EPA AQI 1-6), `weather_summary@1` (combined; sky / precip / temp /
  wind one-liner; Met Office / WMO METAR / Beaufort 8 thresholds).
  Total algorithms: 102 → 105.
- `air_quality` band entry — carved 7 dims off the front of
  `_reserved_512` (offset 192, shrunk 512 → 505) for CAMS scalars
  (`cams.pm25`, `cams.pm10`, `cams.no2`, `cams.o3`, `cams.so2`,
  `cams.co`, `cams.aod_550`). Bands count: 33 → 34;
  `total_dims` stays 1792.
- `/v1/ask` enrichments: `band_observations[]` inventory
  fall-through, `imagery_hint` block for imagery topics,
  `out_of_scope` caveat suppression when facts already exist,
  per-fact `band_metadata` + `value_decoded`.
- Agent surface: `/v1/openapi.action.json` (curated 28-op subset
  for OpenAI Custom GPT Action's 30-op cap), agent-first homepage
  rewrite, `/llms.txt` rewrite, `/agents.md` §5 anatomy of a
  numeric response, MCP resource templates
  (`emem://{band, algorithm, fact, cell}/...`).
- **GDPR / UK-GDPR / DPDP-2023 / CCPA-CPRA compliance surface.**
  SPEC.md §13 expanded to six subsections: per-band privacy class,
  no-PII-in-canonical-channel, Art. 6 lawful basis, data-subject-rights
  table, no-cookies disclosure, IP-handling
  (`agent_ip_hash = base32_nopad_lower(blake3(client_ip)[..8])`).
  `/v1/discover.fanout` adds `privacy`, `terms`, `spec`.
  `/.well-known/agent-card.json.provider` adds privacy / terms /
  support URLs and a `data_protection` extension.
- Privacy enforcement: `ops/systemd/journald-30day-retention.conf`
  with `MaxRetentionSec=30day`. POST canary verified absent in
  logs; GET canary present (paired with hashed IP, 30-day window).
- Production SPEC.md v0.0.4 (was v0.0.4-draft). §22 references
  split Normative + Informative; 43 citation keys defined for
  every upstream and every RFC-grade reference.
- Privacy + consent fixes (2026-05-06):
  - GA4 measurement ID moved out of public repo. `web/index.html`
    holds `__EMEM_GA_ID__`; the responder substitutes
    `EMEM_GA_MEASUREMENT_ID` at startup or strips the GA block
    entirely.
  - Consent storage moved from `localStorage` to a first-party
    cookie `emem_consent` (Path=/, Max-Age=180 days, SameSite=Lax,
    Secure). EU strict-mode browsers were clearing localStorage
    between sessions.

### Changed
- Workspace bumped to `0.0.4`.
- `/v1/discover` shrunk 130 KB → 1,026 B (134×). One-KB system-prompt
  fit: responder pubkey, 4 manifest CIDs, one-line algebra
  `Cell × Band × Tslot → Fact ; cid=blake3(cbor)/b32-32 ;
  sig=ed25519`, primitive→URL map, fanout pointers.
- `/llms.txt` 20 KB → 5 KB; `/agents.md` 27 KB → 16 KB;
  `index.html` 59 KB → 3.5 KB. Two paragraphs and working playground
  links a tool-less agent can follow.
- Hybrid topic routing — keyword exact-match boost runs ahead of
  the transformer pass even in transformer mode. Closes the
  `model2vec/potion-base-8M` 0.35-threshold gap on common Qwen-style
  prompts where a place noun dominated the embedding pool.
- Aggressive alias enrichment for `vegetation_condition`,
  `optical_raw_reflectance`, `radar_all_weather_sar`,
  `public_health` — the framings agents most commonly use that
  previously scored below threshold.
- Inventory-based algorithm dispatch tightened: requires
  `topics_matched > 0` AND every input the AST reads is in
  `want_bands`. Stops `flood_risk@2` and `aqi_class@1` from firing
  on "show me NDVI for Bengaluru".
- Cross-band coherent `/v1/elevation` (above) is the default now;
  point and polygon both route through `post_elevation_coherent`.
- Algorithm temporal-window materialisation parallelised via
  `tokio::task::JoinSet` — the previous serial 60 s timeout is
  gone for `/v1/temporal_route`.

### Removed
- 14 stale `docs/*.md` files (above) — replaced by lowercase
  `docs/{agents, protocol, architecture, registries, data-sources,
  inference, developing, operating, whitepaper}.md`.

### Fixed
- Polygon fan-out for embedded-gazetteer + cached places.
  `locate_inner` now enriches missing polygon bboxes via a single
  Nominatim `/search?q=…&limit=1` lookup at three sites — embedded
  hit, cache hit with no stored bbox, Photon hit with no extent —
  and re-caches the result so subsequent calls short-circuit.
- Polygon visual deliverables on `POST /v1/elevation` —
  `polygon.scene_thumbs[]`, `polygon.scene_overlay_url`,
  `polygon.geojson` outline now match what NDVI / LST / soil
  responses already shipped.
- Dockerfile: added `g++` to the build stage so `model2vec-rs`
  compiles cleanly in CI.
- Honest caveat suppression: `/v1/ask` `out_of_scope` only emits
  when `topics_matched`, `band_observations`, AND `facts.facts` are
  all empty.

## [0.0.3] — 2026-05-01

Closed gaps surfaced by the Katihar (Bihar) man-made-lake test
report — placeholders, hardcodes, silent fallbacks. Tightened every
protocol surface an external agent touches: geocoder cascade,
temporal vocabulary, algorithm registry, multimodal scene path,
brand identity.

### Added
- **Topic registry + transformer router.**
  `crates/emem-core/data/topics-v0.json` — 25 hand-authored topics,
  each `{key, description, aliases[], bands[], algorithms[]}`. The
  `topic_router` module embeds descriptions + aliases with
  model2vec-rs (`minishlab/potion-base-8M`, ~32 MB, sub-ms
  inference, pure-Rust) and routes free-text questions by cosine
  ≥0.35. Falls back to alias keyword matching when the model fails
  to load. Replaces ~639 lines of static `TOPIC_BANDS` /
  `TOPIC_ALGORITHMS` / `TOPIC_KEYWORDS` tables.
- **Formula-AST evaluator + composite dispatcher.** `Expr` enum
  in `emem-core::algorithms` (15 variants: Band, Const, Add, Sub,
  Mul, Div, Linear, WeightedBlend, Clamp, Where, Abs, Sigmoid,
  Relu, Max, Min). `Expr::evaluate(samples) -> Option<f64>`.
  Algorithms gain optional `evaluation: Expr` field; `flood_risk@2`
  is the proof-of-concept (round-trips canonical-CBOR JSON,
  produces 0.4836 byte-stably).
- **Temporal composition.** `Algorithm.temporal_recipe { windows[],
  label, note }`; `/v1/ask` and `/v1/intent` carry an additive
  `temporal_composition[]`. `flood_risk@2` adds GMRT topo +
  `dem_agreement` weighting term.
- **Sentinel-2 / Sentinel-1 fallback ladders.**
  `s2_search_with_fallback` (40 % cloud / 30 d → 60 % / 60 d → 80 % /
  90 d), `s1_search_with_fallback` (15 d → 30 d → 60 d).
- **Adaptive polygon density.** `RecallPolygonReq.cells_per_sqkm` +
  `drill_on_water` parameters; max-cells cap raised 256 → 1024.
- **Photon (komoot.io) geocoder** as the primary live fallback.
  Cascade: embedded → cache → Photon → Nominatim. Configurable via
  `EMEM_PHOTON_BASE`. `/v1/locate.via` reports the resolved path.
- **Overture release auto-discovery** via S3 ListObjectsV2 + XML
  parse; 24 h cached `ReleaseCache`; `EMEM_OVERTURE_RELEASE`
  override.
- Brand identity refresh — new logo + favicon variants; PNGs
  referenced from `index.html`, `agent.json`, `ai-plugin.json`,
  `gemini-extension.json`.

### Changed
- Tslot anchor: u64 anchored at **Unix epoch** (was emem-2026
  epoch in 0.0.2 — pre-2026 observations collapsed to `Tslot(0)`,
  which broke per-tslot historical backfill).
- `algorithms_for_topic[flood_*]` points at `flood_risk@2`;
  `flood_risk@1` retained so existing receipts still resolve.
- `/v1/recall_polygon.max_cells` cap raised 256 → 1024.

### Removed
- Static topic-routing tables (~639 lines): `TOPIC_BANDS`,
  `TOPIC_ALGORITHMS`, `TOPIC_KEYWORDS` `LazyLock` blocks in
  `emem-api-rest/src/lib.rs`. Same data now in `topics-v0.json`,
  consumed via `TopicRouter`.
- Overpass geocoder fallback. The public Overpass instance returns
  503 under load; Photon serves the same OSM corpus in ~100 ms via
  Elasticsearch.

### Fixed
- `/v1/locate` for rural OSM places (Katihar / Laliyahi) — embedded
  → cache → Photon → Nominatim resolves reliably; Overpass timed
  out.
- `/v1/cells/{cell}/scene.png` no longer black for tiles with
  scattered reflectance (percentile helper now returns
  `Option<f64>` and filters non-finite up front).

### Migration
- No breaking changes. `temporal_composition` and `temporal_recipe`
  are additive sibling fields.
- `flood_risk@1` still in the registry; receipts citing v1
  continue to verify.
- If you hardcoded `via == "overpass"`, change it to
  `via == "photon"`.

### Carryover from post-0.0.2 development
- Native HTTPS via in-process rustls + Let's Encrypt (TLS-ALPN-01).
- Persistent Ed25519 responder identity at
  `<EMEM_DATA>/identity.secret.b32` (mode 0600).
- `/v1/locate` (lat/lng or place name → cell64), `/v1/cells/{cell}/info`,
  `/v1/discover` (one-call agent bootstrap), `/v1/contributors[*]`
  (CoIL leaderboard), `/metrics` (Prometheus).
- Production middleware: 16 MiB body cap, 30 s timeout, per-IP
  token bucket (60/min, 120 burst), HSTS / CSP / X-Content-Type-Options /
  X-Frame-Options / Referrer-Policy / Permissions-Policy, optional
  HTTPS redirect via `EMEM_REDIRECT_HTTPS=1`, graceful shutdown on
  SIGTERM.
- `emem-livedemo` and `emem-realdemo` CLI binaries with full
  request + response + receipt traceability written to
  `var/demos/`.
- Cell64 codec gained `cell_from_latlng` / `latlng_from_cell64`
  pair in `emem-codec::geo` with documented bit layout.

## [0.0.2] — 2026-04-26

Initial open-source release. The protocol surface, primitives, MCP
server, and reference responder are all functional. See
`README.md` for the workspace layout and `docs/operating.md` for
production deployment.

End.
