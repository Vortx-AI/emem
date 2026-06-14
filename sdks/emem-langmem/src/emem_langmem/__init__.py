"""LangChain BaseStore backed by emem.dev.

A LangGraph agent gets verifiable, ed25519-signed agent memory in one
line:

    from emem_langmem import EmemStore
    store = EmemStore(base_url="https://emem.dev")
    graph = StateGraph(...).compile(store=store)

`EmemStore` implements the four `BaseStore` verbs (`mget`, `mset`,
`mdelete`, `yield_keys`) by issuing MCP `tools/call` requests to the
emem responder against the six Anthropic memory-tool verbs (`memory_view`,
`memory_create`, `memory_str_replace`, `memory_insert`, `memory_delete`,
`memory_rename`). Every write is content-addressed and ed25519-signed
by the responder; every read carries a receipt verifiable offline at
the responder's `/verify` page.
"""

from emem_langmem.store import EmemStore

__version__ = "0.1.0"
__all__ = ["EmemStore"]
