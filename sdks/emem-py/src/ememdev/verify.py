"""Verify a receipt here, in this process, without asking the responder.

Why this module exists
----------------------
`Client.verify_receipt()` posts to `/v1/verify_receipt`. That endpoint is
useful and it is not verification: it asks emem.dev whether emem.dev's own
signature is good. A caller who wanted to know that the responder is honest
has just asked the responder. The whole argument of the protocol is that you
do not have to.

The ingredients were already installed. `blake3` and `cryptography` ship as
the `signing` extra and are used to SIGN writes; nothing pointed them at
reading. This module points them the other way.

What it does
------------
Rebuilds the canonical preimage under the rule the receipt's own
`preimage_version` names, hashes it with blake3, and checks the ed25519
signature against the responder key carried in the receipt. No network. The
same algorithm the browser verifier runs at emem.dev/verify, ported from
`web/emem-verify-core.js` rather than derived a second time.

The self-test is not optional
-----------------------------
`self_test()` replays vectors emitted by the Rust signer, and
`verify_receipt_offline` refuses to return a verdict when it fails. A drifted
encoder can only compute a wrong digest, and a wrong digest reads as a forged
receipt against one that is perfectly sound. Telling a caller their genuine
data was tampered with is a worse failure than declining to check it.

    from ememdev.verify import verify_receipt_offline
    v = verify_receipt_offline(resp["receipt"])
    if not v.ok:
        raise RuntimeError(v.why)
"""

from __future__ import annotations

import base64
from dataclasses import dataclass
from typing import Any, Mapping, Sequence

__all__ = ["Verdict", "verify_receipt_offline", "self_test", "CRYPTO_AVAILABLE"]

try:  # the `signing` extra; absent in a bare install
    import blake3 as _blake3_mod
    from cryptography.exceptions import InvalidSignature
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

    CRYPTO_AVAILABLE = True
except Exception:  # pragma: no cover - exercised by installs without the extra
    CRYPTO_AVAILABLE = False


def _b3(data: bytes) -> bytes:
    return _blake3_mod.blake3(data).digest()


def _hex(b: bytes) -> str:
    return b.hex()


def _b32decode(s: str) -> bytes:
    s = s.strip().rstrip("=").upper()
    return base64.b32decode(s + "=" * ((8 - len(s) % 8) % 8))


# ---------------------------------------------------------------- CBOR bits
# Only the four shapes a preimage needs, reproducing ciborium's canonical
# output: definite-length text, unsigned ints in shortest form, a definite
# array of text, and a definite map of string->string in a given key order.


def _cbor_head(major: int, n: int, out: bytearray) -> None:
    if n < 24:
        out.append((major << 5) | n)
    elif n < 0x100:
        out.append((major << 5) | 24)
        out.append(n)
    elif n < 0x10000:
        out.append((major << 5) | 25)
        out += n.to_bytes(2, "big")
    elif n < 0x100000000:
        out.append((major << 5) | 26)
        out += n.to_bytes(4, "big")
    else:
        out.append((major << 5) | 27)
        out += n.to_bytes(8, "big")


def _cbor_text(s: str, out: bytearray) -> None:
    b = s.encode()
    _cbor_head(3, len(b), out)
    out += b


def _cbor_uint(n: int, out: bytearray) -> None:
    _cbor_head(0, int(n), out)


def _cbor_array_text(items: Sequence[str], out: bytearray) -> None:
    _cbor_head(4, len(items), out)
    for s in items:
        _cbor_text(s, out)


def _cbor_map_str(pairs: Sequence[tuple], out: bytearray) -> None:
    _cbor_head(5, len(pairs), out)
    for k, v in pairs:
        _cbor_text(k, out)
        _cbor_text(v, out)


# ------------------------------------------------------- v1 segment layout
# Tags mirror emem-attest::receipt_tag / merkle_tag. Order is load-bearing:
# FIELD and MERKLE are appended AFTER fact_cids so a receipt without them
# hashes identically to one signed before those tags existed.

_RT = dict(
    REQUEST_ID=1, SERVED_AT=2, SCOPE=3, AS_OF=4, EDGES=5, MANIFEST=6,
    PRIMITIVE=7, CELLS=8, FACT_CIDS=9, FIELD=10, MERKLE=11,
)
_MT = dict(ROOT=1, LEAF_INDEX=2, PATH=3, RULE_VERSION=4, ABSENT=5)
_DOMAIN = b"emem.preimage.v1\x00"


def _u32le(n: int, out: bytearray) -> None:
    out += int(n).to_bytes(4, "little")


def _seg(tag: int, s: str, out: bytearray) -> None:
    b = s.encode()
    out.append(tag)
    _u32le(len(b), out)
    out += b


def _seg_list(tag: int, items: Sequence[str], out: bytearray) -> None:
    out.append(tag)
    _u32le(len(items), out)
    for s in items:
        b = s.encode()
        _u32le(len(b), out)
        out += b


def _seg_bytes(tag: int, data: bytes, out: bytearray) -> None:
    out.append(tag)
    _u32le(len(data), out)
    out += data


def _start(kind: str) -> bytearray:
    out = bytearray(_DOMAIN)
    d = kind.encode()
    _u32le(len(d), out)
    out += d
    return out


def _bytes_of(v: Any) -> bytes:
    if isinstance(v, (bytes, bytearray)):
        return bytes(v)
    if isinstance(v, list):
        return bytes(int(x) & 0xFF for x in v)
    if isinstance(v, str):
        try:
            return bytes.fromhex(v)
        except ValueError:
            return _b32decode(v)
    return b""


# ------------------------------------------------------------ sub-digests


def _manifest_hex(r: Mapping[str, Any]) -> str | None:
    sv = r.get("source_versions") or {}
    keys = sorted(sv)
    if not keys:
        return None
    out = bytearray()
    _cbor_map_str([(k, str(sv[k])) for k in keys], out)
    return _hex(_b3(bytes(out)))


def _edges_hex(r: Mapping[str, Any]) -> str | None:
    eds = sorted(str(x) for x in (r.get("edge_cids") or []))
    if not eds:
        return None
    out = bytearray()
    _cbor_array_text(eds, out)
    return _hex(_b3(bytes(out)))


def _scope_hex(r: Mapping[str, Any]) -> str | None:
    sc = r.get("scope")
    if not sc:
        return None
    pairs = [(k, str(sc[k])) for k in ("user_id", "agent_id", "run_id", "org_id")
             if sc.get(k) is not None]
    if not pairs:
        return None
    out = bytearray()
    _cbor_map_str(pairs, out)
    return _hex(_b3(bytes(out)))


def _as_of_hex(r: Mapping[str, Any]) -> str | None:
    a = r.get("as_of")
    if not a:
        return None
    n = (a.get("valid_time") is not None) + (a.get("transaction_time") is not None)
    if n == 0:
        return None
    out = bytearray()
    _cbor_head(5, n, out)
    if a.get("valid_time") is not None:
        _cbor_text("valid_time", out)
        _cbor_uint(a["valid_time"], out)
    if a.get("transaction_time") is not None:
        _cbor_text("transaction_time", out)
        _cbor_text(str(a["transaction_time"]), out)
    return _hex(_b3(bytes(out)))


def _field_hex(r: Mapping[str, Any]) -> str | None:
    f = r.get("field")
    if not f:
        return None
    out = _start("field")
    _seg(1, f.get("aoi_cid") or "", out)
    _seg(2, f.get("derivation_cid") or "", out)
    return _hex(_b3(bytes(out)))


def _merkle_binding_v2_hex(r: Mapping[str, Any]) -> str:
    out = _start("merkle")
    p = r.get("merkle_proof")
    if not p:
        # An explicit absence marker, not a skipped segment. If absence were
        # encoded by omission, stripping a proof would produce the same digest
        # as a receipt that never had one, and stripping would stay invisible.
        _seg_bytes(_MT["ABSENT"], b"", out)
    else:
        _seg_bytes(_MT["ROOT"], _bytes_of(p.get("root")), out)
        _seg_bytes(_MT["LEAF_INDEX"], int(p.get("leaf_index") or 0).to_bytes(4, "little"), out)
        path = b"".join(_bytes_of(h) for h in (p.get("path") or []))
        _seg_bytes(_MT["PATH"], path, out)
        _seg_bytes(_MT["RULE_VERSION"], bytes([int(p.get("version") or 0)]), out)
    return _hex(_b3(bytes(out)))


def _preimage_v1(r: Mapping[str, Any]) -> bytes:
    out = _start("receipt")
    _seg(_RT["REQUEST_ID"], r.get("request_id") or "", out)
    _seg(_RT["SERVED_AT"], r.get("served_at") or "", out)
    for tag, h in (
        (_RT["SCOPE"], _scope_hex(r)),
        (_RT["AS_OF"], _as_of_hex(r)),
        (_RT["EDGES"], _edges_hex(r)),
        (_RT["MANIFEST"], _manifest_hex(r)),
    ):
        if h:
            _seg(tag, h, out)
    _seg(_RT["PRIMITIVE"], r.get("primitive") or "", out)
    _seg_list(_RT["CELLS"], [str(c) for c in (r.get("cells") or [])], out)
    _seg_list(_RT["FACT_CIDS"], [str(c) for c in (r.get("fact_cids") or [])], out)
    fh = _field_hex(r)
    if fh:
        _seg(_RT["FIELD"], fh, out)
    if int(r.get("preimage_version") or 0) >= 2:
        _seg(_RT["MERKLE"], _merkle_binding_v2_hex(r), out)
    return bytes(out)


def _preimage_v0(r: Mapping[str, Any]) -> bytes:
    parts: list[bytes] = []
    parts.append((r.get("request_id") or "").encode())
    parts.append(b"|")
    parts.append((r.get("served_at") or "").encode())
    parts.append(b"|")
    for h in (_scope_hex(r), _as_of_hex(r), _edges_hex(r), _manifest_hex(r)):
        if h:
            parts.append(h.encode())
            parts.append(b"|")
    parts.append((r.get("primitive") or "").encode())
    parts.append(b"|")
    for c in (r.get("cells") or []):
        parts.append(str(c).encode())
        parts.append(b",")
    parts.append(b"|")
    for c in (r.get("fact_cids") or []):
        parts.append(str(c).encode())
        parts.append(b",")
    return b"".join(parts)


def _preimage(r: Mapping[str, Any]) -> bytes:
    return _preimage_v1(r) if int(r.get("preimage_version") or 0) >= 1 else _preimage_v0(r)


def _sig_bytes(r: Mapping[str, Any]) -> bytes | None:
    for key in ("signature", "sig_b32", "signature_b32"):
        v = r.get(key)
        if v is None:
            continue
        b = _bytes_of(v)
        if b:
            return b
    return None


def _pub_bytes(r: Mapping[str, Any]) -> bytes | None:
    if r.get("responder_pubkey_b32"):
        return _b32decode(r["responder_pubkey_b32"])
    v = r.get("responder")
    return _bytes_of(v) if v is not None else None


# ------------------------------------------------------------- self-test
# Ground truth emitted by the Rust signer. Identical to the vectors the
# browser verifier replays; if these move, both must move together.

_VECTORS = {
    "manifest": "de14467c03ed214d08ad536ba2923fed93dfd1d2af63c74d1b08131b00ea3915",
    "edges": "5a27c406ddc3b90b10c07edd1c0902a4901ede4c0efc48d6d792c92f85d8cca6",
    "scope": "6306e69a1f2e0df252a01f76785440c4f3d0704b3872a01bda85446256e8d45c",
    "as_of": "89593e75f23de38f21e520b65c8b86a032cd83f48335bbf95cb7b2b7debbc994",
}


def self_test() -> bool:
    """Do these encoders still agree with the Rust signer?"""
    if not CRYPTO_AVAILABLE:
        return False
    try:
        checks = (
            _manifest_hex({"source_versions": {"bands_cid": "bbb", "registry_cid": "rrr",
                                               "schema_cid": "sss", "sources_cid": "ooo"}})
            == _VECTORS["manifest"],
            _edges_hex({"edge_cids": ["e2", "e1"]}) == _VECTORS["edges"],
            _scope_hex({"scope": {"user_id": "u1", "run_id": "r9"}}) == _VECTORS["scope"],
            _as_of_hex({"as_of": {"valid_time": 1767225600,
                                  "transaction_time": "2026-05-29T00:00:00Z"}})
            == _VECTORS["as_of"],
        )
        return all(checks)
    except Exception:
        return False


@dataclass(frozen=True)
class Verdict:
    """The result of checking, and never a claim that checking happened."""

    ok: bool
    state: str
    why: str
    digest_hex: str | None = None
    signer_b32: str | None = None
    preimage_version: int | None = None

    def __bool__(self) -> bool:
        return self.ok


def verify_receipt_offline(receipt: Mapping[str, Any]) -> Verdict:
    """Check a receipt in this process. Never raises; never asks the server.

    `ok` is True only when this code rebuilt the digest and ed25519 accepted
    the signature over it. Every other outcome names itself, so a caller can
    never render a pass for a state where no cryptography ran.
    """
    if not CRYPTO_AVAILABLE:
        return Verdict(False, "crypto_unavailable",
                       "install the signing extra (pip install 'ememdev[signing]') "
                       "to verify locally; nothing was checked")
    if not self_test():
        return Verdict(False, "self_test_failed",
                       "these encoders disagree with the vectors the Rust signer "
                       "emitted, so any digest computed here would be wrong. "
                       "Nothing was checked.")
    if not isinstance(receipt, Mapping):
        return Verdict(False, "no_receipt", "no receipt to check")
    try:
        digest = _b3(_preimage(receipt))
        sig = _sig_bytes(receipt)
        pub = _pub_bytes(receipt)
        if not sig or not pub:
            return Verdict(False, "incomplete",
                           "the receipt carries no signature or no responder key")
        try:
            Ed25519PublicKey.from_public_bytes(pub).verify(sig, digest)
        except InvalidSignature:
            return Verdict(False, "signature_rejected",
                           "the signature does not match the digest computed from "
                           "the receipt's own fields",
                           _hex(digest), receipt.get("responder_pubkey_b32"),
                           int(receipt.get("preimage_version") or 0))
        return Verdict(True, "verified",
                       "rebuilt the canonical preimage, hashed it with blake3, and "
                       "ed25519 accepted the signature against the responder key in "
                       "the receipt",
                       _hex(digest), receipt.get("responder_pubkey_b32"),
                       int(receipt.get("preimage_version") or 0))
    except Exception as e:  # a malformed receipt is a refusal, not a crash
        return Verdict(False, "error", f"{type(e).__name__}: {e}")
