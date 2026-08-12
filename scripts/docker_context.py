#!/usr/bin/env python3
"""Every compile-time include must survive the trip into the Docker context.

`include_str!`, `include_bytes!` and `include_dir!` read from the filesystem at
COMPILE time, so a file they point at is a build input in exactly the way a
`.rs` file is. The difference is that nothing links them: adding an include is
a one-line change in Rust, and the Dockerfile that has to carry the file sits
in another directory and is edited by another task on another day.

That is the whole failure. On 2026-08-11 `server.json` gained an
`include_str!` in `crates/emem-api-rest/src/lib.rs`; the Dockerfile's COPY list
had not changed since 2026-07-12 and copies selectively rather than copying the
context wholesale. The image build then failed:

    error: couldn't read `crates/emem-api-rest/src/../../../server.json`

after **1422 seconds**, because it fails during the release compile, at the end
of a thirteen-stage build. This check answers the same question in about a
second, from static text, with no Docker daemon and no network. A regression
that takes 23 minutes to reproduce is one people learn to retry rather than
read.

It is the same shape as the packaging trap that shipped two empty SDK wheels: a
file that exists in the repo, is committed, passes every local build, and is
absent from the artefact because a manifest somewhere else did not list it.
Local `cargo build` cannot see it. Only the packaging step can, and by then the
feedback is slow enough to be ignored.

Two ways a needed file goes missing, and both are checked:

  1. No `COPY` in the Dockerfile covers it (the file, or a directory above it).
  2. A `.dockerignore` pattern excludes it even though a COPY covers it. This is
     the nastier one, because the Dockerfile reads as correct.

    python3 scripts/docker_context.py            # exit 1 on any missing input
    python3 scripts/docker_context.py --list     # print what was resolved

Exit codes: 0 every include is in the context, 1 at least one is not.
"""

from __future__ import annotations

import os
import re
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# Only includes that ESCAPE their crate matter. A crate-relative include rides
# along with `COPY crates/ crates/` by construction, so flagging those would be
# noise, and a check that reports things which cannot break gets ignored.
INCLUDE = re.compile(r'include_(?:str|bytes|dir)!\s*\(\s*"((?:\.\./)+[^"]+)"')


def rust_includes() -> dict[str, set[str]]:
    """{repo-relative path needed: {the .rs files that need it}}."""
    needed: dict[str, set[str]] = {}
    for root, dirs, files in os.walk(os.path.join(REPO, "crates")):
        dirs[:] = [d for d in dirs if d != "target"]
        for name in files:
            if not name.endswith(".rs"):
                continue
            src = os.path.join(root, name)
            try:
                text = open(src, encoding="utf-8", errors="replace").read()
            except OSError:
                continue
            for m in INCLUDE.finditer(text):
                target = os.path.normpath(os.path.join(os.path.dirname(src), m.group(1)))
                rel = os.path.relpath(target, REPO)
                if rel.startswith(".."):
                    continue  # outside the repo entirely; not our context to fix
                needed.setdefault(rel, set()).add(os.path.relpath(src, REPO))
    return needed


def copy_sources() -> list[str]:
    """Source tokens from every build-stage COPY (not `COPY --from=`)."""
    path = os.path.join(REPO, "Dockerfile")
    sources: list[str] = []
    for line in open(path, encoding="utf-8"):
        line = line.strip()
        if not line.upper().startswith("COPY "):
            continue
        parts = line[5:].split()
        parts = [p for p in parts if not p.startswith("--")]
        if any("--from" in tok for tok in line.split()):
            continue  # copies from an earlier stage, not from the context
        if len(parts) < 2:
            continue
        sources.extend(p.rstrip("/") for p in parts[:-1])  # last token is the dest
    return sources


def ignore_patterns() -> list[str]:
    path = os.path.join(REPO, ".dockerignore")
    if not os.path.exists(path):
        return []
    out = []
    for line in open(path, encoding="utf-8"):
        line = line.strip()
        if line and not line.startswith("#"):
            out.append(line.rstrip("/"))
    return out


def covered_by(rel: str, sources: list[str]) -> str | None:
    """The COPY token that carries `rel`, or None. A directory token carries
    everything beneath it, which is how `COPY docs/ docs/` covers a diagram."""
    parts = rel.split("/")
    for i in range(len(parts), 0, -1):
        candidate = "/".join(parts[:i])
        if candidate in sources or candidate == ".":
            return candidate
    return None


def excluded_by(rel: str, patterns: list[str]) -> str | None:
    import fnmatch

    parts = rel.split("/")
    for pat in patterns:
        if pat.startswith("!"):
            continue  # a re-include; treat as not excluding
        for i in range(len(parts), 0, -1):
            if fnmatch.fnmatch("/".join(parts[:i]), pat):
                return pat
    return None


def main() -> int:
    needed = rust_includes()
    sources = copy_sources()
    patterns = ignore_patterns()
    show = "--list" in sys.argv

    problems: list[str] = []
    for rel in sorted(needed):
        why = None
        if not os.path.exists(os.path.join(REPO, rel)):
            why = "does not exist in the repo at all"
        elif covered_by(rel, sources) is None:
            why = "no COPY in the Dockerfile carries it into the build context"
        else:
            pat = excluded_by(rel, patterns)
            if pat:
                why = f"excluded by the .dockerignore pattern {pat!r}"
        if why:
            users = ", ".join(sorted(needed[rel])[:2])
            problems.append(f"{rel}: {why}\n      included by {users}")
        elif show:
            print(f"  ok   {rel:<48} via COPY {covered_by(rel, sources)}")

    print(f"\ndocker-context: {len(needed)} compile-time includes escape their "
          f"crate, {len(problems)} missing from the image build")
    if problems:
        print("\nThe image build compiles these paths in. A file the Dockerfile "
              "does not copy fails the release build ~23 minutes in, with an "
              "error that names the include and not the COPY.")
        for p in problems:
            print(f"  ✗ {p}")
        return 1
    print("Every file compiled into the binary is carried into the image.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
