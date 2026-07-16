"""ed25519 attester signing for emem memory writes.

The emem responder gates every memory write verb (`create`,
`str_replace`, `insert`, `delete`, `rename`) behind an `attester`
block. The block binds the write to an ed25519 key:

    sig = ed25519(blake3(b"emem.memory_write|" + verb + b"|"
                         + path + b"|" + body_hash))

and travels on the wire as::

    "attester": {"pubkey_b32": "<52 chars>", "sig_b32": "<103 chars>"}

Both fields are base32-nopad, lowercase. The preimage shape is fixed so
a client signs offline with no server round-trip.

This module is a byte-for-byte mirror of
`crates/emem-primitives/src/memory_acl.rs` and of
`sdks/emem-langmem/src/emem_langmem/signing.py`; the function names match
so the three sides can be read next to each other, and
`tests/test_signing.py` pins all of them to one fixed vector so the
mirrors cannot drift apart. It lives here, in the base client, because
the signer is pure blake3 + ed25519 with no LangChain in it, and an agent
that only wants to leave a signed note should not have to install a
framework to do it. The crypto dependencies are the `signing` extra:
`pip install ememdev[signing]`.

`body_hash` is blake3 over the bytes the responder itself hashes, which
is not always the bytes a caller thinks it is sending:

| verb          | body the responder hashes                        |
|---------------|--------------------------------------------------|
| `create`      | UTF-8 bytes of the `file_text` string sent        |
| `str_replace` | UTF-8 bytes of the whole file *after* the edit    |
| `insert`      | UTF-8 bytes of the whole file *after* the insert  |
| `delete`      | empty (blake3 of b"")                             |
| `rename`      | UTF-8 bytes of the `old_path` string              |

So `str_replace` and `insert` cannot be signed without knowing the
file's current content: read it, apply the edit locally, and sign the
result.

`rename` is the one verb whose preimage `path` is not the path the
caller names first. The destination rides `path` and the source rides
the body, so one signature binds both ends of the move. Use
`EmemSigner.rename_attester_block`, which puts them in the right slots.

Writes under `/memories/by_attester/<pubkey8>/...` are restricted to the
holder of the matching private key, where `<pubkey8>` is the first 8
characters of the lowercased base32 pubkey. A signed write aimed at
another key's namespace is refused with a 403.
"""

from __future__ import annotations

import base64
from typing import Union

try:
    from blake3 import blake3
    from cryptography.hazmat.primitives import serialization
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
except ModuleNotFoundError as e:  # pragma: no cover - import-time guard
    raise ModuleNotFoundError(
        "ememdev signing needs the 'signing' extra: pip install ememdev[signing] "
        "(pulls blake3 and cryptography)"
    ) from e

# Mirrors emem_primitives::BY_ATTESTER_PREFIX / PUBKEY_SHORT_LEN.
BY_ATTESTER_PREFIX = "/memories/by_attester/"
PUBKEY_SHORT_LEN = 8

#: The five memory-tool write verbs the responder will verify a
#: signature for. `memory_view` is a read and carries no attester.
WRITE_VERBS = ("create", "str_replace", "insert", "delete", "rename")

_PREIMAGE_TAG = b"emem.memory_write|"
_SEP = b"|"


def b32_nopad_lc(raw: bytes) -> str:
    """Encode bytes the way the responder expects: base32, no padding,
    lowercase."""
    return base64.b32encode(raw).decode("ascii").rstrip("=").lower()


def body_hash(body: bytes = b"") -> bytes:
    """blake3 of the write body, as 32 raw bytes. `delete` carries no
    body and hashes the empty string, which is the default. `rename`
    carries its source path: see `rename_body_hash`."""
    return blake3(body).digest()


def rename_body_hash(old_path: str) -> bytes:
    """The body-hash for a `rename`: blake3 over the *source* path's
    UTF-8 bytes.

    The destination already rides the preimage's `path` component, so
    hashing the source here makes one signature bind both ends of the
    move.
    """
    return body_hash(old_path.encode("utf-8"))


def attester_preimage(verb: str, path: str, body_digest: bytes) -> bytes:
    """Build the 32-byte digest that gets signed.

    `body_digest` is the 32 raw bytes from `body_hash`, not the body.
    """
    if verb not in WRITE_VERBS:
        raise ValueError(
            f"unknown write verb {verb!r}; expected one of {', '.join(WRITE_VERBS)}"
        )
    if len(body_digest) != 32:
        raise ValueError(f"body_digest must be 32 bytes, got {len(body_digest)}")
    h = blake3()
    h.update(_PREIMAGE_TAG)
    h.update(verb.encode("utf-8"))
    h.update(_SEP)
    h.update(path.encode("utf-8"))
    h.update(_SEP)
    h.update(body_digest)
    return h.digest()


def pubkey_short_from_b32(pubkey_b32: str) -> str:
    """First 8 chars of the lowercased base32 pubkey: the segment the
    responder matches against `/memories/by_attester/<pubkey8>/...`."""
    return pubkey_b32.lower()[:PUBKEY_SHORT_LEN]


class EmemSigner:
    """Signs emem memory writes with an ed25519 key.

    The key comes from the caller. Nothing here generates one, and there
    is no fallback key: an `EmemSigner` without a real private key would
    write into a namespace the caller cannot reclaim. To load or mint a
    persistent identity from the standard location, use
    `ememdev.identity.load_or_create_signer`.

    Parameters
    ----------
    private_key:
        The raw 32-byte ed25519 seed (what `cryptography` calls the
        private bytes, and what the Rust side calls
        `SigningKey::from_bytes`). Use `from_hex` for a 64-char hex seed.

    Example
    -------
    >>> import os
    >>> signer = EmemSigner(os.urandom(32))          # persist this seed
    >>> block = signer.attester_block("create", f"{signer.namespace_root}/n.md", b"hi")
    >>> sorted(block)
    ['pubkey_b32', 'sig_b32']
    """

    def __init__(self, private_key: bytes) -> None:
        if isinstance(private_key, str):
            raise TypeError(
                "private_key must be the raw 32-byte seed, not a string; "
                "use EmemSigner.from_hex() for a hex seed"
            )
        if not isinstance(private_key, (bytes, bytearray)):
            raise TypeError(
                f"private_key must be bytes, got {type(private_key).__name__}"
            )
        if len(private_key) != 32:
            raise ValueError(
                f"ed25519 private key must be 32 bytes, got {len(private_key)}"
            )
        self._key = Ed25519PrivateKey.from_private_bytes(bytes(private_key))
        self.pubkey: bytes = self._key.public_key().public_bytes(
            encoding=serialization.Encoding.Raw,
            format=serialization.PublicFormat.Raw,
        )
        self.pubkey_b32: str = b32_nopad_lc(self.pubkey)
        self.pubkey_short: str = pubkey_short_from_b32(self.pubkey_b32)

    @classmethod
    def from_hex(cls, seed_hex: str) -> "EmemSigner":
        """Build a signer from a 64-character hex ed25519 seed."""
        try:
            raw = bytes.fromhex(seed_hex.strip())
        except ValueError as e:
            raise ValueError(f"seed_hex is not valid hex: {e}") from e
        return cls(raw)

    @property
    def seed_bytes(self) -> bytes:
        """The raw 32-byte ed25519 seed. Handle like any other secret."""
        return self._key.private_bytes(
            encoding=serialization.Encoding.Raw,
            format=serialization.PrivateFormat.Raw,
            encryption_algorithm=serialization.NoEncryption(),
        )

    @property
    def namespace_root(self) -> str:
        """This key's own memory namespace: `/memories/by_attester/<pubkey8>`."""
        return f"{BY_ATTESTER_PREFIX}{self.pubkey_short}"

    def sign_preimage(self, preimage: bytes) -> bytes:
        """Raw ed25519 signature (64 bytes) over an already-built preimage."""
        return self._key.sign(preimage)

    def attester_block(self, verb: str, path: str, body: bytes = b"") -> dict:
        """Build the wire `attester` block for one write.

        `body` is the byte string the responder will hash for this verb
        (see the table in this module's docstring). It defaults to empty,
        which is right for `delete`. For `rename`, prefer
        `rename_attester_block`.
        """
        digest = attester_preimage(verb, path, body_hash(body))
        return {
            "pubkey_b32": self.pubkey_b32,
            "sig_b32": b32_nopad_lc(self.sign_preimage(digest)),
        }

    def rename_attester_block(self, old_path: str, new_path: str) -> dict:
        """Build the wire `attester` block for a `rename`.

        The preimage's `path` is the destination and its body is the
        source, which is easy to invert by hand. Pass the paths in the
        same order `memory_rename` takes them and this puts them in the
        right slots.

        The responder additionally requires this key to own both ends of
        the move; it refuses a source in another key's namespace with a
        403. Signing here does not check that.
        """
        return {
            "pubkey_b32": self.pubkey_b32,
            "sig_b32": b32_nopad_lc(
                self.sign_preimage(
                    attester_preimage("rename", new_path, rename_body_hash(old_path))
                )
            ),
        }

    def owns(self, path: str) -> bool:
        """Whether `path` sits in this key's attester namespace. Paths
        outside `/memories/by_attester/` are not owned by anyone, so they
        return False."""
        rest = (
            path[len(BY_ATTESTER_PREFIX) :]
            if path.startswith(BY_ATTESTER_PREFIX)
            else None
        )
        if rest is None:
            return False
        return rest.split("/", 1)[0] == self.pubkey_short

    def __repr__(self) -> str:
        # Never render private bytes.
        return f"EmemSigner(pubkey_short={self.pubkey_short!r})"


def coerce_signer(signing_key: Union["EmemSigner", bytes, bytearray, None]) -> "EmemSigner":
    """Accept either a ready `EmemSigner` or a raw 32-byte seed."""
    if isinstance(signing_key, EmemSigner):
        return signing_key
    return EmemSigner(signing_key)  # type: ignore[arg-type]
