"""Diagram: how one value becomes a fact, drawn from the real bytes of one fact.

Input, saved from emem.dev on 7 November 2026 at 12:17 UTC (a read):
  curl -H 'Accept: application/cbor' \
    https://emem.dev/v1/facts/qejpz7cgvk347ordd6vrdtmwzprqlzcnkmotsjton2x4y2jjbplq > data/fuji-fact.cbor
The script hashes those bytes again with the plugin's pure-Python BLAKE3 and stops if the id does not match.
It then changes the last byte of the value and hashes again, to show that the id changes.
usage: python3 data_flow.py OUT.svg
"""
import base64
import struct
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parents[3] / "tools/verifiers/emem-field-tokens"))
from emem_crypto import blake3  # noqa: E402

CID = "qejpz7cgvk347ordd6vrdtmwzprqlzcnkmotsjton2x4y2jjbplq"
COG = "Copernicus_DSM_COG_10_N35_00_E138_00_DEM.tif"
COG_BYTES = 43_412_544  # HEAD on the AWS object, 7 November 2026


def cid_of(b):
    return base64.b32encode(blake3(b).digest()).decode().lower().rstrip("=")


def short(c):
    return c[:8] + "…" + c[-7:]


raw = (HERE / "data" / "fuji-fact.cbor").read_bytes()
assert cid_of(raw) == CID, "the saved bytes do not hash to the fact id"
key = b"evalue\xfa"
at = raw.index(key) + len(key)
value = struct.unpack(">f", raw[at:at + 4])[0]
assert f"{value:.2f}" == "3539.04"
for tag in (b"copdem30m.elevation_mean", b"defi.zb592.nUkO.zEzE", b"copernicus_dem_30m_aws_pixel@1"):
    assert tag in raw
bad = bytearray(raw)
bad[at + 3] ^= 0x01
bad_value = struct.unpack(">f", bytes(bad[at:at + 4]))[0]
bad_cid = cid_of(bytes(bad))
assert bad_cid != CID
vhex = " ".join(f"{x:02x}" for x in raw[at:at + 4])
bhex = " ".join(f"{x:02x}" for x in bad[at:at + 4])
mb = COG_BYTES / 1e6

steps = [
    ("1", "Read", "HTTP Range", 112, [
        ("mono ex", COG[:30] + "…"),
        ("sans txt", f"An open file on AWS, {mb:.1f} MB. emem reads"),
        ("sans txt", "the first 64 KiB, then the one tile it needs."),
    ]),
    ("2", "Sample", "connector", 112, [
        ("mono ex", "copernicus_dem_30m_aws_pixel@1"),
        ("mono ex", "35.3628 N, 138.7308 E  →  3539.04 m"),
        ("sans txt", "No state. Same pixel in, same value out."),
    ]),
    ("3", "Encode", "canonical CBOR", 112, [
        ("mono ex", f"{len(raw)} bytes   …\"value\" {vhex}…"),
        ("sans txt", "The value is a 4-byte float. Fields keep"),
        ("sans txt", "their record order; keys are not sorted."),
    ]),
    ("4", "Hash", "BLAKE3, base32", 112, [
        ("mono tok", "fact_cid " + short(CID)),
        ("sans txt", "We hashed the 743 bytes again on our own"),
        ("sans txt", "machine. The id is the same."),
    ]),
    ("5", "Sign", "Ed25519", 0, [
        ("sans txt", "The responder signs a Merkle root over fact ids."),
        ("sans txt", "Each read gets a signed receipt that lists them."),
        ("mono tok", "emem:fact:<cell>:<fact_cid>"),
    ]),
]

out = []
y = 64
for i, (n, term, verb, h, lines) in enumerate(steps):
    if i:
        out.append(f'  <path class="flow" d="M48 {y - 26} V{y - 2}" marker-end="url(#a)"/>')
    h = 37 + sum(26 if c.startswith("sans") else 28 for c, _ in lines) + 22
    out.append(f'  <rect class="box" x="24" y="{y}" width="592" height="{h}" rx="8"/>')
    out.append(f'  <circle class="ring" cx="48" cy="{y + 30}" r="15"/>')
    out.append(f'  <text class="sans num" x="48" y="{y + 36}" text-anchor="middle">{n}</text>')
    out.append(f'  <text class="sans term" x="76" y="{y + 37}">{term}</text>')
    out.append(f'  <text class="mono verb" x="196" y="{y + 37}">{verb}</text>')
    ly = y + 37
    for cls, t in lines:
        ly += 26 if cls.startswith("sans") else 28
        t = t.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
        out.append(f'  <text class="{cls}" x="76" y="{ly}">{t}</text>')
    y += h + 26

tamper_y = y + 6
foot_y = tamper_y + 212
height = foot_y + 92
body = "\n".join(out)

desc = (
    "Five steps from top to bottom, drawn from the real bytes of one fact. "
    f"1 Read: emem sends HTTP Range requests to the open Copernicus DEM file {COG} on AWS ({mb:.1f} MB). "
    "It reads the first 64 KiB, then the one tile it needs. "
    "2 Sample: the connector copernicus_dem_30m_aws_pixel@1 turns 35.3628 N, 138.7308 E into 3539.04 m. "
    f"3 Encode: canonical CBOR, {len(raw)} bytes; the value is the 4-byte float {vhex}. "
    f"4 Hash: BLAKE3 in base32 gives fact_cid {short(CID)}. We hashed the bytes again and got the same id. "
    "5 Sign: the responder signs a Merkle root over fact ids with Ed25519, each read gets a signed receipt that lists them, and agents cite the fact as emem:fact:<cell>:<fact_cid>. "
    f"A red panel shows one changed byte: {vhex} becomes {bhex}, the value becomes {bad_value:.4f} m, "
    f"and the id becomes {short(bad_cid)}. The old id does not name the new bytes. "
    "Bytes from emem.dev on 7 November 2026."
)

desc = desc.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
svg = f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 {height}" width="640" height="{height}" role="img" aria-labelledby="t d">
  <title id="t">How one value becomes a fact</title>
  <desc id="d">{desc}</desc>
  <defs>
    <marker id="a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto">
      <path d="M0 0 L10 5 L0 10 z" fill="#8b949e"/>
    </marker>
  </defs>
  <style>
    .sans {{ font-family: "DejaVu Sans", system-ui, -apple-system, "Segoe UI", Helvetica, Arial, sans-serif; }}
    .mono {{ font-family: "DejaVu Sans Mono", ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }}
    .h {{ font-size: 21px; font-weight: 600; fill: #e6edf3; }}
    .term {{ font-size: 20px; font-weight: 600; fill: #e6edf3; }}
    .ex {{ font-size: 16px; fill: #79c0ff; }}
    .tok {{ font-size: 16px; fill: #ffa657; }}
    .bad {{ font-size: 16px; fill: #ff7b72; }}
    .ok {{ font-size: 18px; fill: #7ee787; }}
    .txt {{ font-size: 18px; fill: #8b949e; }}
    .verb {{ font-size: 16px; fill: #8b949e; }}
    .num {{ font-size: 17px; font-weight: 600; fill: #79c0ff; }}
    .box {{ fill: #161b22; stroke: #30363d; stroke-width: 1; }}
    .badbox {{ fill: #161b22; stroke: #ff7b72; stroke-width: 1.5; stroke-dasharray: 6 5; }}
    .ring {{ fill: #0d1117; stroke: #79c0ff; stroke-width: 1.5; }}
    .flow {{ stroke: #8b949e; stroke-width: 1.5; fill: none; }}
  </style>

  <rect x="0" y="0" width="640" height="{height}" rx="12" fill="#0d1117"/>
  <text class="sans h" x="24" y="40">How one value becomes a fact</text>

{body}

  <rect class="badbox" x="24" y="{tamper_y}" width="592" height="172" rx="8"/>
  <text class="sans term" x="48" y="{tamper_y + 36}" style="fill:#ff7b72">Change one byte</text>
  <text class="mono ex" x="48" y="{tamper_y + 68}">{vhex}  →  {bhex}</text>
  <text class="mono ex" x="48" y="{tamper_y + 96}">{value:.4f} m  →  {bad_value:.4f} m</text>
  <text class="mono bad" x="48" y="{tamper_y + 124}">fact_cid {short(bad_cid)}</text>
  <text class="sans txt" x="48" y="{tamper_y + 154}">The old id does not name the new bytes.</text>

  <text class="sans txt" x="24" y="{foot_y}">Same bytes, same id, on every machine.</text>
  <text class="sans verb" x="24" y="{foot_y + 36}">Bytes from emem.dev, 7 November 2026, 12:17 UTC.</text>
  <text class="sans verb" x="24" y="{foot_y + 58}">Value from Copernicus DEM GLO-30.</text>
</svg>
'''
Path(sys.argv[1]).write_text(svg)
print(f"{sys.argv[1]}: value {value} -> {bad_value}, cid {short(bad_cid)}, height {height}")
