# emem MCP server

This page is the listing text for MCP directories. It also records how emem gets into each directory.
The counts and sizes come from emem.dev on 7 November 2026.

| field | value |
|---|---|
| Name | emem |
| Publisher | Vortx.ai |
| Repository | https://github.com/Vortx-AI/emem |
| Homepage | https://emem.dev |
| MCP endpoint | https://emem.dev/mcp (Streamable HTTP, JSON-RPC 2.0) |
| MCP Registry name | `io.github.Vortx-AI/emem` |
| Container | `ghcr.io/vortx-ai/emem:latest` |
| Version | 2.4.4 |

## Description

emem gives AI agents a memory that they share and can verify. It gives each thing one identity that every
agent resolves the same way. Two agents that use different words then refer to the same object.

- A place resolves to one canonical address, the `cell64`.
- An observation becomes one signed fact, with a `fact_cid`.
- An object gets one citable identity, `emem:entity:<entity_cid>`, from `emem_entity`.

A token pins what a paraphrase refers to. When the value at a pinned reference changes, the
change-attribution ledger shows the evidence for each term. The numeric split of that change is roadmap,
not shipped.

Earth memory and agent memory use the same signatures. Every read returns an Ed25519 receipt. Every write
is content-addressed.

The server has 116 MCP tools (18 core, 98 extended), 21 MCP resources and 9 URI templates. Examples of the
templates are `memory://emem/cell/{cell64}`, `memory://emem/fact/{fact_cid}` and
`memory://emem/bundle/{bundle_token}`.

## What the tools do

### Cite a fact with a token

This loop is the reason for the rest of the server.

1. `emem_entity` names a thing. The thing gets one canonical identity.
2. `emem_locate` grounds a place. The place gets one `cell64`.
3. `emem_recall` reads the place. Each observation is one signed fact.
4. `emem_memory_token` makes a one-line citation, `emem:fact:<cell64>:<fact_cid>`. An agent keeps this
   line instead of the payload.
5. `emem_memory_token_resolve` turns the line back into the byte-identical signed body.
6. `emem_verify_receipt` checks the Ed25519 receipt without trust in the responder.
7. `emem_memory_contradictions` shows where signed sources disagree at the same address.
8. `emem_guard_verdict` checks a draft before you send it. Each citation must resolve, and each measured
   value must have a citation. The result is `allow` or `deny`, with a machine-readable `fix`.

On emem.dev, step 8 is advisory: it blocks nothing, and the reply has `advisory: true`. To enforce the
verdict, run your own node. emem.dev gives these 8 steps in this order in the `loop` field of `emem_tools`
(7 November 2026).

The claim is narrow. The same token resolves to the same bytes for everyone, and the receipt verifies
without trust in the server.

### Read Earth observation facts

These tools fill the memory with facts to cite.

- Resolve a place name, an address or a latitude and longitude to a `cell64`.
- Recall signed facts for a cell and a band.
- Compare two cells or two bands.
- Get time series and signed deltas.
- Ask a question in natural language about a real place.
- Find similar places by the 128-D Tessera embedding (`geotessera`). emem.dev no longer computes this band. The
  index holds only the vectors that it stored before.

### Write and read agent memory

Six file verbs follow the Anthropic memory-tool spec: `emem_memory_view`, `emem_memory_create`,
`emem_memory_str_replace`, `emem_memory_insert`, `emem_memory_delete` and `emem_memory_rename`.

Each file has a `kind` from the CoALA taxonomy: `episodic`, `semantic`, `procedural` or `resource`.
`emem_memory_list_by_kind` returns the files of one kind.

You can bind a write to an Ed25519 attester. A path under `/memories/by_attester/<pubkey>/` refuses every
signer except its owner. `emem_memory_bundle` composes several facts into one signed envelope,
`emem:bundle:<bundle_cid>`.

### Search and audit memory

- `emem_memory_search` finds notes by meaning, so a paraphrase matches. It uses BGE-base-en-v1.5 embeddings
  in a LanceDB index over the note bodies.
- `emem_memory_contradictions` reads a separate index of facts from many attesters. It scores disagreement
  for each band kind: scalar, vector or categorical.
- `GET /v1/memory/sse` streams each memory write as Server-Sent Events. You can filter by `path_prefix`,
  `kind` and `attester`.

### Read the memory at a past time

Seven read tools accept `as_of_tslot` (observation time) and `as_of_signed_at` (transaction time):
`emem_recall`, `emem_recall_polygon`, `emem_state`, `emem_state_multi`, `emem_query_region`,
`emem_find_similar` and `emem_trajectory`. When you set a bound, the receipt includes an `as_of` block.
An auditor can then replay a past query byte for byte, without trust in the issuer.

`emem_edges_recall` reads the typed links between facts. It accepts only `as_of_tslot`, which filters
edges by their valid time (`crates/emem-primitives/src/edges.rs`). It has no `as_of_signed_at`. The tool
lists on emem.dev showed these eight tools on 7 November 2026.

## Connect a client

Add the remote endpoint to your MCP client:

```json
{
  "mcpServers": {
    "emem": {
      "url": "https://emem.dev/mcp"
    }
  }
}
```

To make sure that the endpoint answers, ask for the tool list. This command needs `curl` and `jq`. On
7 November 2026 it printed `18`.

```sh
curl -s https://emem.dev/mcp -H 'content-type: application/json' \
  -H 'accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | jq '.result.tools | length'
```

To register the whole catalog, use `https://emem.dev/mcp/full` instead. It is the same server with the same
dispatch. Only the size of the `tools/list` answer changes.

| endpoint | `tools/list` returns | size on 7 November 2026 |
|---|---|---|
| `POST /mcp` | the 18 core tools, one page | about 75 KB |
| `POST /mcp/full` | all 116 tools, in several pages | about 346 KB |

An MCP host loads every listed descriptor into the model context when it connects. That is why `/mcp`
lists the core tools in a single page (about 75 KB). With the whole catalog, the full 116 cost about 346 KB
in every conversation. That includes conversations that do not use Earth observation.

An explicit `{"tier":"core"|"extended"|"all"}` parameter overrides the endpoint default. For example, `/mcp`
with `{"tier":"all"}` lists all 116 tools, over several pages.

### Call a tool that the list leaves out

The tier only changes the list. `tools/call` ignores the tier at both endpoints, so you can call each of the
116 tools by name from `/mcp`. Three tools help you find the rest:

- `emem_tools` is a core tool and maps the whole catalog.
  - With no arguments, it returns the core loop in order and a menu of bundles and shapes.
  - `{"q":"ndvi"}` searches the catalog.
  - `{"name":"emem_ndvi"}` returns one input schema and an example that runs (about 8 KB, versus about 346 KB
    for the full list).
- `emem_ask` and `emem_intent` take a question in free text. The server then calls the data tools for you.

This command finds `emem_ndvi`, an extended tool that `/mcp` does not list. It prints the example
arguments. On 7 November 2026 it printed `{"place":"Yellowstone National Park"}`.

```sh
curl -s https://emem.dev/mcp -H 'content-type: application/json' \
  -H 'accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"emem_tools","arguments":{"name":"emem_ndvi"}}}' \
  | jq -c '.result.structuredContent.tool.example_args'
```

### Filter tools by shape and bundle

Each tool has two labels in the MCP-standard `_meta` field: `dev.emem/shape` and `dev.emem/bundles`. A host
can filter the list that it already has, without another call.

A shape is the form of the answer. Each tool has exactly one shape. Most "which tool do I use" questions are
about the form of the answer, not the topic. For example, `scalar` is one number at one address, and
`raster` is a gridded field over an area.

| shapes | bundles |
|---|---|
| `scalar`, `timeseries`, `raster`, `geometry`, `vector`, `prose`, `reading`, `identity`, `token`, `proof`, `plan`, `file`, `catalog` | `tokenisation`, `verification`, `agent_to_agent`, `long_horizon`, `robotics`, `satellites`, `agriculture`, `forestry`, `climate_risk` |

A bundle groups tools by job, and bundles overlap. You can filter `emem_tools` by shape or bundle.
`tools/list` also accepts a bundle: `{"bundle":"robotics"}` returns only the tools in that bundle. On
7 November 2026 this command printed `15`, in one page:

```sh
curl -s https://emem.dev/mcp -H 'content-type: application/json' \
  -H 'accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":3,"method":"tools/list","params":{"bundle":"robotics"}}' \
  | jq '.result.tools | length'
```

## Example questions

| type | examples |
|---|---|
| Place | Did this site flood recently? What is the elevation here? Is this neighbourhood in a low-lying pocket? Did the vegetation change here? Is this area built-up or agricultural? |
| Audit at a point in time | What did our system know about this place last quarter? Show the signed evidence as of 2024-09-10. Was this plot forest on the EUDR cut-off date? |
| Provenance from many attesters | Which sourcing agent attested this coordinate? Do the forester, the mill and the brand agree on canopy at this cell? |
| Agent memory | Which episodic notes did I write about Mato Grosso last month? Search my procedural playbooks for "flood risk". Show all observations that I signed with my public key. |

## Tags

ai-agents, mcp, memory-substrate, signed-memory, content-addressed, bi-temporal, attester-binding, ed25519,
geospatial, earth-observation, satellite, anthropic-memory-tool, coala-taxonomy, federated-memory,
audit-trail, verifiable-receipts

## Publish to the official MCP Registry

The file `server.json` at the repository root uses the schema
`https://static.modelcontextprotocol.io/schemas/2025-12-11/server.schema.json`. It declares:

- the registry name `io.github.Vortx-AI/emem`
- an OCI package, `ghcr.io/vortx-ai/emem:latest`
- the remote endpoint `https://emem.dev/mcp`

There are two ways to publish:

| path | when | authentication |
|---|---|---|
| `.github/workflows/mcp-publish.yml` | each push of a `v*` tag, or a manual `workflow_dispatch` | GitHub OIDC, no token to keep |
| `scripts/mcp-publish.sh` | by hand, for the first publish or a metadata change between tags | GitHub device flow |

The workflow installs the `mcp-publisher` CLI and runs `mcp-publisher publish` against
`https://registry.modelcontextprotocol.io`. Published versions cannot change. The workflow reads the version
from `server.json`, so bump it there before you tag. On 7 November 2026 the registry listed 2.4.4 as the
latest version, published on 18 October 2026.

The registry also publishes the free text in `server.json`. Check that text before you tag. On 7 November
2026 its `x-emem.directoryCompliance.toolListing` note gave older sizes: about 64 KB for `/mcp`, and 7 pages
and about 290 KB for `/mcp/full`. emem.dev measured 75 KiB, 9 pages and 346 KiB on the same day. The tool
counts in the note (18 and 116) were correct.

The directory at `https://github.com/mcp` reads the official registry on its own schedule. A new version
can take minutes or hours to show there.

## Glama build: pin the MCP Python SDK below 2.0

Glama builds a container that runs `emem-server`. The Python `mcp-proxy` connects that container to stdio.
On 2026-08-04 the build started to fail with this error:

```
File "/opt/pybridge/mcp_proxy/proxy_server.py", line 11, in <module>
    from mcp.server.lowlevel.server import request_ctx
ImportError: cannot import name 'request_ctx' from 'mcp.server.lowlevel.server'
```

emem has no part in this failure. The proxy stops at import time, before it contacts the binary. `mcp-proxy`
declares `mcp>=1.17.0` with no upper bound. `mcp` 2.0.0 removed `request_ctx`. A search of both installed
versions found the symbol four times in 1.29.0 and zero times in 2.0.0.

Pin exact versions in the build step. Do not use a range:

```
pip3 install --break-system-packages --target /opt/pybridge mcp-proxy==0.12.0 mcp==1.29.0
```

This command installs those versions, and `mcp_proxy.__main__` imports correctly. The container runs that
module.

Do not write `"mcp<2"`. The build spec puts its steps into one `RUN /bin/sh -c ...` line and removes the
quotes. A bare `mcp<2` is then a shell redirection: `sh` tries to read a file named `2`, and the step fails
with this error:

```
/bin/sh: 1: cannot open 2: No such file
```

That error names neither pip nor mcp. An exact pin has no shell metacharacter, so it survives in a field
that removes quotes. It also makes the build reproducible.

The same log shows a second line. This line is not fatal:

```
Error: unsafe tar path "."
```

`crane export` skips a `.` entry when it extracts the image. The extraction still worked. `&&` joins the
steps, the later pip step ran, and `install -Dm755` found the binary.

The build spec is in the Glama dashboard, not in this repository. `glama.json` holds only the schema
reference and the list of maintainers. The fix goes into the dashboard. This section records it here because
the next person to see the failure will look for it in this repository first.

## Submit to the Claude connector directory

An enterprise integrator audited `/mcp`. The audit found that no tool had `title`, `readOnlyHint` or
`destructiveHint`, which the directory requires. That audit read a deploy older than 1.3.0.

Since 1.3.0 (live on 2026-07-28), each of the 116 tools sends `title` (at the top level and in
`annotations`) and all four hints. The unit test `every_serialized_tool_carries_the_directory_gate_fields`
in `crates/emem-api-rest/src/lib.rs` keeps this wire shape. An auditor must fetch the list again before a
new audit.

Three labels are correct on purpose. Put them in the submission notes:

- `emem_derive` and `emem_entity_link` are append-only writes. They have `destructiveHint: false` because
  they cannot overwrite or remove anything.
- `emem_entity` has `readOnlyHint: false` because it can mint an identity when none exists.

Do not change these labels to satisfy a reviewer. A wrong label is the type of dishonesty that the protocol
exists to prevent.

The operator submits the listing, not the repository. The operator needs a Team or Enterprise Claude.ai
organization with Directory management access.

1. Open https://claude.ai/admin-settings/directory/submissions/new.
2. Enter the server URL `https://emem.dev/mcp` (Streamable HTTP).
3. Enter the support, privacy and terms pages: https://emem.dev/support, https://emem.dev/privacy and
   https://emem.dev/terms. The contact is avijeet@vortx.ai.

The server needs no authentication. An OAuth discovery surface exists for brokers that require one.
Registration always succeeds, the tokens control nothing, and the status is `open_unverified`.

Until the directory approves the listing, an enterprise admin can allow emem by hand. Go to Admin settings,
then Connectors, then Add custom connector, and enter `https://emem.dev/mcp`. Some enterprise users report
"access denied". That is an organization policy that blocks connectors outside the directory, not a fault
in the server.
