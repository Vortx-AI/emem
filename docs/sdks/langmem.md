# emem-langmem: LangChain and LangGraph memory on a signed store

`emem-langmem` is a LangChain [`BaseStore`](https://python.langchain.com/api_reference/core/stores/langchain_core.stores.BaseStore.html)
that keeps its data on an emem responder.
Give it to `StateGraph.compile(store=...)`, and the long-term memory of your LangGraph agent goes into emem.
Each value is content-addressed and signed with your Ed25519 key.
Other agents can read it at the same path.

The package is a thin adapter.
Each store operation is one MCP `tools/call` to a memory tool on the responder.
A plain HTTP client can do the same calls, so your data does not depend on this package.

## What works in version 2.4.4

Version 2.4.4 is the newest release on PyPI on 7 November 2026.
The source is in [sdks/emem-langmem](https://github.com/Vortx-AI/emem/tree/main/sdks/emem-langmem).

| Operation | Status in 2.4.4 |
|---|---|
| `mget`, `mset`, `mdelete` and their async forms | Work. A refused call raises `EmemStoreError`. |
| `yield_keys()` with no prefix | Fails on emem.dev. See [Known issues](#known-issues-in-244). |
| `yield_keys(prefix=...)` | Works when the prefix ends in `/`. Lists only the first part of a large folder. |
| Receipts | Not returned. `BaseStore` has no place for them. See [Get the receipt](#get-the-receipt). |
| Install next to the newest `langgraph` | Not possible. The package needs `langchain-core` below 1.0. |

## Install the package

```bash
pip install emem-langmem
```

The package needs Python 3.9 or later.
It pins `langchain-core>=0.3,<1.0`, `httpx`, `blake3` and `cryptography`.
On 7 November 2026 the newest `langgraph` (1.2.20) needs `langchain-core` 1.4.7 or later.
`langgraph` 1.1.7 to 1.1.10 need `langchain-core` 1.3 or later.
`langgraph` 1.0.10 and 1.1.0 still accept `langchain-core` 0.3.
Thus pip installs an older `langgraph` next to `emem-langmem`.

## Connect a LangGraph agent

```python
import os
from emem_langmem import EmemStore
from langgraph.graph import StateGraph

SEED = bytes.fromhex(os.environ["EMEM_AGENT_SEED"])  # 32 raw bytes, yours

store = EmemStore(base_url="https://emem.dev", signing_key=SEED)
graph = StateGraph(...).compile(store=store)
```

If you do not give `base_url`, the store uses `EMEM_BASE_URL`, then `https://emem.dev`.
Other arguments: `timeout` (seconds, default 30), `default_kind` (the memory kind for `mset`, default `resource`) and `headers`.
`EmemStore` is a context manager. `close()` closes the sync and the async HTTP clients.

## Give the store a signing key

`signing_key` takes a raw 32-byte Ed25519 seed, or an `EmemSigner`.
Make the seed one time with `os.urandom(32)`.
Keep it with the other secrets of the agent.
If you lose the seed, your notes stay on the responder, but you cannot write to that folder again.

The key controls two things.

The key sets the folder that the store writes to.
With a key, the store root is `/memories/by_attester/<pubkey8>`.
`<pubkey8>` is the first 8 characters of your lowercase base32 public key.
Without a key, the root is `/memories`.

```python
store.root               # '/memories/by_attester/aoqqpp7t'
store.signer.pubkey_b32  # the full base32 public key
```

The key also decides if the responder accepts a write.
emem.dev refuses a write that has no signature (HTTP 401 over REST, code `memory_attestation_required`).
An operator can accept unsigned writes with `EMEM_MEMORY_OPEN=1`. emem.dev does not set it.

Under `/memories/by_attester/`, only the key that matches `<pubkey8>` can write.
Thus several agents can share one responder, and no agent can overwrite the notes of another agent.
Outside `by_attester/`, the first key that makes a path owns it.
In both cases, a write by a different key gets HTTP 403 with the code `memory_namespace_violation`.

A key that starts with `/` is an absolute path. The store does not change it.
Use this form to read outside your own root:

```python
store.mget([(("notes",), "harvest")])  # /memories/by_attester/<pk8>/notes/harvest
store.mget(["/memories/shared/atlas"])   # exactly that path
```

The namespace must be a tuple, as in `(("notes",), "harvest")`.
A flat pair such as `("notes", "harvest")` raises `TypeError: unsupported key shape`.

The responder accepts only paths under `/memories/`, as the Anthropic memory tool specification requires.
On 7 November 2026 a view of `/etc/passwd` gave `invalid_argument`: the path "must stay under `/memories/`".

## What each store operation calls

| `BaseStore` operation | emem MCP tool | Signed |
|---|---|---|
| `mget(keys)` | `emem_memory_view` | No. Reads need no key. |
| `mset(pairs)` | `emem_memory_create` | Yes, with the v1 preimage. |
| `mdelete(keys)` | `emem_memory_view`, then `emem_memory_delete` | Yes, with the v2 preimage over the `file_cid` it read. |
| `yield_keys(prefix)` | `emem_memory_view` on a folder | No. |

The async forms (`amget`, `amset`, `amdelete`, `ayield_keys`) make the same calls through `httpx.AsyncClient`.

The store joins the parts of a LangChain `(namespace, key)` tuple under its root.
Thus `mget([(("prefs",), "tone")])` reads `/memories/by_attester/<pk8>/prefs/tone`.
The store sends text values as UTF-8 and other bytes as base64.

`mget` gives `None` for a key that has no file.
`mdelete` does nothing for a key that is not there, as `BaseStore` requires.

## Known issues in 2.4.4

These issues are in the adapter, not in the responder.
We found them on 7 November 2026 in the package source and in reads on emem.dev.

`yield_keys()` with no prefix fails.
The responder lists a folder only when the path ends in `/` (`memory_view`, `crates/emem-api-rest/src/lib.rs`).
The store sends its root with no final `/`.
The responder answers `cid_not_found`, and the store raises `EmemStoreError`.
Give a prefix that ends in `/`, for example `store.yield_keys(prefix=store.root + "/")` or `store.yield_keys(prefix="notes/")`.

`yield_keys` can stop early on a large folder.
The MCP answer has a size limit. When a listing is too large, the responder keeps the first entries.
It adds an `_emem_truncation` block with the `offset` of the next page.
The store does not read that block, so it gives only the first page.
On 7 November 2026 the folder `/memories/by_attester/` had 5,387 entries, and the MCP answer kept 104.
For a large folder, call `emem_memory_view` yourself with `offset`, and follow `_emem_truncation.next`.

The store can cut an error message.
Over MCP, a refusal puts its `details` block (with `how_to_sign`) in the message text.
`EmemStoreError` keeps only the first 500 characters of that text.
To see the full refusal, send the same call directly. See [Sign writes yourself](#sign-writes-yourself).

The 2.4.0 release had two more defects. The 2.4.2 release fixed them.
`mdelete` signed only the v1 preimage, so the responder refused it.
A tool result with `isError: true` did not raise an error.

## Get the receipt

The responder signs a receipt for each file read.
The receipt lets a third party check that the bytes are the bytes that the responder holds.
The `BaseStore` interface returns values only, so the store drops the receipt.
No version of `BaseStore` can return it.

To get the receipt, call the memory tool directly. The store uses the same endpoint:

```python
import uuid, httpx

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
result = r["result"]
if result.get("isError"):
    raise RuntimeError(result["content"][0]["text"])  # e.g. cid_not_found
print(result["structuredContent"]["receipt"])
```

Replace the path with a path that you wrote. `aoqqpp7t` stands for your own `pubkey8`.
On 7 November 2026 a file view also had an `authorship` block.
It gives the key of the writer, the signature, `preimage_version` and the `signed_path`.
A folder listing has no receipt.

Check the receipt with `POST /v1/verify_receipt`, or offline at [/verify](https://emem.dev/verify).
The page computes the preimage again and checks the signature against the public key of the responder.

## Sign writes yourself

`EmemSigner` is public. Use it to call the memory tools directly with a correct `attester` block:

```python
from emem_langmem import EmemSigner

signer = EmemSigner(SEED)
signer.namespace_root  # '/memories/by_attester/aoqqpp7t'
signer.attester_block("create", f"{signer.namespace_root}/note.md", b"hi")
# {'pubkey_b32': '...', 'sig_b32': '...'}
```

The signature binds the verb, the path and the BLAKE3 hash of the body.
Thus a `create` signature is not a `delete` signature for the same path.
The body is the bytes that the responder hashes for that verb:

| Verb | Body that the responder hashes | Preimage |
|---|---|---|
| `create` | the `file_text` string as UTF-8 | v1, or v2 with `base=` |
| `str_replace`, `insert` | the full file after the edit | v1, or v2 with `base=` |
| `delete` | empty | v2 only: pass `base=` the current `file_cid` |
| `rename` | the source path | v2 only: use `rename_attester_block(old_path, new_path)` |

`attester_block` raises `ValueError` for `delete` or `rename` without `base`.
For `str_replace` and `insert`, read the file first, make the edit locally and sign the result.
For `rename`, the key must own the source and the destination.

If the responder refuses a write, the refusal has `details.how_to_sign` with the digest that it expected.
Compare it with your digest to find the difference.
The full wire format, the verbs and the error codes are in [Memory substrate](../memory.md).

## Use your own responder

```python
EmemStore(base_url="http://127.0.0.1:5051")
```

The responder enforces the folder rules, so they are the same on your own node.
See [Self-host](../self-host.md).

## Where to find more

| Item | Link |
|---|---|
| Package | [`emem-langmem` on PyPI](https://pypi.org/project/emem-langmem/) |
| Source | [sdks/emem-langmem](https://github.com/Vortx-AI/emem/tree/main/sdks/emem-langmem) |
| Issues | [github.com/Vortx-AI/emem/issues](https://github.com/Vortx-AI/emem/issues) |
| Wire format | [Memory substrate](../memory.md) |
| Error codes | [Errors](../errors.md) |

The package has the Apache-2.0 license.
