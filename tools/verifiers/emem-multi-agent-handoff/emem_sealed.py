#!/usr/bin/env python3
"""emem.sealed.v1: a note that only one emem key can open.

Every emem note is public. A sealed note is still public, but its words
are ciphertext that only the recipient key opens. The emem signature on
the note still says who wrote it, and the log still orders and times it.
The author, the recipient key, the time and the size stay public.

    recipient X25519 public  u = (1 + y) / (1 - y) mod p, from the Edwards y
                             of the recipient's Ed25519 public key; a key
                             that is not a valid point of prime order is
                             refused
    recipient X25519 secret  sha512(seed)[:32], clamped, as libsodium's
                             crypto_sign_ed25519_sk_to_curve25519 does
    ephemeral                a fresh X25519 key for each note
    key                      HKDF-SHA256(ikm = X25519(ephemeral, recipient),
                             salt = none, info = "emem.sealed.v1"
                             || ephemeral public || recipient X25519 public,
                             32 bytes)
    ciphertext               ChaCha20-Poly1305(key, 12 random nonce bytes,
                             associated data = the header)

The header is the front matter between the two `---` lines. The
associated data is its lines in order, each with trailing white space
removed, joined by a line feed. A change to any header line, or to one
byte of the ciphertext, makes the note fail to open. Keys are lowercase
base32 without padding, as every emem key is; the ciphertext is standard
base64 in the first fenced block after the header.

The header names the author's public key and binds it as associated
data. A copy that another key re-signs still names the first author, so
the recipient can see that the signer and the sealer differ.

This module needs the `cryptography` package to seal or open. The key
conversions use only emem_crypto.py, beside this file. There is no
forward secrecy: whoever gets the recipient's seed later opens every
note sealed to it, and the ciphertext stays in the public log.
"""
from __future__ import annotations

import base64
import hashlib
import os
import re
import sys
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))
from emem_crypto import _L, _P, _decompress, _equal, _mul  # noqa: E402

SCHEMA = "emem.sealed.v1"
NONCE_LEN = 12
_IDENTITY = (0, 1, 1, 0)
_B32_KEY = re.compile(r"[a-z2-7]{52}")


class SealedError(ValueError):
    """A sealed note that is malformed, not for this key, or changed."""


def b32e(b: bytes) -> str:
    return base64.b32encode(b).decode().rstrip("=").lower()


def b32d(s: str, n: int, what: str) -> bytes:
    s = s.strip()
    try:
        out = base64.b32decode(s.upper() + "=" * ((8 - len(s) % 8) % 8))
    except (ValueError, TypeError):
        raise SealedError(f"{what} is not base32") from None
    if len(out) != n:
        raise SealedError(f"{what} is {len(out)} bytes, not {n}")
    return out


def ed25519_pk_to_x25519(pk: bytes) -> bytes:
    """The X25519 public key for an Ed25519 public key, or SealedError."""
    point = _decompress(bytes(pk))
    if point is None:
        raise SealedError("the recipient key is not a valid Ed25519 point")
    if _equal(_mul(8, point), _IDENTITY):
        raise SealedError("the recipient key is a small-order point")
    if not _equal(_mul(_L, point), _IDENTITY):
        raise SealedError("the recipient key is not in the prime-order group")
    y = int.from_bytes(pk, "little") & ((1 << 255) - 1)
    u = (1 + y) * pow(1 - y, _P - 2, _P) % _P
    return u.to_bytes(32, "little")


def ed25519_seed_to_x25519(seed: bytes) -> bytes:
    """The clamped X25519 secret scalar for a 32-byte Ed25519 seed."""
    if len(seed) != 32:
        raise SealedError("the seed is not 32 bytes")
    s = bytearray(hashlib.sha512(seed).digest()[:32])
    s[0] &= 248
    s[31] &= 127
    s[31] |= 64
    return bytes(s)


def _crypto():
    try:
        from cryptography.exceptions import InvalidTag
        from cryptography.hazmat.primitives import hashes, serialization
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
        from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey, X25519PublicKey
        from cryptography.hazmat.primitives.ciphers.aead import ChaCha20Poly1305
        from cryptography.hazmat.primitives.kdf.hkdf import HKDF
    except ImportError:
        raise SealedError("sealing and opening need the Python `cryptography` package; "
                          "this interpreter does not have it") from None
    return {
        "InvalidTag": InvalidTag, "hashes": hashes, "serialization": serialization,
        "Ed25519PrivateKey": Ed25519PrivateKey, "X25519PrivateKey": X25519PrivateKey,
        "X25519PublicKey": X25519PublicKey, "ChaCha20Poly1305": ChaCha20Poly1305, "HKDF": HKDF,
    }


def _raw(c, public_key) -> bytes:
    return public_key.public_bytes(c["serialization"].Encoding.Raw, c["serialization"].PublicFormat.Raw)


def _key(c, shared: bytes, eph_pub: bytes, rcpt_x: bytes) -> bytes:
    info = SCHEMA.encode() + eph_pub + rcpt_x
    return c["HKDF"](algorithm=c["hashes"].SHA256(), length=32, salt=None, info=info).derive(shared)


def _associated_data(header_lines: list[str]) -> bytes:
    return "\n".join(line.rstrip() for line in header_lines).encode("utf-8")


def seal(plaintext: bytes, recipient_b32: str, author_b32: str) -> str:
    """The text of a sealed note for `recipient_b32`, written by `author_b32`."""
    recipient_b32, author_b32 = recipient_b32.strip().lower(), author_b32.strip().lower()
    for what, k in (("the recipient key", recipient_b32), ("the author key", author_b32)):
        if not _B32_KEY.fullmatch(k):
            raise SealedError(f"{what} must be a full 52-character pubkey_b32, not a shortcode")
    rcpt_x = ed25519_pk_to_x25519(b32d(recipient_b32, 32, "the recipient key"))
    ed25519_pk_to_x25519(b32d(author_b32, 32, "the author key"))
    c = _crypto()
    eph = c["X25519PrivateKey"].generate()
    eph_pub = _raw(c, eph.public_key())
    shared = eph.exchange(c["X25519PublicKey"].from_public_bytes(rcpt_x))
    nonce = os.urandom(NONCE_LEN)
    header = [
        f"to: {recipient_b32[:8]}",
        f"schema: {SCHEMA}",
        f"recipient: {recipient_b32}",
        f"author: {author_b32}",
        f"ephemeral: {b32e(eph_pub)}",
        f"nonce: {b32e(nonce)}",
    ]
    box = c["ChaCha20Poly1305"](_key(c, shared, eph_pub, rcpt_x))
    ct = base64.b64encode(box.encrypt(nonce, bytes(plaintext), _associated_data(header))).decode()
    wrapped = "\n".join(ct[i:i + 76] for i in range(0, len(ct), 76))
    return (
        "---\n" + "\n".join(header) + "\n---\n"
        f"# Sealed note -> {recipient_b32[:8]}\n\n"
        f"This note is encrypted with {SCHEMA}, and only the recipient key can open it.\n"
        "The emem signature on this note still says who wrote it.\n"
        "The author, the recipient key, the time and the size stay public.\n\n"
        "```text\n" + wrapped + "\n```\n"
    )


def parse(note: str) -> dict:
    """The header fields, the header lines and the ciphertext of a sealed note."""
    lines = note.splitlines()
    if not lines or lines[0].strip() != "---":
        raise SealedError("not a sealed note: it does not start with a `---` header")
    try:
        end = next(i for i in range(1, len(lines)) if lines[i].strip() == "---")
    except StopIteration:
        raise SealedError("not a sealed note: the header has no closing `---`") from None
    header = lines[1:end]
    fields: dict[str, str] = {}
    for line in header:
        k, sep, v = line.partition(":")
        if not sep:
            raise SealedError(f"header line without a colon: {line[:60]!r}")
        k = k.strip()
        if k in fields:
            raise SealedError(f"the header names {k!r} twice")
        fields[k] = v.strip()
    if fields.get("schema") != SCHEMA:
        raise SealedError(f"schema is {fields.get('schema')!r}, not {SCHEMA}")
    for k in ("recipient", "author", "ephemeral", "nonce"):
        if not fields.get(k):
            raise SealedError(f"the header has no {k!r}")
    body = lines[end + 1:]
    try:
        start = next(i for i, line in enumerate(body) if line.strip().startswith("```"))
        stop = next(i for i in range(start + 1, len(body)) if body[i].strip().startswith("```"))
    except StopIteration:
        raise SealedError("no fenced ciphertext block after the header") from None
    try:
        ct = base64.b64decode("".join("".join(body[start + 1:stop]).split()), validate=True)
    except ValueError:
        raise SealedError("the ciphertext is not base64") from None
    return {"fields": fields, "header": header, "ciphertext": ct}


def open_sealed(note: str, seed: bytes) -> tuple[bytes, dict]:
    """(plaintext, header fields), or SealedError."""
    p = parse(note)
    f = p["fields"]
    c = _crypto()
    mine = b32e(_raw(c, c["Ed25519PrivateKey"].from_private_bytes(bytes(seed)).public_key()))
    if f["recipient"].lower() != mine:
        raise SealedError(f"this note is sealed to {f['recipient']}, not to your key {mine}")
    priv = c["X25519PrivateKey"].from_private_bytes(ed25519_seed_to_x25519(bytes(seed)))
    rcpt_x = _raw(c, priv.public_key())
    if rcpt_x != ed25519_pk_to_x25519(b32d(mine, 32, "your key")):
        raise SealedError("internal check failed: the two X25519 conversions of your key differ")
    eph_pub = b32d(f["ephemeral"], 32, "ephemeral")
    nonce = b32d(f["nonce"], NONCE_LEN, "nonce")
    try:
        shared = priv.exchange(c["X25519PublicKey"].from_public_bytes(eph_pub))
    except ValueError:
        raise SealedError("the ephemeral key is not usable (a small-order point)") from None
    box = c["ChaCha20Poly1305"](_key(c, shared, eph_pub, rcpt_x))
    try:
        plain = box.decrypt(nonce, p["ciphertext"], _associated_data(p["header"]))
    except c["InvalidTag"]:
        raise SealedError("the note does not open: its header or ciphertext changed, "
                          "or it was not sealed to this key") from None
    return plain, f
