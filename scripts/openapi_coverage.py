#!/usr/bin/env python3
"""Every route the node serves is either in openapi.json or excluded on purpose.

Why this exists
---------------
The router served 159 `/v1` paths and the OpenAPI document described 138. Five
of the missing ones were named in `web/llms.txt` and `README.md`, so an agent
that discovered the API the way those files tell it to could not find the
routes those same files told it to call.

Nothing was lying. Each route was added correctly and the document was simply
never updated alongside it, which is how every drift in this repo has happened:
not by writing something false, but by writing something true in one place and
leaving the other place alone.

The fix that lasts is not documenting those five. It is making the gap
impossible to reach by accident. A route is now in one of exactly two states:

  * described in the OpenAPI document, or
  * listed in UNDOCUMENTED below, with a written reason.

There is no third state, and adding a route without choosing one fails here.

Hermetic by default
-------------------
Both sides are read out of `crates/emem-api-rest/src/lib.rs`, so this runs in
CI with no network and no built binary. Pass `--origin` to additionally check
that the responder actually serves what the source claims, which catches a
build that never got deployed.

Exit codes
----------
  0  every routed path is documented or deliberately excluded
  1  a path is neither, or an exclusion has gone stale
  2  could not run (source unreadable, origin unreachable)

Usage
-----
  python3 scripts/openapi_coverage.py
  python3 scripts/openapi_coverage.py --origin https://emem.dev
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import urllib.request

SRC = "crates/emem-api-rest/src/lib.rs"

# Routes that are deliberately absent from the API description.
#
# Every entry needs a reason a stranger would accept. "Internal" is not one:
# the demo and world INDEXES are documented, and what is excluded here is the
# file delivery beneath them, whose paths a caller gets from the index rather
# than from a spec. If an entry here ever stops being true, delete it and
# document the route.
UNDOCUMENTED = {
    "/v1/demos/{run}":
        "asset delivery under the documented /v1/demos index; a caller gets "
        "these paths from the index, not from a spec",
    "/v1/demos/{run}/{file}":
        "asset delivery under the documented /v1/demos index",
    "/v1/worlds/{preset}/{file}":
        "asset delivery under the documented /v1/worlds index",
    "/v1/places/scene_overlay.svg":
        "returns an image, not a described JSON resource",
    "/v1/openapi.action.json":
        "a variant rendering of this document; listing it inside itself "
        "tells a reader nothing they did not already have",
}


def routed_paths(src):
    """Paths the axum router actually serves, with `:name` and `*name` normalised.

    Both of axum's parameter spellings become OpenAPI's `{name}`. The wildcard
    was missing, so a route declared `/v1/perception/*path` could only be
    satisfied by a description literally spelled `*path` -- which is not
    OpenAPI templating, so the only way to pass was to write something no
    client generator could read. The gate was asking for a spelling that would
    have made the description wrong.
    """
    out = {}
    pat = (r'\.route\(\s*"([^"]+)"\s*,\s*'
           r'((?:get|post|put|delete|patch)\([^)]*\)'
           r'(?:\s*\.\s*(?:get|post|put|delete|patch)\([^)]*\))*)')
    for m in re.finditer(pat, src):
        path, verbs = m.group(1), m.group(2)
        if not path.startswith("/v1/"):
            continue
        tmpl = re.sub(r"[:*]([A-Za-z_][A-Za-z0-9_]*)", r"{\1}", path)
        found = re.findall(r"\b(get|post|put|delete|patch)\(", verbs)
        out.setdefault(tmpl, set()).update(v.upper() for v in found)
    return out


def documented_paths(src):
    """Paths carried by the OpenAPI literal in the same file.

    A documented entry reads `"/v1/x": {"get": ...}`; a route reads
    `"/v1/x", get(...)`. The `: {` is what tells them apart, so this cannot
    mistake a router line for a description of one.
    """
    return set(re.findall(r'"(/v1/[^"]*)"\s*:\s*\{', src))


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--origin", default=None,
                    help="also check the responder serves what the source says")
    a = ap.parse_args()

    try:
        src = open(SRC, encoding="utf-8").read()
    except OSError as e:
        print(f"openapi-coverage: cannot read {SRC}: {e}", file=sys.stderr)
        return 2

    routed = routed_paths(src)
    documented = documented_paths(src)
    if not routed or not documented:
        print("openapi-coverage: parsed 0 routes or 0 descriptions, which means "
              "the source moved and this gate is measuring nothing.",
              file=sys.stderr)
        return 2

    fails = []
    undocumented = sorted(set(routed) - documented)
    unexplained = [p for p in undocumented if p not in UNDOCUMENTED]

    print(f"openapi coverage: {len(routed)} routed /v1 paths, "
          f"{len(documented & set(routed))} described, "
          f"{len(UNDOCUMENTED)} excluded on purpose\n")

    for p in unexplained:
        fails.append(
            f"{p} is served and is not in the API description. Describe it, or "
            f"add it to UNDOCUMENTED in {__file__.split('/')[-1]} with a reason.")

    # An exclusion for a route that no longer exists is a comment pretending to
    # be a decision. It also quietly shrinks what this gate covers.
    for p in sorted(UNDOCUMENTED):
        if p not in routed:
            fails.append(f"{p} is excluded from the API description but is no "
                         f"longer routed; drop the exclusion.")
        elif p in documented:
            fails.append(f"{p} is excluded AND described; the exclusion is "
                         f"stale, drop it.")

    for p in sorted(UNDOCUMENTED):
        if len(UNDOCUMENTED[p]) < 20:
            fails.append(f"{p} is excluded without a usable reason.")

    if a.origin:
        origin = a.origin.rstrip("/")
        try:
            with urllib.request.urlopen(f"{origin}/openapi.json", timeout=90) as r:
                live = set(json.load(r).get("paths") or {})
        except Exception as e:
            print(f"openapi-coverage: {origin} did not answer: {e}",
                  file=sys.stderr)
            return 2
        drift = sorted((documented & set(routed)) - live)
        print(f"  {'ok  ' if not drift else 'FAIL'} the responder serves "
              f"{len(live)} paths, {len(drift)} described here but not there")
        for p in drift:
            fails.append(f"{p} is described in the source and absent from "
                         f"{origin}/openapi.json: the build was not deployed.")

    if fails:
        print("\nA route that is neither described nor deliberately excluded is "
              "a capability nobody can discover.")
        for f in fails:
            print(f"  {f}")
        return 1
    print("Every routed path is described or excluded with a reason.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
