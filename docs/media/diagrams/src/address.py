"""Diagram: what a cell64 string tells you about a place, and what it does not.

Inputs, saved from emem.dev on 7 November 2026 at 12:27 UTC (reads only), in data/address/:
  locate-fuji.json   POST /v1/locate {"place": "Mount Fuji"}
  locate-north.json  POST /v1/locate {"lat": 35.4528, "lng": 138.7307677}   (about 10 km north)
  locate-west.json   POST /v1/locate {"lat": 35.3628384, "lng": -119.0}     (same latitude, California)
The script ports the encoder of crates/emem-codec/src/geo.rs and alphabet.rs, and stops unless the port gives
the same strings as the live responder for all three points and the nine neighborhood_cells.
usage: python3 address.py OUT.svg
"""
import json
import math
import sys
from pathlib import Path
from xml.sax.saxutils import escape

D = Path(__file__).resolve().parent / "data" / "address"

# Port of emem-codec: alphabet.rs build_alphabet_v0 and geo.rs cell_from_latlng.
CONS, VOWS = "bcdfghjklmnpqrstvwxyz", "aeiouAEIOU"
ALPHA = [a + b + c + d for a in CONS for b in VOWS for c in CONS for d in VOWS]
ALPHA += [f"z{i:04x}" for i in range(len(ALPHA), 65536)]
INDEX = {s: i for i, s in enumerate(ALPHA)}
LAT_BITS, LNG_BITS = 21, 22
LAT_MAX, LNG_MAX = (1 << LAT_BITS) - 1, (1 << LNG_BITS) - 1
PREFIX = (1 << 60) | (LAT_BITS << 52) | (0xAB << 44)
PITCH = 180.0 / LAT_MAX * 111_319.491


def qlat(lat):
    return round((lat + 90.0) / 180.0 * LAT_MAX)


def qlng(lng):
    q = round((((lng + 180.0) % 360.0) / 360.0) * LNG_MAX)
    return 0 if q == LNG_MAX else q


def encode(lat_q, lng_q):
    raw = PREFIX | (lat_q << LNG_BITS) | lng_q
    return ".".join(ALPHA[(raw >> s) & 0xFFFF] for s in (48, 32, 16, 0))


def decode(c):
    raw = 0
    for part in c.split("."):
        raw = (raw << 16) | INDEX[part]
    assert raw & 0xFFFFF00000000000 == PREFIX
    return (raw >> LNG_BITS) & LAT_MAX, raw & LNG_MAX


def load(n):
    return json.loads((D / f"locate-{n}.json").read_text())


fuji, north, west = load("fuji"), load("north"), load("west")
F = fuji["cell64"]
assert F == "defi.zb592.nUkO.zEzE"
sel = next(a for a in fuji["alternatives"] if a["is_selected"])
assert encode(qlat(sel["lat"]), qlng(sel["lng"])) == F
for d in (north, west):
    assert encode(qlat(d["lat_input"]), qlng(d["lng_input"])) == d["cell64"]
N, Wc = north["cell64"], west["cell64"]
la, lo = decode(F)
grid = {}
for c in fuji["neighborhood_cells"]:
    a, b = decode(c)
    grid[(a - la, b - lo)] = c
assert set(grid) == {(i, j) for i in (-1, 0, 1) for j in (-1, 0, 1)}


def km(p, q):
    (a1, o1), (a2, o2) = (math.radians(x) for x in p), (math.radians(x) for x in q)
    h = math.sin((a2 - a1) / 2) ** 2 + math.cos(a1) * math.cos(a2) * math.sin((o2 - o1) / 2) ** 2
    return 2 * 6371.0088 * math.asin(math.sqrt(h))


fc = (fuji["centre"]["lat_deg"], fuji["centre"]["lng_deg"])
d_north = km(fc, (north["centre"]["lat_deg"], north["centre"]["lng_deg"]))
d_west = km(fc, (west["centre"]["lat_deg"], west["centre"]["lng_deg"]))
band_km = (1 << 10) * PITCH / 1000
assert F.split(".")[:2] == Wc.split(".")[:2] and F.split(".")[1] != N.split(".")[1]
ns_m = PITCH
ew_m = PITCH * math.cos(math.radians(fc[0]))

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


def parts(x, y, cell, ref, size="cellm"):
    """Draw a cell64 with the parts that differ from ref in the id colour, the rest muted."""
    ps, rs = cell.split("."), ref.split(".")
    spans = '<tspan class="same">.</tspan>'.join(
        f'<tspan class="{"same" if p == r else "diff"}">{escape(p)}</tspan>' for p, r in zip(ps, rs))
    out.append(f'<text class="mono {size}" x="{x}" y="{y}">{spans}</text>')


t("sans h", X - 16, 42, "What a cell64 tells you about a place")
t("sans txt", X - 16, 70, "Mount Fuji, from POST /v1/locate")

# Card 1: the four parts
y = 96
box(X, y, CW, 262)
head(y, "1", "Four parts, 16 bits each")
cw4 = (CW - 32) // 4
labels = [
    ("geo tag", "the same for", "every geo cell"),
    ("latitude", f"band, {band_km:.1f} km", "north to south"),
    ("finer latitude", "and coarse", "longitude"),
    ("fine", "longitude", ""),
]
for i, (part, lab) in enumerate(zip(F.split("."), labels)):
    bx = X + 16 + i * cw4
    box(bx + 2, y + 66, cw4 - 4, 60, "door", 6)
    t("mono big", bx + cw4 // 2, y + 105, part, "middle")
    for k, s in enumerate(lab):
        if s:
            t("sans txt", bx + cw4 // 2, y + 156 + k * 24, s, "middle")
t("sans txt", X + 16, y + 240, f"One cell is about {ns_m:.2f} m north to south.")

# Card 2: neighbours
y = 380
box(X, y, CW, 336)
head(y, "2", "Next-door cells change the end")
t("sans txt", X + 16, y + 70, "All nine start with")
t("mono ex", X + 186, y + 70, "defi.zb592.")
gx, gy, gw, gh = X + 84, y + 92, 160, 56
for r, dl in enumerate((1, 0, -1)):
    for cidx, dg in enumerate((-1, 0, 1)):
        c = grid[(dl, dg)]
        x0, y0 = gx + cidx * gw, gy + r * gh
        box(x0, y0, gw - 8, gh - 8, "own" if (dl, dg) == (0, 0) else "door", 6)
        spans = '<tspan class="same">.</tspan>'.join(
            f'<tspan class="{"same" if p == q else "diff"}">{escape(p)}</tspan>'
            for p, q in zip(c.split(".")[2:], F.split(".")[2:]))
        out.append(f'<text class="mono ex2" x="{x0 + (gw - 8) // 2}" y="{y0 + 31}" text-anchor="middle">{spans}</text>')
t("sans foot", X + 62, gy + 31, "N", "middle")
t("sans foot", X + 62, gy + 143, "S", "middle")
t("sans txt", X + 16, y + 284, "West is on the left. North or south: part 3 changes.")
t("sans txt", X + 16, y + 308, "East or west: part 4 changes.")

# Card 3: a shared start is not nearness
y = 738
box(X, y, CW, 330)
head(y, "3", "The same start does not mean near")
rows = [
    ("Mount Fuji", F, "", F),
    (f"{d_north:.0f} km north", N, "parts 2 and 3 change", F),
    ("35.36 N, 119.00 W", Wc, f"{d_west:,.0f} km away", F),
]
yy = y + 76
for name, cell, note, ref in rows:
    t("sans term2", X + 16, yy, name)
    if note:
        t("sans txt", X + CW - 16, yy, note, "end")
    parts(X + 16, yy + 30, cell, ref)
    yy += 74
t("sans txt", X + 16, y + 292, "To find neighbours, read neighborhood_cells")
t("sans txt", X + 16, y + 316, "from /v1/locate. Do not compare strings.")

y = 1100
foot = [
    "Live reads of POST /v1/locate on 7 November 2026 at 12:27 UTC.",
    "A port of emem-codec geo.rs gives the same 11 strings.",
    f"East to west, one cell at Fuji is about {ew_m:.1f} m.",
]
for s in foot:
    t("sans foot", X - 16, y, s)
    y += 24
H = y + 4

desc = (
    "What a cell64 tells you about a place, shown for Mount Fuji from POST /v1/locate. "
    f"1: the cell64 {F} has four parts of 16 bits. Part 1, defi, is a geo tag and is the same for every geo cell. "
    f"Part 2, zb592, is a latitude band about {band_km:.1f} km tall. Part 3, nUkO, holds finer latitude and "
    f"coarse longitude. Part 4, zEzE, holds fine longitude. One cell is about {ns_m:.2f} m north to south. "
    "2: the nine cells around the summit all start with defi.zb592. Going north or south changes part 3; "
    f"going east or west changes part 4 (west is on the left). 3: a point {d_north:.0f} km north is {N}, so parts 2 and 3 change. "
    f"A point at the same latitude in California, 35.36 N 119.00 W, is {Wc}. It starts with the same "
    f"defi.zb592 but is {d_west:,.0f} km away. To find neighbours, read neighborhood_cells from /v1/locate; do "
    "not compare strings. Live reads on 7 November 2026 at 12:27 UTC; a port of the emem-codec encoder gives "
    "the same 11 strings."
)

svg = f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}" role="img" aria-labelledby="t d">
  <title id="t">What a cell64 tells you about a place</title>
  <desc id="d">{escape(desc)}</desc>
  <style>
    .sans {{ font-family: "DejaVu Sans", system-ui, -apple-system, "Segoe UI", Helvetica, Arial, sans-serif; }}
    .mono {{ font-family: "DejaVu Sans Mono", ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }}
    .h {{ font-size: 23px; font-weight: 600; fill: #e6edf3; }}
    .term {{ font-size: 20px; font-weight: 600; fill: #e6edf3; }}
    .term2 {{ font-size: 18px; font-weight: 600; fill: #e6edf3; }}
    .big {{ font-size: 24px; font-weight: 600; fill: #79c0ff; }}
    .ex {{ font-size: 18px; fill: #79c0ff; }}
    .ex2 {{ font-size: 17px; }}
    .cellm {{ font-size: 21px; }}
    .same {{ fill: #8b949e; }}
    .diff {{ fill: #79c0ff; font-weight: 600; }}
    .txt {{ font-size: 17px; fill: #8b949e; }}
    .foot {{ font-size: 16px; fill: #8b949e; }}
    .num {{ font-size: 17px; font-weight: 600; fill: #79c0ff; }}
    .box {{ fill: #161b22; stroke: #30363d; stroke-width: 1; }}
    .door {{ fill: #0d1117; stroke: #30363d; stroke-width: 1; }}
    .own {{ fill: #0d1117; stroke: #79c0ff; stroke-width: 2; }}
    .ring {{ fill: #0d1117; stroke: #79c0ff; stroke-width: 1.5; }}
  </style>
  <rect x="0" y="0" width="{W}" height="{H}" rx="12" fill="#0d1117"/>
  {chr(10).join("  " + o for o in out).lstrip()}
</svg>
'''
Path(sys.argv[1]).write_text(svg)
print(sys.argv[1], W, H, f"north {d_north:.2f} km, west {d_west:.0f} km, band {band_km:.2f} km, ew {ew_m:.2f} m")
