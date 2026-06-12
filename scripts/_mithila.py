"""Shared ink-on-warm-paper Mithila drawing kit for emem's README diagrams.

One palette, one border, one set of motif helpers, so every generated diagram
(engram, federation, architecture) reads as the same hand. Supersampled for
clean antialiasing; glow via a blurred under-layer.
"""
import math, random
from PIL import Image, ImageDraw, ImageFont, ImageFilter

SS = 4

PAPER   = (243, 238, 226)
PAPER2  = (235, 228, 211)
INK     = (43, 38, 32)
INK_SOFT = (92, 83, 70)
TURMERIC = (213, 154, 42)
VERMIL  = (178, 58, 44)
INDIGO  = (47, 94, 134)
LAC     = (110, 74, 147)
LEAF    = (79, 138, 61)
TEAL    = (46, 140, 132)
GOLD_PALE = (232, 200, 121)
DYES = [INDIGO, LEAF, TEAL, TURMERIC, LAC, VERMIL]

_F = "/usr/share/fonts/truetype/dejavu/"
FONT_SERIF = _F + "DejaVuSerif.ttf"
FONT_SERIF_IT = _F + "DejaVuSerif-Italic.ttf"
FONT_MONO = _F + "DejaVuSansMono.ttf"


def lum(c):
    return 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]


class Canvas:
    def __init__(self, W, H, seed=7):
        self.W, self.H = W, H
        self.CW, self.CH = W * SS, H * SS
        random.seed(seed)
        self.base = Image.new("RGB", (self.CW, self.CH), PAPER)
        self.glow = Image.new("RGBA", (self.CW, self.CH), (0, 0, 0, 0))
        self.ink = Image.new("RGBA", (self.CW, self.CH), (0, 0, 0, 0))
        self.db = ImageDraw.Draw(self.base)
        self.dg = ImageDraw.Draw(self.glow)
        self.di = ImageDraw.Draw(self.ink)
        self._fonts = {}
        self.paper_base()

    # ---- primitives -------------------------------------------------
    def S(self, v):
        return int(round(v * SS))

    def rgba(self, c, a):
        return (c[0], c[1], c[2], a)

    def font(self, path, px):
        key = (path, px)
        if key not in self._fonts:
            self._fonts[key] = ImageFont.truetype(path, px * SS)
        return self._fonts[key]

    def line(self, p0, p1, col, w, a=255, d=None):
        (d or self.di).line([self.S(p0[0]), self.S(p0[1]), self.S(p1[0]), self.S(p1[1])],
                            fill=self.rgba(col, a), width=max(1, self.S(w)))

    def circle(self, cx, cy, r, outline=None, fill=None, w=1, a=255, d=None):
        d = d or self.di
        bb = [self.S(cx - r), self.S(cy - r), self.S(cx + r), self.S(cy + r)]
        d.ellipse(bb, outline=self.rgba(outline, a) if outline else None,
                  fill=self.rgba(fill, a) if fill else None, width=max(1, self.S(w)))

    def dot(self, cx, cy, r, col, a=255, d=None):
        (d or self.di).ellipse([self.S(cx - r), self.S(cy - r), self.S(cx + r), self.S(cy + r)],
                               fill=self.rgba(col, a))

    def dot_ring(self, cx, cy, R, n, r, col, phase=0.0, a=255, d=None):
        for k in range(n):
            t = phase + 2 * math.pi * k / n
            self.dot(cx + R * math.cos(t), cy + R * math.sin(t), r, col, a, d)

    def bez(self, p0, p1, p2, steps=48):
        pts = []
        for s in range(steps + 1):
            t = s / steps
            u = 1 - t
            pts.append((u * u * p0[0] + 2 * u * t * p1[0] + t * t * p2[0],
                        u * u * p0[1] + 2 * u * t * p1[1] + t * t * p2[1]))
        return pts

    def polyline(self, pts, col, w, a=255, d=None):
        sp = [(self.S(x), self.S(y)) for x, y in pts]
        (d or self.di).line(sp, fill=self.rgba(col, a), width=max(1, self.S(w)), joint="curve")

    def text(self, x, y, s, font, col, a=255, center=False, d=None):
        d = d or self.di
        if center:
            x = x - d.textlength(s, font=font) / SS / 2
        d.text((self.S(x), self.S(y)), s, font=font, fill=self.rgba(col, a))

    def textw(self, s, font):
        return self.di.textlength(s, font=font) / SS

    # ---- composite motifs ------------------------------------------
    def paper_base(self):
        cx, cy = self.W * 0.5, self.H * 0.5
        for i in range(220):
            r = 1 - i / 220.0
            col = tuple(int(PAPER[k] + (PAPER2[k] - PAPER[k]) * (i / 220.0)) for k in range(3))
            self.db.ellipse([self.S(cx) - int(r * self.CW * 0.80), self.S(cy) - int(r * self.CH * 0.98),
                             self.S(cx) + int(r * self.CW * 0.80), self.S(cy) + int(r * self.CH * 0.98)], fill=col)
        for _ in range(1300):
            x, y = random.uniform(0, self.W), random.uniform(0, self.H)
            self.db.point([(self.S(x), self.S(y))], fill=tuple(max(0, PAPER2[k] - 18) for k in range(3)))

    def double_border(self):
        W, H = self.W, self.H
        m1, m2 = 30, 42
        for m, wd in ((m1, 2.2), (m2, 1.4)):
            self.di.rounded_rectangle([self.S(m), self.S(m), self.S(W - m), self.S(H - m)],
                                      radius=self.S(16), outline=self.rgba(INK, 255), width=max(1, self.S(wd)))
        mid = (m1 + m2) / 2
        x = m1 + 26
        while x < W - m1:
            self.dot(x, mid, 1.6, INK); self.dot(x, H - mid, 1.6, INK); x += 26
        y = m1 + 26
        while y < H - m1:
            self.dot(mid, y, 1.6, INK); self.dot(W - mid, y, 1.6, INK); y += 26
        for (cx, cy, a0) in [(m2, m2, 0), (W - m2, m2, 90), (W - m2, H - m2, 180), (m2, H - m2, 270)]:
            for k in range(5):
                ang = math.radians(a0 + 12 + k * 16)
                ex, ey = cx + 30 * math.cos(ang), cy + 30 * math.sin(ang)
                self.line((cx, cy), (ex, ey), INK, 1.2)
                self.dot(ex, ey, 2.0, TURMERIC)
            self.circle(cx, cy, 6, outline=INK, w=1.4); self.dot(cx, cy, 2.4, VERMIL)

    def seal(self, cx, cy, r, fill=VERMIL):
        pts = [(cx + r * math.cos(math.radians(60 * k - 30)), cy + r * math.sin(math.radians(60 * k - 30)))
               for k in range(6)]
        sp = [(self.S(x), self.S(y)) for x, y in pts]
        self.di.polygon(sp, outline=self.rgba(INK, 255), fill=self.rgba(fill, 255))
        self.di.line(sp + [sp[0]], fill=self.rgba(INK, 255), width=max(1, self.S(1.2)))
        self.dot(cx, cy, r * 0.22, GOLD_PALE)

    def roundel(self, cx, cy, r, col, signed=False, petal=False, halo=True):
        if halo:
            self.circle(cx, cy, r * 1.7, fill=col, a=20, d=self.dg)
        self.circle(cx, cy, r, outline=INK, fill=col, w=1.6)
        self.circle(cx, cy, r * 0.66, outline=PAPER, w=1.0, a=210)
        self.dot(cx, cy, max(1.4, r * 0.20), PAPER if lum(col) < 130 else INK)
        self.dot_ring(cx, cy, r * 1.28, max(6, int(r / 2)), 1.4, INK, phase=cx)
        if petal:
            for k in range(8):
                t = 2 * math.pi * k / 8 + cx
                tip = (cx + r * 1.5 * math.cos(t), cy + r * 1.5 * math.sin(t))
                l = (cx + r * math.cos(t - 0.12), cy + r * math.sin(t - 0.12))
                rr = (cx + r * math.cos(t + 0.12), cy + r * math.sin(t + 0.12))
                self.polyline(self.bez(l, tip, rr, 12), INK, 1.0)
        if signed:
            self.seal(cx + r * 0.92, cy - r * 0.92, max(5.2, r * 0.34))

    def lotus(self, cx, cy, R, label_glow=True):
        if label_glow:
            for gr, ga in ((R * 1.9, 24), (R * 1.5, 32), (R * 1.15, 44)):
                self.circle(cx, cy, gr, fill=LEAF, a=ga, d=self.dg)
        self.circle(cx, cy, R * 1.34, fill=PAPER, a=236)
        self.circle(cx, cy, R * 1.30, outline=INK, w=1.2, a=140)
        self.dot_ring(cx, cy, R * 1.30, 36, 1.3, INK, a=120)
        self.circle(cx, cy, R, outline=INK, w=2.2)
        self.circle(cx, cy, R * 0.82, outline=INK, w=1.2)

        def petals(n, r0, r1, phase, col):
            for k in range(n):
                t = phase + 2 * math.pi * k / n
                tip = (cx + r1 * math.cos(t), cy + r1 * math.sin(t))
                l = (cx + r0 * math.cos(t - 0.13), cy + r0 * math.sin(t - 0.13))
                rr = (cx + r0 * math.cos(t + 0.13), cy + r0 * math.sin(t + 0.13))
                self.polyline(self.bez(l, tip, rr, 18), INK, 1.3)
                mid = (cx + (r0 + r1) * 0.5 * math.cos(t), cy + (r0 + r1) * 0.5 * math.sin(t))
                self.dot(mid[0], mid[1], 2.0, col)
        petals(16, R * 0.82, R * 1.16, 0.0, TURMERIC)
        petals(16, R * 0.55, R * 0.82, math.pi / 16, LEAF)
        self.circle(cx, cy, R * 0.5, outline=INK, fill=GOLD_PALE, w=1.4)
        self.circle(cx, cy, R * 0.32, outline=INK, w=1.2)
        self.dot_ring(cx, cy, R * 0.40, 12, 2.2, VERMIL)
        self.dot(cx, cy, R * 0.10, INK)
        self.dot_ring(cx, cy, R * 0.18, 6, 1.8, LEAF)

    def save(self, path, blur=5):
        gb = self.glow.filter(ImageFilter.GaussianBlur(SS * blur))
        out = self.base.convert("RGBA")
        out = Image.alpha_composite(out, gb)
        out = Image.alpha_composite(out, self.ink)
        out = out.convert("RGB").resize((self.W, self.H), Image.LANCZOS)
        out.save(path, optimize=True)
        return out.size
