# emem Security Policy

_Last updated: 2026-05-03_

emem is an Apache-2.0, pure-Rust, content-addressed protocol. The
canonical responder is operated by **Vortx AI Private Limited** (India)
at `https://emem.dev` (mirrored at `https://vortx-ai-emem.hf.space`).
L0 and L1 read endpoints require no API keys and no accounts; this
policy covers the protocol implementation and the hosted instance. See
also [Privacy](/privacy) and [Terms](/terms).

## Reporting a vulnerability

If you've found a security issue in emem (the protocol or this
implementation), please email **avijeet@vortx.ai** rather than opening a
public GitHub issue. We'll acknowledge within **72 hours**, work with you
on an embargoed fix, and credit you (with permission) in the release
notes.

For non-sensitive reports — design weaknesses, hardening opportunities,
spec ambiguities — opening a public issue is welcome.

## Safe harbor

We will not pursue legal action or law-enforcement referral against
researchers who, in good faith:

- Test only against accounts/cells/keys you own, or against the public
  hosted instance with reasonable rate (no DoS, no sustained traffic
  intended to degrade service for others).
- Avoid accessing, modifying, or destroying data that does not belong to
  you. The corpus itself is public-by-design; our concern is the
  responder's identity key and operational integrity.
- Give us reasonable time (see disclosure timeline below) to remediate
  before public disclosure.
- Do not exfiltrate more data than is necessary to demonstrate the
  issue.

Activity consistent with this policy is authorised, and we consider it
to be lawful good-faith research.

## Scope

- emem responder (`emem-server` and the underlying crates in this repo)
- emem protocol itself (signatures, merkle log, content addressing)
- Default-build data fetch paths (vsicurl Range reads, Nominatim geocoder)
- Our hosted instance at `https://emem.dev`

Out of scope:

- Third-party MCP / IDE clients (Claude Desktop, Cursor, Cline, etc.)
- Operator-registered upstream connectors that aren't in the default build
- Bugs in cargo dependencies — please report those upstream and link the
  CVE/issue here
- The correctness, availability, or security of upstream open-data
  providers we fetch from (Open-Meteo, MET Norway, GMRT, Copernicus DEM,
  JRC GSW, Hansen GFC, ESA WorldCover, OSM/Overture, NASA/USGS, etc.).
  emem re-signs their payloads under the responder identity so the
  fetch is auditable, but we do not own their security posture — report
  upstream issues to those projects directly.

## Hardening already in place

| concern                  | mitigation                                                  |
|--------------------------|-------------------------------------------------------------|
| TLS                      | rustls 1.2/1.3, modern ciphers only, Let's Encrypt cert     |
| HSTS                     | `max-age=31536000; includeSubDomains; preload`              |
| CSP                      | locked-down default-src 'self' + GA4 origin                 |
| Other headers            | X-Content-Type-Options, X-Frame-Options, Referrer-Policy, Permissions-Policy |
| Body cap                 | 16 MiB on POST endpoints (413 on overflow)                  |
| Request timeout          | 30 s (504 on overflow)                                      |
| Per-IP rate limit        | 60 req/min, 120 burst, `Retry-After: 60`                    |
| Identity                 | ed25519 secret stored mode 0600, never logged               |
| Receipts                 | every read signed; offline-verifiable via /v1/verify_receipt |
| Storage                  | sled with content-addressed keys, no SQL                     |
| `unsafe` code            | `#![forbid(unsafe_code)]` in api-rest                        |

## Cryptographic invariants we'll patch immediately

If any of the following ever fail to hold for a release of emem, that's a
high-severity bug:

1. Two parties computing CBOR canonicalisation over the same logical
   `Fact` produce identical 32-byte CIDs.
2. `merkle_root(canonical_sort(leaves))` is order-stable; any leaf order
   produces the same root.
3. ed25519 receipt signatures verify offline against the embedded
   `responder` pubkey using the documented preimage.
4. The append-only merkle log replays bit-for-bit after a clean restart.
5. The persisted ed25519 secret at `<EMEM_DATA>/identity.secret.b32`
   has mode `0600` and never appears in stdout/journal.

## Disclosure timeline

We aim for 90 days from initial report to public fix + advisory, with the
embargo shortened if the fix is non-controversial or extended (with your
agreement) if coordination across deployers takes longer.
