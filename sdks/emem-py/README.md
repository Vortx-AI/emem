# emem — Python client for emem.dev

Thin, typed Python client for the [emem.dev](https://emem.dev) Earth memory
protocol. Wraps the public REST surface (139 routes, 74 under `/v1/*`)
in a single `Client` class that returns parsed JSON verbatim — every
ed25519-signed receipt and content-addressed CID is preserved for
citation and offline verification.

## Install

Not on PyPI yet. Install from the repo:

```bash
pip install -e sdks/emem-py
```

Requires Python 3.9+. The only runtime dependency is
[`httpx`](https://www.python-httpx.org/).

## Quick start

```python
from emem import Client

with Client() as em:
    cell = em.locate("Mount Fuji")["cell64"]
    facts = em.recall(cell, bands=["copdem30m.elevation_mean"])
    print(facts["facts"][0]["value"])
```

## Async

```python
import asyncio
from emem import AsyncClient

async def main() -> None:
    async with AsyncClient() as em:
        out = await em.ask("How tall is Mount Everest?")
        print(out["answer"])

asyncio.run(main())
```

## Configuration

| Env var               | Default              | Effect                                      |
|-----------------------|----------------------|---------------------------------------------|
| `EMEM_BASE_URL`       | `https://emem.dev`   | Responder root; point at a self-hosted node |
| `EMEM_TIMEOUT_SECS`   | `180`                | HTTP timeout (matches gateway timeout)      |

You can also pass `base_url=` and `timeout=` directly to the constructor.

## Surface coverage

Geocoder + read primitives, physics solvers, boring lat/lng shortcuts, and
introspection — see the inline docstring on `emem.client` for the full
endpoint → method mapping.

## Receipts

Every non-introspection response carries a `receipt` block with:

- `responder_pubkey` (ed25519 base32-nopad-lowercase)
- `signature_b32` (ed25519 over the canonical CBOR preimage)
- `merkle_root` (BLAKE3 over the fact CIDs)
- `fact_cids[]` (the BLAKE3 CIDs of every fact returned)

To cite an answer: quote `receipt.fact_cids[0]` and the responder pubkey.
The signature can be verified offline against the public key at
`https://emem.dev/.well-known/emem.json`; no callback to the responder is
required.

## License

Apache-2.0. Same as the upstream protocol.
