#!/usr/bin/env python3
"""Verify who wrote an emem memory note, offline.

    verify_note.py VIEW.json [NOTE_FILE]
    verify_note.py --self-test      # check the bundled crypto

VIEW.json is a saved emem_memory_view result: either the whole JSON-RPC
response from https://emem.dev/mcp, or the JSON object inside its
result.content[0].text. A note too large for one MCP result arrives with
`content` omitted; download it (GET https://emem.dev<signed_path>) and pass
the file as NOTE_FILE. The receipt on that response proves the responder
stored and served the note; this script checks the other claim, that the
key in `authorship` signed these bytes at this path:

  body    blake3(content) == body_hash_hex    (create, str_replace, insert)
  sig     ed25519 over the per-verb memory-write digest verifies against
          authorship.attester_pubkey_b32

The digest, from `caller_signed_objects` in GET /v1/verifier_spec:

  v1  blake3("emem.memory_write|" + verb + "|" + signed_path + "|" + body_hash)
  v2  blake3("emem.memory_write.v2|" + verb + "|" + signed_path + "|" + body_hash
             + "|" + base)

body_hash enters as 32 raw bytes; base is the replaced file_cid or the
literal "absent". Exit 0 when every check passed, 1 when one failed, 2 on
bad input or a note with no caller signature. No network calls; BLAKE3 and
Ed25519 come from ../../lib/emem_crypto.py, plain Python shipped with the
plugin. A valid signature says which key wrote the note, never that what
it says is true.
"""
from __future__ import annotations

import base64
import json
import sys
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "lib"))
try:
    from emem_crypto import blake3, ed25519_verify, self_test
except ImportError:
    sys.stderr.write("emem_crypto.py not found: it ships in the emem plugin's lib/ directory, two levels above this script\n")
    sys.exit(2)


def b32d(s: str) -> bytes:
    s = s.strip().upper()
    return base64.b32decode(s + "=" * ((8 - len(s) % 8) % 8))


def load_view(path: str) -> dict:
    doc = json.loads(Path(path).read_text())
    if isinstance(doc, dict) and "result" in doc:
        doc = doc["result"]
    if isinstance(doc, dict) and isinstance(doc.get("content"), list):
        doc = json.loads(doc["content"][0]["text"])
    if not isinstance(doc, dict) or "authorship" not in doc:
        raise ValueError("not an emem_memory_view result: no authorship block")
    return doc


def main() -> int:
    if sys.argv[1:] == ["--self-test"]:
        return 0 if self_test() else 1
    if len(sys.argv) not in (2, 3):
        sys.stderr.write(__doc__)
        return 2
    try:
        view = load_view(sys.argv[1])
    except (OSError, ValueError, KeyError, IndexError) as e:
        sys.stderr.write(f"cannot read the view: {e}\n")
        return 2
    a = view["authorship"]
    if not a.get("caller_signed") or not a.get("sig_b32"):
        sys.stderr.write("this note carries no caller signature; only the responder's receipt vouches for it\n")
        return 2

    ok = True
    verb = a["verb"]
    body_hash = bytes.fromhex(a["body_hash_hex"])
    if verb in ("create", "str_replace", "insert"):
        if len(sys.argv) == 3:
            body = Path(sys.argv[2]).read_bytes()
        elif isinstance(view.get("content"), str):
            body = view["content"].encode("utf-8")
        else:
            sys.stderr.write("the view omits `content` (too large for one MCP result); download "
                             f"https://emem.dev{a['signed_path']} and pass it as the second argument\n")
            return 2
        got = blake3(body).digest()
        same = got == body_hash
        ok &= same
        print(f"body    {'MATCH   ' if same else 'MISMATCH'}  blake3(content)={got.hex()}")
    else:
        print(f"body    SKIPPED   verb={verb} signs no body bytes")

    head = verb.encode() + b"|" + a["signed_path"].encode() + b"|" + body_hash
    version = int(a.get("preimage_version") or 1)
    if version == 2:
        base = a.get("base") or "absent"
        preimage = b"emem.memory_write.v2|" + head + b"|" + base.encode()
    else:
        preimage = b"emem.memory_write|" + head
    signer = a["attester_pubkey_b32"]
    good = ed25519_verify(b32d(signer), b32d(a["sig_b32"]), blake3(preimage).digest())
    ok &= good
    print(f"sig     {'VALID   ' if good else 'INVALID '}  v{version} verb={verb} signer={signer}")
    print(f"path    {a['signed_path']}")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
