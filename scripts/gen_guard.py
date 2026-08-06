#!/usr/bin/env python3
"""emem-guard gallery diagrams, in the Mithila SVG hand.

Four things that prose keeps failing to land, each drawn so the ARGUMENT is
the shape rather than the caption:

  40  the checkpoints      many doors, one decision. Vendor-neutrality is
                           convergence, and a list cannot show convergence.
  41  the verdict path     sign-then-log happens BEFORE the answer, inside a
                           hard budget. The order is the product.
  42  the DLP chassis      the same detection everyone else runs, plus the
                           half none of them ship. Two lanes, one difference.
  43  the deployments      hosted-advisory, self-hosted-enforcing, and the
                           split relay. Confusing these is what made a
                           nine-route table read as an overclaim.

Usage: python3 scripts/gen_guard.py [name ...]
"""
import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from _mithila_svg import (Svg, INK, INK_SOFT, TURMERIC, VERMIL, INDIGO, LAC,
                          LEAF, TEAL, PAPER2, SERIF, MONO)

OUT = "docs/diagrams"


def _ground(s, avoid=None, top=176):
    s.paper()
    s.fill_ground(avoid=avoid or [], top=top)
    s.border()


def _header(s, title, sub):
    s.text(78, 108, title, 30, INK, font=SERIF)
    s.text(80, 142, sub, 13, INK_SOFT, font=MONO)
    s.frieze(168, 78, s.W - 78)


def _footer(s, line, under):
    """Footer, lifted clear of the bottom frieze.

    The gallery's shared helper puts its second line at H-50, which sits ON
    the border band: every existing diagram has the closing line crossed out
    by its own decoration. Same rule, same dots, 40px higher.
    """
    W, H = s.W, s.H
    s.line(220, H - 156, W - 220, H - 156, INK_SOFT, 0.8, op=0.45)
    s.dot(220, H - 156, 2.4, VERMIL)
    s.dot(W - 220, H - 156, 2.4, VERMIL)
    s.text(W / 2, H - 120, line, 16, INK, font=SERIF, anchor="middle", italic=True)
    s.text(W / 2, H - 92, under, 11.5, INK_SOFT, font=MONO, anchor="middle")


def _card(s, x, y, w, h, col, title, lines, badge=None, badge_col=None):
    """A bordered panel with a title and a few mono lines."""
    s.rect(x, y, w, h, fill=PAPER2, stroke=INK_SOFT, sw=1.1, rx=6, op=0.92)
    s.rect(x, y, w, 4, fill=col, stroke="none", op=0.85)
    s.text(x + 16, y + 32, title, 13, INK, font=MONO, weight="bold")
    for i, ln in enumerate(lines):
        s.text(x + 16, y + 56 + i * 19, ln, 10.5, INK_SOFT, font=MONO)
    if badge:
        bw = 9.2 * len(badge) + 16
        s.rect(x + w - bw - 12, y + 16, bw, 21, fill=badge_col or col,
               stroke="none", rx=4, op=0.16)
        s.text(x + w - bw - 4, y + 31, badge, 10, badge_col or col, font=MONO,
               weight="bold")


# ── 40 · the checkpoints ────────────────────────────────────────────────

def checkpoints():
    """Many doors, one decision.

    The whole claim of the crate is that WHO asked does not change the answer,
    so the drawing has to converge. A table of nine rows reads as nine
    products; a fan-in reads as one.
    """
    s = Svg(1600, 1040, seed=40)
    W, H = s.W, s.H
    CX, CY = 1006, 566

    doors = [
        ("POST /verdict", "any agent, any model", INDIGO, True),
        ("POST /verdict/mcp", "MCP tools/call, or a result", TEAL, True),
        ("POST /verdict/openai", "any OpenAI-shaped client", LEAF, True),
        ("POST /verdict/cloudevent", "Knative · Dapr · Argo", TURMERIC, True),
        ("POST /verdict/policy", "OPA · Envoy ext_authz", LAC, True),
        ("POST /verdict/batch", "an archive, offline", INDIGO, True),
        ("POST /verdict/anthropic-hook", "Enterprise org, needs a secret", VERMIL, False),
        ("POST /verdict/claude-code", "runs inside the agent", VERMIL, False),
    ]
    y0, y1 = 268, 848
    ys = [y0 + (y1 - y0) * i / (len(doors) - 1) for i in range(len(doors))]
    lx = 372

    avoid = [(W / 2, 150, 620), (W / 2, H - 120, 660), (CX, CY, 240)]
    for y in ys:
        avoid.append((lx, y, 74))
    _ground(s, avoid=avoid)
    _header(s, "Nine doors, one decision",
            "seven belong to nobody · the same evidence yields the same verdict through every one")

    for i, (lab, sub, col, open_) in enumerate(doors):
        y = ys[i]
        s.roundel(lx, y, 21, col, petal=(i % 2 == 0),
                  inner=("ring" if open_ else "hatch"))
        s.text(lx + 38, y - 3, lab, 12, INK, font=MONO,
               weight="bold" if open_ else "normal")
        s.text(lx + 38, y + 15, sub, 10, INK_SOFT, font=MONO)
        # Open routes carry the signed glow; the two vendor doors do not,
        # because they are reachable only inside somebody's product.
        s.flow((lx + 300, y), (CX - 104, CY), col, 1.5 if open_ else 1.0,
               op=0.85 if open_ else 0.45, bow=(CY - y) * 0.10, glow=open_)

    s.text(196, 232, "OPEN", 11, LEAF, font=MONO, weight="bold")
    s.text(196, 250, "no vendor", 10, INK_SOFT, font=MONO)
    s.text(196, 800, "VENDOR", 11, VERMIL, font=MONO, weight="bold")
    s.text(196, 818, "needs their", 10, INK_SOFT, font=MONO)
    s.text(196, 834, "product", 10, INK_SOFT, font=MONO)

    s.lotus(CX, CY, 96)
    s.seal(CX + 78, CY - 78, 12, fill=LEAF)
    s.text(CX, CY + 148, "ONE ENGINE", 14, INK, font=MONO, anchor="middle",
           weight="bold")
    s.text(CX, CY + 170, "policy pipeline · pure · no IO", 11, INK_SOFT,
           font=MONO, anchor="middle")

    _card(s, 1236, 372, 300, 196, LEAF, "the same answer",
          ["allow / deny", "EMEM-GUARD DENY <CODE>", "  token=<t|-> fix=<fix>", "  leaf=<leaf|->",
           "", "a test asserts every door", "reaches the same verdict"],
          badge="TESTED", badge_col=LEAF)

    _footer(s, "A guard that answered differently by vendor would be discriminating between agents on the basis of their host.",
            "seven of the nine also answer on emem.dev · the two that cannot say why, and never 404")
    s.save(f"{OUT}/40-guard-checkpoints.svg")
    return "40-guard-checkpoints"


# ── 41 · the verdict path ───────────────────────────────────────────────

def verdict_path():
    """One verdict, end to end, with the budget drawn as a wall.

    The load-bearing detail is the ORDER: assemble, sign, append, THEN answer.
    Returning first and logging after is faster and produces a verdict that
    was acted on and never recorded, and a gap in an append-only log is
    indistinguishable from a deletion.
    """
    s = Svg(1600, 1040, seed=41)
    W, H = s.W, s.H
    ys = 500
    stages = [
        ("scan", "emem: tokens", INDIGO, "0 ms"),
        ("resolve", "warm only", TEAL, "≤400 ms"),
        ("modules", "fast only", TURMERIC, "≤50 ms ea"),
        ("evaluate", "first match", LEAF, "µs"),
        ("sign", "ed25519", LAC, ""),
        ("append", "chain · fsync", VERMIL, ""),
    ]
    x0, x1 = 214, 1180
    xs = [x0 + (x1 - x0) * i / (len(stages) - 1) for i in range(len(stages))]
    rx = 1418

    avoid = [(W / 2, 150, 620), (W / 2, H - 120, 660), (rx, ys, 168)]
    for x in xs:
        avoid.append((x, ys, 86))
    avoid.append((W / 2, 776, 600))
    _ground(s, avoid=avoid)
    _header(s, "One verdict, in order",
            "assemble · sign · append · THEN answer — the order is the product, not an implementation detail")

    inners = ["ring", "fish", "leaf", "hatch", "ring", "fish"]
    for i, (lab, sub, col, budget) in enumerate(stages):
        if i:
            s.flow((xs[i - 1] + 34, ys), (xs[i] - 34, ys), INK_SOFT, 1.4, op=0.7)
        s.roundel(xs[i], ys, 32, col, petal=(i % 2 == 0), inner=inners[i % 6])
        s.text(xs[i], ys + 54, lab, 12, INK, font=MONO, anchor="middle", weight="bold")
        s.text(xs[i], ys + 72, sub, 10, INK_SOFT, font=MONO, anchor="middle")
        if budget:
            s.text(xs[i], ys - 52, budget, 10, VERMIL, font=MONO, anchor="middle",
                   weight="bold")

    s.flow((xs[-1] + 34, ys), (rx - 110, ys), LEAF, 1.7, op=0.9, glow=True)
    s.lotus(rx, ys, 88)
    s.seal(rx + 72, ys - 72, 12, fill=LEAF)
    s.text(rx, ys + 142, "ANSWER", 13, INK, font=MONO, anchor="middle", weight="bold")
    s.text(rx, ys + 162, "allow / deny + leaf", 10.5, INK_SOFT, font=MONO, anchor="middle")

    # The wall. 800 ms self-budget under a 1000 ms floor: being late is not a
    # slow verdict, it is an unreachable server, and enough of those stop
    # enforcement for everyone.
    s.line(x0 - 46, ys - 128, xs[-1] + 96, ys - 128, VERMIL, 1.2, op=0.5)
    s.text(x0 - 46, ys - 140, "800 ms self-budget, under the 1000 ms floor the caller can configure",
           11, VERMIL, font=MONO)
    s.text(x0 - 46, ys - 96, "late is not slow: it is a transport failure, and enough of them stop enforcement for everyone",
           10, INK_SOFT, font=MONO)

    _card(s, 214, 700, 420, 150, INK_SOFT, "if the log cannot take it",
          ["default: answer anyway, leaf=-", "our storage must not become",
           "  the customer's outage", "opt-in: downgrade the deny"])
    _card(s, 664, 700, 420, 150, TURMERIC, "shadow mode",
          ["same evaluation, same record", "outcome=allow, evaluated=deny",
           "both signed, so a report is", "  evidence not an estimate"],
          badge="MEASURE FIRST", badge_col=TURMERIC)
    _card(s, 1114, 700, 420, 150, LEAF, "what a denial names",
          ["the code, the token, the fix", "and the leaf that proves it",
           "fetch /log/entry/<leaf> and", "  check it without asking us"])

    _footer(s, "Returning first and logging after would produce a verdict that was acted on and never recorded.",
            "a gap in an append-only log is indistinguishable from a deletion")
    s.save(f"{OUT}/41-guard-verdict-path.svg")
    return "41-guard-verdict-path"


# ── 42 · the DLP chassis ────────────────────────────────────────────────

def dlp_chassis():
    """Where an existing DLP engine plugs in, and what it gets for plugging.

    The positioning in one picture: everyone else's detection is welcome and
    ours is deliberately thin. What nobody else ships is the lane below the
    verdict, where a denial stops being a database row and becomes evidence.
    """
    s = Svg(1600, 1040, seed=42)
    W, H = s.W, s.H

    avoid = [(W / 2, 150, 640), (W / 2, H - 120, 660),
             (400, 470, 250), (1150, 470, 240), (W / 2, 774, 640)]
    _ground(s, avoid=avoid)
    _header(s, "The chassis your DLP runs on",
            "bring the detection · we make its verdicts provable · a denial as evidence, not a database row")

    # Left: how detection arrives. Three transports, one trait.
    s.text(196, 254, "DETECTION ARRIVES THREE WAYS", 11, INK, font=MONO, weight="bold")
    ways = [
        ("in-process", "impl PolicyModule", LEAF, "open engines"),
        ("webhook:<url>", "your classifier, HTTP", TEAL, "presidio · llm-guard · in-house"),
        ("sidecar:<socket>", "one line of JSON, unix", INDIGO, "closed engines, no linking"),
    ]
    for i, (lab, sub, col, note) in enumerate(ways):
        y = 306 + i * 92
        s.roundel(232, y, 22, col, petal=(i % 2 == 0), inner="leaf")
        s.text(272, y - 4, lab, 12, INK, font=MONO, weight="bold")
        s.text(272, y + 14, sub, 10, INK_SOFT, font=MONO)
        s.text(272, y + 30, note, 9.5, col, font=MONO)
        s.flow((402, y), (676, 470), col, 1.4, op=0.8, bow=(470 - y) * 0.08)

    # Centre: the pipeline. Two declarations decide where a module may run,
    # and neither is taken on trust.
    _card(s, 676, 300, 344, 340, TURMERIC, "the pipeline",
          ["native, always:", "  PROV_SIG · PROV_BYTES", "  PROV_DRIFT · CLAIM_UNGROUNDED",
           "", "modules enter as rules", "first match wins, ordered",
           "", "latency_class  fast | slow", "  slow NEVER blocks",
           "  fast that misses 50 ms x3", "  is demoted, measured not",
           "  declared",
           "data_class  text | digests", "  digests_only gets an EMPTY",
           "  transcript, not a request"],
          badge="ENFORCED", badge_col=TURMERIC)

    s.flow((1020, 470), (1076, 470), LEAF, 1.7, op=0.9, glow=True)
    s.lotus(1150, 470, 74)
    s.seal(1150 + 62, 470 - 62, 11, fill=LEAF)
    s.text(1150, 584, "SIGNED VERDICT", 12.5, INK, font=MONO, anchor="middle", weight="bold")
    s.text(1150, 604, "+ config digest of the", 10, INK_SOFT, font=MONO, anchor="middle")
    s.text(1150, 620, "exact module set", 10, INK_SOFT, font=MONO, anchor="middle")

    # Right: what the log does and does not carry. Narrow column, so the lines
    # are cut to fit rather than trusted to.
    _card(s, 1252, 296, 288, 210, VERMIL, "the log",
          ["module id + version", "an evidence DIGEST", "",
           "never what matched.", "a log of a secret",
           "scanner's hits is a", "catalogue of secrets"],
          badge="NO CONTENT", badge_col=VERMIL)

    # The bottom lane: the actual difference, drawn as two outcomes.
    s.line(196, 668, W - 196, 668, INK_SOFT, 0.8, op=0.4)
    _card(s, 196, 698, 620, 152, INK_SOFT, "everyone's verdict path today",
          ["detection → allow/deny → unsigned JSON → a mutable audit table",
           "",
           "the org's own database says what happened.",
           "nobody can prove it later. that is testimony."])
    _card(s, 856, 698, 620, 152, LEAF, "the same verdict, on this chassis",
          ["detection → allow/deny → ed25519 → append-only hash chain",
           "",
           "signatures prove each verdict genuine.",
           "the chain proves none were removed. that is evidence."],
          badge="THE DIFFERENCE", badge_col=LEAF)

    _footer(s, "We will never win a detection-accuracy race, and we do not need to: the half after the verdict is the half nobody ships.",
            "an abstain is never a block · a sidecar's outage must not become the operator's")
    s.save(f"{OUT}/42-guard-dlp-chassis.svg")
    return "42-guard-dlp-chassis"


# ── 43 · the deployments ────────────────────────────────────────────────

def deployments():
    """Three deployments that answer the same question and differ in what
    they are allowed to do about it.

    Drawn because confusing them is not hypothetical: a nine-route table
    describing a self-hosted node, read by somebody standing on the hosted
    one, read as an overclaim until they reasoned it out.
    """
    s = Svg(1600, 1040, seed=43)
    W, H = s.W, s.H
    cy = 520
    cols = [268, 800, 1332]

    avoid = [(W / 2, 150, 640), (W / 2, H - 120, 660)] + [(x, cy, 210) for x in cols]
    _ground(s, avoid=avoid)
    _header(s, "Three deployments, one engine",
            "who is in the request path decides what a verdict is allowed to do")

    panels = [
        (LEAF, "HOSTED · ADVISORY", "emem.dev", [
            "in nobody's request path",
            "blocks nothing, ever",
            "resolves against the shared corpus",
            "no verdict log: it would be",
            "  inventing an audit trail",
            "carries a receipt instead",
        ], "consult it", "curl, no account, no key"),
        (INDIGO, "SELF-HOSTED · ENFORCING", "your node", [
            "in YOUR request path",
            "a deny actually blocks",
            "your key, your log, your rules",
            "modules you chose to load",
            "/log/entry proves any verdict",
            "offline, to someone who",
            "  does not trust you",
        ], "enforce it", "no account here, no key from us"),
        (TURMERIC, "SPLIT · RELAY", "text stays home", [
            "transcript may not cross",
            "  the boundary",
            "--modules-no-text withholds it",
            "  from EVERY module",
            "digests still correlate",
            "a text-hungry module loaded",
            "  here is inert, not a leak",
        ], "when text cannot travel", "the relay sees digests, not prose"),
    ]

    for x, (col, title, who, lines, verb, note) in zip(cols, panels):
        s.roundel(x, cy - 176, 34, col, petal=True, inner="ring")
        # Clear of the roundel's spikes, which reach about 20px past r.
        s.text(x, cy - 112, title, 12.5, INK, font=MONO, anchor="middle", weight="bold")
        s.text(x, cy - 93, who, 11, col, font=MONO, anchor="middle", weight="bold")
        for i, ln in enumerate(lines):
            s.text(x, cy - 54 + i * 21, ln, 10.5, INK_SOFT, font=MONO, anchor="middle")
        s.text(x, cy + 132, verb, 13, INK, font=SERIF, anchor="middle", italic=True)
        s.text(x, cy + 154, note, 10, INK_SOFT, font=MONO, anchor="middle")

    # The one invariant that holds across all three, drawn as a band beneath.
    s.line(216, 742, W - 216, 742, INK_SOFT, 0.9, op=0.45)
    s.text(W / 2, 782, "the same in all three",
           13, INK, font=MONO, anchor="middle", weight="bold")
    invariants = [
        "a citation this node does not hold is an ALLOW, never a deny: it is indistinguishable from one minted elsewhere",
        "an uncertain case allows: we are one control among several, and a wrong deny is what gets a guard switched off",
        "claim_gating denies on ABSENCE, so it ships off and shadow is the only way to turn it on",
    ]
    for i, ln in enumerate(invariants):
        s.dot(300, 812 + i * 26, 2.6, VERMIL)
        s.text(318, 816 + i * 26, ln, 10.5, INK_SOFT, font=MONO)

    _footer(s, "The hosted node answers what a guard would say. Only a node you run can say what happens next.",
            "seven vendor-neutral routes answer on emem.dev · enforcing is one binary away")
    s.save(f"{OUT}/43-guard-deployments.svg")
    return "43-guard-deployments"


REG = {
    "40-guard-checkpoints": checkpoints,
    "41-guard-verdict-path": verdict_path,
    "42-guard-dlp-chassis": dlp_chassis,
    "43-guard-deployments": deployments,
}


def main():
    names = sys.argv[1:] or list(REG)
    for n in names:
        if n not in REG:
            print("unknown:", n)
            continue
        REG[n]()
        print("wrote", f"{OUT}/{n}.svg")


if __name__ == "__main__":
    main()
