"""`EmemStore` — LangChain `BaseStore[str, bytes]` over emem MCP.

The four `BaseStore` verbs map onto the six Anthropic memory-tool MCP
tools served by emem:

| BaseStore verb | MCP tool          | Notes                                  |
|----------------|-------------------|----------------------------------------|
| `mget(keys)`   | `memory_view`     | Returns file content as bytes          |
| `mset(pairs)`  | `memory_create`   | `kind` defaults to `"resource"`        |
| `mdelete(ks)`  | `memory_delete`   |                                        |
| `yield_keys`   | `memory_view` (d) | Recursive walk under prefix            |

Namespaces map to path components. LangChain calls `mget(("ns","key"))`;
emem stores it at `/memories/ns/key`. Top-level namespace `by_attester/`
triggers capability binding on the emem side (see emem
`docs/memory.md`).

Bytes round-trip as UTF-8 unless they decode to non-text, in which case
they are persisted as `application/octet-stream` and reloaded as raw
bytes.
"""

from __future__ import annotations

import json
import os
import uuid
from collections.abc import AsyncIterator, Iterator, Sequence
from typing import Optional, Union

import httpx
from langchain_core.stores import BaseStore

# LangChain BaseStore is generic over (K, V); we standardise on
# (path-string, opaque-bytes) — the most flexible shape for downstream
# users who want to round-trip JSON, pickles, msgpack, etc.
_K = str
_V = bytes

_DEFAULT_BASE = "https://emem.dev"
_USER_AGENT = "emem-langmem/0.0.8 (+https://emem.dev)"
_MEMORIES_ROOT = "/memories"


class EmemStoreError(RuntimeError):
    """Raised when the emem responder returns a non-OK MCP envelope."""


def _path_from_namespace(namespace: Sequence[str], key: str) -> str:
    """Map (namespace, key) to an emem memory file path."""
    parts = [p.strip("/") for p in (*namespace, key) if p]
    return f"{_MEMORIES_ROOT}/" + "/".join(parts)


def _key_to_path(key: Union[_K, tuple[Sequence[str], str]]) -> str:
    """Accept either a bare path string or LangChain's (namespace, key) tuple."""
    if isinstance(key, tuple) and len(key) == 2 and not isinstance(key[0], str):
        namespace, k = key
        return _path_from_namespace(namespace, k)
    if isinstance(key, str):
        if key.startswith("/"):
            return key
        return f"{_MEMORIES_ROOT}/{key.lstrip('/')}"
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
    timeout:
        Per-request timeout in seconds (default 30).
    default_kind:
        CoALA memory kind for `mset` writes. One of
        `episodic / semantic / procedural / resource`. Default `resource`.
    headers:
        Extra HTTP headers (e.g. for self-hosted auth proxies).
    """

    def __init__(
        self,
        base_url: Optional[str] = None,
        *,
        timeout: float = 30.0,
        default_kind: str = "resource",
        headers: Optional[dict[str, str]] = None,
    ) -> None:
        self.base_url = (base_url or os.environ.get("EMEM_BASE_URL") or _DEFAULT_BASE).rstrip("/")
        self.default_kind = default_kind
        self._headers = {"user-agent": _USER_AGENT, **(headers or {})}
        self._client = httpx.Client(timeout=timeout, headers=self._headers)
        self._async_client: Optional[httpx.AsyncClient] = None
        self._async_timeout = timeout

    # ---------- transport ----------

    def _mcp_call(self, tool: str, arguments: dict) -> dict:
        req = {
            "jsonrpc": "2.0",
            "id": str(uuid.uuid4()),
            "method": "tools/call",
            "params": {"name": tool, "arguments": arguments},
        }
        r = self._client.post(f"{self.base_url}/mcp", json=req)
        r.raise_for_status()
        return _unwrap_mcp(r.json(), tool)

    async def _mcp_call_async(self, tool: str, arguments: dict) -> dict:
        if self._async_client is None:
            self._async_client = httpx.AsyncClient(
                timeout=self._async_timeout, headers=self._headers
            )
        req = {
            "jsonrpc": "2.0",
            "id": str(uuid.uuid4()),
            "method": "tools/call",
            "params": {"name": tool, "arguments": arguments},
        }
        r = await self._async_client.post(f"{self.base_url}/mcp", json=req)
        r.raise_for_status()
        return _unwrap_mcp(r.json(), tool)

    # ---------- BaseStore sync surface ----------

    def mget(self, keys: Sequence[_K]) -> list[Optional[_V]]:
        out: list[Optional[_V]] = []
        for k in keys:
            path = _key_to_path(k)
            try:
                resp = self._mcp_call("memory_view", {"path": path})
            except EmemStoreError as e:
                if "not_found" in str(e).lower():
                    out.append(None)
                    continue
                raise
            out.append(_decode_value(resp.get("content")))
        return out

    def mset(self, key_value_pairs: Sequence[tuple[_K, _V]]) -> None:
        for k, v in key_value_pairs:
            path = _key_to_path(k)
            self._mcp_call(
                "memory_create",
                {
                    "path": path,
                    "content": _encode_value(v),
                    "kind": self.default_kind,
                },
            )

    def mdelete(self, keys: Sequence[_K]) -> None:
        for k in keys:
            path = _key_to_path(k)
            self._mcp_call("memory_delete", {"path": path})

    def yield_keys(self, *, prefix: Optional[str] = None) -> Iterator[_K]:
        path = _key_to_path(prefix) if prefix else _MEMORIES_ROOT
        resp = self._mcp_call("memory_view", {"path": path})
        for entry in _walk_view(resp):
            yield entry

    # ---------- BaseStore async surface ----------

    async def amget(self, keys: Sequence[_K]) -> list[Optional[_V]]:
        out: list[Optional[_V]] = []
        for k in keys:
            path = _key_to_path(k)
            try:
                resp = await self._mcp_call_async("memory_view", {"path": path})
            except EmemStoreError as e:
                if "not_found" in str(e).lower():
                    out.append(None)
                    continue
                raise
            out.append(_decode_value(resp.get("content")))
        return out

    async def amset(self, key_value_pairs: Sequence[tuple[_K, _V]]) -> None:
        for k, v in key_value_pairs:
            path = _key_to_path(k)
            await self._mcp_call_async(
                "memory_create",
                {
                    "path": path,
                    "content": _encode_value(v),
                    "kind": self.default_kind,
                },
            )

    async def amdelete(self, keys: Sequence[_K]) -> None:
        for k in keys:
            path = _key_to_path(k)
            await self._mcp_call_async("memory_delete", {"path": path})

    async def ayield_keys(self, *, prefix: Optional[str] = None) -> AsyncIterator[_K]:
        path = _key_to_path(prefix) if prefix else _MEMORIES_ROOT
        resp = await self._mcp_call_async("memory_view", {"path": path})
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


def _unwrap_mcp(envelope: dict, tool: str) -> dict:
    if "error" in envelope and envelope["error"] is not None:
        err = envelope["error"]
        raise EmemStoreError(f"emem {tool}: {err.get('code')} {err.get('message')}")
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
