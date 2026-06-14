#!/usr/bin/env python3
"""Connect & evolve, drawn in the Mithila hand.

Replaces the plain box-and-arrow schematic that used to sit inline on
/how-it-works. Same information — three attesters at one (cell, band, tslot),
two that corroborate and one outlier, folding into a signed bundle while the
disagreement is recorded as a scored contradiction, and a refinement loop that
re-attests — but told with roundels, a lotus bundle, a vermilion contradiction
seal, and a running vine, matching the rest of the gallery.

Writes web/art/connect-evolve.svg (served at /art/connect-evolve.svg).
"""
import os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from _mithila_svg import (Svg, INK, INK_SOFT, TURMERIC, VERMIL, INDIGO, LAC,
                          LEAF, TEAL, GOLD_PALE, PAPER, DYES, SERIF, MONO)

OUT = "web/art/connect-evolve.svg"


def scene():
    s = Svg(1600, 1040, seed=11)
    W, H = s.W, s.H
    s.paper()

    # anchors
    ax = 340                                   # attester column
    a_ys = [372, 568, 764]
    bundle = (812, 470)                        # CONNECT · signed bundle (lotus)
    contra = (1118, 690)                       # RECORD · contradiction (seal)
    refine = (1342, 470)                       # REFINE roundel

    avoid = [
        (ax, a_ys[0], 150), (ax, a_ys[1], 150), (ax, a_ys[2], 150),
        (bundle[0], bundle[1], 200),
        (contra[0], contra[1], 150),
        (refine[0], refine[1], 150),
        (W / 2, 150, 560), (W / 2, H - 96, 560),
    ]
    s.fill_ground(avoid=avoid, top=176)
    s.border()

    # header
    s.text(78, 108, "connect & evolve", 30, INK, font=SERIF)
    s.text(80, 142, "facts at one cell  ·  typed edges  ·  recorded disagreement  ·  refinement", 13, INK_SOFT, font=MONO)
    s.frieze(168, 78, W - 78)

    # storytelling motifs in the open zones
    s.sun(bundle[0], 250, 28)
    s.bird(bundle[0] - 250, 262, 38, col=TEAL, angle=0.1)
    s.bird(bundle[0] + 250, 262, 38, col=LAC, angle=-0.1)
    s.fish(150, H - 150, 26, col=INDIGO, angle=-0.15)
    s.fish(208, H - 116, 20, col=TEAL, angle=0.12)
    s.fish(W - 180, H - 168, 24, col=VERMIL, angle=0.1)
    s.fish(W - 120, H - 196, 18, col=TURMERIC, angle=-0.1)

    # ---- three attesters at one (cell, band, tslot) ----------------------
    s.text(ax, a_ys[0] - 118, "FACTS · one (cell, band, tslot)", 13, INK_SOFT, font=MONO, anchor="middle")
    att = [
        ("attester A", "NDVI 0.61", "tc54…aeesh", TEAL, "ring", False),
        ("attester B", "NDVI 0.58", "wbqy…6m5q", INDIGO, "leaf", False),
        ("attester C", "NDVI 0.19", "cxji…h5bq", LAC, "fish", True),
    ]
    for (name, val, cid, col, inner, outlier), y in zip(att, a_ys):
        s.roundel(ax, y, 40, col, inner=inner, signed=not outlier)
        s.text(ax, y + 70, name, 15, INK, font=MONO, anchor="middle", weight="bold")
        s.text(ax, y + 90, val, 12.5, col if not outlier else VERMIL, font=MONO, anchor="middle", weight="bold")
        tail = "  · outlier" if outlier else ""
        s.text(ax, y + 108, "fact_cid " + cid + tail, 11,
               (VERMIL if outlier else INK_SOFT), font=MONO, anchor="middle")

    # ---- edges: A,B relate into the bundle; C disagrees into the seal ----
    s.flow((ax + 44, a_ys[0]), (bundle[0] - 96, bundle[1] - 18), LEAF, sw=1.6, bow=34, glow=True)
    s.flow((ax + 44, a_ys[1]), (bundle[0] - 96, bundle[1] + 18), LEAF, sw=1.6, bow=-22, glow=True)
    s.text((ax + bundle[0]) / 2, a_ys[0] - 18, "relates_to", 12, LEAF, font=MONO, anchor="middle", weight="bold")
    s.flow((ax + 44, a_ys[2]), (contra[0] - 70, contra[1] - 8), VERMIL, sw=1.7, bow=70)
    s.text((ax + contra[0]) / 2 - 30, a_ys[2] + 44, "disagrees_with", 12, VERMIL, font=MONO, anchor="middle", weight="bold")

    # ---- CONNECT · the signed bundle (lotus medallion) -------------------
    s.lotus(bundle[0], bundle[1], 96)
    s.text(bundle[0], bundle[1] - 150, "CONNECT · memory_bundle", 15, INK, font=MONO, anchor="middle", weight="bold")
    s.text(bundle[0], bundle[1] + 168, "memb:<bundle_cid> · one signed envelope", 12, INK_SOFT, font=MONO, anchor="middle")

    # ---- RECORD · contradiction (vermilion seal, both cids kept) ---------
    s.seal(contra[0], contra[1], 40, fill=VERMIL)
    s.dot_ring(contra[0], contra[1], 56, 12, 1.6, INK, op=0.6)
    s.text(contra[0], contra[1] + 78, "RECORD · contradiction", 14, INK, font=MONO, anchor="middle", weight="bold")
    s.text(contra[0], contra[1] + 98, "severity 0.71 · both fact_cids kept", 11.5, VERMIL, font=MONO, anchor="middle")
    s.text(contra[0], contra[1] + 116, "/v1/memory_contradictions", 11, INK_SOFT, font=MONO, anchor="middle")

    # bundle + contradiction feed REFINE
    s.flow((bundle[0] + 96, bundle[1]), (refine[0] - 42, refine[1] - 6), INDIGO, sw=1.4, bow=-18)
    s.flow((contra[0] + 40, contra[1] - 20), (refine[0] - 20, refine[1] + 40), INDIGO, sw=1.3, bow=40)

    # ---- REFINE roundel + the refinement vine loop back to the attesters --
    s.roundel(refine[0], refine[1], 38, TURMERIC, inner="hatch")
    s.text(refine[0], refine[1] + 70, "REFINE", 15, INK, font=MONO, anchor="middle", weight="bold")
    s.text(refine[0], refine[1] + 90, "flag re-attestation · /v1/reviews", 11, INK_SOFT, font=MONO, anchor="middle")
    s.text(refine[0], refine[1] + 108, "supersede via as_of_signed_at", 11, INK_SOFT, font=MONO, anchor="middle")
    # a long serpentine vine curling from REFINE back down to the attesters
    s.vine(refine[0] - 8, refine[1] + 130, ax + 40, a_ys[2] + 150, col=LEAF, amp=46, n=7, op=0.85)
    s.text((refine[0] + ax) / 2, H - 150, "re-attestation appends a new fact; the old bytes stay replayable",
           12, INK_SOFT, font=MONO, anchor="middle", italic=True)

    # closing line
    s.text(W / 2, H - 64, "nothing is overwritten in the dark · every edge is signed and queryable",
           12.5, INK, font=SERIF, anchor="middle", italic=True)
    return s


def main():
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    scene().save(OUT)
    print("wrote", OUT)


if __name__ == "__main__":
    main()
