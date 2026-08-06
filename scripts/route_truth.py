#!/usr/bin/env python3
"""Every route the site presents as hosted must actually answer on the origin.

Why this exists
---------------
The /guard page shipped a nine-route table where seven of the routes returned
404 on emem.dev. Nothing was technically false: the table describes what a
self-hosted node serves, and the curl blocks all said `localhost:8080`. But a
reader stands on emem.dev, and a reader who tries the first thing they see and
gets a 404 has learned that the page overclaims. `u4aaoieq` caught it by doing
exactly that.

This is the no-hardcoded-numbers rule generalised to routes: no number on the
site that is not fetched from the node, and now no route on the site that is
not reachable where it is shown.

What it checks
--------------
1. Every route the descriptor marks `hosted: true` answers on the origin.
2. Every route it marks `hosted: false` names a reason, and answers a TYPED
   refusal rather than a bare 404, so an agent can tell "not here" from
   "nowhere".
3. Every fenced command block on /guard carries a source marker, so a reader
   always knows whether they are looking at this origin or at a node they have
   not built yet.

Exit codes
----------
  0  the page and the origin agree
  1  a route is presented as hosted and is not, or a block is unmarked
  2  could not run (origin unreachable)

Usage
-----
  python3 scripts/route_truth.py
  python3 scripts/route_truth.py --origin http://127.0.0.1:5051
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import urllib.error
import urllib.request

DEFAULT_ORIGIN = "https://emem.dev"
PAGE = "web/guard.html"


def fetch(url, method="GET", body=None, timeout=45):
    req = urllib.request.Request(
        url,
        data=json.dumps(body).encode() if body is not None else None,
        headers={"content-type": "application/json"},
        method=method,
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, r.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8", "replace")
    except Exception as e:
        return 0, str(e)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--origin", default=DEFAULT_ORIGIN)
    a = ap.parse_args()
    origin = a.origin.rstrip("/")
    fails = []

    status, raw = fetch(f"{origin}/.well-known/emem-guard.json")
    if status != 200:
        print(
            f"route-truth: {origin}/.well-known/emem-guard.json -> {status}. "
            f"A cold agent is told to read this path; it has to answer.",
            file=sys.stderr,
        )
        return 2
    try:
        desc = json.loads(raw)
    except Exception as e:
        print(f"route-truth: descriptor is not json: {e}", file=sys.stderr)
        return 2

    checkpoints = desc.get("checkpoints") or []
    if not checkpoints:
        print("route-truth: descriptor advertises no checkpoints", file=sys.stderr)
        return 2

    print(f"route truth against {origin}\n")

    for cp in checkpoints:
        route = cp.get("route", "")
        hosted = cp.get("hosted")
        if hosted is True:
            code, _ = fetch(f"{origin}{route}", "POST", {})
            ok = 200 <= code < 300
            print(f"  {'ok  ' if ok else 'FAIL'} hosted     {route:<28} {code}")
            if not ok:
                fails.append(f"{route} is advertised as hosted and answered {code}")
        elif hosted is False:
            code, body = fetch(f"{origin}{route}", "POST", {})
            # A typed refusal, not a bare 404. The distinction is the whole
            # point: an agent that cannot tell "not here" from "nowhere" gives
            # up on both.
            typed = '"code"' in body and "selfhost" in body
            why = isinstance(cp.get("why"), str) and len(cp["why"]) > 10
            ok = typed and why
            print(f"  {'ok  ' if ok else 'FAIL'} not-hosted {route:<28} {code} "
                  f"{'typed refusal, reason given' if ok else 'bare or unexplained'}")
            if not ok:
                fails.append(
                    f"{route} is marked not-hosted but "
                    + ("gives no reason" if not why else "answers a bare refusal")
                )

    # Anything the descriptor says is absent must actually be absent AND
    # explained, or the document is describing a different deployment.
    for route, reason in (desc.get("not_served_here") or {}).items():
        if "{" in route:
            continue
        code, body = fetch(f"{origin}{route}")
        typed = '"code"' in body
        ok = code >= 400 and typed and len(reason) > 10
        print(f"  {'ok  ' if ok else 'FAIL'} absent     {route:<28} {code} "
              f"{'explained' if ok else 'unexplained or unexpectedly present'}")
        if not ok:
            fails.append(f"{route} is listed as not served here but answered {code}")

    # Every diagram the pages LINK must answer. A diagram file added to
    # docs/diagrams and referenced from a page still 404s until it is
    # registered in the DOCS_DIAGRAMS table in emem-api-rest, which is exactly
    # the shape of the failure this whole gate exists for: something shown
    # where it does not resolve. Four guard diagrams shipped that way.
    linked = set()
    for page in ("web/guard.html", "docs/diagrams/index.html"):
        try:
            body = open(page, encoding="utf-8").read()
        except OSError:
            continue
        linked.update(re.findall(r"/docs/diagrams/([A-Za-z0-9._-]+\.svg)", body))
    missing = []
    for svg in sorted(linked):
        code, _ = fetch(f"{origin}/docs/diagrams/{svg}")
        if code != 200:
            missing.append(f"{svg} -> {code}")
    print(f"\n  {'ok  ' if not missing else 'FAIL'} {len(linked)} diagrams linked from the "
          f"pages, {len(missing)} do not answer")
    for m in missing:
        fails.append(f"linked but not served: {m}")

    # Every emem.dev URL a machine surface advertises must resolve.
    #
    # A directory or a cold agent reads these files and follows what it finds.
    # docs/integrations.md carried a worked `curl` against a bundle token that
    # had stopped resolving, so the one example an integrator was most likely
    # to paste returned 404.
    #
    # The METHOD comes from the responder's own OpenAPI document rather than
    # from a guess: half these routes are POST, and probing them with GET
    # returns 405, which is not drift. That is the same mistake the parity
    # harness made before it learned to read operationIds.
    methods = {}
    try:
        doc = json.loads(fetch(f"{origin}/openapi.json")[1])
        for path, ops in (doc.get("paths") or {}).items():
            for verb in ops:
                if verb.upper() in ("GET", "POST"):
                    methods.setdefault(path, verb.upper())
    except Exception:
        methods = {}

    surfaces = ["README.md", "AGENTS.md", "llms-install.md", "server.json",
                "web/llms.txt", "web/skills.md", "web/ai-plugin.json",
                "docs/agents.md", "docs/integrations.md",
                "crates/emem-guard/SKILL.md"]
    advertised = {}
    for rel in surfaces:
        try:
            body = open(rel, encoding="utf-8").read()
        except OSError:
            continue
        for m in re.finditer(r"https://emem\.dev[\w./?=&#:%-]*", body):
            u = m.group(0).rstrip(".,)`\"'>\\")
            # A template placeholder (`/v1/facts/{cid}`, `/skills/<name>/...`)
            # gets cut by the character class above, leaving a bare directory
            # that was never advertised. Checking that reports the regex, not
            # the docs, and three such artefacts were the first thing this
            # sweep "found".
            nxt = body[m.end():m.end() + 1]
            if nxt in ("{", "<") or u.endswith("/"):
                continue
            advertised.setdefault(u, set()).add(rel)

    dead = []
    for u in sorted(advertised):
        path = u[len("https://emem.dev"):].split("?")[0] or "/"
        verb = methods.get(path, "GET")
        # A POST route is probed with an empty body: the question is whether
        # the route EXISTS, not whether these arguments are valid, so anything
        # that is not 404 or 405 counts as present.
        # Short timeout: this is a reachability sweep over ~100 URLs, not a
        # latency test, and a slow route is checked elsewhere.
        code, _ = fetch(u, verb, {} if verb == "POST" else None, timeout=12)
        if code == 404 or code == 405 or code == 0:
            dead.append((u, code, sorted(advertised[u])))
    print(f"\n  {'ok  ' if not dead else 'FAIL'} {len(advertised)} emem.dev URLs "
          f"advertised in {len(surfaces)} machine surfaces, {len(dead)} dead")
    for u, c, where in dead:
        fails.append(f"advertised but {c}: {u}  (in {', '.join(where)})")

    # And the page itself: no command block without a source marker.
    try:
        page = open(PAGE, encoding="utf-8").read()
    except OSError as e:
        print(f"\nroute-truth: cannot read {PAGE}: {e}", file=sys.stderr)
        return 2
    heads = re.findall(r'<div class="code-head">(.*?)</div>', page, re.S)
    unmarked = [
        h for h in heads
        if not any(m in h for m in ("your own node", "emem.dev", "this origin"))
    ]
    print(f"\n  {'ok  ' if not unmarked else 'FAIL'} {len(heads)} command blocks on "
          f"{PAGE}, {len(unmarked)} without a source marker")
    for h in unmarked[:5]:
        fails.append("unmarked block: " + re.sub(r"<[^>]+>", " ", h).strip()[:60])

    if fails:
        print("\nA route shown where it does not run teaches a reader that the page "
              "overclaims, and they are right.")
        for f in fails:
            print(f"  {f}")
        return 1
    print("\nEvery route the site presents as hosted answers here, and every "
          "command block says where it runs.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
