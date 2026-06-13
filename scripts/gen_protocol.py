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


def _header(s, title, sub):
    s.paper(); s.border()
    s.text(80, 56, title, 30, INK, font=SERIF)
    s.text(82, 88, sub, 13, INK_SOFT, font=MONO)
    fy = 110
    s.line(82, fy, s.W - 82, fy, INK_SOFT, 0.7, op=0.45)
    x = 82
    while x <= s.W - 82:
        s.dot(x, fy, 1.4, INK, op=0.5); x += 22
    bx = 150
    while bx < s.W - 120:
        for k in (-1, 0, 1):
            s.qpath((bx, fy), (bx + k * 7, fy - 13), (bx + k * 11, fy - 2), INK, 1.0)
        s.dot(bx, fy - 7, 1.9, TURMERIC, op=0.85)
        bx += 150


def _pipeline(name, seed, title, sub, stages, endlabel, endsub, footer):
    s = Svg(1600, 1040, seed=seed); W, H = s.W, s.H
    _header(s, title, sub)
    ys = 540
    x0, x1 = 200, 1120
    n = len(stages)
    xs = [x0 + (x1 - x0) * i / (n - 1) for i in range(n)]
    for i, (lab, sub2, col) in enumerate(stages):
        if i:
            s.flow((xs[i - 1] + 32, ys), (xs[i] - 32, ys), INK_SOFT, 1.2, op=0.7)
        s.roundel(xs[i], ys, 27, col, petal=(i % 2 == 0))
        s.text(xs[i], ys + 44, lab, 11, INK, font=MONO, anchor="middle")
        if sub2:
            s.text(xs[i], ys + 58, sub2, 9.5, INK_SOFT, font=MONO, anchor="middle")
    rx = 1372
    s.flow((xs[-1] + 32, ys), (rx - 88, ys), LEAF, 1.5, op=0.85, glow=True)
    s.lotus(rx, ys, 70); s.seal(rx + 58, ys - 58, 11, fill=LEAF)
    s.text(rx, ys + 94, endlabel, 12, INK, font=MONO, anchor="middle", weight="bold")
    s.text(rx, ys + 110, endsub, 10, INK_SOFT, font=MONO, anchor="middle")
    s.line(200, H - 110, W - 200, H - 110, INK_SOFT, 0.7, op=0.4)
    s.text(800, H - 78, footer, 15, INK, font=SERIF, anchor="middle", italic=True)
    s.text(800, H - 50, "anyone re-checks the receipt at /verify · no trust in the server", 11, INK_SOFT, font=MONO, anchor="middle")
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


def agent_loop():
    s = Svg(1600, 1040, seed=4); W, H = s.W, s.H
    _header(s, "The agent loop", "ask, recall, cite, act, and the memory is better for the next turn")
    CX, CY = 800, 580; R = 250
    stages = [("ask", "free text", INDIGO), ("recall", "signed fact", TEAL),
              ("cite", "fact_cid", LEAF), ("act", "in the world", TURMERIC),
              ("write back", "a new memory", LAC)]
    pos = []
    for i in range(len(stages)):
        th = math.radians(-90 + i * 360 / len(stages))
        pos.append((CX + R * math.cos(th), CY + R * math.sin(th) * 0.92))
    for i in range(len(stages)):
        a, b = pos[i], pos[(i + 1) % len(stages)]
        mx, my = (a[0] + b[0]) / 2, (a[1] + b[1]) / 2
        ox, oy = mx - CX, my - CY; ol = math.hypot(ox, oy) or 1
        s.qpath(a, (mx + ox / ol * 40, my + oy / ol * 40), b, LEAF, 1.4, op=0.7, cls="flowline")
        s.dot(mx + ox / ol * 40, my + oy / ol * 40, 2.4, LEAF, op=0.8)
    for i, (lab, sub, col) in enumerate(stages):
        x, y = pos[i]
        s.roundel(x, y, 30, col, signed=(lab == "recall"), petal=(i % 2 == 0))
        s.text(x, y + 46, lab, 12, INK, font=MONO, anchor="middle")
        s.text(x, y + 60, sub, 10, INK_SOFT, font=MONO, anchor="middle")
    s.lotus(CX, CY, 58)
    s.text(CX, CY + 80, "the shared memory", 12, INK, font=MONO, anchor="middle")
    s.text(CX, CY + 96, "grows each turn", 10, INK_SOFT, font=MONO, anchor="middle")
    s.line(200, H - 96, W - 200, H - 96, INK_SOFT, 0.7, op=0.4)
    s.text(800, H - 64, "Every claim the agent makes carries a handle the next agent can re-check.", 15, INK, font=SERIF, anchor="middle", italic=True)
    s.save(f"{OUT}/04-agent-loop.svg"); return "04-agent-loop"


def fact_to_reasoning():
    s = Svg(1600, 1040, seed=5); W, H = s.W, s.H
    _header(s, "Grounding in the context window", "a recalled fact the model can point at, instead of a number it made up")
    CY = 540
    # left: a signed fact card
    px, py, pw, ph = 110, 360, 380, 220
    s.rect(px, py, pw, ph, fill=PAPER, op=0.62, stroke=INK, sw=1.6, rx=10)
    s.seal(px + 26, py + 26, 7, fill=LEAF)
    s.text(px + 44, py + 31, "signed fact", 13, INK, font=MONO, weight="bold")
    s.line(px + 18, py + 44, px + pw - 18, py + 44, INK_SOFT, 0.8, op=0.6)
    rows = [("band", "copdem30m.elevation_mean"), ("cell", "defi.zb5c4.guxe.nuxe"),
            ("value", "1609 m"), ("fact_cid", "72wdchiyurfr…")]
    for i, (k, v) in enumerate(rows):
        ry = py + 76 + i * 34
        s.text(px + 28, ry, k, 12, INK_SOFT, font=MONO)
        s.text(px + pw - 20, ry, v, 12, INK, font=MONO, anchor="end")
    # arrow into the context window
    cwx, cwy, cww, cwh = 700, 320, 540, 360
    s.flow((px + pw + 4, CY), (cwx - 8, CY), LEAF, 1.5, op=0.85, glow=True)
    s.rect(cwx, cwy, cww, cwh, fill=PAPER, op=0.5, stroke=INK, sw=1.8, rx=12)
    s.dot_ring(cwx + cww / 2, cwy, int(cww / 14), 0, 0, INK)
    nrv = int(cww / 16)
    for k in range(nrv):
        s.dot(cwx + (k + 0.5) * cww / nrv, cwy, 1.0, INK, op=0.4)
        s.dot(cwx + (k + 0.5) * cww / nrv, cwy + cwh, 1.0, INK, op=0.4)
    s.text(cwx + cww / 2, cwy + 30, "context window", 13, INK, font=MONO, anchor="middle", weight="bold")
    s.text(cwx + 28, cwy + 80, "USER:  how high is Denver?", 12, INK_SOFT, font=MONO)
    s.text(cwx + 28, cwy + 120, "FACT:  elevation 1609 m", 12, LEAF, font=MONO)
    s.text(cwx + 28, cwy + 140, "       cell defi.zb5c4… · 72wdchiyurfr…", 10, INK_SOFT, font=MONO)
    s.text(cwx + 28, cwy + 188, "ASSISTANT:", 12, INK_SOFT, font=MONO)
    s.text(cwx + 28, cwy + 210, "Denver sits at 1609 m, mile-high.", 12, INK, font=MONO)
    s.text(cwx + 28, cwy + 230, "Citation: 72wdchiyurfr…", 10, INDIGO, font=MONO)
    s.lotus(cwx + cww / 2, cwy + cwh + 6, 8)  # tiny seal accent
    s.line(200, H - 96, W - 200, H - 96, INK_SOFT, 0.7, op=0.4)
    s.text(800, H - 64, "The answer points at a fact id, so the reader can pull the same bytes and check.", 15, INK, font=SERIF, anchor="middle", italic=True)
    s.save(f"{OUT}/05-fact-to-reasoning.svg"); return "05-fact-to-reasoning"


def memory_vs_stac():
    s = Svg(1600, 1040, seed=6); W, H = s.W, s.H
    _header(s, "What the agent carries", "raw tiles make the agent do the work; emem signs the answer once")
    # top row: raw STAC (long chain, uncited)
    ty = 320
    stac = ["pick a scene", "range-read pixels", "undo scaling", "choose a mask", "a number"]
    x0, x1 = 220, 1180
    xs = [x0 + (x1 - x0) * i / (len(stac) - 1) for i in range(len(stac))]
    s.text(150, ty - 70, "raw STAC", 13, INK, font=MONO)
    s.text(150, ty - 54, "you carry it", 10, INK_SOFT, font=MONO)
    for i, lab in enumerate(stac):
        if i:
            s.flow((xs[i - 1] + 24, ty), (xs[i] - 24, ty), INK_SOFT, 1.0, op=0.6)
        s.roundel(xs[i], ty, 20, INK_SOFT if i < len(stac) - 1 else VERMIL)
        s.text(xs[i], ty + 38, lab, 10.5, INK, font=MONO, anchor="middle")
    s.text(1300, ty, "uncited", 12, VERMIL, font=MONO, anchor="middle")
    s.text(1300, ty + 16, "no handle", 10, INK_SOFT, font=MONO, anchor="middle")
    # bottom row: emem (one recall -> signed)
    by = 640
    s.text(150, by - 70, "emem", 13, INK, font=MONO)
    s.text(150, by - 54, "it carries it", 10, INK_SOFT, font=MONO)
    s.roundel(360, by, 26, INDIGO, petal=True)
    s.text(360, by + 44, "one recall", 11, INK, font=MONO, anchor="middle")
    s.flow((392, by), (1180, by), LEAF, 1.6, op=0.85, glow=True)
    s.lotus(1300, by, 66); s.seal(1300 + 56, by - 56, 11, fill=LEAF)
    s.text(1300, by + 92, "signed number", 11, INK, font=MONO, anchor="middle")
    s.text(1300, by + 108, "+ fact_cid", 10, INK_SOFT, font=MONO, anchor="middle")
    s.line(200, H - 96, W - 200, H - 96, INK_SOFT, 0.7, op=0.4)
    s.text(800, H - 64, "emem does the scene-picking, scaling, and masking once, and signs the result.", 15, INK, font=SERIF, anchor="middle", italic=True)
    s.save(f"{OUT}/06-memory-vs-stac.svg"); return "06-memory-vs-stac"


def cite_economy():
    s = Svg(1600, 1040, seed=7); W, H = s.W, s.H
    _header(s, "The cite economy", "a fact that gets used gets found; citation is the discovery signal")
    CX, CY = 800, 580; R = 250
    stages = [("attester", "signs a fact", INDIGO), ("content id", "names the bytes", TEAL),
              ("agent cites", "the CID", LEAF), ("registry", "CAS update", TURMERIC),
              ("score grows", "more cites", LAC), ("discovery", "favours it", VERMIL)]
    pos = []
    for i in range(len(stages)):
        th = math.radians(-90 + i * 360 / len(stages))
        pos.append((CX + R * math.cos(th), CY + R * math.sin(th) * 0.9))
    for i in range(len(stages)):
        a, b = pos[i], pos[(i + 1) % len(stages)]
        mx, my = (a[0] + b[0]) / 2, (a[1] + b[1]) / 2
        ox, oy = mx - CX, my - CY; ol = math.hypot(ox, oy) or 1
        s.qpath(a, (mx + ox / ol * 38, my + oy / ol * 38), b, INK_SOFT, 1.3, op=0.65, cls="flowline")
        s.dot(mx + ox / ol * 38, my + oy / ol * 38, 2.2, TURMERIC, op=0.8)
    for i, (lab, sub, col) in enumerate(stages):
        x, y = pos[i]
        s.roundel(x, y, 28, col, signed=(lab == "attester"), petal=(i % 2 == 0))
        s.text(x, y + 44, lab, 11.5, INK, font=MONO, anchor="middle")
        s.text(x, y + 58, sub, 10, INK_SOFT, font=MONO, anchor="middle")
    s.lotus(CX, CY, 54)
    s.text(CX, CY + 76, "the fact", 12, INK, font=MONO, anchor="middle")
    s.line(200, H - 96, W - 200, H - 96, INK_SOFT, 0.7, op=0.4)
    s.text(800, H - 64, "The more an answer is cited, the easier the next agent finds it.", 15, INK, font=SERIF, anchor="middle", italic=True)
    s.save(f"{OUT}/07-cite-economy.svg"); return "07-cite-economy"


def address_algebra():
    s = Svg(1600, 1040, seed=9); W, H = s.W, s.H
    _header(s, "The address algebra", "four wire forms, each content-stable across every responder")
    rows = [("cell", "64", "four base-1024 bigrams", "defi.zb493.xuqA.zcb5f", INDIGO),
            ("tslot", "64", "base32-nopad leb128, t. prefix", "t.aaaaagy", TEAL),
            ("cid", "32 B blake3", "base32-nopad-lowercase", "qi3jo4sq…l2hgjtwm", LEAF),
            ("vec", "1792-D fp16", "12-byte prefix in receipts", "full vector via recall", LAC)]
    tx, ty, tw = 200, 260, 1200
    rh = 110
    # header
    cols = [tx + 40, tx + 230, tx + 470, tx + 850]
    for cx2, lab in zip(cols, ["field", "bits", "wire form", "example"]):
        s.text(cx2, ty - 16, lab, 11, INK_SOFT, font=MONO)
    s.line(tx, ty, tx + tw, ty, INK, 1.4)
    for i, (f, bits, wire, ex, col) in enumerate(rows):
        ry = ty + 40 + i * rh
        s.seal(cols[0] - 16, ry - 5, 8, fill=col)
        s.text(cols[0] + 6, ry, f, 16, INK, font=MONO, weight="bold")
        s.text(cols[1], ry, bits, 13, INK, font=MONO)
        s.text(cols[2], ry, wire, 13, INK, font=MONO)
        s.text(cols[3], ry, ex, 13, col, font=MONO)
        if i:
            s.line(tx, ry - rh / 2 - 8, tx + tw, ry - rh / 2 - 8, INK_SOFT, 0.7, op=0.4)
    s.line(tx, ty + 40 + len(rows) * rh - rh / 2 - 8, tx + tw, ty + 40 + len(rows) * rh - rh / 2 - 8, INK_SOFT, 0.7, op=0.4)
    s.text(800, H - 96, "The active grid is about 9.55 m at the equator; adjacent cells share a string prefix,", 14, INK, font=SERIF, anchor="middle", italic=True)
    s.text(800, H - 70, "so an LLM that emits defi.zb493… already lands in roughly the right place.", 14, INK, font=SERIF, anchor="middle", italic=True)
    s.save(f"{OUT}/09-address-algebra.svg"); return "09-address-algebra"


def _encoders(name, seed, title, sub, footer):
    s = Svg(1600, 1040, seed=seed); W, H = s.W, s.H
    _header(s, title, sub)
    # orbit band (top): satellites / encoders
    oy = 300
    sats = [("Sentinel-2", TEAL), ("Sentinel-1", INDIGO), ("HLS", LEAF), ("MODIS", TURMERIC), ("DEM", LAC)]
    xs = [300 + i * 250 for i in range(len(sats))]
    s.text(150, oy, "in orbit", 13, INK, font=MONO)
    for i, (lab, col) in enumerate(sats):
        x = xs[i]
        # tiny satellite: a body + two panels
        s.rect(x - 9, oy - 9, 18, 18, fill=col, stroke=INK, sw=1.4, rx=3)
        s.rect(x - 26, oy - 6, 12, 12, fill=PAPER, stroke=INK, sw=1.0)
        s.rect(x + 14, oy - 6, 12, 12, fill=PAPER, stroke=INK, sw=1.0)
        s.text(x, oy + 30, lab, 10, INK, font=MONO, anchor="middle")
        s.flow((x, oy + 14), (800, 560), LEAF if i % 2 else INK_SOFT, 1.0, op=0.5, beads=False)
    # ground (centre): the responder fuses + signs
    s.lotus(800, 560, 86)
    s.seal(800 + 72, 560 - 72, 12, fill=LEAF)
    s.text(800, 560 + 118, "the responder", 13, INK, font=MONO, anchor="middle", weight="bold")
    s.text(800, 560 + 135, "decodes + fuses + signs", 11, INK_SOFT, font=MONO, anchor="middle")
    # encoders ring
    encs = [("Clay v1.5", INDIGO), ("Prithvi-EO-2", TEAL), ("Tessera", LEAF), ("Galileo", LAC)]
    for i, (lab, col) in enumerate(encs):
        th = math.radians(-90 + i * 90)
        x, y = 800 + 175 * math.cos(th), 560 + 175 * math.sin(th) * 0.9
        s.line(800, 560, x, y, INK_SOFT, 0.8, op=0.4)
        s.roundel(x, y, 18, col, signed=True)
        s.text(x, y + 30, lab, 10, INK, font=MONO, anchor="middle")
    s.line(200, H - 96, W - 200, H - 96, INK_SOFT, 0.7, op=0.4)
    s.text(800, H - 64, footer, 15, INK, font=SERIF, anchor="middle", italic=True)
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


REG = {"01-architecture": architecture, "08-decentralised": federation, "10-trust-plane": trust_plane,
       "02-data-flow": data_flow, "03-anatomy-of-a-request": anatomy, "04-agent-loop": agent_loop,
       "05-fact-to-reasoning": fact_to_reasoning, "06-memory-vs-stac": memory_vs_stac,
       "07-cite-economy": cite_economy, "09-address-algebra": address_algebra,
       "31-encoders-in-orbit-decoders-on-ground": encoders_split, "33-fusion-orbit-and-ground": fusion}


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
