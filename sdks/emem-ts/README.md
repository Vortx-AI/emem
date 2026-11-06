# @vortxai/emem: TypeScript client for emem.dev

`@vortxai/emem` is the TypeScript client for the [emem.dev](https://emem.dev)
Earth memory protocol. Use it when a JavaScript or TypeScript agent must
read signed Earth-observation facts and keep the receipts for citation.
Reads need no key.

```bash
npm install @vortxai/emem
```

```ts
import { Client } from "@vortxai/emem";

const loc = (await new Client().locate({ place: "Mount Fuji" })) as { cell64: string };
console.log(loc.cell64);
```

The script prints the cell64 address of Mount Fuji (`defi.zb592.nUkO.zEzE`
on 6 November 2026). For the protocol itself, read
[docs/quickstart.md](https://github.com/Vortx-AI/emem/blob/main/docs/quickstart.md)
and [docs/agents.md](https://github.com/Vortx-AI/emem/blob/main/docs/agents.md).

> We did not run the TypeScript snippets in this README on 6 November 2026,
> because the test host has no Node.js. On that day we sent the same HTTP
> requests with `curl`. The outputs shown come from those requests.

## What the client does

The `Client` class wraps the public REST surface. Each method sends one
request and returns the parsed JSON without changes. The client keeps every
Ed25519-signed receipt and every content-addressed CID, so you can cite a
fact and verify it offline. The full REST surface is the list in
[`/openapi.json`](https://emem.dev/openapi.json). This README gives no
count, because a count goes out of date.

Every method returns the type `Json`. Cast the result to the shape that you
read, as the examples here do.

## Install

```bash
npm install @vortxai/emem
```

The npm package is `@vortxai/emem`, version 2.4.4 on 6 November 2026. npm
refuses the name `ememdev`, so the package has the `@vortxai` scope. The
Python client has the name `ememdev` on PyPI.

The client needs Node.js 18 or later. It also runs on Bun, Deno, browsers
and edge runtimes, because it uses the platform `fetch`. It has no runtime
dependencies.

To build the client from a checkout of this repository:

```bash
cd sdks/emem-ts
npm install
npm run build
```

The build writes `dist/`. Then, in your project, run
`npm install <path-to-checkout>/sdks/emem-ts`.

## Read a fact

```ts
import { Client } from "@vortxai/emem";

const em = new Client();
const loc = (await em.locate({ place: "Mount Fuji" })) as { cell64: string };
const res = (await em.recall({
  cell: loc.cell64,
  bands: ["copdem30m.elevation_mean"],
})) as { facts: Array<{ value: number; fact_cid: string }> };
console.log(res.facts[0].value);    // 3539.042236328125
console.log(res.facts[0].fact_cid); // qejpz7cgvk347ordd6vrdtmwzprqlzcnkmotsjton2x4y2jjbplq
```

The values in the comments came from `curl` on 6 November 2026. A recall
on a cell that has no fact yet fetches the source data first. That first
call can take tens of seconds.

The file [`examples/recall.ts`](https://github.com/Vortx-AI/emem/blob/main/sdks/emem-ts/examples/recall.ts) does the same steps and
also prints the receipt. Run it from `sdks/emem-ts` with
`npx tsx examples/recall.ts`.

## Configure the client

| Option | Environment variable | Default | Effect |
|---|---|---|---|
| `baseUrl` | `EMEM_BASE_URL` | `https://emem.dev` | Responder root. Set it to use a self-hosted node. |
| `timeoutMs` | `EMEM_TIMEOUT_SECS` | `180000` ms (180 s) | Stops a request that takes longer. |
| `fetch` | none | `globalThis.fetch` | Use a different `fetch`, for example in tests. |
| `headers` | none | `{}` | Adds HTTP headers to every request. |

The environment variables apply only where `process.env` exists (Node.js,
Bun, Deno). An option that you give to `new Client({...})` overrides the
variable.

## Handle errors

A response with an HTTP status other than 2xx throws `EmemHTTPError`. It
has the fields `status`, `url` and `body`. `body` holds the parsed error
JSON from the responder. `EmemHTTPError` extends `EmemError`, and the
package exports both. When no `fetch` exists, the constructor throws
`EmemError`. When a request goes past the timeout, the call rejects with
the `AbortError` of the platform.

## Keep the receipt

Every response from a read primitive (for example `recall`) has a `receipt`
object. On 6 November 2026 a recall receipt had these fields, among others:

| Field | Content |
|---|---|
| `responder_pubkey_b32` | Ed25519 public key of the responder, lowercase base32 without padding |
| `signature_b32` | Ed25519 signature over the BLAKE3 preimage digest |
| `preimage_version` | The rule that made the preimage. Read it; do not assume it. The value was `2`. |
| `merkle_proof` | Inclusion proof for `fact_cids[0]`, when the log has one |
| `fact_cids` | The CIDs of the facts that the receipt covers |

To cite a fact, give `receipt.fact_cids[0]` and the responder public key.
The responder also publishes the key at
[`/.well-known/emem.json`](https://emem.dev/.well-known/emem.json), field
`federation.node.responder_pubkey_b32`. To verify a receipt, read section 7
of [docs/protocol.md](https://github.com/Vortx-AI/emem/blob/main/docs/protocol.md#7-receipt).

Keep the receipt object exactly as the responder sent it. From
`preimage_version: 2`, the signature also covers the inclusion proof. This
stops an intermediary that removes the proof. But a receipt that you change
in any way does not verify. If you remove `merkle_proof`, rename a field or
rebuild the object, the check returns `signature_valid: false`. That result
looks the same as a forgery.

## License

Apache-2.0.
