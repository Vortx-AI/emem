#!/usr/bin/env python3
"""The README comic strip: what emem is, in six panels, in the Mithila hand.

Design rules this file enforces rather than hopes for:

  * Every panel splits into a TEXT column and an ART column with a hard gutter
    between them. Nothing is drawn into the other's range, so the two cannot
    overlap no matter how the copy changes.
  * Every string is measured before it is drawn. DejaVu Sans Mono advances at
    ~0.602em, so a line's width is computable; a line that would overrun its
    column fails the build instead of shipping clipped. Same for the serif
    titles at a measured ~0.55em.
  * The strip is authored at 1120px wide because GitHub renders README images
    at about 890px. That is a 0.79 scale, so 19px body copy lands near 15px on
    the page, which is readable. Authoring wider would look sharp here and
    illegible there.

The copy is the honest version on purpose. Panel six is the limits panel and
is not decoration: a developer who reads this strip and then finds out about
the single responder, or about entity tokens binding an anchor rather than a
body, should find it was on the first image in the README.
"""
from __future__ import annotations

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import _mithila_svg as M
from _mithila_svg import Svg

W = 1120
MX = 78                      # panel left/right margin (inside the inner rule)
PW = W - 2 * MX              # panel width
PH = 226                     # panel height
GAP = 15
HEAD_TOP, HEAD_H = 66, 128
FOOT_H, BOTTOM = 46, 62

TEXT_X = 34                  # text column, panel-relative
TEXT_W = 592
ART_X = TEXT_X + TEXT_W + 26  # hard gutter: art may not start before this
ART_W = PW - ART_X - 26

MONO_EM = 0.602              # DejaVu Sans Mono advance, exact: it is monospace
# Serif is proportional, so a single factor can only ever be an upper bound.
# 0.55 was not: it predicted the 54px "emem" wordmark at 118px when it renders
# near 180px, because lowercase m and e are among the widest glyphs in the face
# and the wordmark is nothing else. The header no longer places anything beside
# the wordmark, so no layout depends on this number; it now guards the titles
# only, at a bound wide enough to cover an all-wide-glyph string.
SERIF_EM = 0.62

VIOLATIONS: list[str] = []


def fits(kind: str, s: str, size: float, maxw: float, where: str):
    em = MONO_EM if kind == "mono" else SERIF_EM
    w = len(s) * size * em
    if w > maxw:
        VIOLATIONS.append(f"{where}: {kind} {size}px runs {w:.0f}px in {maxw:.0f}px — {s!r}")
    return s


# ---------------------------------------------------------------------------
# The six beats. Body lines are pre-wrapped by hand so the line breaks fall
# where the sentence breathes, not where an algorithm ran out of room.
# ---------------------------------------------------------------------------
PANELS = [
    dict(
        n="1",
        title="Two agents describe one field",
        body=["One reports 0.62. One reports \"looks healthy\".",
              "Neither can check the other, and neither names",
              "the same patch of ground the same way twice.",
              "That drift is the bug emem exists to remove."],
        art="drift",
    ),
    dict(
        n="2",
        title="One address, one signed fact",
        body=["The place resolves to a cell64 every agent",
              "derives identically. The reading becomes a fact:",
              "value, band, time, hashed with blake3 over",
              "canonical CBOR and signed with ed25519."],
        art="ground",
    ),
    dict(
        n="3",
        title="The fact collapses to one line",
        body=["emem:fact:<cell64>:<fact_cid>",
              "The cid is the full 32-byte digest, 52 base32",
              "characters, never truncated. An agent keeps",
              "this line instead of carrying the payload."],
        art="token",
        mono_title_line=0,
    ),
    dict(
        n="4",
        title="Anyone resolves it, anyone checks it",
        body=["Hand the token to another model or vendor and",
              "it returns the byte-identical signed body, or it",
              "does not resolve at all. The receipt verifies in",
              "your process. No key, no account, no callback."],
        art="handoff",
    ),
    dict(
        n="5",
        title="emem-guard checks before you assert",
        body=["Point it at a transcript and it reads the emem:",
              "tokens in it. PROV_SIG: the signature fails.",
              "PROV_BYTES: it resolves to different bytes.",
              "PROV_DRIFT: it moved past the band's threshold."],
        art="guard",
    ),
    dict(
        n="6",
        title="What it does not do",
        body=["One responder signs it; not a network consensus.",
              "A real citation can still sit on a wrong claim.",
              "Only emem:fact: binds a whole body: entity and",
              "bundle tokens co-refer, they do not pin bytes."],
        art="limits",
    ),
]


# ---------------------------------------------------------------------------
# Panel art. Each routine is handed its own box and draws only inside it.
# ---------------------------------------------------------------------------
def art_drift(s: Svg, x, y, w, h):
    cy = y + h / 2
    ax, bx = x + 46, x + w - 46
    s.roundel(ax, cy - 8, 27, M.INDIGO, inner="hatch", petal=True)
    s.roundel(bx, cy - 8, 27, M.LAC, inner="leaf", petal=True)
    # two readings that never meet: the paths bow apart and stay dashed
    s.qpath((ax + 30, cy - 8), ((ax + bx) / 2, cy - 62), (bx - 30, cy - 8),
            M.INK_SOFT, 1.2, op=0.42, dash="5 8")
    s.qpath((ax + 30, cy - 8), ((ax + bx) / 2, cy + 46), (bx - 30, cy - 8),
            M.INK_SOFT, 1.2, op=0.42, dash="5 8")
    s.bud((ax + bx) / 2, cy - 62, 11, col=M.VERMIL)
    s.bud((ax + bx) / 2, cy + 46, 11, col=M.VERMIL)
    s.text(x + w / 2, cy + 84, "no shared referent", 15, M.INK_SOFT,
           font=M.MONO, anchor="middle", italic=True)


def art_ground(s: Svg, x, y, w, h):
    cx, cy = x + w / 2, y + h / 2 - 6
    s.lotus(cx, cy, 44)
    s.fish(cx - 86, cy + 4, 19, col=M.INDIGO, angle=0.10)
    s.leaf(cx + 86, cy + 2, 19, col=M.LEAF, angle=-0.5)
    s.seal(cx, cy + 74, 9, fill=M.VERMIL)
    s.text(cx + 20, cy + 80, "signed", 15, M.INK_SOFT, font=M.MONO)


def art_token(s: Svg, x, y, w, h):
    cy = y + h / 2
    x0, x1 = x + 26, x + w - 34
    # the token as one unbroken thread of beads, ending in the seal
    s.line(x0, cy, x1 - 16, cy, M.INK, 1.6, op=0.75)
    n = 9
    for i in range(n):
        t = x0 + (x1 - 16 - x0) * i / (n - 1)
        s.dot(t, cy, 3.4, M.DYES[i % len(M.DYES)], op=0.95)
    s.seal(x1, cy, 10, fill=M.LEAF)
    s.text(x + w / 2, cy - 36, "one line, not a payload", 15, M.INK_SOFT,
           font=M.MONO, anchor="middle", italic=True)
    s.text(x + w / 2, cy + 46, "resolves, or it does not", 15, M.INK_SOFT,
           font=M.MONO, anchor="middle", italic=True)


def art_handoff(s: Svg, x, y, w, h):
    cy = y + h / 2 - 4
    ax, bx = x + 44, x + w - 44
    s.roundel(ax, cy, 27, M.TEAL, inner="fish", signed=True)
    s.roundel(bx, cy, 27, M.TURMERIC, inner="ring", signed=True)
    s.qpath((ax + 31, cy), ((ax + bx) / 2, cy - 34), (bx - 31, cy),
            M.LEAF, 5.0, op=0.20, glow=True)
    s.qpath((ax + 31, cy), ((ax + bx) / 2, cy - 34), (bx - 31, cy),
            M.LEAF, 2.0, op=0.85)
    s.seal((ax + bx) / 2, cy - 30, 10, fill=M.VERMIL)
    s.text(x + w / 2, cy + 62, "same bytes, no shared trust", 15, M.INK_SOFT,
           font=M.MONO, anchor="middle", italic=True)


def art_guard(s: Svg, x, y, w, h):
    cx, cy = x + w / 2, y + h / 2 - 20
    s.sun(cx, cy, 34, col=M.TURMERIC)
    for i, (col, lab) in enumerate([(M.VERMIL, "SIG"), (M.INDIGO, "BYTES"),
                                    (M.LAC, "DRIFT")]):
        dx = cx + (i - 1) * 74
        s.dot(dx, cy + 62, 6.5, col)
        s.text(dx, cy + 86, lab, 14, M.INK_SOFT, font=M.MONO, anchor="middle")


def art_limits(s: Svg, x, y, w, h):
    cx, cy = x + w / 2, y + h / 2 - 6
    s.roundel(cx, cy, 30, M.VERMIL, inner="ring", signed=True)
    # one responder: the ring around it is drawn open, not closed
    for k in range(14):
        a0 = math.radians(k * 25.7)
        a1 = a0 + math.radians(13)
        r = 58
        s.qpath((cx + r * math.cos(a0), cy + r * math.sin(a0)),
                (cx + (r + 4) * math.cos((a0 + a1) / 2), cy + (r + 4) * math.sin((a0 + a1) / 2)),
                (cx + r * math.cos(a1), cy + r * math.sin(a1)), M.INK_SOFT, 1.3, op=0.5)
    s.text(cx, cy + 88, "stated, not hidden", 15, M.INK_SOFT, font=M.MONO,
           anchor="middle", italic=True)


ART = {"drift": art_drift, "ground": art_ground, "token": art_token,
       "handoff": art_handoff, "guard": art_guard, "limits": art_limits}


def panel_frame(s: Svg, x, y, w, h):
    """A panel edge in the same hand as the page border, one octave quieter."""
    s.rect(x, y, w, h, fill=M.PAPER2, op=0.34, rx=10)
    s.rect(x, y, w, h, stroke=M.INK, sw=1.8, rx=10)
    s.rect(x + 4, y + 4, w - 8, h - 8, stroke=M.INK, sw=0.7, rx=8, op=0.5)
    for (cx, cy, a0) in [(x + 4, y + 4, 0), (x + w - 4, y + 4, 90),
                         (x + w - 4, y + h - 4, 180), (x + 4, y + h - 4, 270)]:
        s.circle(cx, cy, 5.5, stroke=M.INK, sw=1.2, op=0.8)
        s.dot(cx, cy, 2.0, M.TURMERIC, op=0.9)
        for k in range(3):
            a = math.radians(a0 + 22 + k * 22)
            s.qpath((cx + 6 * math.cos(a), cy + 6 * math.sin(a)),
                    (cx + 13 * math.cos(a), cy + 13 * math.sin(a)),
                    (cx + 6 * math.cos(a + 0.30), cy + 6 * math.sin(a + 0.30)),
                    M.INK, 0.9, op=0.55)


def build() -> Svg:
    n = len(PANELS)
    H = HEAD_TOP + HEAD_H + n * PH + (n - 1) * GAP + FOOT_H + BOTTOM
    s = Svg(W, H, seed=5, fscale=1.0)
    s.paper()
    s.border()

    # ---- header: wordmark and the one-line claim ----
    # The tagline sits BELOW the wordmark rather than beside it. Placing them
    # on one line made the layout depend on guessing a proportional font's
    # width, and the guess was wrong by 60px, which is how they came to overlap.
    s.text(MX + 2, HEAD_TOP + 52, "emem", 54, M.INK, font=M.SERIF, weight="bold")
    s.text(MX + 4, HEAD_TOP + 92,
           fits("mono", "shared, verifiable memory for AI agents and machines",
                19, PW - 20, "header tagline"),
           19, M.INK_SOFT, font=M.MONO)
    s.frieze(HEAD_TOP + HEAD_H - 14, MX, W - MX, col=M.TURMERIC)

    y = HEAD_TOP + HEAD_H
    for p in PANELS:
        panel_frame(s, MX, y, PW, PH)

        # panel number, in its own roundel at the text column's head
        s.circle(MX + TEXT_X - 6, y + 40, 15, fill=M.PAPER, stroke=M.INK, sw=1.5)
        s.text(MX + TEXT_X - 6, y + 45, p["n"], 17, M.VERMIL, font=M.MONO,
               anchor="middle", weight="bold")

        tx = MX + TEXT_X + 22
        tw = TEXT_W - 22
        title_is_mono = p.get("mono_title_line") is not None
        s.text(tx, y + 46,
               fits("serif", p["title"], 25, tw, f"panel {p['n']} title"),
               25, M.INK, font=M.SERIF, weight="bold")

        by = y + 84
        for i, line in enumerate(p["body"]):
            mono_code = title_is_mono and i == 0
            size = 19
            s.text(tx, by,
                   fits("mono", line, size, tw, f"panel {p['n']} body{i}"),
                   size, M.VERMIL if mono_code else M.INK_SOFT,
                   font=M.MONO, weight="bold" if mono_code else "normal")
            by += 30

        ART[p["art"]](s, MX + ART_X, y + 12, ART_W, PH - 24)
        y += PH + GAP

    # ---- footer ----
    s.seal(MX + 10, y + 20, 9, fill=M.LEAF)
    s.text(MX + 30, y + 26,
           fits("mono", "emem.dev   ·   MCP + REST   ·   no key to read", 18,
                PW - 60, "footer"),
           18, M.INK, font=M.MONO)
    return s


def main() -> int:
    s = build()
    if VIOLATIONS:
        print("layout violations (nothing written):")
        for v in VIOLATIONS:
            print(f"  x {v}")
        return 1
    s.save("web/emem-strip.svg")
    print(f"wrote web/emem-strip.svg  ({s.W}x{s.H})")
    import cairosvg
    cairosvg.svg2png(url="web/emem-strip.svg", write_to="web/emem-strip.png",
                     output_width=s.W, output_height=s.H)
    print("wrote web/emem-strip.png")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
