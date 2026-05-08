#!/usr/bin/env python3
"""
Offline verifier for emem receipts.

Usage:
    verify.py <path_to_receipt.json>
    verify.py -                  # read receipt JSON from stdin

Exits 0 with "VALID" + diagnostic lines on success, exits 1 with
"INVALID" on failure. The math here matches
crates/emem-storage/src/server.rs:132-148 in the emem source —
if your verification passes, the receipt has not been tampered with
since the responder signed it.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

try:
    from blake3 import blake3
except ImportError:
    sys.stderr.write("missing dep: pip install blake3\n")
    sys.exit(2)

try:
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
    from cryptography.exceptions import InvalidSignature
except ImportError:
    sys.stderr.write("missing dep: pip install cryptography\n")
    sys.exit(2)


B32_ALPHA = "abcdefghijklmnopqrstuvwxyz234567"


def b32_nopad_decode(s: str) -> bytes:
    """base32-nopad-lowercase → bytes. Matches emem-codec::cid64::b32_decode."""
    s = s.lower().strip()
    if not all(c in B32_ALPHA for c in s):
        raise ValueError(f"non-base32 character in {s!r}")
    bits = "".join(format(B32_ALPHA.index(c), "05b") for c in s)
    out = bytearray()
    for i in range(0, len(bits) - len(bits) % 8, 8):
        out.append(int(bits[i : i + 8], 2))
    return bytes(out)


def to_bytes(field, expected_len: int, name: str) -> bytes:
    """Coerce a list[int] (live JSON shape) or hex/base64 string to bytes."""
    if isinstance(field, (list, tuple)):
        b = bytes(field)
    elif isinstance(field, str):
        # try hex, then base64
        try:
            b = bytes.fromhex(field)
        except ValueError:
            import base64
            b = base64.b64decode(field)
    else:
        raise ValueError(f"{name}: unexpected type {type(field).__name__}")
    if len(b) != expected_len:
        raise ValueError(f"{name}: expected {expected_len} bytes, got {len(b)}")
    return b


def build_preimage(receipt: dict) -> bytes:
    """
    Mirror of crates/emem-storage/src/server.rs::sign_receipt preimage:

        request_id | served_at | primitive |
        cell_0 , cell_1 , … cell_N , |
        fact_cid_0 , fact_cid_1 , … fact_cid_M ,

    Pipes between sections, comma after EVERY list element.
    """
    parts: list[bytes] = []
    parts.append(receipt.get("request_id", "").encode("utf-8"))
    parts.append(b"|")
    parts.append(receipt.get("served_at", "").encode("utf-8"))
    parts.append(b"|")
    parts.append(receipt.get("primitive", "").encode("utf-8"))
    parts.append(b"|")
    for cell in receipt.get("cells", []):
        parts.append(cell.encode("utf-8"))
        parts.append(b",")
    parts.append(b"|")
    for cid in receipt.get("fact_cids", []):
        parts.append(cid.encode("utf-8"))
        parts.append(b",")
    return b"".join(parts)


def main() -> int:
    if len(sys.argv) != 2:
        sys.stderr.write(__doc__)
        return 2
    arg = sys.argv[1]
    if arg == "-":
        raw = sys.stdin.read()
    else:
        raw = Path(arg).read_text()
    receipt = json.loads(raw)
    # Some endpoints return {receipt: {...}, facts: [...]}; unwrap.
    if "receipt" in receipt and "request_id" not in receipt:
        receipt = receipt["receipt"]

    # Build preimage and digest.
    preimage = build_preimage(receipt)
    digest = blake3(preimage).digest()

    # Signature → 64 bytes, pubkey → 32 bytes.
    sig = to_bytes(receipt["signature"], 64, "signature")
    if "responder" in receipt and isinstance(receipt["responder"], (list, tuple)):
        pubkey_bytes = bytes(receipt["responder"])
    elif receipt.get("responder_pubkey_b32"):
        pubkey_bytes = b32_nopad_decode(receipt["responder_pubkey_b32"])
    else:
        sys.stderr.write("INVALID\nreceipt has no responder pubkey field\n")
        return 1
    if len(pubkey_bytes) != 32:
        sys.stderr.write(f"INVALID\npubkey length {len(pubkey_bytes)} != 32\n")
        return 1

    # Verify Ed25519.
    try:
        Ed25519PublicKey.from_public_bytes(pubkey_bytes).verify(sig, digest)
    except InvalidSignature:
        print("INVALID")
        print(f"preimage_len: {len(preimage)} bytes")
        print(f"digest:       {digest.hex()}")
        print(f"signature:    {sig.hex()}")
        print(f"signer:       {pubkey_bytes.hex()}")
        return 1

    print("VALID")
    print(f"preimage_len: {len(preimage)} bytes")
    print(f"digest:       {digest.hex()}")
    print(f"signer:       {pubkey_bytes.hex()}")
    print(f"primitive:    {receipt.get('primitive')}")
    print(f"cells:        {len(receipt.get('cells', []))}")
    print(f"fact_cids:    {len(receipt.get('fact_cids', []))}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
