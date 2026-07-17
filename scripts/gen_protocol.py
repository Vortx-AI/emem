#!/usr/bin/env python3
"""Protocol gallery diagrams in the Mithila SVG hand: architecture, the
decentralised federation, and the trust plane (the v1 receipt preimage).

Vector SVG into docs/diagrams/, matching the README's architecture.png /
federation.png / engram. PNG previews under /tmp.

Usage: python3 scripts/gen_protocol.py [name ...]
"""
import math, os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from _mithila_svg import (Svg, INK, INK_SOFT, TURMERIC, VERMIL, INDIGO, LAC,
                          LEAF, TEAL, GOLD_PALE, PAPER, PAPER2, DYES, SERIF, MONO)
OUT = "docs/diagrams"


def architecture():
    s = Svg(1600, 1040, seed=5); W, H = s.W, s.H
    CX, CY = 800, 566
    RR = 244; LR = 96
    SRCX, SIDX = 138, 1462
    LOGY = CY + 300
    avoid = [(CX, CY, RR + 56), (SRCX, CY, 150), (SIDX, CY, 120),
             (CX - 170, 224, 90), (CX + 170, 224, 90), (CX, 224, 130),
             (CX, LOGY, 380), (W / 2, 150, 600)]
    _ground(s, avoid=avoid)
    _header(s, "One binary, signed end to end",
            "the same handlers answer MCP and REST · reads need no auth · every write is logged")

    # left: 46 sources feed in
    for i in range(9):
        y = CY - 210 + i * 52
        s.roundel(SRCX, y, 13, DYES[i % len(DYES)],
                  inner=["ring", "hatch", "leaf"][i % 3])
        s.flow((SRCX + 18, y), (CX - LR - 60, CY + (y - CY) * 0.25), INK_SOFT, 1.0, op=0.5, bow=16 * math.sin(i))
    s.text(SRCX, CY - 250, "46 sources", 13, INK, font=MONO, anchor="middle", weight="bold")
    s.text(SRCX, CY + 268, "STAC + COG, signed", 11, INK_SOFT, font=MONO, anchor="middle")
    s.text(SRCX, CY + 286, "on the first miss", 11, INK_SOFT, font=MONO, anchor="middle")

    # right: GPU sidecar
    s.circle(SIDX, CY, 40, fill=LAC, stroke=INK, sw=2.0)
    s.dot_ring(SIDX, CY, 20, 4, 4.6, GOLD_PALE)
    s.fish(SIDX, CY, 22, col=PAPER, op=0.85)
    s.dot_ring(SIDX, CY, 52, 10, 1.6, INK, phase=SIDX)
    s.flow((SIDX - 44, CY), (CX + LR + 56, CY), LAC, 1.3, op=0.7, bow=-30)
    s.text(SIDX, CY - 64, "GPU sidecar", 13, INK, font=MONO, anchor="middle", weight="bold")
    s.text(SIDX, CY + 70, "Clay · Prithvi", 11, INK_SOFT, font=MONO, anchor="middle")
    s.text(SIDX, CY + 88, "Tessera · Galileo", 11, INK_SOFT, font=MONO, anchor="middle")

    # top: clients
    for (cx, label, col) in [(CX - 170, "MCP", TEAL), (CX + 170, "REST", INDIGO)]:
        s.circle(cx, 224, 30, fill=col, stroke=INK, sw=2.0)
        s.dot_ring(cx, 224, 38, 10, 1.5, INK, phase=cx)
        s.dot_ring(cx, 224, 14, 6, 1.4, PAPER, op=0.8)
        s.text(cx, 230, label, 12, PAPER, font=MONO, anchor="middle", weight="bold")
        s.flow((cx, 256), (CX, CY - LR - 56), INK_SOFT, 1.3, op=0.7)
    s.text(CX, 200, "clients · same handlers", 13, INK, font=MONO, anchor="middle", weight="bold")

    # bottom: append-only signed log
    xs = [CX - 270 + i * 90 for i in range(7)]
    for i, x in enumerate(xs):
        if i:
            s.line(xs[i - 1] + 18, LOGY, x - 18, LOGY, INK, 1.6)
            s.dot((xs[i - 1] + x) / 2, LOGY, 1.8, INK)
        s.seal(x, LOGY, 17, fill=[INDIGO, LEAF, TEAL, TURMERIC, LAC, VERMIL, GOLD_PALE][i])
    s.flow((CX, CY + LR + 56), (CX, LOGY - 26), LEAF, 1.6, op=0.9, glow=True)
    s.text(CX, LOGY + 38, "append-only signed log", 13, INK, font=MONO, anchor="middle", weight="bold")
    s.text(CX, LOGY + 58, "blake3-chained · merkle root per batch", 11, INK_SOFT, font=MONO, anchor="middle")

    # inner ring of primitives
    prims = [("recall", INDIGO), ("find_similar", TEAL), ("verify", LEAF),
             ("hunt", TURMERIC), ("eudr_dds", VERMIL), ("state", LAC)]
    inners = ["fish", "ring", "leaf", "hatch", "ring", "fish"]
    for i, (name, col) in enumerate(prims):
        th = math.radians(-60 + i * 360 / len(prims))   # rotated: top/bottom gaps
        x, y = CX + RR * math.cos(th), CY + RR * math.sin(th) * 0.92
        s.line(CX, CY, x, y, INK_SOFT, 0.9, op=0.4)
        s.roundel(x, y, 23, col, signed=(i % 2 == 0), inner=inners[i])
        ly = y - 38 if math.sin(th) < 0 else y + 42      # label radially outward
        s.text(x, ly, name, 11, INK, font=MONO, anchor="middle", weight="bold")

    s.lotus(CX, CY, LR)
    s.seal(CX + LR * 0.86, CY - LR * 0.86, 11, fill=LEAF)
    s.text(CX, CY + LR + 42, "the responder · one binary", 13, INK, font=MONO,
           anchor="middle", weight="bold")

    # manifests row top-right (under the frieze, clear of the title)
    s.text(W - 520, 142, "manifests:", 11, INK, font=MONO, weight="bold")
    for i, m in enumerate(["bands_cid", "algorithms_cid", "sources_cid", "schema_cid"]):
        x = W - 408 + i * 96
        s.seal(x, 138, 8, fill=[INDIGO, LEAF, TURMERIC, LAC][i])
        s.text(x + 12, 142, m.split("_")[0], 10.5, INK_SOFT, font=MONO)
    s.save(f"{OUT}/01-architecture.svg")
    return "01-architecture"


def federation():
    s = Svg(1600, 1040, seed=11); W, H = s.W, s.H
    CX, CY = 800, 588
    N, R = 8, 326
    nodes = []
    pubs = ["7f2a", "b41c", "9de0", "2c63", "e0a7", "55b9", "c1f4", "8a3d"]
    for i in range(N):
        th = math.radians(-90 + i * 360 / N)
        nodes.append((CX + R * math.cos(th), CY + R * math.sin(th) * 0.82, DYES[i % len(DYES)]))
    avoid = [(CX, CY, 160), (W / 2, 150, 600), (W / 2, H - 70, 540)]
    for (x, y, c) in nodes:
        avoid.append((x, y, 80))
    avoid += [(CX - R - 116, CY - 150, 80), (CX + R + 116, CY - 150, 80), (CX + R + 116, CY + 150, 80)]
    _ground(s, avoid=avoid)
    _header(s, "One memory, many responders",
            "every node signs under its own key · every node resolves the same id")
    for (x, y, col) in nodes:
        ctrl = ((x + CX) / 2 + (CY - y) * 0.08, (y + CY) / 2 - (CX - x) * 0.08)
        s.qpath((x, y), ctrl, (CX, CY), LEAF, 6.0, op=0.22, glow=True)
        s.qpath((x, y), ctrl, (CX, CY), LEAF, 1.2, op=0.55, cls="flowline")
    for i in range(N):
        a, b = nodes[i], nodes[(i + 1) % N]
        mx, my = (a[0] + b[0]) / 2, (a[1] + b[1]) / 2
        ox, oy = mx - CX, my - CY
        ol = math.hypot(ox, oy) or 1
        ctrl = (mx + ox / ol * 46, my + oy / ol * 46)
        if i == 5:
            s.qpath((a[0], a[1]), ctrl, (b[0], b[1]), VERMIL, 1.7, op=0.85)
            s.circle(ctrl[0], ctrl[1], 11, fill=PAPER, stroke=INK, sw=1.2)
            s.line(ctrl[0] - 6, ctrl[1] - 3, ctrl[0] + 6, ctrl[1] - 3, VERMIL, 1.4)
            s.line(ctrl[0] - 6, ctrl[1] + 3, ctrl[0] + 6, ctrl[1] + 3, VERMIL, 1.4)
        else:
            s.qpath((a[0], a[1]), ctrl, (b[0], b[1]), INK_SOFT, 1.1, op=0.6)
    s.lotus(CX, CY, 78)
    s.seal(CX + 66, CY - 66, 11, fill=LEAF)
    s.text(CX, CY + 116, "one content id", 13, INK, font=MONO, anchor="middle", weight="bold")
    s.text(CX, CY + 136, "resolved byte-for-byte", 11, INK_SOFT, font=MONO, anchor="middle")
    inners = ["fish", "leaf", "hatch", "ring"]
    for i, (x, y, col) in enumerate(nodes):
        s.roundel(x, y, 32, col, signed=True, petal=(i % 2 == 0), inner=inners[i % 4])
        # push labels below for the lower nodes, above for the upper nodes
        below = y >= CY
        s.text(x, y + (54 if below else -50), "responder", 11, INK, font=MONO, anchor="middle", weight="bold")
        s.text(x, y + (72 if below else -34), "key:" + pubs[i], 10.5, INK_SOFT, font=MONO, anchor="middle")

    def client(cx, cy, near):
        s.circle(cx, cy, 17, fill=PAPER, stroke=INK, sw=1.5)
        s.qpath((cx - 11, cy), (cx, cy - 8), (cx + 11, cy), INK, 1.3)
        s.qpath((cx - 11, cy), (cx, cy + 8), (cx + 11, cy), INK, 1.3)
        s.circle(cx, cy, 4.2, fill=LEAF, stroke=INK, sw=1.0)
        pts = [(cx, cy), ((cx + near[0]) / 2, (cy + near[1]) / 2 - 16), (near[0], near[1])]
        for t in [0.15, 0.35, 0.55, 0.75]:
            u = 1 - t
            px = u * u * pts[0][0] + 2 * u * t * pts[1][0] + t * t * pts[2][0]
            py = u * u * pts[0][1] + 2 * u * t * pts[1][1] + t * t * pts[2][1]
            s.dot(px, py, 1.2, LEAF, op=0.8)
    client(CX - R - 116, CY - 150, nodes[3]); client(CX + R + 116, CY - 150, nodes[7])
    client(CX + R + 116, CY + 150, nodes[6])
    s.text(CX - R - 116, CY - 186, "client", 11, INK, font=MONO, anchor="middle", weight="bold")
    s.text(CX - R - 116, CY - 120, "verifies offline", 10.5, INK_SOFT, font=MONO, anchor="middle")
    s.text(W / 2, H - 76, "Eight responders, one content id — resolved byte-for-byte under eight keys.",
           16, INK, font=SERIF, anchor="middle", italic=True)
    s.save(f"{OUT}/08-decentralised.svg")
    return "08-decentralised"


def trust_plane():
    s = Svg(1600, 1040, seed=10); W, H = s.W, s.H
    CY = 512
    px, py, pw = 84, 232, 440
    segs = [("domain", '"emem.preimage.v1" · "receipt"'), ("request_id", "ULID"),
            ("served_at", "ISO 8601"), ("[scope]·[as_of]", "optional, tagged"),
            ("[edges]·[manifest]", "optional, tagged"), ("primitive", "emem.recall"),
            ("cells[]", "len-prefixed list"), ("fact_cids[]", "len-prefixed list")]
    ph = 60 + len(segs) * 38
    bx, rx = 760, 1156
    avoid = [(px + pw / 2, py + ph / 2, pw * 0.62), (bx, CY, 70), (rx, CY, 170),
             (W / 2, 150, 600), (W / 2, H - 110, 640)]
    _ground(s, avoid=avoid)
    _header(s, "The trust plane",
            "what a responder signs, and how anyone re-checks it without trusting the server")

    # corner motifs in the open band above the row
    s.bird(180, 280, 34, col=TEAL, angle=0.12)
    s.sun(W - 160, 280, 26, col=TURMERIC)
    s.fish(150, H - 230, 24, col=INDIGO, angle=-0.12)
    s.vine(250, H - 240, 520, H - 252, col=LEAF, n=4, amp=16)

    # left: the v1 preimage as a stack of tagged, length-prefixed segments
    s.rect(px, py, pw, ph, fill=PAPER, op=0.78, stroke=INK, sw=2.2, rx=12)
    s.rect(px + 5, py + 5, pw - 10, ph - 10, stroke=INK, sw=0.8, rx=9, op=0.5)
    s.text(px + 22, py + 36, "preimage_version 1", 14, INK, font=MONO, weight="bold")
    s.text(px + pw - 20, py + 36, "tagged · len-prefixed", 10.5, INK_SOFT, font=MONO, anchor="end")
    s.line(px + 18, py + 50, px + pw - 18, py + 50, INK_SOFT, 1.0, op=0.6)
    for i, (tag, desc) in enumerate(segs):
        ry = py + 78 + i * 38
        s.seal(px + 28, ry - 5, 6.5, fill=DYES[i % len(DYES)])
        s.text(px + 46, ry, tag, 13, INK, font=MONO)
        s.text(px + pw - 20, ry, desc, 11, INK_SOFT, font=MONO, anchor="end")
    s.text(px + pw / 2, py + ph + 30, "no two distinct responses share signed bytes", 11.5,
           INK_SOFT, font=MONO, anchor="middle")

    # blake3 digest seal
    s.flow((px + pw + 6, CY), (bx - 38, CY), INK_SOFT, 1.2, op=0.6)
    s.circle(bx, CY, 38, fill=GOLD_PALE, stroke=INK, sw=2.2)
    s.dot_ring(bx, CY, 50, 12, 1.6, INK, phase=bx)
    s.dot_ring(bx, CY, 20, 6, 1.6, INK, op=0.7)
    s.text(bx, CY - 58, "blake3", 13, INK, font=MONO, anchor="middle", weight="bold")
    s.text(bx, CY + 6, "32 B", 12, INK, font=MONO, anchor="middle", weight="bold")
    s.text(bx, CY + 66, "digest", 11, INK_SOFT, font=MONO, anchor="middle")

    # ed25519 sign -> receipt lotus
    s.flow((bx + 40, CY), (rx - 130, CY), LEAF, 1.6, op=0.85, glow=True)
    kx = (bx + rx) / 2
    s.circle(kx, CY - 34, 11, stroke=INK, sw=1.8)
    s.line(kx + 8, CY - 27, kx + 26, CY - 7, INK, 1.8)
    s.line(kx + 21, CY - 12, kx + 28, CY - 5, INK, 1.8)
    s.text(kx, CY - 52, "ed25519", 11, INK, font=MONO, anchor="middle", weight="bold")
    s.lotus(rx, CY, 100)
    s.seal(rx + 84, CY - 84, 13, fill=LEAF)
    s.text(rx, CY + 130, "RECEIPT", 14, INK, font=MONO, anchor="middle", weight="bold")
    s.text(rx, CY + 150, "signed · merkle proof", 11.5, INK_SOFT, font=MONO, anchor="middle")

    # identity note top-right
    s.seal(1392, CY - 196, 10, fill=INDIGO)
    s.text(1392, CY - 174, "responder pubkey", 11.5, INK, font=MONO, anchor="middle", weight="bold")
    s.text(1392, CY - 154, "/.well-known/emem.json", 10.5, INK_SOFT, font=MONO, anchor="middle")

    # footer recipe
    s.line(220, H - 158, W - 220, H - 158, INK_SOFT, 0.8, op=0.45)
    s.dot(220, H - 158, 2.4, VERMIL); s.dot(W - 220, H - 158, 2.4, VERMIL)
    s.text(W / 2, H - 122, "verify offline:  digest = blake3(preimage);  ed25519.verify(sig, digest, pubkey)",
           14, INK, font=MONO, anchor="middle")
    s.text(W / 2, H - 90, "merkle tree: RFC 6962 leaf/node domain separation · duplicate leaves rejected",
           11.5, INK_SOFT, font=MONO, anchor="middle")
    s.text(W / 2, H - 60, "the same blake3 + ed25519 check runs in your browser at /verify",
           11.5, INK_SOFT, font=MONO, anchor="middle")
    s.save(f"{OUT}/10-trust-plane.svg")
    return "10-trust-plane"


def _ground(s, avoid=None, top=176):
    """Paper + dense Mithila ground + the rich border, with keep-clear discs."""
    s.paper(); s.fill_ground(avoid=avoid or [], top=top); s.border()


def _header(s, title, sub):
    """Title + tagline + bud-vine frieze. The ground/border are drawn by the
    caller (so it can pass the right keep-clear discs before the header)."""
    s.text(78, 108, title, 30, INK, font=SERIF)
    s.text(80, 142, sub, 13, INK_SOFT, font=MONO)
    s.frieze(168, 78, s.W - 78)


def _corner_motifs(s, footer_clear=True):
    """Scatter a few storytelling motifs into the four open zones of a scene
    whose centre/sides are otherwise occupied, so no corner sits dead."""
    W, H = s.W, s.H
    s.bird(150, 250, 36, col=TEAL, angle=0.12)
    s.sun(W - 150, 250, 26, col=TURMERIC)
    s.bird(W - 168, H - 230, 38, col=LAC, angle=-0.06)
    s.fish(150, H - 230, 26, col=INDIGO, angle=-0.12)
    s.fish(208, H - 200, 20, col=VERMIL, angle=0.1)
    s.bud(96 + 18, (H + 230) / 2, 20, col=VERMIL)
    s.leaf(W - 116, (H + 230) / 2, 22, col=LEAF, angle=0.3)


def _pipeline(name, seed, title, sub, stages, endlabel, endsub, footer):
    s = Svg(1600, 1040, seed=seed); W, H = s.W, s.H
    ys = 558
    x0, x1 = 196, 1108
    rx = 1392
    n = len(stages)
    xs = [x0 + (x1 - x0) * i / (n - 1) for i in range(n)]
    avoid = [(W / 2, 150, 600), (W / 2, H - 80, 640), (rx, ys, 170)]
    for x in xs:
        avoid.append((x, ys, 90))
    _ground(s, avoid=avoid)
    _header(s, title, sub)
    _corner_motifs(s)

    inners = ["ring", "fish", "leaf", "hatch", "ring"]
    for i, (lab, sub2, col) in enumerate(stages):
        if i:
            s.flow((xs[i - 1] + 36, ys), (xs[i] - 36, ys), INK_SOFT, 1.4, op=0.7)
        s.roundel(xs[i], ys, 34, col, petal=(i % 2 == 0), inner=inners[i % len(inners)])
        s.text(xs[i], ys + 56, lab, 12, INK, font=MONO, anchor="middle", weight="bold")
        if sub2:
            s.text(xs[i], ys + 74, sub2, 10.5, INK_SOFT, font=MONO, anchor="middle")
    s.flow((xs[-1] + 36, ys), (rx - 112, ys), LEAF, 1.6, op=0.85, glow=True)
    s.lotus(rx, ys, 92); s.seal(rx + 76, ys - 76, 12, fill=LEAF)
    s.text(rx, ys + 122, endlabel, 13, INK, font=MONO, anchor="middle", weight="bold")
    s.text(rx, ys + 142, endsub, 11, INK_SOFT, font=MONO, anchor="middle")
    s.line(220, H - 116, W - 220, H - 116, INK_SOFT, 0.8, op=0.45)
    s.dot(220, H - 116, 2.4, VERMIL); s.dot(W - 220, H - 116, 2.4, VERMIL)
    s.text(W / 2, H - 80, footer, 16, INK, font=SERIF, anchor="middle", italic=True)
    s.text(W / 2, H - 50, "anyone re-checks the receipt at /verify · no trust in the server",
           11.5, INK_SOFT, font=MONO, anchor="middle")
    s.save(f"{OUT}/{name}.svg"); return name


def data_flow():
    return _pipeline("02-data-flow", 2, "How data moves",
        "stateless, function-keyed, deterministic from pixel to content id",
        [("open pixel", "vsicurl Range", INDIGO), ("connector", "stateless", TEAL),
         ("PrimaryFact", "or Derivative", LEAF), ("canonical CBOR", "ciborium", TURMERIC),
         ("content id", "blake3", LAC)],
        "FACT", "signed · cite-able",
        "Same bytes in, same content id out, on every machine.")


def anatomy():
    return _pipeline("03-anatomy-of-a-request", 3, "Anatomy of a recall",
        "a place name in, a signed fact out, in about 180 ms",
        [("place name", "\"Denver\"", INDIGO), ("locate", "cell64", TEAL),
         ("cache miss", "cold", VERMIL), ("fetch tile", "upstream COG", TURMERIC),
         ("sign", "ed25519", LAC)],
        "RECEIPT", "fact_cid + proof",
        "A cold read costs about 180 ms; a warm one is under ten, and cite-able forever after.")


def _cycle(name, seed, title, sub, stages, centre, signed_lab, bead, footer,
           phase=-90):
    """A ring of stages flowing clockwise into a central lotus. Shared by the
    agent loop and the cite economy."""
    s = Svg(1600, 1040, seed=seed); W, H = s.W, s.H
    CX, CY = 800, 600; R = 296; LR = 84
    pos = []
    for i in range(len(stages)):
        th = math.radians(phase + i * 360 / len(stages))
        pos.append((CX + R * math.cos(th), CY + R * math.sin(th) * 0.92))
    avoid = [(CX, CY, LR + 60), (W / 2, 150, 600), (W / 2, H - 80, 640)]
    for (x, y) in pos:
        avoid.append((x, y, 96))
    _ground(s, avoid=avoid)
    _header(s, title, sub)
    _corner_motifs(s)

    for i in range(len(stages)):
        a, b = pos[i], pos[(i + 1) % len(stages)]
        mx, my = (a[0] + b[0]) / 2, (a[1] + b[1]) / 2
        ox, oy = mx - CX, my - CY; ol = math.hypot(ox, oy) or 1
        ctrl = (mx + ox / ol * 44, my + oy / ol * 44)
        s.qpath(a, ctrl, b, bead, 1.5, op=0.7, cls="flowline")
        s.dot(ctrl[0], ctrl[1], 2.6, bead, op=0.85)
    inners = ["fish", "ring", "leaf", "hatch", "ring", "fish"]
    for i, (lab, sub2, col) in enumerate(stages):
        x, y = pos[i]
        s.roundel(x, y, 33, col, signed=(lab == signed_lab), petal=(i % 2 == 0),
                  inner=inners[i % len(inners)])
        below = y >= CY - 10
        s.text(x, y + (52 if below else -52), lab, 12.5, INK, font=MONO, anchor="middle", weight="bold")
        s.text(x, y + (70 if below else -34), sub2, 10.5, INK_SOFT, font=MONO, anchor="middle")
    s.lotus(CX, CY, LR)
    s.seal(CX + LR * 0.84, CY - LR * 0.84, 11, fill=LEAF)
    s.text(CX, CY + LR + 40, centre[0], 12.5, INK, font=MONO, anchor="middle", weight="bold")
    s.text(CX, CY + LR + 60, centre[1], 11, INK_SOFT, font=MONO, anchor="middle")
    s.line(220, H - 112, W - 220, H - 112, INK_SOFT, 0.8, op=0.45)
    s.dot(220, H - 112, 2.4, VERMIL); s.dot(W - 220, H - 112, 2.4, VERMIL)
    s.text(W / 2, H - 76, footer, 16, INK, font=SERIF, anchor="middle", italic=True)
    s.save(f"{OUT}/{name}.svg"); return name


def agent_loop():
    return _cycle("04-agent-loop", 4, "The agent loop",
        "ask, recall, cite, act, and the memory is better for the next turn",
        [("ask", "free text", INDIGO), ("recall", "signed fact", TEAL),
         ("cite", "fact_cid", LEAF), ("act", "in the world", TURMERIC),
         ("write back", "a new memory", LAC)],
        ("the shared memory", "grows each turn"), "recall", LEAF,
        "Every claim the agent makes carries a handle the next agent can re-check.")


def fact_to_reasoning():
    s = Svg(1600, 1040, seed=5); W, H = s.W, s.H
    CY = 540
    px, py, pw, ph = 84, 372, 410, 244
    cwx, cwy, cww, cwh = 612, 322, 596, 392
    avoid = [(px + pw / 2, py + ph / 2, pw * 0.66), (cwx + cww / 2, cwy + cwh / 2, cww * 0.62),
             (1396, CY, 170), (W / 2, 150, 600), (W / 2, H - 78, 640)]
    _ground(s, avoid=avoid)
    _header(s, "Grounding in the context window",
            "a recalled fact the model can point at, instead of a number it made up")
    s.bird(170, 280, 34, col=TEAL, angle=0.12)
    s.fish(160, H - 220, 24, col=INDIGO, angle=-0.12)
    s.fish(218, H - 192, 18, col=VERMIL, angle=0.1)
    s.sun(1396, 270, 26, col=TURMERIC)

    # left: a signed fact card (band stacked above value so the long key fits)
    s.rect(px, py, pw, ph, fill=PAPER, op=0.78, stroke=INK, sw=2.2, rx=12)
    s.rect(px + 5, py + 5, pw - 10, ph - 10, stroke=INK, sw=0.8, rx=9, op=0.5)
    s.seal(px + 28, py + 32, 8, fill=LEAF)
    s.text(px + 48, py + 38, "signed fact", 14, INK, font=MONO, weight="bold")
    s.line(px + 18, py + 54, px + pw - 18, py + 54, INK_SOFT, 1.0, op=0.6)
    rows = [("band", "copdem30m.elevation_mean"), ("cell", "defi.zb5c4.guxe.nuxe"),
            ("value", "1609 m"), ("fact_cid", "72wdchiyurfr…")]
    for i, (k, v) in enumerate(rows):
        ry = py + 86 + i * 42
        s.text(px + 28, ry, k, 12.5, INK_SOFT, font=MONO)
        s.text(px + pw - 22, ry, v, 12.5, INK, font=MONO, anchor="end", weight="bold")

    # arrow into the context window
    s.flow((px + pw + 6, CY), (cwx - 10, CY), LEAF, 1.6, op=0.85, glow=True)
    s.rect(cwx, cwy, cww, cwh, fill=PAPER, op=0.66, stroke=INK, sw=2.2, rx=14)
    s.rect(cwx + 5, cwy + 5, cww - 10, cwh - 10, stroke=INK, sw=0.8, rx=11, op=0.5)
    nrv = int(cww / 16)
    for k in range(nrv):
        s.dot(cwx + (k + 0.5) * cww / nrv, cwy, 1.1, INK, op=0.4)
        s.dot(cwx + (k + 0.5) * cww / nrv, cwy + cwh, 1.1, INK, op=0.4)
    s.text(cwx + cww / 2, cwy + 36, "context window", 14, INK, font=MONO, anchor="middle", weight="bold")
    s.line(cwx + 24, cwy + 52, cwx + cww - 24, cwy + 52, INK_SOFT, 0.9, op=0.5)
    s.text(cwx + 30, cwy + 96, "USER:  how high is Denver?", 13, INK_SOFT, font=MONO)
    s.text(cwx + 30, cwy + 144, "FACT:  elevation 1609 m", 13, LEAF, font=MONO, weight="bold")
    s.text(cwx + 30, cwy + 168, "       cell defi.zb5c4… · 72wdchiyurfr…", 11, INK_SOFT, font=MONO)
    s.text(cwx + 30, cwy + 220, "ASSISTANT:", 13, INK_SOFT, font=MONO)
    s.text(cwx + 30, cwy + 248, "Denver sits at 1609 m, mile-high.", 13, INK, font=MONO)
    s.text(cwx + 30, cwy + 272, "Citation: 72wdchiyurfr…", 11, INDIGO, font=MONO, weight="bold")

    # the signed answer flows out to a receipt lotus on the right
    s.flow((cwx + cww + 6, CY), (1396 - 96, CY), LEAF, 1.6, op=0.85, glow=True)
    s.lotus(1396, CY, 86); s.seal(1396 + 72, CY - 72, 12, fill=LEAF)
    s.text(1396, CY + 116, "cited answer", 12.5, INK, font=MONO, anchor="middle", weight="bold")
    s.text(1396, CY + 136, "points at a fact_cid", 11, INK_SOFT, font=MONO, anchor="middle")

    s.line(220, H - 114, W - 220, H - 114, INK_SOFT, 0.8, op=0.45)
    s.dot(220, H - 114, 2.4, VERMIL); s.dot(W - 220, H - 114, 2.4, VERMIL)
    s.text(W / 2, H - 78, "The answer points at a fact id, so the reader can pull the same bytes and check.",
           16, INK, font=SERIF, anchor="middle", italic=True)
    s.save(f"{OUT}/05-fact-to-reasoning.svg"); return "05-fact-to-reasoning"


def memory_vs_stac():
    s = Svg(1600, 1040, seed=6); W, H = s.W, s.H
    ty, by = 372, 668
    stac = ["pick a scene", "range-read pixels", "undo scaling", "choose a mask", "a number"]
    x0, x1 = 256, 1150
    xs = [x0 + (x1 - x0) * i / (len(stac) - 1) for i in range(len(stac))]
    avoid = [(W / 2, 150, 600), (W / 2, H - 78, 640), (1322, by, 150)]
    for x in xs:
        avoid.append((x, ty, 70))
    avoid.append((360, by, 70))
    for x in (640, 940):
        avoid.append((x, by, 60))
    _ground(s, avoid=avoid)
    _header(s, "What the agent carries",
            "raw tiles make the agent do the work; emem signs the answer once")
    s.sun(W - 150, 280, 26, col=TURMERIC)
    s.fish(170, H - 230, 24, col=INDIGO, angle=-0.12)
    s.bird(W - 170, H - 220, 36, col=LAC, angle=-0.06)

    # top row: raw STAC (long chain, uncited)
    s.text(150, ty - 16, "raw STAC", 13, INK, font=MONO, weight="bold")
    s.text(150, ty + 6, "you carry it", 11, INK_SOFT, font=MONO)
    for i, lab in enumerate(stac):
        if i:
            s.flow((xs[i - 1] + 28, ty), (xs[i] - 28, ty), INK_SOFT, 1.2, op=0.6)
        s.roundel(xs[i], ty, 25, INK_SOFT if i < len(stac) - 1 else VERMIL,
                  inner="hatch" if i < len(stac) - 1 else "ring")
        s.text(xs[i], ty + 46, lab, 11, INK, font=MONO, anchor="middle", weight="bold")
    s.text(1322, ty - 6, "uncited", 13, VERMIL, font=MONO, anchor="middle", weight="bold")
    s.text(1322, ty + 14, "no handle", 11, INK_SOFT, font=MONO, anchor="middle")

    # bottom row: emem (one recall -> signed)
    s.text(150, by - 16, "emem", 13, INK, font=MONO, weight="bold")
    s.text(150, by + 6, "it carries it", 11, INK_SOFT, font=MONO)
    s.roundel(360, by, 31, INDIGO, petal=True, inner="fish")
    s.text(360, by + 52, "one recall", 12, INK, font=MONO, anchor="middle", weight="bold")
    s.flow((400, by), (1322 - 100, by), LEAF, 1.7, op=0.85, glow=True)
    s.leaf(640, by - 34, 16, col=LEAF, angle=0.2); s.bud(940, by - 36, 16, col=VERMIL)
    s.lotus(1322, by, 82); s.seal(1322 + 68, by - 68, 12, fill=LEAF)
    s.text(1322, by + 112, "signed number", 12, INK, font=MONO, anchor="middle", weight="bold")
    s.text(1322, by + 132, "+ fact_cid", 11, INK_SOFT, font=MONO, anchor="middle")

    s.line(220, H - 114, W - 220, H - 114, INK_SOFT, 0.8, op=0.45)
    s.dot(220, H - 114, 2.4, VERMIL); s.dot(W - 220, H - 114, 2.4, VERMIL)
    s.text(W / 2, H - 78, "emem does the scene-picking, scaling, and masking once, and signs the result.",
           16, INK, font=SERIF, anchor="middle", italic=True)
    s.save(f"{OUT}/06-memory-vs-stac.svg"); return "06-memory-vs-stac"


def cite_economy():
    return _cycle("07-cite-economy", 7, "The cite economy",
        "a fact that gets used gets found; citation is the discovery signal",
        [("attester", "signs a fact", INDIGO), ("content id", "names the bytes", TEAL),
         ("agent cites", "the CID", LEAF), ("registry", "CAS update", TURMERIC),
         ("score grows", "more cites", LAC), ("discovery", "favours it", VERMIL)],
        ("the fact", "gathers cites"), "attester", TURMERIC,
        "The more an answer is cited, the easier the next agent finds it.")


def address_algebra():
    s = Svg(1600, 1040, seed=9); W, H = s.W, s.H
    rows = [("cell", "64", "four base-65,536 bigrams", "defi.zb493.xuqA.zcb5f", INDIGO),
            ("tslot", "64", "base32-nopad leb128, t. prefix", "t.aaaaagy", TEAL),
            ("cid", "32 B blake3", "base32-nopad-lowercase", "qi3jo4sq…l2hgjtwm", LEAF),
            ("vec", "1792-D fp16", "12-byte prefix in receipts", "full vector via recall", LAC)]
    tx, ty, tw = 196, 300, 1208
    rh = 118
    cols = [tx + 100, tx + 320, tx + 560, tx + 900]
    avoid = [(W / 2, 150, 600), (W / 2, H - 118, 700),
             (tx + tw / 2, ty + rh * 1.6, tw * 0.58)]
    _ground(s, avoid=avoid)
    _header(s, "The address algebra",
            "four wire forms, each content-stable across every responder")
    # margin motifs (clear of the table band)
    s.fish(150, H - 230, 24, col=INDIGO, angle=-0.12)
    s.bird(W - 168, H - 220, 36, col=LAC, angle=-0.06)
    s.sun(W - 150, 264, 24, col=TURMERIC)
    s.bird(160, 268, 32, col=TEAL, angle=0.1)

    for cx2, lab in zip(cols, ["field", "bits", "wire form", "example"]):
        s.text(cx2, ty - 22, lab, 12, INK_SOFT, font=MONO, weight="bold")
    s.line(tx, ty, tx + tw, ty, INK, 2.0)
    s.dot(tx, ty, 2.6, VERMIL); s.dot(tx + tw, ty, 2.6, VERMIL)
    inners = ["fish", "ring", "leaf", "hatch"]
    for i, (f, bits, wire, ex, col) in enumerate(rows):
        ry = ty + 46 + i * rh
        s.roundel(tx + 36, ry - 6, 24, col, signed=(i == 0), inner=inners[i])
        s.text(cols[0] + 4, ry, f, 17, INK, font=MONO, weight="bold")
        s.text(cols[1], ry, bits, 13.5, INK, font=MONO)
        s.text(cols[2], ry, wire, 13.5, INK, font=MONO)
        s.text(cols[3], ry, ex, 13.5, col, font=MONO, weight="bold")
        if i:
            s.line(tx, ry - rh / 2 - 6, tx + tw, ry - rh / 2 - 6, INK_SOFT, 0.8, op=0.4)
    s.line(tx, ty + 46 + len(rows) * rh - rh / 2 - 6, tx + tw,
           ty + 46 + len(rows) * rh - rh / 2 - 6, INK_SOFT, 0.8, op=0.4)

    s.line(220, H - 142, W - 220, H - 142, INK_SOFT, 0.8, op=0.45)
    s.dot(220, H - 142, 2.4, VERMIL); s.dot(W - 220, H - 142, 2.4, VERMIL)
    s.text(W / 2, H - 104, "The active grid is about 9.55 m at the equator; adjacent cells share a string prefix,",
           15, INK, font=SERIF, anchor="middle", italic=True)
    s.text(W / 2, H - 74, "so an LLM that emits defi.zb493… already lands in roughly the right place.",
           15, INK, font=SERIF, anchor="middle", italic=True)
    s.save(f"{OUT}/09-address-algebra.svg"); return "09-address-algebra"


def _encoders(name, seed, title, sub, footer):
    s = Svg(1600, 1040, seed=seed); W, H = s.W, s.H
    CX, CY = 800, 600; LR = 100; ER = 210
    oy = 312
    sats = [("Sentinel-2", TEAL), ("Sentinel-1", INDIGO), ("HLS", LEAF), ("MODIS", TURMERIC), ("DEM", LAC)]
    sxs = [320 + i * 248 for i in range(len(sats))]
    encs = [("Clay v1.5", INDIGO), ("Prithvi-EO-2", TEAL), ("Tessera", LEAF), ("Galileo", LAC)]
    epos = [(CX + ER * math.cos(math.radians(-90 + i * 90)),
             CY + ER * math.sin(math.radians(-90 + i * 90)) * 0.92) for i in range(len(encs))]
    avoid = [(CX, CY, LR + 60), (W / 2, 150, 600), (W / 2, H - 80, 640)]
    for x in sxs:
        avoid.append((x, oy, 70))
    for (x, y) in epos:
        avoid.append((x, y, 80))
    _ground(s, avoid=avoid)
    _header(s, title, sub)

    # the big golden sun presiding upper-right; a vine swims lower-left
    s.sun(W - 156, 300, 30)
    s.fish(160, H - 226, 24, col=INDIGO, angle=-0.12)
    s.fish(214, H - 196, 18, col=VERMIL, angle=0.1)

    # orbit band (top): satellites / encoders
    s.text(150, oy + 4, "in orbit", 13, INK, font=MONO, weight="bold")
    for i, (lab, col) in enumerate(sats):
        x = sxs[i]
        s.rect(x - 11, oy - 11, 22, 22, fill=col, stroke=INK, sw=1.8, rx=4)
        s.rect(x - 32, oy - 8, 15, 16, fill=PAPER, stroke=INK, sw=1.2)
        s.rect(x + 17, oy - 8, 15, 16, fill=PAPER, stroke=INK, sw=1.2)
        s.dot(x, oy, 3, PAPER)
        s.text(x, oy + 40, lab, 11, INK, font=MONO, anchor="middle", weight="bold")
        s.flow((x, oy + 16), (CX, CY - LR - 30), LEAF if i % 2 else INK_SOFT, 1.1, op=0.5, beads=False)

    # encoders ring around the lotus
    inners = ["fish", "ring", "leaf", "hatch"]
    for i, (lab, col) in enumerate(encs):
        x, y = epos[i]
        s.line(CX, CY, x, y, INK_SOFT, 0.9, op=0.4)
        s.roundel(x, y, 24, col, signed=True, inner=inners[i])
        ly = y - 38 if y < CY else y + 40
        s.text(x, ly, lab, 11, INK, font=MONO, anchor="middle", weight="bold")

    # ground (centre): the responder fuses + signs
    s.lotus(CX, CY, LR)
    s.seal(CX + LR * 0.84, CY - LR * 0.84, 13, fill=LEAF)
    s.text(CX, CY + LR + 40, "the responder · decodes + fuses + signs", 12.5,
           INK, font=MONO, anchor="middle", weight="bold")

    s.line(220, H - 116, W - 220, H - 116, INK_SOFT, 0.8, op=0.45)
    s.dot(220, H - 116, 2.4, VERMIL); s.dot(W - 220, H - 116, 2.4, VERMIL)
    s.text(W / 2, H - 80, footer, 16, INK, font=SERIF, anchor="middle", italic=True)
    s.save(f"{OUT}/{name}.svg"); return name


def encoders_split():
    return _encoders("31-encoders-in-orbit-decoders-on-ground", 31,
        "Encoders in orbit, decoders on the ground",
        "the sensors fly; the responder decodes their stacks into signed embeddings",
        "Four foundation encoders read the same place differently, so disagreement is informative.")


def fusion():
    return _encoders("33-fusion-orbit-and-ground", 33,
        "Fusion: orbit and ground",
        "many modalities, one cell, one signed embedding the next agent can cite",
        "S1, S2, DEM and climate fuse at one cell into a vector with a content id.")


def agent_to_token():
    return _pipeline("38-agent-to-token", 38, "From your agent to a token",
        "the whole loop: read with no account, verify with no trust, keep one line",
        [("your agent", "any model", INDIGO), ("MCP · REST", "same handlers", TEAL),
         ("recall", "hit, or fetch once", LEAF), ("open sources", "on the first miss", TURMERIC),
         ("signed fact", "blake3 · ed25519", LAC)],
        "MEMORY TOKEN", "84 chars · resolves anywhere",
        "The agent keeps one line; any agent, on any model, resolves the same bytes.")


REG = {"01-architecture": architecture, "08-decentralised": federation, "10-trust-plane": trust_plane,
       "02-data-flow": data_flow, "03-anatomy-of-a-request": anatomy, "04-agent-loop": agent_loop,
       "05-fact-to-reasoning": fact_to_reasoning, "06-memory-vs-stac": memory_vs_stac,
       "07-cite-economy": cite_economy, "09-address-algebra": address_algebra,
       "31-encoders-in-orbit-decoders-on-ground": encoders_split, "33-fusion-orbit-and-ground": fusion,
       "38-agent-to-token": agent_to_token}


def main():
    import cairosvg
    names = sys.argv[1:] or list(REG)
    for n in names:
        if n not in REG:
            print("unknown:", n); continue
        REG[n]()
        cairosvg.svg2png(url=f"{OUT}/{n}.svg", write_to=f"/tmp/{n}.png", output_width=1000)
        print("wrote", f"{OUT}/{n}.svg")


if __name__ == "__main__":
    main()
