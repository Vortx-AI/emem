# App Metadata

## Name

emem

## Short description

Shared, verifiable memory for AI agents.

## Long description

emem is a shared, verifiable memory for AI agents. Every place resolves to one
canonical address. Every observation about a place becomes one signed fact
that verifies offline. Every fact has a short citation token. The token
resolves anywhere to the byte-identical signed value. Two agents with no
shared model or vendor can cite the same fact. Each agent can check it alone.
Neither agent has to trust the other or the responder.

In ChatGPT, a place-based answer arrives as a measurement with a receipt, not
as a plausible sentence. The app gives:

- point values such as elevation, vegetation, surface water, built-up context
  and flood signals
- an area as a native-resolution field, or as a field over time
- change between dates
- similar places, from an embedding index that is frozen on emem.dev
- an ed25519 receipt that you check against a published key, not against our
  word

Today, Earth observation fills this memory. Satellites and sensors already
measure the world, and anyone can recompute their output from the cited
source. Any data that can prove how it came to be can go in the same way.

Transport: MCP over Streamable HTTP, or Custom GPT Actions through the action
schema. Reads need no API key, no account and no signup.

## Category

DEVELOPER_TOOLS

(The submission JSON is the authority for this field. This page repeats it so
that the two agree. Domains served: science, research, geospatial, climate and
compliance evidence.)

## Publisher

Vortx AI Private Limited (https://vortx.ai)

## Homepage

https://emem.dev

## Repository

https://github.com/Vortx-AI/emem

## License

Apache-2.0

## MCP endpoint

https://emem.dev/mcp

## Action schema (Custom GPT Actions)

https://emem.dev/openapi.action.json

Import this file, not `/openapi.json`. The full OpenAPI document has every
route of the responder. It is far too large for a Custom GPT Action.

## Transport

Streamable HTTP. The server prefers MCP **2025-11-25**. It also accepts
2025-06-18, 2025-03-26 and 2024-11-05. If a request sends no
`MCP-Protocol-Version` header, the server uses 2025-03-26 as a fallback.

## Authentication

None. Reads are anonymous: no key, no account, no callback. The OpenAPI
document has no `securitySchemes`. That absence is the machine-readable form
of this statement.

The app exposes two write tools, `emem_entity` and `emem_entity_link`. They
change the shared address space. Thus each call needs an ed25519 attester
block, signed with a keypair that the caller makes locally. The caller must
also hold tier T3 (declared) on the enlistment ladder at
https://emem.dev/v1/enlist.

## Contact

https://emem.dev/support, avijeet@vortx.ai
