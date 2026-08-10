#!/usr/bin/env python3
"""Every tool that declares an outputSchema must actually satisfy it.

Why this exists
---------------
`outputSchema` is the only descriptor field that binds runtime behaviour: the
MCP spec requires a tool declaring one to return conforming `structuredContent`
on every successful call. A unit test can check the schema is well-formed JSON
Schema; only a live call can check the RESULT conforms to it.

The gap was real. A sweep of the live responder found eleven declaring tools,
all conforming on success, and turned up a documentation defect instead: three
places in this repo said the obligation held "on EVERY call". It does not —
an `isError` result carries prose and no structured mirror, because there is no
result to mirror, and synthesising a conforming object for a failure would make
the schema describe something that did not happen. The code was right and the
comments overstated it, which is the failure mode this repo keeps finding.

So this asserts the thing that is true: declare a schema, and a successful call
returns structured content that validates against it.

Usage:
  scripts/output_schema_conformance.py             # against https://emem.dev
  scripts/output_schema_conformance.py --check     # non-zero exit on violation
  EMEM_RESPONDER=http://localhost:5051 scripts/... # against a local node
"""
from __future__ import annotations

import json
import os
import sys
import urllib.request

RESPONDER = os.environ.get("EMEM_RESPONDER", "https://emem.dev").rstrip("/")
CELL = "defi.zb4e3.zaeed.fEya"

# One successful call per declaring tool. A tool added to the declaring set
# without an entry here fails the run rather than being skipped silently: an
# unverifiable promise is the thing this script exists to prevent.
ARGS: dict[str, dict] = {
    "emem_recall": {"cell": CELL, "bands": ["indices.ndvi"]},
    "emem_memory_token": {"cell": CELL, "fact_cid": "v" * 52},
    "emem_guard_verdict": {"texts": ["Elevation at Leh is 3500 m."]},
    "emem_echo_verify": {},  # filled at runtime with a real token
    "emem_errors": {},
    "emem_manifests": {},
    "emem_capabilities": {},
    "emem_grid_info": {},
    "emem_substrates": {},
    "emem_log_sth": {},
    "emem_log_witnesses": {},
}


def _post(path: str, body: dict, timeout: int = 90):
    req = urllib.request.Request(
        RESPONDER + path, data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json",
                 "Accept": "application/json, text/event-stream"})
    return json.load(urllib.request.urlopen(req, timeout=timeout))


def _rpc(method: str, params: dict, ep: str = "/mcp"):
    return _post(ep, {"jsonrpc": "2.0", "id": 1, "method": method, "params": params})


def _all_tools(ep: str = "/mcp/full") -> list[dict]:
    """Every tool, following `nextCursor` to the end.

    `tools/list` was one response until the catalog outgrew the 24KB result
    cap and the server started paging it. This script read `result.tools` from
    the first page and stopped, so it saw 27 of 107 tools and 1 of the 11 that
    declare an outputSchema. It printed "1 of 27 tools declare an outputSchema"
    and passed, which reads like good news and was the gate losing 10 of its 11
    subjects. Nothing in the output said coverage had shrunk, because a page is
    a well-formed answer to the question it asked.

    Paging is also why the count is not stable: the page break is a byte
    budget, so the first page holds 27 or 28 tools depending on how long the
    descriptions are that day. A gate whose subject set moves with description
    length is measuring the wrong thing entirely.
    """
    tools: list[dict] = []
    cursor = None
    seen_cursors: set[str] = set()
    while True:
        params = {"tier": "all"} if cursor is None else {"tier": "all", "cursor": cursor}
        r = _rpc("tools/list", params, ep=ep)["result"]
        tools.extend(r.get("tools") or [])
        cursor = r.get("nextCursor")
        if not cursor:
            return tools
        # A server that returns the cursor it was handed would spin here
        # forever. Stop and say so rather than hang a CI job.
        if cursor in seen_cursors:
            raise RuntimeError(f"tools/list repeated cursor {cursor!r}; paging is looping")
        seen_cursors.add(cursor)


def _real_token() -> tuple[str, str] | None:
    """A token that resolves, so echo_verify is exercised on its success path."""
    try:
        d = _post("/v1/recall", {"cell64": CELL, "bands": ["indices.ndvi"]})
        cid = (d.get("receipt") or {}).get("fact_cids", [None])[0]
        val = str((d.get("facts") or [{}])[0].get("value"))
        return (f"emem:fact:{CELL}:{cid}", val) if cid else None
    except Exception:
        return None


def main() -> int:
    check = "--check" in sys.argv
    try:
        import jsonschema
    except ImportError:
        print("jsonschema not installed; pip install jsonschema")
        return 0 if not check else 2

    try:
        tools = _all_tools()
    except Exception as e:
        print(f"  (skipped: {RESPONDER} unreachable: {e})")
        return 0 if not check else 2

    declaring = {t["name"]: t["outputSchema"] for t in tools if t.get("outputSchema")}
    print(f"{len(declaring)} of {len(tools)} tools declare an outputSchema")

    # The catalog is paged and this script follows the cursor, so a run that
    # somehow sees only a page would otherwise report a clean sweep over a
    # fraction of the surface. Below the smallest page ever observed (27) is
    # not "fewer tools shipped", it is paging broken again.
    if len(tools) < 40:
        problems_early = (f"tools/list returned {len(tools)} tools, which is about one "
                          f"page. Paging is not being followed and this run would "
                          f"check a fraction of the declaring tools.")
        print(f"\nVIOLATIONS:\n  x {problems_early}")
        return 1 if check else 0

    tok = _real_token()
    if tok:
        ARGS["emem_echo_verify"] = {"token": tok[0], "claimed_value": tok[1]}

    problems: list[str] = []
    # A fixture for a tool that no longer declares a schema is a decision about
    # a file nobody can look at: it stops being exercised and stops being read,
    # and the next person takes its presence as evidence the tool is covered.
    for name in sorted(set(ARGS) - set(declaring)):
        problems.append(f"{name}: has a success-path call in this script but no longer "
                        f"declares an outputSchema; drop the entry or restore the schema")
    for name, schema in sorted(declaring.items()):
        args = ARGS.get(name)
        if args is None:
            problems.append(f"{name}: declares a schema but this script has no "
                            f"success-path call for it, so the promise is unverified")
            continue
        try:
            r = _rpc("tools/call", {"name": name, "arguments": args})["result"]
        except Exception as e:
            problems.append(f"{name}: call failed: {str(e)[:80]}")
            continue
        if r.get("isError"):
            problems.append(f"{name}: test call returned isError; fix the args in "
                            f"this script so the SUCCESS path is what gets checked")
            continue
        sc = r.get("structuredContent")
        if sc is None:
            problems.append(f"{name}: declared an outputSchema and returned no "
                            f"structuredContent on a successful call")
            continue
        try:
            jsonschema.validate(sc, schema)
            print(f"  ok   {name} ({len(sc)} keys)")
        except jsonschema.ValidationError as e:
            problems.append(f"{name}: structuredContent violates its own schema: "
                            f"{e.message[:90]}")

    if problems:
        print("\nVIOLATIONS:")
        for p in problems:
            print(f"  x {p}")
        return 1 if check else 0
    print("\nEvery declared outputSchema is honoured on the success path.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
