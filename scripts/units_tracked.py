#!/usr/bin/env python3
"""Every unit that keeps emem running must exist in the repository.

Why this exists
---------------
Eighteen systemd units run this responder: the server, the watchdog, the
channel bake, the lance compaction, the autonomous responder, the explain
sidecar, the tunnel. Four of them were in the repository. The other fourteen
existed only in ~/.config/systemd/user on one machine, so the operational
half of this system was not backed up, not reviewable, and not reproducible:
losing the box would have lost how it runs, while the code that runs sat
safely in git.

It surfaced through a smaller version of the same fault. The explain sidecar
answered as qwen2.5-7b, I edited the copy in this repository to make it answer
as gemma, restarted it, and nothing changed: the unit pointed at
/home/ubuntu/emem-local/explain_sidecar.py, an untracked twin that happened to
be identical until I edited one of them. An hour of confusion from a file the
repository could not see.

So this checks two things. Every unit on the box is committed here, and every
unit runs a file this repository contains. A service pointing outside the
repo is a piece of production nobody can review.

    python3 scripts/units_tracked.py

Exit: 0 all tracked, 1 something is untracked or points outside, 2 no systemd.
"""

from __future__ import annotations

import glob
import os
import re
import subprocess
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TRACKED = os.path.join(REPO, "deploy/systemd")
LIVE = os.path.expanduser("~/.config/systemd/user")

# Files a unit may legitimately run from outside this repository, with the
# reason. An entry here is a dependency on another project on the same box,
# not an oversight, and it has to be named to stay one.
EXTERNAL_OK = {
    "/home/ubuntu/emem-world/agents/ememagentd.py":
        "the world agent daemon, owned by the emem-world project on this box",
}


def main() -> int:
    if not os.path.isdir(LIVE):
        print("units_tracked: no user systemd directory here; nothing asserted.")
        return 2

    live = {os.path.basename(p) for p in glob.glob(os.path.join(LIVE, "emem-*.service"))
            + glob.glob(os.path.join(LIVE, "emem-*.timer"))}
    if not live:
        print("units_tracked: no emem units installed; nothing asserted.")
        return 2
    tracked = {os.path.basename(p) for p in glob.glob(os.path.join(TRACKED, "emem-*"))}

    problems = []
    for name in sorted(live - tracked):
        problems.append(f"{name} runs on this box and is in no repository. "
                        f"Copy it to deploy/systemd/ so losing the machine does not "
                        f"lose how the machine works.")

    for name in sorted(live & tracked):
        with open(os.path.join(LIVE, name), encoding="utf-8") as fh:
            unit = fh.read()
        with open(os.path.join(TRACKED, name), encoding="utf-8") as fh:
            repo_unit = fh.read()
        if unit.strip() != repo_unit.strip():
            problems.append(f"{name} differs from deploy/systemd/{name}. The copy that "
                            f"runs and the copy that is reviewed have drifted apart.")
        for m in re.finditer(r"^Exec\w*=.*?(/home/\S+\.(?:py|sh))", unit, re.M):
            path = m.group(1)
            if path.startswith(os.path.join(REPO, "")):
                continue
            if path in EXTERNAL_OK:
                continue
            problems.append(
                f"{name} runs {path}, which is outside this repository and not "
                f"declared in EXTERNAL_OK. Editing the repo copy of that file "
                f"changes nothing, which is exactly how the explain sidecar kept "
                f"answering as the wrong model after it was fixed."
            )

    print(f"  {len(live)} unit(s) installed, {len(live & tracked)} tracked here")
    if problems:
        print("\nunits_tracked: production that the repository cannot see.")
        for p in problems:
            print(f"  x {p}")
        return 1
    print("Every unit that runs is committed, and every unit runs a file in this repo.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
