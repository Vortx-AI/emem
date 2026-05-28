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

store = EmemStore(base_url="https://emem.dev")

graph = StateGraph(...).compile(store=store)
```

`EmemStore` implements the four `BaseStore` verbs by issuing MCP
`tools/call` requests to the emem responder against the six Anthropic
memory-tool verbs:

| BaseStore verb | emem MCP tool   |
|----------------|-----------------|
| `mget(keys)`   | `memory_view`   |
| `mset(pairs)`  | `memory_create` |
| `mdelete(ks)`  | `memory_delete` |
| `yield_keys`   | `memory_view` (directory walk) |

LangChain calls `mget(("ns","key"))`; emem stores it at
`/memories/ns/key`. Top-level namespace `by_attester/` triggers
capability binding on the emem side — see
[docs/memory.md](https://emem.dev/docs/memory.md).

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
            "name": "memory_view",
            "arguments": {"path": "/memories/my/note.txt"},
        },
    },
).json()
print(r["result"]["structuredContent"]["receipt"])
```

## License

Apache-2.0.
