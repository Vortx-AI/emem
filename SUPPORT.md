# Support

_Last updated: 2026-05-14_

The hosted instance at `https://emem.dev` is operated by **Vortx AI
Private Limited** (India). All correspondence routes to
**avijeet@vortx.ai**.

## Bugs and feature requests

Open a GitHub issue: <https://github.com/Vortx-AI/emem/issues>

Please include:

- The endpoint or MCP tool you called.
- The exact request body.
- The full response (including `traceparent` if present — that lets us
  correlate to server-side logs).
- The protocol version (`/v1/manifests` returns the active CIDs).

## Security disclosures

Do **not** open public GitHub issues for security vulnerabilities. Email
**avijeet@vortx.ai** directly. See [SECURITY.md](SECURITY.md) for the
disclosure timeline (acknowledgement within 72 hours, embargoed fix,
credit on request).

## MCP / Claude integration questions

GitHub Discussions, or email **avijeet@vortx.ai**.

## Status

- Liveness probe: `GET https://emem.dev/health` (200 = up).
- Prometheus metrics: `GET https://emem.dev/metrics`.
- Manifest CIDs / responder pubkey: `GET https://emem.dev/.well-known/emem.json`.
- Per-band freshness + history bounds: `GET https://emem.dev/v1/coverage_matrix`.
- Live tool inventory: `GET https://emem.dev/v1/tools` (with annotations).

## Self-hosting

See [docs/DEPLOY.md](docs/DEPLOY.md). The server runs as a single Rust
binary (`emem-server`) or via the published OCI image
(`ghcr.io/vortx-ai/emem:latest`). No external dependencies beyond the
upstream open-data providers listed in [PRIVACY.md](PRIVACY.md).
