# App Metadata

## Name

emem

## Short description

Shared, verifiable memory for AI agents.

## Long description

emem is a shared, verifiable memory for AI agents. Every place resolves to one
canonical address, every observation about it becomes one signed fact that
verifies offline, and every fact has a short citation token that resolves
anywhere to the byte-identical signed value. Two agents that share no model and
no vendor can cite the same fact and each check it alone, without trusting each
other or the responder.

In ChatGPT that means a place-based answer arrives as a measurement with a
receipt attached rather than as a plausible sentence: elevation, vegetation,
surface water, built-up context, flood signals; an area read as a
native-resolution field or a field over time; change compared across dates;
similar places found by embedding; and an ed25519 receipt that can be checked
against a published key rather than against our word.

Earth is the substrate that fills this memory today, because satellites and
sensors already measure the world and their output can be recomputed from the
cited source. Anything that can prove how it was produced can be written down
the same way.

Transport: MCP over Streamable HTTP, or Custom GPT Actions via the action
schema. No API key, no account, no signup for reads.

## Category

DEVELOPER_TOOLS

(The submission JSON is the authority on this field; it is repeated here so the
two cannot disagree. Domains served: science, research, geospatial, climate and
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

Import this one, not `/openapi.json`. The full OpenAPI document carries every
route this responder serves and is far past what a Custom GPT Action can hold.

## Transport

Streamable HTTP. The server negotiates MCP **2025-11-25** and also accepts
2025-06-18, 2025-03-26 and 2024-11-05. A request that sends no
`MCP-Protocol-Version` header is read as 2025-03-26, which is a fallback and
not the version this server prefers.

## Authentication

None. Reads are anonymous: no key, no account, no callback. The absence of
`securitySchemes` in the OpenAPI document is the machine-readable form of that
statement.

Writes are a separate surface and are NOT exposed in this app. They require an
ed25519 attester block signed by a keypair the caller generates locally, and
they are gated by an enlistment ladder described at
https://emem.dev/v1/enlist.

## Contact

https://emem.dev/support, avijeet@vortx.ai
