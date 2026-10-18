#!/usr/bin/env python3
"""BLAKE3-256 and Ed25519 signature verification in plain Python.

The emem plugin's verifiers import this module instead of third-party
packages, so checking a receipt needs only a Python 3.8+ interpreter:
no install step, no network, no compiled extension.

    blake3(data=b"")        hasher with update(), digest(), hexdigest()
    ed25519_verify(pk, sig, msg) -> bool   RFC 8032 section 5.1.7, pure
    self_test() -> bool     official BLAKE3 and RFC 8032 vectors

Run `python3 emem_crypto.py` (or any plugin script with `--self-test`) to
check this file against the published test vectors before trusting it.

BLAKE3 follows the reference implementation (BLAKE3-team/BLAKE3,
reference_impl). Ed25519 follows the RFC 8032 section 6 reference code:
cofactorless check [S]B == R + [k]A, S must be below the group order, and
non-canonical point encodings are rejected. This is verification only;
nothing here signs or handles a secret key. It is not constant-time, which
does not matter for checking public signatures over public bytes.
"""
from __future__ import annotations

import hashlib
import struct
import sys

__all__ = ["blake3", "ed25519_verify", "self_test"]

# ---------------------------------------------------------------- BLAKE3

_IV = (
    0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A,
    0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19,
)
_PERM = (2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8)
_CHUNK_START = 1
_CHUNK_END = 2
_PARENT = 4
_ROOT = 8
_BLOCK_LEN = 64
_CHUNK_LEN = 1024
_M32 = 0xFFFFFFFF

# Message schedule for each of the 7 rounds, precomputed from _PERM.
_SCHEDULE = []
_s = list(range(16))
for _ in range(7):
    _SCHEDULE.append(tuple(_s))
    _s = [_s[i] for i in _PERM]
del _s


def _compress(cv, m, counter, block_len, flags):
    """Return the 16-word state of the BLAKE3 compression function."""
    v = [
        cv[0], cv[1], cv[2], cv[3], cv[4], cv[5], cv[6], cv[7],
        _IV[0], _IV[1], _IV[2], _IV[3],
        counter & _M32, (counter >> 32) & _M32, block_len, flags,
    ]
    for s in _SCHEDULE:
        for a, b, c, d, x, y in (
            (0, 4, 8, 12, m[s[0]], m[s[1]]),
            (1, 5, 9, 13, m[s[2]], m[s[3]]),
            (2, 6, 10, 14, m[s[4]], m[s[5]]),
            (3, 7, 11, 15, m[s[6]], m[s[7]]),
            (0, 5, 10, 15, m[s[8]], m[s[9]]),
            (1, 6, 11, 12, m[s[10]], m[s[11]]),
            (2, 7, 8, 13, m[s[12]], m[s[13]]),
            (3, 4, 9, 14, m[s[14]], m[s[15]]),
        ):
            va = (v[a] + v[b] + x) & _M32
            vd = v[d] ^ va
            vd = ((vd >> 16) | (vd << 16)) & _M32
            vc = (v[c] + vd) & _M32
            vb = v[b] ^ vc
            vb = ((vb >> 12) | (vb << 20)) & _M32
            va = (va + vb + y) & _M32
            vd = vd ^ va
            vd = ((vd >> 8) | (vd << 24)) & _M32
            vc = (vc + vd) & _M32
            vb = vb ^ vc
            v[a], v[b], v[c], v[d] = va, ((vb >> 7) | (vb << 25)) & _M32, vc, vd
    for i in range(8):
        v[i] ^= v[i + 8]
        v[i + 8] ^= cv[i]
    return v


def _words(block):
    if len(block) < _BLOCK_LEN:
        block = block + b"\x00" * (_BLOCK_LEN - len(block))
    return struct.unpack("<16I", block)


class _Output:
    __slots__ = ("cv", "m", "counter", "block_len", "flags")

    def __init__(self, cv, m, counter, block_len, flags):
        self.cv, self.m, self.counter, self.block_len, self.flags = cv, m, counter, block_len, flags

    def chaining_value(self):
        return _compress(self.cv, self.m, self.counter, self.block_len, self.flags)[:8]

    def root_bytes(self, length):
        out = b""
        block_counter = 0
        while len(out) < length:
            w = _compress(self.cv, self.m, block_counter, self.block_len, self.flags | _ROOT)
            out += struct.pack("<16I", *w)
            block_counter += 1
        return out[:length]


class _ChunkState:
    __slots__ = ("cv", "counter", "buf", "blocks", "flags")

    def __init__(self, key, counter, flags):
        self.cv, self.counter, self.buf, self.blocks, self.flags = key, counter, b"", 0, flags

    def __len__(self):
        return _BLOCK_LEN * self.blocks + len(self.buf)

    def _start(self):
        return _CHUNK_START if self.blocks == 0 else 0

    def update(self, data):
        while data:
            if len(self.buf) == _BLOCK_LEN:
                self.cv = _compress(self.cv, _words(self.buf), self.counter, _BLOCK_LEN,
                                    self.flags | self._start())[:8]
                self.blocks += 1
                self.buf = b""
            take = min(_BLOCK_LEN - len(self.buf), len(data))
            self.buf += data[:take]
            data = data[take:]

    def output(self):
        return _Output(self.cv, _words(self.buf), self.counter, len(self.buf),
                       self.flags | self._start() | _CHUNK_END)


def _parent_output(left, right, key, flags):
    return _Output(key, tuple(left) + tuple(right), 0, _BLOCK_LEN, _PARENT | flags)


class blake3:  # noqa: N801  (named like the hashlib-style constructor it replaces)
    """Incremental BLAKE3 in the default hash mode, 32-byte output."""

    def __init__(self, data: bytes = b""):
        self._key = _IV
        self._flags = 0
        self._chunk = _ChunkState(self._key, 0, self._flags)
        self._stack = []
        if data:
            self.update(data)

    def update(self, data) -> "blake3":
        data = memoryview(bytes(data)).tobytes()
        while data:
            if len(self._chunk) == _CHUNK_LEN:
                cv = self._chunk.output().chaining_value()
                total = self._chunk.counter + 1
                while total & 1 == 0:
                    cv = _parent_output(self._stack.pop(), cv, self._key, self._flags).chaining_value()
                    total >>= 1
                self._stack.append(cv)
                self._chunk = _ChunkState(self._key, self._chunk.counter + 1, self._flags)
            take = min(_CHUNK_LEN - len(self._chunk), len(data))
            self._chunk.update(data[:take])
            data = data[take:]
        return self

    def digest(self, length: int = 32) -> bytes:
        out = self._chunk.output()
        for left in reversed(self._stack):
            out = _parent_output(left, out.chaining_value(), self._key, self._flags)
        return out.root_bytes(length)

    def hexdigest(self, length: int = 32) -> str:
        return self.digest(length).hex()


# --------------------------------------------------------------- Ed25519

_P = 2**255 - 19
_L = 2**252 + 27742317777372353535851937790883648493
_D = -121665 * pow(121666, _P - 2, _P) % _P
_SQRT_M1 = pow(2, (_P - 1) // 4, _P)


def _recover_x(y, sign):
    if y >= _P:
        return None
    x2 = (y * y - 1) * pow(_D * y * y + 1, _P - 2, _P) % _P
    if x2 == 0:
        return None if sign else 0
    x = pow(x2, (_P + 3) // 8, _P)
    if (x * x - x2) % _P != 0:
        x = x * _SQRT_M1 % _P
    if (x * x - x2) % _P != 0:
        return None
    if (x & 1) != sign:
        x = _P - x
    return x


def _decompress(s):
    if len(s) != 32:
        return None
    y = int.from_bytes(s, "little")
    sign = y >> 255
    y &= (1 << 255) - 1
    x = _recover_x(y, sign)
    if x is None:
        return None
    return (x, y, 1, x * y % _P)


def _add(p, q):
    a = (p[1] - p[0]) * (q[1] - q[0]) % _P
    b = (p[1] + p[0]) * (q[1] + q[0]) % _P
    c = 2 * p[3] * q[3] * _D % _P
    d = 2 * p[2] * q[2] % _P
    e, f, g, h = b - a, d - c, d + c, b + a
    return (e * f % _P, g * h % _P, f * g % _P, e * h % _P)


def _mul(s, p):
    q = (0, 1, 1, 0)
    while s > 0:
        if s & 1:
            q = _add(q, p)
        p = _add(p, p)
        s >>= 1
    return q


def _equal(p, q):
    return (p[0] * q[2] - q[0] * p[2]) % _P == 0 and (p[1] * q[2] - q[1] * p[2]) % _P == 0


_GY = 4 * pow(5, _P - 2, _P) % _P
_GX = _recover_x(_GY, 0)
_G = (_GX, _GY, 1, _GX * _GY % _P)


def ed25519_verify(public_key: bytes, signature: bytes, message: bytes) -> bool:
    """True when `signature` is a valid Ed25519 signature of `message`."""
    public_key, signature, message = bytes(public_key), bytes(signature), bytes(message)
    if len(public_key) != 32 or len(signature) != 64:
        return False
    a = _decompress(public_key)
    if a is None:
        return False
    r = _decompress(signature[:32])
    if r is None:
        return False
    s = int.from_bytes(signature[32:], "little")
    if s >= _L:
        return False
    k = int.from_bytes(hashlib.sha512(signature[:32] + public_key + message).digest(), "little") % _L
    return _equal(_mul(s, _G), _add(r, _mul(k, a)))


# ------------------------------------------------------------ self-test

# First 32 bytes of the "hash" field of BLAKE3-team/BLAKE3
# test_vectors/test_vectors.json. Input is bytes(i % 251 for i in range(n)).
_BLAKE3_VECTORS = (
    (0, "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"),
    (1, "2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213"),
    (63, "e9bc37a594daad83be9470df7f7b3798297c3d834ce80ba85d6e207627b7db7b"),
    (64, "4eed7141ea4a5cd4b788606bd23f46e212af9cacebacdc7d1f4c6dc7f2511b98"),
    (65, "de1e5fa0be70df6d2be8fffd0e99ceaa8eb6e8c93a63f2d8d1c30ecb6b263dee"),
    (1023, "10108970eeda3eb932baac1428c7a2163b0e924c9a9e25b35bba72b28f70bd11"),
    (1024, "42214739f095a406f3fc83deb889744ac00df831c10daa55189b5d121c855af7"),
    (1025, "d00278ae47eb27b34faecf67b4fe263f82d5412916c1ffd97c8cb7fb814b8444"),
    (2049, "5f4d72f40d7a5f82b15ca2b2e44b1de3c2ef86c426c95c1af0b6879522563030"),
    (8193, "bab6c09cb8ce8cf459261398d2e7aef35700bf488116ceb94a36d0f5f1b7bc3b"),
    (102400, "bc3e3d41a1146b069abffad3c0d44860cf664390afce4d9661f7902e7943e085"),
)

# RFC 8032 section 7.1, TEST 1, 2 and 3: (public key, message, signature).
_ED25519_VECTORS = (
    ("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a", "",
     "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155"
     "5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"),
    ("3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c", "72",
     "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da"
     "085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00"),
    ("fc51cd8e6218a1a38da47ed00230f0580816ed13ba3303ac5deb911548908025", "af82",
     "6291d657deec24024827e69c3abe01a30ce548a284743a445e3680d7db5ac3ac"
     "18ff9b538d16f290ae67f760984dc6594a7c15e9716ed28dc027beceea1ec40a"),
)


def self_test(out=sys.stdout) -> bool:
    ok = True
    for n, want in _BLAKE3_VECTORS:
        data = bytes(i % 251 for i in range(n))
        one = blake3(data).hexdigest()
        split = blake3(data[: n // 3]).update(data[n // 3:]).hexdigest()
        good = one == want and split == want
        ok &= good
        out.write(f"blake3   len={n:<6} {'ok' if good else 'FAIL'}\n")
    for i, (pk, msg, sig) in enumerate(_ED25519_VECTORS, 1):
        good = ed25519_verify(bytes.fromhex(pk), bytes.fromhex(sig), bytes.fromhex(msg))
        ok &= good
        out.write(f"ed25519  rfc8032 test {i}  {'ok' if good else 'FAIL'}\n")
    pk, msg, sig = _ED25519_VECTORS[1]
    negatives = (
        ("flipped message", pk, "73", sig),
        ("flipped signature bit", pk, msg, sig[:-2] + "01"),
        ("wrong public key", _ED25519_VECTORS[0][0], msg, sig),
        ("S not below group order", pk, msg,
         sig[:64] + (int.from_bytes(bytes.fromhex(sig[64:]), "little") + _L).to_bytes(32, "little").hex()),
    )
    for label, npk, nmsg, nsig in negatives:
        rejected = not ed25519_verify(bytes.fromhex(npk), bytes.fromhex(nsig), bytes.fromhex(nmsg))
        ok &= rejected
        out.write(f"ed25519  rejects {label}  {'ok' if rejected else 'FAIL'}\n")
    out.write("SELF-TEST PASS\n" if ok else "SELF-TEST FAIL\n")
    return ok


if __name__ == "__main__":
    sys.exit(0 if self_test() else 1)
