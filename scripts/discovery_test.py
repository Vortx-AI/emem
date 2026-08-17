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
            with urllib.request.urlopen(req, timeout=60) as r:
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


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--origin", default=os.environ.get("EMEM_ORIGIN", DEFAULT_ORIGIN))
    ap.add_argument("--budget", type=int, default=DEFAULT_BUDGET)
    ap.add_argument("--json", action="store_true")
    a = ap.parse_args()

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
    executed = skipped = 0
    if rows:
        print("\n  executing every served row's first_call:")
        for r in rows:
            if r.get("coverage") != "served":
                continue
            call = parse_first_call(r.get("first_call", ""))
            if not call:
                skipped += 1
                notes.append(f"{r['need'][:44]}: first_call is prose, not executed")
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
        tok, _ = p.get("/v1/memory_token", method="POST", body={"fact_cid": fact_cid})
        token = (tok or {}).get("token") if isinstance(tok, dict) else None
        if not token:
            problems.append("a fact could not be turned into a citation; "
                            f"/v1/memory_token answered {str(tok)[:90]}")
        else:
            print(f"    ok   agent A minted a citation          {token}")
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

    # The cost verdict. Discovery that works but costs more than an agent will
    # spend has not solved the problem it set out to solve.
    print(f"\n  {p.fetches} fetches, {p.bytes:,} B to go from a need to a verified call")
    print(f"  budget {a.budget:,} B, {'within' if p.bytes <= a.budget else 'OVER'}")
    if p.bytes > a.budget:
        problems.append(
            f"reaching a first call cost {p.bytes:,} B against a budget of "
            f"{a.budget:,} B. Past this an agent skips the unknown service, so "
            f"the capability is functionally undiscoverable however good it is."
        )

    if notes:
        print("\n  not executed (reported, never silently passed):")
        for n in notes:
            print(f"    - {n}")
    print(f"  executed {executed} served rows, {skipped} unparseable")

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
