# Google Gemini and Google Cloud listings for emem

This page tracks the places that list emem for Gemini CLI and Google Cloud users. It also gives the next action for each path.
We checked all states on this page on 7 November 2026, unless a row gives a different date.

## State of each path

| Path | State on 7 November 2026 | Evidence |
|---|---|---|
| Gemini CLI extension gallery | Listed as `Vortx-AI/emem`, extension version 2.4.3, rank 238 | `https://geminicli.com/extensions.json`, entry last updated 14 October 2026 |
| `Piebald-AI/awesome-gemini-cli-extensions` | Listed, under Data & Analytics | PR #21, merged 28 July 2026 |
| `Piebald-AI/awesome-gemini-cli` | Listed, under MCP Servers | PR #83, merged 28 July 2026; PR #101 updated the text, merged 19 August 2026 |
| Issue to Google, `google-gemini/gemini-cli` | Open, no reply from Google staff | Issue #28556, opened 28 July 2026, labels `priority/p3`, `area/extensions` |
| Separate extension repository | `Vortx-AI/emem-gemini-extension`, release v2.4.0 | Released 13 September 2026; has the `gemini-cli-extension` topic |
| Agent Registry example in Google Cloud samples | Not started | No emem PR in `GoogleCloudPlatform/generative-ai` or `GoogleCloudPlatform/vertex-ai-samples` |
| Google Earth Engine community example | Not started | No emem PR in `google/earthengine-community` |
| Google for Startups Cloud Program | Not known | This page cannot see applications |

These checks used `gh api` and the public gallery JSON.

## What emem already serves

| Asset | Where | Notes |
|---|---|---|
| Installable manifest | `gemini-extension.json` at the repository root | Version 2.4.4. It names `httpUrl` `https://emem.dev/mcp` and `contextFileName` `GEMINI.md`. |
| Manifest to read by URL | `https://emem.dev/gemini-extension.json` | The server builds it on each request with live counts (`serve_example_gemini` in `crates/emem-api-rest/src/lib.rs`). |
| Manifest template | `examples/gemini-extension.json` | Old: version 1.1.0, and its description gives a then-113 tool count. The server does not serve this file as is. |
| MCP endpoint | `https://emem.dev/mcp` | Streamable HTTP. `tools/list` gives 18 core tools; `/mcp/full` gives all 116. |
| A2A agent card | `https://emem.dev/.well-known/agent-card.json` | |
| OpenAPI | `https://emem.dev/openapi.json` | |
| Auth declaration | `https://emem.dev/.well-known/oauth-protected-resource` | `auth_required: false`. Reads need no key. Writes need an Ed25519 signature from the caller. |

Check the installable manifest:

```bash
curl -s https://raw.githubusercontent.com/Vortx-AI/emem/main/gemini-extension.json \
  | jq '{name, version, url: .mcpServers.emem.httpUrl}'
```

On 7 November 2026 this returned:

```json
{ "name": "emem", "version": "2.4.4", "url": "https://emem.dev/mcp" }
```

## Path 1: Gemini CLI

### How Gemini CLI installs an extension

The Gemini CLI reference says to install from a GitHub repository URL or a local path.
It does not list a JSON URL as a source (`docs/extensions/reference.md` in `google-gemini/gemini-cli`).
The emem router says the same in a comment above the `/gemini-extension.json` route.

The install command for emem is:

```bash
gemini extensions install https://github.com/Vortx-AI/emem
```

This command did not run for this page. The host that made the checks does not have the Gemini CLI.
To test it, start a Gemini session after the install and ask about a place, for example "What is the elevation of Mount Fuji?".
The session must call `emem_locate`, then `emem_recall`, and show a fact with a receipt.

If the install fails, check these keys in `gemini-extension.json`:

- `mcpServers.emem.httpUrl` is `https://emem.dev/mcp`. Gemini CLI uses `httpUrl` for Streamable HTTP and `url` for SSE (`docs/tools/mcp-server.md` in `google-gemini/gemini-cli`).
- `contextFileName` names a file that exists at the repository root (`GEMINI.md`).
- The file is valid JSON: `jq . gemini-extension.json`.

### The gallery

The gallery at `https://geminicli.com/extensions/browse/` indexes public repositories with no request to Google.
The Gemini CLI release guide gives three conditions: a public repository, the `gemini-cli-extension` topic, and the manifest at the root.
A crawler finds the manifest through GitHub code search, once each day.

On 7 November 2026 the gallery listed `Vortx-AI/emem` at extension version 2.4.3, from 14 October 2026.
The repository root manifest is at 2.4.4. The gallery did not list `Vortx-AI/emem-gemini-extension`.
`Vortx-AI/emem` does not carry the `gemini-cli-extension` topic, but the crawler found it.

### The two awesome lists

Both entries give this install command:

```text
gemini extensions install https://emem.dev/gemini-extension.json
```

That is a JSON URL, which the Gemini CLI reference does not list as a source.
The next PR to each list should change it to `gemini extensions install https://github.com/Vortx-AI/emem`.
The `awesome-gemini-cli` entry also gives a then-108 tool count. On 7 November 2026 `/mcp/full` gave 116.

### The issue to Google

Issue #28556 asks Google to add emem to the official `gemini-cli-extensions` organization.
That organization had 46 public repositories on 7 November 2026.
The release guide says that the gallery needs no issue or email, so this issue is optional.

The issue body has text that is no longer true. Correct it in a new comment, and do not edit the old text:

| Issue text | True on 7 November 2026 |
|---|---|
| Install from `https://emem.dev/gemini-extension.json` | Install from `https://github.com/Vortx-AI/emem` |
| A then-104 tool count | 116 tools in `/mcp/full`, 18 in `tools/list` at `/mcp` |
| No rate limits for reads | Reads can get HTTP 429 with a `Retry-After` header |
| A 38-character `emem:fact:` handle | An `emem:fact:` token is 83 or 84 characters; 38 is the length of an `emem:bundle:` token |

## Path 2: Agent Registry in Google Cloud

Agent Registry is a catalog of MCP servers and agents in a Google Cloud project.
Google's overview page puts it in Gemini Enterprise Agent Platform.
You must register a remote MCP server outside Google Cloud by hand, as a `Service` resource.

The earlier version of this page showed `gcloud agent-registry services create` with `--mcp-url` and `--auth-type`.
The gcloud reference has no such flags. It also showed a `create_service` MCP tool, which Google does not document. This version removes both.

The gcloud reference documents these flags for `gcloud agent-registry services create` on 7 November 2026:

| Flag | Use |
|---|---|
| `--location` | The region, for example `us-central1` |
| `--display-name` | Up to 63 characters |
| `--description` | Up to 2,048 characters |
| `--interfaces` | `url=...,protocolBinding=...`; the manual registration page lists `http-json`, `grpc` and `jsonrpc` |
| `--mcp-server-spec-type` | `no-spec` or `tool-spec` |
| `--mcp-server-spec-content` | The `tools/list` result, for `tool-spec`, up to 10 KB |

No command for emem has run against Agent Registry. A test needs a Google Cloud project with Agent Registry ready and the `roles/agentregistry.editor` role.
Google's pages give no MCP server example, so test the flags before you publish a command.
The 10 KB limit is smaller than the emem core `tools/list` answer (76,820 bytes on 7 November 2026), so use `no-spec` or a short tool list.

When the command works, add an example notebook to `GoogleCloudPlatform/generative-ai`, directory `agents/agent_engine`.
The notebook must register emem, call it from an agent, and verify one `emem:fact:` token.

## Path 3: Google for Startups Cloud Program

Apply at `https://cloud.google.com/startup`. This page did not check the credit amounts or the form fields. Read them on the program page.
Use only numbers that are true on the day you apply, for example the tool count from `/mcp/full`.
The cell size is about 9.55 m at the equator (`docs/model.md`).

## Path 4: Google Earth Engine community example

`google/earthengine-community` takes community tutorials and examples. No emem PR exists.
A useful example reads an Earth Engine dataset at a point, then reads the emem fact for the same cell, and cites both.
This needs an Earth Engine account.

## Next actions

| Order | Action | Needs |
|---|---|---|
| 1 | Test `gemini extensions install https://github.com/Vortx-AI/emem` in a Gemini CLI session | Gemini CLI |
| 2 | PR to each awesome list: change the install command to the GitHub URL; update the tool count | A GitHub account |
| 3 | Comment on issue #28556 with the corrections in the table above | A GitHub account |
| 4 | Decide to keep or archive `Vortx-AI/emem-gemini-extension` (v2.4.0); the gallery lists the main repository | Maintainer decision |
| 5 | Update `examples/gemini-extension.json`, or say in its `_doc` that it is an old template | A code change |
| 6 | Test an Agent Registry command for emem, then write the Agent Engine notebook | A Google Cloud project |
| 7 | Earth Engine community example | An Earth Engine account |

## References

- Gemini CLI extension gallery: https://geminicli.com/extensions/browse/
- Gemini CLI extension release guide: https://github.com/google-gemini/gemini-cli/blob/main/docs/extensions/releasing.md
- Gemini CLI extension reference: https://github.com/google-gemini/gemini-cli/blob/main/docs/extensions/reference.md
- Issue #28556: https://github.com/google-gemini/gemini-cli/issues/28556
- Awesome Gemini CLI Extensions: https://github.com/Piebald-AI/awesome-gemini-cli-extensions
- Awesome Gemini CLI: https://github.com/Piebald-AI/awesome-gemini-cli
- Agent Registry overview: https://docs.cloud.google.com/agent-registry/overview
- Agent Registry manual registration: https://docs.cloud.google.com/agent-registry/manual-registration
- gcloud reference, `services create`: https://docs.cloud.google.com/sdk/gcloud/reference/agent-registry/services/create
- Google for Startups Cloud Program: https://cloud.google.com/startup
- Earth Engine community: https://github.com/google/earthengine-community
- emem manifest by URL: https://emem.dev/gemini-extension.json
