#!/usr/bin/env python3
"""The 2.1.0 release card in the Mithila hand.

The art is the release, not decoration around it. 2.1.0's correctness work was
the admission that the nine token shapes do not carry one guarantee, so the
card draws them as nine roundels around the consolidation lotus and marks the
one that binds a complete signed body: `emem:fact:` is drawn large, sealed and
double-ringed, and the streams flow from it. The other eight are drawn true to
what they are, which is smaller and unsealed.

Writes web/release-2.1.0.svg (source of truth) and rasterises it to
web/release-2.1.0.png at 1200x630, the same frame as the og-image, so the card
can be posted anywhere a social preview is expected.
"""
import math
import os
import re
import sys

_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import _mithila_svg as M
from _mithila_svg import Svg

def _workspace_version() -> str:
    """The one place a version is allowed to live. A constant here silently
    regenerates the previous release's card; that is how release-current.svg
    still said 2.1.0 after the 2.2.0 bump, on the file served at /release.png."""
    txt = open(os.path.join(_ROOT, "Cargo.toml"), encoding="utf-8").read()
    m = re.search(r'^version\s*=\s*"([^"]+)"', txt, re.M)
    if not m:
        raise SystemExit("gen_release_card: no version in Cargo.toml")
    return m.group(1)


def _canon(key: str) -> int:
    """sync_counts owns the counts and gates them against the live node."""
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    from sync_counts import CANON
    return CANON[key]


VERSION = _workspace_version()

# The nine shapes, in the order the reference lists them. `strong` is the one
# claim this release made precise: only emem:fact: hashes the complete body at
# full width, so only it is drawn sealed.
SHAPES = [
    ("fact",        M.VERMIL,  True),
    ("cell",        M.INDIGO,  False),
    ("entity",      M.LAC,     False),
    ("bundle",      M.TEAL,    False),
    ("raster",      M.LEAF,    False),
    ("cube",        M.TURMERIC, False),
    ("rasterset",   M.INDIGO,  False),
    ("trace",       M.TEAL,    False),
    ("attestation", M.LAC,     False),
]


def build():
    # 1200x630 is read small; keep the wordmark huge and the body copy short.
    s = Svg(1200, 630, seed=11, fscale=1.0)
    HX, HY = 896, 318          # lotus hub, right third
    LR = 66

    # Nine shapes on an ellipse around the hub. fact sits at the top of the
    # ring (-90 degrees) so the eye lands on the one that actually verifies.
    ring = []
    n = len(SHAPES)
    for i, (name, col, strong) in enumerate(SHAPES):
        th = -math.pi / 2 + 2 * math.pi * i / n
        rx, ry = 234, 188
        x, y = HX + rx * math.cos(th), HY + ry * math.sin(th)
        ring.append((x, y, col, 30 if strong else 19, name, strong))

    avoid = [(HX, HY, LR + 34), (250, 315, 340)]
    for (x, y, _c, r, _n, _s) in ring:
        avoid.append((x, y, r + 20))
    s.paper()
    s.fill_ground(avoid=avoid, top=68)
    s.border()

    # Streams: every shape reaches the hub, but only the sealed one carries a
    # solid, full-strength line. The rest are drawn thinner and dashed, because
    # what they hand you is a shared reference rather than shared bytes.
    for (x, y, col, r, name, strong) in ring:
        ctrl = ((x + HX) / 2 + (HY - y) * 0.14, (y + HY) / 2 - (HX - x) * 0.14)
        if strong:
            # deliberately NOT the animated .flowline class: that carries a
            # stroke-dasharray, and this line has to read as unbroken in the
            # rasterised card, where the contrast with the eight dashed
            # references is the whole point.
            s.qpath((x, y), ctrl, (HX, HY), M.VERMIL, 5.4, op=0.22, glow=True)
            s.qpath((x, y), ctrl, (HX, HY), M.VERMIL, 2.2, op=0.85)
        else:
            s.qpath((x, y), ctrl, (HX, HY), M.INK_SOFT, 1.1, op=0.34, dash="4 7")

    s.lotus(HX, HY, LR)

    inners = {"fact": "ring", "cell": "hatch", "entity": "leaf", "bundle": "fish",
              "raster": "hatch", "cube": "ring", "rasterset": "fish",
              "trace": "leaf", "attestation": "ring"}
    for (x, y, col, r, name, strong) in ring:
        s.roundel(x, y, r, col, signed=strong, petal=not strong,
                  inner=inners[name])
        if strong:
            # a second ring: this is the one that re-hydrates byte-identical
            s.circle(x, y, r + 9, stroke=M.VERMIL, sw=1.6, op=0.75)

    # Wordmark column, left. Serif mark, mono body, same hand as the og-image.
    s.text(74, 178, "emem", 92, M.INK, font=M.SERIF, weight="bold")
    s.text(392, 178, VERSION, 46, M.VERMIL, font=M.MONO, weight="bold")
    s.frieze(214, 76, 520, col=M.TURMERIC)

    s.text(78, 274, "verifiable memory for", 25, M.INK, font=M.MONO)
    s.text(78, 308, "agents and machines.", 25, M.INK, font=M.MONO)

    s.text(78, 366, "+ an intent registry: a need maps to one call", 15, M.INK_SOFT, font=M.MONO)
    s.text(78, 392, "+ A2A message/stream, tasks, a signed mailbox", 15, M.INK_SOFT, font=M.MONO)
    s.text(78, 418, "\u00b7 only emem:fact: binds the whole body", 15, M.INK_SOFT, font=M.MONO)

    s.seal(90, 486, 10, fill=M.LEAF)
    s.text(112, 492, "emem.dev", 22, M.INK, font=M.MONO, weight="bold")
    s.text(78, 536, f"{_canon('mcp_tools')} tools \u00b7 {_canon('algorithms')} algorithms \u00b7 no key to read", 14, M.INK_SOFT, font=M.MONO)
    return s


def main():
    s = build()
    # Two copies on purpose. The versioned pair is the archive: every release
    # keeps its own card and old ones are never overwritten. The `-current`
    # pair is what the responder bakes in and serves at /release.svg and
    # /release.png, so the served card follows the release without the binary
    # growing a new route for every version.
    svg_path = f"web/release-{VERSION}.svg"
    png_path = f"web/release-{VERSION}.png"
    s.save(svg_path)
    s.save("web/release-current.svg")
    print(f"wrote {svg_path} + web/release-current.svg")
    import cairosvg
    for out in (png_path, "web/release-current.png"):
        cairosvg.svg2png(url=svg_path, write_to=out,
                         output_width=1200, output_height=630)
    print(f"wrote {png_path} + web/release-current.png")


if __name__ == "__main__":
    main()
