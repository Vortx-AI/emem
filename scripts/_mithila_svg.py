"""SVG version of the Mithila ink-on-warm-paper kit, for the served gallery.

Vector, theme-stable, embed-anywhere, so it fits the existing
include_str! SVG pipeline. Same palette, border, and motifs as the raster
kit (_mithila.py), so the gallery reads as the README's hand.
"""
import math, random, html

PAPER = "#f3eee2"; PAPER2 = "#ebe4d3"
INK = "#2b2620"; INK_SOFT = "#5c5346"
TURMERIC = "#d59a2a"; VERMIL = "#b23a2c"; INDIGO = "#2f5e86"
LAC = "#6e4a93"; LEAF = "#4f8a3d"; TEAL = "#2e8c84"; GOLD_PALE = "#e8c879"
DYES = [INDIGO, LEAF, TEAL, TURMERIC, LAC, VERMIL]

SERIF = "Georgia,'DejaVu Serif',serif"
MONO = "'DejaVu Sans Mono','JetBrains Mono',ui-monospace,monospace"


def esc(s):
    return html.escape(str(s), quote=True)


class Svg:
    def __init__(self, W=1600, H=1040, seed=7):
        self.W, self.H = W, H
        random.seed(seed)
        self.glow = []
        self.main = []

    def _add(self, s, glow=False):
        (self.glow if glow else self.main).append(s)

    # ---- primitives ------------------------------------------------
    def rect(self, x, y, w, h, fill="none", stroke="none", sw=1, rx=0, op=1, **kw):
        extra = "".join(f' {k.replace("_","-")}="{v}"' for k, v in kw.items())
        self._add(f'<rect x="{x:.1f}" y="{y:.1f}" width="{w:.1f}" height="{h:.1f}" '
                  f'rx="{rx}" fill="{fill}" stroke="{stroke}" stroke-width="{sw}" opacity="{op}"{extra}/>')

    def circle(self, cx, cy, r, fill="none", stroke="none", sw=1, op=1, glow=False, cls=""):
        cc = f' class="{cls}"' if cls else ""
        self._add(f'<circle cx="{cx:.1f}" cy="{cy:.1f}" r="{r:.1f}" fill="{fill}" '
                  f'stroke="{stroke}" stroke-width="{sw}" opacity="{op}"{cc}/>', glow)

    def dot(self, cx, cy, r, fill, op=1, glow=False):
        self.circle(cx, cy, r, fill=fill, op=op, glow=glow)

    def line(self, x0, y0, x1, y1, stroke, sw=1, op=1, glow=False):
        self._add(f'<line x1="{x0:.1f}" y1="{y0:.1f}" x2="{x1:.1f}" y2="{y1:.1f}" '
                  f'stroke="{stroke}" stroke-width="{sw}" opacity="{op}" stroke-linecap="round"/>', glow)

    def qpath(self, p0, c, p1, stroke, sw=1, op=1, glow=False, dash=None, cls=""):
        d = f'M{p0[0]:.1f},{p0[1]:.1f} Q{c[0]:.1f},{c[1]:.1f} {p1[0]:.1f},{p1[1]:.1f}'
        da = f' stroke-dasharray="{dash}"' if dash else ""
        cc = f' class="{cls}"' if cls else ""
        self._add(f'<path d="{d}" fill="none" stroke="{stroke}" stroke-width="{sw}" '
                  f'opacity="{op}" stroke-linecap="round"{da}{cc}/>', glow)

    def poly(self, pts, fill="none", stroke="none", sw=1, op=1, close=False):
        d = "M" + " L".join(f"{x:.1f},{y:.1f}" for x, y in pts) + (" Z" if close else "")
        self._add(f'<path d="{d}" fill="{fill}" stroke="{stroke}" stroke-width="{sw}" '
                  f'opacity="{op}" stroke-linejoin="round" stroke-linecap="round"/>')

    def dot_ring(self, cx, cy, R, n, r, fill, phase=0.0, op=1):
        for k in range(n):
            t = phase + 2 * math.pi * k / n
            self.dot(cx + R * math.cos(t), cy + R * math.sin(t), r, fill, op)

    def text(self, x, y, s, size, fill, font=MONO, anchor="start", op=1, weight="normal", italic=False):
        st = ' font-style="italic"' if italic else ""
        self._add(f'<text x="{x:.1f}" y="{y:.1f}" font-family="{font}" font-size="{size}" '
                  f'fill="{fill}" text-anchor="{anchor}" opacity="{op}" font-weight="{weight}"{st}>{esc(s)}</text>')

    # ---- motifs ----------------------------------------------------
    def paper(self):
        self.main.insert(0, f'<rect x="0" y="0" width="{self.W}" height="{self.H}" fill="url(#paper)"/>')
        # faint mithila ground: scattered ink specks
        g = []
        for _ in range(260):
            x, y = random.uniform(40, self.W - 40), random.uniform(40, self.H - 40)
            g.append(f'<circle cx="{x:.1f}" cy="{y:.1f}" r="0.9" fill="{INK}" opacity="0.05"/>')
        self.main.insert(1, "".join(g))

    def border(self):
        W, H = self.W, self.H
        for m, sw in ((30, 2.2), (42, 1.4)):
            self.rect(m, m, W - 2 * m, H - 2 * m, stroke=INK, sw=sw, rx=16)
        mid = 36
        x = 56
        while x < W - 36:
            self.dot(x, mid, 1.6, INK); self.dot(x, H - mid, 1.6, INK); x += 26
        y = 56
        while y < H - 36:
            self.dot(mid, y, 1.6, INK); self.dot(W - mid, y, 1.6, INK); y += 26
        for (cx, cy, a0) in [(42, 42, 0), (W - 42, 42, 90), (W - 42, H - 42, 180), (42, H - 42, 270)]:
            for k in range(5):
                ang = math.radians(a0 + 12 + k * 16)
                self.line(cx, cy, cx + 30 * math.cos(ang), cy + 30 * math.sin(ang), INK, 1.2)
                self.dot(cx + 30 * math.cos(ang), cy + 30 * math.sin(ang), 2.0, TURMERIC)
            self.circle(cx, cy, 6, stroke=INK, sw=1.4); self.dot(cx, cy, 2.4, VERMIL)

    def seal(self, cx, cy, r, fill=VERMIL):
        pts = [(cx + r * math.cos(math.radians(60 * k - 30)), cy + r * math.sin(math.radians(60 * k - 30)))
               for k in range(6)]
        self.poly(pts, fill=fill, stroke=INK, sw=1.2, close=True)
        self.dot(cx, cy, r * 0.22, GOLD_PALE)

    def roundel(self, cx, cy, r, col, signed=False, petal=False):
        self.circle(cx, cy, r * 1.7, fill=col, op=0.10, glow=True)
        self.circle(cx, cy, r, fill=col, stroke=INK, sw=1.6)
        self.circle(cx, cy, r * 0.66, stroke=PAPER, sw=1.0, op=0.85)
        self.dot(cx, cy, max(1.4, r * 0.2), PAPER if col in (INDIGO, LAC, VERMIL, INK) else INK)
        self.dot_ring(cx, cy, r * 1.28, max(6, int(r / 2)), 1.4, INK)
        if petal:
            for k in range(8):
                t = 2 * math.pi * k / 8 + cx
                tip = (cx + r * 1.5 * math.cos(t), cy + r * 1.5 * math.sin(t))
                l = (cx + r * math.cos(t - 0.12), cy + r * math.sin(t - 0.12))
                rr = (cx + r * math.cos(t + 0.12), cy + r * math.sin(t + 0.12))
                self.qpath(l, tip, rr, INK, 1.0)
        if signed:
            self.seal(cx + r * 0.92, cy - r * 0.92, max(5.2, r * 0.34), fill=LEAF)

    def lotus(self, cx, cy, R, pulse=True):
        if pulse:
            for gr in (R * 1.7, R * 1.3, R * 1.05):
                self.circle(cx, cy, gr, fill=LEAF, op=0.07, glow=True, cls="pulse")
        self.circle(cx, cy, R * 1.34, fill=PAPER, op=0.92)
        self.circle(cx, cy, R * 1.30, stroke=INK, sw=1.2, op=0.55)
        self.dot_ring(cx, cy, R * 1.30, 36, 1.3, INK, op=0.5)
        self.circle(cx, cy, R, stroke=INK, sw=2.2)
        self.circle(cx, cy, R * 0.82, stroke=INK, sw=1.2)

        def petals(n, r0, r1, phase, col):
            for k in range(n):
                t = phase + 2 * math.pi * k / n
                tip = (cx + r1 * math.cos(t), cy + r1 * math.sin(t))
                l = (cx + r0 * math.cos(t - 0.13), cy + r0 * math.sin(t - 0.13))
                rr = (cx + r0 * math.cos(t + 0.13), cy + r0 * math.sin(t + 0.13))
                self.qpath(l, tip, rr, INK, 1.3)
                mid = (cx + (r0 + r1) * 0.5 * math.cos(t), cy + (r0 + r1) * 0.5 * math.sin(t))
                self.dot(mid[0], mid[1], 2.0, col)
        petals(16, R * 0.82, R * 1.16, 0.0, TURMERIC)
        petals(16, R * 0.55, R * 0.82, math.pi / 16, LEAF)
        self.circle(cx, cy, R * 0.5, fill=GOLD_PALE, stroke=INK, sw=1.4)
        self.circle(cx, cy, R * 0.32, stroke=INK, sw=1.2)
        self.dot_ring(cx, cy, R * 0.40, 12, 2.2, VERMIL)
        self.dot(cx, cy, R * 0.10, INK)
        self.dot_ring(cx, cy, R * 0.18, 6, 1.8, LEAF)

    def flow(self, a, b, col, sw=1.2, op=0.9, bow=0.0, beads=True, glow=False, animate=None):
        if animate is None:
            animate = glow            # the signed (glowing) flows carry the motion
        mx, my = (a[0] + b[0]) / 2, (a[1] + b[1]) / 2
        dx, dy = b[0] - a[0], b[1] - a[1]
        nl = math.hypot(dx, dy) or 1
        ctrl = (mx - dy / nl * bow, my + dx / nl * bow)
        if glow:
            self.qpath(a, ctrl, b, col, sw * 3.4, op=0.28, glow=True)
        self.qpath(a, ctrl, b, col, sw, op=op, cls=("flowline" if animate else ""))
        if beads:
            for k in range(1, 6):
                t = k / 6
                u = 1 - t
                px = u * u * a[0] + 2 * u * t * ctrl[0] + t * t * b[0]
                py = u * u * a[1] + 2 * u * t * ctrl[1] + t * t * b[1]
                self.dot(px, py, 1.2, col, op=op)

    # ---- output ----------------------------------------------------
    def svg(self):
        defs = (
            '<defs>'
            f'<radialGradient id="paper" cx="50%" cy="50%" r="72%">'
            f'<stop offset="0%" stop-color="{PAPER}"/><stop offset="100%" stop-color="{PAPER2}"/>'
            '</radialGradient>'
            '<filter id="blur" x="-30%" y="-30%" width="160%" height="160%">'
            '<feGaussianBlur stdDeviation="7"/></filter>'
            '<style>'
            '.flowline{stroke-dasharray:7 11;animation:emflow 1.25s linear infinite}'
            '@keyframes emflow{to{stroke-dashoffset:-18}}'
            '.pulse{animation:empulse 3.4s ease-in-out infinite}'
            '@keyframes empulse{0%,100%{opacity:.45}50%{opacity:1}}'
            '@media(prefers-reduced-motion:reduce){'
            '.flowline{animation:none}.pulse{animation:none}}'
            '</style>'
            '</defs>'
        )
        body = (f'<g filter="url(#blur)">{"".join(self.glow)}</g>' if self.glow else "")
        body += "".join(self.main)
        return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {self.W} {self.H}" '
                f'width="{self.W}" height="{self.H}" role="img">{defs}{body}</svg>')

    def save(self, path):
        with open(path, "w") as f:
            f.write(self.svg())
        return path
