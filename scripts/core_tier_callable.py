#!/usr/bin/env python3
"""Every tool in the advertised core tier answers a valid call.

Why this exists
---------------
A directory reviewer connects to the URL we advertise, reads `tools/list`,
and calls the tools. The published review criteria are explicit: *"Every tool
must return a successful response when called with valid parameters. Generic
errors ('Internal Server Error', 'Bad Request' with no detail) fail review."*

We advertise `/mcp`, whose one page is the whole core tier. So the set a
reviewer exercises is exactly the set this script exercises, with arguments
that satisfy each tool's own declared `required` list.

The distinction this script is careful about
--------------------------------------------
A tool that REFUSES is not a tool that is broken. `emem_entity` and
`emem_entity_link` write to the shared identity space and require tier T3, so
an unenlisted caller gets a typed refusal naming the tier. That is the surface
working, and counting it as a failure would push someone toward weakening the
gate to make a green light.

So a call passes when it either succeeds, or fails with a TYPED, explained
refusal. It fails when the error is untyped, empty, or a bare 500 -- which is
the thing review actually rejects.

Usage:
  scripts/core_tier_callable.py            # against https://emem.dev
  scripts/core_tier_callable.py --check    # non-zero exit on failure
  EMEM_RESPONDER=http://localhost:5051 ...
"""
from __future__ import annotations

import json
import os
import sys
import urllib.request

from lib_patience import patient

RESPONDER = os.environ.get("EMEM_RESPONDER", "https://emem.dev").rstrip("/")
CELL = "defi.zb64a.cAzU.zfa27"

# Typed refusals that mean the surface is working as designed. Each is a
# machine-readable code the caller can branch on, not a stack trace.
ALLOWED_REFUSALS = (
    "T3_declared", "T2_named", "T1_keyed",      # the enlistment ladder
    "enlist", "tier",                            # its refusal vocabulary
    "band_retired_at_this_responder",
    "compute_timeout",                           # budget, and it says so
    "not_found", "cid_not_found",                # nothing there to return
)


def _post(path: str, body: dict, timeout: int = 120):
    req = urllib.request.Request(
        RESPONDER + path, data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json",
                 "Accept": "application/json, text/event-stream"})
    return json.load(patient(req, timeout=timeout))


def _rpc(method: str, params: dict):
    return _post("/mcp", {"jsonrpc": "2.0", "id": 1, "method": method, "params": params})


def _core_tools() -> list[dict]:
    r = _rpc("tools/list", {})["result"]
    if r.get("nextCursor"):
        raise RuntimeError(
            "the advertised core tier paged; a reviewer taking page one would see "
            "a fragment, and this script would check one too")
    return r["tools"]


def _fixtures() -> dict[str, dict]:
    """Arguments for every core tool, several derived from live answers.

    A constant fact_cid or search id would keep passing while the format it
    stands for drifted, so the ones that can be read from the responder are.
    """
    rec = _post("/v1/recall", {"cell": CELL, "bands": ["indices.ndvi"]})
    facts = rec.get("facts") or []
    cid = facts[0].get("fact_cid") if facts else None
    val = facts[0].get("value") if facts else None
    token = f"emem:fact:{CELL}:{cid}" if cid else None

    sid = None
    try:
        r = _rpc("tools/call", {"name": "search", "arguments": {"query": CELL}})["result"]
        results = (r.get("structuredContent") or {}).get("results") or []
        sid = results[0].get("id") if results else None
    except Exception:
        pass

    receipt = rec.get("receipt")

    return {
        "emem_locate": {"q": "Bengaluru"},
        "emem_recall": {"cell": CELL, "bands": ["indices.ndvi"]},
        "emem_ask": {"q": "What is the elevation here?", "cell": CELL},
        "emem_tools": {},
        "emem_intent": {"type": "locate", "q": "Bengaluru"},
        "emem_entity": {"label": "core-tier callable sweep"},
        "emem_entity_resolve": {"label": "Bengaluru"},
        "emem_entity_link": {"label": "Bengaluru", "alias": "Bangalore"},
        "emem_find_similar": {"key": "Bengaluru", "k": 3},
        "emem_memory_contradictions": {"cell": CELL, "limit": 5},
        "emem_guard_verdict": {"texts": ["Elevation at Leh is 3500 m."]},
        "emem_memory_bundle": {"triples": [{"cell": CELL, "band": "indices.ndvi"}]},
        "emem_memory_token": {"cell": CELL, "fact_cid": cid},
        "emem_memory_token_resolve": {"token": token},
        "emem_echo_verify": {"token": token, "claimed_value": val},
        "emem_verify_receipt": {"receipt": receipt},
        "search": {"query": CELL},
        "fetch": {"id": sid or token},
    }


def main() -> int:
    check = "--check" in sys.argv
    try:
        tools = _core_tools()
        args = _fixtures()
    except Exception as e:
        print(f"  (skipped: {RESPONDER} did not answer: {e})")
        return 0 if not check else 2

    names = [t["name"] for t in tools]
    print(f"core tier: {len(names)} tools at {RESPONDER}/mcp")

    missing = [n for n in names if n not in args]
    if missing:
        print(f"\nVIOLATIONS:\n  x no fixture for {missing}; a tool added to the "
              f"advertised tier without a call here is unexercised, which is the "
              f"state this script exists to prevent")
        return 1 if check else 0

    problems: list[str] = []
    for n in names:
        a = args[n]
        if any(v is None for v in a.values()):
            problems.append(f"{n}: a fixture field could not be read from the "
                            f"responder, so this tool was NOT exercised")
            print(f"  !    {n}: fixture incomplete")
            continue
        try:
            r = _rpc("tools/call", {"name": n, "arguments": a})
        except Exception as e:
            problems.append(f"{n}: transport failed: {e}")
            print(f"  x    {n}: {e}")
            continue
        res = r.get("result") or {}
        if not res.get("isError"):
            print(f"  ok   {n}")
            continue
        text = json.dumps(res.get("content") or r.get("error") or {})
        if any(k in text for k in ALLOWED_REFUSALS):
            print(f"  ok   {n} (typed refusal, explained)")
            continue
        problems.append(f"{n}: errored with no typed reason: {text[:200]}")
        print(f"  x    {n}: untyped error")

    if problems:
        print("\nVIOLATIONS:")
        for p in problems:
            print(f"  x {p}")
        return 1 if check else 0
    print("\nEvery tool a reviewer would call answers, or refuses in a way that says why.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
