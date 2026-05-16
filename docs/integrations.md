# emem as long-term memory: integration matrix

emem is a Streamable-HTTP MCP server. Any host that speaks MCP can use it
as a long-term, planet-keyed memory tier without an SDK install, an API
key, or a per-tenant signup. The same handlers also answer plain REST at
`/openapi.json`, so non-MCP runtimes wire in equally well.

This page is a one-page rosetta stone: per runtime, the smallest possible
config that turns emem into the long-term memory layer for an agent that
already exists, plus a pointer to a runnable example in this repository.

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
| Plain REST               | `POST /v1/*`      | none | [`docs/agents.md`](agents.md) — Quick reference |

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

1. **discover** — `GET /v1/agent_card` (or call `tools/list` on the MCP
   transport). Cache the result for the session. The card lists the 50
   read-only tools, their JSON schemas, and trigger / anti-trigger
   phrases for tool selection.
2. **locate** — `POST /v1/locate { "q": "<place>" }` to bridge a place
   name (or lat/lng) to a `cell64`. The response reports which layer of
   the geocoder cascade answered, so the agent can score confidence.
3. **recall** — `POST /v1/recall { "cell": "<cell64>", "bands": [...] }`
   for typed scalar facts at that cell, signed.
4. **reason** — compose `find_similar`, `compare`, `trajectory`,
   `recall_polygon`, `hunt`, or one of the nine domain shortcut tools
   (`emem_ndvi`, `emem_air`, `emem_lst`, `emem_soil`, `emem_water`,
   `emem_forest`, `emem_weather`, `emem_elevation`, `emem_at`).
5. **verify** — surface the `fact_cid` to the user verbatim. Two agents
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
| working          | current conversation, last few turns                 | none — that is the runtime's job                                  |
| long-term store  | user preferences, project state, prior conversations | none — emem is not a personal-memory layer                        |
| **planetary**    | *what is at this place on Earth*                     | **emem** — signed, content-addressed, shared across all agents    |

The split keeps responsibilities clean:

- The runtime's working buffer answers *what was just said*.
- The runtime's long-term store answers *what does this user / project
  care about*.
- emem answers *what is, was, or might be at this place* — once, signed,
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
