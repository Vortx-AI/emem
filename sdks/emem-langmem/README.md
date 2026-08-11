# emem-langmem

LangChain `BaseStore` backed by [emem.dev](https://emem.dev) — drop-in
agent memory for LangGraph with ed25519 receipts and content-addressed
recall.

## Install

```bash
pip install emem-langmem
```

## Use

```python
from emem_langmem import EmemStore
from langgraph.graph import StateGraph

store = EmemStore(base_url="https://emem.dev", signing_key=SEED)

graph = StateGraph(...).compile(store=store)
```

`SEED` is a raw 32-byte ed25519 seed that you own and persist, for
example `os.urandom(32)` once, then kept wherever the agent's other
secrets live. It signs every write, and the responder requires that:
an unattested write is refused unless the operator opened the responder
with `EMEM_MEMORY_OPEN=1`. The key also decides where the store writes.

`EmemStore` implements the four `BaseStore` verbs by issuing MCP
`tools/call` requests to the emem responder against the six Anthropic
memory-tool verbs:

| BaseStore verb | emem MCP tool   | Signed |
|----------------|-----------------|--------|
| `mget(keys)`   | `emem_memory_view`   | read, no signature |
| `mset(pairs)`  | `emem_memory_create` | yes |
| `mdelete(ks)`  | `emem_memory_delete` | yes |
| `yield_keys`   | `emem_memory_view` (directory walk) | read, no signature |

## Namespaces

With a signing key, the store roots itself at
`/memories/by_attester/<pubkey8>/`, where `<pubkey8>` is the first 8
characters of your base32 pubkey. Only that key can write there, so
agents sharing one responder cannot overwrite each other. LangChain
calls `mget(("ns","key"))` and the store reads
`/memories/by_attester/<pubkey8>/ns/key`.

```python
store.root              # '/memories/by_attester/aoqqpp7t'
store.signer.pubkey_b32 # the full base32 pubkey
```

A key that starts with `/` is taken as a literal absolute path and is
left alone, which is how you reach outside your own root. Writing into
another key's namespace is refused with a 403. Without a signing key the
root stays `/memories` and writes go out unattested. See
[docs/memory.html](https://emem.dev/docs/memory.html) for the wire format.

`EmemSigner` is public if you drive the memory tools yourself and need
the same `attester` block:

```python
from emem_langmem import EmemSigner

signer = EmemSigner(SEED)
signer.attester_block("create", f"{signer.namespace_root}/n.md", b"hi")
# {'pubkey_b32': '...', 'sig_b32': '...'}
```

Async variants `amget` / `amset` / `amdelete` / `ayield_keys` are
implemented over `httpx.AsyncClient`.

## Self-host

```python
EmemStore(base_url="http://127.0.0.1:5051")
```

or set `EMEM_BASE_URL` in the environment.

## Receipts

Every read and write returns an ed25519 receipt from the emem responder.
The receipt is currently dropped at the `BaseStore` interface boundary
(LangChain's contract has no receipt slot). To inspect receipts, call
the underlying MCP tool directly:

```python
import json, httpx, uuid
r = httpx.post(
    "https://emem.dev/mcp",
    json={
        "jsonrpc": "2.0",
        "id": str(uuid.uuid4()),
        "method": "tools/call",
        "params": {
            "name": "emem_memory_view",
            "arguments": {"path": "/memories/my/note.txt"},
        },
    },
).json()
print(r["result"]["structuredContent"]["receipt"])
```

## License

Apache-2.0.
