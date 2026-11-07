# emem MCP installation

*For agents and for the developers who connect them. You can paste every command on this page.*

emem is a public remote MCP server. It gives AI agents shared memory that they can verify.
Each place has one permanent address, and each observation is one signed fact.
An agent can check an answer offline. It needs no key, no account and no trust in the operator.

Cline reads this file (`llms-install.md`) to install a server. Other MCP clients can use the same steps.

## What you need

- An MCP client that supports Streamable HTTP.
- `curl` and `jq` to test the endpoint. Both are optional.

## What you get

- The emem tools in your client. Reads need no key.
- A command that tests the endpoint, three test prompts and the rules to verify a receipt.

## Remote MCP endpoint

```text
https://emem.dev/mcp
```

You do not need an API key, an account or a local command. Reads are anonymous.

The server has two endpoints:

| Endpoint | Tools listed | Size of the tool list (7 November 2026) |
|---|---|---|
| `https://emem.dev/mcp` | 18, the core loop | about 76,800 bytes, one page |
| `https://emem.dev/mcp/full` | all 116 | about 354,000 bytes, 8 or 9 pages of 2 to 22 tools |

On `/mcp/full`, send the `nextCursor` value from each page to get the next page.
The server cuts the pages by size, at about 45,000 bytes each.
The number of pages changes when tool descriptions change. It was 8 or 9 on 7 November 2026.
Some MCP clients read only the first page.
On 7 November 2026 the LangChain and Agno adapters read every page.
The LlamaIndex, AutoGen, Semantic Kernel and CrewAI adapters read only page 1 (12 or 13 tools). See the examples in [`examples/`](examples/).
`@mastra/mcp` 0.10.12 also reads only page 1. On 7 November 2026 it loaded 12 tools from `/mcp/full`.

The two endpoints differ only in the list. You can call any of the 116 by name at either endpoint.
If a tool is not in your list, it is still on the server. Call `emem_tools` to search the other tools or to get the schema of one tool.

To check that the endpoint works, list the core tools:

```bash
curl -sX POST https://emem.dev/mcp \
  -H 'content-type: application/json' \
  -H 'accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | jq '.result.tools | length'
# 18   (7 November 2026)
```

## Cline setup

1. Open MCP Servers.
2. Select Add Server, then Remote Server.
3. Set Server name to `emem` and Server URL to `https://emem.dev/mcp`.
4. Set Transport to Streamable HTTP.

## Manual configuration

There are two configuration shapes. You cannot use one in place of the other.
In all other ways the server is the same: the same endpoint, the same 116 tools, and no key for reads.

Cline, Claude Code, Claude Desktop, Cursor and most Streamable HTTP hosts use `mcpServers`:

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

VS Code and GitHub Copilot use `servers`. Put it in `.vscode/mcp.json`, or in the profile that the **MCP: Open User Configuration** command opens:

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

If you use the wrong top-level name, nothing tells you. The file parses, but the server does not load.
If you are an agent that writes this configuration for a user, identify which editor they use first.
In VS Code, the tools show only in the **Agent** mode of Copilot Chat.
Ask mode is the default and shows no tools, so it looks the same as a broken configuration.

You can also install emem in VS Code without a configuration file, because emem is in the GitHub MCP Registry.
Search for `@mcp emem` in the Extensions view, or run this command:

```bash
code --add-mcp '{"name":"emem","type":"http","url":"https://emem.dev/mcp"}'
```

## ChatGPT custom GPT

A custom GPT does not use the MCP endpoint. It calls HTTP operations from an OpenAPI schema.

1. In the GPT builder, open Actions and select Import from URL.
2. Paste `https://emem.dev/openapi.action.json`.
3. Set Authentication to None and the privacy policy URL to `https://emem.dev/privacy`.
4. Copy the `custom_gpt_instructions` text from [`examples/openai-gpt-action.json`](examples/openai-gpt-action.json) into the GPT instructions.

On 7 November 2026 the action schema had 39 operations on 38 paths (OpenAPI 3.1.0).
Do not import `https://emem.dev/openapi.json`. It had 213 operations and 405,695 bytes, which is too large for a GPT.
The action schema includes one write, `emem_attest` (`POST /v1/attest`). It needs a signature.

The instructions text names some tools that are not in the action schema.
On 7 November 2026 these were `emem_ask`, `emem_entity`, `emem_change_attribution`, `emem_derive`, `emem_band_raster`, `emem_band_cube`, `emem_cells_in_bbox`, `emem_raster_resolve` and `emem_cube_resolve`.
The GPT cannot call them. Use the MCP endpoint if you need them.
We did not test these steps in the GPT builder. We checked the URLs and counts live.

## Client libraries (optional)

The endpoints above do not need a library. If you want typed clients, install one of these:

```bash
pip install ememdev          # Python, version 2.4.4 on PyPI (7 November 2026)
npm i @vortxai/emem          # TypeScript, version 2.4.4 on npm (7 November 2026)
```

The package names are not `emem`. There are two reasons.
On PyPI, the name `emem` belongs to a different project.
npm does not accept `ememdev` because it is too near the name of an existing package, so the JavaScript client has a scope.

## When to use emem

Use emem when an answer must be checkable, not only plausible:

- A question about a real place, for example elevation, surface water, vegetation, built-up land, land cover, weather or air quality.
- A claim that must stay correct after context compaction. Keep the `emem:fact:` token. It resolves to the same bytes in the next session or in a different model.
- A handoff between agents. Each agent resolves one token to the same bytes, so they do not exchange paraphrases.
- Work that needs an audit trail. Each read has an Ed25519 receipt that you can verify offline against the published key of the responder.

Do not use emem as a private scratchpad. All data that an agent writes to the shared store is public.
The log is append-only. A deletion unpublishes the data. It does not erase it.

## The loop

1. `emem_locate` changes a place name into its permanent address (cell64).
2. `emem_recall` reads the signed facts at that address.
3. `emem_memory_token` makes a citation token that you can give to another agent.
4. `emem_verify_receipt` checks a receipt, including one that a different agent fetched.

## Test prompts

- "Ask emem for the elevation at Bengaluru, and quote the fact CID."
  On 7 November 2026 the value was 915.07 m (`copdem30m.elevation_mean`).
- "Use emem to check if Helsinki Airport has surface-water signals that matter for flood risk."
- "Verify the receipt from the previous answer, and say what the signature proves."

## Verify a receipt

Each receipt has a `preimage_version` field. Read that field and use the rule for that version. Do not assume one version.

| `preimage_version` | Status (7 November 2026) |
|---|---|
| 2 | Current. The signature also covers the inclusion proof. |
| 1 | Emitted until 5 August 2026. Still valid. The signature does not cover the inclusion proof. |
| 0 or absent | No longer emitted. `POST /v1/verify_receipt` still verifies it. |

The byte layout of each version is at [`/v1/verifier_spec`](https://emem.dev/v1/verifier_spec).
The responder makes the receipt layouts in that document from the same constants that sign receipts.
The prose notes in it are hand-written strings in the code, and they can be out of date.
On 7 November 2026 the `memory_write` note still said that the responder accepts the v1 digest for `create`.
The code on `main` refuses it for every verb since commit `23fa7fd6`. See [the quickstart](docs/quickstart.md).
The spec also lists two constructions that do not use the segment rule: the legacy v0 receipt (verify only) and
caller-signed memory writes.

## Links

- Homepage: https://emem.dev
- Agent guide: https://emem.dev/agents.md
- Machine summary: https://emem.dev/llms.txt
- GitHub: https://github.com/Vortx-AI/emem
- MCP endpoint: https://emem.dev/mcp
- GitHub MCP Registry: https://github.com/mcp/Vortx-AI/emem
- Official MCP Registry: `io.github.Vortx-AI/emem`, latest version 2.4.4 on 7 November 2026
  (https://registry.modelcontextprotocol.io/v0/servers?search=io.github.Vortx-AI/emem&version=latest)
- Glama: https://glama.ai/mcp/servers/Vortx-AI/emem
- Smithery registry entry: https://registry.smithery.ai/servers/vortxai/emem (the web page `smithery.ai/servers/vortxai/emem` returned 404 on 7 November 2026)
- Container: `ghcr.io/vortx-ai/emem:latest`
