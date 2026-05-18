# emem MCP installation for Cline

emem is a public remote MCP server that gives AI agents signed geospatial facts about real places.

## Remote MCP endpoint

Use the hosted MCP endpoint:

```text
https://emem.dev/mcp
```

## Cline setup

In Cline:

1. Open MCP Servers.
2. Choose Add Server / Remote Server.
3. Use:
   - Server name: `emem`
   - Server URL: `https://emem.dev/mcp`
   - Transport: Streamable HTTP

No API key, signup, or local shell command is required for the hosted endpoint.

## Manual config

If editing the MCP JSON config directly:

```json
{
  "mcpServers": {
    "emem": {
      "url": "https://emem.dev/mcp"
    }
  }
}
```

## When Cline should use emem

Use emem when a user asks about:

- A real-world place
- Coordinates
- Address or site context
- Flood signals
- Elevation
- Surface water
- Vegetation
- Built-up context
- Land use
- Physical-world evidence
- Signed/cite-able geospatial facts

## Test prompts

After adding emem, try:

- Ask emem what signed geospatial facts are available for South Mumbai.
- Use emem to check whether Helsinki Airport has elevation or surface-water signals relevant to flood risk.
- Use emem to answer what evidence supports a place-based answer, and include any fact CIDs or receipts.

## Links

- Homepage: https://emem.dev
- GitHub: https://github.com/Vortx-AI/emem
- MCP endpoint: https://emem.dev/mcp
- MCP Registry: io.github.Vortx-AI/emem
- Smithery: https://smithery.ai/servers/vortxai/emem
- Glama: https://glama.ai/mcp/servers/Vortx-AI/emem
