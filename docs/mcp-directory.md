# emem MCP Server

**Name:** emem
**Publisher:** Vortx.ai
**Repository:** https://github.com/Vortx-AI/emem
**Homepage:** https://emem.dev
**MCP endpoint:** https://emem.dev/mcp
**MCP Registry:** io.github.Vortx-AI/emem
**Container:** ghcr.io/vortx-ai/emem:0.0.6
**Version:** 0.0.6

## Description

emem is a cite-able, content-addressed, signed Earth memory MCP server.

It helps AI agents answer place-based questions with verifiable geospatial evidence, including questions about flooding, elevation, surface water, vegetation, built-up areas, weather, and land-use context.

## Key capabilities

- Resolve places, addresses, or latitude/longitude into cell64 identifiers.
- Recall signed facts for a cell and band.
- Ask natural-language questions about a real-world place.
- Compare cells and bands.
- Retrieve time series and signed deltas.
- Return content-addressed receipts that agents can cite.

## MCP transport

Remote HTTP MCP endpoint:

```json
{
  "mcpServers": {
    "emem": {
      "url": "https://emem.dev/mcp"
    }
  }
}
```

## Example questions

- Has this site flooded recently?
- What is the elevation here?
- Is this neighbourhood in a low-lying pocket?
- Has vegetation changed here?
- Is this area built-up or agricultural?
- What evidence supports this place-based answer?

## Tags

geospatial, earth-observation, satellite, memory, receipts, signed-facts, ai-agents, mcp
