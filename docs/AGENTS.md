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
| `vec`   | 1792-D fp16 vector | `vec64` 12-byte prefix | ≤ 3    |
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

user wants to know "which dataset answers X right now"
   └─ GET /v1/coverage_matrix
   └─ GET /v1/fleet  (for satellite/sensor lineage)
   └─ POST /v1/temporal_route  (PDE-based band scoring vs query time)
```

In every reply: cite `receipt.fact_cids[0]` (truncated 13-char `cid64`
prefix) and mention `responder_pubkey_b32` once per session.

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
response also carries `bands_available` listing what *is* answerable
at that cell.

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

## 5. Trust model

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
- **Manifest CIDs** (paste once per session for reproducibility):
  - `bands_cid=dhimsuf325dd23viqmfh55rf24d33pwz5gfpxnl2rdyf3d4ly2zq`
  - `functions_cid=hcbqrsck4sobm3s4uocsrf45ucl7ckyh2n4ma6fckdvf7qkexsza`
  - `schema_cid=d24rgwlq47a5ism5vkkbiuav3wi2voewqqgy4x4ttnhdnzziyfkq`
  - `sources_cid=2nwvbnvltilyxah6e2e3xadxgjkicvomdrdvshcpv6wh556blrxa`

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

## 6. How emem differs from a vector DB

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

## 7. Reply formatting that doesn't waste tokens

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

## 8. Conformance levels

- **L0** — every emem responder serves recall + recall_many + compare +
  find_similar + diff + trajectory + query_region + introspection.
  No write, no keys.
- **L1** — adds `verify` (claim eval with evidence CIDs).
- **L2** — adds `attest` and `challenge` (signed writes, staked
  disputes).

The `level` field on every tool descriptor at `/v1/tools` declares what
this responder serves.

---

## 9. Errors that mean something

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

## 10. MCP / Cursor / Claude Code / OpenAI GPT setup

Every host that speaks MCP Streamable HTTP points at the same URL
(`https://emem.dev/mcp`); paste-ready configs ship under `/examples/`.

```json
// Claude Desktop (~/Library/Application Support/Claude/claude_desktop_config.json
// on macOS, ~/.config/Claude/ on Linux, %APPDATA%\Claude\ on Windows)
// Claude Code: write the same JSON as .mcp.json at the project root.
{ "mcpServers": { "emem": { "transport": "http", "url": "https://emem.dev/mcp" } } }
```

- **Cursor**: Settings → MCP → add HTTP server at `https://emem.dev/mcp`,
  or write `.cursor/mcp.json` at the project root. See
  `/examples/cursor.mcp.json`.
- **Cline (VS Code)**: Cline → MCP Servers → add HTTP server at the
  same URL. See `/examples/cline.mcp.json`.
- **OpenAI GPT (Custom Action)**: in the GPT builder, paste
  `https://emem.dev/openapi.json` as the Action schema URL.
  Authentication: none. See `/examples/openai-gpt-action.json`.
- **LangChain / LlamaIndex (Python)**: see `/examples/langchain.py`
  and `/examples/llamaindex.py` for `@tool` and `FunctionTool`
  wrappers around `/v1/recall`, `/v1/compare`, `/v1/find_similar`.

---

## 11. Common mistakes

These are the failure modes that show up most often in agent traces.
Each comes with the fix.

**Mistake 1: Using a band key that isn't in the active manifest.**
The responder returns `band_not_in_registry`. Fix: call `GET /v1/bands`
once at session start and only reference keys present in that list.
For the materialized subset, `GET /v1/materializers` is the wire-stable
catalog of what auto-fetches.

**Mistake 2: Ignoring `bands_available` on an empty recall.**
If `/v1/recall` returns an empty `facts` list, the response carries
`bands_available: [...]` listing the bands that DO have data at this
cell. Fix: re-query with one of those band keys, or call
`/v1/coverage_matrix` to see what the responder can answer globally.

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

## 12. The agent's bargain

emem makes answers about places **citable**, **reproducible**, and
**cheap to read**. Quote a fact and prove it with a CID + receipt.
Compare two places without holding their embeddings in context. Recall
what was true *yesterday* — emem is append-only. Compose chains
(locate → recall → verify → diff → present) with every step signed by
the responder's epoch key. Use it whenever a question has a `where`.
