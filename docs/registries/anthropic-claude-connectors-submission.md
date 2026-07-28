# Anthropic Claude Connectors Directory — Submission Guide

**Status:** Previous form submission (1FAIpQLSeafJF2...) went to a **deprecated form that is now closed.**  
**Action required:** Re-submit via the new portal.

---

## What is already done (do not re-implement)

The emem server is technically compliant. Before doing anything, verify these are live:

| Requirement | emem endpoint | Expected |
|---|---|---|
| Tool annotations | `POST /mcp` → `tools/list` | Every tool has `annotations.title`, `annotations.readOnlyHint`, `annotations.destructiveHint`, `annotations.idempotentHint`, `annotations.openWorldHint` |
| OAuth Protected Resource | `GET /.well-known/oauth-protected-resource` | Returns RFC 9728 JSON with `auth_required: false` and `auth: "none"` |
| MCP discovery | `GET /.well-known/mcp.json` | Returns server card |
| Privacy policy | `GET https://emem.dev/privacy` | Returns human-readable privacy document |
| Streamable HTTP transport | `POST https://emem.dev/mcp` | Responds to JSON-RPC 2.0 `tools/list` and `tools/call` |
| HTTPS | `https://emem.dev` | TLS required — already live |

**Quick sanity check before submitting:**
```bash
# Confirm annotations are present in tools/list
curl -s -X POST https://emem.dev/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' \
  | jq '.result.tools[0].annotations'
# Expected: { title: "...", readOnlyHint: true, destructiveHint: false, ... }

# Confirm OAuth protected resource
curl -s https://emem.dev/.well-known/oauth-protected-resource | jq .
# Expected: { auth_required: false, auth: "none", ... }

# Confirm privacy policy loads
curl -s https://emem.dev/privacy | head -5
```

If all three return expected output, proceed. Do not touch the server code.

---

## Pre-submission checklist

### 1. Portal access — do this first

The submission portal requires a **Team or Enterprise Claude.ai organization** with Directory management access. Individual plans cannot access it.

- [ ] Confirm Vortx AI has a Claude.ai Team or Enterprise org
- [ ] Confirm you have Owner or Directory management role in that org
- [ ] If not, upgrade at https://claude.ai/settings/billing or create the org at https://claude.ai

### 2. Prepare listing copy (fill these in before opening the portal)

The portal will ask for all of this and **does not save drafts well** — have it ready before you start.

| Field | Limit | Suggested value |
|---|---|---|
| **Name** | 100 chars | `emem — verifiable Earth memory` |
| **Tagline** | 55 chars | `Signed, cite-able facts about any place on Earth` |
| **Description** | 2000 chars | See block below |
| **Categories** | pick from list | `Data & Research`, `Productivity`, `Developer Tools` |
| **Server URL** | — | `https://emem.dev/mcp` |
| **Transport** | — | Streamable HTTP |
| **Auth type** | — | None (reads are anonymous; L2 writes use ed25519 key, not OAuth) |
| **Data direction** | — | Both (read-heavy; write = attest signed derivations) |
| **Privacy policy URL** | — | `https://emem.dev/privacy` |
| **Docs URL** | — | `https://emem.dev/agents.md` |
| **Support contact** | — | team@vortx.ai (or your support email) |
| **Slug** | — | `emem` |
| **Icon** | SVG or URL | `https://vortx.ai/assets/vortx-logo-36.gif` (replace with square SVG if available) |

**Suggested description (fits in 2000 chars):**
```
emem is shared, verifiable memory for AI agents — a cite-able, content-addressed, 
signed record of what every place on Earth looks like right now and how it has changed.

When an agent asks "is the air bad here?", "has this site flooded?", "is this farmland 
being deforested?", or "what is the elevation?" — it calls emem_locate then emem_recall, 
gets a signed fact with a receipt, and quotes the receipt. No API keys, no signup, no 
rate limits for reads.

Every response includes an Ed25519-signed receipt verifiable offline. Facts are 
content-addressed with BLAKE3: change one byte, the ID changes. Agents at different 
companies can agree on the same signed observation without trusting each other or the 
server — only cryptography and the bytes themselves.

Core tools:
• emem_locate — resolve any place name or coordinate to a canonical cell64 address
• emem_recall — read signed facts at that location (air quality, vegetation, flood, fire, elevation, weather)
• emem_memory_token — compose a 38-char cite-able handle for a single fact
• emem_verify_receipt — verify a fact's signature offline
• emem_ask — free-text question about any place, answered with cited signed facts

Use cases: climate risk assessment, agricultural monitoring, deforestation detection, 
wildfire tracking, flood mapping, robot fleet coordination, multi-agent fact sharing, 
audit-grade reporting.

Earth is the substrate, not the subject. 104 tools. Apache 2.0.
```

**Suggested use cases for the portal form:**
```
1. Climate and environmental risk — agents recall signed flood, fire, and deforestation 
   alerts at specific locations for insurance underwriting, ESG reporting, or disaster response.

2. Multi-agent coordination — agents pass emem:fact: tokens to each other as verifiable 
   citations; the receiving agent resolves the token to byte-identical signed bytes without 
   trusting the sender.

3. Long-horizon research — signed facts outlive context compaction and session ends; 
   agents resume by resolving tokens rather than redoing fetches.
```

### 3. Example prompts (portal requires minimum 3)

Prepare these — paste them exactly:

```
1. "Is the air quality safe to exercise outside in Mumbai right now?"
2. "Has the forest cover in the Amazon near Manaus changed in the last 6 months? Give me a citable receipt."
3. "What is the elevation and flood risk for this coordinate: 28.6139° N, 77.2090° E?"
4. "Create a memory token for the current NDVI reading at the Sahel region so I can cite it in my report."
5. "Verify this emem fact receipt: emem:fact:defi.zb572.xoso.zb1ec:jwkqm6ehelmzrwupfwyq2oqotiarexr5bdrt4xbl3znuynhurqxq"
```

### 4. Test account instructions for Anthropic reviewers

emem reads require **no credentials**. Write this exactly in the "Test & Launch" step:

```
No login or API key is required for any read operation.

To test:
1. Add emem to Claude via: Settings → Connectors → Add custom connector → https://emem.dev/mcp
2. Ask: "What is the current air quality in Delhi?"
3. Ask: "Is there recent deforestation near -3.4653° lat, -62.2159° lon (Amazon)?"
4. Ask: "Create a memory token for the vegetation index at Central Park, New York."

All responses include a signed receipt. Use the /verify page at https://emem.dev/verify 
to confirm signatures.

Write operations (emem_attest, emem_derive) require an ed25519 attester key — 
these are not needed for reviewer testing.
```

### 5. Branding assets

- [ ] Prepare a square logo — SVG preferred, minimum 512×512 PNG accepted
- [ ] Prepare a favicon (32×32 or 64×64)
- [ ] If no square SVG exists, create one from the Vortx logo before submitting

---

## Submission steps

1. Go to: **`https://claude.ai/admin-settings/directory/submissions/new`**
2. Complete all 11 portal steps in order:
   - Step 1 Introduction — read and confirm scope
   - Step 2 Connection — URL: `https://emem.dev/mcp`, Transport: Streamable HTTP
   - Step 3 Tools — portal will sync tools automatically; verify annotations are detected (they are)
   - Step 4 Listing — fill from the copy table above
   - Step 5 Use Cases — paste the 3 use cases above
   - Step 6 Company — Vortx AI, contact info
   - Step 7 Authentication — select **"None"**; add note: "L0/L1 reads are anonymous; L2 writes use ed25519 keys via /v1/attest_cbor — no OAuth"
   - Step 8 Data Handling — not a health data product; not sponsored content; API owned by Vortx AI
   - Step 9 Test & Launch — paste the reviewer instructions from section 4 above
   - Step 10 Compliance — acknowledge all 7 policy checkboxes
   - Step 11 Review — submit
3. After submitting, note the submission ID and track at the same admin URL

**Expected review timeline:** 2 weeks to several months (Anthropic does not publish SLAs).  
**emem remains usable as a custom connector at `https://emem.dev/mcp` while review is pending.**

---

## If the portal rejects at Step 3 (tool validation)

The portal scans `tools/list` live. If it flags missing annotations, run:

```bash
curl -s -X POST https://emem.dev/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' \
  | jq '[.result.tools[] | {name, has_title: (.annotations.title != null), read_only: .annotations.readOnlyHint}]'
```

Every tool must show `has_title: true`. If any show `false`, the annotations block in
`crates/emem-mcp/src/lib.rs` is not being serialized to the MCP response — check
`mcp_tool_descriptor()` in `crates/emem-api-rest/src/lib.rs` around line 15561.

---

## References

- Submission portal: https://claude.ai/admin-settings/directory/submissions/new
- Submission guide: https://claude.com/docs/connectors/building/submission
- Review criteria: https://claude.com/docs/connectors/building/review-criteria
- emem MCP endpoint: https://emem.dev/mcp
- emem agent guide: https://emem.dev/agents.md
- emem privacy policy: https://emem.dev/privacy
- emem OAuth protected resource: https://emem.dev/.well-known/oauth-protected-resource
