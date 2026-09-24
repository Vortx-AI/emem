# n8n-nodes-emem

n8n community node for [emem](https://emem.dev) -- shared memory for AI agents. One address per fact, one signature you check. No key to read.

## Install

In your n8n instance, go to **Settings > Community Nodes** and search for `n8n-nodes-emem`, then click **Install**.

Or install manually:

```bash
npm install n8n-nodes-emem
```

## What it does

This node calls emem's MCP tools via its public Streamable HTTP endpoint (`emem.dev/mcp/full`). 114 tools available, no API key or account required for reads. Every fact returned carries an ed25519 receipt you can verify offline against the responder's published key.

See the full documentation at [emem.dev/agents.md](https://emem.dev/agents.md) and the [GitHub repository](https://github.com/Vortx-AI/emem).

## Usage

1. Add the **emem** node to your workflow
2. Select a tool from the dropdown (e.g. `emem_locate`, `emem_recall`, `emem_ask`)
3. Set the parameters as JSON (e.g. `{"place": "Bengaluru"}`)
4. Run the workflow

For tools not in the preset dropdown, select "Custom Tool" and type the exact tool name. The full tool list is at [emem.dev/mcp](https://emem.dev/mcp).

## Example

Locate a place and get its canonical cell64 address:

- **Tool**: `emem_locate`
- **Parameters**: `{"place": "Mumbai, India"}`

Recall signed facts at a location:

- **Tool**: `emem_recall`
- **Parameters**: `{"place": "Mumbai, India", "bands": ["copdem30m.elevation_mean"]}`

Ask a plain-language question:

- **Tool**: `emem_ask`
- **Parameters**: `{"q": "Has this place flooded historically?", "place": "Mumbai, India"}`

## Links

- emem: https://emem.dev
- GitHub: https://github.com/Vortx-AI/emem
- Agent handbook: https://emem.dev/agents.md
- Privacy: https://emem.dev/privacy

## License

Apache-2.0
