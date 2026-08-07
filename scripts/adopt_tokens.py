#!/usr/bin/env python3
"""Move one page off its private token block and onto /tokens.css.

Why this exists
---------------
Every page in web/ carried its own copy of the same :root block. They drifted,
because a copy always does: /guard set --t-base to .84375rem, so its body copy
rendered at 13.5px while the homepage rendered at 16px, and neither page could
tell you which was intended. The web directive asks for one design-token file.
This does the mechanical half of moving a page to it so the judgement can go
into the parts that actually need judgement.

What it does
------------
1. Adds the display face to the page's Google Fonts request and links
   /tokens.css after it.
2. Deletes the page's own :root block and its dark-mode copy. Every value they
   defined is in the shared file, so this is a removal, not a substitution.
3. Rewrites literal font sizes onto the shared scale, rounding UP, because the
   complaint that started this was that the site reads small.

What it does NOT do
-------------------
Anything that needs an eye: which headings take the display face, where a link
colour is missing, whether a section grid leaves a void. Those differ per page
and are done by hand afterwards. Run with --check first; it prints what would
change and touches nothing.

Usage
-----
  python3 scripts/adopt_tokens.py web/solutions.html --check
  python3 scripts/adopt_tokens.py web/solutions.html
"""

from __future__ import annotations

import argparse
import collections
import re
import sys

MONO_LINK = ('<link rel=stylesheet href="https://fonts.googleapis.com/css2?'
             'family=JetBrains+Mono:ital,wght@0,200..800;1,200..800&display=swap">')
BOTH_LINK = ('<link rel=stylesheet href="https://fonts.googleapis.com/css2?'
             'family=JetBrains+Mono:ital,wght@0,200..800;1,200..800'
             '&family=Newsreader:ital,opsz,wght@0,6..72,300..700;1,6..72,300..700'
             '&display=swap">\n<link rel=stylesheet href="/tokens.css">')

# Every literal maps UP to the nearest rung. Rounding down would re-create the
# problem this is here to fix.
SCALE = {
    '.6rem': '--t-3xs', '.62rem': '--t-3xs', '.64rem': '--t-3xs', '.65rem': '--t-3xs',
    '.66rem': '--t-3xs', '.68rem': '--t-3xs', '9px': '--t-3xs', '10px': '--t-3xs',
    '11px': '--t-3xs',
    '.7rem': '--t-2xs', '.72rem': '--t-2xs', '.74rem': '--t-2xs', '.75rem': '--t-2xs',
    '.76rem': '--t-2xs', '.78rem': '--t-2xs', '12px': '--t-2xs',
    '.8rem': '--t-xs', '.82rem': '--t-xs', '.84rem': '--t-xs', '.85rem': '--t-xs',
    '.86rem': '--t-xs', '.88rem': '--t-xs', '13px': '--t-xs', '14px': '--t-xs',
    '.9rem': '--t-sm', '.92rem': '--t-sm', '.94rem': '--t-sm', '.95rem': '--t-sm',
    '15px': '--t-sm',
    '.96rem': '--t-base', '.98rem': '--t-base', '1rem': '--t-base', '16px': '--t-base',
    '1.05rem': '--t-md', '1.06rem': '--t-md', '1.08rem': '--t-md', '1.1rem': '--t-md',
    '1.2rem': '--t-lg', '1.25rem': '--t-lg', '1.3rem': '--t-lg',
    '1.5rem': '--t-xl', '1.6rem': '--t-xl', '2rem': '--t-2xl',
}


def migrate(src):
    notes = []
    s = src

    if '/tokens.css' in s:
        notes.append("already links /tokens.css")
    elif MONO_LINK in s:
        s = s.replace(MONO_LINK, BOTH_LINK, 1)
        notes.append("linked /tokens.css and the display face")
    else:
        notes.append("WARNING: the mono font link was not found verbatim; "
                     "link /tokens.css by hand")

    root = re.search(r':root\s*\{.*?\n\}', s, re.S)
    if root and '--t-base' in root.group(0):
        s = s.replace(root.group(0), ':root{ /* tokens come from /tokens.css */ }', 1)
        notes.append(f"removed a local :root of {len(root.group(0))} chars")
    else:
        notes.append("no local :root with a scale in it")

    # Only when the page speaks the same token names the shared file defines.
    # Six pages use a --bg/--fg namespace of their own, and removing their dark
    # :root left them with no dark palette at all: tokens.css defines --paper
    # and --ink, which those pages never read. /verify shipped a light body on
    # a dark --paper for one deploy because this check was not here.
    shared_namespace = '--paper' in src
    dark = re.search(
        r'@media \(prefers-color-scheme: dark\)\s*\{\s*:root\s*\{.*?\n\s*\}\s*\n?\s*\}',
        s, re.S)
    if dark and shared_namespace:
        s = s.replace(dark.group(0), '', 1)
        notes.append(f"removed a local dark-mode :root of {len(dark.group(0))} chars")
    elif dark:
        notes.append("KEPT the local dark-mode :root: this page uses its own "
                     "token names, which the shared file does not define")

    hits = collections.Counter()

    def sub(m):
        v = m.group(1).strip()
        tok = SCALE.get(v)
        if not tok:
            return m.group(0)
        hits[v] += 1
        return f'font-size:var({tok})'

    s = re.sub(r'font-size:\s*([0-9.]+(?:rem|px))(?=[;}\s])', sub, s)
    if hits:
        notes.append(f"{sum(hits.values())} literal sizes onto the scale: "
                     + ", ".join(f"{k} x{v}" for k, v in hits.most_common()))
    left = sorted(set(re.findall(r'font-size:\s*([0-9.]+(?:rem|px))(?=[;}\s])', s)))
    if left:
        notes.append(f"still literal, decide by hand: {left}")
    return s, notes


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("page")
    ap.add_argument("--check", action="store_true",
                    help="print what would change and write nothing")
    a = ap.parse_args()

    try:
        src = open(a.page, encoding="utf-8").read()
    except OSError as e:
        print(f"adopt-tokens: cannot read {a.page}: {e}", file=sys.stderr)
        return 2

    out, notes = migrate(src)
    print(f"{a.page}:")
    for n in notes:
        print(f"  {n}")
    if a.check:
        print("  (--check, nothing written)")
        return 0
    if out == src:
        print("  nothing to do")
        return 0
    open(a.page, "w", encoding="utf-8").write(out)
    print("  written. Now do the eye work: headings onto the display face, "
          "link colours, and any grid that leaves a void.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
