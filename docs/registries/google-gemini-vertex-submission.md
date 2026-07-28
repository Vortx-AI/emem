# Google Gemini / Vertex AI, Integration Guide

**Status:** Not started  
**Note:** Vertex AI was rebranded to **Gemini Enterprise Agent Platform** in 2026.  
There are **four distinct integration paths**, pursue them in priority order.

---

## What is already done (do not re-implement)

| Asset | Location | Status |
|---|---|---|
| Gemini CLI extension manifest | `examples/gemini-extension.json` served at `https://emem.dev/gemini-extension.json` | ✅ built |
| MCP endpoint | `https://emem.dev/mcp` (Streamable HTTP) | ✅ live |
| Agent card (A2A) | `https://emem.dev/.well-known/agent-card.json` | ✅ live |
| OpenAPI spec | `https://emem.dev/openapi.json` | ✅ live |
| No-auth declaration | `auth_required: false` in `/.well-known/oauth-protected-resource` | ✅ live |

**Verify the extension manifest is live before starting:**
```bash
curl -s https://emem.dev/gemini-extension.json | jq '{name, version, transport: .mcpServers.emem.transport}'
# Expected: { name: "emem", version: "1.1.0", transport: "streamable-http" }
```

---

## Path 1, Gemini CLI Extensions catalog (highest reach, do first)

The `gemini-cli-extensions` GitHub org (66+ extensions, Google-maintained) is the primary discovery surface for Gemini CLI users. Getting listed here puts emem alongside official Google extensions (workspace, bigquery, spanner, security, etc.).

### Step 1: Submit to the community awesome-lists (immediate, no approval needed)

These are open PRs, do both in parallel:

**A. `Piebald-AI/awesome-gemini-cli-extensions`**
```
Repo: https://github.com/Piebald-AI/awesome-gemini-cli-extensions
Action: Fork → add emem under "Data & Analytics" → PR
```

Add this line in the Data & Analytics section of the README:
```markdown
- [emem](https://emem.dev/gemini-extension.json) — Signed, cite-able Earth memory.
  Recall air quality, vegetation, flood, fire, elevation facts for any place.
  No API key. Install: `gemini extensions install https://emem.dev/gemini-extension.json`
```

**B. `Piebald-AI/awesome-gemini-cli`**
```
Repo: https://github.com/Piebald-AI/awesome-gemini-cli
Action: Fork → add emem → PR
```

### Step 2: Contact Google to get listed in the official `gemini-cli-extensions` org

The official org (`github.com/gemini-cli-extensions`) requires Google to invite you. File a request via:

1. Open an issue at `https://github.com/google-gemini/gemini-cli` titled:  
   `[Extension Submission] emem, verifiable Earth memory MCP server`

2. Body of the issue:
```
## Extension: emem

**Install command:**
gemini extensions install https://emem.dev/gemini-extension.json

**Manifest:** https://emem.dev/gemini-extension.json

**What it does:**
Shared, verifiable memory for AI agents grounded in Earth observation.
Any Gemini CLI session can call emem_locate → emem_recall on any place and get a
Ed25519-signed, BLAKE3 content-addressed fact with an offline-verifiable receipt.

No API key, no signup, no rate limits for reads.

**Tools exposed:** 104 MCP tools including emem_locate, emem_recall, emem_ask,
emem_memory_token, emem_deforestation_alert, emem_burn_severity, emem_band_raster.

**Use cases:** climate risk, deforestation detection, flood mapping, wildfire tracking,
agricultural monitoring, multi-agent coordination, audit-grade reporting.

**License:** Apache 2.0
**Author:** Vortx AI <avijeet@vortx.ai>
**Homepage:** https://emem.dev
**GitHub:** https://github.com/Vortx-AI/emem
```

### Step 3: Verify the one-command install works end to end

Before any submission, confirm this works in a live Gemini CLI session:

```bash
gemini extensions install https://emem.dev/gemini-extension.json
# Then in a gemini session:
# > What is the current air quality in Delhi?
# Should call emem_locate then emem_recall and return a signed fact.
```

If this fails, debug `examples/gemini-extension.json`, check:
- `mcpServers.emem.url` is `https://emem.dev/mcp`
- `mcpServers.emem.transport` is `streamable-http`
- The manifest is valid JSON (run `jq . examples/gemini-extension.json`)

---

## Path 2, Gemini Enterprise Agent Platform / Vertex AI Agent Registry

This is the **enterprise path**, not a public marketplace listing, but documentation that lets Google Cloud customers register emem into their own Agent Registry in minutes.

### What this looks like for a customer

An enterprise customer registers emem like this in their Google Cloud project:

```bash
# Using gcloud CLI
gcloud agent-registry services create emem \
  --location=us-central1 \
  --display-name="emem — verifiable Earth memory" \
  --mcp-url="https://emem.dev/mcp" \
  --auth-type=none \
  --description="Signed, cite-able Earth observation facts for AI agents. No API key."
```

Or via Agent Registry MCP tool (once they have Agent Registry set up):
```json
{
  "tool": "create_service",
  "arguments": {
    "display_name": "emem",
    "mcp_url": "https://emem.dev/mcp",
    "transport": "streamable-http",
    "auth": { "type": "none" },
    "description": "Signed Earth observation facts. emem_locate → emem_recall → cite with emem:fact: token."
  }
}
```

### Action: Add emem as an example in Google Cloud's cookbook

Submit a PR adding an emem example notebook to:
```
https://github.com/GoogleCloudPlatform/generative-ai/tree/main/agents/agent_engine
```

The notebook should show:
1. Registering emem in Agent Registry (one gcloud command)
2. Using emem from a Vertex AI / Gemini Enterprise agent
3. Verifying a returned `emem:fact:` token

**Template PR title:** `Add emem geospatial memory MCP server example for Agent Engine`

### Action: Add emem to the Vertex AI Extensions / Tool Gallery

Submit a PR to add emem to the Vertex AI example gallery:
```
https://github.com/GoogleCloudPlatform/vertex-ai-samples
```

---

## Path 3, Google for Startups (credits, do in parallel, independent)

Up to **$350K in Google Cloud + Vertex AI credits**. Gemini and Earth Engine (GEE) are fully covered.

**Apply at:** `https://cloud.google.com/startup`

**What to fill in:**

| Field | Value |
|---|---|
| Company name | Vortx AI Private Limited |
| Product description | Shared, verifiable Earth memory infrastructure for AI agents, signed, cite-able spatial facts at 9.55m resolution, no API key |
| AI/ML use | Yes, satellite foundation models (Clay, Prithvi, Tessera, Galileo), BGE-768 embeddings, GeoTessera spatial encoders |
| Google products used | Vertex AI, Google Earth Engine, Cloud Run / GKE |
| Stage | Early stage / seed |
| Use of credits | Vertex AI for inference, GEE for satellite data processing, Cloud Run for MCP server hosting |

**Expected:** $50K-$350K credits depending on accelerator partnership and stage. Apply even if rejected, re-applications after traction are common.

---

## Path 4, Google Earth Engine Community Tool

GEE has a large academic + commercial user base (researchers, NGOs, governments). Listing emem as a community tool gives access to this audience directly.

**How:**
1. Create an emem example as a GEE script or Colab notebook showing how to use emem alongside GEE data
2. Submit to the GEE Community repo: `https://github.com/google/earthengine-community`
3. PR title: `Add emem MCP server example for verifiable spatial fact memory`

The notebook should show:
- Loading a GEE dataset for a location
- Calling emem to get the signed fact for the same location
- Comparing and citing both

---

## Summary: what to do and in what order

| Priority | Action | Time | Blocker |
|---|---|---|---|
| 0 | Verify `gemini extensions install https://emem.dev/gemini-extension.json` works | 10 min | None |
| 0 | PR to `Piebald-AI/awesome-gemini-cli-extensions` | 15 min | None |
| 0 | PR to `Piebald-AI/awesome-gemini-cli` | 10 min | None |
| 0 | Apply Google for Startups | 30 min | None |
| 1 | Issue at `google-gemini/gemini-cli` to get into official extensions org | 20 min | Needs Google approval |
| 2 | PR to `GoogleCloudPlatform/generative-ai` (Agent Engine example) | 2-3 hrs | Notebook must work end-to-end |
| 2 | PR to `google/earthengine-community` | 2-3 hrs | GEE account needed |
| 3 | Google Cloud Partner Advantage (for marketplace listing) | 1-2 days | Business verification needed |

---

## References

- Gemini CLI extensions org: https://github.com/gemini-cli-extensions
- Gemini CLI repo (for issues): https://github.com/google-gemini/gemini-cli
- Awesome Gemini CLI Extensions: https://github.com/Piebald-AI/awesome-gemini-cli-extensions
- Awesome Gemini CLI: https://github.com/Piebald-AI/awesome-gemini-cli
- Google for Startups: https://cloud.google.com/startup
- Agent Registry docs: https://docs.cloud.google.com/agent-registry
- GEE Community: https://github.com/google/earthengine-community
- Vertex AI samples: https://github.com/GoogleCloudPlatform/vertex-ai-samples
- emem Gemini extension manifest: https://emem.dev/gemini-extension.json
- emem agent card: https://emem.dev/.well-known/agent-card.json
