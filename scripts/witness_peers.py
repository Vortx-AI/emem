#!/usr/bin/env python3
"""Co-sign every peer's tree head, and prove it grew from the last one we saw.

Phase 0 of the four-node network (docs/federation.md section 8). The whole
mechanism already ships on the server: /v1/log/sth publishes a signed head,
/v1/log/witness accepts an ed25519 co-signature over it, /v1/log/consistency
returns an RFC 6962 proof that one head is an append-only prefix of another.
On 2026-09-02 the log stood at 1.5M entries with head_is_witnessed FALSE and
the freshest co-signature 58,145 entries behind, because nothing ran this.

What a witness proves, exactly: that a node cannot show two different
histories to two different peers without one of them holding a signed head
that fails a consistency proof. That is Certificate Transparency's gossip
property, and it is the reason this network needs no chain.

Three rules this script will not bend:

  1. NEVER co-sign a head you did not verify. The STH signature is checked
     against the responder's published key first. A witness that signs
     unverified heads is a rubber stamp and worse than none.
  2. VERIFY the consistency proof yourself. The server hands back a proof and
     the roots, not a verdict, and that is correct: a server grading its own
     proof proves nothing. RFC 6962 section 2.1.4.2 is implemented below and
     checked against the served roots.
  3. A consistency FAILURE is the finding. It means the log the witness saw
     earlier is not a prefix of the log it sees now: a rollback, a fork, or a
     split view. Exit non-zero, name the peer, keep the evidence, do not
     co-sign.

State: one JSON file of the last verified (tree_size, root) per peer, so the
next run has something to prove consistency AGAINST. First contact records and
co-signs; it cannot prove growth from nothing and says so.
"""
import argparse
import base64
import json
import os
import pathlib
import sys
import time
import urllib.request

import blake3
import nacl.signing

IDENTITY = pathlib.Path.home() / ".config/emem/agent_identity.json"
STATE = pathlib.Path.home() / ".config/emem/witness_state.json"


def b32d(s: str) -> bytes:
    s = s.upper()
    return base64.b32decode(s + "=" * (-len(s) % 8))


def b32e(b: bytes) -> str:
    return base64.b32encode(b).decode().rstrip("=").lower()


def preimage(domain: str, segs) -> bytes:
    """emem_attest::PreimageV1, byte for byte."""
    h = blake3.blake3()
    h.update(b"emem.preimage.v1\x00")
    dm = domain.encode()
    h.update(len(dm).to_bytes(4, "little"))
    h.update(dm)
    for tag, b in segs:
        h.update(bytes([tag]))
        h.update(len(b).to_bytes(4, "little"))
        h.update(b)
    return h.digest()


def get(url: str, timeout=30):
    with urllib.request.urlopen(url, timeout=timeout) as r:
        return json.load(r)


def post(url: str, body: dict, timeout=30):
    req = urllib.request.Request(url, method="POST", data=json.dumps(body).encode(),
                                 headers={"content-type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, json.load(r)
    except urllib.error.HTTPError as e:
        try:
            return e.code, json.load(e)
        except Exception:
            return e.code, {"raw": e.read()[:300].decode(errors="replace")}


def node_hash(left: bytes, right: bytes) -> bytes:
    return blake3.blake3(b"\x01" + left + right).digest()


def verify_consistency(first: int, second: int, first_root: bytes,
                       second_root: bytes, proof: list) -> bool:
    """A line-for-line port of emem_attest::translog::verify_consistency.

    Ported rather than written from the RFC, because my first attempt from
    memory had the spine-advance parity inverted and rejected every real proof
    while still rejecting tampered ones -- strict and wrong, which is the worst
    kind of verifier because it looks careful. The server names this function
    in its own `verify` field; the reference is the thing to match.
    """
    if first == 0 or first > second:
        return False
    if first == second:
        return not proof and first_root == second_root
    proof = list(proof)
    fn_, sn = first - 1, second - 1
    # A power-of-two old tree: its root IS the first node and is not sent.
    if first & (first - 1) == 0:
        proof.insert(0, first_root)
    if not proof:
        return False
    # Advance past the old tree's right spine.
    while fn_ & 1:
        fn_ >>= 1
        sn >>= 1
    it = iter(proof)
    first_node = next(it)
    fr = sr = first_node
    for c in it:
        if sn == 0:
            return False  # proof too long
        if (fn_ & 1) or fn_ == sn:
            fr = node_hash(c, fr)
            sr = node_hash(c, sr)
            while fn_ & 1 == 0 and fn_ != 0:
                fn_ >>= 1
                sn >>= 1
        else:
            sr = node_hash(sr, c)
        fn_ >>= 1
        sn >>= 1
    return sn == 0 and fr == first_root and sr == second_root


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--peers", default=os.environ.get("EMEM_PEERS", "https://emem.dev"),
                    help="comma-separated node origins to witness")
    ap.add_argument("--dry-run", action="store_true", help="verify and report, submit nothing")
    a = ap.parse_args()

    ident = json.loads(IDENTITY.read_text())
    sk = nacl.signing.SigningKey(bytes.fromhex(ident["seed_hex"]))
    my_pk = bytes(sk.verify_key)
    my_pk_b32 = b32e(my_pk)
    assert my_pk_b32 == ident["pubkey_b32"].lower(), "identity file pubkey does not match its seed"

    state = json.loads(STATE.read_text()) if STATE.exists() else {}
    peers = [p.strip().rstrip("/") for p in a.peers.split(",") if p.strip()]
    if not peers:
        print("witness: no peers given; nothing witnessed, which is not a pass")
        return 1

    failures, witnessed = [], 0
    for origin in peers:
        try:
            sth = get(f"{origin}/v1/log/sth")["sth"]
        except Exception as e:
            print(f"  {origin}: unreachable ({str(e)[:50]}); undetermined, not witnessed")
            failures.append(origin)
            continue
        size, root = int(sth["tree_size"]), b32d(sth["root_b32"])
        rpk, sig = b32d(sth["responder_pubkey_b32"]), b32d(sth["signature_b32"])

        # Rule 1: verify before anything.
        msg = preimage("emem.translog.sth.v1", [(1, size.to_bytes(8, "big")), (2, root),
                                                (3, sth["signed_at"].encode()), (4, rpk)])
        try:
            nacl.signing.VerifyKey(rpk).verify(msg, sig)
        except Exception:
            print(f"  {origin}: STH signature does NOT verify against its own key. "
                  f"Refusing to co-sign. This is a finding.")
            failures.append(origin)
            continue

        # Rule 2 and 3: prove growth from what we last saw.
        prev = state.get(origin)
        if prev and prev["tree_size"] < size:
            c = get(f"{origin}/v1/log/consistency?first={prev['tree_size']}&second={size}")
            ok = verify_consistency(prev["tree_size"], size, b32d(prev["root_b32"]), root,
                                    [b32d(x) for x in c["consistency_proof_b32"]])
            served_first_ok = c.get("first_root_b32", "").lower() == prev["root_b32"].lower()
            if not (ok and served_first_ok):
                print(f"  {origin}: CONSISTENCY FAILURE. The head we witnessed at "
                      f"{prev['tree_size']} is not a prefix of the head at {size}.")
                print(f"    served first_root matches ours: {served_first_ok}; proof verifies: {ok}")
                print(f"    This is a rollback, a fork, or a split view. Not co-signed. "
                      f"Evidence kept in {STATE}.")
                failures.append(origin)
                continue
            growth = f"grew {prev['tree_size']} -> {size}, consistency PROVED"
        elif prev and prev["tree_size"] == size:
            # Same size is not "unchanged" until the root says so. A log that
            # rewrites an entry in place keeps its count and changes its root;
            # the first draft of this branch co-signed that without looking.
            if prev["root_b32"].lower() != sth["root_b32"].lower():
                print(f"  {origin}: CONSISTENCY FAILURE. Same tree_size {size}, different root. "
                      f"The log rewrote history in place. Not co-signed. Evidence kept in {STATE}.")
                failures.append(origin)
                continue
            growth = f"unchanged at {size}, same root"
        elif prev:
            print(f"  {origin}: tree SHRANK {prev['tree_size']} -> {size}. That is a rollback. "
                  f"Not co-signed.")
            failures.append(origin)
            continue
        else:
            growth = f"first contact at {size}; nothing to prove growth from yet"

        # Co-sign.
        wmsg = preimage("emem.translog.witness.v1", [(1, size.to_bytes(8, "big")), (2, root),
                                                    (3, my_pk)])
        wsig = bytes(sk.sign(wmsg).signature)
        if a.dry_run:
            print(f"  {origin}: verified, {growth}; dry run, not submitted")
        else:
            code, resp = post(f"{origin}/v1/log/witness", {
                "tree_size": size, "root_b32": sth["root_b32"],
                "witness_pubkey_b32": my_pk_b32, "signature_b32": b32e(wsig)})
            if code != 200:
                print(f"  {origin}: witness POST -> {code}: {json.dumps(resp)[:120]}")
                failures.append(origin)
                continue
            print(f"  {origin}: co-signed tree_size {size}; {growth}")
            witnessed += 1
            # The pin records what we CO-SIGNED, not what we looked at. A dry
            # run that advanced it would erase the chain of evidence a real
            # run needs to prove growth from.
            state[origin] = {"tree_size": size, "root_b32": sth["root_b32"],
                             "signed_at": sth["signed_at"], "witnessed_at": int(time.time())}

    if not a.dry_run:
        STATE.parent.mkdir(parents=True, exist_ok=True)
        STATE.write_text(json.dumps(state, indent=2))
    print(f"\n  {witnessed} head(s) co-signed, {len(failures)} peer(s) failed, "
          f"{len(peers)} attempted, as {my_pk_b32[:8]}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
