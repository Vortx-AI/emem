#!/usr/bin/env python3
"""EUDR supply-chain diagram in the engram's ink-on-paper Mithila hand.

A geolocated plot is checked against the forest baseline and the Hansen
loss-year, signed into one Due Diligence Statement, and that signed handle
clears customs. Same kit as the engram, architecture, and federation.

Output: docs/diagrams/eudr.png
"""
import math, os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from _mithila import (Canvas, INK, INK_SOFT, TURMERIC, VERMIL, INDIGO, LAC,
                      LEAF, TEAL, GOLD_PALE, PAPER, FONT_SERIF, FONT_MONO)

c = Canvas(1600, 1040, seed=3)
W, H = c.W, c.H
c.double_border()

f_title = c.font(FONT_SERIF, 30)
f_sub = c.font(FONT_MONO, 12)
f_lab = c.font(FONT_MONO, 12)
f_tiny = c.font(FONT_MONO, 10)

c.text(56, 50, "EUDR: a plot, proven deforestation-free", f_title, INK)
c.text(58, 86, "forest baseline + Hansen loss-year against the 2020 cut-off, signed into one handle that clears customs",
       f_sub, INK_SOFT)

MID = 470  # vertical centre of the narrative band

def field(cx, cy, w, h, col, cleared=False):
    """A geolocated plot: bordered patch with crop rows + a location pin."""
    c.di.rounded_rectangle([c.S(cx - w / 2), c.S(cy - h / 2), c.S(cx + w / 2), c.S(cy + h / 2)],
                           radius=c.S(6), outline=c.rgba(INK, 255),
                           fill=c.rgba(col, 60), width=max(1, c.S(1.6)))
    for k in range(1, 5):  # crop rows
        x = cx - w / 2 + k * w / 5
        c.line((x, cy - h / 2 + 5), (x, cy + h / 2 - 5), col if not cleared else VERMIL, 1.0, a=150)
    c.dot_ring(cx, cy, max(w, h) * 0.66, 0, 0, INK)  # no-op guard
    # dotted border beads
    n = int(w / 9)
    for k in range(n):
        x = cx - w / 2 + (k + 0.5) * w / n
        c.dot(x, cy - h / 2, 1.1, INK, a=160); c.dot(x, cy + h / 2, 1.1, INK, a=160)
    # location pin
    px, py = cx + w / 2 - 8, cy - h / 2 - 2
    c.polyline(c.bez((px, py - 14), (px - 9, py - 6), (px, py), 14), INK, 1.4)
    c.polyline(c.bez((px, py - 14), (px + 9, py - 6), (px, py), 14), INK, 1.4)
    c.circle(px, py - 9, 3.4, outline=INK, fill=VERMIL, w=1.0)

def tree(cx, cy, col=LEAF, cleared=False):
    """Forest baseline motif: trunk + dotted canopy. Vermillion stump if cleared."""
    if cleared:
        c.line((cx, cy), (cx, cy - 10), INK_SOFT, 2.2)
        c.line((cx - 6, cy - 10), (cx + 6, cy - 6), VERMIL, 1.6)  # cut mark
        return
    c.line((cx, cy), (cx, cy - 18), INK, 2.0)
    for r, n in ((16, 10), (11, 7)):
        c.dot_ring(cx, cy - 26, r, n, 2.2, col)
    c.circle(cx, cy - 26, 18, outline=INK, w=1.2)

# ----------------------------------------------------------------- plots
PX = 250
plots = [(MID - 150, INDIGO, False), (MID, TEAL, False), (MID + 150, LEAF, False)]
c.text(PX, MID - 250, "plots", f_lab, INK, center=True)
c.text(PX, MID - 233, "6-decimal geolocation", f_tiny, INK_SOFT, center=True)
for (py, col, cleared) in plots:
    field(PX, py, 150, 84, col, cleared)
# two baselines feeding the check
c.text(PX, MID + 200, "JRC GFC2020 · Hansen v1.12", f_tiny, INK_SOFT, center=True)
tree(PX - 46, MID + 250); tree(PX + 4, MID + 252, col=TEAL); tree(PX + 52, MID + 250, cleared=True)

# ----------------------------------------------------------------- centre: the verdict (signed DDS)
VX, VY = 800, MID
# plots -> verdict
for (py, col, _) in plots:
    pts = c.bez((PX + 80, py), ((PX + VX) / 2, (py + VY) / 2), (VX - 96, VY), 44)
    c.polyline(pts, INK_SOFT, 1.1, a=160)
    for k in range(4, len(pts), 9):
        c.dot(pts[k][0], pts[k][1], 1.2, INK_SOFT, a=160)
c.lotus(VX, VY, 78)
c.text(VX, VY + 100, "Due Diligence Statement", f_sub, INK, center=True)
c.text(VX, VY + 117, "one signed 26-char handle", f_lab, INK_SOFT, center=True)
c.seal(VX + 66, VY - 66, 11, fill=LEAF)  # PASS seal

# cut-off timeline beneath the verdict
TY = MID + 250
tx0, tx1 = VX - 200, VX + 200
c.line((tx0, TY), (tx1, TY), INK, 1.4)
years = [2018, 2020, 2022, 2024]
for yr in years:
    x = tx0 + (yr - 2017) / (2025 - 2017) * (tx1 - tx0)
    c.line((x, TY - 4), (x, TY + 4), INK, 1.2)
    c.text(x, TY + 9, str(yr), f_tiny, INK_SOFT, center=True)
# cut-off flag at 2020-12-31
cx_cut = tx0 + (2021.0 - 2017) / (2025 - 2017) * (tx1 - tx0)
c.line((cx_cut, TY), (cx_cut, TY - 40), VERMIL, 1.6)
c.di.polygon([(c.S(cx_cut), c.S(TY - 40)), (c.S(cx_cut + 22), c.S(TY - 34)), (c.S(cx_cut), c.S(TY - 28))],
             fill=c.rgba(VERMIL, 255), outline=c.rgba(INK, 255))
c.text(cx_cut, TY - 56, "2020-12-31", f_tiny, VERMIL, center=True)
# loss-year histogram (signed) as small bars to the right of the cut-off
hb = [3, 5, 2, 4, 1]
for i, v in enumerate(hb):
    x = cx_cut + 50 + i * 16
    c.di.rectangle([c.S(x), c.S(TY - 6 - v * 6), c.S(x + 9), c.S(TY - 6)],
                   fill=c.rgba(TURMERIC, 220), outline=c.rgba(INK, 255), width=max(1, c.S(0.8)))
c.text(cx_cut + 90, TY - 52, "loss-year histogram", f_tiny, INK_SOFT, center=True)

# ----------------------------------------------------------------- right: customs gate (TRACES NT)
GX, GY = 1320, MID
# signed CID crossing from verdict to customs (green, glowing)
pts = c.bez((VX + 96, VY), ((VX + GX) / 2, VY - 30), (GX - 70, GY), 50)
c.polyline(pts, LEAF, 6.0, a=52, d=c.dg)
c.polyline(pts, LEAF, 1.6, a=210)
for k in range(0, len(pts), 8):
    c.dot(pts[k][0], pts[k][1], 1.5, LEAF, a=190)
c.seal((VX + GX) / 2, VY - 34, 10, fill=GOLD_PALE)
c.text((VX + GX) / 2, VY - 58, "proof of origin", f_tiny, INK, center=True)
c.text((VX + GX) / 2, VY - 44, "in one CID", f_tiny, INK_SOFT, center=True)

# torana gate
gw, gh = 150, 200
for sx in (GX - gw / 2, GX + gw / 2):
    c.di.rectangle([c.S(sx - 12), c.S(GY - gh / 2), c.S(sx + 12), c.S(GY + gh / 2)],
                   outline=c.rgba(INK, 255), fill=c.rgba(INDIGO, 55), width=max(1, c.S(1.6)))
    for k in range(6):
        yy = GY - gh / 2 + 16 + k * (gh - 32) / 5
        c.dot(sx, yy, 1.6, TURMERIC)
# lintel/arch
c.polyline(c.bez((GX - gw / 2 - 12, GY - gh / 2), (GX, GY - gh / 2 - 40), (GX + gw / 2 + 12, GY - gh / 2), 40),
           INK, 1.8)
c.dot_ring(GX, GY - gh / 2 - 6, 0, 0, 0, INK)
for k in range(7):  # arch dots
    t = math.pi * (0.15 + 0.7 * k / 6)
    c.dot(GX - 86 * math.cos(t), (GY - gh / 2) - 30 * math.sin(t), 1.8, TURMERIC)
# EU ring of stars on the arch
c.dot_ring(GX, GY - gh / 2 - 18, 22, 8, 2.2, GOLD_PALE)
c.text(GX, GY - gh / 2 - 56, "TRACES NT · EU customs", f_lab, INK, center=True)
c.text(GX, GY + gh / 2 + 16, "the handle clears the border", f_tiny, INK_SOFT, center=True)

# ----------------------------------------------------------------- legend
ly = H - 60
lx = 70
c.seal(lx + 7, ly, 7, fill=LEAF); c.text(lx + 18, ly - 7, "pass (deforestation-free)", f_lab, INK); lx += 18 + c.textw("pass (deforestation-free)", f_lab) + 28
c.line((lx, ly - 8), (lx, ly + 8), VERMIL, 1.6); c.text(lx + 8, ly - 7, "2020 cut-off", f_lab, INK); lx += 8 + c.textw("2020 cut-off", f_lab) + 28
c.di.rectangle([c.S(lx), c.S(ly - 8), c.S(lx + 9), c.S(ly)], fill=c.rgba(TURMERIC, 220), outline=c.rgba(INK, 255)); c.text(lx + 16, ly - 7, "signed loss-year histogram", f_lab, INK); lx += 16 + c.textw("signed loss-year histogram", f_lab) + 28
c.line((lx, ly), (lx + 26, ly), LEAF, 1.6); c.text(lx + 32, ly - 7, "signed CID crosses customs", f_lab, INK)

print("eudr.png", c.save("docs/diagrams/eudr.png"))
