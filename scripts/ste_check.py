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
25 words (the longest was 45) and 16 used the passive. The core 18 were
rewritten first, then the other 97 tools (206 problems in 85 tools). This
check now covers every tool and fails when any sentence breaks a limit. The passive check is a pattern, not a parser, so it reports only the
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
NOT_PASSIVE = {"signed", "registered", "wired", "attested", "embedded", "cached", "curated", "shared"}


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
    # A table row has cells and no closing stop; a sentence with a pipe in it
    # ("Region is {place} | {cells}.") is still a sentence.
    text = "\n".join(
        line for line in text.splitlines()
        if " | " not in line or line.rstrip().endswith((".", "!", "?"))
    )
    text = re.sub(r"`[^`]*`", "CODE", text)
    # A stop that ends an abbreviation ends no sentence.
    for abbr in ("e.g.", "i.e.", "vs.", "etc."):
        text = text.replace(abbr, abbr.replace(".", "\u2024"))
    # A sentence can start with a lower-case tool name (emem_ask, memory_*).
    parts = re.split(r"(?:(?<=[.!?])|(?<=[.!?]\)))\s+(?=\S)", text)
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


def instructions_problems(origin: str) -> list[str]:
    """The MCP `initialize` instructions, which the server builds at run time."""
    import json
    import urllib.request

    problems = []
    for path in ("/mcp", "/mcp/full"):
        body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": {"name": "ste_check", "version": "1"}}}).encode()
        req = urllib.request.Request(origin + path, data=body, headers={
            "content-type": "application/json", "accept": "application/json, text/event-stream"})
        with urllib.request.urlopen(req, timeout=60) as r:
            text = json.load(r)["result"].get("instructions", "")
        if not text:
            problems.append(f"{path}: no instructions came back")
        # Each line is its own unit: list items and headings end without a stop.
        for line in text.splitlines():
            for s in sentences(line):
                n = words(s)
                first = s.split()[0].lower().strip("(\"'") if s.split() else ""
                limit = INSTRUCTION_MAX if first in IMPERATIVE else DESCRIPTIVE_MAX
                if n > limit:
                    problems.append(f"{path} instructions: {n} words (max {limit}): {s[:120]}")
                p = PASSIVE.search(s)
                if p and p.group(2).lower() not in NOT_PASSIVE:
                    problems.append(f"{path} instructions: passive '{p.group(0)}': {s[:120]}")
    return problems


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--report", action="store_true", help="list every problem, never fail")
    ap.add_argument("--instructions-from", metavar="ORIGIN",
                    help="also check the initialize instructions a running node serves")
    a = ap.parse_args()
    if a.instructions_from:
        problems = instructions_problems(a.instructions_from.rstrip("/"))
        if problems:
            print(f"{len(problems)} instruction sentence(s) break an ASD-STE100 limit:")
            for p in problems:
                print("  " + p)
            return 1
        print("the MCP instructions on /mcp and /mcp/full meet the ASD-STE100 sentence rules.")
        return 0
    if a.report:
        for p in check(None):
            print(p)
        return 0
    core = [t for t in tools(SRC.read_text()) if t[1] == "core"]
    # A check that parsed nothing would pass: refuse that instead.
    if len(core) < 10:
        print(f"parsed only {len(core)} core tools from {SRC.name}; the parser no longer matches the source")
        return 1
    problems = check(None)
    if problems:
        print(f"{len(problems)} tool sentence(s) break an ASD-STE100 limit:")
        for p in problems:
            print("  " + p)
        print("Split the sentence, or make it active. See the docstring for the rules.")
        return 1
    n = sum(1 for _ in tools(SRC.read_text()))
    print(f"all {n} tool descriptions ({len(core)} core) meet the ASD-STE100 sentence rules.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
