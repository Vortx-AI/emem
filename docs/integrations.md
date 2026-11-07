# Connect an agent runtime to emem

emem is a Streamable HTTP MCP server at `https://emem.dev/mcp`. Any MCP host can use it as a long-term memory for
facts about places. You need no SDK, no API key and no account to read. The same handlers also answer plain REST,
described at `/openapi.json`.

This page gives the smallest config for each runtime and a link to a runnable example in `examples/`. Then it shows
the calls an agent makes after it connects. The requests and responses on this page ran on emem.dev on 7 November
2026.

## Connect an MCP host

### Hosts that use `mcpServers`

For a host with native Streamable HTTP, the config has one entry:

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

`"type": "http"` is the MCP name of the Streamable HTTP transport. The URL itself is HTTPS.

`/mcp` advertises the 18 core tools in `tools/list`, so the host registers about 75 KB of descriptors
rather than about 346 KB for all 116. Both sizes are from 7 November 2026 (1 KB = 1024 bytes). The other 98 tools stay callable by name.
`emem_tools` searches them and returns the schema of one tool. To register all 116 tools at the start, use
`https://emem.dev/mcp/full`. It returns them in more than one page, so follow `nextCursor`.

### VS Code and GitHub Copilot

VS Code reads `.vscode/mcp.json`, or the profile file that **MCP: Open User Configuration** opens. Its top-level key
is `servers`, not `mcpServers`:

```json
{
  "servers": {
    "emem": {
      "type": "http",
      "url": "https://emem.dev/mcp"
    }
  }
}
```

The wrong key fails silently: the JSON is valid, but VS Code shows no server and no error. emem is also in the GitHub
MCP Registry. To install it with no file, search `@mcp emem` in the Extensions view, or run:

```bash
code --add-mcp '{"name":"emem","type":"http","url":"https://emem.dev/mcp"}'
```

Copilot Chat shows the tools in **Agent** mode only. The default **Ask** mode shows no tools.

### Hosts without Streamable HTTP

Older hosts speak stdio only. Use the `mcp-remote` bridge:

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

### Runtimes and their examples

| Runtime | Surface | Auth | Example |
|---|---|---|---|
| Claude Code | MCP (`http`) | none | [`examples/claude-code.mcp.json`](https://github.com/Vortx-AI/emem/blob/main/examples/claude-code.mcp.json) |
| Claude Desktop | MCP (`url` only) | none | [`examples/claude-desktop.json`](https://github.com/Vortx-AI/emem/blob/main/examples/claude-desktop.json) |
| Cursor 0.42 and later | MCP (`url` only) | none | [`examples/cursor.mcp.json`](https://github.com/Vortx-AI/emem/blob/main/examples/cursor.mcp.json) |
| Cline (VS Code) | MCP (`url` only) | none | [`examples/cline.mcp.json`](https://github.com/Vortx-AI/emem/blob/main/examples/cline.mcp.json) |
| Gemini CLI | extension install | none | Run `gemini extensions install https://github.com/Vortx-AI/emem`. It reads [`gemini-extension.json`](https://github.com/Vortx-AI/emem/blob/main/gemini-extension.json) at the repository root. |
| OpenAI Custom GPT Action | OpenAPI 3.1 | none | [`examples/openai-gpt-action.json`](https://github.com/Vortx-AI/emem/blob/main/examples/openai-gpt-action.json) |
| LangChain | MCP through an adapter | none | [`examples/langchain/`](https://github.com/Vortx-AI/emem/tree/main/examples/langchain) |
| LlamaIndex | MCP through an adapter | none | [`examples/llamaindex/`](https://github.com/Vortx-AI/emem/tree/main/examples/llamaindex) |
| AutoGen | MCP tool | none | [`examples/autogen/`](https://github.com/Vortx-AI/emem/tree/main/examples/autogen) |
| CrewAI | MCP tool | none | [`examples/crewai/`](https://github.com/Vortx-AI/emem/tree/main/examples/crewai) |
| Semantic Kernel | MCP tool | none | [`examples/semantic-kernel/`](https://github.com/Vortx-AI/emem/tree/main/examples/semantic-kernel) |
| Mastra (TypeScript) | MCP tool | none | [`examples/mastra/`](https://github.com/Vortx-AI/emem/tree/main/examples/mastra) |
| Agno | MCP tool | none | [`examples/agno/`](https://github.com/Vortx-AI/emem/tree/main/examples/agno) |
| Pydantic AI | MCP tool | none | No example yet. Use the standard MCP client config with `https://emem.dev/mcp`. |
| Dify | MCP (`http`) | none | No example yet. Add `https://emem.dev/mcp` as an MCP server in the tool settings of the workspace. |
| OpenAI Codex | MCP | none | No example yet. Use native Streamable HTTP if the build has it, or the `mcp-remote` bridge. |
| Any stdio-only host | `mcp-remote` | none | The bridge config above. |
| Plain REST | `POST /v1/*` | none | The quick reference in [`docs/agents.md`](agents.md). |
| Any runtime, as a guardrail | `POST /v1/guard/verdict` | none | [Check a draft before you publish it](#check-a-draft-before-you-publish-it) |

For a Custom GPT, import `https://emem.dev/openapi.action.json`, not `/openapi.json`. On 7 November 2026 the full
document was 405,695 bytes and the action document was 126,810 bytes. A Custom GPT cannot hold the full document.

A host that has no row here reaches the same 116 tools at the same URL. The rows are the runtimes with a checked-in
example or a known config.

### Run a framework example

Each framework folder has a README and one script. The script connects over Streamable HTTP and asks one question
about a place. emem needs no key, but each script uses an OpenAI model, so set `OPENAI_API_KEY` first.

```bash
cd examples/langchain
pip install langchain-mcp-adapters langgraph langchain-openai
export OPENAI_API_KEY="sk-..."
python emem_mcp_geospatial_agent.py
```

The other folders use the same steps with these values:

| Folder | Install | MCP client in the script | Endpoint |
|---|---|---|---|
| `langchain/` | `pip install langchain-mcp-adapters langgraph langchain-openai` | `MultiServerMCPClient` | `/mcp` |
| `llamaindex/` | `pip install llama-index-tools-mcp llama-index-llms-openai llama-index-core` | `BasicMCPClient` | `/mcp` |
| `autogen/` | `pip install autogen-agentchat 'autogen-ext[openai,mcp]'` | `McpWorkbench` | `/mcp` |
| `crewai/` | `pip install 'crewai[tools]' 'mcp[cli]'` | `MCPServerAdapter` | `/mcp/full` |
| `semantic-kernel/` | `pip install semantic-kernel` | `MCPStreamableHttpPlugin` | `/mcp` |
| `agno/` | `pip install agno openai` | `MCPTools` | `/mcp` |
| `mastra/` | `npm install`, then `npx tsx emem_mcp_geospatial_agent.ts` | `MCPClient` from `@mastra/mcp` | `/mcp/full` |

Set `EMEM_MCP_URL` to point a script at your own node. On 7 November 2026, `tools/list` on `/mcp` returned 18
tools in one page. On `/mcp/full` the first page had 12 tools and a `nextCursor`.

To use REST instead of MCP, `examples/langchain.py` wraps nine routes as LangChain tools and `examples/llamaindex.py`
wraps six as LlamaIndex tools. Copy `langchain.py` to a new name, such as `langchain_emem.py`, before you import it.
The name `langchain.py` hides the `langchain` package. Set `EMEM_URL` to use a node other than emem.dev.

### Timeouts and retries

Reads are idempotent. Retry on a 5xx status. Treat a 4xx status as permanent.

| Limit | Value on emem.dev | Where it comes from |
|---|---|---|
| One cold-band fetch | 14 s | `EMEM_MATERIALIZER_TIMEOUT_SECS`, read by `materializer_timeout_secs` in `crates/emem-api-rest/src/lib.rs` |
| One request | 40 s | `EMEM_TIMEOUT_SECS`, quoted live as `runtime.gateway_timeout_secs` in `GET /v1/agent_card` |

The agent card advises a client read timeout of `gateway_timeout_secs` plus 5 s. Read both values from
`GET /v1/agent_card` at the start of a session. They can change.

## What the agent does after it connects

Every runtime uses the same loop. Each step names the REST route and the MCP tool.

1. Discover. Call `tools/list`, or `emem_tools` in a session, or `GET /v1/agent_card`. Keep the result for the
   session. The card lists the tools, their JSON schemas, and trigger and anti-trigger phrases. `tools/list` on
   `/mcp` returns the core tier only. Send `{"tier":"all"}` or use `/mcp/full` for every tool. `emem_tools` also
   filters by the shape of the answer (`{"shape":"raster"}`) or by the job (`{"bundle":"robotics"}`).
2. Name the object. `POST /v1/entity` (`emem_entity`) mints or returns one identity,
   `emem:entity:<entity_cid>`, so two agents refer to the same object. The body needs a `label` and an
   anchor. The anchor is `place`, `cell`, `lat` and `lng`, or `external_ids.anchor` for an object with no
   place. This is a write to the shared entity space: emem.dev asks for a signed `attester` and refuses an
   unsigned call. The key must also be at tier T3, or the call fails with 403. `GET /v1/enlist` lists the
   tiers and their checks. To find an identity that exists, use `POST /v1/entity/resolve`.
3. Locate. `POST /v1/locate {"place":"Mount Fuji"}` (`emem_locate`) changes a place name, or `lat` and `lng`, into
   a `cell64`. `q`, `query` and `name` also work as the field name. The response says which geocoder answered, so the
   agent can judge the match.
4. Recall. `POST /v1/recall {"cell":"<cell64>","bands":[...]}` (`emem_recall`) returns signed facts at that cell.
   On a miss it fetches the value first. `deterministic: true` keeps only the facts that the cited raw source can
   recompute.
5. Fill a gap. If a value is missing, use `find_similar`, `compare`, `trajectory`, `recall_polygon`, `hunt`, or one
   of the nine domain tools: `emem_ndvi`, `emem_air`, `emem_lst`, `emem_soil`, `emem_water`, `emem_forest`,
   `emem_weather`, `emem_elevation` and `emem_at`.
6. Cite. `POST /v1/memory_token` (`emem_memory_token`) makes `emem:fact:<cell64>:<fact_cid>`. The agent keeps this
   line instead of the payload. The `fact_cid` comes from `receipt.fact_cids` in the recall response. For many facts,
   `POST /v1/memory_bundle` makes one `emem:bundle:` token.
7. Resolve and verify. The receiver calls `POST /v1/memory_token/resolve` (`emem_memory_token_resolve`) and gets
   the same signed bytes. Then it checks the receipt with `POST /v1/verify_receipt` (`emem_verify_receipt`), or in a
   browser at `/verify`. Neither step trusts the sender, and neither needs a key.
8. Find disagreement. `POST /v1/memory_contradictions` (`emem_memory_contradictions`) shows where signed sources
   disagree at the same address. It does not average them.

[`examples/agent-walkthroughs.md`](https://github.com/Vortx-AI/emem/blob/main/examples/agent-walkthroughs.md) shows this loop on real questions, as `curl`
sequences.

## Cite one fact with a memory token

A memory token is one string that names a cell and a fact:

```
emem:fact:<cell64>:<fact_cid>
```

This token names the elevation of a cell in South Mumbai:

```
emem:fact:defi.zb4d7.ze56c.zf24c:4qakixs4xcax3bm2ntlw6ue47bvg4ry5wwa7oog4h6kfp7lvk5ma
```

The old prefixes `memt:`, `memb:` and `meme:` still resolve.

### Make a token

```bash
curl -sX POST https://emem.dev/v1/memory_token \
  -H 'content-type: application/json' \
  -d '{
    "cell":     "defi.zb4d7.ze56c.zf24c",
    "fact_cid": "4qakixs4xcax3bm2ntlw6ue47bvg4ry5wwa7oog4h6kfp7lvk5ma"
  }'
```

Response:

```json
{
  "memory_token": "emem:fact:defi.zb4d7.ze56c.zf24c:4qakixs4xcax3bm2ntlw6ue47bvg4ry5wwa7oog4h6kfp7lvk5ma",
  "cell_token":   "emem:cell:defi.zb4d7.ze56c.zf24c",
  "cell":         "defi.zb4d7.ze56c.zf24c",
  "fact_cid":     "4qakixs4xcax3bm2ntlw6ue47bvg4ry5wwa7oog4h6kfp7lvk5ma",
  "grammar":      "emem:fact:<cell64>:<fact_cid>",
  "docs":         "https://emem.dev/reference#token-fact"
}
```

You can also make a token on the client. The endpoint is a convenience and the reference for the grammar.

### Get the signed fact

The last part of the token is the fact CID:

```bash
curl -sS https://emem.dev/v1/facts/4qakixs4xcax3bm2ntlw6ue47bvg4ry5wwa7oog4h6kfp7lvk5ma
```

This returns the fact as JSON. On 7 November 2026 it was `copdem30m.elevation_mean`, value `10.0`, unit `m`. Send
`Accept: application/cbor` to get the canonical CBOR. The CID checks itself:
`blake3(canonical_cbor(fact)) == base32_decode(fact_cid)`, a 52-character name for the full 32-byte digest. If a
server sends a different fact for the same CID, the digest check fails.

### Parse a token

```
parts    = token.split(":", 3)
assert parts[0] == "emem" and parts[1] == "fact"
cell64   = parts[2]
fact_cid = parts[3]
```

Old `memt:` tokens have three parts (`memt:<cell64>:<fact_cid>`) and still resolve. Neither `cell64` nor `fact_cid`
can contain `:`, and the compose endpoint refuses one that does. The full grammar and the named failures
(`malformed`, `cid_not_found`, drift on re-encode) are in
[the whitepaper, section 3](whitepaper-v2.md#3-the-token-grammar).

### Where to put a token

| Place | What the token does |
|---|---|
| An LLM prompt | Gives the model a handle to name as the source of a number. |
| A tool-call argument | Gives the receiving tool one string to parse and resolve. |
| A message to another agent | Makes both agents read the same signed bytes. |
| A log line or an audit trail | Lets a later debugger get the same bytes again. |
| A long-term memory store | Gives the runtime a stable key that it resolves when it needs the value. |

### Keep a token in your runtime's memory store

Store the token in the long-term store of your runtime (mem0, LangGraph, AutoGen or similar), under the task or the
user intent. In the next session, read the token from the store and resolve it with `GET /v1/facts/<fact_cid>`. You
get the same bytes back, with no embedding lookup and no similarity search.

This sketch uses invented helper names. The REST calls behind them are the ones above.

```python
# Sketch: `client` and `long_term_memory` are your own objects.
user_intent = "track elevation of South Mumbai over time"
token = client.recall_then_memory_token(place="South Mumbai", band="copdem30m.elevation_mean")
long_term_memory.set(user_intent, token)

# Next session: the same signed fact.
token = long_term_memory.get(user_intent)
fact = client.resolve_memory_token(token)  # GET /v1/facts/<fact_cid>
print(fact["value"], fact["unit"])         # 10.0 m
```

The token outlives the conversation that made it. Two agents on two hosts get the same bytes from any emem
responder that holds the fact.

## Cite many facts with a bundle

A bundle collects facts at many places into one signed envelope, named `emem:bundle:<bundle_cid>`. Use it to hand
over all the citations of a finished task as one token.

```bash
curl -sX POST https://emem.dev/v1/memory_bundle \
  -H 'content-type: application/json' \
  -d '{
    "triples": [
      {"cell":"defi.zb4d7.ze56c.zf24c","band":"copdem30m.elevation_mean"},
      {"cell":"defi.zb4d7.ze56c.zf24c","band":"surface_water.recurrence"},
      {"cell":"defi.zb4d7.ze56c.zf24c","band":"hansen.loss_year"}
    ],
    "purpose":"flood-risk site assessment 2026-05"
  }' | jq '.bundle_token'
# "emem:bundle:pzcbww6am7hyzhcicblv3imqei"   (on 7 November 2026)

curl -sS https://emem.dev/v1/memory_bundle/emem:bundle:pzcbww6am7hyzhcicblv3imqei | jq .
```

One call takes at most 256 triples. The responder resolves each triple through recall. Then it hashes the `purpose`
into `bundle_cid`. It also hashes the cell, band, resolved tslot and fact CID of each triple, in the order you sent.
The same facts and the same purpose give the same `bundle_cid` on any responder. A triple resolves to the newest fact, so a newer fact gives a new `bundle_cid`. To bind exact facts, send
`fact_cids` instead of `triples`.

Only the responder that made a bundle can resolve its token. Another responder answers 404. Paste the token at the
responder that made it, or make the bundle again from the same input.

## Read the state of a place as one vector

`POST /v1/state` returns the state of a cell as one fixed-width vector. Each band has a fixed offset. `coverage[]`
says which slots have a measured value, a signed absence, or no value yet.

```bash
curl -sX POST https://emem.dev/v1/state \
  -H 'content-type: application/json' \
  -d '{"cell":"Colaba, Mumbai","view":"cube"}'
```

Response on 7 November 2026, shortened:

```json
{
  "cell":          "defi.zb4d7.jEbO.zf263",
  "view":          "cube",
  "dim":           1792,
  "vector":        [0.0, 0.0, ...],
  "filled_bands":  0,
  "missing_bands": 41,
  "coverage":      [{"key": "geotessera", "offset": 0, "dims": 128, "status": "missing"},
                    {"key": "overture", "offset": 128, "dims": 64, "status": "missing"}, ...],
  "state_cid":     "<52 characters, base32>",
  "memory_token":  "",
  "hint":          "all cube slots are cold at this cell on this responder. Pass `materialize: true` ...",
  "resolved_from": {...},
  "receipt":       {...}
}
```

`state_cid` is the content ID of the vector. To hand the state to another agent, cite the facts behind it with an
`emem:bundle:` token.

Inputs:

- `cell` is a `cell64` or a place name. The geocoder resolves a name, and `resolved_from` says which layer answered.
- `materialize: true` fetches every empty slot. On an empty cell that is about 30 upstream calls. To fill only some
  slots, call `/v1/recall` with a short `bands` list.
- `view: "encoder"` reads one vector band. emem.dev has retired its foundation encoders (Tessera, Clay, Prithvi,
  Galileo). Facts that they signed still read and verify, but emem.dev makes no new ones. A cell with no such fact answers
  404 and says so.

| Use | What the vector does |
|---|---|
| A prefix in the LLM context | Gives the model numbers that describe the place. |
| Change detection | Compare two `/v1/state` reads of one cell at two dates. |
| A query for `/v1/find_similar` | Send one band's vector as `key: "inline:[...]"`. The default band is `geotessera`. emem.dev no longer adds to its index. |
| A cache in agent memory | Store the vector under a task key, and read it back later. |

## Write notes in the agent's memory files

emem has six file verbs that follow the Anthropic memory tool (header `context-management-2025-06-27`):
`emem_memory_view`, `emem_memory_create`, `emem_memory_str_replace`, `emem_memory_insert`, `emem_memory_delete` and
`emem_memory_rename`. Files live under `/memories/`. Each write gets a `file_cid` and an Ed25519 signature. An MCP
host that uses the Anthropic memory tool can point it at emem.

The files are not private on emem.dev. Any caller can read them until owner-scoped reads ship.

```jsonc
// Write (needs a signed attester block on emem.dev, see below)
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
  "name":"emem_memory_create",
  "arguments":{
    "path":"/memories/runbook/2026-05-28.md",
    "file_text":"Mount Fuji elevation 3776 m via Cop-DEM, no surface water in 5 km buffer.",
    "kind":"episodic"
  }
}}

// Search the file contents by meaning (BGE embeddings in a Lance index)
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
  "name":"emem_memory_search",
  "arguments":{"q":"elevation observations at Japanese mountains","k":3}
}}

// Get each write as it happens (server-sent events)
GET /v1/memory/sse?path_prefix=/memories/runbook/&kind=episodic
```

`kind` uses the CoALA types: `episodic` for observations, `semantic` for durable facts, `procedural` for playbooks,
and `resource` (the default) for general notes. `emem_memory_list_by_kind` returns one kind, newest `signed_at`
first.

### Sign each write

Send an `attester` block with every write. A release build refuses an unsigned write to any path with 401
`memory_attestation_required`, and emem.dev runs a release build. The key also gives each agent its own namespace,
`/memories/by_attester/<pubkey>/`.

```jsonc
{"name":"emem_memory_create","arguments":{
  "path":"/memories/by_attester/wosdtqio/note.md",
  "file_text":"...",
  "kind":"episodic",
  "attester":{
    "pubkey_b32":"wosdtqio...",
    "sig_b32":"<ed25519 signature over the v2 digest>"
  }
}}
```

The signature covers the v2 digest:

```
blake3("emem.memory_write.v2|" || verb || "|" || path || "|" || body_hash || "|" || base)
```

`base` is the `file_cid` now at `path`, or the literal `absent` if the path is empty. It stops a replay of an old
signature after the file changes. `body_hash` is the BLAKE3 of the content that the verb makes:

| Verb | `body_hash` covers |
|---|---|
| `create` | `file_text` |
| `str_replace`, `insert` | The whole file after the edit |
| `delete` | The empty string |
| `rename` | The `old_path`. The signed `path` is the `new_path`, so one signature covers both ends. |

emem.dev has refused the v1 digest (no `base`) for every verb since 5 November 2026. Notes signed with v1 before
then still verify when read. The easy way to get the digest right: send the write unsigned first. The 401 response
gives the exact digest in `details.how_to_sign`.

An invalid signature returns 401 `memory_attestation_invalid`. A write outside your namespace returns 403
`memory_namespace_violation`. The reference code is `attester_preimage_v2`, `rename_body_hash` and `verify_attester`
in `crates/emem-primitives/src/memory_acl.rs`. In a multi-agent flow (LangChain, AutoGen, CrewAI), each agent has
its own key and signs its own writes. `memory_contradictions` then shows where two agents disagree on the same
`(cell, band, tslot)`.

### Read the data as it stood on a date

Read primitives such as recall take two time bounds. `as_of_tslot` asks what the world looked like at a date.
`as_of_signed_at` asks what emem knew at a date. When you set a bound, the receipt has an `as_of` block. An auditor
can then repeat an old query and get the same answer.

```bash
curl -sX POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb4d7.ze56c.zf24c","bands":["copdem30m.elevation_mean"],"as_of_signed_at":"2026-05-01T00:00:00Z"}' \
  | jq '.receipt.as_of'
# {"transaction_time":"2026-05-01T00:00:00Z"}
```

This call returns no facts, because emem signed the elevation fact on 16 May 2026, after the bound. Use this bound to
cite evidence as it was on a fixed date. Examples are the submission date of an EUDR due diligence statement and the
bind date of an insurance policy. A third is a log entry under Article 12 of the EU AI Act.

## Check a draft before you publish it

A runtime can ground a claim with emem. A second call tells you when a cited claim no longer holds. It works with any
vendor.

| Mode | What you call | Who is in the request path |
|---|---|---|
| Advise | `POST https://emem.dev/v1/guard/verdict`, or the MCP tool `emem_guard_verdict` (core tier) | Nobody. emem.dev answers what a guard would say over its own data, and blocks nothing. |
| Enforce | Your own `emem-guard` node | You. It makes its own signing key and adds every verdict to a log that anyone can audit offline. |

To get the setup steps for your own node, run `curl -s https://emem.dev/v1/guard/selfhost | jq -r .skill`, or call
the MCP tool `emem_guard_selfhost`.

### Send the body your framework already has

The `?shape=` parameter accepts the payload of your framework as it is.

```bash
# Any HTTP client.
curl -sS -X POST https://emem.dev/v1/guard/verdict \
  -H 'content-type: application/json' \
  -d '{"texts":["Canopy height in Sumatra reached 31 m on 2026-08-05."]}'
```

```python
# Any OpenAI-compatible client. The path is not /v1/moderations, because this
# checks grounding and finds no content-safety category.
import httpx
r = httpx.post("https://emem.dev/v1/guard/verdict?shape=openai",
               json={"input": draft_text}).json()
if r["results"][0]["flagged"]:
    print(r["results"][0]["emem"]["reason"])
```

```python
# LangChain, LangGraph, LlamaIndex, CrewAI, AutoGen: send the message list.
httpx.post("https://emem.dev/v1/guard/verdict",
           json={"messages": [m.dict() for m in state["messages"]]})
```

```javascript
// An MCP proxy that checks a tool call before it runs. On a deny, `result` is
// a CallToolResult with isError set. Return it, and the model reads the denial
// where it reads errors.
const v = await fetch("https://emem.dev/v1/guard/verdict?shape=mcp", {
  method: "POST", headers: {"content-type": "application/json"},
  body: JSON.stringify(jsonRpcRequest),
}).then(r => r.json());
if (!v.allow) return v.result;
```

```bash
# An OPA-compatible policy client, or Envoy external authorization.
curl -sS -X POST 'https://emem.dev/v1/guard/verdict?shape=policy' \
  -H 'content-type: application/json' \
  -d '{"input":{"prompt":"...","tool":"publish"}}'
# -> {"result":{"allow":true,"deny":[],"verdict":{...}},"advisory":true,...}
```

`?shape=cloudevent` accepts CloudEvents 1.0, so a Knative, Dapr or Argo Events mesh can send agent output through
the check with no adapter.

### Read the verdict

The draft in the first example above has no citation, so the verdict is `allow` with `checked: 0`. This draft
cites the South Mumbai elevation token but writes 50 m. The signed value is 10 m:

```bash
curl -sS -X POST https://emem.dev/v1/guard/verdict \
  -H 'content-type: application/json' \
  -d '{"texts":["South Mumbai elevation is 50 m (emem:fact:defi.zb4d7.ze56c.zf24c:4qakixs4xcax3bm2ntlw6ue47bvg4ry5wwa7oog4h6kfp7lvk5ma)."]}' \
  | jq '{action, code, fix, checked, reason}'
```

Response on 7 November 2026:

```json
{
  "action": "deny",
  "code": "PROV_VALUE",
  "fix": "correct_value",
  "checked": 1,
  "reason": "EMEM-GUARD DENY PROV_VALUE token=emem:fact:defi.zb4d7.ze56c.zf24c:4qakixs4xcax3bm2ntlw6ue47bvg4ry5wwa7oog4h6kfp7lvk5ma fix=correct_value leaf=-"
}
```

With 10 m in the draft, the same call returns `allow` with `checked: 1`. Use the `fix` field in your code, not the
prose in `reason`:

| `fix` | What to do |
|---|---|
| `refresh_token` | Resolve the token again and retry. |
| `remove_reference` | Remove the citation. It cannot verify. |
| `contact_admin` | A person restricted the item. The evidence is not the problem. |
| `cite_observation` | Resolve the observation through emem and cite the token that it returns. |
| `correct_value` | Change the number in the draft to the signed value. |
| `redact_and_retry` | Only a self-hosted guard returns this value. Remove the flagged span and retry. |

A citation that the node does not hold is an allow. It looks the same as a citation from another responder, so a
deny would punish an agent that cites across nodes.

Add `?claim_gating=true` to also list measurable claims about a place that have no citation. For each claim, the
verdict names the emem band that would answer it. This rule reports a missing citation, not a failed check. So it is off by default. On your own
node, keep it off until you measure it on your own traffic with `--shadow` and `--report`.

## Where emem fits in an agent's memory

Most agents already have two memory tiers. The working buffer is the context window and any scratchpad. The long-term
store is mem0, Letta, LangGraph state, a vector database or a SQL log. emem adds a third tier for facts about places.

| Tier | Holds | Role of emem |
|---|---|---|
| Working buffer | The current conversation | None. The runtime does this. |
| Long-term store | User preferences, project state, past conversations | Shared, signed notes through the `/memories/` verbs, which another agent or a later run can resolve and verify. Not private on emem.dev: any caller can read them until owner-scoped reads ship. |
| Planetary | What is at a place on Earth | emem: signed, content-addressed facts that all agents share. |

The fact CID connects the tiers. An agent stores `fact_cid` strings in its long-term store. Later it resolves them
with `GET /v1/facts/<fact_cid>`, without the original query.

## Read more

- [`docs/agents.md`](agents.md): the four discovery URLs to fetch at the start, and the REST quick reference.
- [`docs/mcp-directory.md`](mcp-directory.md): the listing of emem in MCP directories.
- [`docs/whitepaper-v2.md`](whitepaper-v2.md): the protocol and the trust model.
