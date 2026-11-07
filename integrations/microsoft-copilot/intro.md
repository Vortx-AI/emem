# emem, verifiable memory for AI agents

emem gives every place one address and every observation one signed fact.
Ask about a place, and the answer is a measurement from a public
Earth-observation source. The answer has an Ed25519 receipt. It also has a
short `emem:fact:` token, which resolves anywhere to the same signed bytes.
Anyone can check the receipt at https://emem.dev/verify and does not have to trust emem.

## Setup

Add the connector URL `https://emem.dev/mcp`. Reads need no key and no
account. emem limits the rate of reads for each client. A client over the
limit gets HTTP 429 with a `Retry-After` header.

## Example prompts

- "What is the vegetation index near Mount Fuji right now? Give me a receipt I can cite."
- "Was there forest loss after 2020 at -3.47, -62.22?"
- "What is the elevation at 28.61, 77.21?"

## What the tools do

The connector serves the 18 core tools of emem. They make a loop:

| step | tools |
|---|---|
| Name a thing | `emem_entity` |
| Find the address of a place | `emem_locate` |
| Read signed facts | `emem_recall`, `emem_ask`, `emem_intent` |
| Cite facts | `emem_memory_token`, `emem_memory_bundle` |
| Verify facts | `emem_verify_receipt` |

`emem_tools` lists the other tools of the server. A client calls any of them by
name with `tools/call`.

Nine core tools only read data. Seven more can measure and sign a new fact
when nobody has made the reading yet: `emem_recall`, `emem_ask`,
`emem_intent`, `emem_find_similar`, `emem_memory_bundle`, `search` and
`fetch`. For this reason, their `readOnlyHint` is false. What they write comes
from public Earth-observation sources, never from what the user sent. They
never overwrite a fact.

## Authentication

Reads need no authentication. Two tools write shared identities:
`emem_entity` and `emem_entity_link`. They need an attester block that the
caller signs with their own Ed25519 key. Without that block, the REST route
refuses with HTTP 403, and the error names the missing field. Over MCP, the
same refusal comes back as a tool error. The connector does not send
or store a user identity.

## Known limitations

- A cell is about 10 m across. A finer query resolves to the cell that contains it.
- When nobody has made a reading yet, emem fetches it from its upstream source
  on the first request. That can take some seconds. emem serves later requests
  from its signed store.
- When an upstream source has no reading, emem returns a signed absence, never
  a value.

## Support

- Homepage: https://emem.dev
- Docs: https://emem.dev/agents.md
- Source: https://github.com/Vortx-AI/emem
- Email: avijeet@vortx.ai
