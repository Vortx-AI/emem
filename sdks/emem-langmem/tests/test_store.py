"""Smoke tests for EmemStore — pure shape tests using respx for the
HTTP boundary. No live emem responder needed."""

import json

import httpx
import pytest
import respx

from emem_langmem import EmemStore
from emem_langmem.store import _key_to_path, _decode_value, _encode_value


def test_path_mapping_bare_string():
    assert _key_to_path("hello") == "/memories/hello"
    assert _key_to_path("/memories/abs/path.txt") == "/memories/abs/path.txt"


def test_path_mapping_namespace_tuple():
    assert _key_to_path((["user", "u1"], "note")) == "/memories/user/u1/note"


def test_value_round_trip_text():
    encoded = _encode_value(b"hello world")
    assert encoded == "hello world"
    assert _decode_value(encoded) == b"hello world"


def test_value_round_trip_binary():
    raw = bytes(range(256))
    encoded = _encode_value(raw)
    # Should base64-encode non-UTF8
    import base64
    assert base64.b64decode(encoded) == raw


@respx.mock
def test_mget_round_trip():
    respx.post("https://emem.dev/mcp").mock(
        return_value=httpx.Response(
            200,
            json={
                "jsonrpc": "2.0",
                "id": "x",
                "result": {
                    "structuredContent": {"content": "remembered"},
                },
            },
        )
    )
    store = EmemStore(base_url="https://emem.dev")
    out = store.mget(["my-note"])
    assert out == [b"remembered"]


@respx.mock
def test_mset_dispatches_memory_create():
    captured = {}

    def _capture(request: httpx.Request):
        captured["body"] = json.loads(request.content)
        return httpx.Response(
            200,
            json={"jsonrpc": "2.0", "id": "x", "result": {"structuredContent": {"ok": True}}},
        )

    respx.post("https://emem.dev/mcp").mock(side_effect=_capture)
    store = EmemStore(base_url="https://emem.dev")
    store.mset([("my-note", b"hello")])

    assert captured["body"]["method"] == "tools/call"
    assert captured["body"]["params"]["name"] == "memory_create"
    assert captured["body"]["params"]["arguments"]["path"] == "/memories/my-note"
    assert captured["body"]["params"]["arguments"]["file_text"] == "hello"
    assert captured["body"]["params"]["arguments"]["kind"] == "resource"


@respx.mock
def test_mdelete_dispatches_memory_delete():
    captured = {}

    def _capture(request: httpx.Request):
        captured["body"] = json.loads(request.content)
        return httpx.Response(
            200,
            json={"jsonrpc": "2.0", "id": "x", "result": {"structuredContent": {"ok": True}}},
        )

    respx.post("https://emem.dev/mcp").mock(side_effect=_capture)
    store = EmemStore(base_url="https://emem.dev")
    store.mdelete(["my-note"])

    assert captured["body"]["params"]["name"] == "memory_delete"
    assert captured["body"]["params"]["arguments"]["path"] == "/memories/my-note"
