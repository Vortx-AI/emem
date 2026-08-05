"""`EmemStore`: LangChain `BaseStore[str, bytes]` over emem MCP.

The four `BaseStore` verbs map onto the six Anthropic memory-tool MCP
tools served by emem:

| BaseStore verb | MCP tool          | Notes                                  |
|----------------|-------------------|----------------------------------------|
| `mget(keys)`   | `memory_view`     | Returns file content as bytes          |
| `mset(pairs)`  | `memory_create`   | `kind` defaults to `"resource"`        |
| `mdelete(ks)`  | `memory_delete`   |                                        |
| `yield_keys`   | `memory_view` (d) | Recursive walk under prefix            |

Writes are signed. The responder refuses an unattested write by
default, so pass `signing_key` (a raw 32-byte ed25519 seed, or an
`EmemSigner`) when constructing the store. Every write verb it issues
then carries an `attester` block, and the store roots itself in that
key's own namespace, `/memories/by_attester/<pubkey8>/`, which no other
key can write to. See `signing.py` for the preimage.

Namespaces map to path components under that root. A store signing with
pubkey8 `q4f2ab7x` answers `mget(("user","u1"), "note")` from
`/memories/by_attester/q4f2ab7x/user/u1/note`. Without a signing key the
root stays `/memories` and writes go out unattested, which works only
against a responder the operator has opened with `EMEM_MEMORY_OPEN=1`.
A key string beginning with `/` is always taken as a literal absolute
path and is left alone, which is the escape hatch for reading or writing
outside the store's root.

Bytes round-trip as UTF-8 unless they decode to non-text, in which case
they are base64-wrapped in transit and reloaded as raw bytes.
"""

from __future__ import annotations

import json
import os
import uuid
from collections.abc import AsyncIterator, Iterator, Sequence
from importlib.metadata import PackageNotFoundError, version as _pkg_version
from typing import Optional, Union

import httpx
from langchain_core.stores import BaseStore

from emem_langmem.signing import EmemSigner, coerce_signer

# LangChain BaseStore is generic over (K, V); we standardise on
# (path-string, opaque-bytes) — the most flexible shape for downstream
# users who want to round-trip JSON, pickles, msgpack, etc.
_K = str
_V = bytes

_DEFAULT_BASE = "https://emem.dev"

# Derive the User-Agent version from the installed distribution (pyproject.toml
# is the single source), so it never drifts from the shipped package version.
# Resolved here rather than imported from `emem_langmem/__init__.py`, which
# imports this module.
try:
    _VERSION = _pkg_version("emem-langmem")
except PackageNotFoundError:  # running from a source checkout, not installed
    _VERSION = "0+unknown"
_USER_AGENT = f"emem-langmem/{_VERSION} (+https://emem.dev)"
_MEMORIES_ROOT = "/memories"


class EmemStoreError(RuntimeError):
    """Raised when the emem responder returns a non-OK MCP envelope."""


class EmemAttestationError(EmemStoreError):
    """Raised when the responder refuses a write over its attester
    binding: no signature where one is required, a signature that does
    not verify, or a path owned by a different key."""


def _path_from_namespace(
    namespace: Sequence[str], key: str, root: str = _MEMORIES_ROOT
) -> str:
    """Map (namespace, key) to an emem memory file path under `root`."""
    parts = [p.strip("/") for p in (*namespace, key) if p]
    return f"{root.rstrip('/')}/" + "/".join(parts)


def _key_to_path(
    key: Union[_K, tuple[Sequence[str], str]], root: str = _MEMORIES_ROOT
) -> str:
    """Accept either a bare path string or LangChain's (namespace, key)
    tuple, and resolve it under `root`.

    A string that already starts with `/` is an absolute emem path and is
    returned untouched, so a caller can still reach outside `root`.
    """
    if isinstance(key, tuple) and len(key) == 2 and not isinstance(key[0], str):
        namespace, k = key
        return _path_from_namespace(namespace, k, root)
    if isinstance(key, str):
        if key.startswith("/"):
            return key
        return f"{root.rstrip('/')}/{key.lstrip('/')}"
    raise TypeError(f"unsupported key shape: {type(key).__name__}")


def _decode_value(content: object) -> Optional[_V]:
    """emem's memory_view returns the file body in a `content` field that
    may be a string (text), a base64-wrapped blob, or `None` for absent."""
    if content is None:
        return None
    if isinstance(content, (bytes, bytearray)):
        return bytes(content)
    if isinstance(content, str):
        return content.encode("utf-8")
    if isinstance(content, dict) and "base64" in content:
        import base64

        return base64.b64decode(content["base64"])
    # Fallback: serialise as JSON. Useful when the responder returns
    # a structured listing (e.g. directory walk).
    return json.dumps(content, separators=(",", ":")).encode("utf-8")


def _encode_value(value: _V) -> str:
    """Encode bytes for transport. Text bytes pass through as UTF-8;
    anything else is base64-wrapped."""
    if not isinstance(value, (bytes, bytearray)):
        raise TypeError(f"value must be bytes, got {type(value).__name__}")
    try:
        return value.decode("utf-8")
    except UnicodeDecodeError:
        import base64

        return base64.b64encode(value).decode("ascii")


class EmemStore(BaseStore[_K, _V]):
    """LangChain `BaseStore` backed by emem MCP memory file ops.

    Parameters
    ----------
    base_url:
        Origin of the emem responder. Defaults to `EMEM_BASE_URL` env var
        or `https://emem.dev`.
    signing_key:
        The caller's ed25519 key, as a raw 32-byte seed or a ready
        `EmemSigner`. With a key, every write carries an `attester` block
        and the store roots itself at `/memories/by_attester/<pubkey8>`.
        Without one, writes go out unattested and a responder running the
        default policy will refuse them.
    timeout:
        Per-request timeout in seconds (default 30).
    default_kind:
        CoALA memory kind for `mset` writes. One of
        `episodic / semantic / procedural / resource`. Default `resource`.
    headers:
        Extra HTTP headers (e.g. for self-hosted auth proxies).

    Attributes
    ----------
    root:
        The path every relative key resolves under.
    signer:
        The `EmemSigner` in use, or None for an unsigned store.
    """

    def __init__(
        self,
        base_url: Optional[str] = None,
        *,
        signing_key: Union[EmemSigner, bytes, bytearray, None] = None,
        timeout: float = 30.0,
        default_kind: str = "resource",
        headers: Optional[dict[str, str]] = None,
    ) -> None:
        self.base_url = (base_url or os.environ.get("EMEM_BASE_URL") or _DEFAULT_BASE).rstrip("/")
        self.default_kind = default_kind
        self.signer: Optional[EmemSigner] = (
            coerce_signer(signing_key) if signing_key is not None else None
        )
        # A signing key gets its own namespace, which is the only space
        # the responder will let it write. No key means the old open root.
        self.root = self.signer.namespace_root if self.signer else _MEMORIES_ROOT
        self._headers = {"user-agent": _USER_AGENT, **(headers or {})}
        self._client = httpx.Client(timeout=timeout, headers=self._headers)
        self._async_client: Optional[httpx.AsyncClient] = None
        self._async_timeout = timeout

    # ---------- paths + signing ----------

    def _path(self, key: Union[_K, tuple[Sequence[str], str]]) -> str:
        return _key_to_path(key, self.root)

    def _write_args(self, verb: str, path: str, body: bytes, extra: dict) -> dict:
        """Assemble MCP arguments for a write verb, signing when the
        store holds a key. `body` must be the bytes the responder hashes
        for this verb (see `signing.py`)."""
        args = {"path": path, **extra}
        if self.signer is not None:
            args["attester"] = self.signer.attester_block(verb, path, body)
        return args

    # ---------- transport ----------

    def _envelope(self, tool: str, arguments: dict) -> dict:
        return {
            "jsonrpc": "2.0",
            "id": str(uuid.uuid4()),
            "method": "tools/call",
            "params": {"name": tool, "arguments": arguments},
        }

    def _mcp_call(self, tool: str, arguments: dict) -> dict:
        r = self._client.post(f"{self.base_url}/mcp", json=self._envelope(tool, arguments))
        r.raise_for_status()
        return _unwrap_mcp(r.json(), tool, signer=self.signer)

    async def _mcp_call_async(self, tool: str, arguments: dict) -> dict:
        if self._async_client is None:
            self._async_client = httpx.AsyncClient(
                timeout=self._async_timeout, headers=self._headers
            )
        r = await self._async_client.post(
            f"{self.base_url}/mcp", json=self._envelope(tool, arguments)
        )
        r.raise_for_status()
        return _unwrap_mcp(r.json(), tool, signer=self.signer)

    # ---------- BaseStore sync surface ----------

    def mget(self, keys: Sequence[_K]) -> list[Optional[_V]]:
        out: list[Optional[_V]] = []
        for k in keys:
            path = self._path(k)
            try:
                resp = self._mcp_call("emem_memory_view", {"path": path})
            except EmemStoreError as e:
                if "not_found" in str(e).lower():
                    out.append(None)
                    continue
                raise
            out.append(_decode_value(resp.get("content")))
        return out

    def mset(self, key_value_pairs: Sequence[tuple[_K, _V]]) -> None:
        for k, v in key_value_pairs:
            # Anthropic memory-tool spec field name is `file_text`, not
            # `content`, confirmed against the emem responder which
            # returns `tool error (-32602): missing field 'file_text'`
            # when the wrong field is sent.
            file_text = _encode_value(v)
            self._mcp_call(
                "emem_memory_create",
                self._write_args(
                    "create",
                    self._path(k),
                    # The responder hashes `file_text.as_bytes()`, so the
                    # signature covers the encoded string, not `v`. These
                    # differ whenever `v` was base64-wrapped.
                    file_text.encode("utf-8"),
                    {"file_text": file_text, "kind": self.default_kind},
                ),
            )

    def mdelete(self, keys: Sequence[_K]) -> None:
        for k in keys:
            # delete carries no body: the responder hashes b"".
            self._mcp_call(
                "emem_memory_delete", self._write_args("delete", self._path(k), b"", {})
            )

    def yield_keys(self, *, prefix: Optional[str] = None) -> Iterator[_K]:
        path = self._path(prefix) if prefix else self.root
        resp = self._mcp_call("emem_memory_view", {"path": path})
        for entry in _walk_view(resp):
            yield entry

    # ---------- BaseStore async surface ----------

    async def amget(self, keys: Sequence[_K]) -> list[Optional[_V]]:
        out: list[Optional[_V]] = []
        for k in keys:
            path = self._path(k)
            try:
                resp = await self._mcp_call_async("emem_memory_view", {"path": path})
            except EmemStoreError as e:
                if "not_found" in str(e).lower():
                    out.append(None)
                    continue
                raise
            out.append(_decode_value(resp.get("content")))
        return out

    async def amset(self, key_value_pairs: Sequence[tuple[_K, _V]]) -> None:
        for k, v in key_value_pairs:
            # See note on mset: field is `file_text`, and the signature
            # covers the encoded string the responder hashes.
            file_text = _encode_value(v)
            await self._mcp_call_async(
                "emem_memory_create",
                self._write_args(
                    "create",
                    self._path(k),
                    file_text.encode("utf-8"),
                    {"file_text": file_text, "kind": self.default_kind},
                ),
            )

    async def amdelete(self, keys: Sequence[_K]) -> None:
        for k in keys:
            await self._mcp_call_async(
                "emem_memory_delete", self._write_args("delete", self._path(k), b"", {})
            )

    async def ayield_keys(self, *, prefix: Optional[str] = None) -> AsyncIterator[_K]:
        path = self._path(prefix) if prefix else self.root
        resp = await self._mcp_call_async("emem_memory_view", {"path": path})
        for entry in _walk_view(resp):
            yield entry

    # ---------- lifecycle ----------

    def close(self) -> None:
        self._client.close()
        if self._async_client is not None:
            try:
                import asyncio

                asyncio.get_event_loop().run_until_complete(self._async_client.aclose())
            except Exception:
                pass

    def __enter__(self) -> "EmemStore":
        return self

    def __exit__(self, *_exc: object) -> None:
        self.close()


# ---------- helpers ----------


def _is_attestation_error(message: object) -> bool:
    """The responder's typed `memory_attestation_*` details are dropped
    by the MCP layer, which forwards only the message text. Every one of
    those messages names the `attester` block, so that is the marker."""
    return isinstance(message, str) and "attester" in message.lower()


def _attestation_hint(signer: Optional[EmemSigner]) -> str:
    if signer is None:
        return (
            " This EmemStore has no signing key, so its writes are unattested, and the "
            "responder refuses those by default. Pass a key: "
            "EmemStore(signing_key=<raw 32-byte ed25519 seed>), which also roots the store "
            "in that key's own /memories/by_attester/<pubkey8>/ space. If you meant to write "
            "unattested, the operator has to run the responder with EMEM_MEMORY_OPEN=1."
        )
    return (
        f" This EmemStore signed the write with pubkey {signer.pubkey_short!r} and the "
        f"responder still rejected it. Check the path is under {signer.namespace_root}/ "
        "(another key's namespace is refused), and that this is the key that namespace "
        "belongs to."
    )


def _unwrap_mcp(envelope: dict, tool: str, *, signer: Optional[EmemSigner] = None) -> dict:
    if "error" in envelope and envelope["error"] is not None:
        err = envelope["error"]
        message = f"emem {tool}: {err.get('code')} {err.get('message')}"
        if _is_attestation_error(err.get("message")):
            raise EmemAttestationError(message + _attestation_hint(signer))
        raise EmemStoreError(message)
    result = envelope.get("result")
    if not isinstance(result, dict):
        raise EmemStoreError(f"emem {tool}: malformed result envelope")
    # MCP `tools/call` wraps the tool's actual return inside
    # `result.content[0].text` (text content) or `result.structuredContent`.
    if "structuredContent" in result:
        return result["structuredContent"]
    content = result.get("content")
    if isinstance(content, list) and content:
        first = content[0]
        if first.get("type") == "text":
            try:
                return json.loads(first["text"])
            except (KeyError, json.JSONDecodeError):
                return {"content": first.get("text")}
    return result


def _walk_view(resp: dict) -> Iterator[str]:
    """memory_view on a directory returns `entries: [{path, kind, ...}]`.
    On a file it returns `content`. We yield paths for directory walks
    and a single path for file responses."""
    entries = resp.get("entries")
    if isinstance(entries, list):
        for entry in entries:
            if isinstance(entry, dict) and "path" in entry:
                yield entry["path"]
    elif resp.get("path"):
        yield resp["path"]
