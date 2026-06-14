"""SVG version of the Mithila ink-on-warm-paper kit, for the served gallery.

Vector, theme-stable, embed-anywhere, so it fits the existing
include_str! SVG pipeline. Same palette as the raster kit (_mithila.py), but
reworked to read as a real Madhubani painting: horror vacui (no dead paper),
bold double/triple outlines, multi-band decorative borders with corner lotus
medallions, big figurative motifs (fish, lotus, peacock, sun, vine), dense
geometric fills, and large readable type in exactly two fonts.
"""
import math, random, html

PAPER = "#f3eee2"; PAPER2 = "#ebe4d3"
INK = "#2b2620"; INK_SOFT = "#5c5346"
TURMERIC = "#d59a2a"; VERMIL = "#b23a2c"; INDIGO = "#2f5e86"
LAC = "#6e4a93"; LEAF = "#4f8a3d"; TEAL = "#2e8c84"; GOLD_PALE = "#e8c879"
DYES = [INDIGO, LEAF, TEAL, TURMERIC, LAC, VERMIL]

# Exactly two fonts everywhere: SERIF for titles only, MONO for all else.
SERIF = "Georgia,'DejaVu Serif',serif"
MONO = "'DejaVu Sans Mono','JetBrains Mono',ui-monospace,monospace"


def esc(s):
    return html.escape(str(s), quote=True)


class Svg:
    def __init__(self, W=1600, H=1040, seed=7, fscale=1.42):
        self.W, self.H = W, H
        random.seed(seed)
        self.glow = []
        self.main = []
        # global font scale: every text() size is multiplied by this, so the
        # whole gallery reads big at gallery size (~+42%).
        self.fscale = fscale
        # margins of the inner art field (inside the border bands)
        self.M = 64

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

    def qpath(self, p0, c, p1, stroke, sw=1, op=1, glow=False, dash=None, cls="", fill="none"):
        d = f'M{p0[0]:.1f},{p0[1]:.1f} Q{c[0]:.1f},{c[1]:.1f} {p1[0]:.1f},{p1[1]:.1f}'
        da = f' stroke-dasharray="{dash}"' if dash else ""
        cc = f' class="{cls}"' if cls else ""
        self._add(f'<path d="{d}" fill="{fill}" stroke="{stroke}" stroke-width="{sw}" '
                  f'opacity="{op}" stroke-linecap="round"{da}{cc}/>', glow)

    def cpath(self, p0, c0, c1, p1, stroke="none", sw=1, op=1, fill="none", close=False, cls=""):
        d = (f'M{p0[0]:.1f},{p0[1]:.1f} C{c0[0]:.1f},{c0[1]:.1f} '
             f'{c1[0]:.1f},{c1[1]:.1f} {p1[0]:.1f},{p1[1]:.1f}')
        if close:
            d += " Z"
        cc = f' class="{cls}"' if cls else ""
        self._add(f'<path d="{d}" fill="{fill}" stroke="{stroke}" stroke-width="{sw}" '
                  f'opacity="{op}" stroke-linejoin="round" stroke-linecap="round"{cc}/>')

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
        sz = size * self.fscale
        self._add(f'<text x="{x:.1f}" y="{y:.1f}" font-family="{font}" font-size="{sz:.1f}" '
                  f'fill="{fill}" text-anchor="{anchor}" opacity="{op}" font-weight="{weight}"{st}>{esc(s)}</text>')

    # ================================================================
    #  GROUND — flood the canvas so no paper sits dead (horror vacui)
    # ================================================================
    def paper(self):
        self.main.insert(0, f'<rect x="0" y="0" width="{self.W}" height="{self.H}" fill="url(#paper)"/>')

    def fill_ground(self, avoid=None, top=120):
        """Flood the inner field with a dense Mithila ground: a fine stipple
        grid plus dozens of scattered tiny motifs at low opacity, all behind the
        foreground. `avoid` is a list of (cx, cy, r) keep-clear discs (the busy
        centre, ledgers, etc.); `top` reserves the header band.
        """
        avoid = avoid or []
        W, H = self.W, self.H
        m = self.M + 8
        g = []

        def clear(x, y, pad=0):
            for (ax, ay, ar) in avoid:
                if (x - ax) ** 2 + (y - ay) ** 2 < (ar + pad) ** 2:
                    return False
            return True

        # 1) fine cross-stipple grid — the dotted "rice-paste" ground
        step = 21
        yy = top
        row = 0
        while yy < H - m:
            xx = m + (step / 2 if row % 2 else 0)
            while xx < W - m:
                if clear(xx, yy, 6):
                    g.append(f'<circle cx="{xx:.1f}" cy="{yy:.1f}" r="0.95" fill="{INK}" opacity="0.065"/>')
                xx += step
            yy += step
            row += 1

        # 2) scattered tiny motifs in the open zones (kept low-opacity)
        kinds = ["fish", "leaf", "bud", "ring", "tri", "tri"]
        tries = 0
        placed = 0
        target = int((W * H) / 23000)
        while placed < target and tries < target * 14:
            tries += 1
            x = random.uniform(m + 10, W - m - 10)
            y = random.uniform(top + 6, H - m - 6)
            if not clear(x, y, 26):
                continue
            k = random.choice(kinds)
            col = random.choice(DYES)
            o = 0.17
            if k == "fish":
                self._ground_fish(g, x, y, random.uniform(7, 11), col, o)
            elif k == "leaf":
                self._ground_leaf(g, x, y, random.uniform(7, 11), col, o)
            elif k == "bud":
                self._ground_bud(g, x, y, random.uniform(5, 8), col, o)
            elif k == "ring":
                g.append(f'<circle cx="{x:.1f}" cy="{y:.1f}" r="{random.uniform(4,7):.1f}" '
                         f'fill="none" stroke="{INK}" stroke-width="0.8" opacity="{o}"/>')
                g.append(f'<circle cx="{x:.1f}" cy="{y:.1f}" r="1.6" fill="{col}" opacity="{o+0.04:.2f}"/>')
            else:  # three-dot cluster / tiny triangle of dots
                for dx, dy in ((-3.5, 2.5), (3.5, 2.5), (0, -4)):
                    g.append(f'<circle cx="{x+dx:.1f}" cy="{y+dy:.1f}" r="1.4" fill="{INK}" opacity="{o}"/>')
            placed += 1

        # insert the whole ground just above the paper rect, behind everything
        self.main.insert(1, "".join(g))

    # low-opacity ground glyph builders (write into list, stay behind fg) ----
    @staticmethod
    def _ground_fish(g, x, y, r, col, o):
        a = (x - r, y); b = (x + r, y)
        g.append(f'<path d="M{a[0]:.1f},{a[1]:.1f} Q{x:.1f},{y-r*0.7:.1f} {b[0]:.1f},{b[1]:.1f} '
                 f'Q{x:.1f},{y+r*0.7:.1f} {a[0]:.1f},{a[1]:.1f} Z" fill="{col}" opacity="{o*0.6:.2f}" '
                 f'stroke="{INK}" stroke-width="0.7"/>')
        g.append(f'<path d="M{b[0]:.1f},{b[1]:.1f} l{r*0.5:.1f},{-r*0.5:.1f} l0,{r:.1f} Z" '
                 f'fill="none" stroke="{INK}" stroke-width="0.7" opacity="{o:.2f}"/>')
        g.append(f'<circle cx="{x-r*0.4:.1f}" cy="{y:.1f}" r="0.9" fill="{INK}" opacity="{o+0.05:.2f}"/>')

    @staticmethod
    def _ground_leaf(g, x, y, r, col, o):
        g.append(f'<path d="M{x:.1f},{y+r:.1f} Q{x-r:.1f},{y:.1f} {x:.1f},{y-r:.1f} '
                 f'Q{x+r:.1f},{y:.1f} {x:.1f},{y+r:.1f} Z" fill="{col}" opacity="{o*0.55:.2f}" '
                 f'stroke="{INK}" stroke-width="0.7"/>')
        g.append(f'<line x1="{x:.1f}" y1="{y+r:.1f}" x2="{x:.1f}" y2="{y-r:.1f}" '
                 f'stroke="{INK}" stroke-width="0.6" opacity="{o:.2f}"/>')

    @staticmethod
    def _ground_bud(g, x, y, r, col, o):
        g.append(f'<path d="M{x:.1f},{y+r:.1f} Q{x-r*0.9:.1f},{y-r*0.3:.1f} {x:.1f},{y-r:.1f} '
                 f'Q{x+r*0.9:.1f},{y-r*0.3:.1f} {x:.1f},{y+r:.1f} Z" fill="{col}" '
                 f'opacity="{o*0.6:.2f}" stroke="{INK}" stroke-width="0.7"/>')
        g.append(f'<circle cx="{x:.1f}" cy="{y-r*1.3:.1f}" r="1.4" fill="{TURMERIC}" opacity="{o+0.05:.2f}"/>')

    # ================================================================
    #  BORDER — outer/inner rule + a real motif band + corner medallions
    # ================================================================
    def border(self):
        W, H = self.W, self.H
        o1, o2, ib = 22, 60, 50   # outer rule, motif-band outer, inner rule
        # outer thick + thin frame
        self.rect(o1, o1, W - 2 * o1, H - 2 * o1, stroke=INK, sw=3.0, rx=18)
        self.rect(o1 + 7, o1 + 7, W - 2 * (o1 + 7), H - 2 * (o1 + 7), stroke=INK, sw=1.0, rx=14, op=0.7)
        # inner rule (the art field edge)
        self.rect(ib, ib, W - 2 * ib, H - 2 * ib, stroke=INK, sw=2.0, rx=12)
        self.rect(ib + 5, ib + 5, W - 2 * (ib + 5), H - 2 * (ib + 5), stroke=INK, sw=0.8, rx=10, op=0.55)

        # the motif band lives between o2 and ib; fill it with a running lotus-
        # petal / fish-scale vine in dye colours.
        self._border_band(o2, ib, W, H)

        # dotted rivets on the inner rule
        x = o2 + 14
        while x < W - o2:
            self.dot(x, (o2 + ib) / 2, 1.5, INK, op=0.7)
            self.dot(x, H - (o2 + ib) / 2, 1.5, INK, op=0.7)
            x += 26
        y = o2 + 14
        while y < H - o2:
            self.dot((o2 + ib) / 2, y, 1.5, INK, op=0.7)
            self.dot(W - (o2 + ib) / 2, y, 1.5, INK, op=0.7)
            y += 26

        # ornate corner lotus medallions
        for (cx, cy, a0) in [(ib + 4, ib + 4, 0), (W - ib - 4, ib + 4, 90),
                             (W - ib - 4, H - ib - 4, 180), (ib + 4, H - ib - 4, 270)]:
            self._corner_medallion(cx, cy, a0)

    def _border_band(self, o2, ib, W, H):
        bandc = (o2 + ib) / 2          # band centre offset
        # a continuous running vine along all four sides, with petals + dots
        def run(horizontal, fixed, lo, hi, col):
            step = 30
            t = lo
            i = 0
            while t < hi:
                cx = t if horizontal else fixed
                cy = fixed if horizontal else t
                # alternating lotus-petal arcs above/below the vine line
                up = -1 if i % 2 == 0 else 1
                if horizontal:
                    self.qpath((cx - step * 0.42, cy), (cx, cy + up * 11),
                               (cx + step * 0.42, cy), INK, 1.1, op=0.7)
                    self.dot(cx, cy + up * 7.5, 1.7, col, op=0.85)
                else:
                    self.qpath((cx, cy - step * 0.42), (cx + up * 11, cy),
                               (cx, cy + step * 0.42), INK, 1.1, op=0.7)
                    self.dot(cx + up * 7.5, cy, 1.7, col, op=0.85)
                t += step
                i += 1
        run(True, bandc, o2 + 30, W - o2 - 30, TURMERIC)
        run(True, H - bandc, o2 + 30, W - o2 - 30, LEAF)
        run(False, bandc, o2 + 30, H - o2 - 30, INDIGO)
        run(False, W - bandc, o2 + 30, H - o2 - 30, VERMIL)

    def _corner_medallion(self, cx, cy, a0):
        # a quarter lotus fan filling the corner, bolder than before
        self.circle(cx, cy, 11, stroke=INK, sw=2.0)
        self.circle(cx, cy, 6, stroke=INK, sw=1.0)
        self.dot(cx, cy, 3.0, VERMIL)
        for k in range(7):
            ang = math.radians(a0 + 8 + k * 12.5)
            r0, r1 = 12, 34
            tip = (cx + r1 * math.cos(ang), cy + r1 * math.sin(ang))
            l = (cx + r0 * math.cos(ang - 0.12), cy + r0 * math.sin(ang - 0.12))
            rr = (cx + r0 * math.cos(ang + 0.12), cy + r0 * math.sin(ang + 0.12))
            self.qpath(l, tip, rr, INK, 1.3)
            mc = TURMERIC if k % 2 == 0 else LEAF
            self.dot((cx + (r0 + r1) / 2 * math.cos(ang)),
                     (cy + (r0 + r1) / 2 * math.sin(ang)), 2.0, mc)
        # a small arc tying the fan tips
        self.dot_ring(cx, cy, 34, 9, 1.1, INK, phase=math.radians(a0 + 8), op=0.5)

    # ================================================================
    #  MOTIF LIBRARY — reusable storytelling glyphs
    # ================================================================
    def fish(self, cx, cy, r, col=INDIGO, angle=0.0, op=1.0):
        """A Mithila fish: lens body, triangular tail, eye and scale ticks."""
        ca, sa = math.cos(angle), math.sin(angle)
        def rot(px, py):
            return (cx + px * ca - py * sa, cy + px * sa + py * ca)
        a = rot(-r, 0); b = rot(r, 0)
        top = rot(0, -r * 0.62); bot = rot(0, r * 0.62)
        self._add(f'<path d="M{a[0]:.1f},{a[1]:.1f} Q{top[0]:.1f},{top[1]:.1f} {b[0]:.1f},{b[1]:.1f} '
                  f'Q{bot[0]:.1f},{bot[1]:.1f} {a[0]:.1f},{a[1]:.1f} Z" fill="{col}" opacity="{op:.2f}" '
                  f'stroke="{INK}" stroke-width="1.4"/>')
        # tail
        t1 = rot(r + r * 0.7, -r * 0.55); t2 = rot(r + r * 0.7, r * 0.55)
        self.poly([b, t1, t2], fill=col, stroke=INK, sw=1.2, close=True, op=op)
        # eye + gill + scale ticks
        eye = rot(-r * 0.45, -r * 0.12)
        self.dot(eye[0], eye[1], max(1.3, r * 0.12), PAPER)
        self.dot(eye[0], eye[1], max(0.7, r * 0.06), INK)
        for fx in (-0.15, 0.15, 0.45):
            p0 = rot(r * fx, -r * 0.4); p1 = rot(r * fx, r * 0.4)
            self.line(p0[0], p0[1], p1[0], p1[1], INK, 0.8, op=0.7 * op)

    def leaf(self, cx, cy, r, col=LEAF, angle=0.0, op=1.0):
        """A pointed leaf with a midrib and veins."""
        ca, sa = math.cos(angle), math.sin(angle)
        def rot(px, py):
            return (cx + px * ca - py * sa, cy + px * sa + py * ca)
        tip = rot(0, -r); base = rot(0, r)
        l = rot(-r * 0.62, 0); rr = rot(r * 0.62, 0)
        self.qpath(base, l, tip, INK, 1.4, op=op, fill=col)
        self.qpath(base, rr, tip, INK, 1.4, op=op, fill=col)
        self.line(base[0], base[1], tip[0], tip[1], INK, 1.0, op=0.8 * op)
        for fy in (-0.35, 0.0, 0.35):
            m = rot(0, r * fy)
            vl = rot(-r * 0.34, r * fy + r * 0.12)
            vr = rot(r * 0.34, r * fy + r * 0.12)
            self.line(m[0], m[1], vl[0], vl[1], INK, 0.7, op=0.6 * op)
            self.line(m[0], m[1], vr[0], vr[1], INK, 0.7, op=0.6 * op)

    def bud(self, cx, cy, r, col=VERMIL, op=1.0):
        """A lotus bud / paisley teardrop on a tiny stem."""
        self._add(f'<path d="M{cx:.1f},{cy+r:.1f} Q{cx-r*0.95:.1f},{cy-r*0.2:.1f} {cx:.1f},{cy-r:.1f} '
                  f'Q{cx+r*0.95:.1f},{cy-r*0.2:.1f} {cx:.1f},{cy+r:.1f} Z" fill="{col}" '
                  f'opacity="{op:.2f}" stroke="{INK}" stroke-width="1.3"/>')
        self.dot(cx, cy + r * 0.1, r * 0.22, GOLD_PALE, op=op)
        self.dot(cx, cy - r * 1.25, max(1.4, r * 0.22), LEAF, op=op)

    def sun(self, cx, cy, r, col=TURMERIC, op=1.0):
        """A Mithila sun: ringed face with rays and a dot ground."""
        for gr in (r * 1.7,):
            self.circle(cx, cy, gr, fill=col, op=0.10 * op, glow=True)
        n = 16
        for k in range(n):
            t = 2 * math.pi * k / n
            a = (cx + r * 1.08 * math.cos(t), cy + r * 1.08 * math.sin(t))
            b = (cx + r * 1.5 * math.cos(t), cy + r * 1.5 * math.sin(t))
            self.line(a[0], a[1], b[0], b[1], INK, 1.3, op=op)
            self.dot(b[0], b[1], 1.6, col, op=op)
        self.circle(cx, cy, r, fill=col, stroke=INK, sw=1.8, op=op)
        self.circle(cx, cy, r * 0.66, stroke=PAPER, sw=1.0, op=0.8 * op)
        self.dot_ring(cx, cy, r * 0.44, 8, 1.6, INK, op=op)
        self.dot(cx, cy, r * 0.16, INK, op=op)

    def bird(self, cx, cy, r, col=TEAL, angle=0.0, op=1.0):
        """A small Mithila peacock/bird: body, curved neck, crest, plume tail."""
        ca, sa = math.cos(angle), math.sin(angle)
        def rot(px, py):
            return (cx + px * ca - py * sa, cy + px * sa + py * ca)
        # body (lens), head up-left, tail plume to the right
        body_a = rot(-r * 0.2, r * 0.5); body_b = rot(r * 0.7, -r * 0.1)
        bt = rot(r * 0.25, r * 0.05); bb = rot(r * 0.25, r * 0.7)
        self._add(f'<path d="M{body_a[0]:.1f},{body_a[1]:.1f} Q{bt[0]:.1f},{bt[1]:.1f} {body_b[0]:.1f},{body_b[1]:.1f} '
                  f'Q{bb[0]:.1f},{bb[1]:.1f} {body_a[0]:.1f},{body_a[1]:.1f} Z" fill="{col}" '
                  f'opacity="{op:.2f}" stroke="{INK}" stroke-width="1.4"/>')
        # neck + head
        neck0 = rot(-r * 0.1, r * 0.15); neck1 = rot(-r * 0.55, -r * 0.55)
        nc = rot(-r * 0.55, r * 0.0)
        self.qpath(neck0, nc, neck1, INK, 2.4, op=op)
        self.dot(neck1[0], neck1[1], max(2.2, r * 0.2), col, op=op)
        self.circle(neck1[0], neck1[1], max(2.2, r * 0.2), stroke=INK, sw=1.2, op=op)
        eye = rot(-r * 0.58, -r * 0.58)
        self.dot(eye[0], eye[1], 1.0, INK, op=op)
        # beak + crest dot
        beak = rot(-r * 0.78, -r * 0.62)
        self.line(neck1[0], neck1[1], beak[0], beak[1], TURMERIC, 1.6, op=op)
        crest = rot(-r * 0.55, -r * 0.85)
        self.dot(crest[0], crest[1], 1.6, TURMERIC, op=op)
        # plume tail: three eyed feathers fanning right
        base = rot(r * 0.6, r * 0.1)
        for j, ang in enumerate((-0.5, -0.1, 0.3)):
            tip = rot(r * 1.7, r * 0.1 + r * 1.0 * ang)
            cc = rot(r * 1.1, r * 0.1 + r * 0.7 * ang)
            self.qpath(base, cc, tip, INK, 1.2, op=op)
            self.dot(tip[0], tip[1], max(2.0, r * 0.18), [INDIGO, LEAF, TURMERIC][j], op=op)
            self.circle(tip[0], tip[1], max(2.0, r * 0.18), stroke=INK, sw=1.0, op=op)

    def vine(self, x0, y0, x1, y1, col=LEAF, leaves=True, amp=18, n=4, op=0.85):
        """A serpentine running vine between two points, with leaves and buds."""
        dx, dy = x1 - x0, y1 - y0
        L = math.hypot(dx, dy) or 1
        nx, ny = -dy / L, dx / L
        prev = (x0, y0)
        for k in range(1, n + 1):
            t = k / n
            mx = x0 + dx * (t - 0.5 / n)
            my = y0 + dy * (t - 0.5 / n)
            side = 1 if k % 2 else -1
            ctrl = (mx + nx * amp * side, my + ny * amp * side)
            pt = (x0 + dx * t, y0 + dy * t)
            self.qpath(prev, ctrl, pt, col, 1.6, op=op)
            if leaves:
                la = math.atan2(pt[1] - ctrl[1], pt[0] - ctrl[0]) + math.pi / 2 * side
                self.leaf(ctrl[0], ctrl[1], 9, col=col, angle=la, op=op)
            prev = pt
        # a bud at the far end
        self.dot(x1, y1, 2.2, TURMERIC, op=op)

    # ================================================================
    #  SEALS, ROUNDELS, LOTUS
    # ================================================================
    def seal(self, cx, cy, r, fill=VERMIL):
        pts = [(cx + r * math.cos(math.radians(60 * k - 30)), cy + r * math.sin(math.radians(60 * k - 30)))
               for k in range(6)]
        self.poly(pts, fill=fill, stroke=INK, sw=1.4, close=True)
        self.dot(cx, cy, r * 0.26, GOLD_PALE)

    def roundel(self, cx, cy, r, col, signed=False, petal=False, inner="ring"):
        """A bigger roundel with an internal Mithila pattern instead of a plain
        disc. `inner` in {ring, fish, leaf, hatch}."""
        self.circle(cx, cy, r * 1.7, fill=col, op=0.10, glow=True)
        self.circle(cx, cy, r, fill=col, stroke=INK, sw=2.0)
        self.circle(cx, cy, r * 0.74, stroke=PAPER, sw=1.1, op=0.85)
        light = col not in (INDIGO, LAC, VERMIL, INK)
        rim = INK if light else PAPER
        if inner == "fish":
            self.fish(cx, cy, r * 0.5, col=PAPER, op=0.9)
        elif inner == "leaf":
            self.leaf(cx, cy, r * 0.6, col=PAPER, op=0.55)
        elif inner == "hatch":
            for k in range(-2, 3):
                yy = cy + k * r * 0.28
                hw = math.sqrt(max(0.0, (r * 0.7) ** 2 - (k * r * 0.28) ** 2))
                self.line(cx - hw, yy, cx + hw, yy, rim, 0.8, op=0.7)
        else:  # ring of dots + centre dot
            self.dot_ring(cx, cy, r * 0.5, 8, max(1.2, r * 0.1), rim, op=0.85)
            self.dot(cx, cy, max(1.6, r * 0.22), rim)
        self.dot_ring(cx, cy, r * 1.3, max(8, int(r / 1.7)), 1.5, INK)
        if petal:
            for k in range(10):
                t = 2 * math.pi * k / 10 + cx
                tip = (cx + r * 1.55 * math.cos(t), cy + r * 1.55 * math.sin(t))
                l = (cx + r * 1.02 * math.cos(t - 0.13), cy + r * 1.02 * math.sin(t - 0.13))
                rr = (cx + r * 1.02 * math.cos(t + 0.13), cy + r * 1.02 * math.sin(t + 0.13))
                self.qpath(l, tip, rr, INK, 1.1)
                self.dot(tip[0], tip[1], 1.5, TURMERIC if k % 2 else LEAF)
        if signed:
            self.seal(cx + r * 0.95, cy - r * 0.95, max(6.0, r * 0.36), fill=LEAF)

    def lotus(self, cx, cy, R, pulse=True):
        """A lusher central lotus: more petal layers, a rich seed core, bolder
        outlines, and a ring of buds, all over a clean paper medallion."""
        if pulse:
            for gr in (R * 1.85, R * 1.45, R * 1.12):
                self.circle(cx, cy, gr, fill=LEAF, op=0.07, glow=True, cls="pulse")
        # clean medallion so the lotus reads against any busy ground
        self.circle(cx, cy, R * 1.5, fill=PAPER, op=0.94)
        self.circle(cx, cy, R * 1.46, stroke=INK, sw=1.3, op=0.6)
        self.dot_ring(cx, cy, R * 1.46, 44, 1.4, INK, op=0.5)
        # outer ring of small buds pointing outward
        for k in range(16):
            t = 2 * math.pi * k / 16
            bx, by = cx + R * 1.34 * math.cos(t), cy + R * 1.34 * math.sin(t)
            self.bud(bx, by, 6.5, col=(TURMERIC if k % 2 else VERMIL), op=0.9)

        self.circle(cx, cy, R * 1.02, stroke=INK, sw=2.6)
        self.circle(cx, cy, R * 0.84, stroke=INK, sw=1.3)

        def petals(n, r0, r1, phase, col, spread=0.13):
            for k in range(n):
                t = phase + 2 * math.pi * k / n
                tip = (cx + r1 * math.cos(t), cy + r1 * math.sin(t))
                l = (cx + r0 * math.cos(t - spread), cy + r0 * math.sin(t - spread))
                rr = (cx + r0 * math.cos(t + spread), cy + r0 * math.sin(t + spread))
                self.qpath(l, tip, rr, INK, 1.4)
                mid = (cx + (r0 + r1) * 0.5 * math.cos(t), cy + (r0 + r1) * 0.5 * math.sin(t))
                self.dot(mid[0], mid[1], 2.3, col)
        # three petal layers, offset, in the dye palette
        petals(18, R * 0.84, R * 1.18, 0.0, TURMERIC)
        petals(18, R * 0.6, R * 0.86, math.pi / 18, VERMIL, spread=0.15)
        petals(18, R * 0.4, R * 0.62, 0.0, LEAF, spread=0.17)
        # rich seed core
        self.circle(cx, cy, R * 0.42, fill=GOLD_PALE, stroke=INK, sw=1.6)
        self.circle(cx, cy, R * 0.28, stroke=INK, sw=1.2)
        self.dot_ring(cx, cy, R * 0.35, 14, 2.4, VERMIL)
        self.dot_ring(cx, cy, R * 0.20, 8, 1.8, LEAF)
        self.dot(cx, cy, R * 0.1, INK)

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
                self.dot(px, py, 1.4, col, op=op)

    def frieze(self, y, x0, x1, col=TURMERIC):
        """A header rule with a running lotus-bud vine (used under titles)."""
        self.line(x0, y, x1, y, INK_SOFT, 0.8, op=0.5)
        x = x0
        while x <= x1:
            self.dot(x, y, 1.5, INK, op=0.55); x += 22
        bx = x0 + 70
        i = 0
        while bx < x1 - 40:
            for k in (-1, 0, 1):
                self.qpath((bx, y), (bx + k * 7, y - 14), (bx + k * 12, y - 2), INK, 1.1)
            self.dot(bx, y - 8, 2.0, col if i % 2 == 0 else LEAF, op=0.9)
            bx += 150
            i += 1

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
            '@keyframes empulse{0%,100%{opacity:.35}50%{opacity:1}}'
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
