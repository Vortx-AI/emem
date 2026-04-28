# emem Privacy Policy

_Last updated: 2026-04-28_

emem is an open, content-addressed protocol that returns signed facts about
geographic cells. This document describes the data the **canonical responder**
operated by Vortx-AI at `https://emem.dev` (and mirrored at
`https://vortx-ai-emem.hf.space`) collects, processes, and retains. Self-hosted
emem deployments are governed by their own operators and are out of scope.

## Tl;dr

- **No accounts. No keys. No PII.** L0 and L1 read endpoints are anonymous.
- We do not collect, store, or sell user data.
- We do not run third-party analytics that profile visitors.
- We log standard request metadata (timestamp, IP, user-agent, path, status,
  duration) for operational health, with a 30-day rolling retention.

## What we collect

| Surface | Data | Purpose | Retention |
|---|---|---|---|
| `GET /…`, `POST /v1/*`, `POST /mcp` | Request method, path, status, duration, response size, originating IP, user-agent header | Server health, abuse mitigation, capacity planning | 30 days, then deleted |
| `POST /v1/attest`, `POST /v1/attest_cbor` | The signed attestation payload itself: ed25519 attester pubkey, fact CIDs, Merkle root, attestation timestamp | Persisted to the public, content-addressed corpus by design — that is the whole protocol | Indefinite (the corpus is a public ledger) |
| `POST /v1/recall*`, `POST /v1/intent`, `POST /v1/locate` | Request body (cell, place name, bands) | Used in-memory only to compute the response; not associated with the requesting IP | Not persisted beyond the request |
| Auto-materialized facts | Upstream provider response (Copernicus DEM, JRC GSW, Hansen, ESA WorldCover, OSM/Overture, Open-Meteo, …) re-signed under the responder's identity | Becomes part of the public corpus once attested | Indefinite |

**We never log:**
- Free-text questions sent to `emem_ask` or `emem_intent`
- Per-user query histories
- Cookies, fingerprints, or device identifiers (we set none)

## What we do NOT collect

- No conversation context from your MCP host
- No data from other tools, files, or memory of your AI agent
- No location data beyond what you explicitly include in a request
- No payment information (the public responder is free for L0/L1)

## Third parties

When a request triggers auto-materialization, the responder fetches data from
public open-data providers — these requests are made _by the emem responder_,
not by you, and your IP is not forwarded:

- Copernicus Data Space Ecosystem (Sentinel-1, Sentinel-2, Cop-DEM)
- JRC Global Surface Water (`storage.googleapis.com/global-surface-water`)
- Hansen Global Forest Change (`storage.googleapis.com/earthenginepartners-hansen`)
- ESA WorldCover (`esa-worldcover.s3.amazonaws.com`)
- Overture Maps (`overturemaps-us-west-2.s3.amazonaws.com`)
- OpenStreetMap (`overpass-api.de`, `nominatim.openstreetmap.org`)
- Open-Meteo (`api.open-meteo.com`)

Each provider has its own privacy policy; their licences are surfaced via
`GET /v1/sources`.

## Receipts and signatures

Every response includes a signed receipt: the responder's ed25519 public key,
the request canonicalisation hash, and the fact CIDs. The receipt does **not**
contain user identifiers. You can verify any receipt offline using the public
key at `/.well-known/emem.json`.

## Children

emem returns geographic facts; it has no concept of user accounts and is not
directed at children under 13.

## Changes

We may revise this policy as the protocol evolves. The canonical version is
the file `PRIVACY.md` in [github.com/Vortx-AI/emem](https://github.com/Vortx-AI/emem);
the live HTTPS rendering is at `https://emem.dev/privacy`.

## Contact

- Issues, bugs, security: <https://github.com/Vortx-AI/emem/issues>
- Privacy enquiries: **avijeet@vortx.ai**
