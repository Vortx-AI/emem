"""Diagram: what answers when an agent calls emem, drawn from live reads of emem.dev.

Inputs, saved from emem.dev on 7 November 2026 at 12:22 UTC (reads only), in data/architecture/:
  sources.json        GET /v1/sources (the sources registry)
  sth.json            GET /v1/log/sth (signed tree head of the transparency log)
  openapi-paths.json  the path list of GET /openapi.json
  well-known.json     the responder block of GET /.well-known/emem.json
  tools.json          the tool names of GET /v1/tools
The fact id in step 4 is the Mount Fuji elevation fact that data_flow.py re-hashes from its CBOR bytes.
usage: python3 architecture.py OUT.svg
"""
import json
import sys
from pathlib import Path
from xml.sax.saxutils import escape

D = Path(__file__).resolve().parent / "data" / "architecture"


def load(name):
    return json.loads((D / name).read_text())


sources = load("sources.json")["sources"]
sth = load("sth.json")["sth"]
paths = load("openapi-paths.json")["paths"]
wk = load("well-known.json")
tools = load("tools.json")

n_sources = len(sources)
n_sidecar = sum(any(p["kind"] == "gpu_sidecar" for p in s["providers"]) for s in sources)
n_v1 = sum(p.startswith("/v1/") for p in paths)
n_tools = tools["total"]
assert n_tools == len(tools["names"])
key = wk["responder"]["pubkey_b32"]
assert key == sth["responder_pubkey_b32"]
assert wk["responder"]["signature_alg"] == "ed25519" and wk["responder"]["hash_alg"] == "blake3"
schemes = {s["scheme"] for s in sources}
assert {"sentinel2.l2a", "copernicus.dem.30m", "open_meteo"} <= schemes
size = f"{sth['tree_size']:,}"
at = sth["signed_at"]
assert at.startswith("2026-11-07T")
hhmm = at[11:16]
FACT = "qejpz7cgvk347ordd6vrdtmwzprqlzcnkmotsjton2x4y2jjbplq"

W = 640
X, CW = 40, 560          # card left and width
TX = 92                  # text left inside a card
out = []


def t(cls, x, y, s, anchor=None):
    a = f' text-anchor="{anchor}"' if anchor else ""
    out.append(f'<text class="{cls}" x="{x}" y="{y}"{a}>{escape(s)}</text>')


def card(y, h, num, term, verb, lines):
    out.append(f'<rect class="box" x="{X}" y="{y}" width="{CW}" height="{h}" rx="8"/>')
    out.append(f'<circle class="ring" cx="{X + 24}" cy="{y + 30}" r="15"/>')
    t("sans num", X + 24, y + 36, num, "middle")
    t("sans term", TX, y + 37, term)
    if verb:
        t(verb[0], TX + verb[1], y + 37, verb[2])
    yy = y + 69
    for cls, s in lines:
        if cls == "pair":
            t("mono ex", TX, yy, s[0])
            t("sans txt", TX + 196, yy, s[1])
        else:
            t(cls, TX, yy, s)
        yy += 28
    return y + h


def arrow(y0, y1, cls="flow", m="a"):
    out.append(f'<path class="{cls}" d="M{X + 24} {y0} V{y1 - 2}" marker-end="url(#{m})"/>')


steps = [
    ("2", "Look up", ("sans txt", 120, "sled cache on local disk"), [
        ("sans txt", "A hit returns the stored fact at once."),
        ("sans txt", "A miss goes to step 3."),
    ]),
    ("3", "Fetch on a miss", ("sans txt", 190, "open data, no copy"), [
        ("sans txt", f"{n_sources} sources in the registry, for example:"),
        ("pair", ("sentinel2.l2a", "STAC, Planetary Computer")),
        ("pair", ("copernicus.dem.30m", "COG, AWS open data")),
        ("pair", ("open_meteo", "JSON API")),
        ("sans txt", "HTTP range reads take only the bytes needed."),
    ]),
    ("4", "Hash", ("sans txt", 90, "canonical CBOR, BLAKE3"), [
        ("sans txt", "The value, cell, time and method become bytes."),
        ("sans txt", "Their hash is the fact id:"),
        ("mono ex", FACT[:26]),
        ("mono ex", FACT[26:]),
    ]),
    ("5", "Sign", ("sans txt", 90, "Ed25519, one responder key"), [
        ("mono sig", key[:26]),
        ("mono sig", key[26:]),
        ("sans txt", "The key signs a batch root of fact ids,"),
        ("sans txt", "and a receipt for each read."),
    ]),
    ("6", "Log", ("mono verb", 70, "GET /v1/log/sth"), [
        ("sans txt", "Each write appends to an RFC 6962 Merkle log."),
        ("sans txt", "A signed tree head proves it only grows."),
        ("sans val", f"{size} entries at {hhmm} UTC"),
    ]),
]

y = 24
t("sans h", X - 16, y + 18, "One binary fetches, hashes, signs and logs")
t("sans txt", X - 16, y + 46, "What answers when an agent calls emem.dev")
y = 96

y = card(y, 152, "1", "Call", ("sans txt", 80, "two doors, same handlers"), [])
# two doors inside card 1
for i, (name, route, count) in enumerate((("REST", f"/v1/*", f"{n_v1} paths"),
                                          ("MCP", "POST /mcp", f"{n_tools} tools"))):
    bx = TX + i * 240
    out.append(f'<rect class="door" x="{bx}" y="{y - 100}" width="224" height="80" rx="6"/>')
    t("sans term", bx + 14, y - 70, name)
    t("mono ex", bx + 90, y - 70, route)
    t("sans txt", bx + 14, y - 38, count)
arrow(y, y + 50)

# the binary: an outline around steps 2 to 6
top = y + 50
y = top + 52
inner = []
heights = [116, 204, 172, 172, 144]
for (num, term, verb, lines), h in zip(steps, heights):
    y = card(y, h, num, term, verb, lines)
    if num != "6":
        arrow(y, y + 26)
        y += 26
bottom = y + 16
out.insert(0, f'<rect class="bin" x="{X - 20}" y="{top}" width="{CW + 40}" height="{bottom - top}" rx="12"/>')
t("sans term", X, top + 34, "emem-server")
t("sans txt", X + 152, top + 34, f"one Rust binary, version {wk['version']}")

arrow(bottom, bottom + 50)
y = card(bottom + 50, 172, "7", "Answer", ("sans txt", 110, "fact + receipt + token"), [
    ("sans txt", "Anyone checks the receipt offline with the"),
    ("sans txt", "public key at /.well-known/emem.json."),
    ("sans ok", "valid: the bytes are what this key signed."),
    ("sans txt", "A signature shows who signed, not that it is true."),
])

y += 36
foot = [
    f"Live reads of emem.dev on 7 November 2026 at {hhmm} UTC:",
    "/v1/sources, /openapi.json, /v1/tools, /v1/log/sth.",
    f"{n_sidecar} of the {n_sources} sources name a GPU model sidecar that no longer runs.",
]
for s in foot:
    t("sans foot", X - 16, y, s)
    y += 24
H = y + 4

desc = (
    f"What answers when an agent calls emem.dev. Step 1, Call: REST has {n_v1} paths under /v1, MCP at POST /mcp "
    f"has {n_tools} tools, and both reach the same handlers. Steps 2 to 6 run inside one Rust binary, emem-server "
    f"version {wk['version']}. 2 Look up: a sled cache on local disk; a hit returns the stored fact. "
    f"3 Fetch on a miss: {n_sources} sources in the registry, for example sentinel2.l2a through Planetary Computer "
    "STAC, copernicus.dem.30m as a COG on AWS open data, and open_meteo as a JSON API; HTTP range reads take only "
    f"the bytes needed. 4 Hash: canonical CBOR, and its BLAKE3 hash is the fact id, for example {FACT}. "
    f"5 Sign: one Ed25519 responder key, {key}, signs a batch root of fact ids and a receipt for each read. "
    f"6 Log: each write appends to an RFC 6962 Merkle log; the signed tree head showed {size} entries at "
    f"{hhmm} UTC. 7 Answer: fact, receipt and token; anyone checks the receipt offline with the public key at "
    "/.well-known/emem.json. A signature shows who signed, not that the value is true. "
    f"Live reads on 7 November 2026. {n_sidecar} of the {n_sources} sources name a GPU model sidecar that no "
    "longer runs."
)

svg = f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}" role="img" aria-labelledby="t d">
  <title id="t">One binary fetches, hashes, signs and logs</title>
  <desc id="d">{escape(desc)}</desc>
  <defs>
    <marker id="a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto">
      <path d="M0 0 L10 5 L0 10 z" fill="#8b949e"/>
    </marker>
  </defs>
  <style>
    .sans {{ font-family: "DejaVu Sans", system-ui, -apple-system, "Segoe UI", Helvetica, Arial, sans-serif; }}
    .mono {{ font-family: "DejaVu Sans Mono", ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }}
    .h {{ font-size: 23px; font-weight: 600; fill: #e6edf3; }}
    .term {{ font-size: 20px; font-weight: 600; fill: #e6edf3; }}
    .val {{ font-size: 18px; font-weight: 600; fill: #e6edf3; }}
    .ex {{ font-size: 16px; fill: #79c0ff; }}
    .sig {{ font-size: 16px; fill: #d2a8ff; }}
    .ok {{ font-size: 18px; fill: #7ee787; }}
    .txt {{ font-size: 18px; fill: #8b949e; }}
    .verb {{ font-size: 16px; fill: #8b949e; }}
    .foot {{ font-size: 16px; fill: #8b949e; }}
    .num {{ font-size: 17px; font-weight: 600; fill: #79c0ff; }}
    .box {{ fill: #161b22; stroke: #30363d; stroke-width: 1; }}
    .door {{ fill: #0d1117; stroke: #30363d; stroke-width: 1; }}
    .bin {{ fill: none; stroke: #79c0ff; stroke-width: 1.5; stroke-dasharray: 7 5; }}
    .ring {{ fill: #0d1117; stroke: #79c0ff; stroke-width: 1.5; }}
    .flow {{ stroke: #8b949e; stroke-width: 1.5; fill: none; }}
  </style>
  <rect x="0" y="0" width="{W}" height="{H}" rx="12" fill="#0d1117"/>
  {chr(10).join("  " + o for o in out).lstrip()}
</svg>
'''
Path(sys.argv[1]).write_text(svg)
print(sys.argv[1], W, H)
