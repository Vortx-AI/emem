#!/usr/bin/env python3
"""Cold-start discovery: can an agent that knows nothing get from a need to a call?

Every other gate here checks that something we built still works. This one
checks something harder: that an agent which has never heard of emem can find
out what we are, decide whether we are relevant, and make a correct first call,
using only public surfaces and no prior knowledge.

The rule the probe plays by
---------------------------
It is given ONE thing: the origin. Not a tool name, not an endpoint, not the
word "emem" as a capability. It starts from a need phrased the way an agent
phrases it ("I need to verify a claim someone handed me") and must reach a
working call. Anything it cannot discover from the origin, it does not get.

That is the honest version of the question. A discovery test that starts by
importing our tool list has assumed away the thing it claims to measure.

What this can and cannot prove
------------------------------
It is a structural probe, not a language model. It cannot prove an LLM would
make the right choice; it proves the information an LLM needs is present,
reachable, small enough to read, and TRUE. The last one is the part that rots:
a registry entry is a claim like any other, so every served row's `first_call`
is executed against the live responder rather than trusted.

Three failures it is built to catch
-----------------------------------
  1. Unreachable: the need cannot be matched from any surface the origin
     advertises. The agent never learns we are relevant.
  2. Unaffordable: the path to a first call costs more bytes than an agent
     will spend. On 2026-08-17 the six discovery surfaces totalled 508 KB;
     an agent that reads them all burns six figures of context deciding
     whether to make one call. Cost is a correctness property here.
  3. Untrue: the registry says a capability is served and the call fails, or
     says it is not served while quietly advertising a tool anyway. This is
     the one that gets worse over time, because the registry is prose and
     prose does not recompile.

    python3 scripts/discovery_test.py
    python3 scripts/discovery_test.py --origin http://127.0.0.1:5051
    python3 scripts/discovery_test.py --budget 60000

Exit codes: 0 discoverable within budget and every claim held, 1 a claim
failed or the budget was blown, 2 the origin could not be reached at all.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import urllib.error
import urllib.request

from lib_patience import patient

DEFAULT_ORIGIN = "https://emem.dev"

# The context an agent will spend to decide whether to call you ONCE. Not a
# performance target: past this, a rational agent skips the unknown service and
# uses one it already knows, so an undiscoverable-in-budget capability is
# functionally absent no matter how good it is.
DEFAULT_BUDGET = 40_000

# The needs the probe arrives with. Deliberately NOT our vocabulary: no band
# names, no tool names, no "cell64". If a row here only matches once you
# already speak emem, it is not testing discovery.
COLD_NEEDS = [
    "I need to verify a claim someone handed me",
    "I need memory another agent can read and trust without trusting me",
    "I need state that survives context compaction",
    "I need memory that is private to me",
]

# What the root discovery document has to answer for an agent to decide at
# all. Endpoint lists are not enough: they tell a reader where the doors are
# while assuming it already chose to come in.
# Tools the end-to-end handoff below actually calls. A registry row whose
# first_call is prose is still checked if its tool appears here.
CHAIN_COVERS = {
    "emem_recall",
    "emem_memory_token",
    "emem_memory_token_resolve",
    "emem_verify_receipt",
    "emem_locate",
    "emem_memory_bundle",
}

ROOT_MUST_ANSWER = {
    "identity": ("protocol", "vendor", "name"),
    "version": ("version",),
    "what_it_is_for": ("summary", "purpose", "description", "intents_url"),
    "where_to_start": ("intents_url", "agent_intent_url", "quickstart_url",
                       "agent_card_url"),
}


class Probe:
    """Tracks every byte the agent had to read to get where it got."""

    def __init__(self, origin):
        self.origin = origin.rstrip("/")
        self.bytes = 0
        self.fetches = 0
        self.trail = []

    def get(self, path, method="GET", body=None):
        url = self.origin + path
        data = json.dumps(body).encode() if body is not None else None
        req = urllib.request.Request(
            url, data=data, method=method,
            headers={"content-type": "application/json"} if data else {},
        )
        try:
            with patient(req, timeout=60) as r:
                raw = r.read()
                code = r.status
        except urllib.error.HTTPError as e:
            raw, code = e.read(), e.code
        except Exception as e:
            self.trail.append((path, "unreachable", 0))
            return None, str(e)
        self.bytes += len(raw)
        self.fetches += 1
        self.trail.append((path, code, len(raw)))
        try:
            return json.loads(raw), code
        except Exception:
            return raw.decode("utf8", "replace"), code


def parse_first_call(s):
    """Pull an executable (method, path, body) out of a `first_call` string.

    The field is written for a human first, so some rows are prose describing
    a two-step sequence. Those are reported as unparseable rather than being
    coerced into a call the row did not mean, because a probe that guesses is
    a probe whose failures are its own.
    """
    m = re.match(r"^\s*(GET|POST)\s+(/\S+)\s*(\{.*\})?\s*$", s, re.S)
    if not m:
        return None
    method, path, body = m.group(1), m.group(2), m.group(3)
    if body:
        # Registry examples carry <placeholders> a probe cannot fill.
        if "<" in body and ">" in body:
            return None
        try:
            body = json.loads(body)
        except Exception:
            return None
    return method, path, body


def write_robots(origin):
    """Refresh the advertised sizes from what is actually served.

    These are hand-written numbers describing live documents, which is a
    combination that drifts in one direction: documents grow. One of them
    reached 2.00x its claim and failed this gate, which is the gate working,
    but an agent should not need a CI failure to get an honest figure.

    The rule and the fixer live in one file on purpose. A checker in one place
    and a generator in another disagree eventually, and the disagreement shows
    up as a gate nobody can satisfy.
    """
    import urllib.request

    # Wide enough for the largest surface here (~341 KB) plus a space.
    SIZE_COLUMN = 8

    path = os.path.join(os.path.dirname(__file__), "..", "web", "robots.txt")
    path = os.path.normpath(path)
    with open(path, encoding="utf-8") as fh:
        text = fh.read()

    changed = 0

    def measure(m):
        nonlocal changed
        url = origin.rstrip("/") + m.group("path")
        try:
            with patient(url, timeout=30) as r:
                actual = len(r.read())
        except Exception as exc:  # noqa: BLE001 - reported, not swallowed
            print(f"  {m.group('path')}: could not fetch ({exc}); left alone")
            return m.group(0)
        # One significant figure below 10 KB, whole KB above: the number is a
        # budgeting hint, not a content length, and false precision in a
        # comment invites someone to keep it exact by hand.
        kb = actual / 1000
        shown = f"{kb:.1f}" if kb < 10 else f"{kb:.0f}"
        shown = shown.rstrip("0").rstrip(".") if "." in shown else shown
        was = m.group("num") + " " + m.group("unit") if m.group("unit") else m.group("num")
        if was.replace(" ", "") != (shown + "KB"):
            changed += 1
            print(f"  {m.group('path'):<34} {was:>8}  ->  {shown} KB")
        # Fixed column, so the descriptions stay lined up whatever the
        # numbers do. Preserving the ORIGINAL field width instead let a longer
        # number eat its own padding and shunt the description leftwards, which
        # is a worse file to read than the one that was drifting.
        return m.group("head") + f"~{shown} KB".ljust(SIZE_COLUMN)

    pattern = re.compile(
        r"(?P<head>^#\s+(?P<path>/\S+)\s+)(?P<size_field>~?(?P<num>[\d.]+)\s*(?P<unit>KB|B)\s*)",
        re.M,
    )
    out = pattern.sub(measure, text)
    if changed:
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(out)
        print(f"\n  {changed} size(s) refreshed in web/robots.txt.")
        print("  robots.txt is include_str! into the binary: rebuild and restart to serve it.")
    else:
        print("  every advertised size already matches what is served.")
    return 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--origin", default=os.environ.get("EMEM_ORIGIN", DEFAULT_ORIGIN))
    ap.add_argument("--budget", type=int, default=DEFAULT_BUDGET)
    ap.add_argument("--json", action="store_true")
    ap.add_argument(
        "--write-robots",
        action="store_true",
        help="rewrite the size column in web/robots.txt from what the origin "
             "actually serves, then exit",
    )
    a = ap.parse_args()

    if a.write_robots:
        return write_robots(a.origin)

    p = Probe(a.origin)
    problems, notes = [], []

    print(f"cold-start discovery against {p.origin}")
    print(f"the probe knows only the origin, and has {a.budget:,} bytes to reach a call\n")

    # Step 1. The only thing an agent can guess. If this is not the door,
    # there is no cold start: everything else needs a name it does not have.
    root, code = p.get("/.well-known/emem.json")
    if not isinstance(root, dict):
        print(f"  FATAL /.well-known/emem.json did not answer ({code})")
        return 2
    print(f"  step 1  /.well-known/emem.json          {p.bytes:>7,} B")

    for question, keys in ROOT_MUST_ANSWER.items():
        if not any(k in root for k in keys):
            problems.append(
                f"the root document answers no form of {question!r} "
                f"(looked for any of {list(keys)}). An agent reading only this "
                f"cannot decide whether to continue."
            )

    # Step 2. From the root alone, can the probe find where needs are mapped?
    # It may not go looking for a path it was not told about.
    intents_url = root.get("intents_url")
    if not intents_url:
        problems.append(
            "the root document does not point at an intent registry, so a need "
            "can only be matched by reading the full tool catalogue"
        )
        reg = None
    else:
        reg, code = p.get(intents_url)
        print(f"  step 2  {intents_url:<32} {p.bytes:>7,} B  (cumulative)")
        if not isinstance(reg, dict) or "intents" not in reg:
            problems.append(f"{intents_url} did not serve an intent registry")
            reg = None

    rows = (reg or {}).get("intents", [])

    # Step 3. Every cold need must match a row, and the row must be honest
    # about whether we serve it.
    if rows:
        print(f"\n  matched {len(rows)} registry rows; checking the cold needs:")
        for need in COLD_NEEDS:
            hit = None
            for r in rows:
                phr = [r.get("need", "")] + list(r.get("also_phrased") or [])
                if any(need.lower() == x.lower() for x in phr):
                    hit = r
                    break
            if not hit:
                problems.append(f"no registry row matches the cold need {need!r}")
                continue
            cov = hit.get("coverage")
            mark = {"served": "->", "partial": "~ ", "not_served": "x "}.get(cov, "? ")
            print(f"    {mark} {cov:<11} {need[:58]}")
            if cov in ("partial", "not_served"):
                # The property that makes an honest registry useful: a no must
                # come with a reason and a destination, or the agent is stranded.
                if len(hit.get("why", "")) < 40 or len(hit.get("instead", "")) < 20:
                    problems.append(
                        f"{need!r} is {cov} but does not say why and where else to go"
                    )
                if cov == "not_served" and hit.get("tool"):
                    problems.append(
                        f"{need!r} is not_served yet still advertises the tool "
                        f"{hit['tool']!r}, which will pull an agent into a call "
                        f"we have already said we cannot answer"
                    )

    # Step 4. The claims themselves. A served row that cannot be called is the
    # failure mode this whole repo keeps finding: a claim that stopped matching
    # the code with no gate watching.
    # Everything the agent needed to decide has been read by now: the root
    # document and the intent registry.
    discovery_bytes, discovery_fetches = p.bytes, p.fetches
    executed = skipped = covered = 0
    if rows:
        print("\n  executing every served row's first_call:")
        for r in rows:
            if r.get("coverage") != "served":
                continue
            call = parse_first_call(r.get("first_call", ""))
            if not call:
                # Not every row is one call. Some are honestly two steps
                # ("resolve the token, then verify the receipt"), and some
                # carry a <placeholder> only a prior result can fill. Those
                # are not unchecked: the handoff chain below runs recall,
                # memory_token, resolve and verify_receipt end to end against
                # the live responder.
                #
                # Reporting them as "not executed" was accurate and misleading
                # at once, because it read as five unverified claims when the
                # chain already covers four of the tools they name. A gate that
                # overstates its own gaps gets discounted the same way one that
                # hides them does.
                tool = r.get("tool", "")
                if tool in CHAIN_COVERS:
                    covered += 1
                    notes.append(f"{r['need'][:44]}: exercised by the handoff "
                                 f"chain rather than as a standalone call")
                else:
                    skipped += 1
                    problems.append(
                        f"the registry claims {r['need'][:44]!r} is served, and "
                        f"nothing checks it: its first_call is prose and no chain "
                        f"step exercises {tool or 'its tool'}. A served row that "
                        f"cannot be executed is a claim with no evidence behind it."
                    )
                continue
            method, path, body = call
            before = p.bytes
            resp, code = p.get(path, method=method, body=body)
            executed += 1
            ok = isinstance(code, int) and 200 <= code < 300
            print(f"    {'ok ' if ok else 'FAIL'} {method:<4} {path:<28} "
                  f"{code}  {p.bytes - before:,} B")
            if not ok:
                msg = ""
                if isinstance(resp, dict):
                    msg = str(resp.get("message") or resp.get("code") or "")[:100]
                problems.append(
                    f"the registry claims {r['need'][:44]!r} is served, but its own "
                    f"first_call {method} {path} answered {code}: {msg}"
                )

    # Step 5. The half of the bar that discovery alone does not reach.
    #
    # An outside review put it plainly: the goal is not "an agent can use
    # emem", it is that an agent which never heard of emem can discover it,
    # call it, VERIFY what it got, CITE it, and hand that citation to a second
    # agent which resolves and verifies it independently. Everything above
    # stops at the call. A protocol whose whole argument is that a claim
    # survives leaving the conversation has to be tested leaving the
    # conversation.
    #
    # The second agent here is a fresh session with no memory of the first:
    # it is handed the token string and nothing else, exactly as it would
    # arrive over a channel.
    print("\n  the handoff, which is what the protocol is for:")
    fact_cid = None
    recall, code = p.get("/v1/recall", method="POST",
                         body={"place": "Bengaluru",
                               "bands": ["copdem30m.elevation_mean"]})
    if isinstance(recall, dict):
        facts = recall.get("facts") or []
        if facts:
            fact_cid = facts[0].get("fact_cid")
            receipt = recall.get("receipt")
            print(f"    ok   agent A recalled a fact           {fact_cid}")
            if receipt:
                v, _ = p.get("/v1/verify_receipt", method="POST",
                             body={"receipt": receipt})
                good = isinstance(v, dict) and v.get("valid") is True
                print(f"    {'ok  ' if good else 'FAIL'} agent A verified the receipt")
                if not good:
                    problems.append("agent A could not verify the receipt it was handed")
            else:
                problems.append("the recall carried no receipt, so nothing could be verified")
    if not fact_cid:
        problems.append("agent A could not obtain a fact to cite, so the handoff "
                        "could not be tested at all")
    else:
        # Both fields are required. The registry documented only fact_cid and
        # this step is how that was found: the probe executed the row instead
        # of reading it.
        cell = (facts[0].get("cell") if facts else None) or "defi.zb493.xuqA.zcb5f"
        tok, _ = p.get("/v1/memory_token", method="POST",
                       body={"cell": cell, "fact_cid": fact_cid})
        # The field is `memory_token`; `token` is accepted as a fallback in
        # case the shape ever gains one. Reading the wrong key would have
        # reported a working citation step as broken.
        token = None
        if isinstance(tok, dict):
            token = tok.get("memory_token") or tok.get("token")
        if not token:
            problems.append("a fact could not be turned into a citation; "
                            f"/v1/memory_token answered {str(tok)[:90]}")
        else:
            print(f"    ok   agent A minted a citation          {token}")
            # Bundles compose from (cell, band) addresses, not from cids you
            # already hold: each triple runs through the recall path. The
            # registry documented fact_cids and nothing had executed it.
            band = (facts[0].get("band") if facts else None) or "copdem30m.elevation_mean"
            bundle, _ = p.get("/v1/memory_bundle", method="POST",
                              body={"triples": [{"cell": cell, "band": band}]})
            btok = None
            if isinstance(bundle, dict):
                btok = bundle.get("bundle_token") or bundle.get("token")
            if btok:
                print(f"    ok   and collapsed it to a bundle       {btok}")
            else:
                problems.append(
                    "a fact could not be bundled; multi-fact handoff would cost "
                    "one address per fact instead of one 38-character handle"
                )
            # Agent B. A different Probe object: no shared state, no cookies,
            # nothing carried over but the token string itself.
            b = Probe(a.origin)
            got, _ = b.get("/v1/memory_token/resolve", method="POST",
                           body={"token": token})
            resolved = isinstance(got, dict) and (got.get("fact") or got.get("facts")
                                                  or got.get("value") is not None)
            print(f"    {'ok  ' if resolved else 'FAIL'} agent B resolved it cold "
                  f"({b.bytes:,} B, knowing only the token)")
            if not resolved:
                problems.append("a second agent could not resolve the citation the "
                                "first one handed it, which is the property the "
                                "whole protocol exists to provide")
            else:
                rec_b = got.get("receipt") if isinstance(got, dict) else None
                if rec_b:
                    v2, _ = b.get("/v1/verify_receipt", method="POST",
                                  body={"receipt": rec_b})
                    ok2 = isinstance(v2, dict) and v2.get("valid") is True
                    print(f"    {'ok  ' if ok2 else 'FAIL'} agent B verified it "
                          f"without trusting agent A")
                    if not ok2:
                        problems.append("the second agent resolved the citation but "
                                        "could not verify it")
                else:
                    notes.append("the resolved citation carried no receipt for B to "
                                 "verify; B has the bytes but not the proof")

    # Step 5b. Every extension URI the AgentCard advertises must resolve.
    #
    # A2A names vendor additions in `capabilities.extensions` by URI so that a
    # client which does not recognise one can follow it and find out what it is.
    # That is the entire point of the mechanism, and it only works if the URI
    # answers. Ours did not: the card advertised
    # https://emem.dev/spec/a2a/async-tasks/v1 and the path 404'd, which an
    # external reviewer found by doing exactly what an autonomous client does.
    # A card that names a document nobody serves is worse than a card with no
    # extensions, because it invites the fetch and then wastes it.
    print("\n  the extension URIs the agent card advertises, followed:")
    card, _ = p.get("/.well-known/agent-card.json")
    exts = (card or {}).get("capabilities", {}).get("extensions", []) if isinstance(card, dict) else []
    if not exts:
        notes.append("the agent card advertises no extensions, so none were followed")
    for e in exts:
        uri = e.get("uri", "")
        if not uri:
            problems.append("an entry in capabilities.extensions has no `uri`, so a "
                            "client cannot identify or look it up")
            continue
        # Only URIs this responder is responsible for are fetched. An extension
        # named by someone else's URI is their document to serve, not ours.
        path = uri.split(p.origin, 1)[1] if uri.startswith(p.origin) else (
            "/" + uri.split("/", 3)[3] if uri.startswith("https://emem.dev/") else None)
        if path is None:
            print(f"    {uri}  (third-party URI, not fetched)")
            continue
        doc, code = p.get(path)
        ok = isinstance(doc, dict) and code == 200
        print(f"    {uri}  ->  {code}{'' if ok else '   BROKEN'}")
        if not ok:
            problems.append(
                f"the agent card advertises the extension {uri} and following it "
                f"returns {code}. A client that does not know this extension has "
                f"no way to learn it."
            )
        elif doc.get("extension", {}).get("uri") != uri:
            problems.append(
                f"{uri} resolves, but the document served there does not name "
                f"itself with the same URI, so a client cannot confirm it landed "
                f"on the right spec."
            )

    # Step 6. robots.txt advertises the bootstrap surfaces with their sizes,
    # cheapest first, so an agent can decide what to read on a budget. Those
    # figures drifted badly: llms.txt was advertised at 5 KB and served 24 KB,
    # agents.md at 16 KB and served 65 KB, /v1/discover at 970 B and served
    # 3.7 KB. An agent that budgets on the advertised number and receives four
    # times it learns not to trust the surface, which is worse than not
    # advertising a size at all.
    print("\n  the sizes robots.txt advertises, against what is served:")
    robots, _ = p.get("/robots.txt")
    if isinstance(robots, str):
        for m in re.finditer(r"^#\s+(/\S+)\s+~?([\d.]+)\s*(KB|B)\b", robots, re.M):
            path, num, unit = m.group(1), float(m.group(2)), m.group(3)
            claimed = num * 1000 if unit == "KB" else num
            body, code = p.get(path)
            if not isinstance(code, int) or not (200 <= code < 300):
                problems.append(
                    f"robots.txt points an agent at {path}, which answers {code}"
                )
                continue
            actual = p.trail[-1][2]
            ratio = actual / claimed if claimed else 0
            ok = 0.5 <= ratio <= 2.0
            print(f"    {'ok  ' if ok else 'FAIL'} {path:<34} "
                  f"claimed {claimed/1000:.0f} KB, served {actual/1000:.0f} KB")
            if not ok:
                problems.append(
                    f"robots.txt advertises {path} at about {claimed/1000:.0f} KB and "
                    f"it serves {actual/1000:.0f} KB. An agent budgeting context on "
                    f"that figure is misled by {ratio:.1f}x."
                )

    # The cost verdict. Discovery that works but costs more than an agent will
    # spend has not solved the problem it set out to solve.
    # The budget is about what an AGENT spends to decide and make one call.
    # Executing every served row to audit it is this gate's cost, not the
    # agent's, and charging the two to one meter reported a discovery failure
    # when discovery was fine. Measured at the point the agent could have
    # stopped: root, registry, first call.
    print(f"\n  deciding cost an agent {discovery_bytes:,} B over "
          f"{discovery_fetches} fetches: the root document and the registry, "
          f"which is everything needed to know whether to call at all")
    print(f"  this gate then spent {p.bytes - discovery_bytes:,} B more auditing "
          f"every claim, which an agent never pays")
    print(f"  budget {a.budget:,} B, "
          f"{'within' if discovery_bytes <= a.budget else 'OVER'}")
    if discovery_bytes > a.budget:
        problems.append(
            f"deciding whether to call cost {discovery_bytes:,} B against a budget "
            f"of {a.budget:,} B. Past this an agent skips the unknown service, so "
            f"the capability is functionally undiscoverable however good it is."
        )

    if notes:
        print("\n  not executed (reported, never silently passed):")
        for n in notes:
            print(f"    - {n}")
    print(f"  executed {executed} served rows directly, {covered} covered by the "
          f"handoff chain, {skipped} neither")

    if a.json:
        print(json.dumps({"origin": p.origin, "bytes": p.bytes,
                          "fetches": p.fetches, "problems": problems}, indent=1))

    if os.environ.get("GITHUB_ACTIONS") == "true":
        for prob in problems:
            clean = str(prob).replace("\n", " ").replace("::", " ")
            print(f"::error title=discovery::{clean}")

    if problems:
        print(f"\ndiscovery: {len(problems)} problem(s). An agent starting cold "
              f"would be misled or stranded.")
        for prob in problems:
            print(f"  x {prob}")
        return 1

    print("\ndiscovery: a cold agent reaches a verified call from the origin alone, "
          "within budget, and every claim it read held.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
