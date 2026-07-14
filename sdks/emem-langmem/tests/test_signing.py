"""Attester signing tests.

The preimage and the wire encoding have to agree with the responder
byte for byte or every write 401s, so these tests pin both against
independent references rather than against the implementation:

* the ed25519 layer is checked against RFC 8032 test vector 1, so a
  wrong seed-to-pubkey convention (seed vs expanded key) fails here;
* the preimage is rebuilt from the literal spec in
  `crates/emem-primitives/src/memory_acl.rs::attester_preimage`;
* `test_signature_verifies_like_the_rust_verifier` walks the same steps
  `verify_attester` takes, including the base32 round-trip.
"""

import base64

import pytest
from blake3 import blake3
from cryptography.hazmat.primitives.asymmetric.ed25519 import (
    Ed25519PrivateKey,
    Ed25519PublicKey,
)
from cryptography.exceptions import InvalidSignature

from emem_langmem.signing import (
    BY_ATTESTER_PREFIX,
    EmemSigner,
    WRITE_VERBS,
    attester_preimage,
    b32_nopad_lc,
    body_hash,
    pubkey_short_from_b32,
    rename_body_hash,
)

# RFC 8032 section 7.1, TEST 1. A published vector, used only to prove
# the seed convention matches ed25519-dalek's `SigningKey::from_bytes`.
_RFC8032_SEED = bytes.fromhex(
    "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60"
)
_RFC8032_PUBKEY = bytes.fromhex(
    "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
)
_RFC8032_SIG_EMPTY_MSG = bytes.fromhex(
    "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155"
    "5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"
)

# A fixed, throwaway seed so path assertions are stable. Test-only.
_SEED = bytes(range(32))


def _signer() -> EmemSigner:
    return EmemSigner(_SEED)


# ---------- the ed25519 layer ----------


def test_seed_convention_matches_rfc8032():
    """EmemSigner takes the 32-byte seed, which is what the Rust
    verifier's counterpart signs with."""
    signer = EmemSigner(_RFC8032_SEED)
    assert signer.pubkey == _RFC8032_PUBKEY
    assert signer.sign_preimage(b"") == _RFC8032_SIG_EMPTY_MSG


def test_rejects_wrong_key_shapes():
    with pytest.raises(ValueError):
        EmemSigner(b"too short")
    with pytest.raises(TypeError):
        EmemSigner("a-hex-string-is-not-a-seed")
    assert EmemSigner.from_hex(_RFC8032_SEED.hex()).pubkey == _RFC8032_PUBKEY


def test_repr_hides_private_bytes():
    assert "EmemSigner(pubkey_short=" in repr(_signer())
    assert _SEED.hex()[:16] not in repr(_signer())


# ---------- the preimage ----------


def test_body_hash_is_blake3_of_the_body():
    assert body_hash(b"hello") == blake3(b"hello").digest()
    # `delete` carries no body and hashes the empty string, matching the
    # Rust `body_hash(b"")`. `rename` hashes its source path instead.
    assert body_hash() == blake3(b"").digest()
    assert len(body_hash(b"x")) == 32


def test_preimage_matches_the_rust_spec():
    """Rebuild `attester_preimage` from the literal Rust source: tag,
    verb, sep, path, sep, body_hash, all fed to one blake3 hasher."""
    verb, path, body = "create", "/memories/notes.md", b"hello"
    bh = blake3(body).digest()

    expected = blake3(
        b"emem.memory_write|" + verb.encode() + b"|" + path.encode() + b"|" + bh
    ).digest()

    assert attester_preimage(verb, path, bh) == expected
    assert len(expected) == 32


def test_preimage_is_domain_separated():
    bh = body_hash(b"x")
    base = attester_preimage("create", "/memories/a.md", bh)
    # Every field is covered: change any one and the digest moves.
    assert attester_preimage("delete", "/memories/a.md", bh) != base
    assert attester_preimage("create", "/memories/b.md", bh) != base
    assert attester_preimage("create", "/memories/a.md", body_hash(b"y")) != base


def test_preimage_rejects_bad_inputs():
    with pytest.raises(ValueError):
        attester_preimage("frobnicate", "/memories/a.md", body_hash(b""))
    with pytest.raises(ValueError):
        attester_preimage("create", "/memories/a.md", b"not-32-bytes")


# ---------- the wire block ----------


def test_attester_block_encoding():
    block = _signer().attester_block("create", "/memories/x.md", b"hi")
    assert sorted(block) == ["pubkey_b32", "sig_b32"]
    # base32-nopad of 32 and 64 bytes, lowercase, as docs/memory.md states.
    assert len(block["pubkey_b32"]) == 52
    assert len(block["sig_b32"]) == 103
    for field in block.values():
        assert field == field.lower()
        assert "=" not in field


def test_b32_round_trips_through_the_rust_decoder():
    """The Rust uppercases before BASE32_NOPAD.decode, so our lowercase
    must survive that trip."""
    raw = bytes(range(32))
    encoded = b32_nopad_lc(raw)
    padded = encoded.upper() + "=" * (-len(encoded) % 8)
    assert base64.b32decode(padded) == raw


@pytest.mark.parametrize("verb", WRITE_VERBS)
def test_signature_verifies_like_the_rust_verifier(verb):
    """Replay `verify_attester`: decode the base32 fields, rebuild the
    preimage, check the signature. Runs for all five write verbs."""
    signer = _signer()
    path = f"{signer.namespace_root}/notes.md"
    if verb == "delete":
        body = b""
    elif verb == "rename":
        # `path` is the destination for a rename; the body is the source.
        body = f"{signer.namespace_root}/old.md".encode()
    else:
        body = b"some body"
    block = signer.attester_block(verb, path, body)

    def _b32d(s: str) -> bytes:
        return base64.b32decode(s.upper() + "=" * (-len(s) % 8))

    pubkey = _b32d(block["pubkey_b32"])
    sig = _b32d(block["sig_b32"])
    assert len(pubkey) == 32 and len(sig) == 64

    preimage = attester_preimage(verb, path, body_hash(body))
    # No exception means the responder's verify_strict would pass too.
    Ed25519PublicKey.from_public_bytes(pubkey).verify(sig, preimage)


def test_tampered_body_fails_verification():
    """Mirror of the Rust `tampered_body_rejected` test."""
    signer = _signer()
    path = "/memories/notes.md"
    block = signer.attester_block("create", path, b"hello")
    sig = base64.b32decode(block["sig_b32"].upper() + "=")
    pubkey = Ed25519PublicKey.from_public_bytes(
        base64.b32decode(block["pubkey_b32"].upper() + "====")
    )
    with pytest.raises(InvalidSignature):
        pubkey.verify(sig, attester_preimage("create", path, body_hash(b"different")))


# ---------- the rename contract ----------


def test_rename_body_hash_is_blake3_of_the_source_path():
    """Rebuilt from the Rust `rename_body_hash`: blake3 over the source
    path's UTF-8 bytes."""
    old_path = "/memories/by_attester/aaaabbbb/a.md"
    assert rename_body_hash(old_path) == blake3(old_path.encode("utf-8")).digest()


def test_rename_block_signs_destination_as_path_and_source_as_body():
    """The move's two ends land in different preimage slots: `new_path`
    is the `path` component, `old_path` is the body. Rebuilt from the
    literal Rust spec rather than from `attester_preimage`."""
    signer = _signer()
    old_path = f"{signer.namespace_root}/a.md"
    new_path = f"{signer.namespace_root}/b.md"
    block = signer.rename_attester_block(old_path, new_path)

    expected = blake3(
        b"emem.memory_write|"
        + b"rename"
        + b"|"
        + new_path.encode()
        + b"|"
        + blake3(old_path.encode()).digest()
    ).digest()

    pubkey = Ed25519PublicKey.from_public_bytes(
        base64.b32decode(block["pubkey_b32"].upper() + "====")
    )
    sig = base64.b32decode(block["sig_b32"].upper() + "=")
    # No exception means the responder's verify_strict would pass too.
    pubkey.verify(sig, expected)

    # The paths are not interchangeable: signing the reverse move is a
    # different signature.
    assert signer.rename_attester_block(new_path, old_path) != block


def test_rename_signature_does_not_verify_for_a_different_old_path():
    """Mirror of the Rust `rename_signature_binds_the_source_path`: a
    captured rename signature cannot be replayed to drag a different
    file to the same destination."""
    signer = _signer()
    new_path = f"{signer.namespace_root}/b.md"
    signed_source = f"{signer.namespace_root}/a.md"
    other_source = f"{signer.namespace_root}/secrets.md"

    block = signer.rename_attester_block(signed_source, new_path)
    pubkey = Ed25519PublicKey.from_public_bytes(
        base64.b32decode(block["pubkey_b32"].upper() + "====")
    )
    sig = base64.b32decode(block["sig_b32"].upper() + "=")

    forged = blake3(
        b"emem.memory_write|"
        + b"rename"
        + b"|"
        + new_path.encode()
        + b"|"
        + blake3(other_source.encode()).digest()
    ).digest()
    with pytest.raises(InvalidSignature):
        pubkey.verify(sig, forged)


# ---------- namespace ownership ----------


def test_namespace_root_is_the_key_short_form():
    signer = _signer()
    assert signer.pubkey_short == signer.pubkey_b32[:8]
    assert len(signer.pubkey_short) == 8
    assert signer.namespace_root == f"{BY_ATTESTER_PREFIX}{signer.pubkey_short}"
    assert pubkey_short_from_b32(signer.pubkey_b32.upper()) == signer.pubkey_short


def test_owns_only_its_own_namespace():
    signer = _signer()
    other = EmemSigner(bytes(range(1, 33)))
    assert signer.owns(f"{signer.namespace_root}/a/b.md")
    assert not signer.owns(f"{other.namespace_root}/a.md")
    # Unscoped paths belong to no key.
    assert not signer.owns("/memories/a.md")
    # A prefix that merely starts with the short form is not ownership.
    assert not signer.owns(f"{BY_ATTESTER_PREFIX}{signer.pubkey_short}xx/a.md")
