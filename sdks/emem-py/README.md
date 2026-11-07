# ememdev: Python client for emem.dev

<!-- mcp-name: io.github.Vortx-AI/emem -->

`ememdev` is the Python client for the [emem.dev](https://emem.dev) Earth
memory protocol. Use it when a Python program or agent must read signed
Earth-observation facts and keep the receipts for citation. Reads need no key.

```bash
pip install ememdev
python -c 'from ememdev import Client; print(Client().locate("Mount Fuji")["cell64"])'
```

The second line prints the cell64 address of Mount Fuji (`defi.zb592.nUkO.zEzE`
on 7 November 2026). For the protocol itself, read
[docs/quickstart.md](https://github.com/Vortx-AI/emem/blob/main/docs/quickstart.md)
and [docs/agents.md](https://github.com/Vortx-AI/emem/blob/main/docs/agents.md).

## What the client does

The `Client` class wraps the public REST surface. Each method sends one
request and returns the parsed JSON without changes. That JSON includes
the Ed25519-signed receipt and the content-addressed CIDs, so you can cite a
fact and verify it offline. The module docstring of `ememdev.client` maps
each endpoint to its method. The full REST surface is the list in
[`/openapi.json`](https://emem.dev/openapi.json). This README gives no count,
because a count goes out of date.

The client does not wrap endpoints that do not return JSON over HTTP/1.1.
Examples are `/v1/stream` (SSE), the scene PNG and RGB images, and
`/v1/attest_cbor`. Call them with `httpx` directly. The module docstring
lists all of them.

## Install

```bash
pip install ememdev
```

The PyPI distribution and the import package have the same name, `ememdev`.
The shorter name `emem` on PyPI belongs to a different project.
To install from a checkout of this repository:

```bash
pip install -e sdks/emem-py
```

The client needs Python 3.9 or later. Its only runtime dependency is
[`httpx`](https://www.python-httpx.org/). To sign memory writes or verify
receipts offline, add the `signing` extra. It installs `blake3` and
`cryptography`:

```bash
pip install "ememdev[signing]"
```

## Read a fact

```python
from ememdev import Client

with Client() as em:
    cell = em.locate("Mount Fuji")["cell64"]
    facts = em.recall(cell, bands=["copdem30m.elevation_mean"])
    print(facts["facts"][0]["value"])
```

On 7 November 2026 this printed `3539.042236328125` (metres). If no fact
is in memory for the cell, `recall` fetches the source first. That first
read can take tens of seconds; the client timeout is 180 seconds.

## Use the async client

```python
import asyncio
from ememdev import AsyncClient

async def main() -> None:
    async with AsyncClient() as em:
        out = await em.ask("How tall is Mount Everest?")
        print(out["answer"])

asyncio.run(main())
```

`AsyncClient` has the same methods as `Client`.

## Configure the client

| Environment variable | Default            | Effect                                         |
|----------------------|--------------------|------------------------------------------------|
| `EMEM_BASE_URL`      | `https://emem.dev` | Responder root. Set it to use a self-hosted node. |
| `EMEM_TIMEOUT_SECS`  | `180`              | HTTP timeout in seconds. It matches the gateway timeout. |

You can also give `base_url=` and `timeout=` to the constructor.

## Write a signed note

Reads are anonymous. A write into the agent memory namespace must carry an
Ed25519 `attester` block. The MCP client cannot make that signature for you.
`ememdev[signing]` makes it from a persistent identity, so one command
writes a signed note.

The key owns your namespace at `/memories/by_attester/<pubkey8>/...`, so
the client uses the same identity in every session. It looks for the key
in this order:

1. `EMEM_AGENT_KEY`: a 64-character hex seed or the path of a key file.
2. `~/.emem/agent_ed25519.pem`. Set `EMEM_HOME` to use a different directory.
3. If there is no key, the client makes one at that path with mode `0600`.

From the command line, for an agent that already uses emem over MCP:

```bash
ememdev whoami          # your pubkey and namespace_root

# write only the attester block to stdout; no network call:
ememdev sign  --verb create --path /memories/by_attester/<you>/note.md --stdin < note.md

# sign a create and POST it to /mcp in one step (this is a write to emem.dev):
ememdev write --path /memories/by_attester/<you>/note.md --body-file note.md
```

In Python:

```python
from ememdev import load_or_create_signer

signer = load_or_create_signer()             # persistent identity
path = f"{signer.namespace_root}/note.md"
body = b"a durable, signed, citeable note"
attester = signer.attester_block("create", path, body)
# POST {path, file_text, kind, attester} to /mcp emem_memory_create.
```

For `str_replace` and `insert`, sign the whole file after the edit. For
`rename`, use `signer.rename_attester_block(old, new)`, which signs both
paths. The `ememdev.signing` docstring gives the body rule for each verb.
The server refuses a write into the namespace of a different key with a 403.
The `sign` and `write` commands refuse it locally before they send anything.

## Cite and verify a receipt

Every response from a read primitive (for example `recall`) has a `receipt`
block. Some responses, such as `locate`, have none. A receipt contains these
fields:

- `responder_pubkey_b32`: the Ed25519 public key, base32 without padding, lowercase.
- `signature_b32`: the Ed25519 signature over the BLAKE3 preimage digest.
- `preimage_version`: the rule that made the signature. Read it; do not assume it.
- `merkle_proof`: the inclusion proof for `fact_cids[0]`, if the log recorded one.
- `fact_cids[]`: the BLAKE3 CIDs of all facts in the response.

To cite an answer, quote `receipt.fact_cids[0]` and the responder pubkey.
Anyone can verify the signature offline. With the `signing` extra installed,
`ememdev.verify` does the check in your process:

```python
from ememdev import Client
from ememdev.verify import verify_receipt_offline

with Client() as em:
    cell = em.locate("Mount Fuji")["cell64"]
    resp = em.recall(cell, bands=["copdem30m.elevation_mean"])
    v = verify_receipt_offline(resp["receipt"])
    print(v.ok, v.state, v.signer_b32)
```

On 7 November 2026 this printed `True verified 777er3yi...` with
`preimage_version` 2. The function checks the signature against the key in
the receipt. Compare `v.signer_b32` with the key you trust, for example
`federation.node.responder_pubkey_b32` in
`https://emem.dev/.well-known/emem.json`. On 7 November 2026 the two keys
were the same. The function does not raise and does not use the network.
If no check ran, `v.ok` is `False` and `v.state` gives the cause.
Without the extra, the state is `crypto_unavailable`. If the local encoders
disagree with the Rust test vectors, the state is `self_test_failed`.
`Client.verify_receipt()` is different: it asks the
responder to do the check.

Keep and forward the receipt whole. `ememdev` returns it without changes;
store it and pass it on the same way. From `preimage_version: 2`, the
signature also covers the inclusion proof. Thus a party between you and the
reader cannot remove the proof without detection.

The cost is that a changed receipt fails. Remove `merkle_proof`, rename a
field or summarise the envelope, and the check gives
`signature_valid: false`. That result looks the same as a forgery, although
nobody changed the data. If the responder can prove that someone reshaped
the receipt, `POST /v1/verify_receipt` gives `reason:
receipt_reshaped_after_signing` instead of `signature_invalid`. It never
accepts the changed receipt.

## License

Apache-2.0, the same license as the protocol.
