#!/usr/bin/env python3
"""Tests for emem.sealed.v1 (emem_sealed.py and seal_note.py).

    python3 test_sealed.py

Every key here is a throwaway made with os.urandom. The control checks the
Ed25519-to-X25519 conversions against libsodium through PyNaCl, on random
keys and on points both must refuse. It is skipped, and says so, when
PyNaCl is not installed. The other tests need the `cryptography` package;
EMEM_TESTS_REQUIRE_CRYPTOGRAPHY=1 makes a missing package a failure rather
than a skip. The emem-signer plugin opens notes with its own copy of
emem_sealed.py, which must stay byte-identical to this one.
"""
from __future__ import annotations

import base64
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import emem_sealed as es  # noqa: E402
from emem_crypto import _L, _P, _add, _decompress, _mul  # noqa: E402

try:
    import nacl.bindings as sodium
except ImportError:
    sodium = None

try:
    from cryptography.hazmat.primitives import serialization
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
    from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey
except ImportError:
    Ed25519PrivateKey = None
if Ed25519PrivateKey is None and os.environ.get("EMEM_TESTS_REQUIRE_CRYPTOGRAPHY") == "1":
    sys.exit("EMEM_TESTS_REQUIRE_CRYPTOGRAPHY=1, and the cryptography package is missing")


def raw(pub) -> bytes:
    return pub.public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)


def keypair():
    """(seed, pubkey_b32) for a throwaway key."""
    seed = os.urandom(32)
    return seed, es.b32e(raw(Ed25519PrivateKey.from_private_bytes(seed).public_key()))


def compress(p) -> bytes:
    zi = pow(p[2], _P - 2, _P)
    x, y = p[0] * zi % _P, p[1] * zi % _P
    return (y | ((x & 1) << 255)).to_bytes(32, "little")


def random_point():
    while True:
        p = _decompress(os.urandom(32))
        if p is not None:
            return p


def torsion_generator():
    """A point of order 8."""
    while True:
        t = _mul(_L, random_point())
        if len({compress(_mul(k, t)) for k in range(8)}) == 8:
            return t


def torsion_points() -> list[bytes]:
    """The eight points of small order, as canonical encodings."""
    t = torsion_generator()
    return sorted({compress(_mul(k, t)) for k in range(8)})


def invalid_points() -> list[bytes]:
    """Small-order points, a point of mixed order, and bytes that are no point."""
    out = torsion_points()
    prime_order = _mul(8, random_point())
    out.append(compress(_add(prime_order, torsion_generator())))
    while True:
        b = os.urandom(32)
        if _decompress(b) is None and int.from_bytes(b, "little") & ((1 << 255) - 1) < _P:
            out.append(b)
            return out


@unittest.skipIf(sodium is None, "PyNaCl is not installed: the libsodium control did not run")
class ControlAgainstLibsodium(unittest.TestCase):
    def test_public_key_conversion_matches(self):
        for _ in range(32):
            pk, _sk = sodium.crypto_sign_seed_keypair(os.urandom(32))
            self.assertEqual(es.ed25519_pk_to_x25519(pk), sodium.crypto_sign_ed25519_pk_to_curve25519(pk))

    def test_secret_key_conversion_matches(self):
        for _ in range(32):
            seed = os.urandom(32)
            _pk, sk = sodium.crypto_sign_seed_keypair(seed)
            self.assertEqual(es.ed25519_seed_to_x25519(seed), sodium.crypto_sign_ed25519_sk_to_curve25519(sk))

    def test_both_refuse_the_same_invalid_points(self):
        for enc in invalid_points():
            with self.assertRaises(Exception, msg=f"libsodium accepted {enc.hex()}"):
                sodium.crypto_sign_ed25519_pk_to_curve25519(enc)
            with self.assertRaises(es.SealedError, msg=f"emem_sealed accepted {enc.hex()}"):
                es.ed25519_pk_to_x25519(enc)


@unittest.skipIf(Ed25519PrivateKey is None, "the cryptography package is not installed")
class Sealed(unittest.TestCase):
    def setUp(self):
        self.seed, self.pub = keypair()
        self.author_seed, self.author = keypair()
        self.plain = "# findings\n\nthe gate is open at km 41.2\néè ✓\n".encode() + os.urandom(3000)
        self.note = es.seal(self.plain, self.pub, self.author)

    def test_x25519_public_from_seed_equals_converted_public(self):
        for _ in range(8):
            seed, pub = keypair()
            x_pub = raw(X25519PrivateKey.from_private_bytes(es.ed25519_seed_to_x25519(seed)).public_key())
            self.assertEqual(x_pub, es.ed25519_pk_to_x25519(es.b32d(pub, 32, "key")))

    def test_round_trip(self):
        plain, fields = es.open_sealed(self.note, self.seed)
        self.assertEqual(plain, self.plain)
        self.assertEqual(fields["author"], self.author)
        self.assertEqual(fields["to"], self.pub[:8])

    def test_empty_plaintext_round_trip(self):
        self.assertEqual(es.open_sealed(es.seal(b"", self.pub, self.author), self.seed)[0], b"")

    def test_each_seal_is_fresh(self):
        self.assertNotEqual(self.note, es.seal(self.plain, self.pub, self.author))

    def test_wrong_key_is_refused(self):
        other_seed, _ = keypair()
        with self.assertRaisesRegex(es.SealedError, "sealed to"):
            es.open_sealed(self.note, other_seed)

    def test_relabelled_recipient_does_not_open(self):
        other_seed, other = keypair()
        moved = self.note.replace(f"recipient: {self.pub}", f"recipient: {other}")
        moved = moved.replace(f"to: {self.pub[:8]}", f"to: {other[:8]}")
        with self.assertRaisesRegex(es.SealedError, "does not open"):
            es.open_sealed(moved, other_seed)

    def test_tampered_ciphertext_does_not_open(self):
        lines = self.note.splitlines(keepends=True)
        i = next(n for n, line in enumerate(lines) if line.startswith("```text")) + 3
        c = lines[i][10]
        lines[i] = lines[i][:10] + ("A" if c != "A" else "B") + lines[i][11:]
        with self.assertRaisesRegex(es.SealedError, "does not open"):
            es.open_sealed("".join(lines), self.seed)

    def test_truncated_ciphertext_does_not_open(self):
        block = self.note.split("```text\n", 1)[1].split("\n```", 1)[0]
        short = base64.b64encode(es.parse(self.note)["ciphertext"][:-1]).decode()
        with self.assertRaisesRegex(es.SealedError, "does not open"):
            es.open_sealed(self.note.replace(block, short), self.seed)

    def test_non_canonical_key_is_refused(self):
        for y in (_P, _P + 1, _P + 3):
            with self.assertRaises(es.SealedError):
                es.ed25519_pk_to_x25519(y.to_bytes(32, "little"))

    def test_tampered_header_does_not_open(self):
        f = es.parse(self.note)["fields"]
        _, other = keypair()
        eph = es.b32e(raw(X25519PrivateKey.generate().public_key()))
        changes = {
            "to": f"to: {other[:8]}",
            "author": f"author: {other}",
            "ephemeral": f"ephemeral: {eph}",
            "nonce": f"nonce: {es.b32e(os.urandom(12))}",
        }
        for k, new in changes.items():
            bad = self.note.replace(f"{k}: {f[k]}", new, 1)
            self.assertNotEqual(bad, self.note)
            with self.assertRaises(es.SealedError, msg=f"opened with a changed {k} line"):
                es.open_sealed(bad, self.seed)
        extra = self.note.replace("---\n# Sealed", "line: hello\n---\n# Sealed", 1)
        with self.assertRaisesRegex(es.SealedError, "does not open"):
            es.open_sealed(extra, self.seed)
        with self.assertRaisesRegex(es.SealedError, "schema"):
            es.open_sealed(self.note.replace("schema: emem.sealed.v1", "schema: emem.sealed.v2"), self.seed)

    def test_human_lines_are_not_bound(self):
        # Only the header and the ciphertext are bound; the note's signature covers the rest.
        edited = self.note.replace("stay public.", "stay public, as always.")
        self.assertEqual(es.open_sealed(edited, self.seed)[0], self.plain)

    def test_seal_refuses_bad_recipients(self):
        with self.assertRaisesRegex(es.SealedError, "52-character"):
            es.seal(b"x", self.pub[:8], self.author)
        for enc in torsion_points():
            with self.assertRaises(es.SealedError):
                es.seal(b"x", es.b32e(enc), self.author)


@unittest.skipIf(Ed25519PrivateKey is None, "the cryptography package is not installed")
class SealCommand(unittest.TestCase):
    def run_seal(self, args):
        return subprocess.run([sys.executable, str(HERE / "seal_note.py"), *args],
                              capture_output=True, timeout=60)

    def test_seal_note_output_opens_with_the_recipient_seed(self):
        seed, pub = keypair()
        _, author = keypair()
        with tempfile.TemporaryDirectory() as d:
            src, out = Path(d, "plain.md"), Path(d, "sealed.md")
            src.write_bytes(b"meet at the north gate\n")
            r = self.run_seal(["--to", pub, "--author", author, str(src), "-o", str(out)])
            self.assertEqual(r.returncode, 0, r.stderr)
            note = out.read_text()
        self.assertIn(f"to: {pub[:8]}", note)
        self.assertNotIn("north gate", note)
        plain, fields = es.open_sealed(note, seed)
        self.assertEqual((plain, fields["author"]), (b"meet at the north gate\n", author))

    def test_seal_note_refuses_a_shortcode(self):
        _, pub = keypair()
        _, author = keypair()
        with tempfile.TemporaryDirectory() as d:
            src = Path(d, "plain.md")
            src.write_bytes(b"x")
            self.assertEqual(self.run_seal(["--to", pub[:8], "--author", author, str(src)]).returncode, 2)


if __name__ == "__main__":
    unittest.main(verbosity=2)
