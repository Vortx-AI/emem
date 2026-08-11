# ememdev — TypeScript client for emem.dev

Thin, typed TypeScript client for the [emem.dev](https://emem.dev) Earth
memory protocol. Wraps the public REST surface in a single `Client` class.
Every call returns parsed JSON verbatim, so every ed25519-signed receipt and
content-addressed CID is preserved for citation and offline verification.
The current surface is whatever [`/openapi.json`](https://emem.dev/openapi.json)
lists; this README does not restate a count that would go stale.

Runs on Node 18+, Bun, Deno, browsers, and edge runtimes. Uses the
platform `fetch`; zero runtime dependencies.

## Install

Not on NPM yet. Install from the repo:

```bash
cd sdks/emem-ts && npm install && npm run build
# then add as a local dependency in your project
```

## Quick start

```ts
import { Client } from "@vortxai/emem";

const em = new Client();
const located = await em.locate({ place: "Mount Fuji" });
const facts = await em.recall({
  cell: (located as any).cell64,
  bands: ["copdem30m.elevation_mean"],
});
console.log(facts);
```

## Configuration

| Option / env var                    | Default              | Effect                                |
|-------------------------------------|----------------------|---------------------------------------|
| `baseUrl` / `EMEM_BASE_URL`         | `https://emem.dev`   | Responder root (point at self-hosted) |
| `timeoutMs` / `EMEM_TIMEOUT_SECS`   | `180000` / `180`     | Aborts pending requests on timeout    |
| `fetch`                             | `globalThis.fetch`   | Inject a custom fetch (e.g. for tests) |
| `headers`                           | `{}`                 | Extra HTTP headers per request        |

## Receipts

Every non-introspection response carries a `receipt` with:

- `responder_pubkey_b32` (ed25519 base32-nopad-lowercase)
- `signature_b32` (ed25519 over the BLAKE3 preimage digest)
- `preimage_version` (which rule signed it; read it, do not assume)
- `merkle_proof` (inclusion proof for `fact_cids[0]`, when one was recorded)
- `fact_cids[]`

Cite `receipt.fact_cids[0]` and the responder pubkey. Verify offline
against the public key at `https://emem.dev/.well-known/emem.json`.

**Pass the receipt on whole.** From `preimage_version: 2` the signature
covers the inclusion proof as well as the fields it already bound, which is
what stops an intermediary stripping the proof in transit. The cost is that
a receipt is byte-for-byte or nothing: dropping `merkle_proof`, re-keying a
field, or reshaping the envelope returns `signature_valid: false` on data
nobody tampered with, which is indistinguishable from forgery. Do not
destructure a receipt and rebuild it; keep the object the responder sent.

## License

Apache-2.0.
