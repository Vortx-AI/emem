#!/usr/bin/env python3
"""Check /v1/ask against the fixed question set, on a running node.

The question set is crates/emem-api-rest/src/testdata/ask_cases.json. Its
`expect` half is checked offline by cargo test; this script checks its `live`
half, which needs a node: what the answer says, in what form, from which band,
and what it must never say.

    python3 scripts/ask_check.py --base http://127.0.0.1:5098
    python3 scripts/ask_check.py --base http://127.0.0.1:5098 --only dated_snow

Cases marked `needs_upstream` depend on an upstream archive answering (a
Sentinel-2 scene, a DEM tile). They are reported apart, and only the others
decide the exit code, so a slow upstream does not read as a regression.

Run it against a local node, not against emem.dev: it sends one request per
case, in sequence, and a cold node fetches from upstream archives.
"""

import argparse
import json
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

CASES = Path(__file__).resolve().parent.parent / "crates/emem-api-rest/src/testdata/ask_cases.json"


def post(base, body, timeout):
    req = urllib.request.Request(
        base.rstrip("/") + "/v1/ask",
        data=json.dumps(body).encode(),
        headers={"content-type": "application/json"},
        method="POST",
    )
    t0 = time.monotonic()
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, time.monotonic() - t0, json.loads(r.read())
    except urllib.error.HTTPError as e:
        raw = e.read()
        try:
            return e.code, time.monotonic() - t0, json.loads(raw)
        except ValueError:
            return e.code, time.monotonic() - t0, {"_raw": raw[:300].decode("utf-8", "replace")}
    except Exception as e:  # a timeout or a refused connection is a result too
        return 0, time.monotonic() - t0, {"_error": f"{type(e).__name__}: {e}"}


def check(case, status, body):
    """Return the list of ways this answer misses the case's `live` block."""
    want = case["live"]
    miss = []
    if status != want.get("http_status", 200):
        miss.append(f"http {status}, want {want.get('http_status', 200)}")
        return miss
    if "error_code" in want and body.get("code") != want["error_code"]:
        miss.append(f"error code {body.get('code')!r}, want {want['error_code']!r}")
    routed = body.get("routed_to")
    if "routed_to" in want and routed != want["routed_to"]:
        miss.append(f"routed_to {routed!r}, want {want['routed_to']!r}")
    if "routed_to_in" in want and routed not in want["routed_to_in"]:
        miss.append(f"routed_to {routed!r} not in {want['routed_to_in']}")
    if "routed_to_not" in want and routed == want["routed_to_not"]:
        miss.append(f"routed_to {routed!r}")
    direct = body.get("direct_answer") or {}
    if "verdict_in" in want and direct.get("verdict") not in want["verdict_in"]:
        miss.append(f"verdict {direct.get('verdict')!r} not in {want['verdict_in']}")
    if "band_in" in want and direct.get("band") not in want["band_in"]:
        miss.append(f"answered from {direct.get('band')!r}, want one of {want['band_in']}")
    answer = (body.get("answer") or "").lower()
    for w in want.get("must_mention", []):
        if w.lower() not in answer:
            miss.append(f"answer lacks {w!r}")
    for w in want.get("must_not_mention", []):
        if w.lower() in answer:
            miss.append(f"answer says {w!r}")
    if want.get("no_live_perception") and (body.get("live_perception") or {}).get("answers_now"):
        miss.append("a camera answered a question about another time")
    inherited = (body.get("context") or {}).get("inherited") or []
    for k in want.get("context_inherits", []):
        if k not in inherited:
            miss.append(f"did not inherit {k!r} from the thread")
    return miss


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--base", default="http://127.0.0.1:5098")
    ap.add_argument("--only", action="append", help="case id; repeatable")
    ap.add_argument("--timeout", type=float, default=120.0)
    ap.add_argument("--out", help="write per-case results as JSON here")
    args = ap.parse_args()

    cases = json.loads(CASES.read_text())["cases"]
    by_id = {c["id"]: c for c in cases}
    contexts = {}
    rows = []
    for c in cases:
        if "live" not in c or (args.only and c["id"] not in args.only):
            continue
        body = {"q": c["q"]}
        if c.get("place"):
            body["place"] = c["place"]
        if c.get("follows"):
            prior = contexts.get(c["follows"])
            if prior is None and args.only:
                # Run the case it follows first, so the thread exists.
                p = by_id[c["follows"]]
                _, _, pb = post(args.base, {"q": p["q"], "place": p.get("place")}, args.timeout)
                prior = (pb.get("context") or {}).get("context_id")
            if prior:
                body["context_id"] = prior
        status, secs, out = post(args.base, body, args.timeout)
        if isinstance(out, dict) and (out.get("context") or {}).get("context_id"):
            contexts[c["id"]] = out["context"]["context_id"]
        miss = check(c, status, out if isinstance(out, dict) else {})
        upstream = bool(c["live"].get("needs_upstream"))
        rows.append({
            "id": c["id"],
            "pass": not miss,
            "needs_upstream": upstream,
            "seconds": round(secs, 1),
            "misses": miss,
            "verdict": (out.get("direct_answer") or {}).get("verdict") if isinstance(out, dict) else None,
            "answer": (out.get("answer") or "")[:400] if isinstance(out, dict) else "",
        })
        mark = "PASS" if not miss else "FAIL"
        tag = " (upstream)" if upstream else ""
        print(f"{mark}{tag:11} {c['id']:24} {secs:6.1f}s  {'; '.join(miss)}")

    core = [r for r in rows if not r["needs_upstream"]]
    up = [r for r in rows if r["needs_upstream"]]
    print()
    print(f"core:     {sum(r['pass'] for r in core)}/{len(core)} pass")
    print(f"upstream: {sum(r['pass'] for r in up)}/{len(up)} pass")
    if args.out:
        Path(args.out).write_text(json.dumps(rows, indent=2))
    return 0 if all(r["pass"] for r in core) else 1


if __name__ == "__main__":
    sys.exit(main())
