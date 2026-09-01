#!/usr/bin/env python3
"""Does every surface that states a version state THIS version?

    python3 scripts/version_surfaces.py

Why this exists
---------------
`Cargo.toml` said 2.3.0 with 19 workspace members. `SECURITY.md` told the world
that 1.0.x was current and everything else superseded, `CITATION.cff` minted
citations for 2.2.0, `CONTRIBUTING.md` sent contributors looking for 16 crates,
and `README.md` opened its "Honest limits" section with "Version 2.1.0, a
minor". Four documents, four different answers, none of them the repository's.

Nothing caught it because nothing was looking. `release_check.py` runs the other
gates; it does not compare a document to the manifest. A version bump touches
the manifest and then relies on a person remembering five other files, which is
the kind of promise that is kept until the day it is not.

Why this is not sync_counts
---------------------------
`sync_counts.py` already derives `crates` and `version` from `Cargo.toml`, so
the facts were in the tree. It never saw these files: it scans `docs/*.md`,
`docs/**/*.md` and `web/*.html`, and every file above lives at the REPO ROOT.
Its own comments say as much, in the notes explaining why two root-level
exemptions were removed as unreachable.

The root is not simply added to those scans, because sync_counts WRITES. Its
pre-commit hook rewrites what it finds, and the root holds CHANGELOG.md, the
dated agent handoffs and registry notes -- documents whose numbers are records
of what was true then. Pointing an auto-corrector at them is how a historical
quotation gets silently edited, which has happened here before.

So this one is read-only, names its surfaces one at a time, and reads the same
manifest sync_counts does rather than carrying a copy of the answer.

What it checks, and how it avoids the obvious trap
--------------------------------------------------
NAMED surfaces, each with the pattern that finds it. Not a sweep for anything
version-shaped: this repository is full of version strings that must NOT move --
every CHANGELOG heading, "the receipt preimage last changed in 2.0.0", "the 1.x
line promised", the pinned `ort`/Prithvi versions. A gate that flagged those
would be switched off within a week, and it would be right to switch it off.

So each surface is a (file, regex, what it should equal) triple, and a surface
whose pattern MATCHES NOTHING is a failure, not a pass. That is the half that
matters over time: a heading gets reworded, the regex quietly stops matching,
and a checker that treats "no match" as "no problem" reports green about a file
it can no longer see.
"""
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent


def workspace_facts() -> tuple[str, int]:
    """The version and member count, read from the manifest that defines them."""
    text = (REPO / "Cargo.toml").read_text(encoding="utf-8")
    m = re.search(r'^\s*version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"', text, re.M)
    if not m:
        raise SystemExit("Cargo.toml has no workspace version; nothing to compare against")
    members = re.search(r"members\s*=\s*\[(.*?)\]", text, re.S)
    n = len(re.findall(r'"([^"]+)"', members.group(1))) if members else 0
    if n == 0:
        raise SystemExit("Cargo.toml declares no workspace members; the count check would be vacuous")
    return m.group(1), n


# (file, regex with one capture group, what it must equal, human name)
# "minor" means the surface states MAJOR.MINOR rather than the full version.
def surfaces(version: str, crates: int):
    major_minor = ".".join(version.split(".")[:2])
    return [
        # The SDK manifests were NOT here, and this file's closing line says
        # "Every surface that states a version states this one". Four manifests
        # state one, and llama-index-tools-emem sat at 2.1.0 -- two minors
        # behind -- through every green run. The sentence was broader than the
        # list, which is the whole failure this file exists to prevent
        # elsewhere.
        ("sdks/emem-py/pyproject.toml", r'^version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"', version,
         "the published Python client"),
        ("sdks/emem-langmem/pyproject.toml", r'^version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"', version,
         "the LangChain store"),
        ("sdks/llama-index-tools-emem/pyproject.toml", r'^version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"', version,
         "the llama-index tool spec, unpublished but versioned"),
        ("sdks/emem-ts/package.json", r'"version":\s*"([0-9]+\.[0-9]+\.[0-9]+)"', version,
         "the published TypeScript client"),
        ("CITATION.cff", r"^version:\s*([0-9]+\.[0-9]+\.[0-9]+)\s*$", version,
         "the version citations are minted against"),
        ("AGENTS.md", r"version ([0-9]+\.[0-9]+\.[0-9]+), MSRV", version,
         "the version the agent guide states"),
        ("README.md", r"Version ([0-9]+\.[0-9]+\.[0-9]+), a minor", version,
         "the version README's Honest limits opens with"),
        ("SECURITY.md", r"\|\s*([0-9]+\.[0-9]+)\.x\s*\|\s*Yes\. Current", major_minor,
         "the supported-version row of the security policy"),
        ("CONTRIBUTING.md", r"edition 2021, ([0-9]+) crates", str(crates),
         "the crate count contributors are told to expect"),
        ("AGENTS.md", r"Rust workspace, ([0-9]+) crates", str(crates),
         "the crate count in the agent guide"),
    ]


SELF_TEST = [
    # (text, regex, should_find) -- the patterns, checked against the shapes
    # they are meant to match and one they must not.
    ("version: 2.3.0\n", r"^version:\s*([0-9]+\.[0-9]+\.[0-9]+)\s*$", "2.3.0"),
    ("| 2.3.x | Yes. Current. Fixes land here. |",
     r"\|\s*([0-9]+\.[0-9]+)\.x\s*\|\s*Yes\. Current", "2.3"),
    ("Rust 1.91, edition 2021, 19 crates, one binary",
     r"edition 2021, ([0-9]+) crates", "19"),
    # and the shape that must NOT be picked up: a historical statement
    ("The receipt preimage last changed in 2.0.0, which was a major",
     r"Version ([0-9]+\.[0-9]+\.[0-9]+), a minor", None),
]


def self_test() -> list[str]:
    bad = []
    for text, pattern, expect in SELF_TEST:
        m = re.search(pattern, text, re.M)
        got = m.group(1) if m else None
        if got != expect:
            bad.append(f"pattern {pattern!r} gave {got!r}, expected {expect!r}")
    return bad


def main() -> int:
    broken = self_test()
    if broken:
        print("THE PATTERNS ARE NOT WORKING, so nothing below would be found:")
        for b in broken:
            print(f"  {b}")
        return 1

    version, crates = workspace_facts()
    fails, checked = [], 0
    for fname, pattern, want, what in surfaces(version, crates):
        path = REPO / fname
        if not path.exists():
            fails.append(f"{fname} is missing, so {what} is not being checked at all")
            continue
        m = re.search(pattern, path.read_text(encoding="utf-8"), re.M)
        if not m:
            # Matching nothing is the failure that hides: the file was reworded
            # and the check silently stopped covering it.
            fails.append(f"{fname}: nothing matches the pattern for {what}. "
                         f"Either the wording moved or the statement is gone; "
                         f"a checker that finds nothing has not checked anything.")
            continue
        checked += 1
        if m.group(1) != want:
            fails.append(f"{fname} says {m.group(1)} where the workspace says {want} "
                         f"({what})")

    print(f"version surfaces: workspace is {version} with {crates} crates; "
          f"{checked} surface(s) located")
    if fails:
        print("\nA SURFACE STATES SOMETHING THE WORKSPACE DOES NOT:")
        for f in fails:
            print(f"  {f}")
        print("\nThese are the files a stranger reads first. Bump them with the "
              "manifest, or delete the claim.")
        return 1
    print("Every surface that states a version or a crate count states this one.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
