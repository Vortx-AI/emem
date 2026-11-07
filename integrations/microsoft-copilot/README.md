# Microsoft 365 Copilot and Copilot Studio package

This folder holds the app package for the Partner Center program "Apps and
Agents for Microsoft 365 and Copilot". The package connects Copilot to the
emem MCP server at `https://emem.dev/mcp`. Only the publisher needs this
folder. The listing plan is in
[docs/registries/microsoft-copilot-studio-semantic-kernel.md](../../docs/registries/microsoft-copilot-studio-semantic-kernel.md).
A Semantic Kernel example is in `examples/semantic-kernel/`.

Microsoft does not list the package yet. The publisher steps in
[Steps only the publisher can do](#steps-only-the-publisher-can-do) are still
open.

## Files in the package

| File | Content |
|---|---|
| `manifest.json` | App manifest, schema `devPreview`, version 2.4.4. It has one `agentConnectors` entry for `https://emem.dev/mcp`, with authorization `None`. |
| `mcptools.json` | The core tool list that `tools/list` on `/mcp` returns: 18 tools. |
| `intro.md` | The documentation page of the connector. |
| `color.png` | Colour icon, 192 x 192 pixels, a copy of `web/icon-192.png`. |
| `outline.png` | Outline icon, 32 x 32 pixels, white on a transparent background. |

## Make the upload file

Put the five files at the top level of one zip file, with no folder. We did
not run this command, because the host has no `zip`.

```sh
cd integrations/microsoft-copilot
zip ../emem-copilot.zip manifest.json mcptools.json intro.md color.png outline.png
```

## Update mcptools.json before each upload

`mcptools.json` becomes old when the description or schema of a core tool
changes. This command writes a new copy from the live server. It ran on
7 November 2026.

```sh
curl -s -X POST https://emem.dev/mcp -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' \
  | jq '{tools: [.result.tools[] | {name, title, description, inputSchema, annotations}]}' \
  > integrations/microsoft-copilot/mcptools.json
```

On 7 November 2026, the committed copy (last changed on 29 October 2026) has
the same 18 tool names as the live server. Four tools differ:
`emem_locate`, `emem_ask` and `emem_memory_contradictions` in the description,
and `emem_memory_contradictions` and `emem_intent` in the input schema.

## Steps only the publisher can do

- Open a Partner Center account and complete the business verification.
- Enrol in the Microsoft 365 and Copilot program.
- Ask Microsoft to confirm that it accepts `"authorization": {"type": "None"}`.
  Reads from emem need no credential. If Microsoft requires a vault
  reference, put an `AzureKeyVault` block with a placeholder value in its
  place.

## Read more

- [docs/registries/microsoft-copilot-studio-semantic-kernel.md](../../docs/registries/microsoft-copilot-studio-semantic-kernel.md):
  the two listing paths, the Semantic Kernel example and the listing status.
- [intro.md](intro.md): the page that users of the connector see.
