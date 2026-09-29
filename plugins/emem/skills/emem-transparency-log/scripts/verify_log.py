#!/usr/bin/env python3
"""Offline checks for emem's RFC 6962 transparency log.

    verify_log.py sth STH.json
        Verify a signed tree head (the body of GET /v1/log/sth).

    verify_log.py inclusion INCLUSION.json
        Verify the STH carried in a GET /v1/log/inclusion response, then
        recompute its root from leaf_hash_b32, leaf_index and the audit path.

    verify_log.py consistency OLD_STH.json NEW_STH.json CONSISTENCY.json
        Verify both STHs, then check that the GET /v1/log/consistency proof
        (first = OLD size, second = NEW size) links OLD's root to NEW's root,
        i.e. the log only grew between them.

    verify_log.py --self-test
        Check the bundled BLAKE3 and Ed25519 code against published vectors.

Every argument is a file path. Nothing here touches the network, and the
only import outside the standard library is ../../../lib/emem_crypto.py, which
ships with the plugin. Exit 0 when every check passed, 1 when one failed,
2 on bad input.

The rules are ported from crates/emem-attest/src/translog.rs and the STH
signer in crates/emem-api-rest/src/lib.rs:

  leaf  = blake3(0x00 || entry_hash)
  node  = blake3(0x01 || left || right)       lone nodes promoted, not duplicated
  sth   = ed25519 over PreimageV1("emem.translog.sth.v1"){1: u64_be tree_size,
          2: root (32 bytes), 3: signed_at (UTF-8), 4: responder pubkey (32 bytes)}

PreimageV1(domain) is blake3("emem.preimage.v1\\0" || u32le(len(domain)) ||
domain || segments), each segment tag || u32le(len) || bytes.
"""
from __future__ import annotations

import base64
import json
import struct
import sys
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "lib"))
try:
    from emem_crypto import blake3, ed25519_verify, self_test
except ImportError:
    sys.stderr.write("emem_crypto.py not found: it ships in the emem plugin's lib/ directory, three levels above this script\n")
    sys.exit(2)


def b32d(s: str) -> bytes:
    s = s.strip().upper()
    return base64.b32decode(s + "=" * ((8 - len(s) % 8) % 8))


def load(arg: str):
    raw = sys.stdin.read() if arg == "-" else Path(arg).read_text()
    doc = json.loads(raw)
    if isinstance(doc, dict) and doc.get("schema") == "emem.error.v1":
        raise ValueError(f"{arg} is an error, not a log object: {doc.get('code')}: {doc.get('message')}")
    return doc


def sth_of(doc: dict) -> dict:
    return doc["sth"] if "sth" in doc else doc


def node(left: bytes, right: bytes) -> bytes:
    return blake3(b"\x01" + left + right).digest()


def verify_sth(sth: dict) -> bool:
    pk = b32d(sth["responder_pubkey_b32"])
    h = blake3()
    h.update(b"emem.preimage.v1\x00")
    dom = b"emem.translog.sth.v1"
    h.update(struct.pack("<I", len(dom)) + dom)
    for tag, data in (
        (1, struct.pack(">Q", int(sth["tree_size"]))),
        (2, b32d(sth["root_b32"])),
        (3, sth["signed_at"].encode()),
        (4, pk),
    ):
        h.update(bytes([tag]) + struct.pack("<I", len(data)) + data)
    if not ed25519_verify(pk, b32d(sth["signature_b32"]), h.digest()):
        print(f"sth          INVALID  tree_size={sth['tree_size']}")
        return False
    print(f"sth          VALID    tree_size={sth['tree_size']} signed_at={sth['signed_at']} signer={sth['responder_pubkey_b32']}")
    return True


def root_from_inclusion(leaf: bytes, m: int, size: int, path: list[bytes]):
    """RFC 6962 section 2.1.1 verification, as translog::verify_inclusion."""
    if m >= size:
        return None
    fn, sn, acc = m, size - 1, leaf
    it = iter(path)
    while sn > 0:
        sib = next(it, None)
        if sib is None:
            return None
        if fn % 2 == 1 or fn == sn:
            acc = node(sib, acc)
            while fn % 2 == 0 and fn != 0:
                fn >>= 1
                sn >>= 1
        else:
            acc = node(acc, sib)
        fn >>= 1
        sn >>= 1
    if next(it, None) is not None:
        return None
    return acc


def consistent(first_size, first_root, second_size, second_root, proof) -> bool:
    """RFC 6962 section 2.1.2 verification, as translog::verify_consistency."""
    if first_size == 0 or first_size > second_size:
        return False
    if first_size == second_size:
        return not proof and first_root == second_root
    proof = list(proof)
    fn, sn = first_size - 1, second_size - 1
    if first_size & (first_size - 1) == 0:
        proof.insert(0, first_root)
    if not proof:
        return False
    while fn % 2 == 1:
        fn >>= 1
        sn >>= 1
    fr = sr = proof[0]
    for c in proof[1:]:
        if sn == 0:
            return False
        if fn % 2 == 1 or fn == sn:
            fr = node(c, fr)
            sr = node(c, sr)
            while fn % 2 == 0 and fn != 0:
                fn >>= 1
                sn >>= 1
        else:
            sr = node(sr, c)
        fn >>= 1
        sn >>= 1
    return sn == 0 and fr == first_root and sr == second_root


def main() -> int:
    if sys.argv[1:] == ["--self-test"]:
        return 0 if self_test() else 1
    if len(sys.argv) < 3:
        sys.stderr.write(__doc__)
        return 2
    cmd, args = sys.argv[1], sys.argv[2:]
    try:
        docs = [load(a) for a in args]
    except (OSError, ValueError) as e:
        sys.stderr.write(f"{e}\n")
        return 2

    if cmd == "sth" and len(docs) == 1:
        return 0 if verify_sth(sth_of(docs[0])) else 1

    if cmd == "inclusion" and len(docs) == 1:
        inc = docs[0]
        sth = sth_of(inc)
        ok = verify_sth(sth)
        if "leaf_hash_b32" in inc:
            leaf = b32d(inc["leaf_hash_b32"])
        else:
            leaf = blake3(b"\x00" + b32d(inc["entry_hash_b32"])).digest()
        path = [b32d(p) for p in inc["audit_path_b32"]]
        got = root_from_inclusion(leaf, int(inc["leaf_index"]), int(inc["tree_size"]), path)
        same = got is not None and got == b32d(sth["root_b32"]) and int(inc["tree_size"]) == int(sth["tree_size"])
        print(f"inclusion    {'VALID  ' if same else 'INVALID'}  leaf_index={inc['leaf_index']} path={len(path)} hashes")
        return 0 if ok and same else 1

    if cmd == "consistency" and len(docs) == 3:
        old, new, cons = sth_of(docs[0]), sth_of(docs[1]), docs[2]
        ok = verify_sth(old) & verify_sth(new)
        if int(cons["first_size"]) != int(old["tree_size"]) or int(cons["second_size"]) != int(new["tree_size"]):
            print("consistency  INVALID  the proof's sizes do not match the two STHs; "
                  "request first=<old tree_size>&second=<new tree_size>")
            return 1
        proof = [b32d(p) for p in cons["consistency_proof_b32"]]
        same = consistent(int(old["tree_size"]), b32d(old["root_b32"]),
                          int(new["tree_size"]), b32d(new["root_b32"]), proof)
        print(f"consistency  {'VALID  ' if same else 'INVALID'}  {old['tree_size']} -> {new['tree_size']}, {len(proof)} hashes")
        return 0 if ok and same else 1

    sys.stderr.write(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main())
