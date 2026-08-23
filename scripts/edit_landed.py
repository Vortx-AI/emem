#!/usr/bin/env python3
"""Did this edit land in executable position, or inside a string?

    scripts/edit_landed.py <file.py> <text-that-should-be-code> [...]

Prints, for every occurrence of each pattern, whether it sits in CODE, inside a
STRING LITERAL, or inside a COMMENT. Exits 1 if any occurrence is not code.

Why this is a tool and not a gate
---------------------------------
It answers a question about ONE EDIT, not a property of the repository, and the
gate form does not work. The obvious invariant -- "no Python-looking line inside
a string literal" -- fires on every docstring that illustrates its own API and on
every HTML template carrying JavaScript, which is the same false positive that
gets checkers switched off. There is no repo-wide statement to make here. There
is a question to ask after splicing, and this asks it.

The hazard it exists for
------------------------
`ast.parse` succeeding proves the FILE is valid Python. It says nothing about
whether the text you just inserted is code, because a block spliced into a string
literal parses perfectly -- it is now content. I did that today, to a file whose
HTML templates are thousands of lines of string, and the syntax check passed. The
scope check I ran next also passed, reporting the definition in the right
function, because the string containing it is in that function.

Two gate families here have complementary blind spots and neither knows which it
has. An AST-based checker cannot see code spliced into a literal (correctly, but
it means a shadowed def hiding in a string reads as clean). A regex line scanner
sees inside literals and reports documentation as a finding. This closes the gap
from the other side: rather than asking a checker to distinguish code from
content in general, it asks whether ONE known piece of text is in executable
position, which is exactly answerable.

Four answers, not two
---------------------
    code      outside any string or comment
    data      inside a single-line string that is itself in executable
              position: a dict key, a message, a URL. The STRING is code; its
              CONTENT is data, and an edit that landed here is not running.
    inert     inside the body of a multi-line string. Text that never runs.
    comment   the other inert case.

A match counts as inside a token only when the WHOLE needle fits within it. A
needle straddling a string and the code after it -- a dict key and its colon --
is code, because part of it is. That single condition is what separates the two
mistakes below.

How
---
`tokenize`, not the AST, and TOKEN spans rather than line spans. The first version took every string node's
`lineno..end_lineno` and called all of it string -- which reports `x = f("lit")`
as inert text, because that line CONTAINS a string. My own control caught it on
the first run, on the very edit this tool was written to check. A tool that
misreads working code as dead is worse than none: the next person to see it
wrong stops believing it on the case that matters.

So: only the rows strictly INSIDE a multi-line string token are content. The row
where such a string opens can hold code before it; the rows after it cannot hold
anything. A line is code when it carries at least one token that is not a
string, comment, or layout.
"""
import ast
import io
import sys
import tokenize
from pathlib import Path


def token_spans(src: str) -> list[tuple[int, int, str]]:
    """Absolute character spans of every string and comment token.

    TOKEN granularity, not line granularity, and the upstream is why. My first
    version worked in lines: a row strictly inside a multi-line string was
    content, everything else was code. That gets a dict key right --
    `"verify_emem_facts": (` is a string sitting in executable position, and the
    row carries real tokens -- but it gets this wrong:

        msg = "SPLICED = this is data inside executable code"

    The needle is inside the string; the LINE is code; my tool said code. A
    false negative in the direction that matters, because the whole point is to
    tell you your edit is not running.

    Their rule is the one that separates the two cases: a match counts as
    inside a token only when the WHOLE needle fits within it. A needle
    straddling a string and the code after it -- a dict key and its colon -- is
    code, because part of it is.

    A STRING LITERAL IN CODE IS CODE; ITS CONTENT IS DATA. Both of our first
    versions misread working code as dead, from opposite directions: I called
    code-containing-a-string dead, they called a string-that-is-code dead. Both
    were caught by pointing the tool at a real file rather than at a fixture.
    """
    try:
        ast.parse(src)
    except SyntaxError as e:
        raise SystemExit(f"{e.__class__.__name__}: {e}. Fix the syntax first; "
                         f"this tool answers a narrower question than 'does it parse'.")

    # (row, col) -> absolute offset
    line_start = [0]
    for line in src.split("\n"):
        line_start.append(line_start[-1] + len(line) + 1)

    def off(rc: tuple[int, int]) -> int:
        return line_start[rc[0] - 1] + rc[1]

    # Python 3.12 SPLITS AN F-STRING INTO SEVERAL TOKENS, and this missed all
    # of them. There is no STRING token for `f"..."`: it arrives as
    # FSTRING_START, then FSTRING_MIDDLE for each run of literal text, with
    # ordinary tokens for whatever is inside the braces, then FSTRING_END. So a
    # needle in an f-string's literal text fell outside every span and came back
    # `code`.
    #
    # The upstream found that in their version. Mine was worse: the body of a
    # MULTI-LINE f-string also came back code -- text that never runs, reported
    # as executable, which is the exact failure this tool exists to prevent,
    # inside the tool. And it mattered here rather than in principle:
    # build_channel.py builds every page from multi-line f-strings, so the file
    # I most needed to check was the file this was blindest on.
    #
    # The distinction to preserve is the interesting one:
    #     f"prefix {x} SPLICED suffix"   literal text  -> data
    #     f"{SPLICED}"                   a name in the slot -> code
    # which falls out for free, because the braces hold ordinary tokens and
    # only FSTRING_MIDDLE is literal.
    toks = []
    try:
        toks = list(tokenize.generate_tokens(io.StringIO(src).readline))
    except tokenize.TokenError:
        pass  # truncated file; what was tokenized still applies

    FS_START = getattr(tokenize, "FSTRING_START", None)
    FS_MID = getattr(tokenize, "FSTRING_MIDDLE", None)
    FS_END = getattr(tokenize, "FSTRING_END", None)

    # An f-string's own span, so its literal chunks can be told multi-line
    # (inert) from single-line (data). Needs the END before the MIDDLEs can be
    # classified, hence the two passes.
    fstring_span: list[tuple[int, int, bool]] = []  # (start_off, end_off, multiline)
    if FS_START is not None:
        stack = []
        for tok in toks:
            if tok.type == FS_START:
                stack.append(tok)
            elif tok.type == FS_END and stack:
                st = stack.pop()
                fstring_span.append((off(st.start), off(tok.end), tok.end[0] > st.start[0]))

    def enclosing_fstring_is_multiline(a: int, b: int) -> bool:
        for fs_a, fs_b, multi in fstring_span:
            if fs_a <= a and b <= fs_b:
                return multi
        return False

    spans: list[tuple[int, int, str]] = []
    for tok in toks:
        if tok.type == tokenize.COMMENT:
            spans.append((off(tok.start), off(tok.end), "comment"))
        elif tok.type == tokenize.STRING:
            kind = "inert" if tok.end[0] > tok.start[0] else "data"
            spans.append((off(tok.start), off(tok.end), kind))
        elif FS_MID is not None and tok.type == FS_MID:
            a, b = off(tok.start), off(tok.end)
            spans.append((a, b, "inert" if enclosing_fstring_is_multiline(a, b) else "data"))
    return spans


VERDICTS = {
    "code": ("code", False),
    "data": ("STRING LITERAL — data inside executable code, not code itself", True),
    "inert": ("INERT — the body of a multi-line string; this never runs", True),
    "comment": ("COMMENT — inert text, not code", True),
}


def classify(path: Path, patterns: list[str]) -> int:
    src = path.read_text(encoding="utf-8")
    spans = token_spans(src)
    lines = src.split("\n")
    line_start = [0]
    for line in lines:
        line_start.append(line_start[-1] + len(line) + 1)

    def row_of(offset: int) -> int:
        lo, hi = 0, len(line_start) - 1
        while lo < hi - 1:
            mid = (lo + hi) // 2
            if line_start[mid] <= offset:
                lo = mid
            else:
                hi = mid
        return lo + 1

    def verdict_at(a: int, b: int) -> str:
        # Whole-needle containment: a match straddling a string and the code
        # after it is code, because part of it is.
        for ts, te, kind in spans:
            if ts <= a and b <= te:
                return kind
        return "code"

    bad = 0
    for pat in patterns:
        hits = []
        i = src.find(pat)
        while i >= 0:
            hits.append(i)
            i = src.find(pat, i + 1)
        if not hits:
            # Not finding the text is itself a finding: the edit did not land at
            # all, which is the failure this repo has hit repeatedly with
            # str.replace against a shape cargo fmt had already changed.
            print(f"  NOT FOUND  {pat!r} appears nowhere in {path.name}")
            bad += 1
            continue
        for a in hits:
            kind = verdict_at(a, a + len(pat))
            where, is_bad = VERDICTS[kind]
            bad += 1 if is_bad else 0
            ln = row_of(a)
            print(f"  {path.name}:{ln}  {where}\n       {lines[ln - 1].strip()[:72]}")
    return bad


def main() -> int:
    if len(sys.argv) < 3:
        print(__doc__.strip().split("\n\n")[1])
        return 2
    path = Path(sys.argv[1])
    if not path.exists():
        print(f"  {path}: missing")
        return 1
    bad = classify(path, sys.argv[2:])
    if bad:
        print(f"\n{bad} occurrence(s) are not in executable position. A block "
              f"spliced into a string literal parses fine and does nothing.")
        return 1
    print("\nEvery occurrence is code.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
