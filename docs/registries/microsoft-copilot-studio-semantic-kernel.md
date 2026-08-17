# Microsoft Copilot Studio + Semantic Kernel, Integration Guide

**Status:** Not started  
**Two Microsoft paths, pursue in order:**
1. M365 Copilot Federated Connector (form submission, fastest)
2. Copilot Studio MCP Certification (Partner Center, broader reach)
3. Semantic Kernel example (code contribution)

---

## What is already done

| Asset | Location | Status |
|---|---|---|
| MCP endpoint | `https://emem.dev/mcp` (Streamable HTTP) | ✅ live |
| Tool annotations (`readOnlyHint`, `title`) | All tools in `tools/list` | ✅ present |
| 192×192 color icon | `web/icon-192.png` | ✅ exists |
| 512×512 logo | `web/logo.png` | ✅ exists |
| Privacy policy | `https://emem.dev/privacy` | ✅ live |
| Terms | `https://emem.dev/terms` | ✅ live |
| AutoGen example (close relative) | `examples/autogen/emem_mcp_geospatial_agent.py` | ✅ exists |

**Missing:**
- 32×32 white-on-transparent outline PNG (`outline.png`), must be created
- `mcptools.json`, tool definitions file for Copilot Studio package
- `manifest.json`, Teams/Copilot Studio package manifest
- `intro.md`, connector documentation file
- Semantic Kernel example (`examples/semantic-kernel/`)

---

## Path 1, M365 Copilot Federated Connector (do first)

This lists emem in the **Microsoft 365 Copilot connectors gallery**, available to all M365 enterprise tenants whose admins approve it. Fastest path. No Partner Center needed.

### Auth blocker, email first

The docs say "Authenticated scenarios use OAuth 2.0." emem has **no auth for reads**. Before filling the form, email Microsoft to confirm no-auth servers are accepted:

```
To: submit-fcc@microsoft.com
Subject: Federated Connector submission — no-auth read-only MCP server

Hi,

We're preparing to submit emem (https://emem.dev/mcp) as a federated connector.
emem is a read-only MCP server — all tools carry readOnlyHint: true and reads
require no authentication (no OAuth, no API key).

Could you confirm whether no-auth / anonymous read-only MCP servers are accepted
in the federated connector program, and if so, how to handle the OAuth credential
fields in the submission form?

Thank you,
Vortx AI
```

**If they confirm no-auth is OK**, proceed with the form. If OAuth is required, emem would need to add a lightweight OAuth wrapper for the reads, but this is unlikely to be required for a fully public read API.

### Prepare before filling the form

| Field | Limit | Value |
|---|---|---|
| **MCP server URL** |, | `https://emem.dev/mcp` |
| **Connector display name** |, | `emem, verifiable Earth memory` |
| **Short description** | ≤80 chars | `Signed, cite-able Earth observation facts for any place. No API key.` |
| **Synonyms** (keywords) | ≤10 | `earth observation, geospatial, climate risk, satellite, air quality, flood, deforestation, vegetation, signed memory, MCP` |
| **Color logo** | 192×192 PNG | `web/icon-192.png`, already correct size ✅ |
| **Outline logo** | 32×32 white-on-transparent PNG | **Must be created**, see below |
| **Privacy policy** |, | `https://emem.dev/privacy` |
| **Docs / support** |, | `https://emem.dev/agents.md` |
| **Contact** |, | `avijeet@vortx.ai` |

**Create the 32×32 outline PNG:**
```bash
# Using ImageMagick — install if needed: brew install imagemagick
convert web/logo.png \
  -resize 32x32 \
  -background transparent \
  -alpha set \
  -channel RGB -white-threshold 1% \
  web/outline-32.png
# Or manually create a 32x32 white-on-transparent version of the logo mark
```

**Tool list to include** (all read-only, include in form):
```
emem_locate        — Resolve place name/coordinate to canonical cell64 address
emem_recall        — Read signed facts at a location (air quality, flood, fire, vegetation, elevation)
emem_ask           — Free-text question about a place, answered with signed facts
emem_memory_token  — Compose a cite-able emem:fact: handle for any fact
emem_verify_receipt — Verify a fact's Ed25519 signature offline
emem_deforestation_alert — NDVI drop + embedding change alert
emem_burn_severity — Burn severity from pre/post-fire Sentinel-2
emem_band_raster   — Signed raster derivation for any band over a region
emem_hunt          — Find event hotspots (flood, fire, drought) over a region
emem_entity        — Mint or get a canonical object identity
```
All tools carry `readOnlyHint: true`. Confirm via:
```bash
curl -s -X POST https://emem.dev/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' \
  | jq '[.result.tools[] | select(.annotations.readOnlyHint == true) | .name]'
```

**Sample prompts (form requires ≥3 with expected responses):**
```
1. "Is the air quality in Delhi safe for outdoor exercise today?"
   → emem_locate("Delhi") → emem_recall(band: "aqi") → returns signed AQI fact with receipt

2. "Has there been recent deforestation near the Amazon at -3.47°N, -62.22°W?"
   → emem_deforestation_alert(place: "Amazon, -3.47, -62.22") → signed NDVI alert

3. "What is the elevation and flood risk at 28.61°N, 77.21°E (New Delhi)?"
   → emem_locate → emem_recall(bands: ["elevation", "flood_extent"]) → signed facts
```

**Test credentials for reviewer:**
```
No login required. Add emem as a connector with URL: https://emem.dev/mcp
All read tools work anonymously. Verify any returned receipt at https://emem.dev/verify
```

### Submit
**Form:** `https://aka.ms/FccSubmissionForm`  
**Questions:** `submit-fcc@microsoft.com`

---

## Path 2, Copilot Studio MCP Certification (Partner Center)

This lists emem in **Copilot Studio + Azure Foundry**, broader than M365 Copilot alone.

### Prerequisites (complete before building the package)

- [ ] Create a **Microsoft Partner Center account**: `https://partner.microsoft.com/`
- [ ] Complete **business verification** in Partner Center
- [ ] Enroll in the **Microsoft 365 and Copilot program** inside Partner Center
- [ ] Create an **Azure subscription** (needed for Key Vault)
- [ ] Create an **Azure Key Vault** (required even for no-auth, store placeholder secrets)

### Package files to create

**1. `manifest.json`**, save to `examples/copilot-studio/manifest.json`:
```json
{
  "$schema": "https://developer.microsoft.com/en-us/json-schemas/teams/vDevPreview/MicrosoftTeams.schema.json",
  "manifestVersion": "devPreview",
  "version": "2.2.0",
  "id": "com.vortxai.emem",
  "developer": {
    "name": "Vortx AI Private Limited",
    "websiteUrl": "https://emem.dev",
    "privacyUrl": "https://emem.dev/privacy",
    "termsOfUseUrl": "https://emem.dev/terms"
  },
  "name": {
    "short": "emem",
    "full": "emem — verifiable Earth memory"
  },
  "description": {
    "short": "Signed, cite-able Earth observation facts for any place. No API key.",
    "full": "emem is shared, verifiable memory for AI agents grounded in Earth observation. Recall Ed25519-signed, BLAKE3 content-addressed facts about air quality, vegetation, flood extent, fire severity, elevation, and deforestation for any place on Earth. No API key, no signup, no rate limits for reads. Every response includes an offline-verifiable receipt. 104 MCP tools. Apache 2.0."
  },
  "agentConnectors": [
    {
      "id": "emem-connector",
      "displayName": "emem",
      "description": "Signed Earth observation facts. emem_locate → emem_recall → cite with emem:fact: token.",
      "toolSource": {
        "remoteMcpServer": {
          "mcpServerUrl": "https://emem.dev/mcp",
          "mcpToolDescription": {
            "file": "mcptools.json"
          },
          "authorization": {
            "type": "AzureKeyVault",
            "referenceId": "https://<your-keyvault>.vault.azure.net/"
          }
        }
      }
    }
  ],
  "icons": {
    "outline": "outline-32.png",
    "color": "icon-192.png"
  },
  "accentColor": "#1a3a5c"
}
```
Replace `<your-keyvault>` with your actual Azure Key Vault URI.

**2. `mcptools.json`**, save to `examples/copilot-studio/mcptools.json`:

Generate from the live server:
```bash
curl -s -X POST https://emem.dev/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' \
  | jq '[.result.tools[] | {
      name: .name,
      title: .annotations.title,
      description: .description,
      inputSchema: .inputSchema,
      annotations: { readOnlyHint: .annotations.readOnlyHint }
    }]' > examples/copilot-studio/mcptools.json
```

**3. `intro.md`**, save to `examples/copilot-studio/intro.md`:
```markdown
# emem — verifiable Earth memory

emem provides shared, verifiable memory for AI agents grounded in Earth observation.
Recall Ed25519-signed facts about any place on Earth — air quality, flood extent,
fire severity, vegetation health, elevation, deforestation — with offline-verifiable receipts.

## Setup

No API key or signup required. Add the connector URL: `https://emem.dev/mcp`

## Example prompts

- "Is the air quality in Delhi safe for exercise today?"
- "Has the Amazon near Manaus been deforested recently? Give me a citable receipt."
- "What is the flood risk at 28.6°N, 77.2°E?"

## Authentication

No authentication required for read operations.
Write operations (attesting derivations) require an ed25519 key — not relevant for most users.

## Known limitations

- Ground resolution is ~9.55 m (64-bit cell address). Sub-10-metre queries are not supported.
- High-frequency streams (>1 write/second) are outside the design envelope.
- Write operations are not available through this connector.

## Support

- Homepage: https://emem.dev
- Docs: https://emem.dev/agents.md
- GitHub: https://github.com/Vortx-AI/emem
- Email: avijeet@vortx.ai
```

**4. Icons**, copy existing assets:
```bash
cp web/icon-192.png examples/copilot-studio/icon-192.png
# Create outline-32.png (32x32 white-on-transparent):
convert web/logo-mark.png -resize 32x32 -background transparent examples/copilot-studio/outline-32.png
```

### Submit via Partner Center
1. Go to `https://partner.microsoft.com/`
2. Create new offer → **Apps and Agents for M365 and Copilot**
3. Upload the package (manifest.json + mcptools.json + intro.md + icons)
4. Fill in commercial, legal, support, publisher information
5. Submit for review

---

## Path 3, Semantic Kernel example

Semantic Kernel is Microsoft's .NET + Python agent framework with native MCP support. An example in their repo gives emem visibility to a large enterprise developer audience.

### Create the example

**File: `examples/semantic-kernel/emem_mcp_geospatial_agent.py`**

Base it on `examples/autogen/emem_mcp_geospatial_agent.py`, the pattern is the same. Use `MCPStreamableHttpPlugin`:

```python
"""
emem geospatial memory example for Microsoft Semantic Kernel.

Uses emem as a Streamable HTTP MCP plugin — no API key required.

Install:
    pip install semantic-kernel

Run:
    python emem_mcp_geospatial_agent.py
"""

import asyncio
from semantic_kernel import Kernel
from semantic_kernel.connectors.mcp import MCPStreamableHttpPlugin
from semantic_kernel.connectors.ai.open_ai import OpenAIChatCompletion

async def main():
    kernel = Kernel()
    kernel.add_service(OpenAIChatCompletion(ai_model_id="gpt-4o"))

    async with MCPStreamableHttpPlugin(
        name="emem",
        description="Verifiable Earth memory — signed facts for any place on Earth",
        url="https://emem.dev/mcp",
    ) as emem_plugin:
        kernel.add_plugin(emem_plugin)

        result = await kernel.invoke_prompt(
            "Is the air quality in Delhi safe for outdoor exercise today? "
            "Give me a signed fact with a receipt I can cite."
        )
        print(result)

if __name__ == "__main__":
    asyncio.run(main())
```

Also create `examples/semantic-kernel/README.md` following the same pattern as `examples/autogen/README.md`.

### Submit PR to microsoft/semantic-kernel

```
Repo: https://github.com/microsoft/semantic-kernel
Target path: python/samples/demos/ or python/samples/concepts/mcp/
PR title: "Add emem geospatial memory MCP example for Semantic Kernel"
```

Also submit PR to `MicrosoftDocs/semantic-kernel-docs`:
```
Repo: https://github.com/MicrosoftDocs/semantic-kernel-docs
Target path: semantic-kernel/concepts/plugins/
Add emem as an example under "Adding MCP plugins"
```

---

## Summary: what to do and in what order

| # | Action | Blocker | Time |
|---|---|---|---|
| 1 | Email `submit-fcc@microsoft.com` asking about no-auth servers | None | 5 min |
| 2 | Create `outline-32.png` (32×32 white-on-transparent icon) | None | 10 min |
| 3 | Create `examples/semantic-kernel/` example + README | None | 1 hr |
| 4 | Submit PR to `microsoft/semantic-kernel` | Working example | 30 min |
| 5 | Submit Federated Connector form (`https://aka.ms/FccSubmissionForm`) | Email response from step 1 | 30 min |
| 6 | Set up Partner Center + Azure Key Vault | Microsoft account | 1-2 days |
| 7 | Build Copilot Studio package (manifest + mcptools.json + intro.md) | Partner Center setup | 2 hrs |
| 8 | Submit Partner Center "Apps and Agents for M365 and Copilot" | Package complete | 1 hr |

---

## References

- Federated connector form: https://aka.ms/FccSubmissionForm
- Federated connector docs: https://learn.microsoft.com/en-us/microsoft-365/copilot/connectors/submit-federated-connector
- Copilot Studio MCP certification: https://learn.microsoft.com/en-us/microsoft-copilot-studio/mcp-certification
- Partner Center: https://partner.microsoft.com/
- Semantic Kernel MCP docs: https://github.com/MicrosoftDocs/semantic-kernel-docs/blob/main/semantic-kernel/concepts/plugins/adding-mcp-plugins.md
- emem MCP endpoint: https://emem.dev/mcp
- emem privacy: https://emem.dev/privacy
- emem terms: https://emem.dev/terms
