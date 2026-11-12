#!/usr/bin/env python3
"""
Offline verifier for emem receipts (preimage v1 and v2).

Usage:
    verify.py <path_to_receipt.json> [--pubkey KEY] [--facts DIR]
    verify.py --self-test        # check the bundled crypto against test vectors

--pubkey KEY  the key the receipt must be signed by (base32). Without it the
              key is emem.dev's published key. A receipt names its own signer,
              so checking against that name alone proves nothing about WHO
              signed: anyone can sign with their own key and name it.
--facts DIR   check the values too. The signature covers the fact ids, not
              the values printed beside them. DIR holds <cid>.cbor files, each
              the body of GET /v1/facts/<cid> with `Accept: application/cbor`.
              Each must hash to its cid, and its cell, band, tslot, value and
              signed_at must equal the fact of that id in the saved response.

Accepts a bare receipt or a whole response with a top-level `receipt`.
Exits 0 with "VALID" + diagnostic lines on success, 1 with "INVALID" on a
signature mismatch, 3 when the receipt carries a segment this script does
not rebuild (use POST /v1/verify_receipt for those).

The math matches emem-attest's `receipt_preimage_v1` and
`receipt_preimage_v2` (crates/emem-attest/src/lib.rs): the single source
of truth the signer (emem-storage) and every verifier (POST
/v1/verify_receipt, the /verify page's JS) all call. If this passes, the
receipt was signed by the responder pubkey and has not been tampered with
since. BLAKE3 and Ed25519 come from emem_crypto.py beside this script, plain
Python with no third-party packages, so the script runs on a bare
interpreter and never touches the network.

The preimage is domain-separated and length-prefixed: the whole stream is
`blake3("emem.preimage.v1\\0" || u32le(len(domain)) || domain || segments)`,
and ed25519 signs the resulting 32-byte digest. Each segment is
`tag || u32le(len) || bytes`; a list segment is
`tag || u32le(count) || (u32le(len) || bytes)*`. Segments, in order:
REQUEST_ID 1, SERVED_AT 2, SCOPE 3, AS_OF 4, EDGES 5, MANIFEST 6,
PRIMITIVE 7, CELLS 8, FACT_CIDS 9, FIELD 10, and under v2 MERKLE 11.
Optional segments are emitted only when present. MERKLE is always
written under v2: it binds the receipt's `merkle_proof`, or an explicit
absence marker when there is none, so a stripped proof breaks the
signature instead of passing silently. Receipts served today carry
`preimage_version: 2`.
"""
from __future__ import annotations

import base64
import json
import struct
import sys
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))
try:
    from emem_crypto import blake3, ed25519_verify, self_test
except ImportError:
    sys.stderr.write("emem_crypto.py not found: keep it beside this script\n")
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


def _b32(b: bytes) -> str:
    return base64.b32encode(b).decode().rstrip("=").lower()


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


def _sub_preimage(domain: bytes, segs) -> bytes:
    h = blake3()
    h.update(b"emem.preimage.v1\x00")
    h.update(_u32(len(domain)))
    h.update(domain)
    for tag, data in segs:
        h.update(_seg(tag, data))
    return h.digest()


def _bytes32(v) -> bytes:
    if isinstance(v, (list, tuple)):
        return bytes(v)
    if isinstance(v, str):
        try:
            return bytes.fromhex(v)
        except ValueError:
            return b32_nopad_decode(v)
    raise ValueError(f"cannot read a 32-byte hash from {v!r}")


def _merkle_hex(proof) -> str:
    """merkle_binding_v2: PreimageV1("merkle"), either ROOT 1, LEAF_INDEX 2
    (u32le), PATH 3 (concatenated 32-byte siblings), RULE_VERSION 4, or the
    ABSENT 5 marker alone when the receipt carries no proof."""
    if not proof:
        return _sub_preimage(b"merkle", [(0x05, b"")]).hex()
    path = b"".join(_bytes32(p) for p in proof.get("path", []))
    return _sub_preimage(
        b"merkle",
        [
            (0x01, _bytes32(proof["root"])),
            (0x02, _u32(int(proof.get("leaf_index", 0)))),
            (0x03, path),
            (0x04, bytes([int(proof.get("version", 0))])),
        ],
    ).hex()


def build_preimage(receipt: dict, version: int) -> bytes:
    """Rebuild the v1 or v2 preimage stream (the bytes blake3 then hashes)."""
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
    if version >= 2:
        out += _seg(0x0B, _merkle_hex(receipt.get("merkle_proof")).encode("utf-8"))
    return out


class Unsupported(Exception):
    pass


# emem.dev's responder key, as /.well-known/emem.json published it on
# 12 November 2026 (responder.pubkey_b32).
EMEM_DEV_PUBKEY = "777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka"


def _cbor(b: bytes, i: int = 0):
    """A small CBOR decoder: enough for a fact's canonical bytes."""
    ib = b[i]
    mt, ai = ib >> 5, ib & 31
    i += 1
    if ai < 24:
        n = ai
    elif ai in (24, 25, 26, 27):
        w = 1 << (ai - 24)
        n = int.from_bytes(b[i:i + w], "big")
        i += w
    else:
        raise ValueError("indefinite-length CBOR is not canonical")
    if mt == 0:
        return n, i
    if mt == 1:
        return -1 - n, i
    if mt == 2:
        return b[i:i + n], i + n
    if mt == 3:
        return b[i:i + n].decode("utf-8"), i + n
    if mt in (4, 5):
        out = [] if mt == 4 else {}
        for _ in range(n):
            if mt == 4:
                v, i = _cbor(b, i)
                out.append(v)
            else:
                k, i = _cbor(b, i)
                v, i = _cbor(b, i)
                out[k] = v
        return out, i
    if mt == 6:
        return _cbor(b, i)
    if ai == 25:
        return struct.unpack(">e", n.to_bytes(2, "big"))[0], i
    if ai == 26:
        return struct.unpack(">f", n.to_bytes(4, "big"))[0], i
    if ai == 27:
        return struct.unpack(">d", n.to_bytes(8, "big"))[0], i
    return {20: False, 21: True, 22: None}.get(n), i


def _flat_fact(f):
    """A fact as {cell, band, tslot, value, signed_at}, whichever wrapping it has."""
    if isinstance(f, dict):
        for k in ("Primary", "Derivative", "Absence", "primary", "derivative", "absence"):
            if isinstance(f.get(k), dict):
                f = f[k]
                break
        return {k: f.get(k) for k in ("cell", "band", "tslot", "value", "signed_at")}
    return {}


def check_facts(response: dict, receipt: dict, folder: Path) -> bool:
    """The second claim: each fact id is the hash of the value printed beside it."""
    printed = {}
    for f in response.get("facts") or []:
        cid = (f.get("fact_cid") or f.get("cid")) if isinstance(f, dict) else None
        if cid:
            printed[cid] = _flat_fact(f)
    ok = True
    for cid in receipt.get("fact_cids", []):
        path = folder / f"{cid}.cbor"
        if not path.exists():
            print(f"fact {cid[:12]}… NOT CHECKED  no {path.name} in {folder}")
            ok = False
            continue
        body = path.read_bytes()
        if _b32(blake3(body).digest()) != cid:
            print(f"fact {cid[:12]}… INVALID      the saved bytes do not hash to this id")
            ok = False
            continue
        signed = _flat_fact(_cbor(body)[0])
        if cid not in printed:
            print(f"fact {cid[:12]}… BYTES MATCH  the response prints no fact with this id to compare")
            continue
        diff = [k for k in signed if printed[cid].get(k) is not None and printed[cid][k] != signed[k]]
        if diff:
            ok = False
            for k in diff:
                print(f"fact {cid[:12]}… INVALID      {k}: printed {printed[cid][k]!r}, signed {signed[k]!r}")
        else:
            print(f"fact {cid[:12]}… MATCH        {signed.get('band')} = {signed.get('value')!r} at tslot {signed.get('tslot')}")
    return ok


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
    args = sys.argv[1:]
    if args == ["--self-test"]:
        return 0 if self_test() else 1
    opts = {}
    while len(args) >= 2 and args[-2] in ("--pubkey", "--facts"):
        opts[args[-2]] = args[-1]
        args = args[:-2]
    if len(args) != 1:
        sys.stderr.write(__doc__)
        return 2
    want_key = opts.get("--pubkey", EMEM_DEV_PUBKEY)
    raw = sys.stdin.read() if args[0] == "-" else Path(args[0]).read_text()
    try:
        receipt = json.loads(raw)
    except json.JSONDecodeError:
        sys.stderr.write("input is not JSON (an empty body usually means the request itself failed)\n")
        return 2
    if isinstance(receipt, dict) and receipt.get("schema") == "emem.error.v1":
        sys.stderr.write(f"input is an error, not a receipt: {receipt.get('code')}: {receipt.get('message')}\n")
        return 2
    response = receipt if isinstance(receipt, dict) else {}
    if isinstance(receipt, dict) and isinstance(receipt.get("receipt"), dict) and "signature" not in receipt:
        receipt = receipt["receipt"]

    pv = receipt.get("preimage_version")
    if pv not in (1, 2, None):
        sys.stderr.write(f"unknown preimage_version {pv!r}; upgrade this script or use /v1/verify_receipt\n")
        return 3
    if pv is None:
        sys.stderr.write(
            "this receipt predates preimage v1 (no preimage_version field). This script "
            "verifies v1 and v2 receipts; for a legacy receipt use POST /v1/verify_receipt.\n"
        )
        return 3

    try:
        preimage = build_preimage(receipt, pv)
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

    if _b32(pubkey) != want_key.strip().lower():
        print("INVALID")
        print(f"signer:       {_b32(pubkey)}")
        print(f"pinned key:   {want_key}")
        print("The receipt names a different signer from the key you trust. For another node, or after a key")
        print("rotation, pass --pubkey <responder.pubkey_b32 from that node's /.well-known/emem.json>.")
        return 1
    if not ed25519_verify(pubkey, sig, digest):
        print("INVALID")
        print(f"preimage_len: {len(preimage)} bytes")
        print(f"digest:       {digest.hex()}")
        print(f"signer:       {_b32(pubkey)}")
        return 1

    print("VALID")
    print(f"preimage_v{pv}:  {len(preimage)} bytes")
    print(f"digest:       {digest.hex()}")
    print(f"signer:       {_b32(pubkey)}")
    print(f"primitive:    {receipt.get('primitive')}")
    print(f"cells:        {len(receipt.get('cells', []))}")
    print(f"fact_cids:    {len(receipt.get('fact_cids', []))}")
    if receipt.get("field"):
        print(f"field_bound:  aoi={receipt['field'].get('aoi_cid','')[:12]}… derivation={receipt['field'].get('derivation_cid','')[:12]}…")
    print("covers:       the request id, the time served, the primitive, the cells and the fact ids.")
    print("              Not the values printed beside them: check those with --facts.")
    if "--facts" in opts:
        if not check_facts(response, receipt, Path(opts["--facts"])):
            print("INVALID: the signature holds, but a printed value is not the signed one (see above)")
            return 1
        print("facts:        every fact id hashes to its bytes, and every printed value is the signed one")
    return 0


if __name__ == "__main__":
    sys.exit(main())
