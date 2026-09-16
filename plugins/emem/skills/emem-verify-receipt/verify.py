#!/usr/bin/env python3
"""
Offline verifier for emem receipts (preimage v1).

Usage:
    verify.py <path_to_receipt.json>
    verify.py -                  # read receipt JSON from stdin

Exits 0 with "VALID" + diagnostic lines on success, 1 with "INVALID" on a
signature mismatch, 3 when the receipt carries a segment this script does
not rebuild (use POST /v1/verify_receipt for those).

The math matches emem-attest's `receipt_preimage_v1`
(crates/emem-attest/src/lib.rs): the single source of truth the signer
(emem-storage) and every verifier (POST /v1/verify_receipt, the /verify
page's JS) all call. If this passes, the receipt was signed by the
responder pubkey and has not been tampered with since.

Preimage v1 is domain-separated and length-prefixed: the whole stream is
`blake3("emem.preimage.v1\\0" || u32le(len(domain)) || domain || segments)`,
and ed25519 signs the resulting 32-byte digest. Each segment is
`tag || u32le(len) || bytes`; a list segment is
`tag || u32le(count) || (u32le(len) || bytes)*`. Segments, in order:
REQUEST_ID 1, SERVED_AT 2, SCOPE 3, AS_OF 4, EDGES 5, MANIFEST 6,
PRIMITIVE 7, CELLS 8, FACT_CIDS 9, FIELD 10. Optional segments are
emitted only when present, so a plain recall receipt (manifest only) and
a field-token receipt (manifest + field) both rebuild exactly.
"""
from __future__ import annotations

import json
import struct
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
    """base32-nopad-lowercase -> bytes. Matches emem-codec::cid64::b32_decode."""
    s = s.lower().strip()
    if not all(c in B32_ALPHA for c in s):
        raise ValueError(f"non-base32 character in {s!r}")
    bits = "".join(format(B32_ALPHA.index(c), "05b") for c in s)
    out = bytearray()
    for i in range(0, len(bits) - len(bits) % 8, 8):
        out.append(int(bits[i : i + 8], 2))
    return bytes(out)


def _u32(n: int) -> bytes:
    return struct.pack("<I", n)


def _seg(tag: int, b: bytes) -> bytes:
    return bytes([tag]) + _u32(len(b)) + b


def _seg_list(tag: int, items) -> bytes:
    body = b""
    count = 0
    for it in items:
        bb = it.encode("utf-8")
        body += _u32(len(bb)) + bb
        count += 1
    return bytes([tag]) + _u32(count) + body


def _cbor_text(s: str) -> bytes:
    """CBOR definite-length text string, matching ciborium."""
    b = s.encode("utf-8")
    n = len(b)
    if n < 24:
        hdr = bytes([0x60 | n])
    elif n < 256:
        hdr = bytes([0x78, n])
    elif n < 65536:
        hdr = bytes([0x79]) + struct.pack(">H", n)
    else:
        hdr = bytes([0x7A]) + struct.pack(">I", n)
    return hdr + b


def _cbor_str_map(d: dict) -> bytes:
    """CBOR map of text->text with BTreeMap (sorted-key) ordering, matching
    the signer's `ciborium::into_writer(&BTreeMap<String,String>)`."""
    items = sorted(d.items())
    n = len(items)
    if n < 24:
        out = bytes([0xA0 | n])
    elif n < 256:
        out = bytes([0xB8, n])
    else:
        out = bytes([0xB9]) + struct.pack(">H", n)
    for k, v in items:
        out += _cbor_text(str(k)) + _cbor_text(str(v))
    return out


def _manifest_hex(source_versions: dict):
    if not source_versions:
        return None
    return blake3(_cbor_str_map(source_versions)).hexdigest()


def _field_hex(field: dict) -> str:
    """field_binding_v1: PreimageV1("field").seg(1, aoi_cid).seg(2, derivation_cid)."""
    h = blake3()
    h.update(b"emem.preimage.v1\x00")
    dom = b"field"
    h.update(_u32(len(dom)))
    h.update(dom)
    h.update(_seg(0x01, str(field["aoi_cid"]).encode("utf-8")))
    h.update(_seg(0x02, str(field["derivation_cid"]).encode("utf-8")))
    return h.hexdigest()


def build_preimage_v1(receipt: dict) -> bytes:
    """Rebuild the v1 preimage stream (the bytes blake3 then hashes)."""
    # Segments this script does not rebuild. If a receipt carries one, its
    # digest would silently mismatch, so refuse with a clear pointer instead.
    if receipt.get("scope"):
        raise Unsupported("scope")
    as_of = receipt.get("as_of")
    if isinstance(as_of, dict) and any(as_of.get(k) for k in ("valid_time", "transaction_time")):
        raise Unsupported("as_of")
    if receipt.get("edge_cids") or receipt.get("edges"):
        raise Unsupported("edges")

    out = b"emem.preimage.v1\x00"
    dom = b"receipt"
    out += _u32(len(dom)) + dom
    out += _seg(0x01, receipt.get("request_id", "").encode("utf-8"))
    out += _seg(0x02, receipt.get("served_at", "").encode("utf-8"))
    mh = _manifest_hex(receipt.get("source_versions") or {})
    if mh:
        out += _seg(0x06, mh.encode("utf-8"))
    out += _seg(0x07, receipt.get("primitive", "").encode("utf-8"))
    out += _seg_list(0x08, receipt.get("cells", []))
    out += _seg_list(0x09, receipt.get("fact_cids", []))
    field = receipt.get("field")
    if isinstance(field, dict) and field.get("aoi_cid") and field.get("derivation_cid"):
        out += _seg(0x0A, _field_hex(field).encode("utf-8"))
    return out


class Unsupported(Exception):
    pass


def _pubkey_bytes(receipt: dict) -> bytes:
    r = receipt.get("responder")
    if isinstance(r, (list, tuple)):
        return bytes(r)
    b32 = receipt.get("responder_pubkey_b32")
    if b32:
        return b32_nopad_decode(b32)
    raise ValueError("receipt has no responder pubkey field")


def _sig_bytes(receipt: dict) -> bytes:
    s = receipt.get("signature")
    if isinstance(s, (list, tuple)):
        return bytes(s)
    if isinstance(s, str):
        try:
            return bytes.fromhex(s)
        except ValueError:
            return b32_nopad_decode(s)
    raise ValueError("receipt has no signature")


def main() -> int:
    if len(sys.argv) != 2:
        sys.stderr.write(__doc__)
        return 2
    raw = sys.stdin.read() if sys.argv[1] == "-" else Path(sys.argv[1]).read_text()
    receipt = json.loads(raw)
    if isinstance(receipt, dict) and "receipt" in receipt and "request_id" not in receipt:
        receipt = receipt["receipt"]

    pv = receipt.get("preimage_version")
    if pv not in (1, None):
        sys.stderr.write(f"unknown preimage_version {pv!r}; upgrade this script or use /v1/verify_receipt\n")
        return 3
    if pv is None:
        sys.stderr.write(
            "this receipt predates preimage v1 (no preimage_version field). This script "
            "verifies v1 receipts; for a legacy receipt use POST /v1/verify_receipt.\n"
        )
        return 3

    try:
        preimage = build_preimage_v1(receipt)
    except Unsupported as u:
        sys.stderr.write(
            f"this receipt carries a `{u}` segment this offline script does not rebuild. "
            f"Verify it server-side: POST /v1/verify_receipt with {{\"receipt\": <receipt>}}.\n"
        )
        return 3

    digest = blake3(preimage).digest()  # the 32-byte message ed25519 signed
    try:
        sig = _sig_bytes(receipt)
        pubkey = _pubkey_bytes(receipt)
    except ValueError as e:
        sys.stderr.write(f"INVALID\n{e}\n")
        return 1
    if len(pubkey) != 32 or len(sig) != 64:
        sys.stderr.write(f"INVALID\nbad lengths: pubkey {len(pubkey)} (want 32), sig {len(sig)} (want 64)\n")
        return 1

    try:
        Ed25519PublicKey.from_public_bytes(pubkey).verify(sig, digest)
    except InvalidSignature:
        print("INVALID")
        print(f"preimage_len: {len(preimage)} bytes")
        print(f"digest:       {digest.hex()}")
        print(f"signer:       {pubkey.hex()}")
        return 1

    print("VALID")
    print(f"preimage_v1:  {len(preimage)} bytes")
    print(f"digest:       {digest.hex()}")
    print(f"signer:       {pubkey.hex()}")
    print(f"primitive:    {receipt.get('primitive')}")
    print(f"cells:        {len(receipt.get('cells', []))}")
    print(f"fact_cids:    {len(receipt.get('fact_cids', []))}")
    if receipt.get("field"):
        print(f"field_bound:  aoi={receipt['field'].get('aoi_cid','')[:12]}… derivation={receipt['field'].get('derivation_cid','')[:12]}…")
    return 0


if __name__ == "__main__":
    sys.exit(main())
