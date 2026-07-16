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

Usage: scripts/doc_lint.py            # lint, exit non-zero on findings
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

SKIP = {
    "docs/whitepaper-v1.md",  # frozen DOI record
}
SKIP_DIRS = ("docs/book/",)

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


def main() -> int:
    problems: list[str] = []
    for f in prose_files():
        rel = f.relative_to(REPO).as_posix()
        raw = f.read_text(encoding="utf-8")
        prose = strip_code(raw)

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
