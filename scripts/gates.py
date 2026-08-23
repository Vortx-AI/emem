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
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

# The one gate whose result governs whether the others' results mean anything.
TRUST = (
    ["python3", "scripts/shadowed_definitions.py"],
    "shadowed definitions",
    "decides whether every other gate's verdict is about the code that runs",
)

# Everything else. Order among these is not load-bearing; it is roughly
# cheapest-first so a fast failure arrives fast.
GATES = [
    (["python3", "scripts/doc_lint.py"], "doc lint",
     "prose rules over the documents"),
    (["python3", "scripts/design_tokens.py"], "design tokens",
     "one token file, one scale, dark mode on every page"),
    (["python3", "scripts/provenance_classes.py"], "provenance classes",
     "every class emitted or published is one we declare"),
    (["python3", "scripts/openapi_coverage.py"], "openapi coverage",
     "every routed path described or excluded with a reason"),
    (["python3", "scripts/sync_counts.py", "--check"], "counts",
     "every stated count matches the responder, and nothing goes unread"),
]


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
    if "--list" in sys.argv:
        print(f"  0. {TRUST[1]:22} {TRUST[2]}")
        print("     ^ if this fails, nothing below is run: its verdicts would be")
        print("       about code that is not the code running")
        for i, (_c, label, why) in enumerate(GATES, 1):
            print(f"  {i}. {label:22} {why}")
        return 0

    ok, line = run(TRUST[0], TRUST[1])
    print(f"  {'PASS' if ok else 'FAIL'}  {TRUST[1]:22} {line}")
    if not ok:
        print()
        print("  The trust check failed, so the remaining gates were NOT run.")
        print("  A gate script with a shadowed definition reports on code that")
        print("  is not running, and its PASS would mean nothing. Fix the")
        print("  shadowing, then run this again.")
        return 1

    failed = []
    for cmd, label, _why in GATES:
        ok, line = run(cmd, label)
        print(f"  {'PASS' if ok else 'FAIL'}  {label:22} {line}")
        if not ok:
            failed.append(label)

    print()
    if failed:
        print(f"  {len(failed)} gate(s) failed: {', '.join(failed)}")
        return 1
    print(f"  {len(GATES) + 1} gates passed, and the first one says so about the rest.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
