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
        tools = _rpc("tools/list", {}, ep="/mcp/full")["result"]["tools"]
    except Exception as e:
        print(f"  (skipped: {RESPONDER} unreachable: {e})")
        return 0 if not check else 2

    declaring = {t["name"]: t["outputSchema"] for t in tools if t.get("outputSchema")}
    print(f"{len(declaring)} of {len(tools)} tools declare an outputSchema")

    tok = _real_token()
    if tok:
        ARGS["emem_echo_verify"] = {"token": tok[0], "claimed_value": tok[1]}

    problems: list[str] = []
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
