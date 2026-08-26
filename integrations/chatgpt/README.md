# emem for ChatGPT

**emem is shared, verifiable memory for AI agents.** Two agents that share no
model, no vendor and no trust can cite the same signed fact and each check it
alone. Ask a model the same thing twice and you get two answers; ask emem twice
and the same signed bytes come back, with a receipt you can verify without
trusting the responder that sent it.

Earth is what fills that memory today, because satellites and sensors already
measure the world and their output can be recomputed from the cited source. It
is the substrate, not the subject.

No key. No account. No signup.

```
https://emem.dev/mcp
```

## What a user can ask

- What is the elevation here, and what is the evidence?
- Has this site flooded, and how would I check that claim?
- Has surface water or vegetation changed between these two dates?
- Find places that resemble this one.
- Give me a citation I can hand to another agent and they can resolve.

## How it works

The app connects to emem's public MCP endpoint over Streamable HTTP. When a
question names a real place, ChatGPT calls emem to resolve the place to one
canonical address, read the signed facts there, and return a short citation
token that resolves anywhere to the byte-identical signed value.

The token is the part that matters. It is about 50 characters, it survives
summarisation and a model swap, and `emem_memory_token_resolve` returns the
same bytes in another session or in another agent's session.

## Tools

The app exposes **9 tools**, listed with their inputs and their MCP annotations
in [tools.md](tools.md), which is GENERATED from the catalogue the responder
actually serves. Seven are strictly read-only; `emem_ask` and `emem_recall` can
materialise and sign new facts on a cold cell, which is why their
`readOnlyHint` is false, and they say so in their own justification. None of
emem's write verbs is exposed here.

## Example prompts

- What signed facts are available for South Mumbai?
- Is Helsinki-Vantaa Airport low-lying?
- What evidence supports that answer, and can I verify it offline?
- Compare vegetation at this field between 2023 and 2025.
- Find places similar to Lake Erie.

## Setup

### For users

Once published to the ChatGPT app directory: Settings, Apps, find **emem**,
enable it. Then ask a question that names a real place.

### For developers testing now

Point any MCP client at the same endpoint:

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

For Custom GPT Actions instead of MCP, import
[`https://emem.dev/openapi.action.json`](https://emem.dev/openapi.action.json).
Import that one and not `/openapi.json`: the full document carries every route
this responder serves and is far past what a Custom GPT can hold.

## Links

- Homepage: https://emem.dev
- Repository: https://github.com/Vortx-AI/emem
- MCP endpoint: https://emem.dev/mcp
- Action schema: https://emem.dev/openapi.action.json
- Agent card (A2A): https://emem.dev/.well-known/agent-card.json
- Verify any answer: https://emem.dev/verify
- Privacy: [privacy.md](privacy.md) and https://emem.dev/privacy
- Support: https://emem.dev/support, avijeet@vortx.ai
