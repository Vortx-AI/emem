#!/usr/bin/env python3
"""Wait for the responder before asking twenty gates what it says.

    scripts/await_responder.py [--origin https://emem.dev] [--wait 180]

Why this exists
---------------
Half this suite reads the live responder. Deploys restart it, and during the
restart the port refuses connections: longer than any single gate's patience,
and every one of those gates then reports a finding about the thing it checks
rather than about the service being down.

How long a restart actually takes is a measured number, and it has moved. A
warm restart is about ninety seconds. A COLD one is not: `storage_open`
measured 971,233 / 1,264,857 / 1,644,625 ms on 2026-09-14, because the open
page-faults a multi-gigabyte sled heap off the volume. No wait that fits inside
a CI job covers that, so this gate does not pretend to. It draws the line
instead: a timeout here says the responder was down for the whole wait, which
is an outage to go and look at, and explicitly NOT a finding from the gates
behind it. Re-derive the boot cost with
`journalctl -u emem-server | grep 'boot step step="storage_open"'`.
One run of this repository's history has `route_truth` failing on
`/.well-known/emem-guard.json` while three curls seconds later returned 200 in
28 ms.

Per-gate patience cannot fix that without making every gate slow when the
service is genuinely gone. A single wait at the top of the job can: one place
pays the ninety seconds, once, and the gates behind it run against a responder
that is up.

`/health` and `/metrics` are deliberately registered OUTSIDE the concurrency
limiter in the router, so they answer even when the heavy-request pool is
saturated. That is what makes them the right thing to poll: a 200 here means
the process is serving, not that it happens to be idle.

What it does NOT do
-------------------
Pass when the responder never answers. A suite that ran against nothing and
reported clean is the failure this whole file exists to avoid, so a timeout is
an error that names the wait, and the gates behind it do not run.
"""
import argparse
import time
import urllib.error
import urllib.request


def probe(url: str, timeout: float = 10.0) -> int:
    try:
        with urllib.request.urlopen(url, timeout=timeout) as r:
            return r.status
    except urllib.error.HTTPError as e:
        # An HTTP status is an answer: the process is serving.
        return e.code
    except Exception:
        return 0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--origin", default="https://emem.dev")
    ap.add_argument("--wait", type=float, default=300.0,
                    help="seconds to wait (a warm restart is about 90; a cold "
                         "one exceeds any CI wait, see the module docstring)")
    args = ap.parse_args()

    url = f"{args.origin.rstrip('/')}/health"
    started = time.monotonic()
    attempt = 0
    while True:
        attempt += 1
        code = probe(url)
        waited = time.monotonic() - started
        if code and code < 500:
            if attempt == 1:
                print(f"  {args.origin} is answering ({code}).")
            else:
                print(f"  {args.origin} answered {code} after {waited:.0f}s "
                      f"and {attempt} probes.")
            return 0
        if waited >= args.wait:
            print(f"  {args.origin} did not answer /health within {args.wait:.0f}s "
                  f"({attempt} probes, last result {code or 'no connection'}).")
            print("  Not running the checks behind this: a suite that ran against")
            print("  nothing and reported clean is worse than one that did not run.")
            print("  Read this as an OUTAGE, not as a finding about any gate behind")
            print("  it. A cold storage_open takes longer than this whole wait, so a")
            print("  deploy in flight reaches CI as exactly this line.")
            return 1
        time.sleep(min(5.0, max(1.0, args.wait / 30)))


if __name__ == "__main__":
    raise SystemExit(main())
