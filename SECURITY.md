# emem Security Policy

_Last updated: 2026-11-07_

emem is an Apache-2.0, pure-Rust, content-addressed protocol. Vortx AI
Private Limited (India) operates the canonical responder at
`https://emem.dev`. A mirror runs at `https://vortx-ai-emem.hf.space`.
The L0 and L1 read endpoints need no API key and no account. This policy
covers the protocol code and the hosted instance. See also
[Privacy](/privacy) and [Terms](/terms).

For the trust model itself (what a receipt proves, the transparency log,
witnesses), read [docs/security.md](docs/security.md) and the
[threat model](docs/security/threat-model.md).

## Report a vulnerability

To report a security issue in the protocol or in this code, use
one of these two channels:

- Send an email to **avijeet@vortx.ai**.
- Open a [GitHub security advisory](https://github.com/Vortx-AI/emem/security/advisories/new).

[`/.well-known/security.txt`](https://emem.dev/.well-known/security.txt)
lists both as `Contact:`. Do not open a public issue for a vulnerability.
We acknowledge each report within **72 hours**. We work with you on an
embargoed fix. With your permission, we credit you in the release notes.

A report that is not sensitive can go in a public issue. Examples are a
design weakness, a hardening idea or an ambiguity in the spec.

`security.txt` has no `Encryption:` field on purpose. RFC 9116 expects
that field to name an encryption key. The only key this responder
publishes is the ed25519 signing key at `/.well-known/emem.json`. ed25519
is a signature scheme and cannot encrypt, so the field would offer
something that does not exist. To send sensitive material, say so in a
first email and we agree on a channel.

## Supported versions

| Version | Supported |
|---|---|
| 2.4.x | Yes. Current. Fixes land here. |
| <= 2.3.x | No. Superseded. Upgrade before you report against these. |

There is one canonical responder, and it runs the tip of `main`. Thus the
hosted instance at `https://emem.dev` is always the supported version.

A node states its version and exact commit in the signed
`operator_attestation` block of
[`/.well-known/emem.json`](https://emem.dev/.well-known/emem.json). The
signed fields include `version`, `git_commit`, `build_timestamp` and
`binary_blake3`, the BLAKE3 hash of the running executable. Thus you can
see what a node runs without trust in a tag. On 7 November 2026, emem.dev
reported version `2.4.4` there. Registry listings pin older versions by
design. A listing is not a statement of support.

## Disclosure timeline

Our target is 90 days from the first report to a public fix and advisory.
We shorten the embargo when the fix is simple. With your agreement, we
extend it when deployers need more time to coordinate.

## Safe harbor

We do not take legal action, or refer to law enforcement, a researcher
who acts in good faith and obeys these rules:

- Test only against accounts, cells and keys that you own, or against the
  public hosted instance at a reasonable rate. Do not cause a denial of
  service. Do not send sustained traffic that degrades the service for
  others.
- Do not access, change or destroy data that is not yours. The corpus is
  public by design. Our concern is the responder's identity key and its
  operational integrity.
- Give us a reasonable time to fix the issue before public disclosure
  (see [Disclosure timeline](#disclosure-timeline)).
- Do not copy out more data than you need to show the issue.

We authorise activity that obeys this policy, and we consider it lawful
good-faith research.

## Scope

In scope:

- The emem responder (`emem-server` and the crates in this repository).
- The emem protocol (signatures, Merkle log, content addressing).
- The data fetch paths in the default build: HTTP Range reads of
  Cloud-Optimized GeoTIFFs, and the Nominatim geocoder.
- The hosted instance at `https://emem.dev`.

Out of scope:

- Third-party MCP and IDE clients (Claude Desktop, Cursor, Cline and
  others).
- Upstream connectors that an operator registers and that are not in the
  default build.
- Bugs in cargo dependencies. Report these upstream, then link the CVE or
  the issue here.
- The upstream open-data providers and their correctness, availability
  and security. These include Open-Meteo, MET Norway, GMRT, Copernicus DEM, JRC GSW,
  Hansen GFC, ESA WorldCover, OSM, Overture, NASA, USGS and others. emem
  signs their payloads under the responder identity, so each fetch is
  auditable. We do not control their security. Report their issues to
  those projects.

## Protections in place

These values are the defaults in the code. An operator can change the
ones that name an environment variable. The live responder declares its
current limits under `runtime` at
[`/v1/agent_card`](https://emem.dev/v1/agent_card).
`scripts/sync_counts.py` compares the limits in this table with that
block.

| Concern | Mitigation |
|---|---|
| TLS | TLS 1.2 and 1.3 only, modern ciphers, an ACME certificate. On 7 November 2026, emem.dev used TLS 1.3 and the issuer was `Let's Encrypt`. |
| HSTS | `max-age=31536000; includeSubDomains; preload` |
| CSP | `default-src 'self'`. Scripts need a hash or an allowed origin; scripts get no `unsafe-inline`. The live allow list also names `esm.sh` and `cdn.redocly.com` for scripts and `fonts.googleapis.com` for styles. The list names no analytics origin. |
| Other headers | `X-Content-Type-Options`, `Referrer-Policy`, `Permissions-Policy`, and the CSP directives `frame-ancestors`, `base-uri` and `form-action`. CSP controls framing, not `X-Frame-Options`. |
| Body cap | 16 MiB on POST endpoints (413 when larger). `EMEM_BODY_LIMIT_MB`. |
| Request timeout | 40 s (504 when longer). `EMEM_TIMEOUT_SECS`. |
| Load shedding | 128 requests in flight across the process. The next request gets a fast 503. `EMEM_MAX_INFLIGHT`. |
| Per-IP rate limit | 600 req/min sustained, 120 burst, `Retry-After: 1`. `EMEM_RATE_LIMIT_RPS` and `EMEM_RATE_LIMIT_BURST`. |
| Identity | The responder keeps its ed25519 secret in a file with mode 0600 and never writes it to a log. |
| Receipts | Each fact read carries a signed receipt. You can verify it offline, or with `/v1/verify_receipt`. |
| Storage | sled with content-addressed keys. No SQL. |
| `unsafe` code | `#![forbid(unsafe_code)]` in every crate except `emem-airgap`, `emem-scorecard` and `emem-sleep-agent`. Those three use no `unsafe`. |

## Cryptographic invariants we patch at once

If one of these invariants fails in a release of emem, that is a
high-severity bug:

1. Two parties that compute canonical CBOR over the same logical `Fact`
   get the same 32-byte CID.
2. `merkle_root(canonical_sort(leaves))` is independent of order. Each
   order of the same leaves gives the same root.
3. An ed25519 receipt signature verifies offline against the embedded
   `responder` public key with the documented preimage.
4. The append-only Merkle log replays bit for bit after a clean restart.
5. The stored ed25519 secret at `<EMEM_DATA>/identity.secret.b32` has
   mode `0600` and never appears in stdout or in the journal.
