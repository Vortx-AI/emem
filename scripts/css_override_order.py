#!/usr/bin/env python3
"""Catch a media-query override that loses to the rule it means to override.

The bug
-------
Two rules of EQUAL specificity are decided by source order, and a media query
adds no specificity. So this, in one stylesheet, in this order:

    @media (max-width:860px){ .side{ position:static } }
    .side{ position:absolute }

leaves `.side` absolutely positioned on a phone. The class is present, the
media query matches, DevTools shows the rule, and it is struck through. Nothing
errors and nothing looks wrong in the source.

web/index.html produced this four times in one day: the names offset, the label
font-size, the mobile top, and the caption bar that was supposed to stop two
labels printing through the body text. The last one shipped, and an outside
reviewer measured the served page still rendering the labels absolutely
positioned and fully transparent.

What this checks
----------------
For every inline <style> block: if a declaration of property P for selector S
appears inside a media block, and a declaration of the SAME property for the
SAME selector appears later at top level, the override cannot win. Reported
with both line numbers.

This is deliberately narrow. It matches selectors textually, so it will not
catch an override defeated by a different-but-overlapping selector, and it says
so rather than implying it caught everything.

Usage:
  python3 scripts/css_override_order.py
  python3 scripts/css_override_order.py web/index.html
"""
from __future__ import annotations

import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parent.parent
RULE = re.compile(r'([^{}@/][^{}]*?)\{([^{}]*)\}', re.S)


def decls(body: str) -> set[str]:
    out = set()
    for part in body.split(";"):
        if ":" in part:
            out.add(part.split(":", 1)[0].strip().lower())
    return {d for d in out if d and not d.startswith("/*")}


def scan(css: str, line0: int) -> list[str]:
    """(selector, property) -> earliest line inside a media block, and the
    latest line at top level."""
    in_media: dict[tuple[str, str], int] = {}
    top: dict[tuple[str, str], int] = {}
    depth, i, n = 0, 0, len(css)
    media_depth = None
    for m in re.finditer(r'@media[^{]*\{|\{|\}|', css):
        pass  # placeholder; real walk below

    # simple brace walk, tracking whether we are inside an @media block
    buf, sel_start = [], 0
    pos, depth, media_at = 0, 0, None
    while pos < n:
        ch = css[pos]
        if ch == "{":
            prelude = css[sel_start:pos].strip()
            # @keyframes stop-selectors are percentages, not selectors: "50%"
            # inside one looked like a rule setting `opacity` on an element
            # called 50%. Skip the whole at-rule body rather than parse it.
            if prelude.startswith("@keyframes") or prelude.startswith("@-webkit-keyframes") \
               or prelude.startswith("@supports") is False and prelude.startswith("@") \
               and not prelude.startswith("@media"):
                d2, k = 1, pos + 1
                while k < n and d2:
                    if css[k] == "{": d2 += 1
                    elif css[k] == "}": d2 -= 1
                    k += 1
                sel_start = k
                pos = k
                continue
            if prelude.startswith("@media"):
                depth += 1
                media_at = depth
                sel_start = pos + 1
                pos += 1
                continue
            # a plain rule: find its close
            end = css.find("}", pos)
            if end < 0:
                break
            body = css[pos + 1:end]
            line = line0 + css.count("\n", 0, sel_start)
            for sel in (x.strip() for x in prelude.split(",")):
                if not sel:
                    continue
                for prop in decls(body):
                    key = (sel, prop)
                    if media_at is not None:
                        in_media.setdefault(key, line)
                    else:
                        top[key] = max(top.get(key, 0), line)
            sel_start = end + 1
            pos = end + 1
            continue
        if ch == "}":
            depth -= 1
            if media_at is not None and depth < media_at:
                media_at = None
            sel_start = pos + 1
        pos += 1

    out = []
    for key, mline in sorted(in_media.items(), key=lambda kv: kv[1]):
        tline = top.get(key)
        if tline and tline > mline:
            sel, prop = key
            out.append(f"line {mline}: @media sets `{prop}` on `{sel}`, but line {tline} "
                       f"sets it again at top level and wins (equal specificity, later rule)")
    return out


def main() -> int:
    targets = [pathlib.Path(a) for a in sys.argv[1:]] or sorted((REPO / "web").glob("*.html"))
    findings, scanned, blocks = [], 0, 0
    for path in targets:
        try:
            html = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        scanned += 1
        for m in re.finditer(r'<style[^>]*>(.*?)</style>', html, re.S):
            blocks += 1
            line0 = html.count("\n", 0, m.start(1)) + 1
            for f in scan(m.group(1), line0):
                # A path outside the repo is a legitimate input (a control
                # fixture lives in a scratch dir), and relative_to RAISES on
                # one. A checker that dies on its own control is the failure
                # mode it exists to prevent.
                try:
                    shown = path.relative_to(REPO)
                except ValueError:
                    shown = path
                findings.append(f"{shown}: {f}")

    # Reaching nothing is not agreement.
    if scanned == 0 or blocks == 0:
        print("no inline <style> blocks were read. Undetermined, not clean.")
        return 1
    print(f"css override order: {blocks} inline style block(s) across {scanned} page(s)")
    if findings:
        print(f"\n{len(findings)} override(s) that cannot win:")
        for f in findings:
            print("  ", f)
        print("\nA media query adds no specificity. Move the override AFTER the rule it")
        print("overrides, or make it more specific on purpose and say why.")
        return 1
    print("Every media-query override appears after the rule it overrides.")
    print("(Textual selector match: an override defeated by a DIFFERENT but")
    print(" overlapping selector is not covered by this check.)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
