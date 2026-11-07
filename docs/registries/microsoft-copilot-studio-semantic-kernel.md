# Microsoft Copilot and Semantic Kernel listings

This page records how emem can reach three Microsoft surfaces, and the state of
each path. We checked all states, counts and Microsoft requirements on
7 November 2026. We did not check steps that need a Microsoft or Partner
Center login. The page says so where it applies.

| Path | What ships in this repo | State on 7 November 2026 |
|---|---|---|
| Microsoft 365 Copilot federated connector | The package in [`integrations/microsoft-copilot/`](../../integrations/microsoft-copilot/) | No listing found. Submission state needs a Partner Center login; not checked. |
| Copilot Studio MCP certification | The same package | No listing found. The package uses `"authorization": {"type": "None"}`. Microsoft now asks for an Azure Key Vault block. |
| Semantic Kernel example | [`examples/semantic-kernel/`](../../examples/semantic-kernel/) | Upstream PR [microsoft/semantic-kernel#14228](https://github.com/microsoft/semantic-kernel/pull/14228) closed without merge on 28 September 2026. |

## What the package holds

[`integrations/microsoft-copilot/README.md`](../../integrations/microsoft-copilot/README.md)
lists the five files. Zip them flat, with no folder, to upload.

| File | Content | Check on 7 November 2026 |
|---|---|---|
| `manifest.json` | `devPreview` app manifest, version 2.4.4, one `agentConnectors` entry for `https://emem.dev/mcp` | ASCII only |
| `mcptools.json` | The 18 core tools from `tools/list` on `/mcp` | 18 tools. Four differ from the live list (see below). 8 lines hold non-ASCII characters. |
| `intro.md` | The connector documentation page | Its tool counts match the live list |
| `color.png` | Colour icon | 192 x 192 PNG |
| `outline.png` | White-on-transparent outline icon | 32 x 32 PNG |

On 7 November 2026, the live `/mcp` list had 18 tools on one page. Each tool
has a `title` and the annotations `readOnlyHint`, `destructiveHint`,
`idempotentHint` and `openWorldHint`.

| `readOnlyHint` | Count | Tools |
|---|---|---|
| `true` | 9 | `emem_locate`, `emem_memory_token`, `emem_memory_token_resolve`, `emem_verify_receipt`, `emem_memory_contradictions`, `emem_guard_verdict`, `emem_tools`, `emem_echo_verify`, `emem_entity_resolve` |
| `false` | 9 | `emem_recall`, `emem_ask`, `emem_intent`, `emem_find_similar`, `emem_memory_bundle`, `search`, `fetch`, `emem_entity`, `emem_entity_link` |

The first seven tools in the `false` row can measure and sign a new fact
when the reading does not exist yet. `emem_entity` and `emem_entity_link`
write shared identities and need a signed attester block. `intro.md` gives
the same split.

Count the read-only tools yourself:

```sh
curl -s -X POST https://emem.dev/mcp \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' \
  | jq '[.result.tools[] | select(.annotations.readOnlyHint == true) | .name]'
```

On 7 November 2026 this returned the 9 names in the table.

### Make mcptools.json current before you upload

On 7 November 2026, the repo `mcptools.json` differed from the live list for
four tools:

| Tool | Field that differs |
|---|---|
| `emem_locate` | `description` |
| `emem_ask` | `description` |
| `emem_intent` | `inputSchema` |
| `emem_memory_contradictions` | `description`, `inputSchema` |

Run the regeneration command in the package README before each upload. Then
look for non-ASCII characters, because Microsoft warns that they can fail
validation:

```sh
grep -nP '[^\x00-\x7F]' integrations/microsoft-copilot/mcptools.json
```

## Federated connector for Microsoft 365 Copilot

A federated connector puts a remote MCP server in the Connectors Gallery of
Microsoft 365 Copilot tenants. Each tenant admin must then approve it.

Microsoft's page
[Submit a federated Microsoft 365 Copilot connector](https://learn.microsoft.com/en-us/microsoft-365/copilot/connectors/submit-federated-connector)
had its last update on 16 October 2026. It gives these rules for an independent
software vendor:

- Submit a plugin package in Partner Center. You need the developer or
  manager role.
- Every tool needs a human-readable title and the `readOnlyHint` annotation.
- Give OAuth 2.0 credentials "for any authenticated scenario", and a test
  account for reviewers.
- Assets: a description of 80 characters or fewer, up to 10 synonyms, a
  192 x 192 colour logo and a 32 x 32 outline logo.
- Give at least three sample prompts with expected responses.

The emem package meets the title, annotation and logo rules. The manifest
short description has 70 characters.

The old plan used the form link `https://aka.ms/FccSubmissionForm` and the
address `submit-fcc@microsoft.com`. On 7 November 2026, the link still
redirected to a Microsoft Forms page. The current Microsoft page names
neither the form nor the address.

### Open question: no OAuth for reads

emem reads need no credential. The Microsoft page asks for OAuth only for
authenticated scenarios, but its metadata table has an "OAuth credentials"
row. Ask Microsoft if they accept an anonymous read server before you submit.

### Sample prompts

The old plan listed prompts that used the bands `aqi` and `flood_extent`.
Neither band exists. On 7 November 2026, `GET /v1/bands` listed 43 bands.
Air quality comes from `cams.*` bands such as `cams.pm25`. Water comes from
`surface_water.*` bands. Use the three prompts in `intro.md`. They ask about
vegetation, forest loss after 2020, and elevation.

Reviewers need no login. Tell them to add `https://emem.dev/mcp` as the
connector URL and to check a receipt at `https://emem.dev/verify`.

## Copilot Studio MCP certification

A certified MCP server appears in Copilot Studio and Azure Foundry.
Microsoft's page
[Microsoft MCP server certification (preview)](https://learn.microsoft.com/en-us/microsoft-copilot-studio/mcp-certification)
had its last update on 2 November 2026. It gives these rules:

- The publisher must have a Partner Center account with business
  verification. It must enrol in the Microsoft 365 and Copilot program.
- New submissions start with an intake form. Microsoft then sends
  instructions. The legacy process stays open through 18 November 2026.
- The offer type in Partner Center is "Apps and Agents for M365 and Copilot".
- Each package needs a manifest, a tool file, `intro.md`, and an Azure Key
  Vault authentication block.
- Manifest and tool files must use ASCII header names and values.

The emem package differs from this in two places:

| Item | emem package | Microsoft example |
|---|---|---|
| `authorization` | `{"type": "None"}` | `{"type": "AzureKeyVault", "referenceId": "<vault URI>"}` |
| Manifest schema | `devPreview` | `v1.30` |

The package README says what to do if Microsoft refuses `None`: use an
`AzureKeyVault` block that holds a placeholder. The Microsoft FAQ lists the
secrets for OAuth2 and Azure AD providers only. It names no secret set for
a server without auth.

These steps need the publisher's accounts. Nobody checked them on
7 November 2026:

1. Make a Partner Center account and complete business verification.
2. Enrol in the Microsoft 365 and Copilot program.
3. Send the intake form.
4. If Microsoft asks for Key Vault, make an Azure subscription and a vault.

## Semantic Kernel example

[`examples/semantic-kernel/emem_mcp_geospatial_agent.py`](../../examples/semantic-kernel/emem_mcp_geospatial_agent.py)
connects a Semantic Kernel kernel to `https://emem.dev/mcp` through
`MCPStreamableHttpPlugin`. It asks the model to locate South Mumbai, recall
`copdem30m.elevation_mean`, and verify the receipt.

On 7 November 2026, Semantic Kernel 1.47.0 was the newest release on PyPI
(5 November 2026). Its `python/semantic_kernel/connectors/mcp.py` still
defines `MCPStreamableHttpPlugin`. The example needs an OpenAI key, so we
did not run it.

```sh
pip install semantic-kernel
export OPENAI_API_KEY="sk-..."
python examples/semantic-kernel/emem_mcp_geospatial_agent.py
```

### Upstream state

| Target | State on 7 November 2026 |
|---|---|
| [microsoft/semantic-kernel#14228](https://github.com/microsoft/semantic-kernel/pull/14228) | Opened 28 July 2026 by kumari-jaya. It added `python/samples/concepts/mcp/emem_mcp_geospatial_agent.py`. Closed without merge on 28 September 2026. The maintainer wrote that Semantic Kernel takes no new features and pointed to microsoft/agent-framework. |
| `MicrosoftDocs/semantic-kernel-docs` | No emem PR found. The page `semantic-kernel/concepts/plugins/adding-mcp-plugins.md` exists. |
| `microsoft/agent-framework` | No emem issue or PR found. |

## References

- Federated connector guide: https://learn.microsoft.com/en-us/microsoft-365/copilot/connectors/submit-federated-connector
- Copilot Studio MCP certification: https://learn.microsoft.com/en-us/microsoft-copilot-studio/mcp-certification
- Partner Center: https://partner.microsoft.com/
- Semantic Kernel MCP plugins: https://github.com/MicrosoftDocs/semantic-kernel-docs/blob/main/semantic-kernel/concepts/plugins/adding-mcp-plugins.md
- emem MCP endpoint: https://emem.dev/mcp
- emem privacy policy: https://emem.dev/privacy (200 on 7 November 2026)
- emem terms: https://emem.dev/terms (200 on 7 November 2026)
