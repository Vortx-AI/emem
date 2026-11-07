# Submit emem to the Anthropic Connectors Directory

This page is a checklist for the person who submits emem to the Claude
Connectors Directory. It lists what the server already does, the listing
copy to paste, and what each portal step asks for.

The portal steps below come from Anthropic's
[Submit a connector](https://claude.com/docs/connectors/building/submission)
page, read on 7 November 2026. Read that page again before you submit,
because Anthropic changes the portal.

## Submission status

An earlier submission went to a Google form that Anthropic has closed.
Submit again through the developer portal at
[claude.ai/directory/manage](https://claude.ai/directory/manage).
This page does not record a new submission. Nobody checked the portal for
one on 7 November 2026, because the portal needs a login.

## What the server already does

Do not change the server code for the submission. Check that these items
are live before you start. All of them were live on 7 November 2026.

| Directory requirement | emem endpoint | State on 7 November 2026 |
|---|---|---|
| Remote server over HTTPS | `https://emem.dev/mcp` | Answers JSON-RPC 2.0 `tools/list` and `tools/call` over Streamable HTTP |
| A title and a `readOnlyHint` or `destructiveHint` on every tool | `tools/list` on `/mcp` and `/mcp/full` | All 18 tools on `/mcp` and all 116 tools on `/mcp/full` carry `title`, `readOnlyHint`, `destructiveHint`, `idempotentHint` and `openWorldHint` |
| No authentication for public data | `GET /.well-known/oauth-protected-resource` | RFC 9728 JSON with `"auth": "none"` and `"auth_required": false` |
| Privacy policy | `GET https://emem.dev/privacy` | Markdown policy, last updated 29 September 2026 |
| Documentation | `GET https://emem.dev/agents.md` | The agent guide, served as Markdown |
| MCP server card | `GET /.well-known/mcp.json` | JSON server card |

On `/mcp/full`, 65 of the 116 tools set `readOnlyHint: true`. Five tools set
`destructiveHint: true`: `emem_memory_create`, `emem_memory_str_replace`,
`emem_memory_insert`, `emem_memory_delete` and `emem_memory_rename`.

Run these three checks before you submit:

```bash
# 1. The first tool on /mcp has all five annotations.
curl -s -X POST https://emem.dev/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' \
  | jq '.result.tools[0].annotations'

# 2. The protected-resource document says no authentication.
curl -s https://emem.dev/.well-known/oauth-protected-resource | jq '{auth, auth_required}'

# 3. The privacy policy loads.
curl -s https://emem.dev/privacy | head -3
```

On 7 November 2026, check 1 returned this for `emem_entity`:

```json
{
  "destructiveHint": false,
  "idempotentHint": true,
  "openWorldHint": true,
  "readOnlyHint": false,
  "title": "Mint or get a canonical object identity"
}
```

Check 2 returned `{"auth": "none", "auth_required": false}`. Check 3
returned the heading `# emem Privacy Policy`.

## Before you open the portal

### Get an account that can submit

Anthropic's
[Confirm you can submit](https://claude.com/docs/directory/publish#confirm-you-can-submit-to-the-directory)
section sets these rules (read on 7 November 2026):

- Use a Pro, Max, Ultra, Team or Enterprise plan. Free accounts cannot submit.
- On Team and Enterprise, an Owner submits. An Owner can also give the
  Directory permission to a member through a custom role.
- The listing belongs to the organization that submits it. Submit from the
  organization that must own the emem listing.

The earlier version of this page said that only Team and Enterprise
organizations can submit. That rule no longer applies.

### Prepare the listing copy

The Listing step asks for these fields. The limits come from the Anthropic
page. The portal saves your progress in the browser, but have the copy
ready before you start.

| Field | Limit | Suggested value |
|---|---|---|
| Server name | 100 characters | `emem, verifiable Earth memory` |
| One-liner | 200 characters | `Signed facts about any place on Earth that another agent can cite and verify offline` |
| Description | 2,000 characters | The block below |
| Categories | 1 to 5, from the portal list | Data and research, developer tools |
| Documentation URL | | `https://emem.dev/agents.md` |
| Privacy policy URL | | `https://emem.dev/privacy` |
| Support contact | | `avijeet@vortx.ai`, the contact in the privacy policy, or a support address |
| Icon | | `web/icon-512.png` (512 by 512 PNG, served at `https://emem.dev/icon-512.png`) or `web/favicon.svg` |
| URL slug | Permanent once published | `emem` |

The old icon URL, `https://vortx.ai/assets/vortx-logo-36.gif`, returned
404 on 7 November 2026. Use one of the icons in `web/`.

Suggested description (1,127 characters):

```text
emem is shared, verifiable memory for AI agents. It keeps a signed,
content-addressed record of what places on Earth look like and how they
change.

An agent asks about air quality, flooding, forest loss, elevation or
vegetation at a place. It calls emem_locate, then emem_recall, and gets a
signed fact with a receipt. Reads need no API key and no signup.

Every response carries an Ed25519 receipt that anyone can verify offline.
Each fact has a BLAKE3 content address, so a change to one byte changes
its ID. Agents at different companies can agree on one signed observation
without trusting each other or the server.

Core tools:
- emem_locate: turn a place name or a coordinate into a cell64 address.
- emem_recall: read the signed facts at that address.
- emem_memory_token: make a citation token for one fact.
- emem_verify_receipt: check a receipt signature.
- emem_ask: ask a free-text question about a place and get cited facts.

Use cases: climate risk, crop monitoring, deforestation checks, wildfire
and flood mapping, multi-agent fact sharing, audit reports.

18 core tools on /mcp, 116 on /mcp/full. Apache-2.0.
```

The old description said "no rate limits for reads". That is false: the
server returns 429 `rate_limited` with `Retry-After`. It also called a
citation token 38 characters long. An `emem:fact:` token is 83 to 84
characters. A 38-character token is an `emem:bundle:` token
(`crates/emem-mcp/src/lib.rs`, `emem_memory_token` description).

### Prepare the use cases

The Use cases step asks for the main use cases and what users need before
they connect. It also asks if the connector reads data, writes data, or both.

```text
1. Climate and environment risk. Agents recall signed flood, fire and
   deforestation facts at a place for insurance, ESG reports or disaster
   response.

2. Multi-agent work. Agents pass emem:fact: tokens to each other as
   citations. The receiver resolves a token to the same signed bytes and
   does not have to trust the sender.

3. Long tasks. Signed facts outlive a context reset. An agent resumes by
   resolving its tokens again, not by fetching the data again.

What users need: nothing. Reads need no account and no key.

Data direction: both. Most calls read. Writes are signed memory notes
and derivations, each signed with the writer's own Ed25519 key.
```

### Prepare the example prompts

Use prompts that reviewers can run with no account:

```text
1. Is the air quality safe for exercise outside in Mumbai now?
2. Did the forest cover near Manaus change in the last 6 months? Give me a citable receipt.
3. What is the elevation at 28.6139 N, 77.2090 E, and is there surface water nearby?
4. Make a citation token for the current NDVI at Central Park, New York.
5. Verify this emem fact: emem:fact:defi.zb572.xoso.zb1ec:jwkqm6ehelmzrwupfwyq2oqotiarexr5bdrt4xbl3znuynhurqxq
```

On 7 November 2026, the token in prompt 5 resolved through
`POST /v1/memory_token/resolve` to an `indices.ndvi` fact with value
0.487. The old prompt 4 named "the Sahel region", which is not one place.
Prompt 4 now names a point.

### Prepare the reviewer instructions

The Test & launch step asks for setup steps a reviewer can follow, and for
your confirmation that you ran every tool. Paste this text:

```text
No login or API key is needed for any read.

To test:
1. In Claude, add a custom connector with the URL https://emem.dev/mcp.
2. Ask: "What is the current air quality in Delhi?"
3. Ask: "Is there recent deforestation near -3.4653, -62.2159 (Amazon)?"
4. Ask: "Make a citation token for the vegetation index at Central Park, New York."

Each answer carries a signed receipt. Check a receipt at
https://emem.dev/verify.

Writes (emem_derive and the emem_memory_* write tools) need an Ed25519
key that the caller makes. Reviewers do not need them to test.
```

The old text named `emem_attest` as a write tool. No MCP tool has that
name on 7 November 2026. Attestation is a REST route (`POST /v1/attest_cbor`).

## Portal steps

Open [claude.ai/directory/manage](https://claude.ai/directory/manage),
select **Submit new**, and choose **MCP connector**. On 7 November 2026
the portal had 10 steps:

| Step | What it asks for | What to enter for emem |
|---|---|---|
| 1. Connection | The `https://` server URL | `https://emem.dev/mcp`. Users all connect to the same URL. |
| 2. Tools | Nothing. The portal reads tools, prompts and resources from the server. | Check that the portal flags no tool for a missing title or annotation. |
| 3. Listing | The listing fields | The listing table above |
| 4. Use cases | Use cases, what users need, read or write | The use-case block above |
| 5. Company | Company name, website, main contact | Vortx AI and its contact |
| 6. Authentication | Whether users must sign in, and how | Not required. Add: "Reads are anonymous. Writes use a per-write Ed25519 signature, not OAuth." |
| 7. Data handling | Who owns the API, health data, sponsored content | Vortx AI owns the API. No personal health data. No sponsored content. |
| 8. Test & launch | Reviewer setup and your test confirmation | The reviewer block above |
| 9. Compliance | Seven policy acknowledgments | Read and accept all seven. The portal needs all of them. |
| 10. Review and submit | A final check | Read any quality warnings, then submit. |

The earlier version of this page listed 11 steps and an
`admin-settings/directory/submissions/new` URL. The current page has no
separate Introduction step.

## After you submit

Anthropic's page says that an automatic scan checks the submission for
policy compliance. By default, Anthropic lists a submission that passes
as a Community connector. Some submissions also get a review by a person, and
the time depends on the queue. The status and any reviewer feedback show
in the portal at claude.ai/directory/manage. For escalations, the page
gives `mcp-review@anthropic.com`.

emem stays usable as a custom connector at `https://emem.dev/mcp` while a
submission is open.

## If the Tools step flags a tool

The portal reads `tools/list` from the live server. To see which tool
lacks a title or a hint, run:

```bash
curl -s -X POST https://emem.dev/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' \
  | jq '[.result.tools[] | {name, has_title: (.annotations.title != null), read_only: .annotations.readOnlyHint}]'
```

Every tool must show `has_title: true`. On 7 November 2026, all 18 did.
If one shows `false`, look at the annotation data in
`crates/emem-mcp/src/lib.rs` and at `mcp_tool_descriptor` in
`crates/emem-api-rest/src/lib.rs`, which writes the descriptor that
`tools/list` returns.

## References

- Developer portal: https://claude.ai/directory/manage
- Submit a connector: https://claude.com/docs/connectors/building/submission
- Review criteria: https://claude.com/docs/connectors/building/review-criteria
- Who can submit: https://claude.com/docs/directory/publish#confirm-you-can-submit-to-the-directory
- emem MCP endpoint: https://emem.dev/mcp
- emem agent guide: https://emem.dev/agents.md
- emem privacy policy: https://emem.dev/privacy
- emem protected-resource document: https://emem.dev/.well-known/oauth-protected-resource
