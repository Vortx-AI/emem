#!/usr/bin/env python3
"""Re-derive the /v1/log/* timings the transparency code cites in its comments.

    scripts/manual/translog_timing.py
    scripts/manual/translog_timing.py --origin http://127.0.0.1:5051 --no-append

Those comments assert numbers -- "2.886s across a single append", "0.90s at
first=1,479,000" -- and a number in a comment is a claim that rots exactly like
a constant while getting commentary's level of scrutiny. This is the command
that checks them.

Three things it measures, each answering a specific claim:

  sth across an append   The defect was that ANY append rebuilt the whole tree,
                         so the cost tracks GROWTH, not elapsed time. Probing on
                         a timer mostly lands between arrivals and reports the
                         cheap case -- which is how it stayed hidden. So this
                         forces an append (a recall that materialises a fact)
                         and times the very next call, rather than waiting and
                         hoping. --no-append skips that and only samples.

  consistency vs `first` The prefix root used to be folded, making the route
                         linear in a parameter nobody varies. A flat line here
                         is the fix; a slope is the regression.

  inclusion              Shares the same snapshot. Uses a REAL leaf index, not
                         a made-up cid: a 400 comes back in milliseconds and
                         measures the rejection path, not the proof.
"""
import argparse, json, statistics, sys, urllib.request, time

def get(url, timeout=90):
    t0 = time.monotonic()
    with urllib.request.urlopen(url, timeout=timeout) as r:
        body = r.read()
    return time.monotonic() - t0, json.loads(body)

def post(url, payload, timeout=180):
    req = urllib.request.Request(
        url, data=json.dumps(payload).encode(),
        headers={"content-type": "application/json"})
    t0 = time.monotonic()
    with urllib.request.urlopen(req, timeout=timeout) as r:
        r.read()
    return time.monotonic() - t0

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--origin", default="https://emem.dev")
    ap.add_argument("--rounds", type=int, default=3)
    ap.add_argument("--no-append", action="store_true",
                    help="do not write; only sample the no-growth case")
    a = ap.parse_args()
    o = a.origin.rstrip("/")

    _, sth = get(f"{o}/v1/log/sth")
    n = sth["sth"]["tree_size"]
    print(f"tree_size {n:,}\n")

    print("sth, no growth between calls:")
    quiet = []
    for _ in range(a.rounds):
        t, s = get(f"{o}/v1/log/sth")
        quiet.append(t)
        print(f"  {t:7.3f}s  tree_size={s['sth']['tree_size']:,}")

    if not a.no_append:
        print("\nsth, spanning a forced append:")
        # Widely separated coordinates per round. Nearby points resolve to a
        # cell that is already materialised, the recall is served from cache,
        # and nothing is appended -- which silently re-measures the quiet case.
        # Two of three rounds did exactly that at 0.37 degrees apart. The run
        # SAYS when it happens rather than averaging it in, because a probe
        # that reproduces the wrong case is a control that cannot fail.
        spread = [(-3.4, -62.2), (34.8, 138.6), (-25.3, 131.0),
                  (64.1, -21.9), (-33.9, 18.4), (55.7, 37.6)]
        for i in range(a.rounds):
            lat, lng = spread[i % len(spread)]
            post(f"{o}/v1/recall", {"lat": lat, "lng": lng, "bands": ["elevation"]})
            t, s = get(f"{o}/v1/log/sth")
            grew = s["sth"]["tree_size"] - n
            n = s["sth"]["tree_size"]
            print(f"  {t:7.3f}s  +{grew} leaves"
                  + ("   (nothing appended; not the case under test)" if grew == 0 else ""))

    print("\nconsistency by `first` (flat is the fix, a slope is the regression):")
    for first in (2, 1000, n // 2, n - 1000):
        if first < 1:
            continue
        t, _ = get(f"{o}/v1/log/consistency?first={first}")
        print(f"  first={first:<10,} {t:7.3f}s")

    print("\ninclusion at a real leaf index:")
    for m in (5, n // 2, n - 1):
        t, d = get(f"{o}/v1/log/inclusion?leaf_index={m}")
        print(f"  leaf_index={m:<10,} {t:7.3f}s  path={len(d['audit_path_b32'])} nodes")

    print(f"\nquiet-call median {statistics.median(quiet):.3f}s")
    return 0

if __name__ == "__main__":
    sys.exit(main())
