# emem-langmem

`emem-langmem` is a LangChain key-value store that keeps its values as
signed memory notes on an emem responder. Use it when a LangChain agent
must keep notes that other agents can read and check.

`EmemStore` is a `langchain_core.stores.BaseStore[str, bytes]`.
It is not a LangGraph store (`langgraph.store.base.BaseStore`). A
LangGraph graph compiles with it, but a node that calls `store.get` or
`store.put` fails with `AttributeError`. We checked this with
langgraph 1.0.1 on 7 November 2026.

Try it. Reads need no key:

```bash
pip install emem-langmem
```

```python
from emem_langmem import EmemStore

store = EmemStore()  # https://emem.dev, or EMEM_BASE_URL
path = "/memories/by_attester/y6dg4qz6/integration-friction-log-2026-08-11.md"
print(store.mget([path])[0][:38])
# b'# Integration friction log, 2026-08-11'
```

Version 2.4.4 is on PyPI (uploaded 18 October 2026). It needs Python
3.9 or later, `langchain-core` 0.3, `httpx`, `blake3` and `cryptography`.
The source is in
[sdks/emem-langmem](https://github.com/Vortx-AI/emem/tree/main/sdks/emem-langmem).
For the memory wire format, read
[docs/memory.html](https://emem.dev/docs/memory.html).

## Write with a signing key

```python
import os
from emem_langmem import EmemStore

SEED = os.urandom(32)  # make it once, then keep it with the agent's other secrets
store = EmemStore(signing_key=SEED)
store.mset([((("notes",), "today"), b"field 7 looked dry")])
```

The `mset` call is a write. We did not run it against emem.dev.

`SEED` is a raw 32-byte Ed25519 seed. It signs every write. The
responder refuses an unsigned write, unless the operator started it with
`EMEM_MEMORY_OPEN=1`. The key also sets where the store writes. If you
lose the seed, you lose write access to that namespace.

`EmemStore` sends each `BaseStore` verb as an MCP `tools/call` request
to the responder:

| BaseStore verb | emem MCP tool | Signed |
|---|---|---|
| `mget(keys)` | `emem_memory_view` | no, it is a read |
| `mset(pairs)` | `emem_memory_create` | yes |
| `mdelete(keys)` | `emem_memory_view`, then `emem_memory_delete` | yes |
| `yield_keys(prefix=)` | `emem_memory_view` on a directory | no, it is a read |

The async verbs `amget`, `amset`, `amdelete` and `ayield_keys` do the
same work over `httpx.AsyncClient`.

Values are bytes. The store sends text bytes as UTF-8 and other bytes
as base64. `mset` writes with the memory kind `resource`. To change it,
pass `default_kind` (`episodic`, `semantic`, `procedural` or `resource`).

## Namespaces

With a signing key, the store roots itself at
`/memories/by_attester/<pubkey8>/`. `<pubkey8>` is the first 8
characters of your base32 public key. Only that key can write there, so
agents that share one responder cannot overwrite each other.

A key can be a string or a `(namespace, key)` tuple. The tuple
`(("notes",), "today")` reads and writes
`/memories/by_attester/<pubkey8>/notes/today`.

```python
store.root               # '/memories/by_attester/ys3ez46s' (example)
store.signer.pubkey_b32  # the full base32 public key, 52 characters
```

A key that starts with `/` is an absolute path. The store does not
change it, so you can read outside your own root. If you write into the
namespace of a different key, the responder refuses it with a 403. Without a signing
key, the root is `/memories` and writes go out unsigned.

`mget` returns `None` for a key that does not exist. `yield_keys` on a
namespace that has no notes yet raises `EmemStoreError` with
`cid_not_found`.

`EmemSigner` is public. Use it when you call the memory tools yourself
and need the same `attester` block:

```python
from emem_langmem import EmemSigner

signer = EmemSigner(SEED)
signer.attester_block("create", f"{signer.namespace_root}/n.md", b"hi")
# {'pubkey_b32': '...', 'sig_b32': '...'}
```

## Use your own responder

```python
EmemStore(base_url="http://127.0.0.1:5051")
```

You can also set `EMEM_BASE_URL` in the environment.

## Get the receipt

The responder returns an Ed25519 receipt with each memory read and
write. The `BaseStore` interface has no place for it, so `EmemStore`
drops it. To get the receipt, call the MCP tool directly:

```python
import json, uuid
import httpx

path = "/memories/by_attester/y6dg4qz6/integration-friction-log-2026-08-11.md"
r = httpx.post(
    "https://emem.dev/mcp",
    json={
        "jsonrpc": "2.0",
        "id": str(uuid.uuid4()),
        "method": "tools/call",
        "params": {"name": "emem_memory_view", "arguments": {"path": path}},
    },
    timeout=60,
).json()
body = json.loads(r["result"]["content"][0]["text"])
print(body["file_cid"], body["receipt"]["primitive"])
# fhoisfurm535cqzrigsauqmf6e emem.memory_file
```

On 7 November 2026, emem.dev sent the tool result as JSON text in
`result.content[0].text`, with no `structuredContent`. To check a
receipt offline, use the [verify page](https://emem.dev/verify).

## License

Apache-2.0.
