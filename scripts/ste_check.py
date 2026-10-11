#!/usr/bin/env python3
"""Check the core MCP tool descriptions against ASD-STE100 sentence rules.

A host shows a model the core tools' descriptions and nothing else, and the
model picks a tool from them. Long sentences with several clauses are where a
model, or a reader whose first language is not English, takes the wrong
meaning. ASD-STE100 (Simplified Technical English) gives measurable limits:

  - a descriptive sentence has at most 25 words (rule 6.2);
  - an instruction has at most 20 words (rule 5.1);
  - use the active voice (rule 3.6).

On 2026-10-11, 42 of the 167 sentences in the 18 core descriptions were over
25 words (the longest was 45) and 16 used the passive. After the rewrite this
check counts 0 and 0, and it fails when any core sentence breaks a limit
again. The passive check is a pattern, not a parser, so it reports only the
common forms ("is set", "was given"); a sentence it misses is still reviewed
by a person.

Text in backticks counts as one word: an identifier is a name, not prose.

    python3 scripts/ste_check.py          # check
    python3 scripts/ste_check.py --report # list every sentence over a limit, all tiers
"""
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

SRC = Path(__file__).resolve().parent.parent / "crates/emem-mcp/src/lib.rs"
DESCRIPTIVE_MAX = 25
INSTRUCTION_MAX = 20
IMPERATIVE = {
    "call", "send", "use", "do", "pass", "quote", "read", "treat", "give", "anchor",
    "choose", "resolve", "mint", "get", "set", "add", "narrow", "draw", "put", "try",
}
PASSIVE = re.compile(
    r"\b(is|are|was|were|be|been|being)\s+"
    r"(\w+ed|given|made|done|shown|known|seen|written|built|kept|held|sent|found|taken)\b",
    re.I,
)
# Words that end in -ed but are adjectives in these texts, not participles.
NOT_PASSIVE = {"signed", "registered", "wired", "attested", "embedded", "cached", "curated"}


def literal(src: str, start: int) -> str:
    """The Rust string literal whose opening quote is at `start`, unescaped."""
    out, i = [], start + 1
    while True:
        c = src[i]
        if c == "\\":
            n = src[i + 1]
            if n == "\n":
                i += 2
                while src[i] in " \t\n":
                    i += 1
                continue
            out.append({"n": "\n", "t": "\t", '"': '"', "\\": "\\"}.get(n, n))
            i += 2
            continue
        if c == '"':
            return "".join(out)
        out.append(c)
        i += 1


def tools(src: str):
    """(name, tier, description, when_to_use) for each ToolDescriptor."""
    for m in re.finditer(r"ToolDescriptor \{\s*\n\s*name: \"([a-z_]+)\",", src):
        block = src[m.end(): m.end() + 8000]
        nxt = block.find("ToolDescriptor {")
        if nxt > 0:
            block = block[:nxt]
        tier = re.search(r'tier: "([a-z]+)"', block)
        fields = {}
        for f in ("description", "when_to_use"):
            j = block.find(f'{f}: "')
            fields[f] = literal(block, j + len(f) + 2) if j >= 0 else ""
        yield m.group(1), tier.group(1) if tier else "", fields["description"], fields["when_to_use"]


def sentences(text: str) -> list[str]:
    # A table (emem_intent's rows) is not prose.
    text = "\n".join(line for line in text.splitlines() if "|" not in line)
    text = re.sub(r"`[^`]*`", "CODE", text)
    parts = re.split(r"(?:(?<=[.!?])|(?<=[.!?]\)))\s+(?=[A-Z0-9('\"])", text)
    return [p.strip() for p in parts if p.strip()]


def words(s: str) -> int:
    return len(s.split())


def check(tier_filter: str | None) -> list[str]:
    src = SRC.read_text()
    problems = []
    for name, tier, desc, when in tools(src):
        if tier_filter and tier != tier_filter:
            continue
        for field, text in (("description", desc), ("when_to_use", when)):
            for s in sentences(text):
                n = words(s)
                first = s.split()[0].lower().strip("(\"'") if s.split() else ""
                limit = INSTRUCTION_MAX if first in IMPERATIVE else DESCRIPTIVE_MAX
                if n > limit:
                    kind = "instruction" if limit == INSTRUCTION_MAX else "sentence"
                    problems.append(f"{name}.{field}: {kind} of {n} words (max {limit}): {s[:120]}")
                p = PASSIVE.search(s)
                if p and p.group(2).lower() not in NOT_PASSIVE:
                    problems.append(f"{name}.{field}: passive '{p.group(0)}': {s[:120]}")
    return problems


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--report", action="store_true", help="every tier, never fails")
    a = ap.parse_args()
    if a.report:
        for p in check(None):
            print(p)
        return 0
    core = [t for t in tools(SRC.read_text()) if t[1] == "core"]
    # A check that parsed nothing would pass: refuse that instead.
    if len(core) < 10:
        print(f"parsed only {len(core)} core tools from {SRC.name}; the parser no longer matches the source")
        return 1
    problems = check("core")
    if problems:
        print(f"{len(problems)} core tool sentence(s) break an ASD-STE100 limit:")
        for p in problems:
            print("  " + p)
        print("Split the sentence, or make it active. See the docstring for the rules.")
        return 1
    print(f"the {len(core)} core tool descriptions meet the ASD-STE100 sentence rules.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
