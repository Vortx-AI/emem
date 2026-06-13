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
    s.paper(); s.border()
    s.text(80, 56, "One binary, signed end to end", 30, INK, font=SERIF)
    s.text(82, 88, "the same handlers answer MCP and REST · reads need no auth · every write is logged", 13, INK_SOFT, font=MONO)
    CX, CY = 800, 560

    # left: 46 sources feed in
    SRCX = 150
    for i in range(9):
        y = CY - 200 + i * 50
        s.roundel(SRCX, y, 12, DYES[i % len(DYES)])
        s.flow((SRCX + 16, y), (CX - 150, CY + (y - CY) * 0.25), INK_SOFT, 0.9, op=0.45, bow=16 * math.sin(i))
    s.text(SRCX, CY - 238, "46 sources", 13, INK, font=MONO, anchor="middle")
    s.text(SRCX, CY + 236, "STAC + COG, signed on the first miss", 10, INK_SOFT, font=MONO, anchor="middle")

    # right: GPU sidecar
    SIDX = 1452
    s.circle(SIDX, CY, 34, fill=LAC, stroke=INK, sw=1.6)
    s.dot_ring(SIDX, CY, 17, 4, 4.2, GOLD_PALE)
    s.dot_ring(SIDX, CY, 44, 8, 1.4, INK, phase=SIDX)
    s.flow((SIDX - 36, CY), (CX + 150, CY), LAC, 1.1, op=0.6, bow=-26)
    s.text(SIDX, CY - 56, "GPU sidecar", 13, INK, font=MONO, anchor="middle")
    s.text(SIDX, CY + 56, "Clay · Prithvi · Tessera · Galileo", 9.5, INK_SOFT, font=MONO, anchor="middle")

    # top: clients
    for (cx, label, col) in [(CX - 150, "MCP", TEAL), (CX + 150, "REST", INDIGO)]:
        s.circle(cx, 168, 26, fill=col, stroke=INK, sw=1.6)
        s.dot_ring(cx, 168, 33, 8, 1.4, INK, phase=cx)
        s.text(cx, 172, label, 10, PAPER, font=MONO, anchor="middle")
        s.flow((cx, 196), (CX, CY - 150), INK_SOFT, 1.2, op=0.7)
    s.text(CX, 132, "clients · same handlers", 13, INK, font=MONO, anchor="middle")

    # bottom: append-only signed log
    LOGY = CY + 240
    xs = [CX - 270 + i * 90 for i in range(7)]
    for i, x in enumerate(xs):
        if i:
            s.line(xs[i - 1] + 16, LOGY, x - 16, LOGY, INK, 1.4)
            s.dot((xs[i - 1] + x) / 2, LOGY, 1.6, INK)
        s.seal(x, LOGY, 15, fill=[INDIGO, LEAF, TEAL, TURMERIC, LAC, VERMIL, GOLD_PALE][i])
    s.flow((CX, CY + 150), (CX, LOGY - 22), LEAF, 1.4, op=0.9, glow=True)
    s.text(CX, LOGY + 30, "append-only signed log", 13, INK, font=MONO, anchor="middle")
    s.text(CX, LOGY + 46, "blake3-chained · merkle root per batch", 10, INK_SOFT, font=MONO, anchor="middle")

    # inner ring of primitives
    prims = [("recall", INDIGO), ("find_similar", TEAL), ("verify", LEAF),
             ("hunt", TURMERIC), ("eudr_dds", VERMIL), ("state", LAC)]
    RR = 170
    for i, (name, col) in enumerate(prims):
        th = math.radians(-90 + i * 360 / len(prims))
        x, y = CX + RR * math.cos(th), CY + RR * math.sin(th) * 0.9
        s.line(CX, CY, x, y, INK_SOFT, 0.8, op=0.4)
        s.roundel(x, y, 19, col, signed=(i % 2 == 0))
        s.text(x, y + 32, name, 10, INK, font=MONO, anchor="middle")

    s.lotus(CX, CY, 72)
    s.text(CX, CY + 98, "the responder", 13, INK, font=MONO, anchor="middle")
    s.text(CX, CY + 114, "one binary", 11, INK_SOFT, font=MONO, anchor="middle")

    # manifests row top-right
    s.text(W - 486, 52, "manifests:", 10, INK, font=MONO)
    for i, m in enumerate(["bands_cid", "algorithms_cid", "sources_cid", "schema_cid"]):
        x = W - 470 + 12 + i * 112
        s.seal(x, 74, 8, fill=[INDIGO, LEAF, TURMERIC, LAC][i])
        s.text(x + 12, 78, m, 10, INK_SOFT, font=MONO)
    s.save(f"{OUT}/01-architecture.svg")
    return "01-architecture"


def federation():
    s = Svg(1600, 1040, seed=11); W, H = s.W, s.H
    s.paper(); s.border()
    s.text(80, 56, "One memory, many responders", 30, INK, font=SERIF)
    s.text(82, 88, "every node signs under its own key · every node resolves the same id", 13, INK_SOFT, font=MONO)
    CX, CY = 800, 560
    N, R = 8, 330
    nodes = []
    pubs = ["7f2a", "b41c", "9de0", "2c63", "e0a7", "55b9", "c1f4", "8a3d"]
    for i in range(N):
        th = math.radians(-90 + i * 360 / N)
        nodes.append((CX + R * math.cos(th), CY + R * math.sin(th) * 0.82, DYES[i % len(DYES)]))
    for (x, y, col) in nodes:
        ctrl = ((x + CX) / 2 + (CY - y) * 0.08, (y + CY) / 2 - (CX - x) * 0.08)
        s.qpath((x, y), ctrl, (CX, CY), LEAF, 6.0, op=0.22, glow=True)
        s.qpath((x, y), ctrl, (CX, CY), LEAF, 1.2, op=0.55)
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
    s.lotus(CX, CY, 64)
    s.text(CX, CY + 90, "one content id", 13, INK, font=MONO, anchor="middle")
    s.text(CX, CY + 107, "resolved byte-for-byte", 11, INK_SOFT, font=MONO, anchor="middle")
    for i, (x, y, col) in enumerate(nodes):
        s.roundel(x, y, 32, col, signed=True, petal=(i % 2 == 0))
        s.text(x, y + 50, "responder", 10, INK, font=MONO, anchor="middle")
        s.text(x, y + 64, "key:" + pubs[i], 10, INK_SOFT, font=MONO, anchor="middle")

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
    s.text(CX - R - 116, CY - 188, "client", 10, INK, font=MONO, anchor="middle")
    s.text(CX - R - 116, CY - 122, "verifies offline", 10, INK_SOFT, font=MONO, anchor="middle")
    s.save(f"{OUT}/08-decentralised.svg")
    return "08-decentralised"


def trust_plane():
    s = Svg(1600, 1040, seed=10); W, H = s.W, s.H
    s.paper(); s.border()
    s.text(80, 56, "The trust plane", 30, INK, font=SERIF)
    s.text(82, 88, "what a responder signs, and how anyone re-checks it without trusting the server", 13, INK_SOFT, font=MONO)
    CY = 470

    # left: the v1 preimage as a stack of tagged, length-prefixed segments
    px, py, pw = 96, 196, 430
    segs = [("domain", '"emem.preimage.v1" · "receipt"'), ("request_id", "ULID"),
            ("served_at", "ISO 8601"), ("[scope]·[as_of]", "optional, tagged"),
            ("[edges]·[manifest]", "optional, tagged"), ("primitive", "emem.recall"),
            ("cells[]", "len-prefixed list"), ("fact_cids[]", "len-prefixed list")]
    ph = 52 + len(segs) * 33
    s.rect(px, py, pw, ph, fill=PAPER, op=0.62, stroke=INK, sw=1.6, rx=10)
    s.text(px + 20, py + 30, "preimage_version 1", 13, INK, font=MONO, weight="bold")
    s.text(px + pw - 18, py + 30, "tagged · length-prefixed", 10, INK_SOFT, font=MONO, anchor="end")
    s.line(px + 18, py + 42, px + pw - 18, py + 42, INK_SOFT, 0.8, op=0.6)
    for i, (tag, desc) in enumerate(segs):
        ry = py + 64 + i * 33
        s.seal(px + 26, ry - 4, 5.5, fill=DYES[i % len(DYES)])
        s.text(px + 42, ry, tag, 12, INK, font=MONO)
        s.text(px + pw - 20, ry, desc, 10.5, INK_SOFT, font=MONO, anchor="end")
    s.text(px + pw / 2, py + ph + 24, "no two distinct responses share signed bytes", 11, INK_SOFT, font=MONO, anchor="middle")

    # blake3 digest seal
    bx = 720
    s.flow((px + pw + 4, CY), (bx - 30, CY), INK_SOFT, 1.1, op=0.6)
    s.circle(bx, CY, 30, fill=GOLD_PALE, stroke=INK, sw=1.8)
    s.dot_ring(bx, CY, 30 * 1.3, 10, 1.4, INK, phase=bx)
    s.text(bx, CY - 46, "blake3", 12, INK, font=MONO, anchor="middle")
    s.text(bx, CY + 4, "32 B", 10, INK, font=MONO, anchor="middle")
    s.text(bx, CY + 50, "digest", 10, INK_SOFT, font=MONO, anchor="middle")

    # ed25519 sign -> receipt lotus
    rx = 1100
    s.flow((bx + 32, CY), (rx - 100, CY), LEAF, 1.4, op=0.85, glow=True)
    # key motif over the flow
    kx = (bx + rx) / 2
    s.circle(kx, CY - 28, 9, stroke=INK, sw=1.5)
    s.line(kx + 6, CY - 22, kx + 22, CY - 6, INK, 1.6)
    s.line(kx + 18, CY - 10, kx + 24, CY - 4, INK, 1.6)
    s.text(kx, CY - 44, "ed25519", 10, INK, font=MONO, anchor="middle")
    s.lotus(rx, CY, 78)
    s.seal(rx + 66, CY - 66, 11, fill=LEAF)
    s.text(rx, CY + 100, "RECEIPT", 13, INK, font=MONO, anchor="middle", weight="bold")
    s.text(rx, CY + 117, "signed · merkle proof", 11, INK_SOFT, font=MONO, anchor="middle")

    # identity note + RFC6962 merkle note
    s.text(1346, CY - 150, "responder pubkey", 11, INK, font=MONO, anchor="middle")
    s.seal(1346, CY - 122, 9, fill=INDIGO)
    s.text(1346, CY - 100, "/.well-known/emem.json", 10, INK_SOFT, font=MONO, anchor="middle")

    # footer recipe
    s.line(200, H - 150, W - 200, H - 150, INK_SOFT, 0.7, op=0.4)
    s.text(800, H - 116, "verify offline:  digest = blake3(preimage);  ed25519.verify(sig, digest, pubkey)", 14, INK, font=MONO, anchor="middle")
    s.text(800, H - 86, "merkle tree: RFC 6962 leaf/node domain separation · duplicate leaves rejected", 11, INK_SOFT, font=MONO, anchor="middle")
    s.text(800, H - 56, "the same blake3 + ed25519 check runs in your browser at /verify", 11, INK_SOFT, font=MONO, anchor="middle")
    s.save(f"{OUT}/10-trust-plane.svg")
    return "10-trust-plane"


REG = {"01-architecture": architecture, "08-decentralised": federation, "10-trust-plane": trust_plane}


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
