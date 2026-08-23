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

How
---
`tokenize`, not the AST. The first version took every string node's
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


def string_and_comment_lines(src: str) -> tuple[set[int], set[int]]:
    """Lines that are string CONTENT, and lines that are only a comment.

    The first version of this took every string node's `lineno..end_lineno` and
    called all of it string. That is wrong in the ordinary case and my own
    control caught it: `x = f("literal")` is code that CONTAINS a string, and it
    was reported as inert text. A tool that misreads working code as dead is
    worse than no tool, because the next person to see it right will stop
    believing the one that matters.

    So: tokenize, and mark only the rows strictly INSIDE a multi-line string
    token. The row where such a string opens can hold code before it, and the
    row where it closes can hold code after it; the rows between cannot hold
    anything. A line is then code when it carries at least one token that is not
    a string, comment, or layout.
    """
    try:
        ast.parse(src)
    except SyntaxError as e:
        raise SystemExit(f"{e.__class__.__name__}: {e}. Fix the syntax first; "
                         f"this tool answers a narrower question than 'does it parse'.")

    interior: set[int] = set()
    comments: set[int] = set()
    real: set[int] = set()
    IGNORE = {
        tokenize.STRING, tokenize.COMMENT, tokenize.NL, tokenize.NEWLINE,
        tokenize.INDENT, tokenize.DEDENT, tokenize.ENDMARKER,
    }
    if hasattr(tokenize, "FSTRING_START"):  # 3.12+ splits f-strings
        IGNORE |= {tokenize.FSTRING_START, tokenize.FSTRING_MIDDLE, tokenize.FSTRING_END}
    try:
        for tok in tokenize.generate_tokens(io.StringIO(src).readline):
            if tok.type == tokenize.COMMENT:
                comments.add(tok.start[0])
            elif tok.type == tokenize.STRING and tok.end[0] > tok.start[0]:
                # Rows after the opening one are content. The closing row is
                # content up to the quote, so treat it as content too: nothing
                # spliced there is executable.
                interior.update(range(tok.start[0] + 1, tok.end[0] + 1))
            elif tok.type not in IGNORE:
                real.add(tok.start[0])
    except tokenize.TokenError:
        pass  # truncated file; what was tokenized still applies

    # A row inside a multi-line string is content even if a later token starts
    # on the closing row, so interior wins over real for the rows it claims.
    return interior, comments - real


def classify(path: Path, patterns: list[str]) -> int:
    src = path.read_text(encoding="utf-8")
    strings, comments = string_and_comment_lines(src)
    lines = src.split("\n")

    bad = 0
    for pat in patterns:
        hits = [i + 1 for i, l in enumerate(lines) if pat in l]
        if not hits:
            # Not finding the text is itself a finding: the edit did not land at
            # all, which is the failure this repo has hit repeatedly with
            # str.replace against a shape cargo fmt had already changed.
            print(f"  NOT FOUND  {pat!r} appears nowhere in {path.name}")
            bad += 1
            continue
        for ln in hits:
            if ln in strings:
                where = "STRING LITERAL — inert text, not code"
                bad += 1
            elif ln in comments and lines[ln - 1].lstrip().startswith("#"):
                where = "COMMENT — inert text, not code"
                bad += 1
            else:
                where = "code"
            print(f"  {path.name}:{ln}  {where}   {lines[ln - 1].strip()[:64]}")
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
