"""Diagram: how to check an emem receipt without trusting the server, from one live receipt.

Inputs, saved from emem.dev on 7 November 2026 at 12:44 UTC (reads only), in data/receipt/:
  recall-fuji.json       POST /v1/recall {"cell": "defi.zb592.nUkO.zEzE", "bands": ["copdem30m.elevation_mean"]}
  emem-well-known.json   GET /.well-known/emem.json (the published responder key)
The script rebuilds the v2 preimage with the offline verifier of the emem-verify-receipt skill, checks the
Ed25519 signature against the published key, and stops unless the check passes. It then changes one character
of served_at and stops unless the same check fails.
usage: python3 receipt.py OUT.svg
"""
import copy
import json
import sys
from pathlib import Path
from xml.sax.saxutils import escape

HERE = Path(__file__).resolve().parent
D = HERE / "data" / "receipt"
sys.dont_write_bytecode = True
sys.path.insert(0, str(HERE.parents[3] / "plugins/emem/skills/emem-verify-receipt/scripts"))
import verify as V  # noqa: E402
from emem_crypto import blake3, ed25519_verify  # noqa: E402

resp = json.loads((D / "recall-fuji.json").read_text())
wk = json.loads((D / "emem-well-known.json").read_text())
R = resp["receipt"]
fact = resp["facts"][0]
KEY = wk["responder"]["pubkey_b32"]
assert R["preimage_version"] == 2 and R["responder_pubkey_b32"] == KEY
pub, sig = V.b32_nopad_decode(KEY), V._sig_bytes(R)


def check(r):
    pre = V.build_preimage(r, 2)
    dig = blake3(pre).digest()
    return pre, dig, ed25519_verify(pub, sig, dig)


PRE, DIG, OK = check(R)
assert OK and len(PRE) == 328
bad = copy.deepcopy(R)
assert bad["served_at"].endswith(":18Z")
bad["served_at"] = bad["served_at"][:-3] + "19Z"
_, BAD_DIG, BAD_OK = check(bad)
assert not BAD_OK

mh = V._manifest_hex(R["source_versions"])
segs = [
    ("header", '"receipt" domain', 17 + 4 + 7),
    ("request_id", R["request_id"], 5 + len(R["request_id"])),
    ("served_at", R["served_at"], 5 + len(R["served_at"])),
    ("manifest", "blake3 of 4 ids", 5 + len(mh)),
    ("primitive", R["primitive"], 5 + len(R["primitive"])),
    ("cells", R["cells"][0], 9 + len(R["cells"][0])),
    ("fact_cids", R["fact_cids"][0][:8] + "...", 9 + len(R["fact_cids"][0])),
    ("merkle", "proof, hashed", 5 + 64),
]
assert sum(s[2] for s in segs) == len(PRE)


def short(h):
    return h[:16] + "..." + h[-8:]


W = 640
X, CW = 40, 560
out = []


def t(cls, x, y, s, anchor=None):
    a = f' text-anchor="{anchor}"' if anchor else ""
    out.append(f'<text class="{cls}" x="{x}" y="{y}"{a}>{escape(s)}</text>')


def box(x, y, w, h, cls="box", rx=8):
    out.append(f'<rect class="{cls}" x="{x}" y="{y}" width="{w}" height="{h}" rx="{rx}"/>')


def head(y, num, term):
    out.append(f'<circle class="ring" cx="{X + 24}" cy="{y + 30}" r="15"/>')
    t("sans num", X + 24, y + 36, num, "middle")
    t("sans term", X + 52, y + 37, term)


t("sans h", X - 16, 42, "How to check a receipt without the server")
t("sans txt", X - 16, 70, f"Mount Fuji, one recall: {fact['value']:.2f} m elevation")

# Card 1: what the signature covers
y = 96
box(X, y, CW, 78 + 32 * len(segs) + 34)
head(y, "1", "Rebuild the signed bytes")
yy = y + 78
for name, val, n in segs:
    t("sans term2", X + 16, yy, name)
    t("mono ex", X + 150, yy, val)
    t("sans txt", X + CW - 16, yy, f"{n} B", "end")
    yy += 32
out.append(f'<line class="rule" x1="{X + 16}" y1="{yy - 20}" x2="{X + CW - 16}" y2="{yy - 20}"/>')
t("sans term2", X + 16, yy + 6, "preimage")
t("sans term2", X + CW - 16, yy + 6, f"{len(PRE)} B", "end")
y1end = yy + 34

# Card 2: hash and check
y = y1end + 22
box(X, y, CW, 246)
head(y, "2", "Hash it, check the signature")
t("sans txt", X + 16, y + 74, "digest = blake3(preimage)")
t("mono ex", X + 16, y + 102, short(DIG.hex()))
t("sans txt", X + 16, y + 140, "key from /.well-known/emem.json")
t("mono ex", X + 16, y + 168, KEY[:8] + "..." + KEY[-6:])
box(X + 16, y + 186, CW - 32, 44, "okbox", 6)
t("sans ok", X + 32, y + 215, "ed25519 verify: VALID")
y2end = y + 246

# Card 3: one character changed
y = y2end + 22
box(X, y, CW, 246)
head(y, "3", "Change one character: it fails")
t("sans txt", X + 16, y + 74, "served_at")
out.append(
    f'<text class="mono ex" x="{X + 16}" y="{y + 102}">{escape(R["served_at"][:-3])}'
    f'<tspan class="badtxt">19</tspan>Z <tspan class="same">(was 18)</tspan></text>')
t("sans txt", X + 16, y + 140, "new digest")
t("mono bad2", X + 16, y + 168, short(BAD_DIG.hex()))
box(X + 16, y + 186, CW - 32, 44, "badbox", 6)
t("sans badtxt", X + 32, y + 215, "same signature: INVALID")
y3end = y + 246

y = y3end + 34
foot = [
    "Read from emem.dev on 7 November 2026 at 12:44 UTC.",
    "Checked offline with the verify.py of the",
    "emem-verify-receipt skill, BLAKE3 and Ed25519 in",
    "plain Python. A VALID check shows which key signed",
    "these fact ids. It does not show the value is true.",
]
for s in foot:
    t("sans foot", X - 16, y, s)
    y += 24
H = y + 4

desc = (
    "How to check an emem receipt without trusting the server, shown with one live recall at Mount Fuji "
    f"(elevation {fact['value']:.2f} m). 1: rebuild the signed bytes from the receipt fields: header "
    f"{segs[0][2]} B, request_id {segs[1][2]} B, served_at {segs[2][2]} B, manifest {segs[3][2]} B, primitive "
    f"{segs[4][2]} B, cells {segs[5][2]} B, fact_cids {segs[6][2]} B, merkle {segs[7][2]} B; the preimage is "
    f"{len(PRE)} bytes. 2: the digest is blake3 of the preimage, {short(DIG.hex())}. Take the responder key "
    f"from /.well-known/emem.json, {KEY[:8]}, and Ed25519 verify gives VALID. 3: change one character of "
    f"served_at, 18 to 19 seconds. The digest becomes {short(BAD_DIG.hex())} and the same signature gives "
    "INVALID. Read from emem.dev on 7 November 2026 at 12:44 UTC and checked offline with verify.py of the "
    "emem-verify-receipt skill. A valid check shows which key signed these fact ids, not that the value is true."
)

svg = f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}" role="img" aria-labelledby="t d">
  <title id="t">How to check a receipt without the server</title>
  <desc id="d">{escape(desc)}</desc>
  <style>
    .sans {{ font-family: "DejaVu Sans", system-ui, -apple-system, "Segoe UI", Helvetica, Arial, sans-serif; }}
    .mono {{ font-family: "DejaVu Sans Mono", ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }}
    .h {{ font-size: 23px; font-weight: 600; fill: #e6edf3; }}
    .term {{ font-size: 20px; font-weight: 600; fill: #e6edf3; }}
    .term2 {{ font-size: 17px; font-weight: 600; fill: #e6edf3; }}
    .ex {{ font-size: 17px; fill: #79c0ff; }}
    .bad2 {{ font-size: 17px; fill: #ff7b72; }}
    .same {{ fill: #8b949e; }}
    .txt {{ font-size: 17px; fill: #8b949e; }}
    .foot {{ font-size: 16px; fill: #8b949e; }}
    .num {{ font-size: 17px; font-weight: 600; fill: #79c0ff; }}
    .ok {{ font-size: 19px; font-weight: 600; fill: #7ee787; }}
    .badtxt {{ font-size: 19px; font-weight: 600; fill: #ff7b72; }}
    .ex .badtxt {{ font-size: 17px; }}
    .box {{ fill: #161b22; stroke: #30363d; stroke-width: 1; }}
    .okbox {{ fill: #0d1117; stroke: #7ee787; stroke-width: 2; }}
    .badbox {{ fill: #0d1117; stroke: #ff7b72; stroke-width: 2; }}
    .ring {{ fill: #0d1117; stroke: #79c0ff; stroke-width: 1.5; }}
    .rule {{ stroke: #30363d; stroke-width: 1; }}
  </style>
  <rect x="0" y="0" width="{W}" height="{H}" rx="12" fill="#0d1117"/>
  {chr(10).join("  " + o for o in out).lstrip()}
</svg>
'''
Path(sys.argv[1]).write_text(svg)
print(sys.argv[1], W, H, "preimage", len(PRE), "digest", DIG.hex(), "bad", BAD_DIG.hex())
