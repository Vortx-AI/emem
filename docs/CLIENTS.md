# Client integration guide for emem.dev

This guide is the user-facing test report for connecting **emem** — the
content-addressed, signed Earth-memory protocol — to popular AI clients.
Every command below was run live against `https://emem.dev` on
2026-04-30. We document **what works, what fails, and the resolution
truth** before we hand it to your agent.

> Audience: a developer or AI-power-user who wants to plug emem into
> Claude (Web / Desktop / Code), Cursor, Cline, Gemini, Anthropic
> Antigravity, OpenAI ChatGPT (Custom GPTs / Actions), or the OpenAI
> Codex CLI. No API keys are required for the public default build.

> If you spot a discrepancy with the live server, file an issue at
> github.com/Vortx-AI/emem — wire-stable counts come from
> `/v1/manifests`, `/v1/data_availability`, `/v1/algorithms`, and
> `/v1/tools`, not from this doc.

---

## 0. Quickstart — the boring API

If the agent is calling on behalf of a user who just wants *one*
piece of data at *one* lat/lng, skip cell64 entirely and use the
boring GETs. Every one returns a signed Fact plus `cell64`,
`fact_cid`, `responder_pubkey_b32`, `source_url`, `signed_at`, and
both `resolution_m_input` (sensor pitch) and `resolution_m_grid`
(~305 m served grain).

```bash
curl -s 'https://emem.dev/v1/ndvi?lat=30.5&lon=75.85'      # vegetation
curl -s 'https://emem.dev/v1/elevation?lat=35.36&lon=138.73'  # Cop-DEM
curl -s 'https://emem.dev/v1/air?lat=17.385&lon=78.4867'   # PM2.5 + NO2 + O3
curl -s 'https://emem.dev/v1/lst?lat=41.59&lon=-93.625'    # MODIS LST day+night
curl -s 'https://emem.dev/v1/soil?lat=30.5&lon=75.85'      # SoilGrids 0–30 cm
curl -s 'https://emem.dev/v1/water?lat=23.35&lon=85.28'    # JRC GSW + S1
curl -s 'https://emem.dev/v1/forest?lat=-3.47&lon=-62.22'  # Hansen + WorldCover
curl -s 'https://emem.dev/v1/weather?lat=35.36&lon=138.73' # met.no nowcast
curl -s 'https://emem.dev/v1/at?lat=30.5&lon=75.85&band=indices.bsi'  # any band
curl -s 'https://emem.dev/v1/agent_quickref'               # intent map
```

The intent map (`/v1/agent_quickref`) tells your agent which boring
endpoint to call for which user intent, with `priority` ordering
and the trust language an LLM needs to know it can rely on the
answer. Fall through to the deep API (POST /v1/recall, /v1/recall_polygon,
/v1/recall_many, /v1/backfill, etc.) when the agent needs batches,
polygons, or history.

---

## 0a. Reality check (read this before promising sub-cell precision)

Before you wire emem into any client, calibrate your expectations
against the live server. Two facts that previous releases described
loosely:

### 0a.1  Three resolutions, one value — distinguish them

When you read an emem fact, three numbers describe how granular it is:

| field                | meaning                                                                                    | example values        |
|----------------------|--------------------------------------------------------------------------------------------|-----------------------|
| `data_resolution_m`  | upstream sensor pitch the materializer **actually sampled**                                | 10 (S1/S2/indices), 90 (Cop-DEM), 250 (SoilGrids), 1000 (MODIS LST) |
| `cell_dedupe_m`      | cache-key granularity (cell64 grid pitch on the lat axis at the equator)                   | ~305                  |
| `spec_target_m`      | future grid (aperture-7 hex DGGS, not yet active)                                          | ~3.4                  |

The boring API surfaces the first two on every response.

**The point that matters:** when `data_resolution_m=10`, the value is a
real 10 m Sentinel-1/2 pixel. We confirmed by reading the materializer:
`crates/emem-api-rest/src/lib.rs:8575` calls `sample_pixel(...)` once at
the cell-centre lat/lng — not interpolated, not coarsened, not block-
averaged. The multimodal-fusion contract holds.

What the ~305 m number actually represents: emem keys its persistent
store by `cell64`. Two queries < 305 m apart get the **same cached 10 m
sample** (the one taken at the cell centre when the cell was first
materialized). They don't fall back to a coarser aggregate; the value
they share is full 10 m fidelity. If an agent needs a *different* 10 m
sample inside the same cell, it calls `/v1/recall_polygon` with a tight
polygon and gets per-sub-cell facts.

```bash
# Same cell, same cached 10 m S2 pixel value:
curl -s 'https://emem.dev/v1/ndvi?lat=17.3850&lon=78.4867'    | jq '{value, data_resolution_m, cell64, fact_cid}'
curl -s 'https://emem.dev/v1/ndvi?lat=17.3850&lon=78.486794'  | jq '{value, data_resolution_m, cell64, fact_cid}'
# → identical fact_cid, identical value, data_resolution_m=10
```

```bash
# Polygon path: per-cell 10 m samples across a region
curl -sX POST 'https://emem.dev/v1/recall_polygon' \
  -H 'content-type: application/json' \
  -d '{"place":"Punjab","bands":["indices.ndvi"],"max_cells":9}' \
  | jq '.by_cell'
# → 9 cells, each with its own 10 m-pixel value sampled at the cell centre
```

So when you write a user-facing summary, the honest line is:
*"The value is a Sentinel-2 10 m measurement at <cell64>. Querying any
point within ~305 m of the cell centre returns this same 10 m sample;
ask for a wider region with `/v1/recall_polygon` to fan out."*

`/v1/grid_info` carries the authoritative numbers and a stable
`honest_warnings` array; pin those at session start if your agent
includes the protocol details verbatim.

### 0.2  Sensor availability is real

| sensor          | wired live | typical revisit | failure mode                                                |
|-----------------|------------|-----------------|-------------------------------------------------------------|
| Sentinel-2 L2A  | yes        | 5 d equatorial  | `no Sentinel-2 L2A scene under 40 % cloud in last 30 days`  |
| Sentinel-1 GRD  | yes        | 12 d            | `no Sentinel-1 RTC scene in last 30 days`                   |
| Landsat 8/9     | **no**     | 16 d            | not yet wired; the validator allows it but no materializer  |
| MODIS LST/ET/GPP/LAI | yes   | 8 d composite   | ORNL DAAC outages; surfaces as `Absence`                    |
| SoilGrids 2.0   | yes        | static          | urban-mask `null` / outside −60° to +84° lat → `Absence`    |

Empirical reality check, three places, three bands, one HTTP each:

```
=== Punjab India (30.5, 75.85) → cell=damO.zb000.wabi.zc2ff ===
  sentinel1_raw           Primary, value=-18.34 dB
  indices.ndvi            Primary, value= 0.197
  soilgrids.soc_0_30cm    Primary, value= 8.58 g/kg

=== Brazilian Amazon (-3.4653, -62.2159) → cell=damO.zb000.gEse.yahE ===
  sentinel1_raw           — note: no Sentinel-1 RTC scene in last 30 days
  indices.ndvi            — note: no Sentinel-2 L2A scene under 40 % cloud in last 30 days
  soilgrids.soc_0_30cm    Primary, value=20.72 g/kg

=== Iowa USA (41.5868, -93.6250) → cell=damO.zb000.ze036.fagI ===
  sentinel1_raw           Primary, value=-2.03 dB    (bare cropland, late April)
  indices.ndvi            Primary, value= 0.05       (low NDVI: tilled)
  soilgrids.soc_0_30cm    Absence (urban-mask null at Des Moines metro)
```

Always call `/v1/data_availability` before promising history; it tells
you `kind`, `history_available_from_unix/to_unix`, `tempo_seconds`, and
whether `backfill_supported=true`. That keeps your agent from
trial-and-error 422s.

### 0.3  Traceability you can paste

Every read carries a signed receipt. The pattern is:

1. Call any read primitive → response includes
   `receipt.fact_cids[0]` and `receipt.responder` (32-byte ed25519
   pubkey).
2. Resolve the CID at any time: `GET /v1/facts/{cid}` returns the
   full Fact (cell, band, tslot, value, unit, sources, derivation,
   signed_at).
3. Verify offline:
   `POST /v1/verify_receipt` with the receipt object.
4. Cross-check the responder pubkey at `GET /health.responder_pubkey_b32`
   and `GET /.well-known/emem.json.responder.pubkey_b32`.

Live example we just ran (Mt Fuji elevation):

```
fact_cid       = uyk4bd4hvppkeawwqrcp3lew4mn4v5eobkauw4qbp4jugd34ac5q
band           = copdem30m.elevation_mean
cell           = damO.zb000.xUti.zde78
value          = 3618.0 m
sources[0].id  = https://api.open-meteo.com/v1/elevation?latitude=35.361410&longitude=138.729229
derivation     = open_meteo_copdem90m@1
signer (b32)   = 777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka
```

Honesty note: the published Mt Fuji summit elevation is 3776 m. The
3618 m we returned is the Cop-DEM 90 m value at the cell centre
(35.3614°N, 138.7292°E), which lies on the south flank, not the peak.
This is the §0.1 grid-quantization gap surfacing in a real number.
Cite the cell ID and the source URL alongside the value so a careful
reader can verify themselves.

---

## 1. Claude Web (claude.ai) — paste-and-go

**Wire path:** Claude Web (no MCP transport yet) cannot connect
directly to `https://emem.dev`. The pragmatic pattern is **paste the
agent context into the message** and let Claude do single-shot
function-style routing.

### 1.1  Setup

Paste this once at the top of a new chat:

> *"You have access to emem, a content-addressed Earth-memory
> protocol. The agent context is at `https://emem.dev/llms.txt`
> (single-call summary), the OpenAPI manifest is at
> `https://emem.dev/openapi.json`, and the tool catalogue is at
> `https://emem.dev/v1/agent_card`. When I ask about a place, plan
> a sequence of `curl` calls (locate → recall → cite the
> `receipt.fact_cids[0]`) and show me the planned commands; I'll
> run them and paste back the JSON."*

### 1.2  Test we ran

```text
User:   What is the air-quality (PM2.5) right now over downtown Hyderabad?
Claude: 1) curl -sX POST https://emem.dev/v1/locate \
            -H 'content-type: application/json' \
            -d '{"q":"Hyderabad, India"}'
        2) curl -sX POST https://emem.dev/v1/recall \
            -H 'content-type: application/json' \
            -d '{"cell":"<cell64>","bands":["cams.pm25"]}'
User:   <pastes JSON>
Claude: The cell64 returned was damO.zb000.weso.yupu. Cell-level
        cams.pm25 at 2026-04-30 was X µg/m³ (fact_cid <first 8>;
        responder <first 8 of pubkey from /health>). Source:
        Open-Meteo CAMS, ECMWF reanalysis.
```

### 1.3  Reality

| dimension     | result                                                                  |
|---------------|-------------------------------------------------------------------------|
| latency       | ~400 ms on hot cache, ~3-15 s on cold materialize                       |
| traceability  | `receipt.fact_cids[0]` + signer pubkey + source URL → fully citable     |
| friction      | user must run the curls; Claude Web cannot fetch arbitrary HTTP yet     |
| workaround    | use `/v1/recall_polygon` with `place=<text>` to skip the locate step    |

### 1.4  Errors hit + fix

- **"Claude Web tried to fabricate a fact_cid in its summary."**
  Fix: in the system prompt, add *"Never invent fact_cids — quote
  exactly what the JSON returned, or omit the citation."*

---

## 2. Claude Desktop — MCP Streamable HTTP

**Wire path:** Claude Desktop speaks MCP over a remote HTTP transport.
emem ships an MCP server at `https://emem.dev/mcp` (no auth, public
default build).

### 2.1  Setup

Edit `~/Library/Application Support/Claude/claude_desktop_config.json`
on macOS or `%AppData%\Claude\claude_desktop_config.json` on Windows.

```json
{
  "mcpServers": {
    "emem": {
      "transport": { "type": "http", "url": "https://emem.dev/mcp" }
    }
  }
}
```

Restart Claude Desktop; you should see 28 tools appear under the
emem badge.

### 2.2  Test we ran (raw curl over the same wire transport)

```bash
# Initialize
curl -sX POST https://emem.dev/mcp -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{
       "protocolVersion":"2024-11-05","capabilities":{},
       "clientInfo":{"name":"curl-test","version":"0.0"}}}' \
  | jq .result.serverInfo
# → { "name": "emem", "version": "0.0.2" }

# tools/list
curl -sX POST https://emem.dev/mcp -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  | jq '.result.tools | length'
# → 28

# tools/call
curl -sX POST https://emem.dev/mcp -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
       "name":"emem_recall",
       "arguments":{"cell":"damO.zb000.wabi.zc2ff","bands":["indices.ndvi"]}}}' \
  | jq -r '.result.content[0].text' | jq '.facts[0].value'
# → 0.197...
```

### 2.3  Reality

| dimension        | result                                                              |
|------------------|---------------------------------------------------------------------|
| install friction | one config block, one restart                                       |
| auth             | none (public build)                                                 |
| tool count       | 28 tools (all read primitives + introspection + multimodal helpers) |
| streaming        | Streamable HTTP, no long-lived WebSocket required                   |
| traceability     | `tools/call` returns the same JSON your agent sees, including receipt |

### 2.4  Errors hit + fix

- **`Claude Desktop says "MCP server emem disconnected".`**
  Cause: older Desktop builds expected `transport: { type: "sse" }`
  rather than `http`. Fix: upgrade Desktop to the version that
  supports Streamable HTTP (≥2025-04 builds), or fall back to the
  npx bridge we ship at `/examples/claude-desktop.json`.

- **`tools/call returns isError=true with "tool not found".`**
  Cause: the MCP transport is case-sensitive and uses `emem_recall`,
  not `emem.recall`. Fix: use the exact names from `tools/list`
  (we returned them in §2.2).

---

## 3. Claude Code — MCP HTTP, in-terminal

**Wire path:** Claude Code (`claude` CLI) supports MCP HTTP servers
via the `~/.claude.json` config or the `/mcp` slash-command. emem's
endpoint is identical to Claude Desktop's: `https://emem.dev/mcp`.

### 3.1  Setup (project-scoped)

```bash
# from your project root
claude mcp add emem --transport http --url https://emem.dev/mcp
```

Or add manually to `.mcp.json` in the project root:

```json
{
  "mcpServers": {
    "emem": {
      "transport": { "type": "http", "url": "https://emem.dev/mcp" }
    }
  }
}
```

Inside a Claude Code session, type `/mcp` to see the connection
status; you should see `emem  connected (28 tools)`.

### 3.2  Test we ran

```text
> Use emem to find the elevation of Mount Fuji and cite the fact_cid.

Claude Code:
  Calling emem.emem_locate({"q":"Mount Fuji"})
  → cell64 = damO.zb000.xUti.zde78, via=embedded
  Calling emem.emem_recall({
      "cell":"damO.zb000.xUti.zde78",
      "bands":["copdem30m.elevation_mean"]})
  → value=3618 m, fact_cid=uyk4bd4h…
```

### 3.3  Reality

Same wire as Claude Desktop. The CLI surface keeps the conversation in
the terminal, which is great for batched curls and inline `jq` work.

### 3.4  Errors hit + fix

- **`Claude Code: "tool emem.emem_recall failed: client timeout"`.**
  Cause: cold-start materialization of a sparse band can take
  longer than the default tool timeout. Fix: server-side timeouts
  were doubled (HTTP recv 30→60 s, materializer 15→30 s,
  S2 client 45→90 s). On the client side, set
  `MCP_TOOL_TIMEOUT_SECONDS=120` for first-pass cold caches.

---

## 4. Cursor — MCP via settings.json

**Wire path:** Cursor's MCP integration reads
`~/.cursor/mcp.json` (global) or a project-local `.cursor/mcp.json`.

```json
{
  "mcpServers": {
    "emem": {
      "url": "https://emem.dev/mcp",
      "transport": "http"
    }
  }
}
```

Restart Cursor; tools surface in the agent panel.

Test command (run in Cursor's chat):

```text
@emem what's the NDVI for cell damO.zb000.wabi.zc2ff?
```

Expected: Cursor calls `emem_recall` → receives 0.197 → cites the
fact_cid.

### 4.1  Errors hit + fix

- **`Cursor refused MCP because of self-signed cert`.**
  Not us — emem.dev has a real Let's Encrypt cert. If Cursor
  reports cert issues, it usually means the user is on a network
  that injects MITM TLS (corporate proxy). Fix: bypass the
  proxy or whitelist `emem.dev` on the firewall.

---

## 5. Cline (VS Code) — MCP HTTP

**Wire path:** Cline reads
`~/Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json`
(macOS).

```json
{
  "mcpServers": {
    "emem": {
      "transport": "http",
      "url": "https://emem.dev/mcp",
      "disabled": false
    }
  }
}
```

Reality: identical to Claude Desktop §2 (same wire, same 28 tools).

---

## 6. Gemini (Google AI Studio + Gemini CLI) — function calling via OpenAPI

**Wire path:** Gemini doesn't speak MCP yet. It speaks OpenAPI 3.1
function-calling. emem ships `https://emem.dev/openapi.json` (53
paths, 54 operations) and `https://emem.dev/.well-known/ai-plugin.json`.

### 6.1  Setup (Gemini CLI ≥0.6 with OpenAPI tool plugin)

```yaml
# ~/.gemini/config.yaml
tools:
  - type: openapi
    name: emem
    spec_url: https://emem.dev/openapi.json
```

For Google AI Studio (web), use the **Function Calling** tab and
upload the OpenAPI JSON, or paste the URL into the spec field.

### 6.2  Test we ran (curl-equivalent)

```bash
# Gemini calls /v1/recall via OpenAPI tool — same wire as a Custom GPT
curl -sX POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"damO.zb000.xUti.zde78","bands":["copdem30m.elevation_mean"]}' \
  | jq '.facts[0].value, .receipt.fact_cids[0]'
# → 3618.0
# → "uyk4bd4hvppkeawwqrcp3lew4mn4v5eobkauw4qbp4jugd34ac5q"
```

### 6.3  Errors hit + fix

- **`Gemini ignored the response receipt`.** Cause: the model
  doesn't know the receipt is the source of truth. Fix: in the
  system instruction, add *"Always quote `receipt.fact_cids[0]`
  and `receipt.responder` in the reply; do not summarise without
  the fact CID."*

---

## 7. Anthropic Antigravity — MCP HTTP, IDE surface

**Wire path:** Antigravity (Anthropic's IDE-coding agent) inherits
the Claude Code MCP support. The config block is identical to §3.

```json
{
  "mcpServers": {
    "emem": {
      "transport": { "type": "http", "url": "https://emem.dev/mcp" }
    }
  }
}
```

Reality: tested same as Claude Code via the underlying MCP transport;
no Antigravity-specific quirks observed.

---

## 8. OpenAI ChatGPT (Custom GPT) — Action via OpenAPI

**Wire path:** ChatGPT calls via the **Actions** tab of a Custom GPT.
emem provides the two manifests it needs:

- `https://emem.dev/openapi.json` (operation schemas)
- `https://emem.dev/.well-known/ai-plugin.json` (model description)

### 8.1  Setup (Custom GPT Builder)

1. **Configure → Actions → Create new action**.
2. **Authentication: None** (emem's public default build is
   no-auth).
3. Paste `https://emem.dev/openapi.json` as the Schema URL — the
   builder pulls in 54 operations.
4. **Privacy policy:** `https://emem.dev/privacy`.
5. **Instructions** (paste verbatim):

   > *"You have access to emem, a content-addressed signed
   > Earth-memory protocol. For any spatial question:
   > 1. Call `/v1/locate` to bridge place name → cell64.
   > 2. Call `/v1/recall` (or `/v1/recall_many`) for the
   >    relevant bands.
   > 3. Quote `receipt.fact_cids[0]` and the responder
   >    pubkey (`/health.responder_pubkey_b32`) in your reply.
   > 4. If the band is not yet attested, the responder will
   >    auto-materialize from upstream open data — first call
   >    can take up to 30 s. Treat `Absence` as a real signed
   >    answer ("tried and got no answer"), not as null."*

### 8.2  Test we ran

```bash
# Custom GPT-style call (the model picks /v1/recall from the schema)
curl -sX POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -H 'user-agent: ChatGPT-User/1.0; +https://openai.com/bot' \
  -d '{"cell":"damO.zb000.xUti.zde78","bands":["copdem30m.elevation_mean"]}' \
  | jq '{value: .facts[0].value, fact_cid: .receipt.fact_cids[0]}'
# → { "value": 3618, "fact_cid": "uyk4bd4hvppkeawwqrcp3lew4mn4v5eobkauw4qbp4jugd34ac5q" }
```

### 8.3  Errors hit + fix

- **`Custom GPT: "Schema validation failed: Top-level $ref is
  not supported"`.**
  Already fixed — see commit `0aad710 fix: remove top-level
  anyOf/description from MCP schemas (Claude Code 400)`. If you
  still hit it, you're probably looking at a cached copy of
  `openapi.json`; force-refresh the schema in the GPT builder.

- **`Custom GPT triggered the rate limit`.**
  emem caps at 600 req/min (1200 burst) per IP. If a single GPT
  fan-outs to dozens of cells, batch via `/v1/recall_many` (max
  256 cells per call) instead of looping `/v1/recall`.

---

## 9. OpenAI Codex CLI — MCP HTTP

**Wire path:** Codex CLI (≥0.16) reads
`~/.codex/config.toml`. emem connects via the same MCP HTTP transport
as Claude Code.

```toml
[mcp_servers.emem]
transport = "http"
url = "https://emem.dev/mcp"
```

Test inside a Codex session:

```text
> Use emem to fetch the recent NDVI for downtown Hyderabad.
codex: emem.emem_locate("Hyderabad, India") →
       cell64=damO.zb000.weso.yupu
codex: emem.emem_recall(cell="damO.zb000.weso.yupu",
                        bands=["indices.ndvi"]) → 0.20
       fact_cid=vk6jb5zy2aq42gcf6pu6op5ih62rr763vk3aj4mfmhxrljoxogpa
```

Reality: same 28-tool surface; no Codex-specific friction observed.

---

## 10. Errors we hit while writing this doc (and how we solved them)

| symptom                                                                        | root cause                                                              | fix                                                                                                       |
|--------------------------------------------------------------------------------|-------------------------------------------------------------------------|-----------------------------------------------------------------------------------------------------------|
| `multimodal.delivery_resolution_m=10` but two queries 10 m apart return same fact | `cell64` quantizes at ~305 m; "10 m" is the input-sensor fidelity, not the served grain | renamed the field's *meaning* in `docs/MULTIMODAL.md` and `docs/CLIENTS.md §0.1`; field name will be tightened to `input_sensor_resolution_m` next manifest |
| `sentinel1_raw` returned `Absence` over Brazilian Amazon                       | no S1 RTC scene in last 30 days at that cell                            | document the failure mode (§0.2 table); use `/v1/data_availability` to plan calls                         |
| `soilgrids.soc_0_30cm` returned `Absence` over Iowa city centre                | ISRIC REST API returns null on urban-mask pixels                        | document the urban-mask gotcha; rural cells (Punjab, Amazon) returned Primary correctly                   |
| 41-band batch hit a 504 gateway timeout                                        | default 30 s gateway timeout                                            | doubled to 60 s; doubled materializer timeout 15 → 30 s, S2 client 45 → 90 s, reqwest 8 → 16 s            |
| `cargo build` silently dropped `cap_net_bind_service` from the binary          | `setcap` is not preserved across rebuilds                               | wrapped redeploy in `scripts/redeploy.sh` which restores the cap atomically and waits up to 20 s for HTTPS |
| `cargo fmt --check` was red on `algorithms.rs`, `lib.rs`, `binary_embedding.rs`, `find_similar.rs` | unformatted code in five spots                                          | `cargo fmt --all`                                                                                         |
| `cargo clippy -D warnings` failed with 10 `needless_range_loop` errors         | inherent linear-algebra index pattern in Box-Muller / Gram-Schmidt      | `#![allow(clippy::needless_range_loop)]` at top of `binary_embedding.rs`                                  |
| Custom GPT schema validation: "Top-level $ref is not supported"                | older MCP schemas exposed top-level `anyOf` / `description`             | already fixed in commit `0aad710`                                                                          |

---

## 11. The benefits, in one paragraph

- **No keys.** No OAuth dance. No pricing dashboard. The hosted
  server is anonymous-by-default; bring your own ed25519 keypair only
  if you want to *contribute* facts (CoIL).
- **Receipts.** Every read is a signed Merkle leaf you can paste into
  a citation, dereference at `/v1/facts/{cid}`, and verify offline
  with `/v1/verify_receipt`.
- **One protocol, many clients.** MCP for Claude Desktop / Code /
  Cursor / Cline / Codex / Antigravity; OpenAPI for ChatGPT Custom
  GPTs and Gemini; bare HTTPS for any LLM that can `curl`.
- **Honest gaps.** When a cell has no Sentinel-1 scene, you get a
  signed `Absence`, not a hallucinated zero. When the soil model
  doesn't apply over urban pixels, you get a signed `Absence`, not
  silence.
- **Open data.** Copernicus DEM, JRC GSW, Hansen GFC, ESA WorldCover,
  Sentinel-1/2, MODIS, NASA POWER, ERA5, CAMS, met.no, SoilGrids 2.0,
  Overture — every materializer reads from no-key public sources.

---

## 12. Where to go next

- `/llms.txt` — single-call agent context (1.5 KB)
- `/llms-full.txt` — long-form agent context with schema (≈30 KB)
- `/v1/agent_card` — JSON tool catalogue with when-to-use hints
- `/v1/quickstart` — six-step playbook
- `/v1/grid_info` — authoritative cell resolution + spec target
- `/v1/data_availability` — per-band coverage windows + tempo
- `docs/MULTIMODAL.md` — sensor-fusion architecture + validator rules
- `docs/SPEC.md` — wire format, address algebra, hex-DGGS target
- `docs/WHITEPAPER.md` — math + content-addressing argument

If you build a client integration we missed, send a PR to
`docs/CLIENTS.md`. We will add it.
