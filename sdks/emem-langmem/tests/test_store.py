"""Smoke tests for EmemStore: pure shape tests using respx for the
HTTP boundary. No live emem responder needed."""

import base64
import json

import httpx
import pytest
import respx

from emem_langmem import EmemAttestationError, EmemSigner, EmemStore, EmemStoreError
from emem_langmem.signing import attester_preimage, body_hash
from emem_langmem.store import _key_to_path, _decode_value, _encode_value

# Fixed, throwaway seed so the derived namespace is stable. Test-only.
_SEED = bytes(range(32))
_SIGNER = EmemSigner(_SEED)


def _ok(request: httpx.Request) -> httpx.Response:
    return httpx.Response(
        200,
        json={"jsonrpc": "2.0", "id": "x", "result": {"structuredContent": {"ok": True}}},
    )


def _capture_into(box: dict):
    def _fn(request: httpx.Request) -> httpx.Response:
        box["body"] = json.loads(request.content)
        return _ok(request)

    return _fn


def _args_of(box: dict) -> dict:
    return box["body"]["params"]["arguments"]


def test_path_mapping_bare_string():
    assert _key_to_path("hello") == "/memories/hello"
    assert _key_to_path("/memories/abs/path.txt") == "/memories/abs/path.txt"


def test_path_mapping_namespace_tuple():
    assert _key_to_path((["user", "u1"], "note")) == "/memories/user/u1/note"


def test_path_mapping_under_an_attester_root():
    """The namespace mapping is preserved, just re-rooted."""
    root = _SIGNER.namespace_root
    assert _key_to_path("hello", root) == f"{root}/hello"
    assert _key_to_path((["user", "u1"], "note"), root) == f"{root}/user/u1/note"
    # An absolute key stays an escape hatch out of the root.
    assert _key_to_path("/memories/elsewhere.txt", root) == "/memories/elsewhere.txt"


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
def test_mset_dispatches_emem_memory_create():
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
    # Canonical name. The bare `memory_create` still dispatches server-side
    # until 3.0, but this SDK should send the name that outlives the alias.
    assert captured["body"]["params"]["name"] == "emem_memory_create"
    assert captured["body"]["params"]["arguments"]["path"] == "/memories/my-note"
    assert captured["body"]["params"]["arguments"]["file_text"] == "hello"
    assert captured["body"]["params"]["arguments"]["kind"] == "resource"


@respx.mock
def test_mdelete_dispatches_emem_memory_delete():
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

    assert captured["body"]["params"]["name"] == "emem_memory_delete"
    assert captured["body"]["params"]["arguments"]["path"] == "/memories/my-note"


# ---------- signed writes ----------


def _verify_block(block: dict, verb: str, path: str, body: bytes) -> None:
    """Check an attester block the way the responder does."""
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

    def _b32d(s: str) -> bytes:
        return base64.b32decode(s.upper() + "=" * (-len(s) % 8))

    Ed25519PublicKey.from_public_bytes(_b32d(block["pubkey_b32"])).verify(
        _b32d(block["sig_b32"]), attester_preimage(verb, path, body_hash(body))
    )


def test_signing_key_reroots_the_store():
    store = EmemStore(base_url="https://emem.dev", signing_key=_SEED)
    assert store.root == _SIGNER.namespace_root
    assert store.root.startswith("/memories/by_attester/")
    assert store.signer.pubkey_b32 == _SIGNER.pubkey_b32
    # A ready-made signer is accepted too.
    assert EmemStore(signing_key=_SIGNER).root == _SIGNER.namespace_root
    # No key keeps the old open root.
    assert EmemStore(base_url="https://emem.dev").root == "/memories"


@respx.mock
def test_mset_signs_and_lands_in_the_owner_namespace():
    captured = {}
    respx.post("https://emem.dev/mcp").mock(side_effect=_capture_into(captured))
    store = EmemStore(base_url="https://emem.dev", signing_key=_SEED)
    store.mset([("my-note", b"hello")])

    args = _args_of(captured)
    assert args["path"] == f"{_SIGNER.namespace_root}/my-note"
    assert args["file_text"] == "hello"
    assert args["attester"]["pubkey_b32"] == _SIGNER.pubkey_b32
    # The path segment must match the signing key or the responder 403s.
    assert args["path"].split("/")[3] == _SIGNER.pubkey_short
    _verify_block(args["attester"], "create", args["path"], b"hello")


@respx.mock
def test_mset_signs_the_encoded_file_text_not_the_raw_value():
    """Binary values are base64-wrapped into `file_text`, and the
    responder hashes `file_text.as_bytes()`. Signing the pre-encoding
    bytes would 401 on every binary write."""
    captured = {}
    respx.post("https://emem.dev/mcp").mock(side_effect=_capture_into(captured))
    raw = bytes(range(256))  # not valid UTF-8, so it gets base64-wrapped
    EmemStore(base_url="https://emem.dev", signing_key=_SEED).mset([("blob", raw)])

    args = _args_of(captured)
    assert base64.b64decode(args["file_text"]) == raw
    _verify_block(args["attester"], "create", args["path"], args["file_text"].encode("utf-8"))


@respx.mock
def test_mdelete_signs_with_an_empty_body_hash():
    captured = {}
    respx.post("https://emem.dev/mcp").mock(side_effect=_capture_into(captured))
    EmemStore(base_url="https://emem.dev", signing_key=_SEED).mdelete(["my-note"])

    args = _args_of(captured)
    assert args["path"] == f"{_SIGNER.namespace_root}/my-note"
    _verify_block(args["attester"], "delete", args["path"], b"")


@respx.mock
def test_unsigned_store_sends_no_attester():
    captured = {}
    respx.post("https://emem.dev/mcp").mock(side_effect=_capture_into(captured))
    EmemStore(base_url="https://emem.dev").mset([("my-note", b"hello")])

    assert "attester" not in _args_of(captured)
    assert _args_of(captured)["path"] == "/memories/my-note"


@respx.mock
def test_yield_keys_walks_the_store_root():
    captured = {}

    def _listing(request: httpx.Request):
        captured["body"] = json.loads(request.content)
        return httpx.Response(
            200,
            json={
                "jsonrpc": "2.0",
                "id": "x",
                "result": {"structuredContent": {"entries": [{"path": "/memories/a"}]}},
            },
        )

    respx.post("https://emem.dev/mcp").mock(side_effect=_listing)
    store = EmemStore(base_url="https://emem.dev", signing_key=_SEED)
    assert list(store.yield_keys()) == ["/memories/a"]
    assert _args_of(captured)["path"] == _SIGNER.namespace_root


# ---------- the refusal is legible ----------


def _refusal(message: str):
    return httpx.Response(
        200,
        json={"jsonrpc": "2.0", "id": "x", "error": {"code": -3, "message": message}},
    )


_REQUIRED_MSG = (
    "unattested `create` is refused by this responder's memory-write policy; supply an "
    "`attester: {pubkey_b32, sig_b32}` block (see /memories/by_attester/<pubkey8>/...), "
    "or the operator can relax the policy."
)


@respx.mock
def test_unsigned_write_refusal_names_the_fix():
    respx.post("https://emem.dev/mcp").mock(return_value=_refusal(_REQUIRED_MSG))
    store = EmemStore(base_url="https://emem.dev")

    with pytest.raises(EmemAttestationError) as ei:
        store.mset([("my-note", b"hello")])

    msg = str(ei.value)
    assert "signing_key" in msg
    assert "EMEM_MEMORY_OPEN=1" in msg
    assert _REQUIRED_MSG in msg  # the responder's own words survive


@respx.mock
def test_signed_write_refusal_points_at_the_namespace():
    respx.post("https://emem.dev/mcp").mock(
        return_value=_refusal(
            "memory_attestation_invalid: signature verified, but path is under a "
            "different attester's namespace."
        )
    )
    store = EmemStore(base_url="https://emem.dev", signing_key=_SEED)

    with pytest.raises(EmemAttestationError) as ei:
        store.mset([("my-note", b"hello")])

    msg = str(ei.value)
    assert _SIGNER.pubkey_short in msg
    assert _SIGNER.namespace_root in msg
    # It should not tell a caller who already has a key to go get one.
    assert "EMEM_MEMORY_OPEN=1" not in msg


@respx.mock
def test_unrelated_errors_are_not_relabelled_as_attestation():
    """A write failing for some other reason must stay a plain
    EmemStoreError, with no signing advice bolted on."""
    respx.post("https://emem.dev/mcp").mock(
        return_value=_refusal("memory tool persistence requires a sled-backed hot cache")
    )
    store = EmemStore(base_url="https://emem.dev", signing_key=_SEED)

    with pytest.raises(EmemStoreError) as ei:
        store.mset([("my-note", b"hello")])

    assert not isinstance(ei.value, EmemAttestationError)
    assert "signing_key" not in str(ei.value)


@respx.mock
def test_mget_still_reports_a_miss_as_none():
    respx.post("https://emem.dev/mcp").mock(return_value=_refusal("not_found: no such path"))
    store = EmemStore(base_url="https://emem.dev", signing_key=_SEED)
    assert store.mget(["missing"]) == [None]
