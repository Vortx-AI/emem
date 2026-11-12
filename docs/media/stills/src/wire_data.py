"""Decode the three ids of one live fact (cell64, tslot, fact_cid) and check each part against the response.

Reads, saved from emem.dev on 7 November 2026 at 15:40 UTC (read-only):
  curl -H 'Accept: application/cbor' https://emem.dev/v1/facts/<CID> -o data/wire/fact.cbor
  curl https://emem.dev/v1/facts/<CID>                               -o data/wire/fact.json
  POST /v1/locate {"place":"Mount Fuji"}                             -o data/wire/locate.json
The rules follow docs/protocol.md sections 1 to 3 and the codec (geo.rs, cell64.rs, alphabet.rs,
tslot_text.rs). The script stops if a decoded part does not agree with the live response.
usage: python3 wire_data.py   (prints JSON; wire_forms.py imports decode())
"""
import base64
import datetime as dt
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
sys.path.insert(0, str(ROOT / "tools/verifiers/emem-field-tokens"))
from emem_crypto import blake3  # noqa: E402

D = HERE / "data" / "wire"
CID = "banvya7hgenuowjggkgguezvltxtefp6jgihocwkgcbddyvqv6bq"

CONS, VOWS = "bcdfghjklmnpqrstvwxyz", "aeiouAEIOU"
ALPHA = [c1 + v1 + c2 + v2 for c1 in CONS for v1 in VOWS for c2 in CONS for v2 in VOWS]
ALPHA += [f"z{i:04x}" for i in range(len(ALPHA), 65536)]
INDEX = {s: i for i, s in enumerate(ALPHA)}
assert len(ALPHA) == 65536 and len(INDEX) == 65536
assert ".".join(ALPHA[d] for d in (4432, 44800, 32, 0)) == "dedi.zaf00.bafi.baba"  # protocol.md 1.4

LAT_MAX, LNG_MAX = (1 << 21) - 1, (1 << 22) - 1


def to_tslot_text(n):
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        out.append(b | (0x80 if n else 0))
        if not n:
            break
    return bytes(out), "t." + base64.b32encode(bytes(out)).decode().rstrip("=").lower()


for n, want in ((0, "t.aa"), (1, "t.ae"), (26, "t.di"), (1024, "t.qaea")):  # tslot_text.rs tests
    assert to_tslot_text(n)[1] == want


def decode():
    fact = json.loads((D / "fact.json").read_text())
    loc = json.loads((D / "locate.json").read_text())
    raw = (D / "fact.cbor").read_bytes()

    cell = fact["cell"]
    assert cell == loc["cell64"]
    lanes = [INDEX[p] for p in cell.split(".")]
    word = 0
    for d in lanes:
        word = (word << 16) | d
    mode, res, base, rsv = word >> 60, (word >> 52) & 0xFF, (word >> 44) & 0xFF, (word >> 43) & 1
    lat_q, lng_q = (word >> 22) & LAT_MAX, word & LNG_MAX
    assert (mode, res, base, rsv) == (1, 21, 0xAB, 0)
    lat = lat_q / LAT_MAX * 180 - 90
    lng = lng_q / LNG_MAX * 360 - 180
    c = loc["centre"]
    assert abs(lat - c["lat_deg"]) < 1e-9 and abs(lng - c["lng_deg"]) < 1e-9, (lat, lng, c)

    tslot = fact["tslot"]
    start = dt.datetime.fromtimestamp(tslot * 86_400, dt.timezone.utc)
    obs = dt.datetime.fromisoformat(fact["sources"][0]["captured_at"].replace("Z", "+00:00"))
    assert start <= obs < start + dt.timedelta(days=1)
    varint, ttext = to_tslot_text(tslot)

    digest = blake3(raw).digest()
    assert base64.b32encode(digest).decode().rstrip("=").lower() == CID
    assert cell.encode() in raw

    return {
        "cell": cell,
        "lanes": [[p, d] for p, d in zip(cell.split("."), lanes)],
        "word_hex": f"{word:016x}",
        "fields": {"mode": mode, "resolution": res, "base": f"0x{base:02x}", "reserved": rsv,
                   "lat_q": lat_q, "lng_q": lng_q},
        "lat": lat, "lng": lng,
        "locate_centre": [c["lat_deg"], c["lng_deg"]],
        "band": fact["band"], "value": fact["value"],
        "tslot": tslot, "varint_hex": varint.hex(" "), "tslot_text": ttext,
        "slot_start": start.strftime("%-d %B %Y, %H:%M UTC"),
        "observed": obs.strftime("%-d %B %Y, %H:%M UTC"),
        "cid": CID, "cbor_bytes": len(raw), "digest_hex": digest.hex(),
    }


if __name__ == "__main__":
    print(json.dumps(decode(), indent=1))
