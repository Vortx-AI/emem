#!/usr/bin/env python3
"""Doc lint: the prose convention, enforced instead of remembered.

The convention (CONTRIBUTING.md, "Docs follow the house prose
convention"): no em or en dashes, no buzzword framing, roadmap never
presented as capability, every relative link resolving. The first two
and the last are mechanical, so a gate can hold them; this is that gate.
The judgment calls (claim-vs-code, roadmap-vs-shipped) stay with review,
and counts stay with scripts/sync_counts.py.

Checks, over README.md, CONTRIBUTING.md, and docs/**/*.md:
  1. No em dash (U+2014) or en dash (U+2013). Ranges spell "to"
     ("0.5 s to 1.6 s"), asides use a comma or a colon.
  2. No AGI-family vocabulary (owner's rule: the economics is stated as
     cheap generation versus scarce verification, never as an era).
  3. No buzzword tells from the short list below.
  4. Every relative markdown link resolves from the file's directory
     (or the repo root, matching how the docs site serves them).

Exemptions, all visible here rather than silent:
  * docs/book/ is generated output; lint the sources, not the render.
  * docs/whitepaper-v1.md is the frozen DOI record, archived unedited.
  * LEGACY_DASHES is the burn-down list: files that predate the gate
    with their dash count pinned. A file on the list may only shrink;
    fixing it fully removes it from the list. New files never join.
  * Check 4 resolves relative links only. Site-absolute targets (`/verify`,
    `/docs/diagrams/*.svg`) are served by the responder, not by the repo, so
    there is nothing on disk to resolve them against. Measured against
    https://emem.dev on 2026-08-10: all 14 distinct site-absolute targets in
    docs/ answer 200, so the hole is real but currently empty.

Every name in SKIP and LEGACY_DASHES is checked to still exist. An entry for
a file that has been renamed or deleted stops being a decision and becomes a
comment, and it shrinks what this gate covers without saying so.

Usage: scripts/doc_lint.py            # lint, exit non-zero on findings
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

SKIP = {
    "docs/whitepaper-v1.md",  # frozen DOI record
    # Generated from the ledger by scripts/build_channel.py: it reproduces other
    # agents' SIGNED notes verbatim. Editing their words to satisfy our house
    # style would falsify a record whose only value is being unedited, and the
    # signatures would stop verifying against the bytes. The generator itself is
    # linted; its output is evidence, not prose we wrote.
    "docs/collaboration-log.md",
}
SKIP_DIRS = ("docs/book/",)

# SKIP_DIRS entries that are generated build output, not committed files.
# Their absence is the NORMAL state: docs/book/ is gitignored and mdbook only
# builds it at deploy/CI time, so a clean checkout does not have it and a
# developer box that has run the build does. Asserting such a directory exists
# therefore tests the checkout, not the exemption, and fails everywhere the
# gate actually runs. What the assertion is for is catching a rename, so for
# these the rename is caught where it would really show up: the .gitignore
# rule that names the same path.
GENERATED_DIRS = {"docs/book/"}

# The burn-down list: pre-gate dash debt, pinned so it can only shrink.
# Every entry is (file, max allowed dashes). Fix a file, delete its row.
LEGACY_DASHES = {
    "docs/developers/data-sources.md": 48,
    "docs/developers/inference.md": 40,
    "docs/plans/v0.0.8-and-v0.0.9.md": 37,
    "docs/developers/architecture.md": 20,
    "docs/developers/developing.md": 20,
    "docs/operators/operating.md": 17,
    "docs/registries/PR-modelcontextprotocol-servers.md": 7,  # archived April PR text
}

DASH = re.compile(r"[–—]")
BANNED_TERMS = re.compile(r"\b(?:post-)?agi\b|superintelligen", re.IGNORECASE)
# Small and deliberate: each of these is a tell, not a style choice.
# Extend only with words that never belong in this repo's voice.
BUZZWORDS = re.compile(
    r"\b(?:delve|seamless(?:ly)?|game-?changer|cutting-edge|revolutioni[sz]e[sd]?|"
    r"supercharge[sd]?|unleash(?:es|ed|ing)?|paradigm shift)\b",
    re.IGNORECASE,
)
MD_LINK = re.compile(r"\]\(([^)#\s]+?)(?:#[^)\s]*)?\)")


def prose_files() -> list[Path]:
    files = [REPO / "README.md", REPO / "CONTRIBUTING.md"]
    files += sorted((REPO / "docs").rglob("*.md"))
    out = []
    for f in files:
        rel = f.relative_to(REPO).as_posix()
        if rel in SKIP or any(rel.startswith(d) for d in SKIP_DIRS):
            continue
        out.append(f)
    return out


def strip_code(text: str) -> str:
    """Blank out fenced code blocks and inline code so a verbatim shell
    line or a quoted wire string cannot trip a prose check."""
    text = re.sub(r"```.*?```", lambda m: "\n" * m.group(0).count("\n"), text, flags=re.S)
    return re.sub(r"`[^`\n]*`", "", text)


def stale_exemptions() -> list[str]:
    """Names in SKIP and LEGACY_DASHES that no longer point at a file.

    Both tables are keyed by repo-relative path, and both are consulted with a
    lookup that treats a miss as "not exempt". So a renamed file silently drops
    off its own burn-down row and re-enters the gate at cap 0, or, for SKIP,
    silently re-enters the gate entirely. Either is arguably fine; what is not
    fine is that the table keeps carrying a reason for a file nobody can open,
    and the next reader takes the row as evidence a decision was made about the
    file that exists today.
    """
    out = []
    for rel in sorted(SKIP):
        if not (REPO / rel).exists():
            out.append(f"{rel}: listed in SKIP but does not exist; drop the entry or fix the name")
    for rel in sorted(LEGACY_DASHES):
        if not (REPO / rel).exists():
            out.append(f"{rel}: has a LEGACY_DASHES row ({LEGACY_DASHES[rel]}) but does not "
                       f"exist; drop the row or fix the name")
    ignore_rules = set()
    gitignore = REPO / ".gitignore"
    if gitignore.exists():
        ignore_rules = {
            line.strip() for line in gitignore.read_text(encoding="utf-8").splitlines()
        }
    for d in SKIP_DIRS:
        if d in GENERATED_DIRS:
            # Built at CI/deploy time, so the filesystem cannot answer this.
            # The .gitignore rule can: it moves when the directory is renamed.
            if not gitignore.exists():
                out.append(f"{d}: generated-dir exemption cannot be checked, .gitignore is missing")
            elif d not in ignore_rules and d.rstrip("/") not in ignore_rules:
                out.append(
                    f"{d}: listed in SKIP_DIRS as generated output but .gitignore no longer "
                    f"names it; the directory was probably renamed, so fix the name or drop it"
                )
        elif not (REPO / d).exists():
            out.append(f"{d}: listed in SKIP_DIRS but no such directory; drop it or fix the name")
    return out


# `fact_cid` is the FULL 32-byte blake3 digest, 52 base32 characters. The
# truncated `[..16]` rule (26 characters) belongs to `entity_cid` and
# `bundle_cid`, which anchor a reference rather than binding a whole body.
#
# This is checked because the wrong rule already shipped, was corrected in the
# whitepaper's errata as "the most consequential of v1's errors: it breaks
# content addressing itself", and was STILL LIVE in protocol.md and agents.md
# for months afterwards. A correction that lands where the error was noticed
# and not where it is read is not a correction. dpwotikn reported the widths
# as inconsistent across our documents on 2026-08-13, from outside, while
# building against them.
#
# An implementer who follows the truncated rule computes an id that can never
# match a real fact_cid: every lookup misses and every signature check fails,
# looking like a corrupt corpus rather than a wrong sentence.
# Matches only a line that DEFINES fact_cid as truncated. `fact_cid` appearing
# inside another construction that is legitimately truncated is not a finding:
# the bundle preimage lists fact_cids and then truncates the BUNDLE digest to
# 16 bytes, which is correct and would be a false positive. A lint that cries
# wolf on a correct line gets switched off, and then it is not protecting the
# incorrect one either.
FACT_CID_TRUNCATION = re.compile(
    r"(?:fact_?cid|FactCid)\s*=[^\n]*\[\.\.16\]",
    re.I,
)


def check_fact_cid_width(text: str, rel: str) -> list[str]:
    hits = []
    for i, line in enumerate(text.split("\n"), 1):
        if "entity_cid" in line or "bundle_cid" in line:
            continue  # the truncated rule is correct for those two
        if FACT_CID_TRUNCATION.search(line):
            hits.append(
                f"{rel}:{i}: fact_cid described as truncated `[..16]`. It is the "
                f"full 32-byte digest, 52 characters; `[..16]` is entity_cid and "
                f"bundle_cid. A reader following this computes an id that can "
                f"never match."
            )
    return hits


def main() -> int:
    # "clean over 0 files" is a pass, and it is the most misleading pass this
    # script can emit: nothing was read, so nothing was cleared. A rename of
    # docs/, a glob change, or running from the wrong directory all produce it,
    # and each looks identical to a genuinely clean tree in CI. Exit 2 means
    # the gate could not run, which is the honest verdict when it read nothing.
    files = list(prose_files())
    if not files:
        print("doc-lint: found no prose files to check. This repo has dozens, "
              "so the glob is wrong or the working directory is. Reporting "
              "'clean' here would certify files nobody opened.", file=sys.stderr)
        return 2

    problems: list[str] = stale_exemptions()
    for f in files:
        rel = f.relative_to(REPO).as_posix()
        raw = f.read_text(encoding="utf-8")
        prose = strip_code(raw)

        # Checked on the RAW text, not the prose: the wrong rule shipped inside
        # a fenced code block in protocol.md, which is exactly where a reader
        # copies it from, and strip_code would have hidden it.
        problems.extend(check_fact_cid_width(raw, rel))

        dashes = len(DASH.findall(prose))
        cap = LEGACY_DASHES.get(rel, 0)
        if dashes > cap:
            for i, line in enumerate(prose.splitlines(), 1):
                if DASH.search(line):
                    problems.append(f"{rel}:{i}: em/en dash (cap {cap}, found {dashes})")
        elif cap and dashes < cap:
            problems.append(
                f"{rel}: dash debt shrank to {dashes} (cap {cap}); lower its LEGACY_DASHES row"
            )

        for i, line in enumerate(prose.splitlines(), 1):
            if BANNED_TERMS.search(line):
                problems.append(f"{rel}:{i}: AGI-family vocabulary")
            if BUZZWORDS.search(line):
                m = BUZZWORDS.search(line)
                problems.append(f"{rel}:{i}: buzzword tell: {m.group(0)!r}")

        for m in MD_LINK.finditer(raw):
            target = m.group(1)
            if re.match(r"^[a-z][a-z0-9+.-]*:", target) or target.startswith("/"):
                continue  # absolute URL or site-absolute path
            if not ((f.parent / target).exists() or (REPO / target).exists()):
                line = raw[: m.start()].count("\n") + 1
                problems.append(f"{rel}:{line}: broken relative link: {target}")

    if problems:
        print(f"doc-lint: {len(problems)} finding(s)")
        for p in problems:
            print(f"  {p}")
        return 1
    print(f"doc-lint: clean over {len(prose_files())} files")
    return 0


if __name__ == "__main__":
    sys.exit(main())
