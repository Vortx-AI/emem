#!/usr/bin/env python3
"""Run the gates CI runs, before pushing, reading CI's own list.

I spent a day discovering CI failures after pushing, and the reason was not
carelessness about any one gate: I had a local sweep of thirteen scripts and CI
runs a different twenty. Fourteen of CI's gates had never been run on this
machine. A hand-maintained list of "the checks" is a second copy of the CI
config, and the two drift the moment either is edited.

So this does not carry a list. It PARSES .github/workflows/ci.yml, takes every
`run:` step that invokes `python3 scripts/...`, and runs those. Add a gate to CI
and it appears here with no edit; delete one and it disappears. The two cannot
disagree, because there is only one of them.

WHAT IT DOES NOT COVER, stated because a green run here is not a green CI:
everything that is not a python script -- cargo test, cargo clippy, cargo audit,
the docker build, the node and pytest SDK suites. Those are the majority of CI
by count and by minutes. This answers "did I leave a gate red", not "will CI
pass".
"""
import argparse
import pathlib
import re
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parent.parent
CI = REPO / ".github/workflows/ci.yml"


def ci_steps() -> tuple[list[str], list[str]]:
    """(python gates CI runs, everything else it runs).

    Reads BOTH `run: cmd` and `run: |` blocks. The first version read only
    single-line steps and silently missed ten gates -- discovery_test, parity,
    mcp_host_compat, sync_counts, route_truth, state_claims, live_fields,
    output_schema_conformance, spec_verifier_check, gen_decision_layer -- every
    one of them in a `run: |` block. A preflight that misses a third of CI while
    printing a confident count is worse than no preflight, and it was written to
    end exactly that failure. Found by parsing the same file twice, two ways,
    and comparing the counts.
    """
    lines = CI.read_text(encoding="utf-8").splitlines()
    py, other, i = [], [], 0
    while i < len(lines):
        m = re.match(r"^(\s*)run:\s*(.*)$", lines[i])
        if not m:
            i += 1
            continue
        indent, rest = m.group(1), m.group(2).strip()
        if rest in ("|", ">", "|-", ">-"):
            j = i + 1
            while j < len(lines) and (
                not lines[j].strip()
                or len(lines[j]) - len(lines[j].lstrip()) > len(indent)
            ):
                cmd = lines[j].strip()
                if cmd.startswith("python3 scripts/"):
                    py.append(cmd)
                elif cmd:
                    other.append(cmd)
                j += 1
            i = j
            continue
        (py if rest.startswith("python3 scripts/") else other).append(rest)
        i += 1
    # de-duplicate, keep CI's order
    seen, uniq = set(), []
    for r in py:
        if r not in seen:
            seen.add(r)
            uniq.append(r)
    return uniq, other


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--origin", default="https://emem.dev")
    ap.add_argument("--timeout", type=int, default=300,
                    help="per-gate seconds; example_check needs far more")
    ap.add_argument("--skip", default="example_check,await_responder,scanner_surface",
                    help="comma-separated script names to skip, REPORTED not hidden")
    a = ap.parse_args()

    py, other = ci_steps()
    if not py:
        print("preflight: VACUOUS -- parsed no python gate out of ci.yml.")
        print("  CI runs many. Zero means the workflow moved and this is reading")
        print("  nothing, which is the state it exists to prevent. Not a pass.")
        return 1

    skip = {s.strip() for s in a.skip.split(",") if s.strip()}
    failed, ran, skipped = [], 0, []
    for cmd in py:
        name = cmd.split("/", 1)[1].split()[0].removesuffix(".py")
        if name in skip:
            skipped.append(name)
            continue
        resolved = cmd.replace("${EMEM_ORIGIN:-https://emem.dev}", a.origin)
        try:
            p = subprocess.run(resolved, shell=True, cwd=REPO,
                               capture_output=True, text=True, timeout=a.timeout)
            rc = p.returncode
            why = ""
        except subprocess.TimeoutExpired:
            rc, why = 124, f" (timed out after {a.timeout}s)"
        ran += 1
        if rc:
            failed.append((name, rc, why))
        print(f"  {'ok  ' if not rc else 'FAIL'}  {name}{why}")

    print(f"\n  {ran} of CI's {len(py)} python gate(s) run, {len(skipped)} skipped: "
          f"{', '.join(skipped) or 'none'}")
    print(f"  NOT COVERED: {len(other)} CI steps that are not python scripts "
          f"(cargo test, clippy, audit, docker, node, pytest).")
    print("  A green run here means no gate is red. It does not mean CI passes.")

    if failed:
        print("\npreflight: fix these before pushing.")
        for name, rc, why in failed:
            print(f"  x {name} exited {rc}{why}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
