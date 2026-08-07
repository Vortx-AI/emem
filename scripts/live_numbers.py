#!/usr/bin/env python3
"""A figure the page presents as live must not ship a number in the HTML.

Why this exists
---------------
`u4aaoieq`'s web directive (file_cid 7s3zme6mlbi3qbxybnnsf7w6be) states the
rule plainly: no number appears on the site that is not fetched from the node
and citeable, and hardcoded stats are forbidden. The homepage already agreed
with itself in two places. Its caption says "Every number below is fetched from
the running responder when this page loads; nothing here is typed in", and the
script that fills them carries the comment "a failure leaves the placeholder
rather than a lie".

The markup kept neither promise. Five tiles shipped baked numbers, and three
had gone stale against the responder they claimed to be reporting:

    104 tools     the node served 107
    130 paths     the node served 154
    707,949       the tree had grown 114,194 attestations past it

Nobody was lying on purpose. A tile is written once with whatever the number
was that day, and a static number cannot notice that the world moved. The
caption is what made it worse: a reader whose fetch failed saw a wrong figure
under a sentence promising the figure was live.

So this refuses the state that makes it possible. A live figure holds a
placeholder in the HTML and gets its value from the responder or shows nothing.

What it checks
--------------
1. Every element the page marks `data-pending` is free of digits in the source.
2. Every one of those ids is actually written by a `put(...)` call, so a
   placeholder that nothing fills can never sit there reading as pending
   forever.
3. Every `put(...)` target exists in the markup, so a rename cannot quietly
   turn a live figure into a dead one.

What it deliberately does NOT check
-----------------------------------
Ordinary prose numbers: a port in an install command, `RFC 6962`, a cookie
lifetime, the recorded NDVI value that the page cites with a fact id beside it.
Those are not stats about the running node and making them live would be
theatre. The rule is about figures the page presents as the node's own.

Exit codes
----------
  0  every live figure is fetched, and every placeholder has a filler
  1  a live figure is baked, orphaned, or targets nothing
  2  could not run

Usage
-----
  python3 scripts/live_numbers.py
"""

from __future__ import annotations

import re
import sys

# Every page, not just the homepage: /solutions declares live figures too,
# and a gate that watches one page while a second drifts is theatre.
import glob
PAGES = sorted(glob.glob("web/*.html"))


def main():
    fails = []
    pending, filled, ids_in_markup, pages_with = [], set(), set(), []
    for page in PAGES:
        try:
            s = open(page, encoding="utf-8").read()
        except OSError as e:
            print(f"live-numbers: cannot read {page}: {e}", file=sys.stderr)
            return 2
        p_pending = re.findall(r'<([a-z]+)([^>]*\bdata-pending\b[^>]*)>(.*?)</\1>', s, re.S)
        if p_pending:
            pages_with.append(page.split("/")[-1])
        pending += p_pending
        filled |= set(re.findall(r"put\(\s*'([\w-]+)'", s))
        ids_in_markup |= set(re.findall(r'id="([\w-]+)"', s))
    if not pending:
        print("live-numbers: no data-pending elements found, so this gate is "
              "watching nothing. If the live tiles were removed, remove this "
              "check with them.", file=sys.stderr)
        return 1

    pending_ids = []
    for tag, attrs, inner in pending:
        m = re.search(r'id="([\w-]+)"', attrs)
        pid = m.group(1) if m else "(no id)"
        pending_ids.append(pid)
        text = re.sub(r"<[^>]+>", "", inner)
        if re.search(r"\d", text):
            fails.append(
                f'{pid} is marked live and ships "{text.strip()[:24]}" in the '
                f"HTML. A static number cannot notice that the world moved.")

    for pid in pending_ids:
        if pid != "(no id)" and pid not in filled:
            fails.append(f"{pid} holds a placeholder that no put() call ever "
                         f"fills, so it reads as pending forever.")

    for fid in sorted(filled):
        if fid not in ids_in_markup:
            fails.append(f"put('{fid}') targets an id that is not in the "
                         f"markup; the figure it fed is now dead.")

    print(f"live numbers: {len(pending_ids)} figures declared live across "
          f"{len(pages_with)} pages ({', '.join(pages_with)}), "
          f"{len(filled)} filled from the responder")

    if fails:
        print("\nA page that says its numbers are live has to mean it.")
        for f in fails:
            print(f"  {f}")
        return 1
    print("Every live figure is fetched, and every placeholder has a filler.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
