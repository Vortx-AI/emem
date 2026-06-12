#!/usr/bin/env python3
"""Render the emem 'engram of Earth' hero: a neural-memory diagram drawn in
the project's ink-on-warm-paper design language with Mithila mandala motifs.

Cells are engram nodes, edges are synapses (relates / supersedes / disagrees),
and recall pathways converge on a central consolidation lotus. Deterministic:
a fixed seed places the field, so re-running produces the same image.

Output: docs/diagrams/memory-engram.png  (1600x1040, supersampled 4x)
"""
import math, random
from PIL import Image, ImageDraw, ImageFont, ImageFilter

SS = 4
W, H = 1600, 1040
CW, CH = W * SS, H * SS
random.seed(7)

# ---- palette: warm paper + ink + natural-dye accents (turmeric, vermillion,
# indigo, lac, leaf, teal) — the site's oklch hues read as Mithila dyes.
PAPER  = (243, 238, 226)
PAPER2 = (235, 228, 211)
INK    = (43, 38, 32)
INK_SOFT = (92, 83, 70)
TURMERIC = (213, 154, 42)
VERMIL  = (178, 58, 44)
INDIGO  = (47, 94, 134)
LAC     = (110, 74, 147)
LEAF    = (79, 138, 61)
TEAL    = (46, 140, 132)
GOLD_PALE = (232, 200, 121)

FAMILIES = [INDIGO, LEAF, TEAL, TURMERIC, LAC, VERMIL]
FAM_NAME = ["elevation", "forest", "water", "climate", "embedding", "hazard"]

FONT_SERIF = "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf"
FONT_SERIF_IT = "/usr/share/fonts/truetype/dejavu/DejaVuSerif-Italic.ttf"
FONT_MONO  = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"

def font(path, px):
    return ImageFont.truetype(path, px * SS)

def S(v):  # scale a base coordinate to supersampled space
    return int(round(v * SS))

CX, CY = W * 0.5, H * 0.52

# ============================== layers ==============================
base = Image.new("RGB", (CW, CH), PAPER)
glow = Image.new("RGBA", (CW, CH), (0, 0, 0, 0))     # blurred under linework
ink  = Image.new("RGBA", (CW, CH), (0, 0, 0, 0))      # crisp top linework
db = ImageDraw.Draw(base)
dg = ImageDraw.Draw(glow)
di = ImageDraw.Draw(ink)

def rgba(c, a):
    return (c[0], c[1], c[2], a)

# ---- paper vignette + faint speckle so it reads as hand-laid paper
for i in range(220):
    r = (1 - i / 220.0)
    col = tuple(int(PAPER[k] + (PAPER2[k] - PAPER[k]) * (i / 220.0)) for k in range(3))
    db.ellipse([S(CX) - int(r * CW * 0.78), S(CY) - int(r * CH * 0.95),
                S(CX) + int(r * CW * 0.78), S(CY) + int(r * CH * 0.95)], fill=col)
for _ in range(1400):  # paper grain
    x, y = random.uniform(0, W), random.uniform(0, H)
    a = random.randint(4, 12)
    db.point([(S(x), S(y))], fill=tuple(max(0, PAPER2[k] - 18) for k in range(3)))

def line(draw, p0, p1, col, w, a=255):
    draw.line([S(p0[0]), S(p0[1]), S(p1[0]), S(p1[1])], fill=rgba(col, a), width=max(1, S(w)))

def circle(draw, cx, cy, r, outline=None, fill=None, w=1, a=255):
    bb = [S(cx - r), S(cy - r), S(cx + r), S(cy + r)]
    draw.ellipse(bb, outline=rgba(outline, a) if outline else None,
                 fill=rgba(fill, a) if fill else None, width=max(1, S(w)))

def dot(draw, cx, cy, r, col, a=255):
    draw.ellipse([S(cx - r), S(cy - r), S(cx + r), S(cy + r)], fill=rgba(col, a))

def dot_ring(draw, cx, cy, R, n, r, col, phase=0.0, a=255):
    for k in range(n):
        t = phase + 2 * math.pi * k / n
        dot(draw, cx + R * math.cos(t), cy + R * math.sin(t), r, col, a)

def bez(p0, p1, p2, steps=48):
    pts = []
    for s in range(steps + 1):
        t = s / steps
        u = 1 - t
        x = u * u * p0[0] + 2 * u * t * p1[0] + t * t * p2[0]
        y = u * u * p0[1] + 2 * u * t * p1[1] + t * t * p2[1]
        pts.append((x, y))
    return pts

def polyline(draw, pts, col, w, a=255):
    sp = [(S(x), S(y)) for x, y in pts]
    draw.line(sp, fill=rgba(col, a), width=max(1, S(w)), joint="curve")

# ============================== border ==============================
def double_border():
    m1, m2 = 30, 42
    for m, wd in ((m1, 2.2), (m2, 1.4)):
        di.rounded_rectangle([S(m), S(m), S(W - m), S(H - m)], radius=S(16),
                             outline=rgba(INK, 255), width=max(1, S(wd)))
    # dot rivets between the two rules
    step = 26
    mid = (m1 + m2) / 2
    x = m1 + step
    while x < W - m1:
        dot(di, x, mid, 1.6, INK)
        dot(di, x, H - mid, 1.6, INK)
        x += step
    y = m1 + step
    while y < H - m1:
        dot(di, mid, y, 1.6, INK)
        dot(di, W - mid, y, 1.6, INK)
        y += step
    # corner quarter-lotus
    for (cx, cy, a0) in [(m2, m2, 0), (W - m2, m2, 90), (W - m2, H - m2, 180), (m2, m2_h := m2, 0)]:
        pass
    for (cx, cy, a0) in [(m2, m2, 0), (W - m2, m2, 90), (W - m2, H - m2, 180), (m2, H - m2, 270)]:
        for k in range(5):
            ang = math.radians(a0 + 12 + k * 16)
            r = 30
            ex, ey = cx + r * math.cos(ang), cy + r * math.sin(ang)
            line(di, (cx, cy), (ex, ey), INK, 1.2)
            dot(di, ex, ey, 2.0, TURMERIC)
        circle(di, cx, cy, 6, outline=INK, w=1.4)
        dot(di, cx, cy, 2.4, VERMIL)

# ============================== consolidation lotus ==============================
def lotus(cx, cy, R):
    # outer glow
    for gr, ga in ((R * 1.9, 26), (R * 1.5, 34), (R * 1.15, 46)):
        circle(dg, cx, cy, gr, fill=LEAF, a=ga)
    # clean paper medallion so the lotus reads against the recall convergence
    circle(di, cx, cy, R * 1.34, fill=PAPER, a=236)
    circle(di, cx, cy, R * 1.30, outline=INK, w=1.2, a=140)
    dot_ring(di, cx, cy, R * 1.30, 36, 1.3, INK, a=120)
    # rings
    circle(ink_d := di, cx, cy, R, outline=INK, w=2.2)
    circle(di, cx, cy, R * 0.82, outline=INK, w=1.2)
    # 16 petals (two layers, offset) with dot-fill
    def petals(n, r0, r1, phase, col):
        for k in range(n):
            t = phase + 2 * math.pi * k / n
            tip = (cx + r1 * math.cos(t), cy + r1 * math.sin(t))
            l = (cx + r0 * math.cos(t - 0.13), cy + r0 * math.sin(t - 0.13))
            r = (cx + r0 * math.cos(t + 0.13), cy + r0 * math.sin(t + 0.13))
            polyline(di, bez(l, tip, r, 18), INK, 1.3)
            mid = (cx + (r0 + r1) * 0.5 * math.cos(t), cy + (r0 + r1) * 0.5 * math.sin(t))
            dot(di, mid[0], mid[1], 2.0, col)
    petals(16, R * 0.82, R * 1.16, 0.0, TURMERIC)
    petals(16, R * 0.55, R * 0.82, math.pi / 16, LEAF)
    # inner seed core: concentric + dot grid
    circle(di, cx, cy, R * 0.5, outline=INK, fill=GOLD_PALE, w=1.4)
    circle(di, cx, cy, R * 0.32, outline=INK, w=1.2)
    dot_ring(di, cx, cy, R * 0.40, 12, 2.2, VERMIL)
    dot(di, cx, cy, R * 0.10, INK)
    dot_ring(di, cx, cy, R * 0.18, 6, 1.8, LEAF)

# ============================== engram node ==============================
def node(cx, cy, r, col, signed=False, petal=False, fish=False):
    # soft halo
    circle(dg, cx, cy, r * 1.7, fill=col, a=22)
    # double ink ring + dye core
    circle(di, cx, cy, r, outline=INK, fill=col, w=1.6)
    circle(di, cx, cy, r * 0.66, outline=PAPER, w=1.0, a=210)
    dot(di, cx, cy, max(1.4, r * 0.20), PAPER if _lum(col) < 130 else INK)
    # mithila dot ring around the rim
    dot_ring(di, cx, cy, r * 1.28, max(6, int(r / 2)), 1.4, INK, phase=cx)
    if petal:
        for k in range(8):
            t = 2 * math.pi * k / 8 + cx
            tip = (cx + r * 1.5 * math.cos(t), cy + r * 1.5 * math.sin(t))
            l = (cx + r * 1.0 * math.cos(t - 0.12), cy + r * 1.0 * math.sin(t - 0.12))
            rr = (cx + r * 1.0 * math.cos(t + 0.12), cy + r * 1.0 * math.sin(t + 0.12))
            polyline(di, bez(l, tip, rr, 12), INK, 1.0)
    if fish:  # a small mithila fish = a fact/seed inside the cell
        t = cx
        a = (cx + r * 0.5 * math.cos(t), cy + r * 0.5 * math.sin(t))
        b = (cx - r * 0.5 * math.cos(t), cy - r * 0.5 * math.sin(t))
        c1 = (cx + r * 0.5 * math.cos(t + 1.6), cy + r * 0.5 * math.sin(t + 1.6))
        c2 = (cx + r * 0.5 * math.cos(t - 1.6), cy + r * 0.5 * math.sin(t - 1.6))
        polyline(di, bez(a, c1, b, 16), PAPER, 1.0, a=220)
        polyline(di, bez(a, c2, b, 16), PAPER, 1.0, a=220)
    if signed:
        seal(cx + r * 0.95, cy - r * 0.95, 6.2)

def _lum(c):
    return 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]

def seal(cx, cy, r):
    pts = [(cx + r * math.cos(math.radians(60 * k - 30)),
            cy + r * math.sin(math.radians(60 * k - 30))) for k in range(6)]
    sp = [(S(x), S(y)) for x, y in pts]
    di.polygon(sp, outline=rgba(INK, 255), fill=rgba(VERMIL, 255))
    di.line(sp + [sp[0]], fill=rgba(INK, 255), width=max(1, S(1.2)))
    dot(di, cx, cy, 1.4, GOLD_PALE)

# ============================== compose ==============================
double_border()

# concentric memory-field rings (radial lattice the engram orbits) — drawn
# faint, in dotted ink, so the whole field reads as one structure.
for ri, R in enumerate([190, 285, 380, 472, 560]):
    n = int(R / 5.2)
    dot_ring(di, CX, CY, R, n, 1.3, INK, phase=ri * 0.3, a=42)
    dot_ring(di, CX, CY, R, max(8, n // 6), 2.0, TURMERIC, phase=ri * 0.7 + 0.4, a=70)

# phyllotaxis field of cells — spread to fill the frame
GOLD = math.radians(137.507)
nodes = []
N = 80
SQUASH = 0.74
for i in range(1, N + 1):
    rr = 150 + 47.0 * math.sqrt(i)       # spiral radius
    th = i * GOLD
    jx = random.uniform(-9, 9); jy = random.uniform(-9, 9)
    x = CX + rr * math.cos(th) + jx
    y = CY + rr * math.sin(th) * SQUASH + jy
    if 96 < x < W - 96 and 116 < y < H - 120 and rr > 150:
        fam = i % len(FAMILIES)
        rad = random.uniform(13, 25)
        nodes.append((x, y, rad, fam))

# synapse edges: each node to its 3 nearest neighbours (deduped) for a web
def dist(a, b):
    return math.hypot(a[0] - b[0], a[1] - b[1])
edges = set()
for i, a in enumerate(nodes):
    nn = sorted(range(len(nodes)), key=lambda j: dist(a, nodes[j]) if j != i else 1e9)[:3]
    for j in nn:
        edges.add((min(i, j), max(i, j)))
edges = list(edges)

# draw edges first (under nodes). most are 'relates' (ink). a few supersedes
# (lac, double) and disagrees (vermillion, beaded). every 4th node also sends a
# recall pathway to the consolidation lotus (leaf-green, glowing).
for idx, (i, j) in enumerate(edges):
    a, b = nodes[i], nodes[j]
    mx, my = (a[0] + b[0]) / 2, (a[1] + b[1]) / 2
    nx, ny = -(b[1] - a[1]), (b[0] - a[0])
    nlen = math.hypot(nx, ny) or 1
    off = 22 * math.sin(idx)
    ctrl = (mx + nx / nlen * off, my + ny / nlen * off)
    pts = bez((a[0], a[1]), ctrl, (b[0], b[1]), 40)
    kind = idx % 9
    if kind == 0:      # disagrees_with — vermillion, beaded twin line
        polyline(di, pts, VERMIL, 1.5, a=200)
        for k in range(0, len(pts), 6):
            dot(di, pts[k][0], pts[k][1], 1.6, VERMIL, a=220)
    elif kind == 1:    # supersedes — lac, doubled
        polyline(di, pts, LAC, 1.6, a=210)
        polyline(di, [(p[0] + 1.6, p[1] + 1.6) for p in pts], LAC, 0.9, a=120)
    else:              # relates_to — ink hairline with mid bead
        polyline(di, pts, INK_SOFT, 1.0, a=150)
        dot(di, ctrl[0], ctrl[1], 1.3, INK_SOFT, a=160)

# recall pathways to the lotus (glow under), from every 3rd node
for i, a in enumerate(nodes):
    if i % 3 != 0:
        continue
    mx, my = (a[0] + CX) / 2, (a[1] + CY) / 2
    ctrl = (mx + (CY - a[1]) * 0.12, my - (CX - a[0]) * 0.12)
    pts = bez((a[0], a[1]), ctrl, (CX, CY), 60)
    polyline(dg, pts, LEAF, 7.0, a=60)
    polyline(di, pts, LEAF, 1.5, a=200)
    for k in range(0, len(pts), 8):
        dot(di, pts[k][0], pts[k][1], 1.6, LEAF, a=190)

# faint background motifs for mithila density (under everything visually, drawn
# on ink layer at low alpha in the gaps)
for _ in range(120):
    x, y = random.uniform(60, W - 60), random.uniform(96, H - 110)
    if dist((x, y, 0, 0), (CX, CY, 0, 0)) < 150:
        continue
    if any(dist((x, y), (n[0], n[1])) < n[2] * 2.0 for n in nodes):
        continue
    di.ellipse([S(x - 1.1), S(y - 1.1), S(x + 1.1), S(y + 1.1)], fill=rgba(INK, 26))

lotus(CX, CY, 78)

for i, (x, y, r, fam) in enumerate(nodes):
    node(x, y, r, FAMILIES[fam],
         signed=(i % 3 == 0), petal=(i % 5 == 0), fish=(i % 2 == 0))

# ============================== type ==============================
f_title = font(FONT_SERIF, 30)
f_sub   = font(FONT_MONO, 12)
f_lab   = font(FONT_MONO, 11)
f_cap   = font(FONT_SERIF_IT, 14)

di.text((S(56), S(50)), "An engram of Earth", font=f_title, fill=rgba(INK, 255))
di.text((S(58), S(86)), "every cell a memory · every edge a synapse · every fact signed",
        font=f_sub, fill=rgba(INK_SOFT, 255))

# centre label under lotus
clab = "consolidation"
tw = di.textlength(clab, font=f_sub)
di.text((S(CX) - tw / 2, S(CY + 96)), clab, font=f_sub, fill=rgba(INK, 255))
clab2 = "the shared memory"
tw2 = di.textlength(clab2, font=f_lab)
di.text((S(CX) - tw2 / 2, S(CY + 113)), clab2, font=f_lab, fill=rgba(INK_SOFT, 255))

# legend (bottom rule)
ly = H - 64
lx = 70
def legend_dot(x, y, col, label):
    circle(di, x, y, 7, outline=INK, fill=col, w=1.4)
    di.text((S(x + 14), S(y - 7)), label, font=f_lab, fill=rgba(INK, 255))
    return x + 16 + di.textlength(label, font=f_lab) / SS + 30
lx = legend_dot(lx, ly, INDIGO, "cell  (engram)")
# edge legend
line(di, (lx, ly), (lx + 26, ly), INK_SOFT, 1.0); di.text((S(lx + 32), S(ly - 7)), "relates", font=f_lab, fill=rgba(INK,255)); lx += 32 + di.textlength("relates", font=f_lab)/SS + 28
line(di, (lx, ly), (lx + 26, ly), LAC, 1.6); di.text((S(lx + 32), S(ly - 7)), "supersedes", font=f_lab, fill=rgba(INK,255)); lx += 32 + di.textlength("supersedes", font=f_lab)/SS + 28
line(di, (lx, ly), (lx + 26, ly), VERMIL, 1.5); di.text((S(lx + 32), S(ly - 7)), "disagrees", font=f_lab, fill=rgba(INK,255)); lx += 32 + di.textlength("disagrees", font=f_lab)/SS + 28
line(di, (lx, ly), (lx + 26, ly), LEAF, 1.6); di.text((S(lx + 32), S(ly - 7)), "recall → consolidation", font=f_lab, fill=rgba(INK,255)); lx += 32 + di.textlength("recall → consolidation", font=f_lab)/SS + 28
seal(lx + 6, ly, 6.2); di.text((S(lx + 18), S(ly - 7)), "signed", font=f_lab, fill=rgba(INK,255))

# ============================== flatten ==============================
glow_b = glow.filter(ImageFilter.GaussianBlur(SS * 5))
out = base.convert("RGBA")
out = Image.alpha_composite(out, glow_b)
out = Image.alpha_composite(out, ink)
out = out.convert("RGB").resize((W, H), Image.LANCZOS)
out.save("docs/diagrams/memory-engram.png")
print("wrote docs/diagrams/memory-engram.png", out.size)
