# emem MCP Server

**Name:** emem
**Publisher:** Vortx.ai
**Repository:** https://github.com/Vortx-AI/emem
**Homepage:** https://emem.dev
**MCP endpoint:** https://emem.dev/mcp
**MCP Registry:** io.github.Vortx-AI/emem
**Container:** ghcr.io/vortx-ai/emem:latest
**Version:** 2.0.0

## Description

emem is a shared, verifiable memory for AI agents: a vendor-neutral, citeable identity layer that stops referential drift, both the paraphrase that drifts from its referent (the token pins it) and the readout that drifts at a pinned reference (the change-attribution ledger reports the per-term evidence; the numeric split is roadmap). Every place resolves to one canonical address (cell64), every observation to one signed fact (fact_cid), and every object to one citeable identity (emem:entity:<entity_cid>, minted by emem_entity), so different models reason from the same world object instead of divergent descriptions. Earth memory and agent memory on one signed trust surface: every read returns an ed25519 receipt, every write is content-addressed, every byte is reproducible on any peer. 107 MCP tools (16 core, 91 extended), plus 18 MCP resources + 8 URI templates (e.g. `memory://emem/cell/<cell64>`, `memory://emem/fact/<cid>`, `memory://emem/bundle/<token>`).

## Key capabilities

**The citeable token loop.** The reason the rest exists. Name a thing with `emem_entity` and it gets one canonical identity every agent resolves the same way. Ground a place with `emem_locate` and it gets one `cell64`. Read it with `emem_recall` and every observation is one signed fact. Cite it with `emem_memory_token` and the fact collapses to one line, `emem:fact:<cell64>:<fact_cid>`, that an agent keeps instead of the payload. `emem_memory_token_resolve` turns that line back into the byte-identical signed body for anyone who holds it, `emem_verify_receipt` checks the ed25519 receipt without trusting the responder, and `emem_memory_contradictions` shows where signed sources disagree at the same address. The claim is narrow: the same token resolves to the same bytes for everyone, and the receipt verifies without trusting the server.

**Earth memory (read).** What populates that memory with facts worth citing, rather than the point of the system. Resolve places, addresses, or latitude/longitude into `cell64` identifiers. Recall signed facts for a cell and band. Compare two cells or two bands. Retrieve time series and signed deltas. Ask natural-language questions about a real-world place. Search for similar places by foundation embedding (Tessera, Clay v1.5, Prithvi-EO-2.0-300M-TL, Galileo).

**Agent memory (write + read).** Six file-op verbs that conform to the Anthropic memory-tool spec (`emem_memory_view`, `emem_memory_create`, `emem_memory_str_replace`, `emem_memory_insert`, `emem_memory_delete`, `emem_memory_rename`). Each file carries a `kind` from the CoALA taxonomy: `episodic`, `semantic`, `procedural`, `resource`. Writes can be capability-bound to an ed25519 attester so paths under `/memories/by_attester/<pubkey>/...` reject any signer that isn't their owner. `emem_memory_list_by_kind` returns the typed slice. `memory_bundle` composes N facts into one signed envelope (`emem:bundle:<bundle_cid>`).

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

`tools/list` at `/mcp` returns the 15 core tools by default (about 39 KB), not the whole catalog. An MCP host loads every advertised descriptor into the model's context at connect, and the full 105 cost about 210 KB of every conversation whether or not it touched Earth observation. Connect to `POST /mcp/full` instead to have `tools/list` advertise all 102. An explicit `{"tier":"core"|"extended"|"all"}` overrides the endpoint default either way, so `/mcp` with `{"tier":"all"}` still returns all 102.

Narrowing discovery removes no capability: `tools/call` ignores tier at both endpoints, so every one of the 105 stays callable by name from `/mcp`. Three things keep the rest reachable. `emem_tools` is itself core and maps the whole surface: no arguments returns the core loop in order plus a bundle and shape menu, `{"q":"ndvi"}` searches, and `{"name":"emem_ndvi"}` returns one tool's input schema and a runnable example (about 2 KB, versus 210 KB for the full list). `emem_ask` and `emem_intent` reach the data tools server-side from a free-text question. And a host that wants everything registered up front uses `/mcp/full`.

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

## Glama build spec: pin the MCP Python SDK below 2.0

Glama builds a container that runs `emem-server` and bridges it to stdio with
the Python `mcp-proxy`. That build began failing on 2026-08-04 with:

```
File "/opt/pybridge/mcp_proxy/proxy_server.py", line 11, in <module>
    from mcp.server.lowlevel.server import request_ctx
ImportError: cannot import name 'request_ctx' from 'mcp.server.lowlevel.server'
```

Nothing in emem is involved: the proxy dies at import time, before our binary
is contacted. `mcp-proxy` declares `mcp>=1.17.0` with no upper bound, `mcp`
2.0.0 shipped, and `request_ctx` was removed in it. Verified by installing
both and grepping the module: 1.29.0 contains the symbol four times, 2.0.0
does not contain it at all.

The build step needs the version constrained. Use exact pins, not a range:

```
pip3 install --break-system-packages --target /opt/pybridge mcp-proxy==0.12.0 mcp==1.29.0
```

Confirmed to install those versions and to import `mcp_proxy.__main__`, the
module the container actually runs.

**Do not write `"mcp<2"` here.** The obvious fix is a range, and it fails in a
way that looks unrelated. The build spec's steps are concatenated into a
`RUN /bin/sh -c ...` line, quotes are not preserved, and a bare `mcp<2` is then
a shell redirection: `sh` tries to open a file named `2` as stdin and the step
dies with

```
/bin/sh: 1: cannot open 2: No such file
```

which names neither pip nor mcp. An exact pin has no shell metacharacter in it
and survives being pasted into any field that strips quoting. It is also the
more honest thing to run in a build that is meant to be reproducible.

A second line appears in the same log and is NOT fatal:

```
Error: unsafe tar path "."
```

That is `crane export` skipping a `.` entry while extracting the image. The
chain is `&&`-joined and reached the pip step, so `install -Dm755` found the
binary and the extraction did its job.

The build spec lives in the Glama dashboard rather than in `glama.json`, which
carries only the schema reference and the maintainer list, so this is applied
there. Recorded here because the next person to see the failure will otherwise
start by looking for it in this repository, where it is not.

## Claude connector directory: submission playbook (2026-07-28)

An enterprise integrator audited `/mcp` and reported every tool missing `title`,
`readOnlyHint`, and `destructiveHint`, the portal's hard gate. That audit hit the
pre-1.3.0 deploy. Since 1.3.0 (live 2026-07-28) every one of the 105 tools serializes
`title` (top-level and in `annotations`) plus all four hints, and a unit test
(`every_serialized_tool_carries_the_directory_gate_fields`) guards the wire shape so a
serializer regression cannot ship. Auditors should re-fetch before re-auditing.

Three labels are deliberate truth-telling, not omissions, and belong in the submission
notes: `emem_derive` and `emem_entity_link` are append-only writes (`destructiveHint:
false`: they can never overwrite or remove anything), and `emem_entity`
(`readOnlyHint: false`) can mint an identity on a miss. Mislabelling them to satisfy a
reviewer's shorthand would be the exact dishonesty the protocol exists to prevent.

To submit (requires a Team/Enterprise Claude.ai org with Directory management access;
the operator does this, not the repo):

1. https://claude.ai/admin-settings/directory/submissions/new
2. Server URL `https://emem.dev/mcp` (Streamable HTTP). No auth required; the OAuth
   discovery surface exists for brokers that insist (registration always succeeds,
   tokens gate nothing; status `open_unverified`).
3. Support/privacy/terms: https://emem.dev/support · /privacy · /terms. Contact
   avijeet@vortx.ai.
4. Until the listing is approved, enterprise admins can allowlist emem manually:
   Admin settings → Connectors → Add custom connector → `https://emem.dev/mcp`. The
   "access denied" reports from enterprise users are org policy blocking non-directory
   connectors, not a fault in the server.
