# Microsoft 365 Copilot / Copilot Studio package

The files Partner Center takes for "Apps and Agents for M365 and Copilot":

| file | what |
|---|---|
| `manifest.json` | devPreview app manifest with one `agentConnectors` entry pointing at `https://emem.dev/mcp` |
| `mcptools.json` | the core tool list, as served by `tools/list` on `/mcp` |
| `intro.md` | the connector's documentation page |
| `color.png` | 192x192 colour icon (from `web/icon-192.png`) |
| `outline.png` | 32x32 white-on-transparent outline icon |

Zip the five files flat (no folder) to upload.

`mcptools.json` goes stale whenever a core tool's description or schema
changes. Regenerate it from the live server:

```sh
curl -s -X POST https://emem.dev/mcp -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' \
  | jq '{tools: [.result.tools[] | {name, title, description, inputSchema, annotations}]}' \
  > integrations/microsoft-copilot/mcptools.json
```

Steps only the publisher can take: the Partner Center account and business
verification, enrolling in the Microsoft 365 and Copilot program, and
confirming with Microsoft that `"authorization": {"type": "None"}` is accepted
for a server whose reads need no credential. If they require a vault
reference instead, swap in an `AzureKeyVault` block holding a placeholder.
See `docs/registries/microsoft-copilot-studio-semantic-kernel.md`.
