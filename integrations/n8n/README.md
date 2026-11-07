# n8n-nodes-emem

An n8n community node that calls one [emem](https://emem.dev) MCP tool per
input item. Use it when an n8n workflow must read signed Earth-observation
facts. Examples are the elevation or the surface-water record at a place. Reads
need no key and no account.

Nobody has published the package yet. On 7 November 2026, the npm registry
returns 404 for `n8n-nodes-emem`, so the **Settings > Community Nodes** search does
not find it. You must build it from this folder. The steps to publish are in
[Status](#status).

## Try the request first

The node sends one JSON-RPC `tools/call` request to
`https://emem.dev/mcp/full`. This `curl` command sends the same request with
the same header. It ran on 7 November 2026 and returned the Mumbai cell
`defi.zb4d8.zcdac.zf45b`.

```bash
curl -s -X POST https://emem.dev/mcp/full \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call",
       "params":{"name":"emem_locate","arguments":{"place":"Mumbai, India"}}}'
```

## Build and run the node locally

These commands use the scripts in `package.json`. We did not run them,
because the host has no Node.js. `npm run dev` comes from `@n8n/node-cli`. It
starts a local n8n with this node loaded.

```bash
cd integrations/n8n
npm install
npm run build
npm run dev
```

## Use the node

1. Add the **emem** node to your workflow.
2. In **Tool**, select a tool, for example `emem_locate`.
3. In **Parameters (JSON)**, type the tool arguments as a JSON object.
4. Run the workflow.

The **Tool** list has nine tools: `emem_locate`, `emem_recall`, `emem_ask`,
`emem_verify_receipt`, `emem_memory_token`, `emem_entity`,
`emem_guard_verdict`, `emem_find_similar` and `emem_memory_contradictions`.
For a different tool, select **Custom Tool** and type its exact name.
`tools/list` on `/mcp/full` returned 116 tools on 7 November 2026, in 9 pages
of different sizes. Follow `nextCursor` to get the next page. The schemas are at [emem.dev/mcp](https://emem.dev/mcp).

The node reads the first text block of the MCP result and parses it as JSON.
If the text is not JSON, the output has one field, `raw_text`. Read facts
carry an Ed25519 receipt. You can verify it offline against the responder's
published key. The server cuts a large result to fit a size budget. The
`_emem_truncation` field then names the omitted fields and the REST route
that returns them in full.

## Examples

Each pair below ran on 7 November 2026 with the node's exact request.

| Tool | Parameters | Result on 7 November 2026 |
|---|---|---|
| `emem_locate` | `{"place": "Mumbai, India"}` | cell64 `defi.zb4d8.zcdac.zf45b` |
| `emem_recall` | `{"place": "Mumbai, India", "bands": ["copdem30m.elevation_mean"]}` | one signed elevation fact with its `memory_token` |
| `emem_ask` | `{"q": "Has this place flooded historically?", "place": "Mumbai, India"}` | surface-water recurrence 0 percent, flood history class 0 |

## Status

The listing plan is in
[docs/registries/integration-targets.md](../../docs/registries/integration-targets.md).
On 7 November 2026 it gives this node the status "not started".
n8n sets two conditions for a verified node:

- A GitHub Action must publish it with a provenance statement. n8n refuses a
  node published from a laptop.
- The package licence must be MIT. `package.json` says Apache-2.0 now.

The `repository` URL in `package.json`,
`https://github.com/Vortx-AI/n8n-nodes-emem`, returns 404 on
7 November 2026.

## Read more

- Agent guide: [emem.dev/agents.md](https://emem.dev/agents.md)
- Source: [github.com/Vortx-AI/emem](https://github.com/Vortx-AI/emem)
- Privacy: [emem.dev/privacy](https://emem.dev/privacy)

## License

Apache-2.0, as `package.json` says. See [Status](#status) for the MIT
condition.
