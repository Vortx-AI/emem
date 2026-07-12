#!/usr/bin/env python3
"""Two robots from two vendors share one verifiable map.

Runnable against the hosted node with no key and no account:

    pip install requests
    python3 fleet_memory.py

Optional full offline verification (no trust in the responder at all):

    pip install blake3 cbor2 cryptography

Robot A (vendor A) names a landmark, reads the terrain around it, and
hands over three short lines: one entity token and two fact tokens.
Robot B (vendor B) converges on the same landmark from its own phrasing,
resolves the same signed bytes, and checks the signature itself. At no
point does B trust A, A's vendor, or a shared backend.
"""
import requests

BASE = "https://emem.dev"
LABEL = "Maasvlakte ramp 7"
ALIAS = "ramp seven at the Maasvlakte terminal"
PLACE = "Maasvlakte, Rotterdam"
BANDS = ["copdem30m.elevation_mean", "surface_water.recurrence"]


def post(path, body):
    r = requests.post(f"{BASE}{path}", json=body, timeout=120)
    r.raise_for_status()
    return r.json()


def robot_a():
    """Vendor A's robot: name the landmark, read the ground, hand off."""
    print("== robot A (vendor A) ==")

    # The convergence discipline: resolve first, mint only on a miss.
    # If any robot in any fleet already named this object, reuse its
    # canonical id instead of inventing a second one.
    res = post("/v1/entity/resolve", {"text": LABEL})
    if res["count"]:
        entity_token = res["candidates"][0]["entity_token"]
        print(f"landmark identity : {entity_token} (already named, reused)")
    else:
        ent = post("/v1/entity", {"label": LABEL, "place": PLACE})
        entity_token = ent["entity_token"]
        print(f"landmark identity : {entity_token} (minted)")

    # Record the phrasing A's own crews use, signed, so other fleets
    # converge on the canonical id from their words, not A's.
    post("/v1/entity/alias", {"entity_token": entity_token, "alias": ALIAS})
    print(f"alias linked      : {ALIAS!r}")

    # Read the ground at the landmark. ensure, not get: whatever is
    # missing is fetched from the named source, signed, and stored for
    # every fleet; whatever exists is reused.
    cell = post("/v1/locate", {"q": PLACE})["cell64"]
    rec = post("/v1/recall", {"cell": cell, "bands": BANDS})
    tokens = [f["memory_token"] for f in rec["facts"]]
    for f in rec["facts"]:
        prov = f.get("band_metadata", {}).get("provenance", {})
        print(f"observed          : {f['band']} = {f.get('value')} "
              f"({prov.get('class')}, deterministic={prov.get('deterministic')})")

    # The handoff. This is everything that crosses the vendor boundary.
    return [entity_token, *tokens], rec["receipt"]


def robot_b(handoff, receipt):
    """Vendor B's robot: converge, resolve, verify. Trust nothing."""
    print("\n== robot B (vendor B) ==")
    entity_token, *fact_tokens = handoff

    # B uses ITS phrasing, not A's label, and still lands on the same id.
    res = post("/v1/entity/resolve", {"text": ALIAS})
    got = res["candidates"][0]["entity_token"]
    assert got == entity_token, "different fleets diverged on the landmark id"
    print(f"converged on      : {got} (from {ALIAS!r})")

    # Each fact token resolves to the byte-identical signed observation.
    for t in fact_tokens:
        r = post("/v1/memory_token/resolve", {"token": t})
        prov = r.get("provenance", {})
        print(f"resolved          : {t[:44]}... -> "
              f"{r['fact'].get('band')} (class={prov.get('class')})")

    # Verify the receipt. Server-side check first; full offline check
    # (blake3 preimage v1 + ed25519, no network trust) when the crypto
    # libraries are installed.
    v = post("/v1/verify_receipt", {"receipt": receipt})
    print(f"receipt valid     : {v.get('valid')}")
    try:
        offline_verify(receipt)
        print("offline verify    : VALID (recomputed locally, no trust in the responder)")
    except ImportError:
        print("offline verify    : skipped (pip install blake3 cbor2 cryptography to run it)")


def offline_verify(r):
    """Recompute the v1 preimage and check ed25519, entirely locally."""
    import base64

    import cbor2
    from blake3 import blake3
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

    h = blake3()
    h.update(b"emem.preimage.v1\x00")
    h.update(len(b"receipt").to_bytes(4, "little"))
    h.update(b"receipt")

    def seg(tag, data):
        h.update(bytes([tag]))
        h.update(len(data).to_bytes(4, "little"))
        h.update(data)

    def seg_list(tag, items):
        h.update(bytes([tag]))
        body = b"".join(len(i.encode()).to_bytes(4, "little") + i.encode() for i in items)
        h.update(len(items).to_bytes(4, "little"))
        h.update(body)

    seg(0x01, r["request_id"].encode())
    seg(0x02, r["served_at"].encode())
    if r.get("source_versions"):
        mh = blake3(cbor2.dumps(dict(sorted(r["source_versions"].items())))).hexdigest()
        seg(0x06, mh.encode())
    seg(0x07, r["primitive"].encode())
    seg_list(0x08, r["cells"])
    seg_list(0x09, r["fact_cids"])

    s = r["responder_pubkey_b32"].upper()
    pk = base64.b32decode(s + "=" * ((-len(s)) % 8))
    Ed25519PublicKey.from_public_bytes(pk).verify(bytes(r["signature"]), h.digest())


if __name__ == "__main__":
    handoff, receipt = robot_a()
    print("\n-- handoff across the vendor boundary: "
          f"{len(handoff)} lines, {sum(len(t) for t in handoff)} characters --")
    robot_b(handoff, receipt)
    print("\nDone. The map entry outlives both robots, both sessions, and "
          "both vendors; any third fleet can repeat robot B's half verbatim.")
