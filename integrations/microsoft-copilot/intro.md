# emem, verifiable memory for AI agents

emem gives every place one address and every observation one signed fact.
Ask about a place and the answer comes back as a measurement from a public
Earth-observation source, with an Ed25519 receipt attached and a short
`emem:fact:` token that resolves anywhere to the same signed bytes. Anyone can
check the receipt at https://emem.dev/verify without trusting emem.

## Setup

Add the connector URL `https://emem.dev/mcp`. Reading needs no key and no
account. Reads are rate-limited per client; a client over the limit gets
HTTP 429 with a `Retry-After` header.

## Example prompts

- "What is the vegetation index near Mount Fuji right now? Give me a receipt I can cite."
- "Was there forest loss after 2020 at -3.47, -62.22?"
- "What is the elevation at 28.61, 77.21?"

## What the tools do

The connector serves emem's 18-tool core loop: name a thing (`emem_entity`),
ground a place (`emem_locate`), read signed facts (`emem_recall`, `emem_ask`,
`emem_intent`), cite them (`emem_memory_token`, `emem_memory_bundle`) and
verify them (`emem_verify_receipt`). `emem_tools` lists the rest of the
server's tools and runs any of them by name.

Nine tools only look things up. Seven more (`emem_recall`, `emem_ask`,
`emem_intent`, `emem_find_similar`, `emem_memory_bundle`, `search`, `fetch`)
can measure and sign a new fact when the requested reading has not been made
yet, which is why their `readOnlyHint` is false. What they write comes from
public Earth-observation sources, never from what the user sent, and nothing is
ever overwritten.

## Authentication

None for reads. The two tools that write shared identities (`emem_entity`,
`emem_entity_link`) need an attester block signed with the caller's own
Ed25519 key; without one they decline with a 403 that names the missing
field. No user identity is sent or stored.

## Known limitations

- A cell is about 10 m across; finer queries resolve to the containing cell.
- A reading that has not been made yet is fetched from its upstream on first
  request and can take seconds; repeats are served from the signed store.
- A missing upstream reading is returned as a signed absence, never as a value.

## Support

- Homepage: https://emem.dev
- Docs: https://emem.dev/agents.md
- Source: https://github.com/Vortx-AI/emem
- Email: avijeet@vortx.ai
