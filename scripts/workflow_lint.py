#!/usr/bin/env python3
"""Validate the workflow files the way GitHub does, not the way PyYAML does.

Why this exists
---------------
On 2026-08-06 a `fail-fast: false` was added to a `strategy:` block that
already had one. `yaml.safe_load` accepted it silently, keeping the last of the
two, so the local check passed. GitHub rejects duplicate mapping keys, so the
workflow never parsed: every run for the next two hours was created with **zero
jobs**, named by file path instead of by its `name:`, and reported `failure`.

Nothing ran. Not the tests, not the gates, not the two live checks written that
same afternoon. And the signal read exactly like a normal red build, so the
obvious reaction was to look for a broken test rather than a broken file.

The lesson is narrow and worth keeping: **a validator more permissive than the
consumer is not a validator.** PyYAML's default loader is more permissive than
GitHub's parser in at least this one way, so this uses a loader that refuses
duplicates, and checks the handful of shapes that turn into a silent
startup failure rather than an error you can read.

Exit codes
----------
  0  every workflow parses the way GitHub will parse it
  1  one does not
"""

from __future__ import annotations

import pathlib
import sys

import yaml


class StrictLoader(yaml.SafeLoader):
    """SafeLoader that refuses duplicate mapping keys, as GitHub does."""


def _no_duplicates(loader, node, deep=False):
    mapping = {}
    for key_node, value_node in node.value:
        key = loader.construct_object(key_node, deep=deep)
        if key in mapping:
            mark = key_node.start_mark
            raise yaml.constructor.ConstructorError(
                None, None,
                f"duplicate key {key!r} (line {mark.line + 1}); GitHub rejects "
                f"the file and runs it with zero jobs",
                key_node.start_mark,
            )
        mapping[key] = loader.construct_object(value_node, deep=deep)
    return mapping


StrictLoader.add_constructor(
    yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, _no_duplicates
)


def check(path):
    """Returns a list of problems, empty when the file is fine."""
    problems = []
    raw = path.read_text(encoding="utf-8")
    try:
        doc = yaml.load(raw, Loader=StrictLoader)
    except yaml.YAMLError as e:
        return [f"does not parse: {e}"]

    if not isinstance(doc, dict):
        return ["is not a mapping at the top level"]

    # A workflow with no `name` is displayed by file path, which is also how a
    # startup failure presents. Requiring one keeps those two states distinct
    # at a glance, which is the thing that cost hours here.
    if not doc.get("name"):
        problems.append("has no `name:`, so a healthy run is indistinguishable "
                        "from a startup failure in the runs list")

    # `on` parses as the boolean True in YAML 1.1 unless quoted. Both spellings
    # work on GitHub; this only checks the trigger exists at all.
    if "on" not in doc and True not in doc:
        problems.append("has no `on:` trigger")

    jobs = doc.get("jobs")
    if not isinstance(jobs, dict) or not jobs:
        problems.append("declares no jobs")
        return problems

    for name, job in jobs.items():
        if not isinstance(job, dict):
            problems.append(f"job {name}: is not a mapping")
            continue
        if not job.get("runs-on") and not job.get("uses"):
            problems.append(f"job {name}: has no `runs-on`")
        steps = job.get("steps")
        if steps is not None:
            if not isinstance(steps, list) or not steps:
                problems.append(f"job {name}: `steps` is empty")
            else:
                for i, st in enumerate(steps):
                    if not isinstance(st, dict):
                        problems.append(f"job {name}: step {i} is not a mapping")
                    elif "uses" not in st and "run" not in st:
                        problems.append(
                            f"job {name}: step {i} has neither `uses` nor `run`")
    return problems


def main():
    root = pathlib.Path(".github/workflows")
    files = sorted(root.glob("*.yml")) + sorted(root.glob("*.yaml"))
    if not files:
        print("workflow-lint: no workflow files found", file=sys.stderr)
        return 1
    bad = 0
    for f in files:
        problems = check(f)
        if problems:
            bad += 1
            print(f"  FAIL {f}")
            for p in problems:
                print(f"       {p}")
        else:
            print(f"  ok   {f}")
    if bad:
        print(f"\n{bad} workflow file(s) GitHub would refuse to run. A run "
              f"created with zero jobs reports `failure` and looks exactly "
              f"like a broken test.", file=sys.stderr)
        return 1
    print(f"\n{len(files)} workflow files parse the way GitHub will parse them.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
