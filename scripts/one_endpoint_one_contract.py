#!/usr/bin/env python3
"""Two documents describe each emem endpoint. Check they describe the same one.

`/openapi.json` is what a developer reads. The MCP `inputSchema` is what an
agent reads. They are written separately and nothing compared them, so they
drifted in both directions and each direction fails differently.

  OpenAPI has a parameter MCP does not.  The capability exists and the agent
  cannot see it. `/v1/log/inclusion` took `tree_size` -- prove a leaf was in a
  tree head you pinned earlier, which is the whole point of an audit log -- and
  the MCP schema omitted it, so an agent connected here could only ever prove
  inclusion under the current head. This is a FAILURE: something real is
  unreachable from the surface most callers use.

  MCP has a parameter OpenAPI does not.  The tool accepts it, the published
  HTTP contract does not mention it, and a developer who reads that contract
  writes a client that never sends it. `POST /v1/recall` documented neither
  `place` nor `lat`/`lng` while the tool took both. This is a RATCHET: there
  were many on the day this check was written, they are documentation debt
  rather than a broken call, and the count may fall but not rise.

The one thing this cannot see is a parameter missing from BOTH, which is why
it is not the only check on the pair.

  python3 scripts/one_endpoint_one_contract.py
  python3 scripts/one_endpoint_one_contract.py --origin http://127.0.0.1:5051
  python3 scripts/one_endpoint_one_contract.py --write-baseline

Exit 0 clean, 1 drift, 2 the responder did not answer, 3 a fault on our side.
"""
from __future__ import annotations

import argparse
import json
import pathlib
import sys
import urllib.request

REPO = pathlib.Path(__file__).resolve().parent.parent
BASELINE = REPO / "docs" / "registries" / "mcp-openapi-gap.json"
UA = "emem-one-endpoint-one-contract/1 (+https://emem.dev)"


def get(url: str, body: bytes | None = None, headers: dict | None = None):
    req = urllib.request.Request(url, data=body, headers={"user-agent": UA, **(headers or {})})
    with urllib.request.urlopen(req, timeout=90) as r:
        return r.read().decode()


def mcp_tools(origin: str) -> dict:
    """Every tool, paged, from the endpoint that advertises all of them."""
    out, cursor = {}, None
    while True:
        params = {} if cursor is None else {"cursor": cursor}
        raw = get(origin + "/mcp/full",
                  json.dumps({"jsonrpc": "2.0", "id": 1, "method": "tools/list",
                              "params": params}).encode(),
                  {"content-type": "application/json",
                   "accept": "application/json, text/event-stream",
                   "mcp-protocol-version": "2025-11-25"})
        if raw.lstrip().startswith("event:") or "\ndata: " in raw:
            raw = [l[6:] for l in raw.splitlines() if l.startswith("data: ")][-1]
        res = json.loads(raw)["result"]
        for t in res["tools"]:
            out[t["name"]] = t
        cursor = res.get("nextCursor")
        if not cursor:
            return out


def rest_params(oa: dict, op: dict) -> set[str]:
    comp = (oa.get("components") or {}).get("schemas") or {}

    def deref(sc, depth=0):
        while isinstance(sc, dict) and "$ref" in sc and depth < 6:
            sc = comp.get(sc["$ref"].split("/")[-1], {})
            depth += 1
        return sc if isinstance(sc, dict) else {}

    names = {p["name"] for p in (op.get("parameters") or [])
             if isinstance(p, dict) and p.get("in") == "query" and p.get("name")}
    body = ((op.get("requestBody") or {}).get("content") or {}) \
        .get("application/json", {}).get("schema")
    if body:
        names |= set(deref(body).get("properties") or {})
    return names


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--origin", default="https://emem.dev")
    ap.add_argument("--write-baseline", action="store_true",
                    help="record the current one-sided count as the ceiling")
    a = ap.parse_args()
    origin = a.origin.rstrip("/")

    try:
        oa = json.loads(get(origin + "/openapi.json"))
        tools = mcp_tools(origin)
    except Exception as e:  # noqa: BLE001
        print(f"could not read {origin}: {type(e).__name__}: {e}")
        print("Undetermined, not clean: both documents come from the responder.")
        return 2

    ops = {}
    for path, item in (oa.get("paths") or {}).items():
        for method, op in item.items():
            if isinstance(op, dict) and op.get("operationId"):
                ops[op["operationId"]] = op

    unreachable: list[str] = []
    undocumented: list[tuple[str, list[str]]] = []
    unmatched = 0
    for name, t in sorted(tools.items()):
        op = next((ops[c] for c in (name, f"{name}_post", f"{name}_get") if c in ops), None)
        if op is None:
            unmatched += 1
            continue
        mcp = set((t.get("inputSchema") or {}).get("properties") or {})
        rest = rest_params(oa, op)
        for gone in sorted(rest - mcp):
            unreachable.append(f"{name}: /openapi.json documents `{gone}`, the tool schema does not")
        extra = sorted(mcp - rest)
        if extra:
            undocumented.append((name, extra))

    if not tools or not ops:
        print("read no tools or no operations; nothing was compared")
        return 3

    total_extra = sum(len(e) for _, e in undocumented)
    if a.write_baseline:
        BASELINE.parent.mkdir(parents=True, exist_ok=True)
        BASELINE.write_text(json.dumps({
            "_what": "Parameters the MCP tool schema declares and /openapi.json does not. "
                     "A ceiling, not a target: it may fall, it may not rise.",
            "_how": "python3 scripts/one_endpoint_one_contract.py --write-baseline",
            "parameters_documented_only_for_agents": total_extra,
            "tools": {n: e for n, e in undocumented},
        }, indent=2) + "\n", encoding="utf-8")
        print(f"baseline written: {total_extra} parameter(s) across {len(undocumented)} tool(s)")
        return 0

    print(f"{len(tools)} tools, {unmatched} with no operationId in /openapi.json")

    if unreachable:
        print(f"\nCAPABILITY AN AGENT CANNOT SEE ({len(unreachable)}):")
        for row in unreachable:
            print("  ", row)

    ceiling = None
    if BASELINE.exists():
        try:
            ceiling = json.loads(BASELINE.read_text(encoding="utf-8")) \
                .get("parameters_documented_only_for_agents")
        except Exception as e:  # noqa: BLE001
            print(f"baseline unreadable: {e}")
            return 3
    if ceiling is None:
        print(f"\nno baseline at {BASELINE.relative_to(REPO)}; run --write-baseline")
        return 3

    print(f"\nparameters an agent can send that /openapi.json does not document: "
          f"{total_extra} (ceiling {ceiling})")
    if total_extra > ceiling:
        print("  the HTTP contract fell further behind the tool surface. Add the parameters")
        print("  to the /openapi.json entry beside the route, or lower them here on purpose.")
        for n, e in undocumented:
            print(f"    {n}: {e}")

    if unreachable or total_extra > ceiling:
        return 1
    print("\nboth documents describe the same endpoints.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
