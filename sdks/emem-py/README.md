# ememdev: Python client for emem.dev

<!-- mcp-name: io.github.Vortx-AI/emem -->

Thin, typed Python client for the [emem.dev](https://emem.dev) Earth memory
protocol. Wraps the public REST surface in a single `Client` class that
returns parsed JSON verbatim. Every ed25519-signed receipt and
content-addressed CID is preserved for citation and offline verification.
The current surface is whatever [`/openapi.json`](https://emem.dev/openapi.json)
lists; this README does not restate a count that would go stale.

## Install

```bash
pip install ememdev
```

The distribution and the import package are both named `ememdev` (so
`pip install ememdev`, then `from ememdev import Client`); the shorter name
`emem` on PyPI belongs to an unrelated project. To work from a checkout
instead:

```bash
pip install -e sdks/emem-py
```

Requires Python 3.9+. The only runtime dependency is
[`httpx`](https://www.python-httpx.org/). To sign memory writes (below),
add the `signing` extra, which pulls `blake3` and `cryptography`:

```bash
pip install "ememdev[signing]"
```

## Quick start

```python
from ememdev import Client

with Client() as em:
    cell = em.locate("Mount Fuji")["cell64"]
    facts = em.recall(cell, bands=["copdem30m.elevation_mean"])
    print(facts["facts"][0]["value"])
```

## Async

```python
import asyncio
from ememdev import AsyncClient

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

## Writing a signed note

Reads are anonymous. Writes into the agent memory namespace are gated by an
ed25519 `attester` block, and that signature is the one thing the MCP client
cannot produce for you. `ememdev[signing]` produces it, from a persistent
identity, so leaving a signed note is one command rather than a hand-rolled
crypto script.

The identity is resolved once and reused every session, because the key is
what owns your namespace at `/memories/by_attester/<pubkey8>/...`. Resolution
order: `EMEM_AGENT_KEY` (a 64-char hex seed or a key-file path), then
`~/.emem/agent_ed25519.pem` (override the directory with `EMEM_HOME`), minted
`0600` on first use if absent.

From the command line, for an agent that already speaks to emem over MCP:

```bash
ememdev whoami          # your pubkey and namespace_root

# emit just the attester block, sign nothing else (no network):
ememdev sign  --verb create --path /memories/by_attester/<you>/note.md --stdin < note.md

# or sign a create and POST it to /mcp in one step:
ememdev write --path /memories/by_attester/<you>/note.md --body-file note.md
```

Or in Python:

```python
from ememdev import load_or_create_signer

signer = load_or_create_signer()             # persistent identity
path = f"{signer.namespace_root}/note.md"
body = b"a durable, signed, citeable note"
attester = signer.attester_block("create", path, body)
# POST {path, file_text, kind, attester} to /mcp emem_memory_create.
```

`str_replace` and `insert` sign the whole file *after* the edit, and `rename`
binds both ends of the move (`signer.rename_attester_block(old, new)`); see the
`ememdev.signing` docstring for the per-verb body rule. A write into another
key's namespace is refused with a 403, and `sign`/`write` refuse it locally
first.

## Receipts

Every non-introspection response carries a `receipt` block with:

- `responder_pubkey_b32` (ed25519 base32-nopad-lowercase)
- `signature_b32` (ed25519 over the BLAKE3 preimage digest)
- `preimage_version` (which rule signed it; read it, do not assume)
- `merkle_proof` (inclusion proof for `fact_cids[0]`, when one was recorded)
- `fact_cids[]` (the BLAKE3 CIDs of every fact returned)

To cite an answer: quote `receipt.fact_cids[0]` and the responder pubkey.
The signature can be verified offline against the public key at
`https://emem.dev/.well-known/emem.json`; no callback to the responder is
required.

**Pass the receipt on whole.** `ememdev` returns it verbatim and you should
store and forward it the same way. From `preimage_version: 2` the signature
covers the inclusion proof as well as every other field it binds, which is
what stops an intermediary stripping the proof in transit. The cost is that
a receipt is byte-for-byte or nothing: drop `merkle_proof` because it looks
redundant, re-key a field, or summarise the envelope, and it comes back
`signature_valid: false` on data nobody tampered with — indistinguishable
from forgery. `POST /v1/verify_receipt` reports `reason:
receipt_reshaped_after_signing` instead of `signature_invalid` when the
responder can prove that is what happened, but it never accepts the receipt.

## License

Apache-2.0. Same as the upstream protocol.
