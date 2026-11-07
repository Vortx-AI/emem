"""Rebuild the signed preimage of one live recall receipt, segment by segment, and check it offline.

Reads, saved from emem.dev on 7 November 2026 at 15:44 UTC by fetch_receipt.py (read-only):
  POST /v1/recall {"cell":"defi.zb592.nUkO.zEzE","bands":["copdem30m.elevation_mean"]}
  POST /v1/verify_receipt on that receipt, and GET /.well-known/emem.json
The segment rules are docs/protocol.md section 7.1; the helpers come from the plugin's verify.py.
The script stops if the offline digest or signature check does not agree with the live response.
usage: python3 receipt_data.py   (prints JSON; receipt_preimage.py imports build())
"""
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
sys.path.insert(0, str(ROOT / "plugins/emem/skills/emem-verify-receipt/scripts"))
import verify as V  # noqa: E402
from emem_crypto import blake3, ed25519_verify  # noqa: E402

D = HERE / "data" / "receipt"


def build():
    rec = json.loads((D / "recall.json").read_text())
    live = json.loads((D / "verify.json").read_text())
    wk = json.loads((D / "well_known.json").read_text())
    r = rec["receipt"]
    fact = rec["facts"][0]
    assert r["preimage_version"] == 2 and live["valid"] is True

    mh = V._manifest_hex(r["source_versions"])
    kh = V._merkle_hex(r["merkle_proof"])
    rows = [  # (tag, name, kind, value shown, encoded segment)
        (0x01, "request_id", "text", r["request_id"], V._seg(0x01, r["request_id"].encode())),
        (0x02, "served_at", "text", r["served_at"], V._seg(0x02, r["served_at"].encode())),
        (0x06, "manifest_hex", "BLAKE3 of 4 manifest ids", mh, V._seg(0x06, mh.encode())),
        (0x07, "primitive", "text", r["primitive"], V._seg(0x07, r["primitive"].encode())),
        (0x08, "cells", "list of 1", r["cells"][0], V._seg_list(0x08, r["cells"])),
        (0x09, "fact_cids", "list of 1", r["fact_cids"][0], V._seg_list(0x09, r["fact_cids"])),
        (0x0B, "merkle_hex", "BLAKE3 of the inclusion proof", kh, V._seg(0x0B, kh.encode())),
    ]
    head = b"emem.preimage.v1\x00" + V._u32(7) + b"receipt"
    stream = head + b"".join(s for *_, s in rows)
    assert stream == V.build_preimage(r, 2)
    digest = blake3(stream).digest()
    assert digest.hex() == live["preimage_blake3_hex"], "offline digest differs from the live one"

    pub = V.b32_nopad_decode(wk["responder"]["pubkey_b32"])
    assert pub == bytes(r["responder"])
    sig = bytes(r["signature"])
    assert ed25519_verify(pub, sig, digest)

    # Change one character of served_at (one second later) and check again.
    t = dict(r)
    t["served_at"] = r["served_at"][:-3] + f"{(int(r['served_at'][-3:-1]) + 1) % 60:02d}Z"
    assert len(t["served_at"]) == len(r["served_at"])
    t_digest = blake3(V.build_preimage(t, 2)).digest()
    assert t_digest != digest and not ed25519_verify(pub, sig, t_digest)

    return {
        "head_len": len(head),
        "rows": [{"tag": f"0x{t_:02x}", "name": n, "kind": k, "value": v, "len": len(s),
                  "hex": s[:5].hex()} for t_, n, k, v, s in rows],
        "absent": ["0x03 scope", "0x04 as_of", "0x05 edges", "0x0a field"],
        "stream_len": len(stream),
        "digest_hex": digest.hex(),
        "sig_b32": r["signature_b32"],
        "pub_b32": wk["responder"]["pubkey_b32"],
        "tampered_served_at": t["served_at"],
        "tampered_digest_hex": t_digest.hex(),
        "value": fact["value"], "band": fact["band"],
        "leaf_count": r["merkle_proof"]["leaf_count"],
        "live_valid": live["valid"], "live_merkle": live["merkle_proof_valid"],
    }


if __name__ == "__main__":
    print(json.dumps(build(), indent=1))
