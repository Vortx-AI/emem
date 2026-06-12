#!/usr/bin/env python3
"""Architecture diagram in the engram's ink-on-paper Mithila hand.

One binary at the centre. Clients reach it over MCP and REST (same handlers).
Primitives ring the core. Upstream sources feed in from the left, the GPU
sidecar from the right, and every write drops into an append-only signed log
below. Four content-addressed manifests pin what produced each answer.

Output: docs/diagrams/architecture.png
"""
import math, os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from _mithila import (Canvas, INK, INK_SOFT, TURMERIC, VERMIL, INDIGO, LAC,
                      LEAF, TEAL, GOLD_PALE, PAPER, DYES, FONT_SERIF, FONT_MONO)

c = Canvas(1600, 1040, seed=5)
W, H = c.W, c.H
CX, CY = W * 0.5, H * 0.55
c.double_border()

f_title = c.font(FONT_SERIF, 30)
f_sub = c.font(FONT_MONO, 12)
f_lab = c.font(FONT_MONO, 12)
f_tiny = c.font(FONT_MONO, 10)

c.text(56, 50, "One binary, signed end to end", f_title, INK)
c.text(58, 86, "the same handlers answer MCP and REST · reads need no auth · every write is logged", f_sub, INK_SOFT)

def flow(a, b, col, w, a_=170, bow=0.0, beads=True, glow=False):
    mx, my = (a[0] + b[0]) / 2, (a[1] + b[1]) / 2
    dx, dy = b[0] - a[0], b[1] - a[1]
    nl = math.hypot(dx, dy) or 1
    ctrl = (mx - dy / nl * bow, my + dx / nl * bow)
    pts = c.bez(a, ctrl, b, 44)
    if glow:
        c.polyline(pts, col, w * 3.4, a=46, d=c.dg)
    c.polyline(pts, col, w, a=a_)
    if beads:
        for k in range(4, len(pts), 9):
            c.dot(pts[k][0], pts[k][1], 1.2, col, a=a_)
    return pts

# ---- left: 46 upstream sources feed in
SRC_X = 150
src_dyes = [INDIGO, LEAF, TEAL, TURMERIC, LAC, VERMIL, INDIGO, LEAF, TEAL]
for i in range(9):
    y = CY - 200 + i * 50
    col = src_dyes[i]
    c.roundel(SRC_X, y, 12, col, halo=False)
    flow((SRC_X + 16, y), (CX - 150, CY + (y - CY) * 0.25), INK_SOFT, 0.9, a_=120, bow=18 * math.sin(i))
c.text(SRC_X, CY - 240, "46 sources", f_lab, INK, center=True)
c.text(SRC_X, CY + 232, "STAC + COG, signed", f_tiny, INK_SOFT, center=True)
c.text(SRC_X, CY + 247, "on the first miss", f_tiny, INK_SOFT, center=True)

# ---- right: GPU sidecar (4 encoders)
SID_X = 1452
c.circle(SID_X, CY, 34, outline=INK, fill=LAC, w=1.6)
c.circle(SID_X, CY, 34 * 0.66, outline=PAPER, w=1.0, a=210)
c.dot_ring(SID_X, CY, 17, 4, 4.2, GOLD_PALE)
c.dot_ring(SID_X, CY, 34 * 1.28, 8, 1.4, INK, phase=SID_X)
flow((SID_X - 36, CY), (CX + 150, CY), LAC, 1.1, a_=150, bow=-26)
c.text(SID_X, CY - 56, "GPU sidecar", f_lab, INK, center=True)
c.text(SID_X, CY + 50, "Clay · Prithvi", f_tiny, INK_SOFT, center=True)
c.text(SID_X, CY + 64, "Tessera · Galileo", f_tiny, INK_SOFT, center=True)

# ---- top: clients over MCP + REST -> same handlers
CLI_Y = 150
mcp = (CX - 150, CLI_Y)
rest = (CX + 150, CLI_Y)
for (p, label) in [(mcp, "MCP"), (rest, "REST")]:
    c.circle(p[0], p[1], 26, outline=INK, fill=TEAL if label == "MCP" else INDIGO, w=1.6)
    c.circle(p[0], p[1], 26 * 0.66, outline=PAPER, w=1.0, a=210)
    c.dot_ring(p[0], p[1], 26 * 1.28, 8, 1.4, INK, phase=p[0])
    c.text(p[0], p[1] - 4, label, f_tiny, PAPER, center=True)
    flow((p[0], p[1] + 28), (CX, CY - 150), INK_SOFT, 1.2, a_=170, bow=0)
c.text(CX, CLI_Y - 44, "clients · same handlers", f_lab, INK, center=True)

# ---- bottom: append-only signed log (chain of linked leaf-seals)
LOG_Y = CY + 250
n_leaf = 7
xs = [CX - 270 + i * 90 for i in range(n_leaf)]
for i, x in enumerate(xs):
    if i:
        c.line((xs[i - 1] + 16, LOG_Y), (x - 16, LOG_Y), INK, 1.4)
        c.dot((xs[i - 1] + x) / 2, LOG_Y, 1.6, INK)
    c.seal(x, LOG_Y, 15, fill=[INDIGO, LEAF, TEAL, TURMERIC, LAC, VERMIL, GOLD_PALE][i])
    c.circle(x, LOG_Y, 15 * 0.6, outline=PAPER, w=0.8, a=160)
flow((CX, CY + 150), (CX, LOG_Y - 22), LEAF, 1.4, a_=190, bow=0, glow=True)
c.text(CX, LOG_Y + 28, "append-only signed log", f_lab, INK, center=True)
c.text(CX, LOG_Y + 44, "blake3-chained · merkle root per batch", f_tiny, INK_SOFT, center=True)

# ---- inner ring of primitives
prims = ["recall", "find_similar", "verify", "hunt", "eudr_dds", "state"]
pr_dyes = [INDIGO, TEAL, LEAF, TURMERIC, VERMIL, LAC]
RR = 168
for i, name in enumerate(prims):
    th = math.radians(-90 + i * 360 / len(prims))
    x, y = CX + RR * math.cos(th), CY + RR * math.sin(th) * 0.9
    c.line((CX, CY), (x, y), INK_SOFT, 0.8, a=110)
    c.roundel(x, y, 19, pr_dyes[i], signed=(i % 2 == 0))
    lx = x + (26 if math.cos(th) >= 0 else -26)
    c.text(x, y + 30, name, f_tiny, INK, center=True)

# ---- centre: the binary
c.lotus(CX, CY, 70)
c.text(CX, CY + 96, "the responder", f_sub, INK, center=True)
c.text(CX, CY + 113, "one binary", f_lab, INK_SOFT, center=True)

# ---- manifests row under the title (4 content-addressed CIDs)
mlabels = ["bands_cid", "algorithms_cid", "sources_cid", "schema_cid"]
mx0 = W - 470
c.text(mx0 - 14, 52, "manifests:", f_tiny, INK)
for i, m in enumerate(mlabels):
    x = mx0 + 12 + i * 112
    c.seal(x, 74, 8, fill=[INDIGO, LEAF, TURMERIC, LAC][i])
    c.text(x + 12, 68, m, f_tiny, INK_SOFT)

# ---- legend
ly = H - 60
lx = 70
c.line((lx, ly), (lx + 26, ly), INK_SOFT, 1.1)
c.text(lx + 32, ly - 7, "request flow", f_lab, INK); lx += 32 + c.textw("request flow", f_lab) + 30
c.line((lx, ly), (lx + 26, ly), LEAF, 1.4); c.text(lx + 32, ly - 7, "signed write", f_lab, INK); lx += 32 + c.textw("signed write", f_lab) + 30
c.seal(lx + 7, ly, 7); c.text(lx + 18, ly - 7, "signed leaf / manifest", f_lab, INK); lx += 18 + c.textw("signed leaf / manifest", f_lab) + 30
c.circle(lx + 8, ly, 9, outline=INK, fill=TEAL, w=1.3); c.text(lx + 21, ly - 7, "node (client / primitive / source)", f_lab, INK)

print("architecture.png", c.save("docs/diagrams/architecture.png"))
