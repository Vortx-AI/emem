#!/usr/bin/env python3
"""Catch the failure where the server is up and storage is not.

On 2026-09-02 every path touching sled -- /v1/health, /v1/recall,
/v1/coverage_matrix -- returned a bare 504 at the 40s transport budget for at
least the length of one probe session, while /live answered in 43 ms and the
unit reported active. io wait was 0.0 and the disk, though nearly full, was not
the cause; a restart cleared it. The daemon agents had been getting tool errors
on emem_recall for a while and nothing was watching.

The point: A LIVENESS CHECK THAT DOES NOT TOUCH STORAGE CANNOT SEE THIS. /live
says so in its own note -- "no storage scan, safe to poll during a deploy" --
which is exactly right for a deploy probe and exactly wrong as the only probe.
Anything polling /live saw a healthy server the whole time.

So this asserts the SHAPE of the wedge rather than a single latency: /live fast
AND a storage read slow is the signature. Both slow is a restart or a cold
start, and is reported differently, because telling an operator "storage is
wedged" during a deploy would be a false alarm they learn to ignore.
"""
import argparse
import json
import sys
import time
import urllib.request

# A cell this responder has held warm all day; a point read, not a scan.
WARM = {"cell": "defi.zb493.xuqA.zcb5f", "bands": ["copdem30m.elevation_mean"]}


def timed(url: str, body=None, timeout=45):
    """(seconds, status, error). status None means no HTTP response."""
    t0 = time.monotonic()
    try:
        if body is None:
            req = urllib.request.Request(url)
        else:
            req = urllib.request.Request(
                url, method="POST", data=json.dumps(body).encode(),
                headers={"content-type": "application/json"})
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return time.monotonic() - t0, r.status, None
    except urllib.error.HTTPError as e:
        return time.monotonic() - t0, e.code, None
    except Exception as e:  # noqa: BLE001
        return time.monotonic() - t0, None, str(e)[:60]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--origin", default="https://emem.dev")
    ap.add_argument("--live-budget", type=float, default=5.0,
                    help="seconds /live may take before the node counts as down")
    ap.add_argument("--storage-budget", type=float, default=10.0,
                    help="seconds a warm point read may take before storage counts as wedged")
    a = ap.parse_args()

    live_s, live_code, live_err = timed(f"{a.origin}/live", timeout=15)
    read_s, read_code, read_err = timed(f"{a.origin}/v1/recall", WARM)

    print(f"  /live        {str(live_code or live_err):<24} {live_s:6.2f}s "
          f"(budget {a.live_budget}s)")
    print(f"  /v1/recall   {str(read_code or read_err):<24} {read_s:6.2f}s "
          f"(budget {a.storage_budget}s)")

    live_ok = live_code == 200 and live_s <= a.live_budget
    read_ok = read_code == 200 and read_s <= a.storage_budget

    if live_ok and read_ok:
        print("\n  The node is up and storage answers. Both checked; neither inferred "
              "from the other.")
        return 0

    if not live_ok and not read_ok:
        # Everything is slow. That is a restart, a cold start, or a dead node --
        # all of which look the same from here and none of which is the wedge.
        print("\n  UNDETERMINED: the whole node is unresponsive, not storage "
              "specifically.")
        print("  A deploy restart, a cold start and an outage are indistinguishable "
              "from outside, so this does not call it a wedge. Re-run once /live "
              "is answering.")
        return 2

    if live_ok and not read_ok:
        print("\n  STORAGE IS WEDGED. /live answers and a warm point read does not.")
        print("  This is the shape seen on 2026-09-02: the unit reports active, an "
              "uptime check polling /live stays green, and every path touching sled "
              "hangs to the transport budget.")
        print("  A restart cleared it: systemctl --user restart emem-server.")
        print("  Capture `journalctl --user -u emem-server` and the thread count "
              "BEFORE restarting, or the next one is diagnosed from scratch too.")
        return 1

    print("\n  /live is slow while storage answers, which is backwards and not a "
          "shape this check has seen before. Reported rather than classified.")
    return 1


if __name__ == "__main__":
    sys.exit(main())
