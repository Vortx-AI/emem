#!/usr/bin/env python3
"""Run the repository gates in an order that reflects what they depend on.

    scripts/gates.py           run them all
    scripts/gates.py --list    print the order and the reason for it

Why an order at all
-------------------
There were five gates and no sequence, which reads as five peers. They are not.
`shadowed_definitions.py` asks whether any gate script defines a name twice --
Python binds the LAST definition, so a replacement spliced above an untouched
original runs the original. If that is true of `provenance_classes.py`, then
`provenance_classes.py`'s verdict is about code that is not running, and it
carries the same weight whether it says PASS or FAIL: none.

So it is not a gate among gates. It is the check that decides whether the others'
verdicts mean anything, and it has to come first. In CI it was running LAST, and
the upstream had theirs ninth of nine when they noticed the same thing about
their own suite. Both of us had put the trust check at the end of the trusted
things.

What happens when the trust check fails
---------------------------------------
The rest do not run, and the exit says so. Printing four PASS lines under a
warning banner invites somebody to read the PASSes, and they would not mean
"these properties hold" -- they would mean "four scripts we cannot vouch for
said something". A refusal that names the reason is more use than a result that
has to be discounted.

What this does NOT claim
------------------------
The order below is one dependency, not a full graph. `sync_counts.py --check`
reaches the live responder and the others do not, `openapi_coverage.py` parses
the same file two ways -- those are properties worth knowing but they do not
order the suite. If a second real dependency turns up, it goes here with its
reason rather than into the sequence silently.
"""
import re
import shlex
import pathlib
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

# The one gate whose result governs whether the others' results mean anything.
TRUST_SCRIPT = "scripts/shadowed_definitions.py"
TRUST_WHY = "decides whether every other gate's verdict is about the code that runs"

CI_WORKFLOW = REPO / ".github" / "workflows" / "ci.yml"

# One line per gate, and it is NOT written here.
#
# The first version of this file listed the gates by hand and covered six of
# the thirteen CI runs. It printed "6 gates passed" while CI was red on
# spacing_scale.py, which was not in the list -- a runner whose green was an
# assurance it had no basis for, and a coverage claim made by omission in the
# tool built to stop coverage claims being made by omission.
#
# So the list is READ FROM THE WORKFLOW. There is no second copy to drift, and
# a gate added to CI is picked up here without anyone remembering. That is the
# same rule as reading KNOWN_PROVENANCE out of substrates.rs rather than
# restating it: a checker carrying its own copy of the thing it checks is the
# defect wearing a lab coat.
def ci_gate_steps() -> list[tuple[list[str], str]]:
    """Every `run: python3 scripts/*.py ...` step in the CI workflow, in order."""
    if not CI_WORKFLOW.exists():
        raise FileNotFoundError(f"{CI_WORKFLOW} is missing, so the gate list "
                                f"cannot be read and this runner has nothing to run")
    text = CI_WORKFLOW.read_text(encoding="utf-8")
    out: list[tuple[list[str], str]] = []
    seen: set[str] = set()
    for raw_line in text.split("\n"):
        line = raw_line.strip()
        # `run: python3 scripts/x.py` AND the same command inside a `run: |`
        # block. The first version matched only the former and missed
        # sync_counts.py, which lives in a multi-line block -- the same
        # coverage-by-omission this file exists to stop, one level in.
        if line.startswith("run:"):
            line = line[4:].strip()
        if line.startswith("#") or not line.startswith("python3 scripts/"):
            continue
        # An `echo` suggesting a command is not a command. Skipping these is
        # what keeps the line `echo "::error::... Run: python3
        # scripts/sync_counts.py --write"` from being executed.
        if "echo" in raw_line:
            continue
        # AND NEVER RUN A WRITER. A checker that mutates the tree it is
        # checking cannot be run to find out whether the tree is clean.
        if "--write" in line:
            continue
        raw = re.sub(r"\$\{[A-Z_]+:-([^}]*)\}", r"\1", line)
        raw = re.sub(r"\$\{?[A-Z_]+\}?", "", raw)
        try:
            cmd = shlex.split(raw)
        except ValueError:
            continue
        script = next((c for c in cmd if c.startswith("scripts/")), "")
        if not script or script.endswith("gates.py") or script in seen:
            continue
        seen.add(script)
        out.append((cmd, pathlib.Path(script).stem.replace("_", " ")))
    return out


def run(cmd: list[str], label: str) -> tuple[bool, str]:
    try:
        p = subprocess.run(cmd, cwd=REPO, capture_output=True, text=True, timeout=900)
    except subprocess.TimeoutExpired:
        return False, f"{label}: timed out"
    except OSError as e:
        # A gate that cannot start is not a gate that passed.
        return False, f"{label}: could not run ({e})"
    tail = [ln for ln in p.stdout.strip().split("\n") if ln.strip()]
    return p.returncode == 0, (tail[-1] if tail else f"{label}: no output")


def main() -> int:
    try:
        steps = ci_gate_steps()
    except FileNotFoundError as e:
        print(f"  {e}")
        return 1
    if not steps:
        # Matching nothing is not passing: the workflow moved or its shape did.
        print(f"  MATCHED NOTHING: no gate steps found in {CI_WORKFLOW}.")
        return 1

    trust = [(c, l) for c, l in steps if TRUST_SCRIPT in c]
    rest = [(c, l) for c, l in steps if TRUST_SCRIPT not in c]
    if not trust:
        print(f"  {TRUST_SCRIPT} is not in the workflow, so nothing establishes")
        print("  whether the other gates' verdicts are about the code that runs.")
        return 1

    if "--list" in sys.argv:
        print(f"  0. {trust[0][1]:24} {TRUST_WHY}")
        print("     ^ if this fails, nothing below runs: their verdicts would be")
        print("       about code that is not the code running")
        for i, (cmd, label) in enumerate(rest, 1):
            print(f"  {i}. {label:24} {' '.join(cmd[1:])}")
        print(f"\n  read from {CI_WORKFLOW.relative_to(REPO)}, never restated here")
        return 0

    ok, line = run(trust[0][0], trust[0][1])
    print(f"  {'PASS' if ok else 'FAIL'}  {trust[0][1]:24} {line}")
    if not ok:
        print()
        print("  The trust check failed, so the remaining gates were NOT run.")
        print("  A gate script with a shadowed definition reports on code that")
        print("  is not running, and its PASS would mean nothing. Fix the")
        print("  shadowing, then run this again.")
        return 1

    failed = []
    for cmd, label in rest:
        ok, line = run(cmd, label)
        print(f"  {'PASS' if ok else 'FAIL'}  {label:24} {line}")
        if not ok:
            failed.append(label)

    print()
    if failed:
        print(f"  {len(failed)} of {len(rest) + 1} gate(s) failed: {', '.join(failed)}")
        return 1
    print(f"  {len(rest) + 1} gates passed -- every one CI runs -- and the first")
    print("  one says so about the rest.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
