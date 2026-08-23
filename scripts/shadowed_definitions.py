#!/usr/bin/env python3
"""The right code present, and not the code running.

The third road to a green line that proves nothing. The other two are a
comparison whose sides come from one value (it cannot disagree) and a comparison
with nothing on either side (it examined nothing). This one is the hardest to
spot, because everything about the check looks correct: the improved version is
sitting in the file, readable, obviously right, and dead.

THE ROAD IS THE EDITING METHOD, NOT THE CONSTRUCT. Read the file, str.replace,
write it back -- which is how most of this repo's scripts get maintained --
produces every shape below, and gating only the shape you happen to have made is
the same narrowness as a phrase list that matches the phrasings someone thought
of. So all four:

  1. A top-level def or class defined twice. Python binds the LAST one; a
     replacement spliced above an untouched original runs the original.
  2. A dict literal with a repeated key. Python keeps the LAST silently, and
     these tables are hand-maintained.
  3. A module constant assigned twice. Same road, and the constant is what
     every importer gets. A forward declaration to None, filled in below once
     the function it needs exists, is an idiom rather than a shadow and is
     exempt -- a gate that flags a real pattern for looking like a defect is one
     people switch off.
  4. The same thing in Rust: serde_json's json! keeps the LAST value for a
     duplicate key, and the OpenAPI spec is a hand-maintained literal with ~165
     path entries. A second entry for an existing path replaces its description
     with no warning, and a regex-based coverage gate sees both and reports the
     path documented while the served document carries one.

This gate also refuses two ways of passing without checking, because it would be
absurd for it not to: it fails when it matched nothing (the second road), and it
reports its own errors as FINDINGS rather than raising (a checker that dies is
indistinguishable from a checker that was never run -- the upstream found that
by pointing their version at a root where it had no business succeeding).
"""
import ast
import collections
import re
import sys
import traceback
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

# Vendored and generated trees. Their defects are not ours to fix and their
# volume would bury ours: a first run over site-packages produced 28 findings,
# 27 of them in third-party code and one a false positive in ours.
SKIP = ("/.venv/", "/site-packages/", "/node_modules/", "/target/", "/docs/book/", "/.git/")


def python_files(root: Path) -> list[Path]:
    return [f for f in sorted(root.rglob("*.py")) if not any(s in "/" + str(f) for s in SKIP)]


def scan_python(root: Path) -> tuple[list[str], dict[str, int]]:
    """Shapes 1-3, over every non-vendored Python file under `root`."""
    problems: list[str] = []
    counts = {"files": 0, "defs": 0, "dicts": 0, "consts": 0}

    for f in python_files(root):
        try:
            tree = ast.parse(f.read_text(encoding="utf-8", errors="ignore"))
        except SyntaxError as e:
            problems.append(f"{f}: will not parse ({e})")
            continue
        counts["files"] += 1

        # 1. top-level defs and classes
        names: dict[str, list[int]] = collections.defaultdict(list)
        for node in tree.body:
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
                names[node.name].append(node.lineno)
        counts["defs"] += len(names)
        for name, lines in names.items():
            if len(lines) > 1:
                problems.append(
                    f"{f}: `{name}` defined {len(lines)}x at lines {lines}; only "
                    f"line {lines[-1]} runs and the rest read as live"
                )

        # 2. dict literals with a repeated constant key
        for node in ast.walk(tree):
            if not isinstance(node, ast.Dict):
                continue
            counts["dicts"] += 1
            keys = [
                k.value
                for k in node.keys
                if isinstance(k, ast.Constant) and isinstance(k.value, (str, int))
            ]
            for k, n in collections.Counter(keys).items():
                if n > 1:
                    problems.append(
                        f"{f}:{node.lineno}: dict key {k!r} appears {n}x; Python "
                        f"keeps the last and the others are dead"
                    )

        # 3. module constants assigned more than once, excluding None placeholders
        assigns: dict[str, list[tuple[int, bool]]] = collections.defaultdict(list)
        for node in tree.body:
            if not isinstance(node, ast.Assign):
                continue
            for t in node.targets:
                if not isinstance(t, ast.Name):
                    continue
                bare = t.id.lstrip("_")
                if not bare or not bare.isupper():
                    continue
                placeholder = isinstance(node.value, ast.Constant) and node.value.value is None
                assigns[t.id].append((node.lineno, placeholder))
        counts["consts"] += len(assigns)
        for name, occ in assigns.items():
            real = [ln for ln, ph in occ if not ph]
            if len(real) > 1:
                problems.append(
                    f"{f}: constant `{name}` assigned {len(real)}x at {real}; "
                    f"importers get the last one"
                )

    return problems, counts


def scan_spec(src_file: Path) -> tuple[list[str], int]:
    """Shape 4: a path declared twice in the OpenAPI literal."""
    if not src_file.exists():
        return [f"{src_file}: missing, so the spec is not being checked at all"], 0
    src = src_file.read_text(encoding="utf-8", errors="ignore")
    keys = re.findall(r'"(/(?:v1|a2a|\.well-known)/[^"]*)"\s*:\s*\{', src)
    dupes = {k: n for k, n in collections.Counter(keys).items() if n > 1}
    return [
        f"openapi literal: `{k}` declared {n}x; json! keeps the LAST and the rest are dead"
        for k, n in sorted(dupes.items())
    ], len(keys)


def main(argv: list[str] | None = None) -> int:
    argv = sys.argv[1:] if argv is None else argv
    py_root = Path(argv[0]) if argv else REPO
    src_file = (
        Path(argv[1])
        if len(argv) > 1
        else REPO / "crates" / "emem-api-rest" / "src" / "lib.rs"
    )

    # A checker that dies looks exactly like a checker nobody ran. Anything
    # thrown in here becomes a finding with its traceback, never an exit 1 from
    # the interpreter that a CI log reads as infrastructure noise.
    try:
        py_problems, counts = scan_python(py_root)
        spec_problems, keys = scan_spec(src_file)
    except Exception:
        print("THIS GATE RAISED, which is not the same as passing:")
        print(traceback.format_exc())
        return 1

    print(
        f"shadowed definitions: {counts['files']} python file(s), "
        f"{counts['defs']} top-level name(s), {counts['dicts']} dict literal(s), "
        f"{counts['consts']} module constant(s), {keys} spec path key(s)"
    )

    # Matching nothing is not passing.
    if counts["files"] == 0:
        print(f"\nMATCHED NOTHING: no parseable python under {py_root}.")
        return 1
    if keys == 0:
        print(f"\nMATCHED NOTHING: no OpenAPI path keys in {src_file}.")
        return 1

    problems = py_problems + spec_problems
    if problems:
        print("\nA DEFINITION IS PRESENT AND NOT THE ONE IN FORCE:")
        for p in problems:
            print(f"  {p}")
        print("\nDelete the dead copy. Until then the file reads as though the")
        print("newer version is in force and it is not, which is the hardest of")
        print("the ways a check stops being able to fail: the control you would")
        print("run to prove the newer version works exercises the older one.")
        return 1

    print("Every definition that reads as live is live.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
