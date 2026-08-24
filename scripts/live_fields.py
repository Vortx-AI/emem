#!/usr/bin/env python3
"""The fields the pages read off live JSON must still exist on the responder.

Why this exists
---------------
live_numbers.py proves a figure is fetched rather than baked. It cannot prove
the fetch finds anything. The homepage reads `mcp_tools.total` off /agent.json,
`sth.tree_size` off /v1/log/sth, `uptime_seconds` off /live. Rename any of them
and put() is handed undefined, put() declines to write, and the tile keeps its
placeholder: the page goes quiet in exactly the way it was built to when the
NODE is down, while the node is fine and the contract is what moved.

That failure is worse than a stale number. A stale number is wrong and legible.
A blank tile under a caption promising live figures reads as "nothing here",
and nothing in the repo would have noticed: doc_lint reads prose, sync_counts
reads counts, route_truth reads status codes, and a 200 carrying a renamed
field passes all three.

So this walks every `j('<url>').then(function(d){ ... d.field ... })` in the
pages, fetches the url, and asserts the field is reachable.

    python3 scripts/live_fields.py

Exit: 0 every read resolves, 1 a page reads a field the responder lost,
2 the responder is unreachable and nothing was asserted.
"""

from __future__ import annotations

import glob
import json
import os
import re
import sys
import urllib.error
import urllib.request

from lib_patience import patient

BASE = os.environ.get("EMEM_BASE", "https://emem.dev")
FETCH = re.compile(r"j\('([^']+)'\)\.then\(function\((\w+)\)\{")
# Names that are JS, not response fields.
NOT_FIELDS = {"length", "map", "filter", "forEach", "innerHTML", "textContent",
              "push", "join", "slice", "toFixed", "toString"}


def reachable(doc, path: str) -> bool:
    cur = doc
    for part in path.split("."):
        if isinstance(cur, list):
            if not cur:
                return True          # empty list: the field exists, there is no row
            cur = cur[0]
        if isinstance(cur, dict) and part in cur:
            cur = cur[part]
        else:
            return False
    return True


def main() -> int:
    pages = sorted(glob.glob("web/*.html"))
    reads: dict[str, set[str]] = {}
    for page in pages:
        text = open(page, encoding="utf-8").read()
        for m in FETCH.finditer(text):
            url, var = m.group(1), m.group(2)
            # Stop at the end of THIS callback. A fixed window bled into the
            # next j(...) block and blamed /agent.json for fields that belong
            # to /v1/agents, which is a gate inventing a defect.
            nxt = text.find("j('", m.end())
            stop = text.find("}).catch", m.end())
            end = min(x for x in (nxt, stop, len(text)) if x > 0)
            body = text[m.end():end]

            # `(a.agents && a.agents.length) || a.count` is one read with a
            # fallback, not two required fields. Requiring every branch would
            # fail on the defence the page is written with. Alternatives are
            # grouped, and the group passes if any branch resolves.
            for expr in re.split(r";|\n", body):
                names = [f.rstrip(".") for f in
                         re.findall(rf"\b{var}\.([A-Za-z_][A-Za-z0-9_.]*)", expr)]
                names = [".".join(p for p in n.split(".") if p not in NOT_FIELDS)
                         for n in names]
                names = [n for n in names if n]
                if not names:
                    continue
                if "||" in expr and len(names) > 1:
                    reads.setdefault(url, set()).add(tuple(sorted(set(names))))
                else:
                    for n in names:
                        reads.setdefault(url, set()).add((n,))

    if not reads:
        print("live_fields: no live reads found in web/*.html")
        return 0

    problems = []
    for url in sorted(reads):
        try:
            with patient(BASE + url, timeout=60) as r:
                doc = json.loads(r.read())
        except (urllib.error.URLError, TimeoutError, OSError, ValueError) as e:
            print(f"live_fields: {url} did not answer ({str(e)[:50]}); nothing asserted.")
            return 2
        gone = [alts for alts in reads[url]
                if not any(reachable(doc, f) for f in alts)]
        ok = len(reads[url]) - len(gone)
        print(f"  {url:<20} {ok} of {len(reads[url])} reads resolve"
              + ("   MISSING: " + "; ".join(" or ".join(a) for a in gone) if gone else ""))
        for alts in gone:
            what = " or ".join(f"`{a}`" for a in alts)
            problems.append(
                f"a page reads {what} off {url} and the responder carries none of them. "
                f"put() is handed undefined, declines to write, and the tile keeps its "
                f"placeholder: the page reads as though the node were down."
            )

    if problems:
        print("\nlive_fields: a page is reading a field that moved.")
        for p in problems:
            print(f"  x {p}")
        return 1
    print("\nEvery field the pages read off the responder is still there.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
