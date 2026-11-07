"""Values for the film emem-note-write, computed offline. Nothing is sent to emem.dev.

The key is a throwaway Ed25519 key from a fixed seed, so the film is reproducible. Do not use it for a real write.
The digest rule is the v2 memory-write preimage: crates/emem-primitives (attester_preimage_v2), the same rule as
plugins/emem-signer/signer/emem_signer.py write_digest. The 401 and 200 shapes come from
crates/emem-api-rest/src/lib.rs (validate_attester_binding, attester_recipe, memory_create_inner).
usage: python3 note_write_data.py   (writes note-write-data.js next to this script)
"""
import base64
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "plugins/emem-signer/signer"))
from emem_crypto import blake3, ed25519_verify  # noqa: E402
from cryptography.hazmat.primitives import serialization  # noqa: E402
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey  # noqa: E402


def b32(b: bytes) -> str:
    return base64.b32encode(b).decode().rstrip("=").lower()


def digest_v2(verb: str, path: str, text: str, base: str) -> bytes:
    body_hash = blake3(text.encode()).digest()
    return blake3(b"emem.memory_write.v2|" + verb.encode() + b"|" + path.encode() + b"|" + body_hash + b"|"
                  + base.encode()).digest()


seed = blake3(b"emem docs film throwaway key, never used for a real write").digest()
sk = Ed25519PrivateKey.from_private_bytes(seed)
pk = sk.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
pubkey_b32 = b32(pk)
path = f"/memories/by_attester/{pubkey_b32[:8]}/notes.md"
text = "NDVI at the Ames field peaked at 0.88 on 1 August 2024."
base = "absent"
body_hash = blake3(text.encode()).digest()
digest = digest_v2("create", path, text, base)
sig = sk.sign(digest)
assert len(pubkey_b32) == 52 and len(b32(sig)) == 103
assert ed25519_verify(pk, sig, digest), "signature must verify over the digest"

# One changed character in the text: the server rebuilds the digest from the body it gets, so the old signature fails.
text2 = text.replace("0.88", "0.98")
digest2 = digest_v2("create", path, text2, base)
assert digest2 != digest and not ed25519_verify(pk, sig, digest2)

file_cid = b32(blake3(text.encode()).digest()[:16])
assert len(file_cid) == 26

out = "window.DATA = " + json.dumps({
    "pubkey_b32": pubkey_b32, "path": path, "text": text, "text2": text2, "base": base,
    "body_hash_hex": body_hash.hex(), "digest_hex": digest.hex(), "digest2_hex": digest2.hex(),
    "sig_b32": b32(sig), "file_cid": file_cid,
}, indent=1) + ";\n"
Path(__file__).with_name("note-write-data.js").write_text(out)
print(out, end="")
