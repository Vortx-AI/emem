"""Values for the film emem-one-byte, from the saved bytes of one live fact.

Reads, saved from emem.dev on 7 November 2026 at 15:06 UTC (read-only):
  curl -H 'Accept: application/cbor' https://emem.dev/v1/facts/<CID> > data/one-byte/fact-live.cbor
  curl https://emem.dev/v1/facts/<changed id>                       > data/one-byte/changed-id.json   (404)
  POST /v1/memory_token/resolve on the token                       > data/one-byte/token-resolve.json
The script hashes the bytes with the plugin's pure-Python BLAKE3 and stops if a value does not agree.
usage: python3 one_byte_data.py   (writes one-byte-data.js next to this script)
"""
import base64
import json
import struct
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
sys.path.insert(0, str(ROOT / "plugins/emem/skills/emem-field-tokens/scripts"))
from emem_crypto import blake3  # noqa: E402

CID = "qejpz7cgvk347ordd6vrdtmwzprqlzcnkmotsjton2x4y2jjbplq"
CELL = "defi.zb592.nUkO.zEzE"
BAND = "copdem30m.elevation_mean"
D = HERE / "data" / "one-byte"


def cid_of(b):
    return base64.b32encode(blake3(b).digest()).decode().lower().rstrip("=")


raw = (D / "fact-live.cbor").read_bytes()
assert raw == (ROOT / "docs/media/diagrams/src/data/fuji-fact.cbor").read_bytes()
assert cid_of(raw) == CID

key = b"evalue\xfa"
at = raw.index(key) + len(key)
value = struct.unpack(">f", raw[at:at + 4])[0]
resolved = json.loads((D / "token-resolve.json").read_text())
assert resolved["fact_cid"] == CID and resolved["value"] == value and resolved["band"] == BAND

bad = bytearray(raw)
bad[at + 3] ^= 0x01
bad_value = struct.unpack(">f", bytes(bad[at:at + 4]))[0]
bad_cid = cid_of(bytes(bad))
assert f"{value:.2f}" == f"{bad_value:.2f}" == "3539.04"

miss = json.loads((D / "changed-id.json").read_text())
assert miss["code"] == "cid_not_found" and bad_cid in miss["message"]

same = sum(a == b for a, b in zip(CID, bad_cid))


def span(tag):
    i = raw.index(tag.encode())
    return [i, i + len(tag)]


data = {
    "n": len(raw),
    "cid": CID,
    "bad_cid": bad_cid,
    "same_chars": same,
    "value": f"{value:.4f}",
    "bad_value": f"{bad_value:.4f}",
    "value_at": at,
    "flip_at": at + 3,
    "hex": raw[at:at + 4].hex(" "),
    "bad_hex": bytes(bad[at:at + 4]).hex(" "),
    "band": span(BAND),
    "cell": span(CELL),
}
(HERE / "one-byte-data.js").write_text("window.D = " + json.dumps(data, indent=1) + ";\n")
print(json.dumps(data, indent=1))
