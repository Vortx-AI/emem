#!/usr/bin/env python3
"""Tests for emem_signer.py, driven over stdio as Claude Code drives it.

    python3 test_signer.py

Every seed here is a throwaway made with os.urandom. The tests need the
`cryptography` package, as the signer does; without it they are skipped,
unless EMEM_TESTS_REQUIRE_CRYPTOGRAPHY=1 makes a missing package a failure.
"""
from __future__ import annotations

import base64
import json
import os
import subprocess
import sys
import unittest
from pathlib import Path

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from emem_crypto import blake3, ed25519_verify  # noqa: E402

try:
    import cryptography  # noqa: F401
    import emem_sealed
except ImportError:
    emem_sealed = None
if emem_sealed is None and os.environ.get("EMEM_TESTS_REQUIRE_CRYPTOGRAPHY") == "1":
    sys.exit("EMEM_TESTS_REQUIRE_CRYPTOGRAPHY=1, and the cryptography package is missing")


def b32d(s: str) -> bytes:
    s = s.upper()
    return base64.b32decode(s + "=" * ((8 - len(s) % 8) % 8))


def digest(verb: str, path: str, text: str, base: str) -> bytes:
    return blake3(b"emem.memory_write.v2|" + verb.encode() + b"|" + path.encode() + b"|"
                  + blake3(text.encode()).digest() + b"|" + base.encode()).digest()


def call(env: dict, calls: list) -> tuple[list, list, str]:
    """(tools, [(text, is_error)], raw stdout) for a list of (tool, arguments)."""
    e = {k: v for k, v in os.environ.items() if not k.startswith("EMEM_SIGNING")}
    e.update(env)
    lines = [json.dumps({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}),
             json.dumps({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})]
    for i, (name, args) in enumerate(calls, 2):
        lines.append(json.dumps({"jsonrpc": "2.0", "id": i, "method": "tools/call",
                                 "params": {"name": name, "arguments": args}}))
    r = subprocess.run([sys.executable, str(HERE / "emem_signer.py")], input="\n".join(lines) + "\n",
                       capture_output=True, text=True, env=e, timeout=60)
    if r.returncode != 0:
        raise AssertionError(r.stderr)
    replies = [json.loads(line) for line in r.stdout.splitlines()]
    results = [(x["result"]["content"][0]["text"], x["result"]["isError"]) for x in replies[2:]]
    return replies[1]["result"]["tools"], results, r.stdout + r.stderr


class Digest(unittest.TestCase):
    def test_local_digest_reproduces_a_digest_the_server_named(self):
        # Refusal of an unsigned create of "hello", recorded 2026-09-28.
        d = digest("create", "/memories/by_attester/abcdefgh/probe.md", "hello", "absent").hex()
        self.assertTrue(d.startswith("a2f3d2bd") and d.endswith("195368"), d)


@unittest.skipIf(emem_sealed is None, "the cryptography package is not installed")
class Signer(unittest.TestCase):
    def setUp(self):
        self.seed = os.urandom(32).hex()
        _, res, _ = call({"EMEM_SIGNING_SEED_HEX": self.seed}, [("signer_whoami", {})])
        self.pub = json.loads(res[0][0])["pubkey_b32"]
        self.path = f"/memories/by_attester/{self.pub[:8]}/scratch/note.md"
        self.text = "# note\n\nhello\n"

    def env(self, role=None):
        e = {"EMEM_SIGNING_SEED_HEX": self.seed}
        if role is not None:
            e["EMEM_SIGNING_ROLE"] = role
        return e

    def test_unconfigured(self):
        _, res, _ = call({}, [("signer_whoami", {}), ("signer_open_sealed", {"sealed_note": "x"})])
        w = json.loads(res[0][0])
        self.assertEqual((w["configured"], w["role"]), (False, None))
        self.assertTrue(res[1][1])

    def test_role_is_reported_and_a_placeholder_is_no_role(self):
        _, res, _ = call(self.env("agent"), [("signer_whoami", {})])
        self.assertEqual(json.loads(res[0][0])["role"], "agent")
        _, res, _ = call(self.env("${user_config.signing_role}"), [("signer_whoami", {})])
        self.assertIsNone(json.loads(res[0][0])["role"])

    def test_signs_without_a_refusal_first(self):
        args = {"verb": "create", "path": self.path, "text": self.text, "base": "absent"}
        _, res, _ = call(self.env("agent"), [
            ("signer_sign_write", args),
            ("signer_sign_write", dict(args, expected_digest_hex="00" * 32)),
            ("signer_sign_write", dict(args, expected_digest_hex=digest("create", self.path, self.text, "absent").hex())),
        ])
        r = json.loads(res[0][0])
        self.assertTrue(ed25519_verify(b32d(r["pubkey_b32"]), b32d(r["sig_b32"]),
                                       digest("create", self.path, self.text, "absent")))
        self.assertTrue(res[1][1])
        self.assertIn("Nothing was signed", res[1][0])
        self.assertFalse(res[2][1])

    def test_a_witness_key_signs_notes_only_when_confirmed(self):
        args = {"verb": "create", "path": self.path, "text": self.text, "base": "absent"}
        witness_path = f"/memories/by_attester/{self.pub[:8]}/witness/sth-1.md"
        for role in ("witness", "Transparency-log Witness"):
            _, res, _ = call(self.env(role), [
                ("signer_sign_write", args),
                ("signer_sign_write", dict(args, confirm_role=True)),
                ("signer_sign_write", dict(args, path=witness_path)),
                ("signer_sign_write", dict(args, confirm_role="true")),
            ])
            self.assertTrue(res[0][1] and "confirm_role: true" in res[0][0] and "sig_b32" not in res[0][0], role)
            self.assertFalse(res[1][1], role)
            self.assertFalse(res[2][1], role)
            self.assertTrue(res[3][1], role)
        _, res, _ = call(self.env("operator"), [("signer_sign_write", args)])
        self.assertFalse(res[0][1])

    def test_open_sealed(self):
        author = self.pub
        note = emem_sealed.seal("meet at the north gate\n".encode(), self.pub, author)
        binary = emem_sealed.seal(b"\xff\xfe\x00", self.pub, author)
        lines = note.splitlines(keepends=True)
        i = next(n for n, line in enumerate(lines) if line.startswith("```text")) + 1
        lines[i] = ("B" if lines[i][0] != "B" else "C") + lines[i][1:]
        tampered = "".join(lines)
        stranger = "a" * 52
        tools, res, raw = call(self.env(), [
            ("signer_open_sealed", {"sealed_note": note}),
            ("signer_open_sealed", {"sealed_note": note, "note_signer_pubkey_b32": author}),
            ("signer_open_sealed", {"sealed_note": note, "note_signer_pubkey_b32": stranger}),
            ("signer_open_sealed", {"sealed_note": tampered}),
            ("signer_open_sealed", {"sealed_note": binary}),
        ])
        self.assertIn("signer_open_sealed", [t["name"] for t in tools])
        first = json.loads(res[0][0])
        self.assertEqual(first["plaintext"], "meet at the north gate\n")
        self.assertEqual(first["header"]["recipient"], self.pub)
        self.assertIsNone(first["author_matches_note_signer"])
        self.assertTrue(json.loads(res[1][0])["author_matches_note_signer"])
        self.assertTrue(res[2][1] and "re-signed" in res[2][0])
        self.assertTrue(res[3][1] and "does not open" in res[3][0])
        self.assertEqual(base64.b64decode(json.loads(res[4][0])["plaintext_b64"]), b"\xff\xfe\x00")
        self.assertNotIn(self.seed, raw)

    def test_a_note_for_another_key_is_not_opened(self):
        other = os.urandom(32).hex()
        _, res, _ = call({"EMEM_SIGNING_SEED_HEX": other}, [("signer_whoami", {})])
        other_pub = json.loads(res[0][0])["pubkey_b32"]
        note = emem_sealed.seal(b"for the other key", other_pub, self.pub)
        _, res, raw = call(self.env(), [("signer_open_sealed", {"sealed_note": note})])
        self.assertTrue(res[0][1] and "sealed to" in res[0][0])
        self.assertNotIn(self.seed, raw)


if __name__ == "__main__":
    unittest.main(verbosity=2)
