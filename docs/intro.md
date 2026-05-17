# emem.dev — Earth memory protocol

Signed, cell-addressed Earth memory. This site renders the canonical docs straight from the repo at `docs/`. Every page here ships in the same signed server binary as the rest of `web/` — there is no separate docs host.

## Where to start

- **Whitepaper** — math and architecture overview
- **Protocol** — wire format, cell64, cid64, tslot, receipts
- **Registries** — bands (41), algorithms (159), functions, sources (43), topics (27), schema, lcv‑1, alphabet
- **Agents** — how AI agents discover and use the protocol (MCP + REST + OpenAPI)
- **Developers / Operators** — build, run, deploy

## Conventions

- *Receipts* are ed25519‑signed over a deterministic preimage and verifiable in‑browser at `/verify`.
- *No silent fallbacks*: an empty result distinguishes wrong‑query from empty‑place.
- *No stubs*: nothing here is aspirational — if a primitive is documented, it ships.

## Discoverability

The same content surfaces are reachable to agents via:

- `GET /openapi.json` — full REST surface (browseable at [/docs/api/](/docs/api/) via ReDoc)
- `GET /llms.txt`, `GET /humans/llms.txt`, `GET /skills.md`
- `GET /agent.json`, `GET /ai-plugin.json`
- `GET /sitemap.xml`
- MCP tools listed at `/agents` and the MCP directory
