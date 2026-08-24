#!/usr/bin/env python3
"""Which routes are built and never called, and which are aliases where zero is right?

    python3 scripts/manual/unvisited_routes.py [--hours 24]

Why this is worth counting
--------------------------
A capability nobody calls is invisible to a test suite and to a changelog. It
passes every gate, appears in the openapi document, and costs maintenance
forever. It is also the cheapest thing to either finish or delete, but only once
you can see it -- and nothing here could, because every check asks "does it
answer", never "does anybody ask".

357 routes are declared and 137 saw no request in 24 hours. That raw number is
not a finding: most of them are aliases (`/agents.md` beside `/agents`),
well-known documents a crawler fetches monthly, or error-path routes that exist
to return a typed refusal. Reporting 137 as dead code would be the same
over-count as measuring "smaller than 24px" and calling it a WCAG failure.

So this classifies rather than counts:

  alias        another route serves the same handler or content, and that one
               IS called. Zero is correct and expected.
  well-known   a discovery document, sitemap, icon or policy file. Fetched
               rarely by design; zero over a day means nothing.
  typed-error  exists to answer a wrong call with a useful refusal (e.g.
               /v1/chat/completions saying emem is not an LLM provider). Zero
               is the GOAL.
  unvisited    none of the above. Built, routed, documented, and nobody called
               it. This is the column worth reading.
"""
import argparse
import re
import subprocess
from collections import defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent

WELL_KNOWN = re.compile(
    r"^/(\.well-known/|robots\.txt|sitemap|favicon|apple-touch-icon|.*\.(?:png|gif|ico|svg|txt|xml)$"
    r"|llms|humans)", re.I)
DOC_ALIAS = re.compile(r"^/(docs?/|examples?/|.*\.md$)", re.I)


def routes():
    src = (REPO / "crates" / "emem-api-rest" / "src" / "lib.rs").read_text(encoding="utf-8")
    return sorted(set(re.findall(r'\.route\(\s*"(/[^"]*)"', src)))


def handlers():
    """route -> handler name, so two routes on one handler are visibly aliases."""
    src = (REPO / "crates" / "emem-api-rest" / "src" / "lib.rs").read_text(encoding="utf-8")
    out = {}
    for m in re.finditer(r'\.route\(\s*"(/[^"]*)"\s*,\s*(?:get|post|put|delete|any)\(([A-Za-z0-9_]+)', src):
        out[m.group(1)] = m.group(2)
    return out


def seen(hours: int):
    out = subprocess.run(
        ["journalctl", "--user", "-u", "emem-server.service",
         "--since", f"{hours} hours ago", "--no-pager"],
        capture_output=True, text=True, timeout=600).stdout
    return set(re.findall(r"http_path=(/[^ ]*)", out))


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--hours", type=int, default=24)
    a = ap.parse_args()

    declared = routes()
    hmap = handlers()
    hits = seen(a.hours)
    if not hits:
        print("  the journal returned no request lines. Undetermined, not "
              "'nothing was called': this needs traffic to classify.")
        return 1

    # a route with a path parameter matches many concrete paths
    def was_called(p: str) -> bool:
        if p in hits:
            return True
        # DIGITS ARE PART OF A PARAMETER NAME. `:[A-Za-z_]+` matched ":cell"
        # and left the "64", so `/v1/cells/:cell64/geojson` became the pattern
        # `/v1/cells/[^/]+64/geojson`, matched nothing, and was reported as
        # never called -- while the traffic log showed 1,459 requests to it.
        # The instrument, not the route.
        # A CATCH-ALL SPANS SEGMENTS. `*path` matches `spark/index.html`, two
        # segments, so mapping it to `[^/]+` matched nothing and reported
        # /splats/*path as never called while the log held 206 requests. A
        # single-segment parameter and a catch-all are different things and
        # collapsing them under-counts exactly the busiest static routes.
        pat = re.sub(r"\*[A-Za-z0-9_]+", "SPLAT", p)
        pat = re.sub(r"\{[^}]*\}|:[A-Za-z0-9_]+", "[^/]+", pat)
        pat = pat.replace("SPLAT", ".+")
        rx = re.compile("^" + pat + "$")
        return any(rx.match(h) for h in hits)

    called_handlers = {hmap.get(p) for p in declared if was_called(p)}
    buckets = defaultdict(list)
    for p in declared:
        if was_called(p):
            buckets["called"].append(p)
        elif WELL_KNOWN.match(p):
            buckets["well-known"].append(p)
        elif hmap.get(p) and hmap[p] in called_handlers:
            buckets["alias"].append(p)
        elif DOC_ALIAS.match(p):
            buckets["doc alias"].append(p)
        else:
            buckets["UNVISITED"].append(p)

    print(f"routes: {len(declared)} declared, {a.hours}h of traffic")
    for k in ("called", "alias", "doc alias", "well-known", "UNVISITED"):
        print(f"  {k:12} {len(buckets[k]):>4}")
    print("\nBuilt, routed, and nobody called it:")
    for p in sorted(buckets["UNVISITED"]):
        print(f"  {p:46} {hmap.get(p, '(inline)')}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
