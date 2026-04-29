# Changelog

All notable changes to the emem reference implementation are recorded
here. The format follows [Keep a Changelog](https://keepachangelog.com/)
and we use [Semantic Versioning](https://semver.org/) once we're past
0.1.

## [Unreleased]

### Added
- `PRIVACY.md` "Your rights" section enumerating GDPR / CCPA / CPRA data-
  subject rights (access, erasure, rectification, objection, opt-out of
  sale/sharing, non-discrimination) and how to exercise them.
- Native HTTPS via in-process rustls + Let's Encrypt (TLS-ALPN-01).
- `/v1/locate` (lat/lng or place name → cell64; OSM Nominatim under the
  hood for place-name lookup).
- `/v1/cells/{cell64}/info` (cell64 → centre + bbox + approx size).
- `/v1/discover` (one-call agent bootstrap: agent_card + manifests +
  canonical places + next-call hints).
- `/api` — 308 redirect to `/v1/agent_card`.
- `/v1/contributors` and `/v1/contributors/{pubkey_b32}` — the
  Contributor-of-Intelligence Layer (CoIL) leaderboard.
- `/metrics` — Prometheus text-format counters.
- `/llms-full.txt` — the comprehensive single-call agent context dump.
- `/examples/agent-walkthroughs.md` — 8 worked end-to-end queries.
- Production middleware: 16 MiB body cap, 30 s timeout, per-IP token-
  bucket rate limit (60/min, 120 burst), HSTS / CSP / X-Content-Type-
  Options / X-Frame-Options / Referrer-Policy / Permissions-Policy,
  optional HTTP→HTTPS redirect via `EMEM_REDIRECT_HTTPS=1`,
  graceful shutdown on SIGTERM.
- Persistent ed25519 responder identity at
  `<EMEM_DATA>/identity.secret.b32` (mode 0600).
- `emem-livedemo` and `emem-realdemo` CLI binaries with full request +
  response + receipt traceability written to `var/demos/`.
- Daily systemd timer (`emem-daily-delta.timer`) capturing contributor
  + metrics + realdemo trace at 03:17 UTC.
- SEO surface: Open Graph + Twitter card meta, geo / ICBM / DC.coverage
  meta, JSON-LD `SoftwareApplication` + `Organization` + `WebSite`,
  GA4 (`G-RBLXX5LR9L`), favicon, OG image, IndexNow key endpoint,
  `/.well-known/security.txt`.

### Changed
- Cell64 codec now exposes a stable `cell_from_latlng` / `latlng_from_cell64`
  pair in `emem-codec::geo`, with a documented bit layout
  (`mode|res|base|hilbert_d`).
- `emem-realdemo` uses the canonical codec — its attested cells now
  match the cells `/v1/locate` returns for the same coordinates.
- Curl examples in `web/index.html` and `web/llms.txt` now reference a
  real, locatable cell that returns real Cop-DEM provenance facts.
- `serve_llms_full` actually serves a comprehensive LLM-targeted text
  rather than the whitepaper.

### Removed
- The third-party `r.jina.ai` external-probe dependency. We use
  `curl --resolve` for direct external connectivity tests now.

## [0.0.2] — 2026-04-26

Initial open-source release. The protocol surface, primitives, MCP
server, and reference responder are all functional. See README.md for
the workspace layout and DEPLOY.md for production deployment.
