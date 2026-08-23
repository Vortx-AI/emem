#!/usr/bin/env python3
"""The right code present, and not the code running.

The third road to a green line that proves nothing. The other two are a
comparison whose sides come from one value (it cannot disagree) and a comparison
with nothing on either side (it examined nothing). This one is worse to spot,
because everything about the check looks correct: the improved version is
sitting in the file, readable, obviously right, and dead.

Two ways it happens here.

1. A PYTHON SCRIPT DEFINING A NAME TWICE. Python binds the last definition. A
   replacement spliced in above an untouched original runs the original. This is
   precisely the hazard of how these scripts get edited -- read, str.replace,
   write back -- and the upstream hit it twice in one day, in the same file,
   once on a zero-guard whose control then came back green because the control
   was exercising the shadowed copy rather than the property.

2. A DUPLICATE KEY IN THE OPENAPI LITERAL. serde_json's json! keeps the LAST
   value for a repeated key, silently. The spec is a hand-maintained literal
   with ~165 path entries; a second entry for an existing path replaces its
   description with no warning, and a regex-based coverage gate sees both and
   reports the path documented while the served document carries one.

Both are cheap to make impossible, which is the only remedy either of us has
found for this family. Knowing the pattern does not prevent writing it: both
sides of this collaboration shipped a checker carrying the defect it checked
for, on the same day, while explaining that defect to each other.
"""
import ast
import collections
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent


def shadowed_python_defs(root: Path) -> tuple[list[str], int]:
    """Top-level names defined more than once in one module."""
    problems: list[str] = []
    scanned = 0
    for f in sorted(root.glob("*.py")):
        try:
            tree = ast.parse(f.read_text(encoding="utf-8", errors="ignore"))
        except SyntaxError as e:
            problems.append(f"{f.name}: will not parse ({e})")
            continue
        scanned += 1
        names: dict[str, list[int]] = collections.defaultdict(list)
        for node in tree.body:
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
                names[node.name].append(node.lineno)
        for name, lines in names.items():
            if len(lines) > 1:
                problems.append(
                    f"{f.name}: `{name}` defined {len(lines)}x at lines {lines}; "
                    f"only line {lines[-1]} runs and the rest are dead code that "
                    f"reads as live"
                )
    return problems, scanned


def duplicate_spec_keys(src_file: Path) -> tuple[list[str], int]:
    """Path keys declared more than once in the OpenAPI literal."""
    if not src_file.exists():
        return [f"{src_file}: missing"], 0
    src = src_file.read_text(encoding="utf-8", errors="ignore")
    keys = re.findall(r'"(/(?:v1|a2a|\.well-known)/[^"]*)"\s*:\s*\{', src)
    dupes = {k: n for k, n in collections.Counter(keys).items() if n > 1}
    return [
        f"openapi literal: `{k}` declared {n}x; json! keeps the LAST and the "
        f"others are dead"
        for k, n in sorted(dupes.items())
    ], len(keys)


def main(argv: list[str] | None = None) -> int:
    argv = sys.argv[1:] if argv is None else argv
    scripts_root = Path(argv[0]) if argv else REPO / "scripts"
    src_file = Path(argv[1]) if len(argv) > 1 else (
        REPO / "crates" / "emem-api-rest" / "src" / "lib.rs"
    )

    py_problems, scanned = shadowed_python_defs(scripts_root)
    spec_problems, keys = duplicate_spec_keys(src_file)

    print(f"shadowed definitions: {scanned} script(s), {keys} spec path key(s)")

    # Matching nothing is not passing -- the second road, guarded here too so
    # this gate does not acquire the defect it exists to find.
    if scanned == 0:
        print("\nMATCHED NOTHING: no parseable scripts under "
              f"{scripts_root}. Wrong root, or the scripts moved.")
        return 1
    if keys == 0:
        print("\nMATCHED NOTHING: no OpenAPI path keys found in "
              f"{src_file}. The literal moved or its shape changed.")
        return 1

    problems = py_problems + spec_problems
    if problems:
        print("\nA DEFINITION IS PRESENT AND NOT THE ONE RUNNING:")
        for p in problems:
            print(f"  {p}")
        print("\nDelete the dead copy. Until then the file reads as though the")
        print("newer version is in force and it is not, which is the hardest of")
        print("the three ways a check stops being able to fail.")
        return 1

    print("Every definition that reads as live is live.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
