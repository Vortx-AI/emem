# emem v2 — Spatial Memory for AI Agents

> **ARCHIVED** — frozen 2026-04-26 design proposal kept for historical
> context only. The implemented protocol is in `docs/SPEC.md` and the
> shipped feature set is in `CHANGELOG.md`. Anything in this file that
> refers to "v2.5", staking, slashing, on-chain anchoring, contribution
> credits, IPLD/Filecoin replication, or any token economy is
> **aspirational and is not implemented in 0.0.x**. Read for context;
> do not cite as a feature contract.
>
> Status: proposal · Author: 2026-04-26 · Supersedes: `docs/product-memory.md`

## TL;DR

emem v1 was a what3words clone with a dimensionality contract bolted on. v2 is something different: **a content-addressed, cryptographically verifiable, agent-contributed memory protocol for what is true about Earth.** Not a SaaS — an infra layer. Agents both *read* (recall, verify, find_similar) and *write* (attest), and once a fact is computed it is hashed, deduplicated, and persisted for every other agent that ever asks the same question. Pure agent product, no human mode. Backed by the 1792D AgriSynth latent cube as the bootstrap corpus, but designed so any agent can extend coverage. Speaks MCP first, REST second, hashes everywhere.

## The reframe: emem is a protocol, not a service

Earlier drafts of this doc treated emem as a hosted memory service. That ceiling is too low. The right shape is a **spatial memory protocol** with these properties:

1. **Content-addressed facts.** Every fact about a place is keyed by `(cell_id, band, time_bin)` and valued by `blake3(canonical_cbor(fact))`. Two agents that compute the same fact from the same sources converge on the same hash. Storage cost drops from `O(agents × facts)` to `O(unique facts)`. The hash *is* the identifier.
2. **Append-only, immutable.** Frozen facts (e.g., "NDVI at cell X on 2025-07-14 was 0.62") are never overwritten — they are settled, signed, and citeable forever. Versioned facts (e.g., "current land cover") chain Merkle-style; old versions remain queryable.
3. **Agents contribute compute.** When an agent answers a question and the underlying fact has no canonical attestation yet, the agent submits one — a signed, sourced, deterministically-reproducible record. The next agent gets the answer in O(1) instead of re-running the pipeline.
4. **Cryptographically verifiable end-to-end.** Sources (Sentinel-2 tiles, AlphaEarth bands) carry their own provider hashes. Computations are deterministic. Receipts are ed25519-signed. Attestation batches are Merkle-rooted. Anyone can re-run a disputed fact from raw sources to ground truth and prove the canonical answer.
5. **Optional on-chain anchoring.** Every N hours we publish the Merkle root of the attestation log to a public chain (Ethereum mainnet via L2, or Filecoin DealClient). The protocol state is independently auditable without Vortx in the loop.
6. **Vortx is the bootstrap operator, not the ceiling.** v2.0 ships closed: Vortx is the only attester, agents read. v2.5 opens write: any agent stakes credits, attests, and earns contribution credits redeemable for future recalls or governance votes.

This is "Filecoin for spatial truth" — but the unit isn't bytes, it's **facts**, and the proof isn't storage, it's **deterministic re-execution from public sources**.

## Why now

| Player (2026) | Surface | Niche owned |
|---|---|---|
| Mapbox MCP | geocode · routing · isochrones | **transactional** ("where is X / route A→B") |
| Google Maps Grounding Lite | places · weather · routing | **freshness** ("what's open right now") |
| CARTO MCP | analytic SQL over user data | **enterprise GIS** ("run my workflow") |
| Hindsight | retain / recall / reflect | **general agent memory** (not spatial) |
| **emem v2** | recall · query · compare · verify · find_similar | **spatial memory + verification** ("what is *true* here") |

Nobody owns "given a structured claim about a place, prove or refute it from sensor truth." That is the wedge. Every shipped agent that touches geography hallucinates it; every insurance/agriculture/trading/compliance/sustainability agent needs the opposite.

## What v2 deletes from v1

1. **Human mode** in `App.tsx` — gone. No more "copy the words" flow. The address layer becomes a thin display nicety; the primary key is the H3 cell ID.
2. **Custom 3m Mercator grid** in `geotessera.ts` — replaced by **H3 (Uber)**. H3 is the de-facto industry standard, has hierarchical resolutions (res 0 = continent, res 13 = ~3.4m), is in every modern data stack (BigQuery, DuckDB, Snowflake, Polars), and agents are already trained on it. We keep the 3-word codec as a vanity render but it is not load-bearing.
3. **Procedural-as-default** — gone. v2 returns `unknown` when we don't know. Honest absence > confident placeholder. Procedural is opt-in, only for tutorials.
4. **Hand-rolled "intent planner"** in `agent.ts` — gone. Real LLMs route real tools via MCP. We don't fake reasoning.
5. **Express + TypeScript as the data plane** — demoted to a thin gateway. The cube data lives in numpy (`farms/*/cube_10m.npz`). The data plane becomes Python + FastAPI, where it can actually touch the bytes.

## What v2 keeps

1. **The 1792D band contract** (`bands.ts`) — this is correct. Becomes the source-of-truth ontology.
2. **Provider abstraction** — also correct. v2 just adds two more providers: `local-cube` (reads agri npz directly) and `lance-index` (vector search).
3. **Provenance over silence** — the v1 principle "real bands are never faked" stays. v2 sharpens it: every response carries a signed receipt.
4. **The React+Vite frontend** — repurposed as the developer playground (request builder, response inspector, MCP-config generator), not as a human map viewer.

## Architecture

```
                        ┌──────────────────────────────────────────────────┐
                        │  Agents (Claude / GPT / Gemini / Llama / custom) │
                        └───────────────┬─────────────┬────────────────────┘
                                        │ MCP         │ REST
                              ┌─────────▼──────┐  ┌───▼────────────┐
                              │  FastMCP        │  │  FastAPI       │
                              │  service/mcp/   │  │  service/api/  │
                              └─────────┬──────┘  └───┬────────────┘
                                        │             │
                                        ▼             ▼
            ┌──────────────────────── PRIMITIVES (service/primitives/) ─────────────────────────┐
            │ recall · query_region · compare · verify · find_similar · trajectory · diff       │
            └─────────────────────────────────┬─────────────────────────────────────────────────┘
                                              │
            ┌─────────────────┬───────────────┼───────────────┬─────────────────────────────────┐
            ▼                 ▼               ▼               ▼                                 ▼
   ┌──────────────┐  ┌────────────────┐  ┌─────────┐  ┌──────────────────┐  ┌─────────────────────┐
   │ Cell engine  │  │ Cube loader    │  │ Bands   │  │ Vector index     │  │ Receipt signer       │
   │ (H3)         │  │ (npz · COG)    │  │ registry│  │ (LanceDB)        │  │ (ed25519 · cell hash)│
   │ service/cells│  │ service/cubes/ │  │ synced  │  │ service/index/   │  │ service/receipts/    │
   └──────────────┘  └────────────────┘  │ from    │  └──────────────────┘  └─────────────────────┘
                                          │ agri    │
                                          │ BAND_   │
                                          │ OFFSETS │
                                          └─────────┘

                            ┌────────────────────────────────────────┐
                            │  Developer playground (Vite + React)   │
                            │  - request builder                     │
                            │  - response viewer (with receipts)     │
                            │  - copy-as: curl · python · MCP config │
                            │  - cell map (live coverage heatmap)    │
                            └────────────────────────────────────────┘
```

## Protocol primitives

Two write primitives, six read primitives, one settlement primitive. Every response carries a `Receipt`: `request_id`, `served_at`, `cell_ids[]`, `cids[]` (content IDs of facts referenced), `band_provenance[]`, `source_versions[]`, `merkle_proof?` (path to current attestation root), `signature` (ed25519).

### Write

#### `attest(facts[], sources[], computation_proof)`
The contribution primitive. `facts[]` is a batch of `(cell_id, band, time_bin, value, derivation_id)` tuples. `sources[]` lists the upstream artifacts (Sentinel-2 tile T43PGS_2025-07-14, AlphaEarth band-12_2024, etc.) by their canonical hashes. `computation_proof` is either:
- a **deterministic re-execution recipe** (function name + version + args), so any verifier can reproduce — gold standard
- or a **stake commitment** (agent puts up credits that get slashed if the fact is later challenged and refuted) — pragmatic fallback

Returns: per-fact CIDs, batch Merkle root, attestation_id, agent's credit delta.

In v2.0, only Vortx-issued keys can attest. In v2.5 this opens to staked third parties.

#### `challenge(attestation_id, counter_evidence)`
Disputes a fact. `counter_evidence` is itself an attestation with conflicting value + sources. Triggers protocol-level re-execution from sources; if the original attestation is refuted, its stake is slashed and challenger is rewarded. v2.5 feature; stubbed in v2.0.

### Read

#### `recall(cell_id | coordinates, bands?, time?)`
"What do you know about this cell." Returns the canonical attestation per `(cell, band, time_bin)`: its CID, value, sources, signer, timestamp. Bands with no attestation return `unknown` — honest absence. Sub-second p99 against the LanceDB hot tier.

#### `query_region(geometry, bands?, agg?)`
Geometry is bbox or GeoJSON polygon. Aggregates band signal over the contained H3 cells via canonical attestations. `agg` ∈ {mean, median, p90, mode, vector_centroid}. Vector centroid returns the average 1792D embedding — the embedding *of the region itself*, suitable as input to downstream models.

#### `compare(cell_a, cell_b, family?)`
Vector-space cosine similarity + per-band delta. Returns which families agree and which disagree. Useful for "is this place like that place" reasoning.

#### `find_similar(cell_or_vector, k=10, filter?)`
k-NN over the LanceDB index of all attested cells. `filter` accepts SQL-like predicates over metadata (`country = "IN" AND ecoregion = "tropical_forest"`). Returns ranked cells with similarity scores + receipts. **This is the AlphaEarth-RAG primitive.**

#### `verify(claim, cell)` — the wedge

The killer feature. `claim` is structured:

```json
{ "type": "land_cover_is", "value": "urban", "as_of": "2025-07" }
{ "type": "ndvi_above", "value": 0.6, "month": "2025-07" }
{ "type": "drought_index_below", "value": -2.0, "year": 2024 }
{ "type": "deforested_since", "year": 2020 }
{ "type": "in_protected_area" }
```

Returns `{agree: bool, confidence: 0..1, evidence: [{cid, band, value, source, captured_at}], receipt}`. Evidence carries the CID an agent can dereference forever — claims become *citeable* even after the protocol moves on. **This is what underwriters, auditors, traders, and journalists pay for.**

If the relevant fact has no attestation, the verifier transparently:
1. Returns `unknown` immediately if `mode=fast`, OR
2. Triggers a self-attestation (Vortx as fallback ingester) and returns the verified result with a fresh CID if `mode=resolve`.

#### `trajectory(cells[] | linestring, sampling?)`
Streams band summaries / latent vectors along a path. SSE in REST, async generator in MCP. Lets autonomous agents (logistics, drone planning, environmental survey) maintain a "where am I in memory" cursor while moving.

#### `diff(cell, t0, t1, bands?)`
Temporal change report. Backed by AlphaEarth year-to-year deltas (we have 9 years), Hansen forest change, JRC water occurrence change. Returns per-band deltas + a verbal summary. Each historical value is a separate frozen CID — diffs are walks across the immutable log.

### Settlement

#### `subscribe(cell_or_region, predicate)` *(v2.1)*
Webhook when a band crosses a threshold. Out of scope for v2.0.

### Memory

#### `recall(cell_id | coordinates, bands?, time?)`
"What do you know about this cell." Returns the 1792D contract: per-band status (live · derived · procedural · unknown), summary statistics (NDVI mean, dominant land class, slope, etc.), latest capture date, and the receipt. Bands marked `unknown` are honest — agents must know when they're guessing.

#### `query_region(geometry, bands?, agg?)`
Geometry is bbox or GeoJSON polygon. Aggregates band signal over contained H3 cells. `agg` ∈ {mean, median, p90, mode, vector_centroid}. Vector centroid returns the average 1792D embedding — the embedding *of the region itself*, suitable as input to downstream models.

#### `compare(cell_a, cell_b, family?)`
Vector-space cosine similarity + per-band delta. Returns which families agree and which disagree. Useful for "is this place like that place" reasoning.

#### `find_similar(cell_or_vector, k=10, filter?)`
k-NN over the LanceDB index of all materialized cells. `filter` accepts SQL-like predicates over metadata (`country = "IN" AND ecoregion = "tropical_forest"`). Returns ranked cells with similarity scores + receipts. **This is the AlphaEarth-RAG primitive.**

### Verification — the wedge

#### `verify(claim, cell)`
The killer feature. `claim` is structured:

```json
{ "type": "land_cover_is", "value": "urban", "as_of": "2025-07" }
{ "type": "ndvi_above", "value": 0.6, "month": "2025-07" }
{ "type": "drought_index_below", "value": -2.0, "year": 2024 }
{ "type": "deforested_since", "year": 2020 }
{ "type": "in_protected_area" }
```

Returns `{agree: bool, confidence: 0..1, evidence: [{band, value, source, captured_at}]}`. Backed by:
- `land_cover_is` → `landcover` band + Sentinel-2 RGB sanity
- `ndvi_above` → `indices` band time series
- `drought_index_below` → `terraclimate` PDSI
- `deforested_since` → `forest_change` Hansen
- `in_protected_area` → `protected` WDPA

Each `evidence` entry has the source citation an agent can show its caller. **This is what underwriters, auditors, traders, and journalists pay for.**

### Navigation

#### `trajectory(cells[] | linestring, sampling?)`
Streams band summaries / latent vectors along a path. SSE in REST, async generator in MCP. Lets autonomous agents (logistics, drone planning, environmental survey) maintain a "where am I in memory" cursor while moving.

#### `diff(cell, t0, t1, bands?)`
Temporal change report. Backed by AlphaEarth year-to-year deltas (we have 9 years), Hansen forest change, JRC water occurrence change. Returns per-band deltas + a verbal summary.

#### `subscribe(cell_or_region, predicate)` *(v2.1)*
Webhook when a band crosses a threshold. Out of scope for v2.0.

## Cell system: H3 + thin display layer

- **Primary key**: H3 cell ID at resolution 13 (~3.4m). Hierarchies: res 11 (~24m) for sub-field, res 9 (~174m) for neighborhood, res 7 (~1.2km) for tile, res 5 (~9km) for region.
- **3-word vanity address**: still computable from any cell ID, but it's a display feature, not a primary key. `emem://chairs.glide.tomato` is what you put in a tweet; `8d2a1072b59afff` (H3) is what an agent uses.
- **Coordinates → cell**: `cells.from_latlng(lat, lng, res=13)`; **cell → coordinates**: `cells.to_latlng(cell_id)`; **cell → bounds**: `cells.boundary(cell_id)`; **cell → parents/children**: `cells.parent(cell_id, res)`, `cells.children(cell_id, res)`.
- **Migration**: existing `cellId` strings (BigInt rendered) are quarantined to a `legacy_cell_id` field on responses for one minor version, then deprecated.

## Tech stack

The data plane goes **Rust** from day one. Verification is the wedge, and verification needs to be (a) fast enough to be called inside an agent's tool loop without budget panic and (b) cryptographically tight enough that a forged or buggy receipt is a nuisance, not an exploit. Python on top of numpy can hit (a) but not (b) — too many `unsafe` C boundaries, GIL bottleneck on fan-out workloads, slow cold start, and a runtime that nobody trusts to sign anything. Rust hits both.

| Layer | Choice | Why |
|---|---|---|
| Cell index | **`h3o`** (pure-Rust H3 port) | faster than C-h3, no FFI, hierarchies, ecosystem |
| HTTP server | **Axum + tokio + tower** | async, type-safe, mature, single static binary |
| Cube I/O | **`ndarray` + `ndarray-npy`**, mmap where possible | reads agri's `.npz` cubes directly, zero-copy where the file allows |
| Vector store | **LanceDB (Rust-native crate)** | columnar, vectors + metadata, embedded, Arrow zero-copy |
| Receipts | **`ed25519-dalek`** over canonical CBOR | tamper-evident, ~50µs sign, the agent can show its caller |
| MCP server | **`rmcp`** (official Rust MCP SDK), stdio + HTTP+SSE | speaks the protocol agents already discover |
| Auth + metering | **JWT (`jsonwebtoken`) + Postgres usage table (`sqlx`)** | swap to Stripe metered when ready |
| OpenAPI | **`utoipa`** auto-generates spec from handlers | every Rust handler emits its own JSON Schema, agents autodiscover |
| Gateway | existing Express **deleted** in v2.0; the Rust binary serves directly | one binary, one port, no Node in the path |
| Playground | existing React+Vite, restyled as a developer tool | not a customer surface; an evaluation surface |
| Container | **`scratch`-based Docker image** (~15MB), `cross` for multi-arch | edge-deployable; cold start <50ms |

Build layout (Cargo workspace at repo root):

```
crates/
  emem-core/        # cell math (h3o), bands ontology, receipt signer
  emem-cubes/       # cube loader (npz mmap, lance index reader)
  emem-primitives/  # recall · query_region · compare · verify · find_similar · trajectory · diff
  emem-api/         # axum HTTP server, utoipa OpenAPI, SSE
  emem-mcp/         # rmcp MCP server (stdio + HTTP+SSE) wrapping primitives
  emem-cli/         # `emem serve` · `emem ingest` · `emem keygen`
sdks/
  emem-py/          # thin Python client (auto-generated from OpenAPI)
  emem-ts/          # thin TypeScript client (auto-generated from OpenAPI)
```

The Python that already exists in agri (cube builder, ATF training) stays in agri. emem v2 is a *consumer* of the cubes that agri produces — it does not build them. Clean separation: agri = factory, emem = warehouse + retail.

## Discovery: how agents actually find emem

Building a great memory product is half the battle. The other half is the agent walking up to it without us telling them. We hit every discovery surface a 2026 agent might use:

### 1. MCP — primary

- **Marketplace listings**: register on `mcpmarket.com`, `mcp.so`, the Anthropic MCP directory, the Glama directory, Smithery. One-line install: `npx -y @emem/mcp` (a thin Node wrapper that downloads our Rust binary on first run) or `cargo install emem-mcp`.
- **stdio transport**: `npx @emem/mcp` works out of the box for Claude Desktop / Cursor / Cline / Continue.
- **HTTP+SSE transport**: hosted at `https://mcp.emem.dev/sse`. One URL, agent mounts us, done.
- **Tool naming convention**: `emem.recall`, `emem.query_region`, `emem.verify`, etc. Always namespaced — agents need to disambiguate when they have 50 servers connected.
- **Tool descriptions written for LLMs**: each tool has a 1-sentence summary, a "when to use this" line, an example invocation, and an example output. LLMs route on these strings — they are product copy.

### 2. Well-known discovery files — every base URL

Any deployment of emem (ours, customer's, on-prem) exposes:

- `GET /.well-known/ai-plugin.json` (the OpenAI plugin manifest, retained for backward compat)
- `GET /.well-known/llms.txt` (Anthropic-led 2025 standard for AI-readable site maps)
- `GET /.well-known/mcp.json` (emerging MCP discovery doc — endpoint, transports, tool catalog)
- `GET /openapi.json` (utoipa-generated, served by axum)
- `GET /v2/discovery` (custom — capability matrix, supported claim types, free-tier quotas, region coverage map, model fingerprints)

Every primitive response also carries `discovery_url` and `mcp_endpoint` headers, so an agent that hits us once knows how to mount us properly the next time.

### 3. Native SDKs on every package registry

- `pip install emem` (PyPI)
- `npm install emem` (npm)
- `cargo add emem` (crates.io)
- `go get github.com/Vortx-AI/emem-go` (Go modules)

Each SDK has a ~50-line "Getting started" that an LLM coding agent can ingest and run end-to-end without reading docs.

### 4. URI scheme that agents can dereference

- `emem://h3:8d2a1072b59afff` — primary cell URI
- `emem://3wa:chairs.glide.tomato` — vanity address
- `emem://verify?cell=8d2a1072b59afff&claim=ndvi_above:0.6:2025-07` — single-shot verify URL

Agents that emit these URIs (in chain-of-thought, in answers, in tool args) train downstream agents to use us. URIs become the meme.

### 5. Hugging Face presence

- Publish a public sample of the 1792D embeddings as a HF dataset (`Vortx-AI/emem-cells-sample`). ML researchers' agents discover us via dataset search.
- Publish a small reference model (`Vortx-AI/emem-mini`) — a tiny adapter trained on emem embeddings — as proof that the vectors are useful. Agents looking for "geospatial embeddings to fine-tune on" land here.

### 6. OGC / institutional catalogs

- Optional but valuable for credibility: register as an **OGC API — Features** endpoint and a **STAC catalog**. USGS, ESA, UN, and university agents discover services via OGC catalogs.

### 7. Search engines + AI overviews

- `https://emem.dev/cell/h3/8d2a1072b59afff` — server-side rendered HTML page per cell, with JSON-LD `Place` schema.org metadata embedded. ChatGPT browsing, Perplexity, Google AI Overviews surface us when an agent's caller types a coordinate or address.
- Every cell page links to the structured `recall` JSON, the verify endpoint, and the SDK docs. The crawl path is the conversion funnel.

### 8. Embedded "powered by emem" attribution

When agents return verified claims to humans, the receipt links back to `emem.dev/r/<receipt_id>`. Click-through traffic is brand-building among the humans whose agents are already calling us. Compounding flywheel.

### Discovery success metric

A naked Claude/GPT/Gemini agent — given just a question that mentions a place — should auto-mount emem within 5 turns and call `verify` within 10. We measure this with an internal eval suite of 200 agent-style prompts; today's number is 0%, target by GA is 60%+.

## Pricing & business model

| Tier | What you get | Pricing |
|---|---|---|
| **Free** | cell math, public bands (DEM, Köppen, ecoregion, admin), 1k recalls/day | $0 |
| **Metered** | live optical/radar recall · region aggregation · vector search · claim verification | $0.0005–$0.05/call by primitive |
| **Vector export** | 1792D embeddings as Parquet/Arrow for downstream training | $/GB or $/M cells |
| **Subscription** | quota bundles, low-latency edge, support | $99–$999/mo |
| **Enterprise** | dedicated cube ingestion (run agri pipeline on customer bbox), private vector index, SLA | custom |

Free tier exists so devs can evaluate. Metered tier is the volume play. Vector export is the highest-ARPU SKU because foundation models need clean training signal. Enterprise is for the customer who says "we have 50,000 sites and need them all in the cube."

## Provenance receipt schema

```json
{
  "request_id": "rcpt_01JK2WVN0XB...",
  "served_at": "2026-04-26T14:32:11Z",
  "cell_ids": ["8d2a1072b59afff"],
  "primitive": "verify",
  "band_provenance": [
    { "band": "indices",  "source": "Sentinel-2 L2A", "captured_at": "2025-07-14T05:11:00Z", "tile": "T43PGS", "checksum": "sha256:..." },
    { "band": "landcover","source": "Overture+SAM3+NDVI v3", "captured_at": "2025-12-31",   "checksum": "sha256:..." }
  ],
  "source_versions": { "alphaearth": "v2025.4", "agri_pipeline": "v2.1.3", "h3": "4.1.0" },
  "signature": "ed25519:..."
}
```

Why receipts: the value of `verify` is destroyed if the agent can't prove the answer to its caller. The receipt is the proof. It is also the audit trail for compliance use cases.

## Open questions for the user

1. **Hosting target** — Fly.io / Railway / our own GPU box? Affects whether the data plane can mmap cubes locally or has to read from S3 (`vortx-agri` bucket already exists).
2. **MCP transport for v2.0** — stdio-only (local agents) or stdio + HTTP+SSE? HTTP+SSE adds auth complexity but is what hosted agents need.
3. **3-word lexicon** — keep the made-up syllabic words (`bahijomelu`)? Or switch to a real-English wordlist (more memorable, more legal risk vs what3words)? Recommend: keep syllabic; declare it a "vanity address," not a navigation primitive.
4. **Licence** — same OSS question that's open in agri (#41). Recommend Apache-2.0 for the SDK + MCP server, source-available (BSL or similar) for the data plane so we can build a hosted business.
5. **Scope of v2.0 ship** — recommend: H3 + recall + find_similar + verify + MCP server + new playground UI. Defer trajectory, diff, subscribe, region aggregation to v2.1.

## Migration / rollout

| Step | Action | Surface |
|---|---|---|
| 1 | Cargo workspace scaffold; `emem-core` band registry + h3 cell engine; unit tests | PR #1 |
| 2 | `emem-cubes` npz mmap loader; `recall` primitive end-to-end (cube → axum → JSON receipt) | PR #2 |
| 3 | `emem-mcp` stdio server wrapping `recall`; `npx @emem/mcp` thin Node bootstrapper | PR #3 |
| 4 | LanceDB ingestion of 56-farm cells; `find_similar` primitive | PR #4 |
| 5 | `verify` with first 5 claim types (land_cover, ndvi_above, drought, deforested, in_protected) | PR #5 |
| 6 | Strip human-mode from `App.tsx`; ship developer playground (Vite still, just restyled) | PR #6 |
| 7 | Discovery surfaces: `/.well-known/*`, OpenAPI, llms.txt, MCP marketplace submissions | PR #7 |
| 8 | SDKs auto-generated from OpenAPI for Python / TS / Go | PR #8 |
| 9 | Public beta: docs site, pricing, hosted MCP at `mcp.emem.dev/sse` | PR #9 |

The Python `service/` track named earlier in this doc is **superseded** by the Rust workspace. Existing Express server stays running until step 6, then is removed.

This is how we move from "interesting project" to "every agent that touches geography has a reason to mount emem on day one."
