# emem-langmem: LangChain and LangGraph memory on a signed store

`emem-langmem` is a LangChain [`BaseStore`](https://python.langchain.com/api_reference/core/stores/langchain_core.stores.BaseStore.html)
implementation backed by an emem responder. You hand it to
`StateGraph.compile(store=...)` and your LangGraph agent's long-term
memory becomes content-addressed, ed25519-signed, and readable by any
other agent you choose to share a namespace with.

It is a thin adapter, not a reimplementation. Every operation is one MCP
`tools/call` against the responder's memory verbs, so anything the store
can do is also reachable from a plain HTTP client, and nothing about your
data is locked inside this package.

```bash
pip install emem-langmem
```

Source lives at
[sdks/emem-langmem](https://github.com/Vortx-AI/emem/tree/main/sdks/emem-langmem).

## The 60-second version

```python
import os
from emem_langmem import EmemStore
from langgraph.graph import StateGraph

SEED = bytes.fromhex(os.environ["EMEM_AGENT_SEED"])  # 32 raw bytes, yours

store = EmemStore(base_url="https://emem.dev", signing_key=SEED)
graph = StateGraph(...).compile(store=store)
```

`base_url` falls back to `EMEM_BASE_URL`, then to `https://emem.dev`.

## The signing key is not optional in practice

`signing_key` takes a raw 32-byte ed25519 seed. Generate one once with
`os.urandom(32)`, then keep it wherever the agent's other secrets live.
Losing it does not destroy the memories, but it does mean you can no
longer write to the namespace that holds them.

Two things follow from the key, and both surprise people:

**It decides where you write.** With a key, the store roots itself at
`/memories/by_attester/<pubkey8>/`, where `<pubkey8>` is the first 8
characters of your base32 public key. Without one the root stays
`/memories`.

```python
store.root               # '/memories/by_attester/aoqqpp7t'
store.signer.pubkey_b32  # the full base32 public key
```

**It decides whether the write is accepted at all.** The public responder
refuses unattested writes. An operator can reopen the global namespace by
starting the server with `EMEM_MEMORY_OPEN=1`, but emem.dev does not, so
an unsigned `mset` against it will fail. The store recognises that
failure and its error message tells you to pass a `signing_key` rather
than leaving you to decode a 403.

Under `by_attester/` only the matching key may write, which is what makes
it safe for several agents to share one responder: they cannot overwrite
each other. A cross-namespace write returns HTTP 403 with the code
`memory_namespace_violation`. Outside `by_attester/`, the first attester
to create a path owns it, and the same code refuses everyone else.

A key that starts with `/` is treated as a literal absolute path and left
alone, which is how you deliberately read outside your own root:

```python
store.mget([("notes", "harvest")])      # /memories/by_attester/<pk8>/notes/harvest
store.mget(["/memories/shared/atlas"])  # exactly that path
```

## What each verb actually calls

| `BaseStore` verb | emem MCP tool | Signed |
|---|---|---|
| `mget(keys)` | `emem_memory_view` | no, reads are open |
| `mset(pairs)` | `emem_memory_create` | yes |
| `mdelete(keys)` | `emem_memory_delete` | yes |
| `yield_keys(prefix)` | `emem_memory_view` as a directory walk | no |

The async surface (`amget`, `amset`, `amdelete`, `ayield_keys`) is the
same set over `httpx.AsyncClient`. `EmemStore` is also a context manager,
and `close()` releases both clients.

Paths are assembled from LangChain's `(namespace, key)` tuple by joining
the parts under the store root, so `mget(("prefs", "tone"))` reads
`/memories/by_attester/<pk8>/prefs/tone`. One consequence worth knowing:
the responder restricts memory paths to `/memories/`, per the Anthropic
memory-tool specification, so a path that escapes that prefix is refused
by the server rather than by the client.

## Receipts, and the one thing this adapter cannot give you

Every read and write returns an ed25519 receipt from the responder. The
receipt is the point of emem: it is what lets a third party confirm that
the bytes you were handed are the bytes the responder attests to holding,
without trusting you or us.

`BaseStore` has no slot for one. LangChain's interface returns values, so
the receipt is dropped at the interface boundary. That is a limitation of
the contract this package implements, not something the package is hiding
from you, and there is no version of it that both satisfies `BaseStore`
and hands back receipts through the same call.

If you need receipts, call the memory tool directly. It is the same
endpoint the store uses:

```python
import json, uuid, httpx

r = httpx.post(
    "https://emem.dev/mcp",
    json={
        "jsonrpc": "2.0",
        "id": str(uuid.uuid4()),
        "method": "tools/call",
        "params": {
            "name": "emem_memory_view",
            "arguments": {"path": "/memories/by_attester/aoqqpp7t/notes/harvest"},
        },
    },
).json()
print(r["result"]["structuredContent"]["receipt"])
```

Verify a receipt with `POST /v1/verify_receipt`, or offline at
[/verify](https://emem.dev/verify), which recomputes the preimage and
checks the signature against the responder's published public key.

## Signing it yourself

`EmemSigner` is public, so you can drive the memory verbs directly and
still produce the `attester` block the responder expects:

```python
from emem_langmem import EmemSigner

signer = EmemSigner(SEED)
signer.namespace_root  # '/memories/by_attester/aoqqpp7t'
signer.attester_block("create", f"{signer.namespace_root}/note.md", b"hi")
# {'pubkey_b32': '...', 'sig_b32': '...'}
```

The preimage shape is fixed and shared with the Rust implementation, so a
block built here verifies there. Note that it is per-verb: the signature
binds the verb, the path and a hash of the body, so a `create` signature
is not a `delete` signature for the same path. `rename` is the exception
worth reading the source for, because its preimage binds the old path
through the body hash while `path` carries the destination.

The full wire format, the verb list and the error codes are in
[Memory substrate](../memory.md).

## Self-hosting

```python
EmemStore(base_url="http://127.0.0.1:5051")
```

Everything above is identical against your own responder, including the
namespace rules, since they are enforced server-side. See
[Self-host](../self-host.md).

## Where things live

| | |
|---|---|
| Package | [`emem-langmem` on PyPI](https://pypi.org/project/emem-langmem/) |
| Source | [sdks/emem-langmem](https://github.com/Vortx-AI/emem/tree/main/sdks/emem-langmem) |
| Issues | [github.com/Vortx-AI/emem/issues](https://github.com/Vortx-AI/emem/issues) |
| Wire format | [Memory substrate](../memory.md) |
| Error codes | [Errors](../errors.md) |

Licensed Apache-2.0.
