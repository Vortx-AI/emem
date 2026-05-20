# emem for ChatGPT

emem brings signed Earth facts into ChatGPT.

Ask questions about real places:

- Has this site flooded?
- Is this location low-lying?
- What is the elevation here?
- Has surface water or vegetation changed?
- What evidence supports this answer?

emem returns signed geospatial facts, caveats, and cite-able receipts.

No key. No signup. Public endpoint:

```
https://emem.dev/mcp
```

## How it works

The emem ChatGPT app connects to emem's public MCP endpoint. When a user asks a place-based question, ChatGPT calls emem tools to resolve the place, recall geospatial data, and return signed facts with content-addressed CIDs.

## Tools

The app exposes 4 tools:

| Tool | What it does |
|------|-------------|
| `emem_ask_place` | All-in-one: takes a place and question, returns signed facts |
| `emem_locate_place` | Resolves a place name to cell64, coordinates, and available bands |
| `emem_recall_facts` | Recalls signed facts for a cell and specific bands |
| `emem_get_receipt` | Returns a verifiable receipt for a specific fact CID |

## Example prompts

- Has this site flooded?
- What signed geospatial facts are available for South Mumbai?
- Is Helsinki Airport low-lying?
- What evidence supports this place-based answer?
- Find surface-water or algal-bloom related signals around Lake Erie.

## Setup

### For users

Once published to the ChatGPT app directory:

1. Go to Settings > Apps
2. Find "emem"
3. Enable it

Then ask:

> @emem has this site flooded?

### For developers testing locally

Use the MCP endpoint directly:

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

## Links

- Homepage: https://emem.dev
- Repository: https://github.com/Vortx-AI/emem
- MCP endpoint: https://emem.dev/mcp
- Privacy: [privacy.md](privacy.md)
- Support: jaya@vortx.ai
