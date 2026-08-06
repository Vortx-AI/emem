#!/usr/bin/env python3
"""MCP/REST parity: the same question down both transports must get the same answer.

Why this exists
---------------
emem answers on two surfaces from one set of handlers. That is a promise, and
an unchecked promise is how a flag ends up meaning one thing over JSON-RPC and
another over REST. The audit rounds found exactly that twice: `deterministic:
true` and a `provenance` allowlist both returned a value on one path and
refused on the other, so an agent's answer depended on which door it used.

`sgozfgkr` built the first cut of this harness against production, specified
what it should assert, and handed the CI integration back. This is that, with
their two correctness notes taken as written:

  1. Compare facts at (band, value), never whole envelopes. The envelopes
     legitimately differ: MCP wraps in stringified JSON, truncates at the wire
     budget, and both carry per-call volatile fields. Their first cut
     mis-flagged `recall/plain` as DIVERGE for exactly this reason.
  2. BOTH_ERR is a PASS. Two paths that refuse identically have parity on the
     error path, which is the property the deterministic-flag fix restored and
     the thing most likely to regress silently.

Categories
----------
  MATCH       both paths returned the same facts at (band, value)
  MCP_TRUNCATED  REST carries more, because the MCP wire budget slimmed it.
              A pass: the verdict-bearing fields survived. MCP carrying MORE
              than REST is never this, it is a DIVERGE.
  BOTH_ERR    both refused, with the same error code. Parity holds.
  ONLY_ERR    one refused and the other answered. The real divergence signal.
  DIVERGE     both answered, and the facts differ beyond tolerance
  TOLERANCE   both answered and agree within EPSILON but not exactly

Exit codes
----------
  0  every case is MATCH, BOTH_ERR or TOLERANCE
  1  any ONLY_ERR or DIVERGE
  2  the harness could not run (responder unreachable, bad arguments)

Usage
-----
  python3 scripts/parity.py                       # against https://emem.dev
  python3 scripts/parity.py --origin http://127.0.0.1:5051
  python3 scripts/parity.py --json                # machine-readable summary
"""

from __future__ import annotations

import argparse
import json
import sys
import urllib.error
import urllib.parse
import urllib.request

DEFAULT_ORIGIN = "https://emem.dev"

# A pinned fixture so cache state cannot flake the run. Bengaluru, one cell,
# one date; every case below addresses this or nothing at all.
CELL = "defi.zb493.xuqA.zcb5f"
PLACE = "Bengaluru"

# Agreement window for aggregates. Not a courtesy: a polygon aggregate sums
# floats over cells that arrive from a parallel fan-out, so the last digits
# move between identical calls on ONE transport. sgozfgkr measured that and it
# is recorded as a known softness rather than hidden by a loose comparison
# everywhere. Only the cases marked `tolerance` use it.
EPSILON = 1e-9

# Fields that legitimately differ per call. Excluding them is what makes the
# comparison about the ANSWER rather than about the envelope.
VOLATILE = {
    "served_at",
    "request_id",
    "signed_at",
    "cost",
    "receipt",
    "_emem_truncation",
    "_meta",
    "lat",
    "lon",
    "place_label",
    "place_resolution",
    "captured_at_range",
    "elapsed_ms",
    "cache",
}


class Case:
    """One (tool, REST path, args) triple and how to compare it."""

    def __init__(self, name, tool, path, args, tolerance=False, shape_only=False,
                 method="POST"):
        self.name = name
        self.tool = tool
        self.path = path
        self.args = args
        self.tolerance = tolerance
        self.shape_only = shape_only
        # The REST twin's verb. Not uniform: some reads are GET with a query
        # string and some are POST with a body, and calling one as the other
        # returns 405, which would look like a divergence and is not.
        self.method = method


# The seed table. Every flag that produced a bug in an audit round is here,
# plus the shape cases. Grown by adding rows, not by loosening assertions.
CASES = [
    Case("recall/plain", "emem_recall", "/v1/recall",
         {"cell": CELL, "bands": ["copdem30m.elevation_mean"]}),
    Case("recall/multi-band", "emem_recall", "/v1/recall",
         {"cell": CELL, "bands": ["copdem30m.elevation_mean", "landcover.class"]}),
    # The two rows that ARE the fix from audit round 4: a deterministic filter
    # and a provenance allowlist must refuse identically on both paths. If
    # either ever silently returns a value again these flip to ONLY_ERR.
    Case("recall/deterministic-drops-model-output", "emem_recall", "/v1/recall",
         {"cell": CELL, "bands": ["weather.temperature_2m"], "deterministic": True}),
    Case("recall/provenance-allowlist", "emem_recall", "/v1/recall",
         {"cell": CELL, "bands": ["weather.temperature_2m"],
          "provenance": ["direct_sensor"]}),
    Case("recall/deterministic-keeps-sensor", "emem_recall", "/v1/recall",
         {"cell": CELL, "bands": ["copdem30m.elevation_mean"], "deterministic": True}),
    Case("state/fingerprint", "emem_state", "/v1/state", {"cell": CELL}),
    Case("bands/catalog", "emem_bands", "/v1/bands", {}, shape_only=True,
         method="GET"),
    Case("grid_info", "emem_grid_info", "/v1/grid_info", {"cell": CELL},
         method="GET"),
    # The guard's own surface, both doors. A grounding gate whose two
    # transports disagreed would be the most embarrassing possible divergence.
    Case("guard/verdict-clean", "emem_guard_verdict", "/v1/guard/verdict",
         {"texts": ["nothing to see here"]}),
    Case("guard/verdict-cited", "emem_guard_verdict", "/v1/guard/verdict",
         {"texts": [f"per emem:fact:{CELL}:yqbolgeoycqkvj3zkxukb4bjw4odhpwvfzqo3fbgwf4spk45zala"]}),
    Case("guard/claim-gating", "emem_guard_verdict", "/v1/guard/verdict",
         {"texts": ["Elevation in Leh is 3500 m."], "claim_gating": True}),
    # Aggregates: known non-deterministic in the last digits, asserted within
    # EPSILON and flagged rather than asserted equal.
    Case("ndvi/aggregate", "emem_ndvi", "/v1/ndvi", {"place": PLACE}, tolerance=True),
]


def _post(url, payload, timeout=120):
    req = urllib.request.Request(
        url,
        data=json.dumps(payload).encode(),
        headers={"content-type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.loads(r.read().decode())


def _get(url, params, timeout=120):
    if params:
        url = f"{url}?{urllib.parse.urlencode(_flatten(params))}"
    with urllib.request.urlopen(url, timeout=timeout) as r:
        return json.loads(r.read().decode())


def _flatten(params):
    """Query-string form of an args dict. Lists become comma-joined."""
    out = {}
    for k, v in params.items():
        out[k] = ",".join(str(x) for x in v) if isinstance(v, list) else v
    return out


def call_rest(origin, case):
    """Returns (ok, payload). ok=False means the responder refused."""
    try:
        if case.method == "GET":
            return True, _get(f"{origin}{case.path}", case.args)
        return True, _post(f"{origin}{case.path}", case.args)
    except urllib.error.HTTPError as e:
        try:
            return False, json.loads(e.read().decode())
        except Exception:
            return False, {"code": f"http_{e.code}"}
    except Exception as e:  # network, timeout, DNS
        return False, {"code": "unreachable", "message": str(e)}


def call_mcp(origin, case):
    try:
        out = _post(
            f"{origin}/mcp",
            {"jsonrpc": "2.0", "id": 1, "method": "tools/call",
             "params": {"name": case.tool, "arguments": case.args}},
        )
    except Exception as e:
        return False, {"code": "unreachable", "message": str(e)}
    if "error" in out:
        return False, {"code": "jsonrpc_error", "message": str(out["error"])[:200]}
    result = out.get("result", {})
    text = (result.get("content") or [{}])[0].get("text", "")

    # `isError` FIRST, before any parse. An MCP tool error is a successful
    # JSON-RPC envelope carrying isError:true and a PROSE body, so a harness
    # that parses first and checks the flag second reads a refusal as an
    # answer and reports a divergence that is not there. This harness did
    # exactly that on its first run, which is the same class of mistake
    # sgozfgkr warned about in the hand-off: build the assertion at the level
    # the protocol actually uses.
    if result.get("isError"):
        return False, {"code": _rest_code_for(text), "message": text[:200]}

    try:
        payload = json.loads(text)
    except Exception:
        # A tool whose result is prose rather than JSON. Shape-only cases
        # tolerate this; value cases do not.
        return True, {"_text": text}
    if isinstance(payload, dict) and payload.get("schema") == "emem.error.v1":
        return False, payload
    return True, payload


# The typed codes a refusal can carry, so an MCP prose error and a REST JSON
# error can be compared as the same refusal. Matched on the message because
# the MCP side renders `tool error (-N): <the REST message>` and the message
# is the part both surfaces share verbatim.
_CODE_HINTS = [
    ("excludes every requested band", "invalid_argument"),
    ("not_found", "not_found"),
    ("invalid", "invalid_argument"),
    ("unsupported", "unsupported"),
    ("rate", "rate_limited"),
]


def _rest_code_for(text):
    low = text.lower()
    for needle, code in _CODE_HINTS:
        if needle in low:
            return code
    return "tool_error"


def facts_of(payload):
    """Every (band, value) the payload asserts, sorted.

    The comparison unit. Envelopes differ legitimately; the facts are the
    answer, and this is the level sgozfgkr's first cut got wrong and fixed.
    """
    out = []

    def walk(v):
        if isinstance(v, dict):
            band = v.get("band")
            if band is not None and "value" in v:
                out.append((band, v["value"]))
            for k, sub in v.items():
                if k not in VOLATILE:
                    walk(sub)
        elif isinstance(v, list):
            for sub in v:
                walk(sub)

    walk(payload)
    return sorted(out, key=lambda x: (str(x[0]), str(x[1])))


def shape_of(payload):
    """Top-level non-volatile keys. For tools with no facts to compare."""
    if not isinstance(payload, dict):
        return type(payload).__name__
    return sorted(k for k in payload if k not in VOLATILE)


def close_enough(a, b, tolerance):
    if a == b:
        return True
    if not tolerance:
        return False
    if isinstance(a, (int, float)) and isinstance(b, (int, float)):
        return abs(a - b) <= EPSILON
    return False


def compare(case, mcp, rest):
    """(category, detail) for one case."""
    (mcp_ok, mcp_body), (rest_ok, rest_body) = mcp, rest

    if not mcp_ok and not rest_ok:
        mc, rc = mcp_body.get("code"), rest_body.get("code")
        if mc == rc:
            return "BOTH_ERR", f"both refused: {mc}"
        return "ONLY_ERR", f"refused differently: mcp={mc} rest={rc}"
    if mcp_ok != rest_ok:
        which = "rest" if mcp_ok else "mcp"
        body = rest_body if not rest_ok else mcp_body
        return "ONLY_ERR", f"{which} refused ({body.get('code')}), the other answered"

    if case.shape_only:
        ms, rs = shape_of(mcp_body), shape_of(rest_body)
        if ms == rs:
            return "MATCH", f"same {len(ms)} top-level keys"
        only_mcp = sorted(set(ms) - set(rs))
        only_rest = sorted(set(rs) - set(ms))
        # The MCP wire budget slims a large catalog, so REST carrying MORE is
        # expected and is not a divergence. MCP carrying more is: it would
        # mean the two surfaces disagree about what the answer contains.
        if not only_mcp and only_rest:
            return "MCP_TRUNCATED", f"rest also carries {only_rest}, within the wire budget"
        return "DIVERGE", f"keys only in mcp={only_mcp} only in rest={only_rest}"

    mf, rf = facts_of(mcp_body), facts_of(rest_body)
    if not mf and not rf:
        # No facts either side: fall back to shape so the row still asserts
        # something rather than passing vacuously.
        ms, rs = shape_of(mcp_body), shape_of(rest_body)
        if ms == rs:
            return "MATCH", f"no facts; same {len(ms)} keys"
        return "DIVERGE", f"no facts and different keys: {ms} vs {rs}"
    if len(mf) != len(rf):
        return "DIVERGE", f"{len(mf)} facts on mcp, {len(rf)} on rest"

    worst = None
    exact = True
    for (mb, mv), (rb, rv) in zip(mf, rf):
        if mb != rb:
            return "DIVERGE", f"band mismatch: {mb} vs {rb}"
        if mv != rv:
            exact = False
            if not close_enough(mv, rv, case.tolerance):
                return "DIVERGE", f"{mb}: {mv} vs {rv}"
            worst = f"{mb}: {mv} vs {rv}"
    if exact:
        return "MATCH", f"{len(mf)} facts identical"
    return "TOLERANCE", f"within {EPSILON}: {worst}"


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--origin", default=DEFAULT_ORIGIN)
    ap.add_argument("--json", action="store_true", help="machine-readable summary")
    ap.add_argument("--only", help="run one case by name")
    a = ap.parse_args()
    origin = a.origin.rstrip("/")

    cases = [c for c in CASES if not a.only or c.name == a.only]
    if not cases:
        print(f"no case named {a.only!r}", file=sys.stderr)
        return 2

    # Fail fast and loudly if the responder is not there at all, rather than
    # reporting fifteen symptoms of one cause.
    try:
        _get(f"{origin}/v1/grid_info", {"cell": CELL}, timeout=30)
    except Exception as e:
        print(f"parity: {origin} is not answering: {e}", file=sys.stderr)
        return 2

    rows = []
    for c in cases:
        cat, detail = compare(c, call_mcp(origin, c), call_rest(origin, c))
        rows.append({"case": c.name, "tool": c.tool, "path": c.path,
                     "category": cat, "detail": detail})

    bad = [r for r in rows if r["category"] in ("ONLY_ERR", "DIVERGE")]

    if a.json:
        print(json.dumps({"origin": origin, "rows": rows,
                          "failed": len(bad), "total": len(rows)}, indent=1))
    else:
        print(f"MCP/REST parity against {origin}\n")
        for r in rows:
            mark = "FAIL" if r["category"] in ("ONLY_ERR", "DIVERGE") else "ok  "
            print(f"  {mark} {r['category']:<10} {r['case']:<40} {r['detail']}")
        print(f"\n{len(rows) - len(bad)} of {len(rows)} cases have parity")
        if bad:
            print("\nA divergence means an agent's answer depends on which door it used.")
            for r in bad:
                print(f"  {r['case']}: {r['detail']}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
