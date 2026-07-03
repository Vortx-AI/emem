#!/usr/bin/env python3
"""Shared-memory diagrams in the Mithila SVG hand: the one-memory hero scene,
the two-agent memt handoff, and the compaction-survival panel.

Vector SVG into docs/diagrams/, plus the 1600x1040 PNG twins under
docs/diagrams/png/ that the README embeds. Token and character counts in the
handoff panel are measured, not asserted: 79 chars via len(), 46-49 BPE
tokens via tiktoken cl100k_base/o200k_base against the live token
memt:defi.zb493.xuqA.zcb5f:yqbolgeoycqkvj3zkxukb4bjw4odhpwvfzqo3fbgwf4spk45zala
and ~1,600 tokens for the full signed response it stands in for
(3,979 bytes live on 2026-07-02; the payload grows as the cell accrues
attested bands, so re-measure rather than trust this comment).

Usage: python3 scripts/gen_shared_memory.py [name ...]
"""
import math, os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from _mithila_svg import (Svg, INK, INK_SOFT, TURMERIC, VERMIL, INDIGO, LAC,
                          LEAF, TEAL, GOLD_PALE, PAPER, DYES, SERIF, MONO)
OUT = "docs/diagrams"


def _ground(s, avoid=None, top=176):
    s.paper(); s.fill_ground(avoid=avoid or [], top=top); s.border()


def _header(s, title, sub):
    s.text(78, 108, title, 30, INK, font=SERIF)
    s.text(80, 142, sub, 13, INK_SOFT, font=MONO)
    s.frieze(168, 78, s.W - 78)


def _footer(s, caption, subcaption=None):
    W, H = s.W, s.H
    y = H - 140 if subcaption else H - 112
    s.line(220, y, W - 220, y, INK_SOFT, 0.8, op=0.45)
    s.dot(220, y, 2.4, VERMIL); s.dot(W - 220, y, 2.4, VERMIL)
    s.text(W / 2, y + 36, caption, 16, INK, font=SERIF, anchor="middle", italic=True)
    if subcaption:
        s.text(W / 2, y + 62, subcaption, 11.5, INK_SOFT, font=MONO, anchor="middle")


def _agent(s, cx, cy, col, label, sublabel=None, r=26):
    """An agent glyph: the client eye from the federation scene, scaled up."""
    s.circle(cx, cy, r * 1.55, fill=col, op=0.10, glow=True)
    s.circle(cx, cy, r, fill=PAPER, stroke=INK, sw=2.0)
    s.qpath((cx - r * 0.62, cy), (cx, cy - r * 0.46), (cx + r * 0.62, cy), INK, 1.4)
    s.qpath((cx - r * 0.62, cy), (cx, cy + r * 0.46), (cx + r * 0.62, cy), INK, 1.4)
    s.circle(cx, cy, r * 0.26, fill=col, stroke=INK, sw=1.1)
    s.dot_ring(cx, cy, r * 1.28, 10, 1.4, INK, phase=cx, op=0.6)
    s.text(cx, cy - r - 18, label, 12.5, INK, font=MONO, anchor="middle", weight="bold")
    if sublabel:
        s.text(cx, cy + r + 26, sublabel, 10.5, INK_SOFT, font=MONO, anchor="middle")


def _card(s, x, y, w, h, title, title_col=INK):
    """The house ledger card: paper fill, double rule."""
    s.rect(x, y, w, h, fill=PAPER, op=0.82, stroke=INK, sw=2.2, rx=12)
    s.rect(x + 5, y + 5, w - 10, h - 10, stroke=INK, sw=0.8, rx=9, op=0.5)
    s.text(x + 24, y + 34, title, 13.5, title_col, font=MONO, weight="bold")
    s.line(x + 18, y + 48, x + w - 18, y + 48, INK_SOFT, 1.0, op=0.6)


def shared_memory():
    """One memory, many agents: writes flow in signed, recalls flow out,
    every agent checks for itself. The hero scene for the README."""
    s = Svg(1600, 1040, seed=34); W, H = s.W, s.H
    CX, CY = 800, 560; LR = 104
    LOGY = CY + 268

    orgs = [
        ("org A", INDIGO, [(232, 336), (356, 260)]),
        ("org B", TEAL, [(1244, 336), (1368, 260)]),
        ("org C", LAC, [(238, 800)]),
        ("auditor, a year later", VERMIL, [(1352, 800)]),
    ]
    avoid = [(CX, CY, LR + 66), (W / 2, 150, 620), (W / 2, H - 90, 660),
             (CX, LOGY, 380)]
    for (_, _, pts) in orgs:
        for (x, y) in pts:
            avoid.append((x, y, 92))
    _ground(s, avoid=avoid)
    _header(s, "One memory, many agents",
            "any agent writes signed facts · any agent recalls them · every agent checks for itself")

    # flows between agents and the memory. Writes are turmeric (attest),
    # reads leaf-green (signed recall), the audit read vermilion-dashed.
    def link(x, y, col, outward, bow):
        a, b = ((x, y), (CX, CY)) if not outward else ((CX, CY), (x, y))
        # trim the endpoints off the glyphs
        dx, dy = b[0] - a[0], b[1] - a[1]
        L = math.hypot(dx, dy) or 1
        a = (a[0] + dx / L * 56, a[1] + dy / L * 56)
        b = (b[0] - dx / L * (LR + 42 if outward is False else 60), b[1] - dy / L * (LR + 42 if outward is False else 60))
        s.flow(a, b, col, 1.5, op=0.85, bow=bow, glow=True)

    # org A writes, org B recalls, org C both, auditor re-checks
    link(232, 336, TURMERIC, False, 24)
    link(356, 260, TURMERIC, False, -18)
    link(1244, 336, LEAF, True, -24)
    link(1368, 260, LEAF, True, 18)
    link(238, 800, TURMERIC, False, -22)
    link(238, 800, LEAF, True, 40)
    link(1352, 800, VERMIL, True, -26)

    for (label, col, pts) in orgs:
        for i, (x, y) in enumerate(pts):
            _agent(s, x, y, col, label if i == 0 else "", None)

    # verbs on the flows
    s.text(492, 330, "attest: signed write", 11.5, TURMERIC, font=MONO, weight="bold")
    s.text(1108, 330, "recall: signed fact", 11.5, LEAF, font=MONO, anchor="end", weight="bold")
    s.text(470, 726, "write + read", 11.5, INK_SOFT, font=MONO)
    s.text(1130, 726, "verify offline", 11.5, VERMIL, font=MONO, anchor="end", weight="bold")

    # the memory itself
    s.lotus(CX, CY, LR)
    s.seal(CX + LR * 0.86, CY - LR * 0.86, 12, fill=LEAF)
    s.text(CX, CY + LR * 1.5 + 24, "the shared memory", 13.5, INK, font=MONO, anchor="middle", weight="bold")

    # append-only log strip under the lotus
    xs = [CX - 240 + i * 96 for i in range(6)]
    for i, x in enumerate(xs):
        if i:
            s.line(xs[i - 1] + 18, LOGY, x - 18, LOGY, INK, 1.6)
            s.dot((xs[i - 1] + x) / 2, LOGY, 1.8, INK)
        s.seal(x, LOGY, 15, fill=DYES[i % len(DYES)])
    s.text(CX, LOGY + 36, "append-only · every fact signed · addressed by its content", 11.5,
           INK_SOFT, font=MONO, anchor="middle")

    _footer(s, "A fact written by one agent is memory for every agent that comes after.",
            "no shared password, no shared database to trust: each reader re-derives the id and checks the signature")
    s.save(f"{OUT}/34-shared-memory.svg")
    return "34-shared-memory"


def two_agents():
    """The memt handoff: agent A recalls, hands a token to agent B, agent B
    resolves and verifies without trusting A. Measured numbers on the card."""
    s = Svg(1600, 1040, seed=35); W, H = s.W, s.H
    AX, AY = 250, 460
    BX, BY = 1350, 460
    MX, MY = 800, 730; LR = 84
    tx, ty, tw, th = 508, 306, 584, 196

    avoid = [(AX, AY, 110), (BX, BY, 110), (MX, MY, LR + 62),
             (tx + tw / 2, ty + th / 2, tw * 0.60), (W / 2, 150, 620),
             (W / 2, H - 90, 660)]
    _ground(s, avoid=avoid)
    _header(s, "Two agents, one memory",
            "agent A hands agent B one line of text · agent B verifies it without trusting agent A")

    _agent(s, AX, AY, INDIGO, "agent A", "company A", r=30)
    _agent(s, BX, BY, TEAL, "agent B", "company B", r=30)

    # 1. A recalls from the shared memory (signed fact + receipt back)
    s.flow((AX + 44, AY + 64), (MX - LR - 30, MY - 44), INK_SOFT, 1.3, op=0.65, bow=26)
    s.flow((MX - LR - 16, MY - 62), (AX + 62, AY + 44), LEAF, 1.5, op=0.85, bow=-26, glow=True)
    s.text(376, 636, "1 · recall", 12, INK, font=MONO, weight="bold")
    s.text(376, 656, "signed fact + receipt", 10.5, INK_SOFT, font=MONO)

    # 2. the token card A hands to B
    _card(s, tx, ty, tw, th, "the whole handoff · one line", title_col=INK)
    s.text(tx + 24, ty + 82, "memt:defi.zb493.xuqA.zcb5f:", 13, INDIGO, font=MONO, weight="bold")
    s.text(tx + 24, ty + 106, "yqbolgeoycqkvj3zkxukb4bjw4odhpwvfzqo3fbgwf4spk45zala", 11.5, INK, font=MONO)
    s.text(tx + 24, ty + 140, "79 characters · about 50 BPE tokens", 10.5, INK_SOFT, font=MONO)
    s.text(tx + 24, ty + 162, "stands in for a signed response of ~1,600 tokens", 10.5, INK_SOFT, font=MONO)
    s.seal(tx + tw - 34, ty + 30, 9, fill=LEAF)
    s.flow((AX + 62, AY - 10), (tx - 8, ty + th / 2), TURMERIC, 1.6, op=0.9, bow=-30, glow=True)
    s.flow((tx + tw + 8, ty + th / 2), (BX - 62, BY - 10), TURMERIC, 1.6, op=0.9, bow=-30, glow=True)
    s.text(W / 2, ty - 22, "2 · paste it in a message, a log, a report", 11.5, TURMERIC,
           font=MONO, anchor="middle", weight="bold")

    # 3. B resolves against the memory, 4. verifies offline
    s.flow((BX - 44, BY + 64), (MX + LR + 30, MY - 44), INK_SOFT, 1.3, op=0.65, bow=-26)
    s.flow((MX + LR + 16, MY - 62), (BX - 62, BY + 44), LEAF, 1.5, op=0.85, bow=26, glow=True)
    s.text(1224, 636, "3 · resolve", 12, INK, font=MONO, anchor="end", weight="bold")
    s.text(1224, 656, "same bytes, any node", 10.5, INK_SOFT, font=MONO)

    # verdict chip by agent B
    vx, vy = BX - 60, BY - 128
    s.rect(vx - 130, vy - 26, 260, 52, fill=PAPER, op=0.85, stroke=LEAF, sw=2.2, rx=12)
    s.seal(vx - 104, vy, 9, fill=LEAF)
    s.text(vx - 84, vy + 5, '4 · "valid": true', 13, INK, font=MONO, weight="bold")

    # the shared memory at the bottom centre
    s.lotus(MX, MY, LR)
    s.seal(MX + LR * 0.84, MY - LR * 0.84, 11, fill=LEAF)
    s.text(MX, MY + LR * 1.5 + 22, "the shared memory · content-addressed · append-only", 11.5,
           INK, font=MONO, anchor="middle", weight="bold")

    _footer(s, "Agent B never trusts agent A: it re-derives the content id and checks the signature itself.",
            "offline, with the responder public key alone · the same check runs in a browser at /verify")
    s.save(f"{OUT}/35-two-agents-one-memory.svg")
    return "35-two-agents-one-memory"


def compaction():
    """Memory outlives the context window: as the conversation is compacted
    turn after turn, payloads fall out of context but the one-line token
    survives, and after compaction it re-hydrates to the exact signed bytes."""
    s = Svg(1600, 1040, seed=36); W, H = s.W, s.H
    MX, MY = 1258, 560; LR = 92

    # three context-window cards, shrinking left to right
    cards = [
        (96, 300, 320, 400, "turn 12", "full context"),
        (476, 348, 256, 304, "turn 60", "compacted"),
        (792, 396, 234, 208, "turn 300", "compacted"),
    ]
    avoid = [(MX, MY, LR + 66), (W / 2, 150, 620), (W / 2, H - 96, 660)]
    for (x, y, w, h, _, _) in cards:
        avoid.append((x + w / 2, y + h / 2, max(w, h) * 0.62))
    avoid.append((914, 820, 300))
    _ground(s, avoid=avoid)
    _header(s, "Memory outlives the context window",
            "compaction keeps the one-line address and drops the payload · the payload re-hydrates on demand")

    def ctx_card(x, y, w, h, turn, state, lines):
        s.rect(x, y, w, h, fill=PAPER, op=0.80, stroke=INK, sw=2.2, rx=12)
        s.rect(x + 5, y + 5, w - 10, h - 10, stroke=INK, sw=0.8, rx=9, op=0.5)
        s.text(x + 18, y + 30, turn + " · " + state, 11.5, INK, font=MONO, weight="bold")
        s.line(x + 14, y + 42, x + w - 14, y + 42, INK_SOFT, 0.9, op=0.55)
        yy = y + 68
        for (txt, col, bold) in lines:
            s.text(x + 18, yy, txt, 10.5, col, font=MONO, weight="bold" if bold else "normal")
            yy += 26

    ctx_card(*cards[0][:4], cards[0][4], cards[0][5], [
        ("USER: survey the plot", INK_SOFT, False),
        ("recall: elevation 38.3 m", INK, False),
        ("recall: ndvi 0.61", INK, False),
        ("receipt: signed, verified", LEAF, False),
        ("full JSON payloads", INK_SOFT, False),
        ("...", INK_SOFT, False),
        ("memt:defi.zb6d9...:sroz...", INDIGO, True),
    ])
    ctx_card(*cards[1][:4], cards[1][4], cards[1][5], [
        ("summary of turns 1-59", INK_SOFT, False),
        ("payloads gone", VERMIL, False),
        ("...", INK_SOFT, False),
        ("memt:defi...:sroz...", INDIGO, True),
    ])
    ctx_card(*cards[2][:4], cards[2][4], cards[2][5], [
        ("summary of summaries", INK_SOFT, False),
        ("memt:defi...:sroz...", INDIGO, True),
    ])

    # compaction arrows between cards, labelled once above the row
    for i in range(2):
        x0 = cards[i][0] + cards[i][2] + 8
        x1 = cards[i + 1][0] - 8
        s.flow((x0, 500), (x1, 500), INK_SOFT, 1.3, op=0.6)
    s.text(cards[0][0] + 4, 264, "the conversation gets compacted, turn after turn",
           11.5, INK_SOFT, font=MONO)

    # the surviving token line re-hydrates from the shared memory
    tok_x, tok_y = cards[2][0] + 24, cards[2][1] + 68 + 26
    s.flow((cards[2][0] + cards[2][2] - 6, tok_y - 6), (MX - LR - 40, MY - 20),
           TURMERIC, 1.6, op=0.9, bow=-34, glow=True)
    s.text(1042, 430, "resolve the token", 11, TURMERIC, font=MONO, weight="bold")
    s.flow((MX - LR - 26, MY + 40), (958, 748), LEAF, 1.6, op=0.9, bow=-30, glow=True)

    # re-hydrated fact card, bottom centre
    rx0, ry0, rw, rh = 700, 748, 430, 148
    s.rect(rx0, ry0, rw, rh, fill=PAPER, op=0.84, stroke=LEAF, sw=2.2, rx=12)
    s.rect(rx0 + 5, ry0 + 5, rw - 10, rh - 10, stroke=INK, sw=0.8, rx=9, op=0.5)
    s.seal(rx0 + 28, ry0 + 30, 8, fill=LEAF)
    s.text(rx0 + 46, ry0 + 36, "re-hydrated, byte for byte", 12.5, INK, font=MONO, weight="bold")
    s.line(rx0 + 16, ry0 + 50, rx0 + rw - 16, ry0 + 50, INK_SOFT, 0.9, op=0.55)
    s.text(rx0 + 24, ry0 + 80, "elevation_mean  38.27 m  · same fact_cid", 11.5, INK, font=MONO)
    s.text(rx0 + 24, ry0 + 106, '"valid": true · signature still checks', 11.5, LEAF, font=MONO, weight="bold")

    # the shared memory
    s.lotus(MX, MY, LR)
    s.seal(MX + LR * 0.84, MY - LR * 0.84, 11, fill=LEAF)
    s.text(MX, MY + LR * 1.5 + 22, "the shared memory", 12.5, INK, font=MONO, anchor="middle", weight="bold")

    _footer(s, "What the summarizer keeps is an address, not a paraphrase.",
            "a 79-character line survives any compaction · the exact signed bytes come back when needed")
    s.save(f"{OUT}/36-memory-outlives-the-context-window.svg")
    return "36-memory-outlives-the-context-window"


REG = {"34-shared-memory": shared_memory, "35-two-agents-one-memory": two_agents,
       "36-memory-outlives-the-context-window": compaction}


def main():
    import cairosvg
    names = sys.argv[1:] or list(REG)
    for n in names:
        if n not in REG:
            print("unknown:", n); continue
        REG[n]()
        # 1600x1040 PNG twin, same convention as the committed png/ set
        cairosvg.svg2png(url=f"{OUT}/{n}.svg", write_to=f"{OUT}/png/{n}.png",
                         output_width=1600)
        print("wrote", f"{OUT}/{n}.svg", "+", f"{OUT}/png/{n}.png")


if __name__ == "__main__":
    main()
