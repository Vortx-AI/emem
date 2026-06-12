#!/usr/bin/env python3
"""Federation diagram: many independent responders, one address space.

Each responder signs under its own key and resolves the same content id
byte-for-byte. Nodes cross-cite each other, one pair disagrees, and clients
verify offline. Same ink-on-paper Mithila hand as the engram.

Output: docs/diagrams/federation.png
"""
import math, os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from _mithila import (Canvas, INK, INK_SOFT, TURMERIC, VERMIL, INDIGO, LAC,
                      LEAF, TEAL, GOLD_PALE, PAPER, DYES, FONT_SERIF, FONT_MONO)

c = Canvas(1600, 1040, seed=11)
W, H = c.W, c.H
CX, CY = W * 0.5, H * 0.52
c.double_border()

f_title = c.font(FONT_SERIF, 30)
f_sub = c.font(FONT_MONO, 12)
f_lab = c.font(FONT_MONO, 11)
f_tiny = c.font(FONT_MONO, 10)

c.text(56, 50, "One memory, many responders", f_title, INK)
c.text(58, 86, "every node signs under its own key · every node resolves the same id", f_sub, INK_SOFT)

# ---- responder ring
N = 8
R = 332
nodes = []
pubs = ["7f2a", "b41c", "9de0", "2c63", "e0a7", "55b9", "c1f4", "8a3d"]
for i in range(N):
    th = math.radians(-90 + i * 360 / N)
    x, y = CX + R * math.cos(th), CY + R * math.sin(th) * 0.82
    nodes.append((x, y, DYES[i % len(DYES)]))

# resolve-the-same-id pathways (under), node -> centre
for (x, y, col) in nodes:
    ctrl = ((x + CX) / 2 + (CY - y) * 0.08, (y + CY) / 2 - (CX - x) * 0.08)
    pts = c.bez((x, y), ctrl, (CX, CY), 50)
    c.polyline(pts, LEAF, 6.0, a=46, d=c.dg)
    c.polyline(pts, LEAF, 1.2, a=150)
    for k in range(0, len(pts), 10):
        c.dot(pts[k][0], pts[k][1], 1.2, LEAF, a=150)

# cross-citation arcs along the ring (ink, beaded); one pair disagrees
for i in range(N):
    a = nodes[i]; b = nodes[(i + 1) % N]
    mx, my = (a[0] + b[0]) / 2, (a[1] + b[1]) / 2
    # bow the arc outward from centre
    ox, oy = mx - CX, my - CY
    ol = math.hypot(ox, oy) or 1
    ctrl = (mx + ox / ol * 46, my + oy / ol * 46)
    pts = c.bez((a[0], a[1]), ctrl, (b[0], b[1]), 40)
    if i == 5:  # disagrees_with
        c.polyline(pts, VERMIL, 1.7, a=210)
        c.polyline([(p[0] + 1.8, p[1] + 1.8) for p in pts], VERMIL, 1.0, a=120)
        # contradiction glyph at the apex
        ax, ay = ctrl
        c.line((ax - 7, ay - 7), (ax + 7, ay + 7), VERMIL, 1.6)
        c.line((ax - 7, ay + 7), (ax + 7, ay - 7), VERMIL, 1.6)
        c.circle(ax, ay, 11, outline=INK, w=1.2, fill=PAPER, a=235)
        c.line((ax - 6, ay - 3), (ax + 6, ay - 3), VERMIL, 1.4)
        c.line((ax - 6, ay + 3), (ax + 6, ay + 3), VERMIL, 1.4)
    else:
        c.polyline(pts, INK_SOFT, 1.1, a=160)
        for k in range(6, len(pts), 8):
            c.dot(pts[k][0], pts[k][1], 1.2, INK_SOFT, a=170)

# a couple of long cross-cites across the ring
for (i, j) in [(0, 3), (2, 6)]:
    a, b = nodes[i], nodes[j]
    mx, my = (a[0] + b[0]) / 2, (a[1] + b[1]) / 2
    ctrl = (mx + (CY - my) * 0.06, my + (mx - CX) * 0.0)
    pts = c.bez((a[0], a[1]), ctrl, (b[0], b[1]), 44)
    c.polyline(pts, INK_SOFT, 0.9, a=110)

# ---- centre: the shared content id
c.lotus(CX, CY, 64)
c.text(CX, CY + 90, "one content id", f_sub, INK, center=True)
c.text(CX, CY + 107, "resolved byte-for-byte", f_lab, INK_SOFT, center=True)

# ---- responder nodes (over the lines)
for i, (x, y, col) in enumerate(nodes):
    c.roundel(x, y, 34, col, signed=True, petal=(i % 2 == 0))
    # label: responder + short pubkey
    lx = x
    c.text(lx, y + 44, "responder", f_tiny, INK, center=True)
    c.text(lx, y + 58, "key:" + pubs[i], f_tiny, INK_SOFT, center=True)

# ---- clients verifying offline (3 of them, outside the ring)
def client(cx, cy, near):
    # an eye-in-roundel = verify
    c.circle(cx, cy, 17, outline=INK, fill=PAPER, w=1.5)
    c.circle(cx, cy, 17 * 0.92, outline=INK, w=0.8, a=120)
    # eye
    c.polyline(c.bez((cx - 11, cy), (cx, cy - 8), (cx + 11, cy), 16), INK, 1.3)
    c.polyline(c.bez((cx - 11, cy), (cx, cy + 8), (cx + 11, cy), 16), INK, 1.3)
    c.circle(cx, cy, 4.2, outline=INK, fill=LEAF, w=1.0)
    c.dot(cx, cy, 1.4, PAPER)
    # dotted verify line to the nearest node, ending in a check seal
    nx, ny, _ = near
    pts = c.bez((cx, cy), ((cx + nx) / 2, (cy + ny) / 2 - 16), (nx, ny), 30)
    for k in range(0, len(pts), 4):
        c.dot(pts[k][0], pts[k][1], 1.2, LEAF, a=200)

client(CX - R - 116, CY - 150, nodes[3])
client(CX + R + 116, CY - 150, nodes[7])
client(CX + R + 116, CY + 150, nodes[6])
c.text(CX - R - 116, CY - 150 - 34, "client", f_tiny, INK, center=True)
c.text(CX - R - 116, CY - 150 + 30, "verifies offline", f_tiny, INK_SOFT, center=True)

# ---- legend
ly = H - 64
lx = 70
c.circle(lx, ly, 8, outline=INK, fill=INDIGO, w=1.4); c.seal(lx + 7, ly - 7, 4.6)
c.text(lx + 18, ly - 7, "responder (its own key)", f_lab, INK); lx += 18 + c.textw("responder (its own key)", f_lab) + 30
c.line((lx, ly), (lx + 26, ly), LEAF, 1.4); c.text(lx + 32, ly - 7, "resolves same id", f_lab, INK); lx += 32 + c.textw("resolves same id", f_lab) + 28
c.line((lx, ly), (lx + 26, ly), INK_SOFT, 1.1); c.text(lx + 32, ly - 7, "cites", f_lab, INK); lx += 32 + c.textw("cites", f_lab) + 28
c.line((lx, ly), (lx + 26, ly), VERMIL, 1.6); c.text(lx + 32, ly - 7, "disagrees", f_lab, INK); lx += 32 + c.textw("disagrees", f_lab) + 28
c.circle(lx + 7, ly, 8, outline=INK, fill=PAPER, w=1.3); c.circle(lx + 7, ly, 3.4, outline=INK, fill=LEAF, w=0.8)
c.text(lx + 20, ly - 7, "client verifies offline", f_lab, INK)

print("federation.png", c.save("docs/diagrams/federation.png"))
