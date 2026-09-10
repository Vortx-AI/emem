#!/usr/bin/env python3
"""No sentence we serve carries the indentation it was written at.

`cargo fmt` collapses a backslash-continued string literal onto one line and
the source indentation goes with it, INTO the string. The result reaches an
agent as

    "...this band needs.                  It is an optional extension..."

which is what emem.dev served on 2026-09-10, in `materialize_notes[].reason`,
on any place question that routed to a sidecar band. Twenty-one literals had
it, several of them years old and none of them noticed: an error message, the
reasoning tier's system prompt, four of the trust-boundary declarations, and
the memory-verb refusals. Nobody re-reads a message that is only produced when
something goes wrong.

WHAT IS NOT DAMAGE. Aligned columns are deliberate and common here:

    "GET  /v1/bands     , band catalogue with tempo"
    "fired rate         {:.4}"

so a literal is left alone when it opens with an HTTP verb, when the run is
followed by a column separator (`,` `|` `{`), or when it contains a newline and
is therefore laid out on purpose. What is left is padding inside a sentence.

This reads Rust source with a scanner rather than a regex. A regex over source
runs off the end of one literal into the next; the first attempt at this
reported 1106 findings, nearly all of them code.

  python3 scripts/no_padded_prose.py
  python3 scripts/no_padded_prose.py crates sdks

Exit 0 clean, 1 padded prose found, 3 nothing was read.
"""
from __future__ import annotations

import pathlib
import re
import sys

# Eight, not three. Three is the width of an aligned column; eight is only ever
# a line's indentation folded into a sentence.
PAD = re.compile(r"\S {8,}\S")
VERB = re.compile(r"^\s*(GET|POST|PUT|DELETE|PATCH|HEAD)\b")
COLUMN = re.compile(r" {8,}[,|{]")
MIN_LEN = 24


def literals(src: str):
    """Every string literal in a Rust source file, with its line number.

    Walks the file rather than matching it: line comments, block comments,
    raw strings, escapes and char literals each have to be stepped over, and a
    pattern that does not step over them reads code as text.
    """
    i, n, line = 0, len(src), 1
    while i < n:
        c = src[i]
        if c == "\n":
            line += 1
            i += 1
            continue
        if c == "/" and i + 1 < n and src[i + 1] == "/":
            while i < n and src[i] != "\n":
                i += 1
            continue
        if c == "/" and i + 1 < n and src[i + 1] == "*":
            depth, i = 1, i + 2
            while i < n and depth:
                if src.startswith("/*", i):
                    depth += 1
                    i += 2
                elif src.startswith("*/", i):
                    depth -= 1
                    i += 2
                else:
                    if src[i] == "\n":
                        line += 1
                    i += 1
            continue
        m = re.match(r'r(#*)"', src[i:])
        if m:
            hashes = m.group(1)
            start = i + m.end()
            end = src.find('"' + hashes, start)
            if end == -1:
                return
            text = src[start:end]
            yield line, text
            line += text.count("\n")
            i = end + 1 + len(hashes)
            continue
        if c == '"':
            j, buf = i + 1, []
            while j < n:
                if src[j] == "\\":
                    buf.append(src[j : j + 2])
                    j += 2
                    continue
                if src[j] == '"':
                    break
                buf.append(src[j])
                j += 1
            text = "".join(buf)
            yield line, text
            line += text.count("\n")
            i = j + 1
            continue
        if c == "'":
            i += 1
            continue
        i += 1


def padded(text: str) -> bool:
    if len(text) < MIN_LEN or "\\n" in text or "\n" in text:
        return False
    if not PAD.search(text):
        return False
    return not (VERB.match(text) or COLUMN.search(text))


def main() -> int:
    roots = [pathlib.Path(a) for a in sys.argv[1:]] or [pathlib.Path("crates")]
    files = [f for r in roots for f in sorted(r.rglob("*.rs"))]
    if not files:
        print(f"no .rs files under {', '.join(str(r) for r in roots)}")
        return 3

    hits = []
    scanned = 0
    for p in files:
        src = p.read_text(encoding="utf-8", errors="replace")
        for line, text in literals(src):
            scanned += 1
            if padded(text):
                hits.append((p, line, text))

    if scanned < 1000:
        print(f"read {scanned} literal(s) from {len(files)} file(s); the scanner is not "
              f"finding them, so a clean result here would mean nothing")
        return 3

    print(f"{len(files)} file(s), {scanned} string literal(s)")
    if hits:
        print(f"\n{len(hits)} sentence(s) carrying their own indentation:")
        for p, line, text in hits:
            run = PAD.search(text)
            print(f"  {p}:{line}")
            print(f"      {text[:110]}")
            print(f"      ^ {len(run.group(0)) - 2} spaces mid-sentence")
        print("\nWrite the literal on one line, however long it reads in the editor. "
              "A `\\`-continued string is what cargo fmt folds.")
        return 1
    print("nothing carries its own indentation.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
