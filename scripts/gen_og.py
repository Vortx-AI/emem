#!/usr/bin/env python3
"""The emem social card (og-image) in the Mithila hand: a dense engram beside
the wordmark. Writes web/og-image.svg (the source of truth) and rasterises it to
web/og-image.png (1200x630, the og:image) with cairosvg, so the two always match
the served gallery's hand.
"""
import math, os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import _mithila_svg as M
from _mithila_svg import Svg


def build():
    # a smaller font scale than the big gallery scenes: this card is read small,
    # so the wordmark is huge but the body copy must not overflow the left half.
    s = Svg(1200, 630, seed=3, fscale=1.0)
    HX, HY = 884, 320          # engram hub on the right
    LR = 86

    # phyllotaxis field of engram cells around the hub
    GOLD = math.radians(137.507)
    nodes = []
    for i in range(1, 34):
        rr = 96 + 26 * math.sqrt(i)
        th = i * GOLD
        x, y = HX + rr * math.cos(th), HY + rr * math.sin(th) * 0.72
        if 612 < x < 1150 and 96 < y < 552 and rr > 96:
            nodes.append((x, y, M.DYES[i % len(M.DYES)], 13 + (i % 3) * 4))

    # keep-clear discs: the hub + the whole left wordmark column
    avoid = [(HX, HY, LR + 40), (270, 315, 360)]
    for (x, y, c, r) in nodes:
        avoid.append((x, y, r + 18))
    s.paper(); s.fill_ground(avoid=avoid, top=70); s.border()

    # recall streams from half the cells into the consolidation lotus
    for i, (x, y, col, r) in enumerate(nodes):
        if i % 2:
            continue
        ctrl = ((x + HX) / 2 + (HY - y) * 0.12, (y + HY) / 2 - (HX - x) * 0.12)
        s.qpath((x, y), ctrl, (HX, HY), M.LEAF, 5.0, op=0.2, glow=True)
        s.qpath((x, y), ctrl, (HX, HY), M.LEAF, 1.4, op=0.6, cls="flowline")

    # a couple of storytelling motifs in the gaps so no corner sits dead
    s.bird(560, 150, 30, col=M.TEAL, angle=0.12)
    s.fish(600, 540, 22, col=M.INDIGO, angle=-0.1)

    s.lotus(HX, HY, LR)
    inners = ["fish", "leaf", "hatch", "ring"]
    for i, (x, y, col, r) in enumerate(nodes):
        s.roundel(x, y, r, col, signed=(i % 3 == 0), petal=(i % 2 == 0),
                  inner=inners[i % len(inners)])

    # wordmark + tagline, left column (huge serif mark, mono body)
    s.text(78, 196, "emem", 96, M.INK, font=M.SERIF, weight="bold")
    s.frieze(232, 80, 470, col=M.TURMERIC)
    s.text(82, 300, "a verifiable memory of Earth", 24, M.INK, font=M.MONO)
    s.text(82, 336, "for AI agents.", 24, M.INK, font=M.MONO)
    s.text(82, 396, "every answer signed · content-addressed", 16, M.INK_SOFT, font=M.MONO)
    s.text(82, 422, "verifiable offline at /verify", 16, M.INK_SOFT, font=M.MONO)
    s.seal(94, 484, 10, fill=M.LEAF)
    s.text(116, 490, "emem.dev", 22, M.INK, font=M.MONO, weight="bold")
    return s


def main():
    s = build()
    s.save("web/og-image.svg")
    print("wrote web/og-image.svg")
    import cairosvg
    cairosvg.svg2png(url="web/og-image.svg", write_to="web/og-image.png",
                     output_width=1200, output_height=630)
    print("wrote web/og-image.png")


if __name__ == "__main__":
    main()
