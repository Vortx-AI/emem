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


def fetch_status(url, timeout=12):
    """Status line only, body left unread. For endpoints that never end."""
    try:
        with urllib.request.urlopen(url, timeout=timeout) as r:
            return r.status
    except urllib.error.HTTPError as e:
        return e.code
    except Exception:
        return 0


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
    streaming = set()
    try:
        doc = json.loads(fetch(f"{origin}/openapi.json")[1])
        for path, ops in (doc.get("paths") or {}).items():
            for verb, op in ops.items():
                if verb.upper() in ("GET", "POST"):
                    methods.setdefault(path, verb.upper())
                # A long-lived stream never finishes a body, so reading one
                # times out and reports code 0, which is indistinguishable
                # from an outage. /v1/memory/sse spent a run in the dead list
                # for that reason while serving perfectly.
                if "event-stream" in json.dumps(op.get("responses") or {}):
                    streaming.add(path)
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

    dead, unreachable = [], []
    for u in sorted(advertised):
        path = u[len("https://emem.dev"):].split("?")[0] or "/"
        verb = methods.get(path, "GET")
        # A POST route is probed with an empty body: the question is whether
        # the route EXISTS, not whether these arguments are valid, so anything
        # that is not 404 or 405 counts as present.
        # Short timeout: this is a reachability sweep over ~100 URLs, not a
        # latency test, and a slow route is checked elsewhere.
        if path in streaming:
            # Headers only: the question is whether the stream opens, and
            # urlopen returns as soon as it has a status line. Reading the
            # body is what hangs, so do not.
            code = fetch_status(u)
        else:
            code, _ = fetch(u, verb, {} if verb == "POST" else None, timeout=12)
        # 0 is `fetch`'s sentinel for "the connection never completed", not an
        # HTTP status. It used to be lumped in with 404/405 as dead, which
        # meant a runner that could not reach the origin reported EVERY
        # advertised route as an overclaim. That is what CI printed on
        # 2026-08-11: twenty-two lines of "advertised but 0" including
        # /v1/recall and /verify, the two routes the whole product stands on.
        # If those were really 404 nothing would work, so the output refuted
        # itself and still exited 1 against the product.
        #
        # A route that answers 404 or 405 is a real overclaim: we connected,
        # and the thing we advertise is not there. A route we could not reach
        # tells us nothing about the route. Only the first is a finding.
        if code in (404, 405):
            dead.append((u, code, sorted(advertised[u])))
        elif code == 0:
            unreachable.append(u)
    if unreachable:
        print(f"\n  ---- {len(unreachable)} of {len(advertised)} advertised URLs "
              f"could not be reached at all; route truth NOT asserted this run")
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

    # A wired endpoint answering the wrong method must say so. axum's default
    # is a 405 with zero bytes, which is a dead end: the reader learns neither
    # that the path exists nor how to call it. It became visible when the
    # ChatGPT listing sent people to GET /v1/verify_receipt and 74 of the 78
    # POST-only paths answered with nothing at all. Every other error this API
    # serves is typed and names the accepted alternative.
    code, text = fetch(f"{origin}/openapi.json")
    try:
        paths = json.loads(text).get("paths", {}) if code == 200 else {}
    except Exception:
        paths = {}
    if not paths:
        print(f"\n  FAIL could not read {origin}/openapi.json ({code}), so no "
              f"POST-only path was checked")
        fails.append("openapi.json unreadable: the empty-405 check asserted nothing")
    post_only = [p for p, v in paths.items() if "post" in v and "get" not in v]
    bare = []
    for p in post_only:
        url = origin + p.replace("{id}", "x").replace("{cid}", "x")
        try:
            req = urllib.request.Request(url, method="GET")
            try:
                r = urllib.request.urlopen(req, timeout=15)
                code, body = r.status, r.read()
            except urllib.error.HTTPError as e:
                code, body = e.code, e.read()
        except Exception:
            continue
        if code == 405 and len(body) < 5:
            bare.append(p)
    print(f"\n  {'ok  ' if not bare else 'FAIL'} {len(post_only)} POST-only paths, "
          f"{len(bare)} answering GET with an empty 405")
    for p in bare[:5]:
        fails.append(f"{p} returns a 405 with no body, so a reader who follows a "
                     f"link to it learns nothing")

    # Reported before any finding: a run that could not reach the origin has
    # not checked the origin, and must read as neither pass nor fail of it.
    # Exit 2 is the code ci.yml already maps to a warning for exactly this.
    if unreachable:
        print(f"\nroute truth: could not reach {len(unreachable)} advertised URLs "
              f"at {origin}, so nothing was asserted about them this run.",
              file=sys.stderr)
        for u in unreachable[:5]:
            print(f"  {u}", file=sys.stderr)
        return 2

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
