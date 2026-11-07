# emem-mcp

`emem-mcp` is the tool catalog of the emem responder. It holds one
descriptor for each MCP tool: the name, the description, the input
schema, an example, and the MCP annotations. It also holds the static
MCP resources, the core loop, and the groups, shapes and bundles that
the `emem_tools` tool uses to sort the catalog.

The crate does not run MCP. The JSON-RPC handlers for `/mcp` and
`/mcp/full` are in `emem-api-rest`. That crate reads this catalog to
build `tools/list`, `emem_tools`, the OpenAPI document, the agent card
and the A2A skills. So one change here changes all of these surfaces.

You need this crate when you add a tool, change a tool description or
schema, or move a tool between tiers. To call the tools, you do not need
it. Connect an MCP client to `https://emem.dev/mcp`.

| | |
|---|---|
| Version | 2.4.4 (workspace version in the root `Cargo.toml`) |
| Used by | `emem-api-rest`. `emem-cli` lists it as a dependency but does not use it. |
| Uses | `serde`, `serde_json`. The tests also use `emem-primitives` and `regex`. |
| Tests | 39 unit tests inline in `src/lib.rs` (all `#[test]`) |
| Source | one file, `src/lib.rs`, 4,994 lines |

## Try it

List the tools that a default MCP connection shows:

```sh
curl -s https://emem.dev/mcp -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' \
  | jq '.result.tools | length, [.[].name]'
```

On 7 November 2026 this gave 18 tools. The first eight are the core loop
in order: `emem_entity`, `emem_locate`, `emem_recall`,
`emem_memory_token`, `emem_memory_token_resolve`, `emem_verify_receipt`,
`emem_memory_contradictions` and `emem_guard_verdict`.

Get one descriptor by name through the `emem_tools` tool:

```sh
curl -s https://emem.dev/mcp -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"emem_tools","arguments":{"name":"emem_recall"}}}' \
  | jq '.result.content[0].text | fromjson | .tool | {name, title, category, tier, annotations}'
```

On 7 November 2026 this gave category `read`, tier `core`, and
`readOnlyHint` false. A recall can fetch and sign a new fact on a miss,
so it is not read-only. Both commands need `curl` and `jq`. Both ran on
that day.

To run the crate tests from a checkout (needs Rust 1.91 or later):

```sh
cargo test -p emem-mcp
```

We did not run `cargo test` for this page, because the host had no Rust
toolchain. The test count comes from the test attributes in the source.

## What the catalog holds

All counts are from `src/lib.rs` on 7 November 2026. The live server
gave the same tool counts on that day.

| Item | Count | What it is |
|---|---|---|
| `TOOLS` | 116 | One `ToolDescriptor` per tool |
| Tier `core` | 18 | Tools that `/mcp` shows in `tools/list` |
| Tier `extended` | 98 | Tools that `/mcp` leaves out of `tools/list` unless the request sets `tier`. `/mcp/full` lists them by default. All tools stay callable by name through `tools/call`. |
| `CORE_LOOP` | 8 | The core loop, in order, with one line on why each step exists |
| `TOOL_GROUPS` | 10 | Groups for the tools outside the loop, by the question they answer |
| `TOOL_SHAPES` | 13 | The shape of each answer, for example `scalar`, `raster` or `token`. Each tool has one shape. |
| `TOOL_BUNDLES` | 9 | Tool sets for a job, for example `verification` or `agriculture`. A tool can be in none or in several. |
| `RESOURCES` | 8 | Static MCP resources, for example `memory://emem/registry/bands` |
| `RESOURCE_TEMPLATES` | 4 | URI templates for a cell, a fact, a bundle and an agent inbox |
| `ASYNC_TASK_TOOLS` | 2 | `emem_eudr_dds` and `emem_hunt`, which accept an MCP task request |

`emem-api-rest` adds more resources and templates. On 7 November 2026,
`resources/list` gave 21 resources and `resources/templates/list` gave
9 templates.

The 116 tools by category: 75 `read`, 20 `introspect`, 10 `verify`,
8 `write` and 3 `plan`. A category is the kind of job, not a safety
level. 51 tools set `readOnlyHint` to false, and most of them are not in
`write`. Read the annotations to know what a tool can change.

## The fields of a tool descriptor

| Field | Use |
|---|---|
| `name` | The wire name, for example `emem_recall`. Do not rename a tool: clients call it by this name. |
| `title` | The display name. It goes out as `annotations.title`. |
| `description`, `when_to_use` | The text an agent reads to choose the tool |
| `input_schema` | A JSON Schema, as a string |
| `output_schema` | A JSON Schema for the result, or `None`. 16 tools declare one. |
| `example_args` | Example arguments that you can paste |
| `level` | `L0` or `L1`. No tool uses `L2`. |
| `category` | `Read`, `Write`, `Verify`, `Introspect` or `Plan` |
| `read_only_hint`, `destructive_hint`, `idempotent_hint`, `open_world_hint` | The four MCP annotations |
| `tier` | `core` or `extended` |

## Rules for a change

- Do not type a tool count into a description. Write `{TOOL_TOTAL}`,
  `{TOOL_CORE}` or `{TOOL_SIDE_EFFECT}`. The function `with_counts`
  puts the numbers in when the server sends the text.
- Give every new tool exactly one shape in `TOOL_SHAPES`. A test fails
  if a tool has none.
- Set `read_only_hint` to false when the tool can write state, also in
  the `read` category. A test checks this.
- An alias property must start its description with
  ``Alias for `<name>` ``. The function `alias_groups` reads that form,
  and a test refuses any other form.
- Declare an `output_schema` only when every successful result can match
  it. For such a tool the server keeps `structuredContent`. It makes the
  payload smaller to fit the wire budget, and does not drop that copy.
- After you add or remove a tool, run `python3 scripts/sync_counts.py
  --check` from the repository root. It finds docs that give the old
  count.

## What this crate does not do

- It does not run MCP or HTTP. See `mcp_jsonrpc` and `mcp_jsonrpc_full`
  in `crates/emem-api-rest/src/lib.rs`.
- It does not run a tool. The code that runs each tool is in
  `emem-api-rest`.
- It does not use the `rmcp` crate. The dependency is commented out in
  `Cargo.toml`.
- `emem-core`, `emem-fact`, `emem-intent`, `emem-claim`, `ciborium`,
  `tokio` and `thiserror` are in `Cargo.toml`, but no code in `src/lib.rs` uses
  them.

## Read more

- [docs/mcp-directory.md](../../docs/mcp-directory.md): the MCP server
  listing, with the tool and resource counts
- [docs/agents.md](../../docs/agents.md): how an agent uses emem
- [integrations/chatgpt/tool-annotations.md](../../integrations/chatgpt/tool-annotations.md):
  the annotations of each tool, in one table
- [https://emem.dev/tools](https://emem.dev/tools): the tool explorer,
  made from this catalog by `scripts/gen_tools_page.py`
