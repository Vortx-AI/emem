#!/usr/bin/env python3
"""The homepage painting, as a static asset the README can show.

The site draws this with JavaScript: one world, handed to two hands. The left
bank is drawn with Math.random, so it is a different village on every load; the
right bank is drawn from a written seed, so it is the same lattice for ever.
A README cannot run JavaScript, so this script emits the same picture as SVG.

The left side here is jittered from a SEED rather than from Math.random. That
is deliberate and it is the one difference from the live page: a committed file
has to be byte-stable or every run of this script shows up as a diff. The point
the drawing makes is unchanged, because the point is that the two sides are
drawn by different hands, not that this particular file is unpredictable.

Emits a light and a dark variant so the README can pick with <picture> and
prefers-color-scheme, since a fixed palette reads as a pasted-in card on
whichever theme it was not made for.

Usage:
  python3 scripts/gen_river_art.py            # write both variants
  python3 scripts/gen_river_art.py --check    # fail if they are stale
"""
from __future__ import annotations

import argparse
import math
import pathlib
import sys

REPO = pathlib.Path(__file__).resolve().parent.parent
OUT = REPO / "web" / "art"

W, H = 1240, 460
MID = W / 2
GROUND = H * 0.78

THEMES = {
    "light": dict(paper="#faf7f0", ink="#1a1814", mute="#6f6a60",
                  slate="#4e678a", ochre="#9d7420", rule="#d8d2c6"),
    "dark":  dict(paper="#14130f", ink="#f0ece2", mute="#9a9285",
                  slate="#93a9c6", ochre="#dcb161", rule="#33302a"),
}


class Rng:
    """mulberry32, the same generator the page uses, so the right bank here and
    the right bank on the site are the same kind of object, not merely similar."""

    def __init__(self, seed: int):
        self.a = seed & 0xFFFFFFFF

    def __call__(self) -> float:
        self.a = (self.a + 0x6D2B79F5) & 0xFFFFFFFF
        t = self.a
        t = (t ^ (t >> 15)) * (1 | t) & 0xFFFFFFFF
        t = (t + ((t ^ (t >> 7)) * (61 | t) & 0xFFFFFFFF)) & 0xFFFFFFFF ^ t
        return ((t ^ (t >> 14)) & 0xFFFFFFFF) / 4294967296


def world(half_w: float):
    """One scene: hills, a mill, sheds, smoke, trees. Returned as plain
    polylines so both hands are handed exactly the same geometry."""
    r = Rng(20260826)
    forms = []

    def ridge(y, amp):
        pts = [(x, y + math.sin(x / 78 + amp) * amp * 0.5 + math.sin(x / 27 + amp * 2) * 3.5)
               for x in range(0, int(half_w) + 1, 18)]
        forms.append(("ridge", pts))

    def box(x, y, w, h):
        forms.append(("box", [(x, y), (x + w, y), (x + w, y + h), (x, y + h)]))

    ridge(H * 0.30, 26)
    ridge(H * 0.46, 20)
    ridge(GROUND, 10)
    # the mill and its sheds
    for i in range(3):
        cx = 62 + i * 132 + r() * 16
        ch = 118 + r() * 56
        box(cx, GROUND - ch, 15, ch)
        sm = [(cx + 7 + t * 14 + math.sin(t * 1.3) * 9, GROUND - ch - 13 - t * 19) for t in range(6)]
        forms.append(("limb", sm))
    for i in range(2):
        box(30 + i * 196 + r() * 20, GROUND - 42, 78, 42)
    # trees along the near bank
    for i in range(5):
        forms.append(("tree", [(24 + r() * (half_w - 48), GROUND - 2 - r() * 26)]))
    return forms


def free_hand(forms, colour, jitter_seed: int) -> list[str]:
    """The guessing hand: every point nudged, every line drawn twice."""
    r = Rng(jitter_seed)
    out = []

    def wob(p, n):
        return (p[0] + (r() - 0.5) * n, p[1] + (r() - 0.5) * n)

    def d_of(pts, close, n):
        d = "".join(("L" if i else "M") + f"{wob(p, n)[0]:.1f},{wob(p, n)[1]:.1f}"
                    for i, p in enumerate(pts))
        return d + ("Z" if close else "")

    def bough(x, y, ang, ln, depth):
        if depth == 0 or ln < 2.5:
            return
        x2, y2 = x + math.cos(ang) * ln, y + math.sin(ang) * ln
        out.append(f'<line x1="{x:.1f}" y1="{y:.1f}" x2="{x2:.1f}" y2="{y2:.1f}" '
                   f'stroke="{colour}" stroke-width="{max(0.3, depth * 0.26):.2f}" '
                   f'stroke-linecap="round" opacity="0.6"/>')
        for _ in range(2 if r() > 0.2 else 3):
            bough(x2, y2, ang + (r() - 0.5) * 1.0, ln * (0.66 + r() * 0.15), depth - 1)

    for kind, pts in forms:
        if kind == "tree":
            bough(pts[0][0], pts[0][1], -math.pi / 2, 16, 6)
            continue
        close = kind == "box"
        if close:
            out.append(f'<path d="{d_of(pts, True, 4)}" fill="{colour}" opacity="0.055"/>')
        for wdt, op, n in ((1.0, 0.5, 4), (0.6, 0.28, 6.5)):
            out.append(f'<path d="{d_of(pts, close, n)}" fill="none" stroke="{colour}" '
                       f'stroke-width="{wdt}" stroke-linecap="round" '
                       f'stroke-linejoin="round" opacity="{op}"/>')
    return out


def lattice_hand(forms, colour) -> list[str]:
    """The measuring hand: the same forms, resolved into vertices and edges."""
    r = Rng(20260826)
    out = []
    for kind, pts in forms:
        if kind == "tree":
            x, y = pts[0]
            joints, level = [], [(x, y - 12)]
            joints.append(((x, y), (x, y - 12)))
            for d in range(2):
                nxt = []
                for p in level:
                    for s in (-1, 1):
                        q = (p[0] + s * 7.5 / (d + 1) * 1.6, p[1] - 9 / (d * 0.6 + 1))
                        joints.append((p, q))
                        nxt.append(q)
                level = nxt
            for a, b in joints:
                out.append(f'<line x1="{a[0]:.1f}" y1="{a[1]:.1f}" x2="{b[0]:.1f}" y2="{b[1]:.1f}" '
                           f'stroke="{colour}" stroke-width="0.7" opacity="0.55"/>')
                out.append(f'<circle cx="{b[0]:.1f}" cy="{b[1]:.1f}" r="{1.3 + r():.2f}" '
                           f'fill="{colour}" opacity="0.85"/>')
            continue
        close = kind == "box"
        d = "".join(("L" if i else "M") + f"{p[0]:.1f},{p[1]:.1f}" for i, p in enumerate(pts))
        if close:
            d += "Z"
            out.append(f'<path d="{d}" fill="{colour}" opacity="0.05"/>')
        out.append(f'<path d="{d}" fill="none" stroke="{colour}" stroke-width="0.75" opacity="0.6"/>')
        for i, p in enumerate(pts):
            if kind == "ridge" and i % 2:
                continue
            out.append(f'<circle cx="{p[0]:.1f}" cy="{p[1]:.1f}" r="{1.4 + r() * 1.2:.2f}" '
                       f'fill="{colour}" opacity="0.9"/>')
    return out


def build(theme: str) -> str:
    t = THEMES[theme]
    half = MID - 70
    forms = world(half)
    left = free_hand(forms, t["slate"], 913371)
    right = lattice_hand(forms, t["ochre"])

    plug_x, plug_y = MID, H * 0.52
    p = t["mute"]
    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}" '
        f'role="img" aria-label="Two banks of one river. On the left the same village, mill and '
        f'sheds drawn freehand, every line twice and never in the same place; on the right the '
        f'identical forms resolved into vertices and edges. A jack plug lies in the gap between '
        f'them, labelled call at-emem. Nothing crosses but the plug.">',
        f'<rect width="{W}" height="{H}" fill="{t["paper"]}"/>',
        # the channel: one hairline the two banks approach and never cross
        f'<line x1="{MID}" y1="0" x2="{MID}" y2="{H}" stroke="{t["rule"]}" stroke-width="1" opacity="0.8"/>',
        # left bank, mirrored so it faces the channel the way the page does
        f'<g transform="translate({half + 8},0) scale(-1,1)">', *left, "</g>",
        f'<g transform="translate({MID + 62},0)">', *right, "</g>",
        # the labels
        f'<text x="42" y="52" font-family="Georgia,serif" font-size="30" fill="{t["ink"]}">LLM</text>',
        f'<text x="42" y="72" font-family="ui-monospace,monospace" font-size="11" '
        f'letter-spacing="1.4" fill="{t["mute"]}">non-deterministic</text>',
        f'<text x="{W - 42}" y="52" text-anchor="end" font-family="Georgia,serif" font-size="30" '
        f'fill="{t["ink"]}">emem</text>',
        f'<text x="{W - 42}" y="72" text-anchor="end" font-family="ui-monospace,monospace" '
        f'font-size="11" letter-spacing="1.4" fill="{t["mute"]}">deterministic</text>',
        # the plug, the only thing in the channel
        f'<g opacity="0.95">',
        f'<line x1="{plug_x - 118}" y1="{plug_y}" x2="{plug_x - 44}" y2="{plug_y}" stroke="{p}" '
        f'stroke-width="1" stroke-dasharray="2 4" opacity="0.7"/>',
        f'<rect x="{plug_x - 44}" y="{plug_y - 5}" width="34" height="10" rx="2.5" fill="{p}" opacity="0.55"/>',
        f'<rect x="{plug_x - 10}" y="{plug_y - 10}" width="40" height="20" rx="4.5" fill="{p}" opacity="0.8"/>',
        f'<circle cx="{plug_x + 20}" cy="{plug_y}" r="5" fill="{t["paper"]}" opacity="0.9"/>',
        f'<line x1="{plug_x + 30}" y1="{plug_y}" x2="{plug_x + 118}" y2="{plug_y}" stroke="{p}" '
        f'stroke-width="1" stroke-dasharray="2 4" opacity="0.7"/>',
        "</g>",
        f'<rect x="{MID - 64}" y="{plug_y + 24}" width="128" height="26" rx="13" fill="{t["paper"]}" '
        f'stroke="{t["rule"]}" stroke-width="1"/>',
        f'<text x="{MID}" y="{plug_y + 41}" text-anchor="middle" font-family="ui-monospace,monospace" '
        f'font-size="12" fill="{t["ink"]}">call @emem</text>',
        f'<text x="{MID}" y="{H - 22}" text-anchor="middle" font-family="Georgia,serif" '
        f'font-size="15" fill="{t["mute"]}">Two ways to know where something is. Only one of them repeats.</text>',
        "</svg>",
    ]
    return "\n".join(parts) + "\n"


# ── Story diagrams ──────────────────────────────────────────────────────────
# The same two-bank language as the hero: cream ground, a hairline channel, the
# guessing hand on the left and the measuring hand on the right, and as few
# words as the picture can carry. These replace the Mithila-styled art, which
# was beautiful and said something else.

SW, SH = 1100, 380


def _svg_open(t, label):
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {SW} {SH}" '
            f'width="{SW}" height="{SH}" role="img" aria-label="{label}">'
            f'<rect width="{SW}" height="{SH}" fill="{t["paper"]}"/>')


def _channel(t):
    return (f'<line x1="{SW/2}" y1="28" x2="{SW/2}" y2="{SH-28}" stroke="{t["rule"]}" '
            f'stroke-width="1" opacity="0.85"/>')


def _cap(t, x, y, s, anchor_="middle", size=13, mono=True, fill=None):
    fam = "ui-monospace,monospace" if mono else "Georgia,serif"
    return (f'<text x="{x}" y="{y}" text-anchor="{anchor_}" font-family="{fam}" '
            f'font-size="{size}" fill="{fill or t["mute"]}">{s}</text>')


def story_one_address(t) -> str:
    """Many ways of saying a place on the left; one address on the right."""
    r = Rng(4242)
    p = [_svg_open(t, "On the left, five different phrasings of the same place, each drawn "
                      "loosely and none matching another. On the right, one address, drawn "
                      "once as a node with edges. A dotted line runs from the five to the one."),
         _channel(t)]
    words = ["\u201cthe old mill\u201d", "\u201cby the river\u201d", "\u201cMill Lane\u201d",
             "\u201cnear the bridge\u201d", "\u201cthat field\u201d"]
    for i, w in enumerate(words):
        y = 96 + i * 42
        wob = (r() - 0.5) * 16
        p.append(_cap(t, 300 + wob, y, w, "middle", 15, False, t["slate"]))
        p.append(f'<path d="M{430 + wob},{y - 5} C {SW/2 - 40},{y - 5} {SW/2 - 20},190 {SW/2 - 6},190" '
                 f'fill="none" stroke="{t["slate"]}" stroke-width="1" opacity="0.35"/>')
    cx, cy = SW / 2 + 150, 190
    for a in range(6):
        import math as _m
        ex, ey = cx + _m.cos(a * 1.047) * 74, cy + _m.sin(a * 1.047) * 58
        p.append(f'<line x1="{cx}" y1="{cy}" x2="{ex:.1f}" y2="{ey:.1f}" stroke="{t["ochre"]}" '
                 f'stroke-width="1.1" opacity="0.55"/>')
        p.append(f'<circle cx="{ex:.1f}" cy="{ey:.1f}" r="3.4" fill="{t["ochre"]}" opacity="0.8"/>')
    p.append(f'<circle cx="{cx}" cy="{cy}" r="9" fill="{t["ochre"]}"/>')
    p.append(_cap(t, cx, cy + 96, "one address", "middle", 14, True, t["ink"]))
    p.append(_cap(t, cx, cy + 116, "every agent resolves to it identically", "middle", 12))
    p.append(_cap(t, 300, 64, "five ways to say it", "middle", 14, True, t["ink"]))
    p.append("</svg>")
    return "\n".join(p) + "\n"


def story_token_crosses(t) -> str:
    """Two agents that share nothing, and the one thing that passes between."""
    p = [_svg_open(t, "Two agents facing a single signed record between them. A short token "
                      "passes from one to the other along a dotted line; no line runs directly "
                      "between the two agents."),
         _channel(t)]

    def figure(x, colour, name):
        return "".join([
            f'<rect x="{x-30}" y="150" width="60" height="76" rx="3" fill="none" '
            f'stroke="{colour}" stroke-width="2.2" opacity="0.85"/>',
            f'<rect x="{x-16}" y="108" width="32" height="34" rx="3" fill="none" '
            f'stroke="{colour}" stroke-width="2.2" opacity="0.85"/>',
            f'<line x1="{x}" y1="108" x2="{x}" y2="92" stroke="{colour}" stroke-width="2.2" opacity="0.85"/>',
            f'<circle cx="{x}" cy="88" r="4" fill="{colour}" opacity="0.85"/>',
            _cap(t, x, 258, name, "middle", 14, True, colour),
        ])
    p.append(figure(190, t["slate"], "agent A"))
    p.append(figure(SW - 190, t["ochre"], "agent B"))
    p.append(f'<line x1="228" y1="188" x2="{SW-228}" y2="188" stroke="{t["mute"]}" '
             f'stroke-width="1" stroke-dasharray="3 6" opacity="0.55"/>')
    p.append(f'<rect x="{SW/2-176}" y="172" width="352" height="32" rx="16" fill="{t["paper"]}" '
             f'stroke="{t["ink"]}" stroke-width="1.2"/>')
    p.append(_cap(t, SW/2, 193, "emem:fact:\u2026", "middle", 14, True, t["ink"]))
    p.append(_cap(t, SW/2, 300, "the only thing that crosses", "middle", 14, True, t["ink"]))
    p.append(_cap(t, SW/2, 322, "each checks it alone, neither has to trust the other", "middle", 12))
    p.append("</svg>")
    return "\n".join(p) + "\n"


# The six-panel explainer, in the two-banks language.
#
# It replaces a Madhubani strip that carried the same six panels. That drawing
# was beautiful and it was also the last place on the front page still speaking
# a different visual language from everything around it -- and its companion
# had shipped with the caption printed over itself, two lines of type occupying
# one line of space, which is what happens when text is placed by hand at a
# fixed y and the string later grows.
#
# So the type here is laid out from a measured line height rather than from
# chosen coordinates: every body line is `y0 + i * LEAD`, and a panel that
# gains a line pushes its own baseline instead of landing on the previous one.
PANELS = [
    ("1", "Two agents describe one field",
     ["One reports 0.62. One reports \u201clooks healthy\u201d.",
      "Neither can check the other, and neither",
      "names the same ground the same way twice."],
     "no shared referent"),
    ("2", "One address, one signed fact",
     ["The place resolves to a cell64 every agent",
      "derives identically. The reading becomes a",
      "fact: blake3 over canonical CBOR, ed25519."],
     "signed"),
    ("3", "The fact collapses to one line",
     ["emem:fact:<cell64>:<fact_cid>",
      "The cid is the full 32-byte digest, 52",
      "characters, never truncated."],
     "one line, not a payload"),
    ("4", "Anyone resolves it, anyone checks it",
     ["Hand the token to another model or vendor",
      "and it returns the byte-identical body, or",
      "it does not resolve. No key, no callback."],
     "same bytes, no shared trust"),
    ("5", "emem-guard checks before you assert",
     ["PROV_SIG: the signature fails.",
      "PROV_BYTES: it resolves to other bytes.",
      "PROV_DRIFT: it moved past the threshold."],
     "checked, not assumed"),
    ("6", "What it does not do",
     ["One responder signs it, not a consensus.",
      "A real citation can sit on a wrong claim.",
      "Only emem:fact: binds a whole body."],
     "stated, not hidden"),
]

PW, PH, LEAD = 500, 132, 19


def story_six_panels(t) -> str:
    w, h = 1100, 3 * PH + 96
    out = [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" '
           f'width="{w}" height="{h}" role="img" aria-label="Six panels: two agents '
           f'describe one field and cannot check each other; one address and one signed '
           f'fact; the fact collapses to one line; anyone resolves and checks it; '
           f'emem-guard checks before an agent asserts; and what emem does not do.">'
           f'<rect width="{w}" height="{h}" fill="{t["paper"]}"/>']
    out.append(f'<text x="40" y="46" font-family="Georgia,serif" font-size="26" '
               f'fill="{t["ink"]}">emem</text>')
    out.append(f'<text x="118" y="46" font-family="ui-monospace,monospace" font-size="13" '
               f'fill="{t["mute"]}">shared, verifiable memory for AI agents and machines</text>')
    out.append(f'<line x1="40" y1="62" x2="{w-40}" y2="62" stroke="{t["rule"]}" stroke-width="1"/>')

    for i, (num, title, body, foot) in enumerate(PANELS):
        col, row = i % 2, i // 2
        x = 40 + col * (PW + 20)
        y = 84 + row * PH
        accent = t["slate"] if col == 0 else t["ochre"]
        out.append(f'<line x1="{x}" y1="{y}" x2="{x}" y2="{y + PH - 26}" '
                   f'stroke="{accent}" stroke-width="2" opacity="0.55"/>')
        out.append(f'<text x="{x + 14}" y="{y + 14}" font-family="ui-monospace,monospace" '
                   f'font-size="12" fill="{accent}">{num}</text>')
        out.append(f'<text x="{x + 34}" y="{y + 15}" font-family="Georgia,serif" '
                   f'font-size="16" fill="{t["ink"]}">{title}</text>')
        for j, line in enumerate(body):
            out.append(f'<text x="{x + 34}" y="{y + 40 + j * LEAD}" '
                       f'font-family="ui-monospace,monospace" font-size="12.5" '
                       f'fill="{t["mute"]}">{line}</text>')
        fy = y + 40 + len(body) * LEAD + 6
        out.append(f'<text x="{x + 34}" y="{fy}" font-family="Georgia,serif" '
                   f'font-size="12" font-style="italic" fill="{accent}">{foot}</text>')

    out.append(f'<line x1="40" y1="{h-40}" x2="{w-40}" y2="{h-40}" '
               f'stroke="{t["rule"]}" stroke-width="1"/>')
    out.append(f'<text x="40" y="{h-20}" font-family="ui-monospace,monospace" '
               f'font-size="12" fill="{t["mute"]}">emem.dev  ·  MCP + REST  ·  no key to read</text>')
    out.append("</svg>")
    return "\n".join(out) + "\n"


STORIES = {"one-address": story_one_address,
           "token-crosses": story_token_crosses,
           "six-panels": story_six_panels}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--check", action="store_true")
    a = ap.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    stale = []
    jobs = [(f"two-banks-{th}.svg", (lambda th=th: build(th))) for th in THEMES]
    for name, fn in STORIES.items():
        for th in THEMES:
            jobs.append((f"{name}-{th}.svg", (lambda fn=fn, th=th: fn(THEMES[th]))))
    for fname, make in jobs:
        path = OUT / fname
        body = make()
        if a.check:
            if not path.exists() or path.read_text(encoding="utf-8") != body:
                stale.append(str(path.relative_to(REPO)))
        else:
            path.write_text(body, encoding="utf-8")
            print(f"wrote {path.relative_to(REPO)} ({len(body)} bytes)")
    if a.check:
        if stale:
            print("stale, run scripts/gen_river_art.py:")
            for f in stale:
                print("  ", f)
            return 1
        print("river art matches its generator")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
