# @emem/client — TypeScript client for emem.dev

Thin, typed TypeScript client for the [emem.dev](https://emem.dev) Earth
memory protocol. Wraps the public REST surface (139 routes, 74 under
`/v1/*`) in a single `Client` class. Every call returns parsed JSON
verbatim — every ed25519-signed receipt and content-addressed CID is
preserved for citation and offline verification.

Runs on Node 18+, Bun, Deno, browsers, and edge runtimes. Uses the
platform `fetch`; zero runtime dependencies.

## Install

NPM publication is coming soon. For now, install from the repo:

```bash
cd sdks/emem-ts && npm install && npm run build
# then add as a local dependency in your project
```

## Quick start

```ts
import { Client } from "@emem/client";

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

- `responder_pubkey` (ed25519 base32-nopad-lowercase)
- `signature_b32` (ed25519 over the canonical CBOR preimage)
- `merkle_root` (BLAKE3 over the fact CIDs)
- `fact_cids[]`

Cite `receipt.fact_cids[0]` and the responder pubkey. Verify offline
against the public key at `https://emem.dev/.well-known/emem.json`.

## License

Apache-2.0.
