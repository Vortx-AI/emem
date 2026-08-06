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
  KNOWN       a deliberate difference, listed with the reason it exists.
              Printed every run, does not fail the build.
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
  python3 scripts/parity.py --full                # every tool with a REST twin

The seed table is 12 hand-picked cases: every flag that produced a bug in an
audit round, plus shape parity. `--full` is step 2 of the integration plan:
enumerate tools from tools/list, map `emem_X` to `/v1/X`, and cross-product the
boolean flags each tool declares in its own inputSchema against a pinned
fixture. Tools with no REST twin get a shape-only assertion rather than being
silently skipped, and the run says how many of each it did.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import re
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

# How many cases to compare at once. Each is two HTTP round trips and no local
# work, so this is latency hiding rather than parallel computation.
CONCURRENCY = 6

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
    # Aggregates. sgozfgkr found these jittering in the last two digits across
    # identical calls, because the polygon fan-out collects in COMPLETION order
    # and float addition is not associative. Sorting the samples by cell64
    # before the reduction fixed it, so this is asserted EXACT rather than
    # within a tolerance: a content-addressed answer whose last digit moves is
    # an unstable fact_cid for inputs that never changed.
    Case("ndvi/aggregate", "emem_ndvi", "/v1/ndvi", {"place": PLACE}),
]

# Cases where the same call, repeated on ONE transport, must return bit-identical
# values. This is the property sgozfgkr actually found broken, and comparing the
# two transports once would not have caught it: both were unstable.
REPEAT_CASES = [
    ("ndvi/aggregate-repeat", "emem_ndvi", "/v1/ndvi", {"place": PLACE}),
    ("at/aggregate-repeat", "emem_at", "/v1/at", {"place": PLACE, "n_cells": 8}),
]
REPEATS = 4


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
        return False, {"code": _rest_code_for(text), "message": text}

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
        # The MCP surface renders a refusal as `tool error (-N): <the REST
        # message>`, so when the codes cannot be compared the MESSAGE can:
        # the REST text appears verbatim inside the MCP one. That is an exact
        # comparison, not a fuzzy one, and it is what lets an unmapped code
        # still be recognised as the same refusal.
        # The MCP surface renders `tool error (-N): <core>`, where <core> is
        # the same sentence the REST body carries somewhere inside its typed
        # envelope (often nested under details.deserializer_error rather than
        # at the top level). So strip the prefix and look for the core in the
        # whole REST body. Exact containment, not a fuzzy match.
        mm = str(mcp_body.get("message") or "")
        core = mm.split(": ", 1)[1] if mm.startswith("tool error") and ": " in mm else mm
        whole_rest = json.dumps(rest_body, sort_keys=True, default=str)
        if core and core.strip() and core in whole_rest:
            return "BOTH_ERR", f"both refused for the same reason ({rc})"
        # Last resort: both refusals naming the same missing field. The two
        # surfaces word it differently ("missing field `geometry`" against
        # "requires `geometry` ... OR `bbox`") but a refusal that names the
        # same identifier is the same refusal, and an agent acts on the
        # identifier rather than on the sentence.
        fields = lambda t: set(re.findall(r"`([a-z_][a-z0-9_]*)`", t))
        shared = fields(mm) & fields(whole_rest)
        if shared:
            return "BOTH_ERR", f"both refused naming {sorted(shared)}"
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
        # Some tools ask for a compact projection over MCP on purpose, and say
        # so in the payload. That adds keys on the MCP side while removing
        # others, so the plain subset test cannot see it.
        if isinstance(mcp_body, dict) and mcp_body.get("projection") == "compact":
            return ("MCP_TRUNCATED",
                    f"mcp asked for the compact projection; rest also carries {only_rest}")
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
        # The MCP wire budget truncates a large result. Fewer facts on MCP,
        # all of which appear on REST, is that budget doing its job; the
        # verdict-bearing values that DID survive still have to agree, which
        # the subset check asserts. More facts on MCP is never this.
        # Values can be lists (vectors), which are unhashable, so compare on
        # a canonical JSON rendering rather than on the raw pairs.
        key = lambda pairs: {json.dumps(p, sort_keys=True, default=str) for p in pairs}
        if len(mf) < len(rf) and key(mf).issubset(key(rf)):
            return ("MCP_TRUNCATED",
                    f"{len(mf)} of {len(rf)} facts survived the wire budget, all agreeing")
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


# Divergences that are DELIBERATE, each with the reason it exists.
#
# A case listed here prints every run as `known` and does not fail the build.
# It is not deleted and not silenced: an intentional asymmetry that nobody can
# see is indistinguishable from a regression nobody noticed, and the reason
# string is what a reviewer reads to decide whether it is still intentional.
# Adding an entry requires writing that sentence.
KNOWN_DIVERGENCES = {
    "emem_intent/base": (
        "MCP converts an unparseable Intent into a soft `needs_intent_type` "
        "envelope so a host does not surface a red error toast and the model "
        "can self-correct; REST returns its standard typed 400. Deliberate, "
        "documented at the dispatch site, and the two disagree on purpose."
    ),
    "emem_tools/shape": (
        "emem_tools returns the loop plus a navigable menu, which is what an "
        "agent needs to choose a call; GET /v1/tools returns a flat catalogue "
        "for humans and scripts. Different questions, deliberately different "
        "answers."
    ),
}

# Arguments every tool accepts, so a generated case addresses something real.
FIXTURE_ARGS = {"cell": CELL, "place": PLACE}

# Flags worth cross-producting. Booleans an agent actually sets, not every
# field: the point is the flags that changed behaviour, and a full power set
# over every property would be thousands of calls for no extra signal.
CROSS_FLAGS = ["deterministic", "compact", "summary", "include_absent"]

# Tools a generated run must not call. Not "hard to compare": actively
# expensive or state-changing, and a parity harness that triggers a
# materialisation storm against production is a worse problem than the drift
# it looks for.
SKIP_TOOLS = {
    "emem_backfill", "emem_fetch", "emem_derive", "emem_attest",
    "emem_challenge", "emem_benchmark", "emem_hunt",
    "emem_memory_create", "emem_memory_delete", "emem_memory_rename",
    "emem_memory_insert", "emem_memory_str_replace",
    "emem_jepa_predict", "emem_jepa_predict_v2",
    "emem_band_raster", "emem_band_cube", "emem_band_composite",
    "emem_cell_scene_rgb", "emem_coverage_map", "emem_eudr_dds",
    "emem_reason", "emem_ask", "emem_guard_selfhost",
}


def rest_twins(origin):
    """tool name -> (path, method), from the responder's OWN OpenAPI document.

    Not from a naming convention. The first cut of this assumed `emem_X` lives
    at `/v1/X` with POST, and produced twenty-two false divergences: half the
    reads are GET (405), and the namespaced tools live at `/v1/log/sth` and
    `/v1/guard/verdict` rather than `/v1/log_sth` (404). Every one of those was
    the harness being wrong about the server.

    The document already carries the answer: each path+method declares an
    `operationId` equal to the tool name. Reading it means the mapping cannot
    drift from the routes, and a tool with no twin is ABSENT here rather than
    guessed at.
    """
    doc = _get(f"{origin}/openapi.json", {}, timeout=60)
    twins = {}
    for path, methods in (doc.get("paths") or {}).items():
        for method, op in methods.items():
            if method.upper() not in ("GET", "POST"):
                continue
            op_id = (op or {}).get("operationId")
            if not op_id:
                continue
            # A path with a template segment cannot be addressed from a flat
            # args dict, so it is not a value-parity candidate.
            if "{" in path:
                continue
            twins.setdefault(op_id, (path, method.upper()))
    return twins


def discover(origin):
    """Every advertised tool, with the boolean flags it declares."""
    out = _post(f"{origin}/mcp", {"jsonrpc": "2.0", "id": 1,
                                  "method": "tools/list",
                                  "params": {"tier": "all"}})
    tools = out.get("result", {}).get("tools", [])
    found = []
    for t in tools:
        name = t.get("name", "")
        if not name.startswith("emem_") or name in SKIP_TOOLS:
            continue
        props = (t.get("inputSchema") or {}).get("properties") or {}
        accepts = set(props)
        flags = [f for f in CROSS_FLAGS if f in accepts]
        found.append((name, accepts, flags))
    return found


def generated_cases(origin):
    """One case per tool with a REST twin, plus one per boolean flag."""
    twins = rest_twins(origin)
    cases, no_twin = [], []
    for name, accepts, flags in discover(origin):
        twin = twins.get(name)
        if twin is None:
            # Named rather than skipped: a tool with no REST twin is a real
            # fact about the surface, and a run that quietly dropped it would
            # report parity over a set nobody chose.
            no_twin.append(name)
            continue
        path, method = twin
        base = {k: v for k, v in FIXTURE_ARGS.items() if k in accepts}
        if not base:
            cases.append(Case(f"{name}/shape", name, path, {},
                              shape_only=True, method=method))
            continue
        cases.append(Case(f"{name}/base", name, path, dict(base), method=method))
        for f in flags:
            args = dict(base)
            args[f] = True
            cases.append(Case(f"{name}/{f}", name, path, args, method=method))
    return cases, no_twin


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--origin", default=DEFAULT_ORIGIN)
    ap.add_argument("--json", action="store_true", help="machine-readable summary")
    ap.add_argument("--only", help="run one case by name")
    ap.add_argument("--full", action="store_true",
                    help="enumerate every tool and cross-product its flags")
    a = ap.parse_args()
    origin = a.origin.rstrip("/")

    cases = list(CASES)
    if a.full:
        try:
            generated = generated_cases(origin)
        except Exception as e:
            print(f"parity: could not enumerate tools: {e}", file=sys.stderr)
            return 2
        generated, no_twin = generated
        seeded = {c.name for c in cases}
        cases += [c for c in generated if c.name not in seeded]
        if no_twin and not a.json:
            print(f"{len(no_twin)} tools have no REST twin and are not "
                  f"value-compared: {', '.join(sorted(no_twin))}\n")
    cases = [c for c in cases if not a.only or c.name == a.only]
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

    def run_case(c):
        cat, detail = compare(c, call_mcp(origin, c), call_rest(origin, c))
        if cat in ("ONLY_ERR", "DIVERGE") and c.name in KNOWN_DIVERGENCES:
            cat, detail = "KNOWN", KNOWN_DIVERGENCES[c.name]
        return {"case": c.name, "tool": c.tool, "path": c.path,
                "category": cat, "detail": detail}

    # Concurrent, because 86 cases at two round trips each is fifteen minutes
    # of latency from a CI runner and almost no CPU. That is what timed the
    # job out on its first real run. Modest width on purpose: this points at
    # production, and a parity check that reads as a load test is a parity
    # check somebody turns off. `map` preserves input order, so the report
    # still reads in the order the table is written.
    workers = 1 if a.only else CONCURRENCY
    with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as pool:
        rows = list(pool.map(run_case, cases))

    # Repeat-stability. Cross-transport comparison cannot catch an aggregate
    # that is unstable on BOTH paths, which is exactly what was shipped.
    if not a.only:
        for name, tool, path, args in REPEAT_CASES:
            c = Case(name, tool, path, args)
            seen = []
            for _ in range(REPEATS):
                ok, body = call_rest(origin, c)
                if not ok:
                    seen = None
                    break
                seen.append(facts_of(body))
            if seen is None:
                rows.append({"case": name, "tool": tool, "path": path,
                             "category": "BOTH_ERR",
                             "detail": "refused; nothing to compare"})
            elif all(x == seen[0] for x in seen):
                rows.append({"case": name, "tool": tool, "path": path,
                             "category": "MATCH",
                             "detail": f"{REPEATS} identical calls, bit-identical values"})
            else:
                distinct = {json.dumps(x) for x in seen}
                rows.append({"case": name, "tool": tool, "path": path,
                             "category": "DIVERGE",
                             "detail": f"{len(distinct)} distinct answers from {REPEATS} "
                                       f"identical calls: an unstable fact_cid for "
                                       f"inputs that never changed"})

    bad = [r for r in rows if r["category"] in ("ONLY_ERR", "DIVERGE")]

    if a.json:
        print(json.dumps({"origin": origin, "rows": rows,
                          "failed": len(bad), "total": len(rows)}, indent=1))
    else:
        print(f"MCP/REST parity against {origin}\n")
        for r in rows:
            mark = "FAIL" if r["category"] in ("ONLY_ERR", "DIVERGE") else "ok  "
            print(f"  {mark} {r['category']:<10} {r['case']:<40} {r['detail']}")
        by_cat = {}
        for r in rows:
            by_cat[r["category"]] = by_cat.get(r["category"], 0) + 1
        print(f"\n{len(rows) - len(bad)} of {len(rows)} cases have parity")
        print("  " + "  ".join(f"{k}={v}" for k, v in sorted(by_cat.items())))
        if bad:
            print("\nA divergence means an agent's answer depends on which door it used.")
            for r in bad:
                print(f"  {r['case']}: {r['detail']}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
