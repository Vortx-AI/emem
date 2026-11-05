#!/usr/bin/env python3
"""Tests for verify_note.py: the supersede banner and the key card.

    python3 test_verify_note.py

Every key here is a throwaway made with os.urandom. The signatures are
built from the server's rule as written here, not with verify_note.py's
own functions, so a wrong rule in the script fails these tests. The key
card tests serve cards from a local server on 127.0.0.1. Signing needs
the `cryptography` package; without it the tests are skipped, unless
EMEM_TESTS_REQUIRE_CRYPTOGRAPHY=1 makes a missing package a failure.
"""
from __future__ import annotations

import base64
import json
import os
import subprocess
import sys
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from emem_crypto import blake3  # noqa: E402

try:
    from cryptography.hazmat.primitives import serialization
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
except ImportError:
    Ed25519PrivateKey = None
if Ed25519PrivateKey is None and os.environ.get("EMEM_TESTS_REQUIRE_CRYPTOGRAPHY") == "1":
    sys.exit("EMEM_TESTS_REQUIRE_CRYPTOGRAPHY=1, and the cryptography package is missing")


def b32(b: bytes) -> str:
    return base64.b32encode(b).decode().rstrip("=").lower()


def keypair():
    k = Ed25519PrivateKey.from_private_bytes(os.urandom(32))
    return k, b32(k.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw))


def digest(verb: str, path: str, body_hash: bytes, base: str | None) -> bytes:
    if base is None:
        return blake3(b"emem.memory_write|" + verb.encode() + b"|" + path.encode() + b"|" + body_hash).digest()
    return blake3(b"emem.memory_write.v2|" + verb.encode() + b"|" + path.encode() + b"|" + body_hash
                  + b"|" + base.encode()).digest()


def note_view(k, pub: str, path: str, content: str) -> dict:
    bh = blake3(content.encode()).digest()
    return {"path": path, "content": content, "file_cid": b32(os.urandom(16)), "_superseded": None,
            "authorship": {"caller_signed": True, "attester_pubkey_b32": pub, "verb": "create",
                           "signed_path": path, "body_hash_hex": bh.hex(), "preimage_version": 2,
                           "base": None, "sig_b32": b32(k.sign(digest("create", path, bh, "absent")))}}


def supersede(view: dict, k, pub: str, target: str, reason: str, base: str | None,
              signed_base: str | None = None) -> dict:
    """The banner memory_view serves after a supersede signed with key `k`.
    `signed_base`, when given, is the base the server says the signature named."""
    bh = blake3(f"{target}|{reason}".encode()).digest()
    v = json.loads(json.dumps(view))
    v["_superseded"] = {"this_note_was_replaced_by": target, "reason": reason, "attester_pubkey_b32": pub,
                        "sig_b32": b32(k.sign(digest("supersede", v["path"], bh, base))),
                        "log_entry_hash_b32": b32(os.urandom(32))}
    if signed_base is not None:
        v["_superseded"]["signed_base"] = signed_base
        v["_superseded"]["signed_base_source"] = "recorded at the supersede"
    return v


def run(view: dict, *args: str) -> tuple[int, str]:
    with tempfile.TemporaryDirectory() as d:
        p = Path(d, "view.json")
        p.write_text(json.dumps(view))
        r = subprocess.run([sys.executable, str(HERE / "verify_note.py"), str(p), *args],
                           capture_output=True, text=True, timeout=60)
    return r.returncode, r.stdout + r.stderr


@unittest.skipIf(Ed25519PrivateKey is None, "the cryptography package is not installed")
class Banner(unittest.TestCase):
    def setUp(self):
        self.k, self.pub = keypair()
        self.view = note_view(self.k, self.pub, f"/memories/by_attester/{self.pub[:8]}/claim.md", "# claim\n\nx = 3\n")
        self.target = b32(os.urandom(16))

    def test_no_banner_is_not_a_failure(self):
        code, out = run(self.view, "--offline")
        self.assertEqual(code, 0, out)
        self.assertIn("banner  NONE", out)

    def test_v2_banner_with_the_note_file_cid_as_base_verifies(self):
        v = supersede(self.view, self.k, self.pub, self.target, "sign flipped", self.view["file_cid"])
        code, out = run(v, "--offline")
        self.assertEqual(code, 0, out)
        self.assertIn("banner  VALID     supersede v2 with base from this note's file_cid, "
                      f"by this note's key; replaced by {self.target}", out)

    def test_v1_banner_verifies(self):
        v = supersede(self.view, self.k, self.pub, self.target, "sign flipped", None)
        code, out = run(v, "--offline")
        self.assertEqual(code, 0, out)
        self.assertIn("banner  VALID     supersede v1, which has no base", out)

    def test_empty_reason_verifies(self):
        v = supersede(self.view, self.k, self.pub, self.target, "", self.view["file_cid"])
        self.assertEqual(run(v, "--offline")[0], 0)

    def test_tampered_reason_fails(self):
        v = supersede(self.view, self.k, self.pub, self.target, "sign flipped", self.view["file_cid"])
        v["_superseded"]["reason"] = "sign flipped, and the old value was right"
        code, out = run(v, "--offline")
        self.assertEqual(code, 1, out)
        self.assertIn("banner  INVALID", out)
        self.assertIn("body    MATCH", out)

    def test_retargeted_banner_fails(self):
        v = supersede(self.view, self.k, self.pub, self.target, "sign flipped", self.view["file_cid"])
        v["_superseded"]["this_note_was_replaced_by"] = b32(os.urandom(16))
        self.assertEqual(run(v, "--offline")[0], 1)

    def test_v2_over_another_base_fails(self):
        v = supersede(self.view, self.k, self.pub, self.target, "sign flipped", b32(os.urandom(16)))
        self.assertEqual(run(v, "--offline")[0], 1)

    def test_signed_base_from_the_banner_is_used(self):
        # A base other than the file_cid the view shows, as after a later edit of the path.
        other_base = b32(os.urandom(16))
        v = supersede(self.view, self.k, self.pub, self.target, "sign flipped", other_base, signed_base=other_base)
        code, out = run(v, "--offline")
        self.assertEqual(code, 0, out)
        self.assertIn("supersede v2 with base from the banner's signed_base", out)
        del v["_superseded"]["signed_base"]
        code, out = run(v, "--offline")
        self.assertEqual(code, 1, out)
        self.assertIn("with base from this note's file_cid", out)

    def test_signed_base_wins_over_the_file_cid(self):
        v = supersede(self.view, self.k, self.pub, self.target, "sign flipped", self.view["file_cid"],
                      signed_base=b32(os.urandom(16)))
        code, out = run(v, "--offline")
        self.assertEqual(code, 1, out)
        self.assertIn("banner  INVALID", out)
        self.assertIn("the banner's signed_base", out)

    def test_banner_signed_by_another_key_says_so(self):
        k2, pub2 = keypair()
        v = supersede(self.view, k2, pub2, self.target, "sign flipped", self.view["file_cid"])
        code, out = run(v, "--offline")
        self.assertEqual(code, 0, out)
        self.assertIn(f"by another key {pub2}", out)

    def test_banner_key_that_did_not_sign_fails(self):
        _, pub2 = keypair()
        v = supersede(self.view, self.k, self.pub, self.target, "sign flipped", self.view["file_cid"])
        v["_superseded"]["attester_pubkey_b32"] = pub2
        self.assertEqual(run(v, "--offline")[0], 1)

    def test_unsigned_banner_is_reported_not_failed(self):
        v = json.loads(json.dumps(self.view))
        v["_superseded"] = {"this_note_was_replaced_by": self.target, "reason": None,
                            "attester_pubkey_b32": None, "sig_b32": None, "log_entry_hash_b32": None}
        code, out = run(v, "--offline")
        self.assertEqual(code, 0, out)
        self.assertIn("banner  UNSIGNED", out)


CARDS: dict = {}


class _Cards(BaseHTTPRequestHandler):
    def do_GET(self):  # noqa: N802 (the name http.server calls)
        doc = CARDS.get(self.path)
        if doc is None:
            self.send_response(404)
            self.end_headers()
            return
        body = json.dumps(doc).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *a):
        pass


@unittest.skipIf(Ed25519PrivateKey is None, "the cryptography package is not installed")
class KeyCard(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.srv = HTTPServer(("127.0.0.1", 0), _Cards)
        threading.Thread(target=cls.srv.serve_forever, daemon=True).start()
        cls.origin = f"http://127.0.0.1:{cls.srv.server_port}"

    @classmethod
    def tearDownClass(cls):
        cls.srv.shutdown()

    def setUp(self):
        CARDS.clear()
        self.k, self.pub = keypair()
        self.view = note_view(self.k, self.pub, f"/memories/by_attester/{self.pub[:8]}/hello.md", "# hello\n")
        self.card_path = f"/memories/by_attester/{self.pub[:8]}/key-card.md"
        self.card = "# key card\n\nrole: witness\noperator: Example Lab\x1b[2J\nother_keys_of_this_operator: abcdefgh\n"

    def test_no_card(self):
        code, out = run(self.view, "--origin", self.origin)
        self.assertEqual(code, 0, out)
        self.assertIn("card    NONE", out)

    def test_card_by_the_same_key_is_shown_as_a_claim(self):
        CARDS[self.card_path] = note_view(self.k, self.pub, self.card_path, self.card)
        code, out = run(self.view, "--origin", self.origin)
        self.assertEqual(code, 0, out)
        self.assertIn("role      witness  (the author's own claim)", out)
        self.assertIn("others    abcdefgh", out)
        self.assertNotIn("\x1b", out)

    def test_card_by_another_key_is_not_shown(self):
        k2, pub2 = keypair()
        CARDS[self.card_path] = note_view(k2, pub2, self.card_path, self.card)
        code, out = run(self.view, "--origin", self.origin)
        self.assertEqual(code, 0, out)
        self.assertIn("card    FOREIGN", out)
        self.assertNotIn("witness", out)

    def test_changed_card_is_not_shown(self):
        doc = note_view(self.k, self.pub, self.card_path, self.card)
        doc["content"] = self.card.replace("witness", "agent")
        CARDS[self.card_path] = doc
        code, out = run(self.view, "--origin", self.origin)
        self.assertEqual(code, 0, out)
        self.assertIn("card    INVALID", out)

    def test_network_failure_does_not_fail_the_note(self):
        code, out = run(self.view, "--origin", "http://127.0.0.1:9")
        self.assertEqual(code, 0, out)
        self.assertIn("card    UNCHECKED", out)


if __name__ == "__main__":
    unittest.main(verbosity=2)
