# Security Policy

## Reporting a vulnerability

If you've found a security issue in emem (the protocol or this
implementation), please email **avijeet@vortx.ai** rather than opening a
public GitHub issue. We'll acknowledge within 72 hours, work with you on
an embargoed fix, and credit you (with permission) in the release notes.

For non-sensitive reports — design weaknesses, hardening opportunities,
spec ambiguities — opening a public issue is welcome.

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
