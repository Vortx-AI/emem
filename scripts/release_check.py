#!/usr/bin/env python3
"""One verdict on whether this tree is fit to carry a new version number.

Why this exists
---------------
A version bump is a claim: that the thing behind the number does what the
surfaces say it does. Today that claim was checked by running gates one at a
time and reading the results, which is how three defects reached production
in a single afternoon while every unit test stayed green. Each was in the path
an autonomous agent walks, and each was found by EXECUTING a claim rather than
reading it.

So this runs everything that can contradict the claim, in one pass, and prints
one line at the end. It is deliberately boring: no new checks live here, it
only refuses to let a green summary stand in for a green run.

The rule it enforces about itself
---------------------------------
A gate that could not run is NOT a pass. Several checks here exit 2 when the
responder is unreachable, and that is reported as UNPROVEN rather than folded
into either column, because "we could not check" and "we checked and it was
fine" are different facts and only one of them justifies a release.

    python3 scripts/release_check.py            # everything
    python3 scripts/release_check.py --offline  # skip checks needing the network
    python3 scripts/release_check.py --quick    # skip the slow test suites

Exit codes: 0 fit to bump, 1 something failed, 2 something could not be
checked and the answer is therefore unknown.
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
import time

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# What a full debug rebuild of the workspace costs, measured rather than
# guessed: target/debug reached 18 GB across the four suites below.
BUILD_HEADROOM_GB = 24.0

# (label, argv, needs_network, slow)
# Ordered cheapest-first so a broken tree fails in seconds rather than minutes.
CHECKS = [
    ("prose convention", ["python3", "scripts/doc_lint.py"], False, False),
    ("image carries every compiled-in file",
     ["python3", "scripts/docker_context.py"], False, False),
    ("design tokens", ["python3", "scripts/design_tokens.py"], False, False),
    ("openapi describes every routed path",
     ["python3", "scripts/openapi_coverage.py"], False, False),
    ("counts match the responder", ["python3", "scripts/sync_counts.py", "--check"],
     True, False),
    ("every route the site shows answers", ["python3", "scripts/route_truth.py"],
     True, False),
    ("live figures are fetched, not typed", ["python3", "scripts/live_numbers.py"],
     True, False),
    # And the fetch has to find something. A renamed field leaves a tile
    # pending, which reads as a node that is down.
    ("the fields the pages read still exist", ["python3", "scripts/live_fields.py"],
     True, False),
    ("documents assert states we are in", ["python3", "scripts/state_claims.py"],
     True, False),
    ("the guide's first screen matches the registry",
     ["python3", "scripts/gen_decision_layer.py", "--check"], True, False),
    ("MCP and REST answer the same", ["python3", "scripts/parity.py"], True, False),
    ("MCP hosts can see and call us", ["python3", "scripts/mcp_host_compat.py"],
     True, False),
    ("declared output schemas hold",
     ["python3", "scripts/output_schema_conformance.py"], True, False),
    ("the spec's own verifier verifies us",
     ["python3", "scripts/spec_verifier_check.py"], True, False),
    # The one that matters most: an agent that never heard of emem, all the way
    # to a second agent resolving and verifying its citation.
    ("a cold agent can discover, call, verify, cite and hand off",
     ["python3", "scripts/discovery_test.py"], True, False),
    ("the browser verifier accepts genuine and refuses forgeries",
     ["node", "scripts/verify_core_test.cjs"], False, False),
    ("the SDK verifies receipts locally",
     ["python3", "-m", "pytest", "sdks/emem-py/tests/", "-q"], False, True),
    ("rust: core", ["cargo", "test", "-p", "emem-core", "--lib"], False, True),
    ("rust: storage", ["cargo", "test", "-p", "emem-storage", "--lib"], False, True),
    ("rust: mcp", ["cargo", "test", "-p", "emem-mcp", "--lib"], False, True),
    ("rust: api", ["cargo", "test", "-p", "emem-api-rest", "--lib"], False, True),
]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--offline", action="store_true",
                    help="skip checks that need the responder")
    ap.add_argument("--quick", action="store_true", help="skip the slow suites")
    a = ap.parse_args()

    env = dict(os.environ)
    env["PYTHONPATH"] = os.pathsep.join(
        [os.path.join(REPO, "sdks/emem-py/src"), env.get("PYTHONPATH", "")]
    )
    # node lives under ~/.local on this box and is not on PATH by default.
    local_node = os.path.expanduser("~/.local/node/bin")
    if os.path.isdir(local_node):
        env["PATH"] = local_node + os.pathsep + env["PATH"]

    # A check that can take production down is not a safe check.
    #
    # The four cargo suites rebuild target/debug to about 18 GB. On 2026-08-17
    # this box had 19 GB free, the rebuild consumed it, sled hit ENOSPC
    # mid-snapshot, and emem.dev crash-looped eighteen times with a corrupt
    # snapshot file. The release checker caused the outage it exists to
    # prevent shipping.
    #
    # So it now looks before it builds. Too little headroom is UNPROVEN, not a
    # pass and not a failure: nothing was found wrong, and nothing was checked
    # either, and running anyway would risk the live store to find out.
    free_gb = shutil.disk_usage(REPO).free / 1e9
    needs_build = any(
        argv[0] == "cargo" for _, argv, _, slow in CHECKS if not (a.quick and slow)
    )
    # Below the headroom, the rust suites are delegated to CI rather than run
    # here. That is not a concession, it is the better place for them: CI
    # builds on a clean machine that shares no volume with a live sled store,
    # and it already runs these four on every push. Rebuilding 18 GB locally
    # to re-verify what CI verified is what created the hazard.
    delegate_rust = needs_build and free_gb < BUILD_HEADROOM_GB
    if delegate_rust:
        CHECKS[:] = [c for c in CHECKS if c[1][0] != "cargo"]
        print(f"note: {free_gb:.1f} GB free, below the {BUILD_HEADROOM_GB:.0f} GB a "
              f"debug rebuild needs.")
        print(f"      The rust suites are delegated to CI, which runs them on a "
              f"machine that does not share a volume with the live store.")
        print(f"      Filling this volume is how emem.dev went down on "
              f"2026-08-17, mid-snapshot, while this checker was running.\n")

    passed, failed, unproven, skipped = [], [], [], []
    print("release check\n")
    for label, argv, needs_net, slow in CHECKS:
        if a.offline and needs_net:
            skipped.append(label)
            print(f"  ..   {label}  (skipped: --offline)")
            continue
        if a.quick and slow:
            skipped.append(label)
            print(f"  ..   {label}  (skipped: --quick)")
            continue
        if shutil.which(argv[0], path=env["PATH"]) is None:
            unproven.append((label, f"{argv[0]} is not installed"))
            print(f"  ??   {label}  ({argv[0]} not installed)")
            continue
        t0 = time.time()
        r = subprocess.run(argv, cwd=REPO, env=env,
                           capture_output=True, text=True, timeout=3600)
        dt = time.time() - t0
        if r.returncode == 0:
            passed.append(label)
            print(f"  ok   {label}  ({dt:.0f}s)")
        elif r.returncode == 2:
            # By convention here, 2 means the gate could not run.
            tail = (r.stderr or r.stdout).strip().splitlines()
            unproven.append((label, tail[-1] if tail else "exit 2"))
            print(f"  ??   {label}  could not run ({dt:.0f}s)")
        else:
            tail = (r.stdout or r.stderr).strip().splitlines()
            why = next((l for l in reversed(tail) if l.strip()), "")
            failed.append((label, why))
            print(f"  FAIL {label}  ({dt:.0f}s)")
            for line in tail[-6:]:
                print(f"         {line[:150]}")

    print(f"\n  {len(passed)} passed, {len(failed)} failed, "
          f"{len(unproven)} unproven, {len(skipped)} skipped")

    if failed:
        print("\nNot fit to bump. These contradict the claim a version number makes:")
        for label, why in failed:
            print(f"  x {label}\n      {why[:160]}")
        return 1
    if unproven:
        # The distinction this file exists to keep: could-not-check is not
        # checked-and-fine, and a release should not be signed on the first
        # while reading like the second.
        print("\nNot fit to bump YET. Nothing failed, but these could not be "
              "checked, so the claim is unproven rather than true:")
        for label, why in unproven:
            print(f"  ? {label}\n      {why[:160]}")
        return 2
    if skipped:
        print(f"\n{len(skipped)} check(s) were skipped by a flag. Run without "
              f"--offline/--quick before bumping.")
        return 2
    if delegate_rust:
        print("\nEvery surface checkable here passed. The rust suites were NOT "
              "run locally; confirm CI is green for this commit before bumping, "
              "because this run did not check them.")
        return 2
    print("\nFit to bump: every surface that could contradict the version "
          "number was executed, and none did.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
