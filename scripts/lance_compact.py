#!/usr/bin/env python3
"""Prune old Lance dataset versions, often enough that it stays cheap.

Why this exists
---------------
The memory text index appends a version per write and nothing pruned them. By
2026-08-17 it held 38,950 versions: 41 GB of manifests over 204 MB of data.
The cost was not only disk. Enumerating that many versions took 557 seconds,
and emem_memory_search timed out for every caller against a 32 s budget, so a
shipped tool was dead in production because of unswept bookkeeping.

The first attempt to clean it up looked hopeless: lance's own
cleanup_old_versions removed 526 manifests in fourteen minutes and then died
racing a concurrent write, which extrapolated to about fifteen hours with
writes stopped. That number was an artifact of the backlog, not a property of
the tool. Measured again after a rebuild, against 1,669 versions:

    1,669 -> 118 versions in 0.9 seconds, 35 MB reclaimed, server live

So the answer is frequency. Run hourly and the backlog never reaches the size
where cleaning it is expensive, and each pass is under a second against a
responder that is still writing.

The window is deliberately generous. Lance resolves a read against a version,
so anything recent must stay: an hour is far longer than any read here takes,
and the versions that matter for cost are the thousands behind it.

    python3 scripts/lance_compact.py            # prune, print what moved
    python3 scripts/lance_compact.py --check    # report only, change nothing

Exit: 0 fine, 1 a dataset is growing faster than this can hold, 2 lance missing.
"""

from __future__ import annotations

import argparse
import glob
import os
import sys
import time
from datetime import timedelta

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LANCE_DIR = os.path.join(REPO, "var/emem/lance")
KEEP = timedelta(hours=1)

# Above this, an hourly pass is no longer keeping up and something changed:
# either write volume grew or the timer stopped firing. Worth a failure rather
# than a quiet slide back to 41 GB.
ALARM_VERSIONS = 20_000


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true", help="report, change nothing")
    a = ap.parse_args()

    try:
        import lance
    except ImportError:
        print("lance_compact: pylance is not installed; nothing asserted.")
        return 2

    datasets = sorted(glob.glob(os.path.join(LANCE_DIR, "*.lance")))
    if not datasets:
        print(f"lance_compact: no datasets under {LANCE_DIR}")
        return 0

    alarms = []
    for path in datasets:
        name = os.path.basename(path)
        try:
            ds = lance.dataset(path)
            before = len(ds.versions())
        except Exception as e:  # a dataset mid-write, or not one at all
            print(f"  ??   {name}: {str(e)[:70]}")
            continue

        if a.check:
            print(f"  {name:<36} {before:>6} versions")
            if before > ALARM_VERSIONS:
                alarms.append(f"{name} holds {before} versions")
            continue

        t0 = time.time()
        try:
            ds.cleanup_old_versions(older_than=KEEP)
            after = len(lance.dataset(path).versions())
            print(f"  {name:<36} {before:>6} -> {after:<6} in {time.time()-t0:.1f}s")
            if after > ALARM_VERSIONS:
                alarms.append(
                    f"{name} still holds {after} versions after a pass; an hourly "
                    f"sweep is no longer keeping up with the write rate"
                )
        except Exception as e:
            # Racing a concurrent write is expected occasionally and is not a
            # failure: the next pass picks up where this one stopped. It only
            # matters if the backlog is also growing, which ALARM catches.
            print(f"  {name:<36} {before:>6} -> stopped early ({str(e)[:44]})")

    if alarms:
        print("\nlance_compact: not keeping up.")
        for x in alarms:
            print(f"  x {x}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
