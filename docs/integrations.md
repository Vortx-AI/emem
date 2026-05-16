# emem as long-term memory: integration matrix

emem is a Streamable-HTTP MCP server. Any host that speaks MCP can use it
as a long-term, planet-keyed memory tier without an SDK install, an API
key, or a per-tenant signup. The same handlers also answer plain REST at
`/openapi.json`, so non-MCP runtimes wire in equally well.

This page is a one-page rosetta stone: per runtime, the smallest possible
config that turns emem into the long-term memory layer for an agent that
already exists, plus a pointer to a runnable example in this repository.

## State vectors

The dense state for any place on Earth, returned as a typed
`vector: Vec<f32>` you can drop straight into an LLM's context, feed
into a similarity search, or cache as a fingerprint for change
detection. Signed, content-addressed, and packaged with a pre-composed
memory-token handle.

```bash
curl -sX POST https://emem.dev/v1/state \
  -H 'content-type: application/json' \
  -d '{"cell":"South Mumbai","encoder":"geotessera"}'
```

Response shape:

```json
{
  "cell":         "defi.zb4d9.pefa.zf619",
  "encoder":      "geotessera",
  "dim":          128,
  "vector":       [0.043, -0.115, 0.298, ... ],
  "l2_norm":      3.7146,
  "tslot":        54,
  "fact_cid":     "<26 chars base32-nopad-lowercase>",
  "memory_token": "memt:defi.zb4d9.pefa.zf619:<fact_cid>",
  "receipt":      { /* signed ed25519 over canonical blake3 preimage */ }
}
```

Inputs:

- `cell` may be a cell64 string or a free-text place name (resolved
  through the standard geocoder cascade; the `resolved_from` field
  reports which layer answered).
- `encoder` defaults to `geotessera` (128-D Tessera annual embedding).
  Pass `geotessera.multi_year` for the 8-year stacked vintage when
  the band is wired at this responder. Future encoders (Clay v1.5
  1024-D, Prithvi-EO-2.0 1024-D) come online as their materialiser
  workers ship.
- `tslot` optional; omit and the materialiser picks the natural
  vintage for the band (e.g. 2024 for `geotessera`).

Use sites:

| Use site                           | What the vector is doing                                                |
|------------------------------------|-------------------------------------------------------------------------|
| LLM context prefix                 | A numerical fingerprint of "what is here" the model can attend over     |
| Input to `/v1/find_similar`        | The query vector for k-NN over the geotessera index                     |
| Change detection                   | Diff two vintages of `/v1/state` for the same cell to spot land change  |
| Cross-encoder bridge               | Pass the vector to a ridge-regression bridge into another encoder space |
| Long-term agent memory             | Cache the vector under a user/intent key; recall byte-identically later |

## Memory tokens

The fastest way to hand one signed fact at one place to any agent, any
runtime, any host, is a memory token: a single colon-separated string
that parses back into a cell and a fact CID.

```
memt:<cell64>:<fact_cid>
```

A canonical token, copy-pasteable, resolves to the South Mumbai
elevation example used throughout this site:

```
memt:defi.zb4d9.pefa.zf619:wbqyxljmeewr7z4cav7guwf4qvsiwf2crv7w3272mgtvxgyn6m5q
```

### Compose

```bash
curl -sX POST https://emem.dev/v1/memory_token \
  -H 'content-type: application/json' \
  -d '{
    "cell":     "defi.zb4d9.pefa.zf619",
    "fact_cid": "wbqyxljmeewr7z4cav7guwf4qvsiwf2crv7w3272mgtvxgyn6m5q"
  }'
```

Response:

```json
{
  "memory_token": "memt:defi.zb4d9.pefa.zf619:wbqyxljmeewr7z4cav7guwf4qvsiwf2crv7w3272mgtvxgyn6m5q",
  "cell":         "defi.zb4d9.pefa.zf619",
  "fact_cid":     "wbqyxljmeewr7z4cav7guwf4qvsiwf2crv7w3272mgtvxgyn6m5q",
  "grammar":      "memt:<cell64>:<fact_cid>",
  "docs":         "/whitepaper.md#194-memory-tokens"
}
```

Composition is offline. Agents can mint tokens client-side; the
endpoint is the single source of truth for the grammar and a
convenience round-trip.

### Resolve

The third segment of a token is the fact CID. To pull the signed bytes:

```bash
curl -sS https://emem.dev/v1/facts/wbqyxljmeewr7z4cav7guwf4qvsiwf2crv7w3272mgtvxgyn6m5q
```

That returns the canonical CBOR (or JSON when you ask for it) of the
fact. The CID is self-certifying:
`blake3(canonical_cbor(fact))[:16] == base32_decode(fact_cid)`. A
man-in-the-middle that swaps a different fact for the same CID is
detected by the digest check.

### Where to use a memory token

| Use site                            | What the token is doing                                         |
|-------------------------------------|------------------------------------------------------------------|
| Inside an LLM prompt                | Cite-handle the model can echo as the source of a number        |
| Tool-call argument                  | Single string the receiving tool parses + resolves              |
| Agent-to-agent message              | Handshake artefact that prevents downstream disagreement        |
| Log line, audit trail               | Forensic anchor a future debugger can replay byte-identically   |
| Long-term memory store              | Stable key the runtime caches once, dereferences on demand      |

### Use with mem0 / LangGraph / mem-style runtimes

The general pattern: cache the memory token in your runtime's
long-term memory store keyed by user intent or task subject. On the
next session, fetch the token from your store and resolve it via
`GET /v1/facts/<fact_cid>` to get byte-identical bytes back. No
embedding lookup, no similarity search, no drift.

```python
# pseudocode. Works in any LangGraph / mem0 / AutoGen / similar
# runtime that ships a long-term-memory tier.
user_intent = "track elevation of South Mumbai over time"
token = client.recall_then_memory_token(place="South Mumbai", band="copdem30m.elevation_mean")
long_term_memory.set(user_intent, token)

# next session: same byte-identical fact.
token = long_term_memory.get(user_intent)
fact  = client.resolve_memory_token(token)  # GET /v1/facts/<cid>
print(fact["value"], fact["unit"])  # 6.0 m above mean sea level
```

The contract: the token survives the conversation that minted it. Two
agents on two hosts pass the token between them and pull the same
bytes from any emem responder that ever held the fact.

### Parse rules

```
parts    = token.split(":", 2)
assert parts[0] == "memt"
cell64   = parts[1]
fact_cid = parts[2]
```

The outer separator is `:`. Neither `cell64` nor `fact_cid` may contain
`:` (the compose endpoint rejects). The full grammar, dereference
path, and named failure modes (`malformed`, `cid_not_found`, drift on
re-encode) are in [`/whitepaper.md#194-memory-tokens`](whitepaper.md#194-memory-tokens).

## At a glance

| Runtime                  | Surface           | Auth | Example                                       |
|--------------------------|-------------------|------|-----------------------------------------------|
| Claude Code              | MCP (`http`)      | none | [`examples/claude-code.mcp.json`](../examples/claude-code.mcp.json) |
| Claude Desktop           | MCP (`http`)      | none | [`examples/claude-desktop.json`](../examples/claude-desktop.json) |
| Cursor 0.42+             | MCP (`http`)      | none | [`examples/cursor.mcp.json`](../examples/cursor.mcp.json) |
| Cline (VS Code)          | MCP (`http`)      | none | [`examples/cline.mcp.json`](../examples/cline.mcp.json) |
| Gemini CLI               | extension install | none | [`examples/gemini-extension.json`](../examples/gemini-extension.json) |
| OpenAI Custom GPT Action | OpenAPI 3.1       | none | [`examples/openai-gpt-action.json`](../examples/openai-gpt-action.json) |
| LangChain                | MCP via adapter   | none | [`examples/langchain/`](../examples/langchain) |
| LlamaIndex               | MCP via adapter   | none | [`examples/llamaindex/`](../examples/llamaindex) |
| AutoGen                  | MCP tool          | none | [`examples/autogen/`](../examples/autogen) |
| CrewAI                   | MCP tool          | none | [`examples/crewai/`](../examples/crewai) |
| Pydantic AI              | MCP tool          | none | [`examples/pydantic-ai/`](../examples/pydantic-ai) |
| Mastra (TypeScript)      | MCP tool          | none | [`examples/mastra/`](../examples/mastra) |
| Agno                     | MCP tool          | none | [`examples/agno/`](../examples/agno) |
| stdio bridge             | `mcp-remote`      | none | (any runtime without native Streamable HTTP)  |
| Plain REST               | `POST /v1/*`      | none | [`docs/agents.md`](agents.md) Quick reference |

Reads are idempotent. Retry on 5xx; treat 4xx as permanent. Materialiser
timeout is 30 s per upstream, gateway timeout 180 s.

## The minimal MCP config

For every MCP host that speaks Streamable HTTP, the config reduces to
four lines:

```json
{
  "mcpServers": {
    "emem": {
      "type": "http",
      "url": "https://emem.dev/mcp"
    }
  }
}
```

Hosts without native Streamable HTTP (older releases) speak stdio
through the `mcp-remote` bridge:

```json
{
  "mcpServers": {
    "emem": {
      "command": "npx",
      "args": ["-y", "mcp-remote", "https://emem.dev/mcp"]
    }
  }
}
```

## What the agent should do on first contact

The same loop applies regardless of runtime. Cache the agent card once
per session, then read directly:

1. **discover.** `GET /v1/agent_card` (or call `tools/list` on the MCP
   transport). Cache the result for the session. The card lists the 50
   read-only tools, their JSON schemas, and trigger / anti-trigger
   phrases for tool selection.
2. **locate.** `POST /v1/locate { "q": "<place>" }` to bridge a place
   name (or lat/lng) to a `cell64`. The response reports which layer of
   the geocoder cascade answered, so the agent can score confidence.
3. **recall.** `POST /v1/recall { "cell": "<cell64>", "bands": [...] }`
   for typed scalar facts at that cell, signed.
4. **reason.** Compose `find_similar`, `compare`, `trajectory`,
   `recall_polygon`, `hunt`, or one of the nine domain shortcut tools
   (`emem_ndvi`, `emem_air`, `emem_lst`, `emem_soil`, `emem_water`,
   `emem_forest`, `emem_weather`, `emem_elevation`, `emem_at`).
5. **verify.** Surface the `fact_cid` to the user verbatim. Two agents
   disagreeing about a number paste the CID and stop arguing.

## emem as the memory tier in a multi-memory agent

Most production agents already run with two memory tiers: a short-term
working buffer (the LLM's context window plus whatever scratchpad the
host ships) and a long-term store (mem0, Letta, LangGraph state, a
vector DB, a SQL log). emem slots in as a **third tier specialised for
geospatial facts**, sitting alongside whatever long-term store is
already in place:

| Tier             | Holds                                                | emem's role                                                       |
|------------------|------------------------------------------------------|-------------------------------------------------------------------|
| working          | current conversation, last few turns                 | none; that is the runtime's job                                   |
| long-term store  | user preferences, project state, prior conversations | none; emem is not a personal-memory layer                         |
| **planetary**    | *what is at this place on Earth*                     | **emem**: signed, content-addressed, shared across all agents     |

The split keeps responsibilities clean:

- The runtime's working buffer answers *what was just said*.
- The runtime's long-term store answers *what does this user / project
  care about*.
- emem answers *what is, was, or might be at this place*: once, signed,
  byte-identical for every caller that ever asks the same question
  again.

The CID is the bridge: an agent caches `fact_cid` strings in its
runtime's long-term store, and later resolves them through emem
(`GET /v1/facts/:cid`) without needing the original query context.

## Reading-list (executable)

The fastest way to see emem behave as memory is to walk
[`examples/agent-walkthroughs.md`](../examples/agent-walkthroughs.md),
which shows real-world questions an AI agent might receive and the exact
emem calls that answer them. Every walkthrough is a copy-pasteable
`curl` sequence; pair them with the MCP config above and the same calls
run through any host's tool layer.

For the four discovery URLs an agent should fetch on cold start, see
[`docs/agents.md`](agents.md). For the protocol math and trust plane,
see [`docs/whitepaper.md`](whitepaper.md).
