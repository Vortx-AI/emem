#!/usr/bin/env python3
"""The design system holds: one token file, one scale, and a dark mode on every page.

Why this exists
---------------
Every page in web/ used to carry its own copy of the token block. They drifted,
because a copy always does. /guard set --t-base to .84375rem, so its body copy
rendered at 13.5px while the homepage rendered at 16px, and neither page could
tell you which was intended. Three pages requested no webfont at all and drew
in whatever monospace the reader's system happened to pick.

Consolidating that was a one-time fix. This is what keeps it fixed, because the
next person to add a page will copy an existing one, and the copy is the whole
problem.

It also encodes a mistake made while consolidating. The migration removed each
page's dark-mode :root unconditionally. Six pages use a --bg/--fg token
namespace of their own rather than the shared --paper/--ink, so for those the
removal left no dark palette at all: /verify shipped a light body over a dark
--paper, and nothing caught it because the tokens themselves were fine. Check 4
is that bug written down.

What it checks
--------------
1. Every page links /tokens.css, and requests both families.
2. No page redefines a token the shared file already defines. A page may add a
   token of its own; it may not fork one.
3. No literal font-size anywhere. Sizes come from the scale or they are drift
   waiting to happen.
4. A page that defines its own colour namespace still has a dark-mode block.
   Removing one is how /verify broke.

Exit codes
----------
  0  the system holds
  1  a page forked a token, hardcoded a size, or lost its dark mode
  2  could not run

Usage
-----
  python3 scripts/design_tokens.py
"""

from __future__ import annotations

import glob
import os
import re
import sys

TOKENS = "web/tokens.css"

# The tokens no page may fork. Type scale, spacing ladder, the ratio
# itself, and the two families. Colour is not here on purpose.
SCALE_TOKENS = frozenset(
    [f"--t-{k}" for k in ("3xs", "2xs", "xs", "sm", "base", "md", "lg",
                          "xl", "2xl", "3xl", "4xl", "5xl", "6xl")]
    + [f"--s-{i}" for i in range(1, 9)]
    + ["--phi", "--mono", "--display"])

# Pages that are one theme on purpose, so the dark-mode check does not apply.
# Each needs a reason a stranger would accept, and each was re-checked on
# 2026-08-10 by asking whether check 4 would still fail without the entry.
# tools.html was here on the reasoning that it is "a generated reference sheet
# with its own light palette". It grew a `prefers-color-scheme: dark` block at
# some point since, so the reason had quietly stopped being true and the entry
# was buying nothing: the page passes check 4 on its own. An exemption that
# exempts nothing is worse than none, because the next reader takes it as
# evidence the page has no dark mode and stops looking. Removed, and the gate
# now holds tools.html to the same rule as every other page, so a regeneration
# that drops the dark block is reported.
SINGLE_THEME = {
    "gallery.html": "a printed-plate look with its own warm paper palette; "
                    "inverting it would be a different artefact",
    "scoreboard.html": "always dark, because it is a scoreboard shown on a wall",
    "api-redoc.html": "hosts a third-party renderer that brings its own theme",
    "card.html": "a share card rendered at a fixed size for embedding",
}

# Pages that are not part of the site's visual system. Each needs a reason.
EXEMPT = {
    # channel.html was here, on the reasoning that a generated page maintains
    # its own styling. That reasoning is what broke it. The generator did not
    # write a stylesheet; it SCRAPED reference.html's inline <style> at build
    # time. When reference.html moved its tokens out to /tokens.css, the channel
    # kept every rule and lost every definition: 32 of the 33 custom properties
    # it referenced resolved to nothing, so the page served Times New Roman on a
    # transparent background with black hairlines, and no gate could see it
    # because this entry said not to look.
    #
    # "Generated" is a fact about who writes the bytes, not a reason the bytes
    # are correct. The generator can be held to the standard even when its
    # output is regenerated every two minutes: the page links /tokens.css and
    # requests both families like every other page, and if build_channel.py ever
    # forks the scale again this gate is what says so.
    # The arcade is a full-bleed 3D game canvas, not a document page. Its type
    # is sized against the WebGL stage rather than a reading measure, its
    # palette is the night sky the globe is lit against, and it has no light
    # mode to offer because there is no light-mode version of space. Holding it
    # to the document scale would mean either breaking the game's layout or
    # widening /tokens.css until it no longer constrains the 23 pages it exists
    # for. It moved into web/ from a gitignored path on 2026-08-10, which is
    # the only reason this gate can suddenly see it.
    # Re-checked 2026-08-10 by measuring what the entry actually buys: the page
    # links no /tokens.css, requests no Newsreader, forks --mono, and carries 88
    # literal font-sizes. All four checks, not one, so this is load-bearing and
    # the reason above is the reason.
    "arcade.html": "a 3D game canvas, not a document page: type is sized "
                   "against the WebGL stage and the palette is its night sky",
}


def main():
    try:
        shared = open(TOKENS, encoding="utf-8").read()
    except OSError as e:
        print(f"design-tokens: cannot read {TOKENS}: {e}", file=sys.stderr)
        return 2

    shared_names = set(re.findall(r'(--[\w-]+)\s*:', shared))
    if not shared_names:
        print(f"design-tokens: {TOKENS} defines no tokens, so this gate is "
              f"measuring nothing.", file=sys.stderr)
        return 2

    pages = sorted(glob.glob("web/*.html"))
    if not pages:
        print("design-tokens: no pages found", file=sys.stderr)
        return 2

    fails = []
    checked = 0

    # A page named in an exemption table that no longer exists is a decision
    # about a file nobody can look at. It also hides a rename: move a page and
    # its exemption silently keeps applying to nothing while the page itself is
    # checked, or worse, a new page inherits the old name and the exemption.
    present = {os.path.basename(p) for p in pages}
    for table, label in ((EXEMPT, "EXEMPT"), (SINGLE_THEME, "SINGLE_THEME")):
        for name in sorted(table):
            if name not in present:
                fails.append(f"{name} is listed in {label} but web/{name} does "
                             f"not exist; drop the entry or fix the name.")

    for path in pages:
        name = os.path.basename(path)
        if name in EXEMPT:
            continue
        checked += 1
        s = open(path, encoding="utf-8").read()

        if "/tokens.css" not in s:
            fails.append(f"{name} does not link /tokens.css, so it carries its "
                         f"own scale and will drift from every other page.")
        if "Newsreader" not in s:
            fails.append(f"{name} does not request the display face. Three pages "
                         f"requested no webfont at all and drew in whatever "
                         f"monospace the reader's system picked.")

        # A page may not fork the SCALE. Colour is deliberately not covered:
        # /gallery, /scoreboard and /tools carry palettes of their own on
        # purpose, and a dark-mode :root legitimately restates every colour.
        # The scale is different. It is the thing that drifted, and there is no
        # honest reason for one page to disagree with another about how big
        # body copy is. /404 was still setting --t-xs to 11px and --t-lg to
        # 22px, which is the small-type complaint in its original form.
        for block in re.findall(r':root\s*\{(.*?)\}', s, re.S):
            for tok in re.findall(r'(--[\w-]+)\s*:', block):
                if tok in SCALE_TOKENS and tok in shared_names:
                    fails.append(
                        f"{name} redefines {tok}. The scale lives in "
                        f"/tokens.css; a page that disagrees with it about how "
                        f"big body copy is has re-created the original bug.")

        for m in re.finditer(r'font-size:\s*([0-9.]+(?:rem|px))(?=[;}\s])', s):
            fails.append(f"{name} sets font-size:{m.group(1)} literally. "
                         f"Sizes come from the scale.")

        # Check 4. Dark mode has to actually reach the page, and there are two
        # ways for it not to.
        #
        # A page with a --bg/--fg namespace of its own never reads the shared
        # --paper/--ink, so it owns its dark palette outright.
        #
        # A page that restates the shared palette in a plain :root is worse,
        # because it looks fine. Its :root comes after the /tokens.css link and
        # carries equal specificity, so later wins and the shared dark :root is
        # overridden by a light value. /404 was dark-dead that way: the tokens
        # were right, the pairing was not, and only rendering it in a dark
        # browser showed it.
        has_dark = bool(re.search(r'@media[^{]*prefers-color-scheme:\s*dark', s))
        own_namespace = bool(re.search(r':root\s*\{[^}]*--(?:bg|fg)\s*:', s, re.S))
        restates_shared = any(
            re.search(r'--(?:paper|ink)\s*:', b)
            for b in re.findall(r':root\s*\{(.*?)\}', s, re.S))
        if (own_namespace or restates_shared) and not has_dark:
            if name in SINGLE_THEME:
                continue
            fails.append(
                f"{name} sets its own palette and has no dark-mode block, so "
                f"the shared dark :root is overridden by a light value. Either "
                f"give it a dark block or stop restating the palette.")

    print(f"design tokens: {checked} pages checked against {len(shared_names)} "
          f"shared tokens, {len(EXEMPT)} exempt")

    if fails:
        print("\nThe next page will be a copy of an existing one. That is the "
              "whole problem.")
        seen = set()
        for f in fails:
            if f in seen:
                continue
            seen.add(f)
            print(f"  {f}")
        return 1
    print("One token file, one scale, and every page has a dark mode.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
