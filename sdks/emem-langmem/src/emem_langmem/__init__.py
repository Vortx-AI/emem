"""LangChain BaseStore backed by emem.dev.

A LangGraph agent gets verifiable, ed25519-signed agent memory in one
line, plus the key it signs with:

    from emem_langmem import EmemStore
    store = EmemStore(base_url="https://emem.dev", signing_key=SEED)
    graph = StateGraph(...).compile(store=store)

`SEED` is a raw 32-byte ed25519 seed the caller owns and persists. It
does two things. It signs every write, which the responder requires:
an unattested write is refused unless the operator opened the responder
with `EMEM_MEMORY_OPEN=1`. And it roots the store at
`/memories/by_attester/<pubkey8>/`, the namespace only that key can
write to, so agents sharing one responder cannot overwrite each other.
Lose the seed and you lose write access to that namespace, so keep it
where the agent's other secrets live.

`EmemStore` implements the four `BaseStore` verbs (`mget`, `mset`,
`mdelete`, `yield_keys`) by issuing MCP `tools/call` requests to the
emem responder against the six Anthropic memory-tool verbs (`memory_view`,
`memory_create`, `memory_str_replace`, `memory_insert`, `memory_delete`,
`memory_rename`). Every write is content-addressed and countersigned by
the responder; every read carries a receipt verifiable offline at the
responder's `/verify` page.

`EmemSigner` is public for callers who drive the memory tools directly
and need the same `attester` block.
"""

from importlib.metadata import PackageNotFoundError, version as _pkg_version

from emem_langmem.signing import EmemSigner
from emem_langmem.store import EmemAttestationError, EmemStore, EmemStoreError

__all__ = ["EmemStore", "EmemSigner", "EmemStoreError", "EmemAttestationError"]

# Single source of truth: the installed distribution's version, which comes
# from pyproject.toml. Keeps __version__ and the store's User-Agent from
# drifting out of sync with what actually ships to PyPI.
try:
    __version__ = _pkg_version("emem-langmem")
except PackageNotFoundError:  # running from a source checkout, not installed
    __version__ = "0+unknown"
