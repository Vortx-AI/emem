#!/usr/bin/env python3
"""Run the benchmark we publish, and fail if it does not reproduce.

Why this exists
---------------
/v1/benchmark is the one artefact that invites a stranger to check us. Every
item carried a note saying "Verified against /agents.md", which is reading,
not running. One row paired the cell defi.zb4d9.pefa.zf619 with a fact_cid
belonging to defi.zb4d9.cojE.zf4be, 4 km away and 109 m higher: the row
claimed 6.0 m and the cell returned 115.0 m. An outside agent ran it and
found that before we did, and was entitled to conclude the addressing had
drifted. It had not. Both cells resolve today exactly as always. The row was
assembled from three different lines of prose.

A benchmark nobody executes is a claim, and this repository has spent a lot of
effort on the difference. So this executes it: every recall item is called
against the live responder and its fact_cid compared.

Similarity items are reported but not enforced. A nearest neighbour is a
property of the whole corpus, and the corpus grows: pinning "the top neighbour
must be this cell" would fail for a reason that is not a defect. What IS
enforced is that the call answers and returns a scored neighbour, because a
similarity row that 500s is broken in a way growth does not excuse.

    python3 scripts/benchmark_reproduces.py

Exit: 0 reproduces, 1 a published item does not, 2 the responder is unreachable.
"""

from __future__ import annotations

import json
import os
import sys
import urllib.error
import urllib.request

BASE = os.environ.get("EMEM_BASE", "https://emem.dev")


def call(path: str, body: dict | None = None, timeout: int = 120):
    url = BASE + path
    data = json.dumps(body).encode() if body is not None else None
    hdrs = {"content-type": "application/json"} if data else {}
    with urllib.request.urlopen(
        urllib.request.Request(url, data=data, headers=hdrs), timeout=timeout
    ) as r:
        return json.loads(r.read())


def main() -> int:
    try:
        items = (call("/v1/benchmark") or {}).get("items") or []
    except (urllib.error.URLError, TimeoutError, OSError) as e:
        print(f"benchmark: responder unreachable ({e}); nothing asserted.")
        return 2
    if not items:
        print("benchmark: the responder published no items at all.")
        return 1

    problems, checked = [], 0
    print(f"running the {len(items)} published items against {BASE}\n")
    for it in items:
        iid = it.get("id", "?")
        exp = it.get("expected") or {}
        kind = exp.get("kind")
        try:
            if kind == "fact_cid":
                d = call("/v1/recall",
                         {"cell": it["cell"], "bands": [it["band_or_encoder"]]})
                facts = d.get("facts") or []
                got = facts[0].get("fact_cid") if facts else None
                want = exp.get("expected")
                checked += 1
                if got == want:
                    print(f"  ok   {iid}  {facts[0].get('value')}")
                else:
                    print(f"  FAIL {iid}")
                    problems.append(
                        f"{iid}: published fact_cid {want} but calling "
                        f"{it['endpoint']} at {it['cell']} returns {got}. Either the "
                        f"row names the wrong cell or the expectation is stale; "
                        f"both make the benchmark a claim rather than a check."
                    )
            else:
                d = call("/v1/find_similar",
                         {"cell": it["cell"], "encoder": it["band_or_encoder"],
                          "limit": 3})
                hits = d.get("neighbors") or d.get("hits") or []
                if not hits:
                    problems.append(
                        f"{iid}: find_similar returned no neighbour at all. Which "
                        f"cell ranks first may drift with the corpus; answering "
                        f"with nothing is not drift."
                    )
                    print(f"  FAIL {iid}  no neighbours")
                else:
                    top = hits[0]
                    score = top.get("cosine") or top.get("similarity")
                    print(f"  ok   {iid}  top={top.get('cell')} cos={score} "
                          f"(rank not enforced: the corpus grows)")
        except (urllib.error.URLError, TimeoutError, OSError) as e:
            print(f"  ??   {iid}: {str(e)[:60]}")
            return 2

    print()
    if problems:
        print(f"benchmark: {len(problems)} published item(s) do not reproduce.")
        for p in problems:
            print(f"  x {p}")
        return 1
    print(f"benchmark: every published item reproduces ({checked} exact-match "
          f"rows executed, not read).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
