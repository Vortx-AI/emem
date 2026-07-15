"""Deterministic CBOR and run_cid for the playground run-manifest.

The viewer recomputes `run_cid` before it will execute a single step
(UI_CONTRACT_playground_replay.md rule 1, fail-closed), so the recorder in
Python and the viewer in JavaScript have to produce byte-identical encodings of
the same manifest. That is the whole reason this module exists instead of a
one-line call into a CBOR library.

The divergence that actually bites is floats, not key order.

Key order is a non-issue for this manifest and it is worth writing down why, so
nobody "fixes" it later: RFC 7049 §3.9 sorted map keys length-first, RFC 8949
§4.2.1 replaced that with bytewise order over the keys' encoded bytes, but for
text keys shorter than 24 bytes the two rules cannot disagree. The CBOR head
byte for a text string is 0x60+len, so a longer key always starts with a larger
byte and therefore already sorts later bytewise. Every key in this manifest is
short ASCII. (Measured, after I claimed the opposite: a `{"z":1,"aa":2}` probe
puts z first under *both* rules, so it proves nothing on its own.)

Floats do diverge, and silently:

    cbor2.dumps(-0.0, canonical=True)  ->  f9 8000   (keeps the sign bit)
    emem_fact::cbor::canonical_float   ->  f9 0000   (folds -0.0 to 0.0)

emem folds -0.0 and every NaN to one representative so that a content address is
a function of the numeric value rather than a transient bit pattern. A library's
`canonical=True` does not know that rule. Encoding explicitly is how the two
sides stay pinned to emem's meaning of canonical rather than to a library's.

So this encodes RFC 8949 §4.2.1 explicitly:

  - maps: keys sorted by the bytewise lexicographic order of their *encoded*
    bytes, not by the Python value
  - ints: shortest form that holds the value
  - floats: shortest form that round-trips (f16 if exact, else f32, else f64)
  - -0.0 folds to 0.0 and every NaN folds to one quiet NaN, matching
    `emem_fact::cbor::canonical_float`, so the address is a function of the
    numeric value rather than a bit pattern
  - no indefinite-length items, no tags

`TEST_VECTOR` at the bottom is the cross-language contract: any implementation
that reproduces its `run_cid` agrees with this one. Ship it with the schema.
"""

from __future__ import annotations

import math
import struct
from typing import Any

from blake3 import blake3

# ── encoding ──────────────────────────────────────────────────────────────


def _head(major: int, n: int) -> bytes:
    """The initial byte plus argument for `major`, shortest form."""
    if n < 24:
        return bytes([(major << 5) | n])
    if n < 0x100:
        return bytes([(major << 5) | 24, n])
    if n < 0x10000:
        return bytes([(major << 5) | 25]) + struct.pack(">H", n)
    if n < 0x100000000:
        return bytes([(major << 5) | 26]) + struct.pack(">I", n)
    return bytes([(major << 5) | 27]) + struct.pack(">Q", n)


def _encode_float(f: float) -> bytes:
    # Fold the two degenerate cases first, so that content addressing is a
    # function of the value. Mirrors emem_fact::cbor::canonical_float.
    if math.isnan(f):
        return b"\xf9\x7e\x00"  # the one canonical quiet NaN (RFC 8949 §4.2.2)
    if f == 0.0:
        f = 0.0  # folds -0.0, since -0.0 == 0.0 is True
    # Shortest form that round-trips exactly: f16, then f32, then f64.
    try:
        h = struct.pack(">e", f)
        if struct.unpack(">e", h)[0] == f:
            return b"\xf9" + h
    except (OverflowError, struct.error):
        pass
    s = struct.pack(">f", f)
    if struct.unpack(">f", s)[0] == f:
        return b"\xfa" + s
    return b"\xfb" + struct.pack(">d", f)


def dumps(v: Any) -> bytes:
    """RFC 8949 §4.2.1 deterministic encoding of `v`."""
    if v is None:
        return b"\xf6"
    if v is True:
        return b"\xf5"
    if v is False:
        return b"\xf4"
    # bool must precede int: True is an int in Python and would encode as 1.
    if isinstance(v, int):
        return _head(0, v) if v >= 0 else _head(1, -v - 1)
    if isinstance(v, float):
        return _encode_float(v)
    if isinstance(v, str):
        b = v.encode("utf-8")
        return _head(3, len(b)) + b
    if isinstance(v, (bytes, bytearray)):
        return _head(2, len(v)) + bytes(v)
    if isinstance(v, (list, tuple)):
        return _head(4, len(v)) + b"".join(dumps(x) for x in v)
    if isinstance(v, dict):
        # Sort on the ENCODED key, which is what §4.2.1 specifies. For the
        # short ASCII keys used here this coincides with length-first order,
        # but sorting on the Python string would not: it diverges for
        # non-ASCII keys, where str order and UTF-8 byte order differ.
        items = sorted(((dumps(k), dumps(val)) for k, val in v.items()), key=lambda kv: kv[0])
        return _head(5, len(items)) + b"".join(k + val for k, val in items)
    raise TypeError(f"no deterministic encoding for {type(v).__name__}")


# ── addressing ────────────────────────────────────────────────────────────

B32_ALPHABET = "abcdefghijklmnopqrstuvwxyz234567"


def b32_nopad_lc(raw: bytes) -> str:
    """base32-nopad lowercase, the text form every id in emem uses."""
    import base64

    return base64.b32encode(raw).decode("ascii").rstrip("=").lower()


def blake3_b32(data: bytes) -> str:
    return b32_nopad_lc(blake3(data).digest())


def run_cid(manifest: dict) -> str:
    """blake3 of the deterministic CBOR of `manifest` with run_cid removed.

    Removed rather than blanked: a self-referential field cannot be part of the
    preimage of its own digest, and blanking it to "" would still commit to a
    key whose presence the verifier has to know to reproduce.
    """
    m = {k: v for k, v in manifest.items() if k != "run"}
    run = {k: v for k, v in manifest.get("run", {}).items() if k != "run_cid"}
    m["run"] = run
    return blake3_b32(dumps(m))


# ── the cross-language contract ───────────────────────────────────────────

#: A tiny manifest whose run_cid any conforming implementation must reproduce.
#: It deliberately contains the three things that break naive encoders: sibling
#: keys of differing length (`z` vs `aa` ordering), a float that is exact in f16
#: (0.0) next to one that is not (0.1), and a negative zero.
TEST_VECTOR = {
    "manifest_version": 1,
    "run": {
        "run_cid": "<removed before hashing>",
        "z": 1,
        "aa": 2,
        "decoding": {"temperature": 0.0, "top_p": 1.0, "nudge": 0.1, "neg": -0.0},
    },
    "steps": [],
}
