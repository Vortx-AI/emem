# emem MCP installation

*For agents and for the developers who connect them. You can paste every command on this page.*

emem is a public remote MCP server. It gives AI agents shared memory that they can verify.
Each place has one permanent address, and each observation is one signed fact.
An agent can check an answer offline. It needs no key, no account and no trust in the operator.

Cline reads this file (`llms-install.md`) to install a server. Other MCP clients can use the same steps.

**What you need:** an MCP client that supports Streamable HTTP, or `curl` to test the endpoint.

**What you get:** the emem tools in your client. Reads need no key.

## Remote MCP endpoint

```text
https://emem.dev/mcp
```

You do not need an API key, an account or a local command. Reads are anonymous.

The server has two endpoints:

| Endpoint | Tools listed | Size of the tool list (7 November 2026) |
|---|---|---|
| `https://emem.dev/mcp` | 18, the core loop | 76,818 bytes, one page |
| `https://emem.dev/mcp/full` | all 116 | 354,716 bytes, 9 pages of 2 to 20 tools |

On `/mcp/full`, send the `nextCursor` value from each page to get the next page.
The server cuts the pages by size, at about 45,000 bytes each. The number of pages can change when tool descriptions change.
Some MCP clients read only the first page. On 7 November 2026 the LangChain and Agno adapters read all 9 pages.
The LlamaIndex, AutoGen, Semantic Kernel and CrewAI adapters read only page 1 (12 tools). See the examples in [`examples/`](examples/).

The two endpoints differ only in the list. You can call any of the 116 by name at either endpoint.
If a tool is not in your list, it is still on the server. Call `emem_tools` to search the other tools or to get the schema of one tool.

To check that the endpoint works, list the core tools:

```bash
curl -sX POST https://emem.dev/mcp \
  -H 'content-type: application/json' \
  -H 'accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | jq '.result.tools | length'
# 18   (6 November 2026)
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

If you use the wrong key, nothing tells you. The file parses, but the server does not load.
If you are an agent that writes this configuration for a user, identify which editor they use first.
In VS Code, the tools show only in the **Agent** mode of Copilot Chat.
Ask mode is the default and shows no tools, so it looks the same as a broken configuration.

You can also install emem in VS Code without a configuration file, because emem is in the GitHub MCP Registry.
Search for `@mcp emem` in the Extensions view, or run this command:

```bash
code --add-mcp '{"name":"emem","type":"http","url":"https://emem.dev/mcp"}'
```

## Client libraries (optional)

The endpoints above do not need a library. If you want typed clients, install one of these:

```bash
pip install ememdev          # Python, version 2.4.4 on PyPI (6 November 2026)
npm i @vortxai/emem          # TypeScript, version 2.4.4 on npm (6 November 2026)
```

The package names are not `emem`, and the reason is intentional.
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
4. `emem_verify_receipt` checks a receipt, also one that a different agent fetched.

## Test prompts

- "Ask emem for the elevation at Bengaluru, and quote the fact CID."
  On 6 November 2026 the value was 915.07 m (`copdem30m.elevation_mean`).
- "Use emem to check if Helsinki Airport has surface-water signals that matter for flood risk."
- "Verify the receipt from the previous answer, and say what the signature proves."

## Verify a receipt

Each receipt has a `preimage_version` field. Read that field and use the rule for that version. Do not assume one version.

| `preimage_version` | Status (6 November 2026) |
|---|---|
| 2 | Current. The signature also covers the inclusion proof. |
| 1 | Emitted until 5 August 2026. Still valid. The signature does not cover the inclusion proof. |
| 0 or absent | No longer emitted. `POST /v1/verify_receipt` still verifies it. |

The byte layout of each version is at [`/v1/verifier_spec`](https://emem.dev/v1/verifier_spec).
The responder makes that document from the same code that signs receipts, so the two cannot differ.

## Links

- Homepage: https://emem.dev
- Agent guide: https://emem.dev/agents.md
- Machine summary: https://emem.dev/llms.txt
- GitHub: https://github.com/Vortx-AI/emem
- MCP endpoint: https://emem.dev/mcp
- GitHub MCP Registry: https://github.com/mcp/Vortx-AI/emem
- Official MCP Registry: `io.github.Vortx-AI/emem`
- Glama: https://glama.ai/mcp/servers/Vortx-AI/emem
- Smithery registry entry: https://registry.smithery.ai/servers/vortxai/emem (the web page `smithery.ai/servers/vortxai/emem` returned 404 on 6 November 2026)
- Container: `ghcr.io/vortx-ai/emem:latest`
