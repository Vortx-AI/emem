#!/usr/bin/env python3
"""Verify emem OCR and document-parse responses offline.

    verify_doc.py RESPONSE.json [DOCUMENT] [--pubkey KEY] [--require-ocr]
    verify_doc.py --self-test               # check the bundled crypto

--pubkey KEY   the key the receipts must be signed by (base32). Without it, emem.dev's published key. A receipt
               names its own signer, so checking against that name alone would pass for anyone's key.
--require-ocr  fail a parse that carries no OCR receipt. A parse of typed or pasted text is signed too, but
               nothing in it ties the text to a picture.

RESPONSE is the body of POST /v1/ocr (receipt domain emem.ocr.v1) or of
POST /v1/lab_report_parse / POST /v1/land_record_parse (emem.doc_parse.v1).
DOCUMENT is optional: the image you sent to /v1/ocr, or the text you sent to
a parser. Each check prints one line:

  image   blake3(image file) == image_blake3_b32          (OCR, with DOCUMENT)
  text    blake3(text) == text_blake3_b32                 (OCR: the returned
          text; parse: DOCUMENT when given)
  result  blake3(serialised `result`) == result_blake3_b32 (parse)
  sig     ed25519 over the domain's PreimageV1 verifies against the
          responder key
  ocr     for a parse that ran OCR first, the embedded OCR response is
          verified the same way and its text hash must equal the parse's

Exit 0 when every check that ran passed, 1 otherwise, 2 on bad input. No
network calls; BLAKE3 and Ed25519 come from emem_crypto.py beside this script, plain
Python.

Preimages, from crates/emem-api-rest/src/reader.rs:
  emem.ocr.v1        {1: image_blake3, 2: source, 3: lang, 4: engine,
                      5: text_blake3, 6: read_at, 7: responder_pubkey}
  emem.doc_parse.v1  {1: kind, 2: text_blake3, 3: result_blake3, 4: parser,
                      5: parsed_at, 6: responder_pubkey}
Hashes and the key enter as raw 32 bytes; every other field as UTF-8.
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


def b32d(s: str) -> bytes:
    s = s.strip().upper()
    return base64.b32decode(s + "=" * ((8 - len(s) % 8) % 8))


def b32e(b: bytes) -> str:
    return base64.b32encode(b).decode().rstrip("=").lower()


def preimage_v1(domain: str, segs) -> bytes:
    h = blake3()
    h.update(b"emem.preimage.v1\x00")
    d = domain.encode()
    h.update(struct.pack("<I", len(d)) + d)
    for tag, data in segs:
        h.update(bytes([tag]) + struct.pack("<I", len(data)) + data)
    return h.digest()


def check(label: str, ok: bool, detail: str) -> bool:
    print(f"{label:7} {'MATCH ' if ok else 'MISMATCH'}  {detail}")
    return ok


# emem.dev's responder key, as /.well-known/emem.json published it on 12 November 2026.
EMEM_DEV_PUBKEY = "777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka"
PINNED = [EMEM_DEV_PUBKEY]


def sig_ok(r: dict, segs, label: str = "sig") -> bool:
    rc = r["receipt"]
    if rc["responder_pubkey_b32"].strip().lower() != PINNED[0].strip().lower():
        print(f"{label:7} INVALID   signer={rc['responder_pubkey_b32']} is not the pinned key {PINNED[0]} "
              "(for another node, pass --pubkey <responder.pubkey_b32 from its /.well-known/emem.json>)")
        return False
    pk = b32d(rc["responder_pubkey_b32"])
    digest = preimage_v1(rc["domain"], segs + [(len(segs) + 1, pk)])
    if ed25519_verify(pk, b32d(rc["signature_b32"]), digest):
        print(f"{label:7} VALID     domain={rc['domain']} signer={rc['responder_pubkey_b32']}")
        return True
    print(f"{label:7} INVALID   domain={rc['domain']}")
    return False


def verify_ocr(r: dict, image: bytes | None, label: str = "sig") -> bool:
    ok = True
    if image is not None:
        got = b32e(blake3(image).digest())
        ok &= check("image", got == r["image_blake3_b32"], got)
    got = b32e(blake3(r["text"].encode()).digest())
    ok &= check("text", got == r["text_blake3_b32"], got)
    ok &= sig_ok(r, [
        (1, b32d(r["image_blake3_b32"])),
        (2, r["source"].encode()),
        (3, r["lang"].encode()),
        (4, r["engine"].encode()),
        (5, b32d(r["text_blake3_b32"])),
        (6, r["read_at"].encode()),
    ], label)
    return ok


def verify_parse(r: dict, text: bytes | None) -> bool:
    ok = True
    if text is not None:
        got = b32e(blake3(text).digest())
        ok &= check("text", got == r["text_blake3_b32"], got)
    # serde_json renders `result` compactly with key-sorted maps; Python's
    # compact sorted dump gives the same bytes for what these parsers emit.
    body = json.dumps(r["result"], sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    got = b32e(blake3(body.encode()).digest())
    ok &= check("result", got == r["result_blake3_b32"], got)
    ok &= sig_ok(r, [
        (1, r["signal"].encode()),
        (2, b32d(r["text_blake3_b32"])),
        (3, b32d(r["result_blake3_b32"])),
        (4, r["parser"].encode()),
        (5, r["parsed_at"].encode()),
    ])
    if isinstance(r.get("ocr"), dict):
        ok &= verify_ocr(r["ocr"], None, "ocr")
        ok &= check("ocr", r["ocr"]["text_blake3_b32"] == r["text_blake3_b32"],
                    "the parsed text is the text the OCR receipt signs")
    else:
        print("source  NO OCR    the text was typed or pasted: no picture was read, so nothing ties it to a document image")
        if REQUIRE_OCR[0]:
            ok = False
    return ok


REQUIRE_OCR = [False]


def main() -> int:
    argv = sys.argv[1:]
    if argv == ["--self-test"]:
        return 0 if self_test() else 1
    if "--require-ocr" in argv:
        REQUIRE_OCR[0] = True
        argv.remove("--require-ocr")
    if "--pubkey" in argv:
        i = argv.index("--pubkey")
        if i + 1 >= len(argv):
            sys.stderr.write("--pubkey needs a key\n")
            return 2
        PINNED[0] = argv[i + 1]
        argv = argv[:i] + argv[i + 2:]
    if len(argv) not in (1, 2):
        sys.stderr.write(__doc__)
        return 2
    try:
        raw = sys.stdin.read() if argv[0] == "-" else Path(argv[0]).read_text()
        r = json.loads(raw)
    except (OSError, json.JSONDecodeError) as e:
        sys.stderr.write(f"cannot read the response: {e}\n")
        return 2
    if r.get("schema") == "emem.error.v1" or "receipt" not in r:
        sys.stderr.write(f"not a signed document response: {r.get('code')}: {r.get('message')}\n")
        return 2
    doc = Path(argv[1]).read_bytes() if len(argv) == 2 else None
    domain = r["receipt"].get("domain")
    if domain == "emem.ocr.v1":
        ok = verify_ocr(r, doc)
    elif domain == "emem.doc_parse.v1":
        ok = verify_parse(r, doc)
    else:
        sys.stderr.write(f"unknown receipt domain {domain!r}\n")
        return 2
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
