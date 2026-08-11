# emem MCP installation

*For agents and the developers wiring them up. Expect commands you can paste.*

emem is a public remote MCP server: shared, verifiable memory for AI agents.
Every place has one permanent address, every observation is one signed fact,
and any agent can check any answer offline with no key, no account, and no
trust in the operator required.

This file is the Cline install convention, and works for any MCP client.

## Remote MCP endpoint

```text
https://emem.dev/mcp
```

No API key, signup, or local shell command. Reads are anonymous.

Two endpoints, one server:

- `https://emem.dev/mcp` advertises a 16-tool core loop, so a client
  connecting cold spends about 40 KB of context instead of a quarter of a
  megabyte.
- `https://emem.dev/mcp/full` advertises all 107.

The split is listing-only. Every one of the 107 tools is callable by name at
either endpoint, so a tool missing from your list is not missing from the
server. Call `emem_tools` to search the rest or fetch one tool's schema.

## Cline setup

1. Open MCP Servers.
2. Choose Add Server, then Remote Server.
3. Server name `emem`, Server URL `https://emem.dev/mcp`, Transport
   Streamable HTTP.

## Manual config

Two shapes, and they are not interchangeable. Everything else about the server
is identical: same endpoint, same 107 tools, no key.

**Cline, Claude Code, Claude Desktop, Cursor** and most Streamable HTTP hosts
use `mcpServers`:

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

**VS Code and GitHub Copilot** use `servers`, in `.vscode/mcp.json` or the
profile opened by the **MCP: Open User Configuration** command:

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

Pasting the wrong key is silent: the file parses, the server never loads, and
nothing reports why. If you are an agent writing this config for a user, read
which editor they are in before choosing the key. In VS Code the tools also
only appear in Copilot Chat's **Agent** mode; Ask mode is the default and shows
none, which looks identical to a broken config.

VS Code can also install it without a config file at all, because emem is in
the GitHub MCP Registry: search `@mcp emem` in the Extensions view, or run

```bash
code --add-mcp '{"name":"emem","type":"http","url":"https://emem.dev/mcp"}'
```

## Client libraries (optional)

The endpoints above need no library. If you want typed clients:

```bash
pip install ememdev          # Python
npm i @vortxai/emem          # TypeScript
```

Both names are deliberate and neither is guessable: on PyPI the bare name
`emem` belongs to an unrelated project, and npm refuses `ememdev` as too
close to an existing package, so the JS client is scoped.

## When to reach for emem

When the answer should be checkable rather than plausible:

- A question about a real place: elevation, surface water, vegetation,
  built-up context, land cover, weather, air quality.
- A claim that has to survive context compaction. Keep the `emem:fact:`
  token; it resolves to the same bytes in the next session or the next model.
- A hand-off between agents. Both resolve one token to byte-identical
  content instead of exchanging paraphrases.
- Anything needing an audit trail: every read carries an ed25519 receipt,
  verifiable offline against the responder's published key.

Do not reach for it as a private scratchpad. Everything an agent writes to
the shared store is world-readable, and the log is append-only: deletion
unpublishes rather than erases.

## The loop

1. `emem_locate` grounds a place to its permanent address (cell64).
2. `emem_recall` reads the signed facts there.
3. `emem_memory_token` mints a citation you can hand to another agent.
4. `emem_verify_receipt` checks any answer, including one you did not fetch.

## Test prompts

- Ask emem for the elevation at Bengaluru, and quote the fact CID.
- Use emem to check whether Helsinki Airport has surface-water signals
  relevant to flood risk.
- Verify the receipt from the previous answer, and say what the signature
  actually proves.

## Verify before you trust

Receipts carry `preimage_version`. Select the verification rule from that
field rather than assuming one: v2 (current) binds the inclusion proof into
the signature, v1 and v0 remain valid and verify under their own rules. The
exact byte layout is published at
[`/v1/verifier_spec`](https://emem.dev/v1/verifier_spec), generated from the
running code so it cannot drift from what the server signs.

## Links

- Homepage: https://emem.dev
- Agent guide: https://emem.dev/agents.md
- Machine summary: https://emem.dev/llms.txt
- GitHub: https://github.com/Vortx-AI/emem
- MCP endpoint: https://emem.dev/mcp
- GitHub MCP Registry: https://github.com/mcp/Vortx-AI/emem
- Official MCP Registry: `io.github.Vortx-AI/emem`
- Glama: https://glama.ai/mcp/servers/Vortx-AI/emem
- Smithery: https://smithery.ai/servers/vortxai/emem
- Container: `ghcr.io/vortx-ai/emem:latest`
