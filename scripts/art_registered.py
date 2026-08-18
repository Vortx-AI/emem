#!/usr/bin/env python3
"""Every drawing a page links to must be one the responder will serve.

Why this exists
---------------
web/art holds the drawings and the responder serves them from a hand-written
list of include_str! entries in emem-api-rest. A file on disk that is not in
that list 404s however correctly a page links to it, and nothing noticed:
doc_lint reads prose, route_truth checks routes the site declares, and
page_health only sees a broken image on a page it happens to load.

I added hero-two-agents.svg, pointed the homepage at it, deployed, and put a
broken image on the front page. The art existed, the markup was right, the
build was clean.

This compares the three sets: what is on disk, what is registered, and what
the pages ask for. It fails when a page links to art nobody serves.

    python3 scripts/art_registered.py

Exit: 0 consistent, 1 a page links to art that will 404.
"""

from __future__ import annotations

import glob
import os
import re
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LIB = os.path.join(REPO, "crates/emem-api-rest/src/lib.rs")


def main() -> int:
    on_disk = {os.path.basename(p) for p in glob.glob(os.path.join(REPO, "web/art/*.svg"))}
    src = open(LIB, encoding="utf-8").read()
    registered = set(re.findall(r'include_str!\("\.\./\.\./\.\./web/art/([^"]+)"\)', src))

    linked: dict[str, set[str]] = {}
    for page in sorted(glob.glob(os.path.join(REPO, "web/*.html"))):
        text = open(page, encoding="utf-8").read()
        for name in set(re.findall(r'/art/([A-Za-z0-9._-]+\.svg)', text)):
            linked.setdefault(name, set()).add(os.path.basename(page))

    problems = []
    for name, pages in sorted(linked.items()):
        if name not in on_disk:
            problems.append(f"{name} is linked by {', '.join(sorted(pages))} and is not in web/art")
        elif name not in registered:
            problems.append(
                f"{name} is linked by {', '.join(sorted(pages))} and is not registered in "
                f"emem-api-rest, so the responder answers 404 for it"
            )

    print(f"  {len(on_disk)} on disk, {len(registered)} registered, {len(linked)} linked by a page")
    unused = sorted(on_disk - set(linked))
    if unused:
        # Not a failure. Art can be drawn before it is placed, and saying so is
        # more useful than pretending the set has to match exactly.
        print(f"  {len(unused)} drawn and not yet placed: {', '.join(unused[:6])}"
              + (" …" if len(unused) > 6 else ""))

    if problems:
        print("\nart_registered: a page links to art nobody serves.")
        for p in problems:
            print(f"  x {p}")
        return 1
    print("Every drawing a page links to is on disk and registered.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
