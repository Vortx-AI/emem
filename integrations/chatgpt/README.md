# emem for ChatGPT

This folder holds the ChatGPT app for emem: the listing text, the tool list,
the privacy page and the submission files. The app connects ChatGPT to the
public emem MCP server at `https://emem.dev/mcp`. Reads need no key, no
account and no signup.

emem is shared, verifiable memory for AI agents. Each place has one address,
and each observation is one signed fact. A user gets a measurement with an
ed25519 receipt, and anyone can check that receipt offline. Earth observation
data fills this memory today.

| You are | Read |
|---|---|
| A ChatGPT user | [Use the app](#use-the-app) |
| A developer who wants the same tools in another MCP client | [Try the endpoint](#try-the-endpoint) |
| A Custom GPT builder | [Custom GPT Actions](#custom-gpt-actions) |
| An OpenAI reviewer | [submission.md](submission.md), [app.md](app.md), [privacy.md](privacy.md) |

## What ships and what does not

| Item | State on 7 November 2026 |
|---|---|
| MCP endpoint `https://emem.dev/mcp` | Live. Serves 18 tools. |
| Action schema `https://emem.dev/openapi.action.json` | Live. OpenAPI 3.1.0, 38 paths. |
| Listing in the ChatGPT app directory | Depends on OpenAI review. This repository records no approval. |

## Try the endpoint

This command lists the 18 tools that `/mcp` serves. It needs only `curl`
and `python3`.

```sh
curl -s -X POST https://emem.dev/mcp \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H 'MCP-Protocol-Version: 2025-11-25' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' \
  | python3 -c 'import json,sys; [print(t["name"]) for t in json.load(sys.stdin)["result"]["tools"]]'
```

The first lines of output on 7 November 2026:

```text
emem_entity
emem_locate
emem_recall
emem_memory_token
```

To use the tools from an MCP client, add this server entry:

```json
{
  "mcpServers": {
    "emem": {
      "type": "http",
      "url": "https://emem.dev/mcp"
    }
  }
}
```

## Use the app

After OpenAI approves the listing, open Settings, then Apps. Find **emem**
and turn it on. Then ask a question that names a real place.

Questions the app can answer:

- What is the elevation here, and what is the evidence?
- Has this site flooded, and how can I check that claim?
- Did surface water or vegetation change between these two dates?
- Which places look like this one?
- Give me a citation that another agent can resolve.

Example prompts:

- What signed facts are available for South Mumbai?
- Is Helsinki-Vantaa Airport low-lying?
- What evidence supports that answer, and can I verify it offline?
- Compare vegetation at this field between 2023 and 2025.
- Find places similar to Lake Erie.

## How the app answers

The app talks to the MCP server over Streamable HTTP. When a question names a
real place, ChatGPT calls emem to do three things:

1. Resolve the place to one canonical address.
2. Read the signed facts at that address.
3. Return a short citation token for each fact.

The token is about 83 characters long. It stays the same through a summary or
a model change. `emem_memory_token_resolve` returns the same signed bytes for
it in any session, for any agent.

## Tools

[tools.md](tools.md) lists the 18 tools with their inputs and MCP annotations.
`scripts/gen_chatgpt_bundle.py` writes that file from the live catalogue.
Do not edit it by hand.

The live annotations on 7 November 2026 put the tools in three groups:

| Group | Tools | What they change |
|---|---|---|
| Read-only (9) | `emem_locate`, `emem_memory_token`, `emem_memory_token_resolve`, `emem_verify_receipt`, `emem_memory_contradictions`, `emem_guard_verdict`, `emem_tools`, `emem_echo_verify`, `emem_entity_resolve` | Nothing. |
| Can sign new facts (7) | `emem_ask`, `emem_recall`, `emem_find_similar`, `emem_intent`, `emem_memory_bundle`, `search`, `fetch` | On a cold cell, they fetch public Earth observation data and sign new facts. |
| Gated writes (2) | `emem_entity`, `emem_entity_link` | They mint or bind an identity in the shared address space. |

`search` and `fetch` call `emem_recall` under the two names that a ChatGPT
connector looks for.

The two gated writes need an ed25519 attester block from a key at tier T3.
Without one, the REST route refuses with HTTP 403 and returns the digest to
sign. Over MCP, the same refusal comes back as a tool error.
A ChatGPT user has no such key, so in this app these tools refuse. The ladder
of tiers is at `https://emem.dev/v1/enlist`.

No tool stores what a user sends. New facts come only from public Earth
observation sources.

Note: the text at the top of [tools.md](tools.md) says that no write verb is
exposed. That is not correct for `emem_entity` and `emem_entity_link`. The
generator script writes that sentence, so the fix belongs in the script.

## Custom GPT Actions

For Custom GPT Actions instead of MCP, import
`https://emem.dev/openapi.action.json`. Do not import `/openapi.json`.
That file has every route of the server, which is too large for a Custom GPT.
On 7 November 2026 the action schema was 126,810 bytes and the full file was
405,695 bytes.

## Files in this folder

| File | Contents |
|---|---|
| [app.md](app.md) | Listing text: name, descriptions, category, transport, authentication |
| [submission.md](submission.md) | Notes for the OpenAI reviewer, and the domain check steps |
| [chatgpt-app-submission.json](chatgpt-app-submission.json) | Submission data, checked by [submission.schema.json](submission.schema.json) |
| [tools.md](tools.md) | The 18 tools, generated from the live catalogue |
| [tool-annotations.md](tool-annotations.md) | Why each tool has its MCP annotations |
| [privacy.md](privacy.md) | What the app sends and what the server keeps |

## Links

- Homepage: https://emem.dev
- Repository: https://github.com/Vortx-AI/emem
- Agent card (A2A): https://emem.dev/.well-known/agent-card.json
- Verify an answer: https://emem.dev/verify
- Privacy: [privacy.md](privacy.md) and https://emem.dev/privacy
- Support: https://emem.dev/support, avijeet@vortx.ai
