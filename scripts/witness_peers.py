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


def verify_sth(sth: dict):
    """Verify a served STH against the key it names. Returns (size, root, responder_pk)
    or raises ValueError. The key is checked by the caller against the head's key."""
    size, root = int(sth["tree_size"]), b32d(sth["root_b32"])
    rpk, sig = b32d(sth["responder_pubkey_b32"]), b32d(sth["signature_b32"])
    msg = preimage("emem.translog.sth.v1", [(1, size.to_bytes(8, "big")), (2, root),
                                            (3, sth["signed_at"].encode()), (4, rpk)])
    try:
        nacl.signing.VerifyKey(rpk).verify(msg, sig)
    except Exception as e:
        raise ValueError("STH signature does not verify") from e
    return size, root, rpk


def leaf_hash(entry: bytes) -> bytes:
    """`blake3(0x00 || entry)`; `entry` is the 32-byte per-record hash the log persists."""
    return blake3.blake3(b"\x00" + entry).digest()


def verify_inclusion(leaf: bytes, m: int, tree_size: int, path: list, root: bytes) -> bool:
    """A line-for-line port of emem_attest::translog::verify_inclusion (RFC 6962 §2.1.1)."""
    if m >= tree_size:
        return False
    fn_, sn = m, tree_size - 1
    acc = leaf
    it = iter(path)
    while sn > 0:
        sibling = next(it, None)
        if sibling is None:
            return False  # path too short
        if (fn_ & 1) or fn_ == sn:
            acc = node_hash(sibling, acc)
            while fn_ & 1 == 0 and fn_ != 0:
                fn_ >>= 1
                sn >>= 1
        else:
            acc = node_hash(acc, sibling)
        fn_ >>= 1
        sn >>= 1
    return next(it, None) is None and acc == root


def audit_indices(root: bytes, my_pk: bytes, size: int, k: int) -> list:
    """k leaf indices derived from the co-signed root and this witness's key.

    A peer cannot know which leaves a witness will ask for before the root
    exists, and two witnesses never ask for the same ones. Filecoin samples
    sectors from chain randomness for the same reason; the root is ours.
    """
    seed = blake3.blake3(b"emem.witness.audit.v1" + root + my_pk).digest(length=8 * k)
    return sorted({int.from_bytes(seed[8 * i:8 * i + 8], "big") % size for i in range(k)})


def audit_custody(origin: str, size: int, root: bytes, rpk: bytes, my_pk: bytes, k: int):
    """Fetch k sampled leaves and their inclusion proofs. Returns (checked, problems).

    `/v1/log/inclusion` proves against the CURRENT head and returns the STH it
    proved against; it does not take a tree size (the first draft passed one,
    the route ignored it, and every proof failed against the older root).
    So each leaf is bound to the co-signed head in two steps: the proof
    reaches the served STH's root, and that STH is an append-only extension
    of the head this witness co-signed. A leaf below the co-signed size that
    is in the extension was in the co-signed tree.
    """
    problems = []
    idx = audit_indices(root, my_pk, size, k)
    extends = {}  # served tree_size -> consistency with (size, root) proven?
    for i in idx:
        try:
            e = get(f"{origin}/v1/log/entries?start={i}&limit=1")["entries"][0]
            inc = get(f"{origin}/v1/log/inclusion?leaf_index={i}")
        except Exception as ex:
            problems.append(f"leaf {i}: unreachable ({str(ex)[:40]})")
            continue
        if int(e.get("leaf_index", -1)) != i:
            problems.append(f"leaf {i}: entries route answered with leaf {e.get('leaf_index')}")
            continue
        try:
            t, root_t, rpk_t = verify_sth(inc["sth"])
        except (ValueError, KeyError) as ex:
            problems.append(f"leaf {i}: inclusion route's STH: {ex}")
            continue
        if rpk_t != rpk:
            problems.append(f"leaf {i}: inclusion route's STH is signed by a different key")
            continue
        if t < size:
            problems.append(f"leaf {i}: inclusion route's head {t} is behind the co-signed {size}")
            continue
        entry = blake3.blake3(b32d(e["attestation_cbor_b32"])).digest()
        if entry != b32d(e["entry_hash_b32"]):
            problems.append(f"leaf {i}: served bytes do not hash to the served entry hash")
            continue
        leaf = leaf_hash(entry)
        if leaf != b32d(inc["leaf_hash_b32"]):
            problems.append(f"leaf {i}: inclusion route's leaf hash is not this entry's")
            continue
        if not verify_inclusion(leaf, i, t, [b32d(x) for x in inc["audit_path_b32"]], root_t):
            problems.append(f"leaf {i}: inclusion proof does not reach the served root at {t}")
            continue
        if t not in extends:
            if t == size:
                extends[t] = root_t == root
            else:
                try:
                    c = get(f"{origin}/v1/log/consistency?first={size}&second={t}")
                    extends[t] = (c.get("first_root_b32", "").lower() == b32e(root).lower()
                                  and b32d(c["second_root_b32"]) == root_t
                                  and verify_consistency(size, t, root, root_t,
                                                         [b32d(x) for x in c["consistency_proof_b32"]]))
                except Exception as ex:
                    extends[t] = False
        if not extends[t]:
            problems.append(f"leaf {i}: served head {t} is not an extension of the co-signed head {size}")
    return len(idx), problems


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
        # Rule 1: verify before anything.
        try:
            size, root, rpk = verify_sth(sth)
        except (ValueError, KeyError):
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
        # Spot-check custody. A co-signature says the head is consistent with
        # what this witness saw before; it says nothing about whether the
        # peer still holds the bytes under it. Kept separate on purpose.
        k = int(os.environ.get("EMEM_WITNESS_SAMPLES", "4"))
        if k > 0:
            checked, problems = audit_custody(origin, size, root, rpk, my_pk, k)
            if problems:
                print(f"  {origin}: CUSTODY FAILURE on {len(problems)} of {checked} sampled leaves:")
                for pr in problems:
                    print(f"    {pr}")
                failures.append(origin)
            else:
                print(f"    audited {checked} sampled leaves under root {b32e(root)[:8]}: all verify")
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
