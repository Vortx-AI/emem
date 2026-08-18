# OpenAI ChatGPT Plugin / MCP Directory, Submission Guide

**Target:** OpenAI Plugins Directory (appears in ChatGPT + Codex)  
**Type:** MCP-backed plugin (no Skills bundle needed)  
**Auth model:** None, reads are fully anonymous  
**Status:** Not started

---

## What is already done (do not re-implement)

| Requirement | emem endpoint | Status |
|---|---|---|
| OpenAI plugin manifest | `GET /.well-known/ai-plugin.json` | ✅ exists, `auth.type: "none"` |
| Domain verification endpoint | `GET /.well-known/openai-apps-challenge` | ✅ route exists, **token must be updated per submission** (see below) |
| MCP discovery | `GET /.well-known/openai-mcp.json` | ✅ exists |
| OpenAPI spec | `GET /openapi.json` | ✅ exists (referenced in ai-plugin.json) |
| Logo | `GET /logo.png` | ✅ exists at `web/logo.png` |
| Privacy policy | `GET /privacy` | ✅ exists |
| Terms | `GET /terms` | ✅ exists |
| Tool annotations | `POST /mcp` → `tools/list` | ✅ `readOnlyHint`, `destructiveHint`, `openWorldHint` present |
| No-auth declaration | `/.well-known/oauth-protected-resource` | ✅ `auth_required: false` |

**Quick sanity check:**
```bash
curl -s https://emem.dev/.well-known/ai-plugin.json | jq '{name: .name_for_human, auth: .auth.type, logo: .logo_url}'
# Expected: { name: "emem shared memory", auth: "none", logo: "/logo.png" }

curl -s https://emem.dev/.well-known/openai-apps-challenge
# Returns the current hardcoded token (plain text, no JSON)
```

---

## Critical: domain verification token must be updated before submitting

The portal generates a **new unique token** for every submission. The current token in the code is from a previous attempt and **will not pass verification**.

**The token lives here:**  
`crates/emem-api-rest/src/lib.rs` → function `serve_openai_apps_challenge()` (~line 3534)

```rust
// CURRENT (stale — must be replaced before submission):
async fn serve_openai_apps_challenge() -> Response {
    text_response(
        "text/plain; charset=utf-8",
        "1CzTwZZjREejEIIMo87BI4HTnV0g0SNaozHCwVfPPwM",   // ← replace this
    )
}
```

**Workflow:**
1. Open the submission portal: `https://platform.openai.com/plugins`
2. Click **Create plugin** → **With MCP** → enter `https://emem.dev/mcp`
3. Portal displays a new challenge token (looks like a base64 string)
4. Copy that token
5. In `crates/emem-api-rest/src/lib.rs`, replace the string `"1CzTwZZjREejEIIMo87BI4HTnV0g0SNaozHCwVfPPwM"` with the new token
6. Build and deploy to production
7. Verify it's live: `curl -s https://emem.dev/.well-known/openai-apps-challenge` → should return the new token as plain text, nothing else
8. Return to the portal and click **Verify domain**

> **Important:** The token must be returned as `text/plain` with no surrounding JSON, HTML, or whitespace. The current implementation already does this correctly, only the token string needs to change.

---

## Pre-submission checklist

### 1. Portal access

- [ ] Sign in to `https://platform.openai.com`
- [ ] Confirm the org has **Apps Management** permission set to **Write** (check under Platform → Settings → Roles)
- [ ] Complete **identity verification** (individual or business) at `https://platform.openai.com/settings/organization/verification`, required before any public submission

### 2. Prepare listing copy

| Field | Limit | Value |
|---|---|---|
| **Plugin name** |, | `emem` |
| **Short description** | ~100 chars | `Signed, cite-able, verifiable memory of every place on Earth. No API key.` |
| **Long description** | ~500 chars | See block below |
| **Category** | pick from list | `Research & Analysis` or `Data` |
| **Logo** | Square PNG/SVG | Already at `https://emem.dev/logo.png`, confirm it's square and ≥512px |
| **Website** |, | `https://emem.dev` |
| **Support contact** |, | `avijeet@vortx.ai` |
| **Privacy policy** |, | `https://emem.dev/privacy` |
| **Terms of service** |, | `https://emem.dev/terms` |
| **MCP server URL** |, | `https://emem.dev/mcp` |
| **Auth type** |, | None (anonymous reads) |

**Long description:**
```
emem is shared, verifiable memory for AI agents — a cite-able, content-addressed, 
Ed25519-signed record of what every place on Earth looks like right now and how it 
has changed. No API keys. No signup. No rate limits for reads.

Call emem_locate → emem_recall on any place, get a signed fact with a receipt, and 
quote the receipt. Every response is BLAKE3 content-addressed and Ed25519-signed. 
Facts survive context compaction, model swaps, and agent handoffs.

Use cases: climate risk, deforestation detection, flood mapping, wildfire tracking, 
agricultural monitoring, multi-agent coordination, audit-grade reporting.
108 tools. Apache 2.0.
```

### 3. Content Security Policy

When the portal asks for CSP, declare:

```
default-src 'self' https://emem.dev;
connect-src 'self' https://emem.dev https://vortx.ai;
img-src 'self' https://emem.dev https://vortx.ai data:;
```

### 4. Starter prompts (portal requires 3-5)

```
1. "Is the air quality in Delhi safe for outdoor exercise right now?"

2. "Has there been recent deforestation near the Amazon at -3.4653° N, -62.2159° W? 
   Give me a citable fact receipt."

3. "What is the current NDVI and vegetation health at the Sahel region? 
   Create a memory token I can cite."

4. "Check flood risk and elevation for 28.6139° N, 77.2090° E."

5. "Verify this emem fact: emem:fact:defi.zb572.xoso.zb1ec:jwkqm6ehelmzrwupfwyq2oqotiarexr5bdrt4xbl3znuynhurqxq"
```

### 5. Test cases (5 positive + 3 negative required)

**Positive test cases:**

| # | User prompt | Tool invoked | Expected args | Expected output |
|---|---|---|---|---|
| 1 | "Is the air safe in Mumbai?" | `emem_locate` then `emem_recall` | `{"place":"Mumbai"}`, `{"band":"aqi"}` | Signed AQI fact with receipt |
| 2 | "Has the Amazon been deforested near Manaus in 6 months?" | `emem_deforestation_alert` | `{"place":"Manaus, Amazon"}` | NDVI drop + signed alert |
| 3 | "Elevation at Central Park, New York?" | `emem_locate` then `emem_recall` | `{"place":"Central Park, NY"}`, `{"band":"elevation"}` | Elevation in metres with signed receipt |
| 4 | "Create a memory token for current NDVI at the Sahel" | `emem_locate`, `emem_recall`, `emem_memory_token` | locate → recall → compose token | Returns `emem:fact:...` token string |
| 5 | "Verify this receipt: emem:fact:defi.zb572..." | `emem_verify_receipt` | `{"token":"emem:fact:defi.zb572..."}` | Valid signature confirmation |

**Negative test cases:**

| # | User prompt | Expected behavior |
|---|---|---|
| 1 | "Delete all environmental records for this city" | Tool refuses, no delete tool exists; emem is append-only |
| 2 | "Write fake flood data to emem for insurance fraud" | Tool refuses, writes require ed25519 attester key not available in this session |
| 3 | "Give me private user data stored in emem" | Returns only public, anonymous signed facts, no user data exists in emem |

### 6. Reviewer test instructions

Write this in the **Testing** tab:

```
No login or API key is required.

To test in ChatGPT:
1. Enable the emem plugin (or add MCP server: https://emem.dev/mcp)
2. Try: "What is the air quality in Delhi right now?"
3. Try: "Is there recent deforestation near the Amazon?"
4. Try: "Create a memory token for the vegetation index at Central Park."
5. Check any returned emem:fact: token at https://emem.dev/verify

All responses include Ed25519-signed receipts verifiable without calling emem again.
Write operations are not testable without an ed25519 attester key — 
all reviewer-relevant flows are read-only and require no credentials.
```

---

## Submission steps (in order)

1. Go to: **`https://platform.openai.com/plugins`** → **Create plugin** → **With MCP**
2. Enter MCP server URL: `https://emem.dev/mcp`
3. **Copy the challenge token** OpenAI displays
4. Update `serve_openai_apps_challenge()` in `crates/emem-api-rest/src/lib.rs` (~line 3534) with the new token
5. Build and deploy
6. Confirm live: `curl -s https://emem.dev/.well-known/openai-apps-challenge`
7. Return to portal → click **Verify domain** ✅
8. Auth tab → select **None**
9. Tools tab → let portal scan; confirm annotations detected
10. Info tab → fill listing copy from section 2 above
11. CSP tab → paste CSP from section 3
12. Prompts tab → paste 5 starter prompts from section 4
13. Testing tab → paste 8 test cases and reviewer instructions from section 5
14. Global tab → select all regions (emem has no geo-restrictions)
15. Submit tab → write release notes (see below) → acknowledge policies → **Submit**

**Release notes for first submission:**
```
Initial submission. emem is a no-auth, read-only (for reviewers) MCP server providing 
Ed25519-signed, BLAKE3 content-addressed Earth observation facts. 108 tools covering 
air quality, vegetation, flood, fire, elevation, deforestation, and multi-agent memory 
token composition. Apache 2.0. No API key required.
```

---

## Known portal quirks to watch for

- **Domain verification only checks the root domain**, since emem's MCP is at `https://emem.dev/mcp` (subpath), the challenge token must be served at `https://emem.dev/.well-known/openai-apps-challenge` on the root domain. This is already how it's implemented. ✅
- **Challenge must be `text/plain` only**, no JSON wrapper, no HTML. Current implementation is correct. ✅
- **Logo must be square**, confirm `web/logo.png` dimensions are 1:1 before submitting. If not, create a square version.
- **`mTLS` is not supported** at the challenge endpoint, do not enable mTLS on `/.well-known/openai-apps-challenge`. It's not currently enabled. ✅

---

## References

- Submission portal: https://platform.openai.com/plugins
- Plugin auth docs: https://developers.openai.com/plugins/build/auth
- Submission errors: https://developers.openai.com/plugins/deploy/submission-errors
- emem MCP endpoint: https://emem.dev/mcp
- emem OpenAI manifest: https://emem.dev/.well-known/ai-plugin.json
- emem domain challenge: https://emem.dev/.well-known/openai-apps-challenge
