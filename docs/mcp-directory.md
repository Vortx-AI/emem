# emem MCP Server

**Name:** emem
**Publisher:** Vortx.ai
**Repository:** https://github.com/Vortx-AI/emem
**Homepage:** https://emem.dev
**MCP endpoint:** https://emem.dev/mcp
**MCP Registry:** io.github.Vortx-AI/emem
**Container:** ghcr.io/vortx-ai/emem:latest
**Version:** 1.1.0

## Description

emem is a shared, verifiable memory for AI agents: a vendor-neutral, citeable identity layer that stops referential drift, both the paraphrase that drifts from its referent (the token pins it) and the readout that drifts at a pinned reference (the change-attribution ledger reports the per-term evidence; the numeric split is roadmap). Every place resolves to one canonical address (cell64), every observation to one signed fact (fact_cid), and every object to one citeable identity (emem:entity:<entity_cid>, minted by emem_entity), so different models reason from the same world object instead of divergent descriptions. Earth memory and agent memory on one signed trust surface: every read returns an ed25519 receipt, every write is content-addressed, every byte is reproducible on any peer. 94 MCP tools (14 core, 80 extended), plus 18 MCP resources + 8 URI templates (e.g. `memory://emem/cell/<cell64>`, `memory://emem/fact/<cid>`, `memory://emem/bundle/<token>`).

## Key capabilities

**The citeable token loop.** The reason the rest exists. Name a thing with `emem_entity` and it gets one canonical identity every agent resolves the same way. Ground a place with `emem_locate` and it gets one `cell64`. Read it with `emem_recall` and every observation is one signed fact. Cite it with `emem_memory_token` and the fact collapses to one line, `emem:fact:<cell64>:<fact_cid>`, that an agent keeps instead of the payload. `emem_memory_token_resolve` turns that line back into the byte-identical signed body for anyone who holds it, `emem_verify_receipt` checks the ed25519 receipt without trusting the responder, and `emem_memory_contradictions` shows where signed sources disagree at the same address. The claim is narrow: the same token resolves to the same bytes for everyone, and the receipt verifies without trusting the server.

**Earth memory (read).** What populates that memory with facts worth citing, rather than the point of the system. Resolve places, addresses, or latitude/longitude into `cell64` identifiers. Recall signed facts for a cell and band. Compare two cells or two bands. Retrieve time series and signed deltas. Ask natural-language questions about a real-world place. Search for similar places by foundation embedding (Tessera, Clay v1.5, Prithvi-EO-2.0-300M-TL, Galileo).

**Agent memory (write + read).** Six file-op verbs that conform to the Anthropic memory-tool spec (`memory_view`, `memory_create`, `memory_str_replace`, `memory_insert`, `memory_delete`, `memory_rename`). Each file carries a `kind` from the CoALA taxonomy: `episodic`, `semantic`, `procedural`, `resource`. Writes can be capability-bound to an ed25519 attester so paths under `/memories/by_attester/<pubkey>/...` reject any signer that isn't their owner. `memory_list_by_kind` returns the typed slice. `memory_bundle` composes N facts into one signed envelope (`emem:bundle:<bundle_cid>`).

**Search + audit.** `memory_search` runs BGE-base-en-v1.5 embeddings against a LanceDB IVF_PQ partition over memory-file contents, so paraphrases match. `memory_contradictions` walks a parallel multi-attester index and scores disagreement per band kind (scalar, vector, categorical). `memory/sse` opens a Server-Sent Events stream filtered by `path_prefix`, `kind`, `attester`.

**Bi-temporal recall.** Every read primitive accepts `as_of_tslot` (observation time) and `as_of_signed_at` (transaction time). The receipt carries an `as_of` block when set, so an auditor replays a past query byte-for-byte without trusting the issuer.

## MCP transport

Remote HTTP MCP endpoint (Streamable HTTP, JSON-RPC 2.0):

```json
{
  "mcpServers": {
    "emem": {
      "url": "https://emem.dev/mcp"
    }
  }
}
```

Swap the URL for `https://emem.dev/mcp/full` to register the whole catalog instead. Same server, same dispatch; the only difference is how much of it `tools/list` advertises.

`tools/list` at `/mcp` returns the 14 core tools by default (about 39 KB), not the whole catalog. An MCP host loads every advertised descriptor into the model's context at connect, and the full 94 cost about 210 KB of every conversation whether or not it touched Earth observation. Connect to `POST /mcp/full` instead to have `tools/list` advertise all 94. An explicit `{"tier":"core"|"extended"|"all"}` overrides the endpoint default either way, so `/mcp` with `{"tier":"all"}` still returns all 94.

Narrowing discovery removes no capability: `tools/call` ignores tier at both endpoints, so every one of the 94 stays callable by name from `/mcp`. Three things keep the rest reachable. `emem_tools` is itself core and maps the whole surface: no arguments returns the core loop in order plus a bundle and shape menu, `{"q":"ndvi"}` searches, and `{"name":"emem_ndvi"}` returns one tool's input schema and a runnable example (about 2 KB, versus 210 KB for the full list). `emem_ask` and `emem_intent` reach the data tools server-side from a free-text question. And a host that wants everything registered up front uses `/mcp/full`.

Every tool carries its selection vocabulary in MCP-standard `_meta`, as `dev.emem/shape` and `dev.emem/bundles`, so a host can filter the list it already holds without another call. A **shape** is the form of the answer, exactly one per tool: `scalar`, `timeseries`, `raster`, `geometry`, `vector`, `identity`, `token`, `proof`, `plan`, `file`, `catalog`. Shape exists because "which tool do I use" is nearly always a question about the shape of the answer rather than its topic, and `scalar` (one number at one address) and `raster` (a gridded field over an area) answer very different questions. A **bundle** is a view of the same tools by job, and they overlap: `tokenisation`, `verification`, `agent_to_agent`, `long_horizon`, `robotics`, `satellites`, `agriculture`, `forestry`, `climate_risk`. Both are filters on `emem_tools`, and `{"bundle":"robotics"}` also works on `tools/list` itself, which returns just that bundle.

## Example questions

- *Place-anchored*: Has this site flooded recently? What is the elevation here? Is this neighbourhood in a low-lying pocket? Has vegetation changed here? Is this area built-up or agricultural?
- *Audit / point-in-time*: What did our system know about this place last quarter? Show me the signed evidence as of 2024-09-10. Was this plot forest on the EUDR cut-off date?
- *Multi-attester provenance*: Which sourcing agent attested this coordinate? Do the forester, mill, and brand agree on canopy at this cell?
- *Agent memory*: What episodic notes did I write about Mato Grosso last month? Search my procedural playbooks for "flood risk." Show me all observations I signed under my pubkey.

## Tags

ai-agents, mcp, memory-substrate, signed-memory, content-addressed, bi-temporal, capability-binding, ed25519, geospatial, earth-observation, satellite, anthropic-memory-tool, coala-taxonomy, federated-memory, audit-trail, verifiable-receipts

## Listing path to the official MCP registry

emem ships a `server.json` at the repository root following the
`https://static.modelcontextprotocol.io/schemas/2025-12-11/server.schema.json`
schema. It declares the registry name `io.github.Vortx-AI/emem`, an
OCI package pointing at `ghcr.io/vortx-ai/emem:latest`, and the
remote endpoint `https://emem.dev/mcp`.

Two publish paths:

* `.github/workflows/mcp-publish.yml` fires on every `v*` tag push,
  authenticates via GitHub OIDC, installs the `mcp-publisher` CLI,
  and runs `mcp-publisher publish` against the official registry at
  `https://registry.modelcontextprotocol.io`. The canonical path for
  every future release.
* `scripts/mcp-publish.sh` is a manual one-off (GitHub device-flow
  auth) for the first publish and for ad-hoc metadata updates
  between release tags.

The aggregator at `https://github.com/mcp` ingests the official
registry on its own cadence; a new publish surfaces there within
minutes to hours, not seconds.
