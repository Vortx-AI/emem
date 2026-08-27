#!/usr/bin/env python3
"""Nobody's identity may be a literal in a binary other people run.

On 2026-08-27 this codebase published one operator's company, country, website
and e-mail from nine literals across six public surfaces, so every self-hosted
node asserted that identity as its own -- including in security.txt, whose only
job is to route a vulnerability report to whoever runs THAT machine.

Two checks, deliberately different in kind:

  1. STRUCTURAL. The fields that carry an identity must take their value from a
     function call, not a string literal. This does not need to know who the
     operator is, which matters: a gate that carried the identity in order to
     look for it would put the value back into the repository.

  2. GENERIC. No e-mail address literal in shipped (non-test) Rust at all.
     Catches a new field nobody thought to add to the list above -- the failure
     mode where a scanner only recognises the forms it was told about.

Exits non-zero when it examined nothing, because these fields do exist and a
zero here means the source moved, not that the source got clean.
"""
import re
import sys
import pathlib

REPO = pathlib.Path(__file__).resolve().parent.parent
SHIPPED = sorted(REPO.glob("crates/*/src/**/*.rs"))

# Fields whose value IS an identity claim about whoever runs the node.
# `organization` was missing and a literal survived in the A2A provider block
# for a full day after the sweep. The generic e-mail check could not see it
# either, because a company name is not an e-mail. Widened, and the lesson
# is the one this file already states: a scanner recognises the forms it was
# told about, so the list is a claim about coverage and not a guarantee.
IDENTITY_FIELDS = ("contact", "contact_email", "vendor", "author",
                   "operator_erasure", "organization", "company",
                   "publisher", "maintainer", "legal_name")

# A value that is not a literal: a call, a match, a variable.
LITERAL = re.compile(r'"(%s)":\s*"([^"]{2,})"' % "|".join(IDENTITY_FIELDS))

EMAIL = re.compile(r'"[\w.+-]+@[\w-]+\.[\w.-]+"')

# Addresses that belong to nobody in particular.
GENERIC = re.compile(r'"(example|test|noreply|none|user|someone)@', re.I)


def show(f: pathlib.Path) -> str:
    """Repo-relative when it can be, absolute when it cannot.

    `Path.relative_to` RAISES on a path outside the repo, and the first thing
    that hands this gate such a path is its own control fixture in a temp
    directory. A gate that dies on its control is indistinguishable in a log
    from a gate nobody ran. css_override_order.py had this exact bug, found the
    exact same way; the fix did not travel with the lesson.
    """
    try:
        return str(f.relative_to(REPO))
    except ValueError:
        return str(f)


def test_spans(text: str) -> list[tuple[int, int]]:
    """Character ranges under a #[cfg(test)] module, which ships nothing."""
    spans = []
    for m in re.finditer(r"#\[cfg\(test\)\]", text):
        depth, i, started = 0, m.end(), False
        while i < len(text):
            if text[i] == "{":
                depth += 1
                started = True
            elif text[i] == "}":
                depth -= 1
                if started and depth == 0:
                    break
            i += 1
        spans.append((m.start(), i))
    return spans


def main() -> int:
    problems: list[str] = []
    fields_examined = 0
    emails_examined = 0
    files_examined = 0

    for f in SHIPPED:
        text = f.read_text(encoding="utf-8", errors="ignore")
        files_examined += 1
        spans = test_spans(text)
        in_test = lambda pos: any(a <= pos <= b for a, b in spans)

        for m in LITERAL.finditer(text):
            if in_test(m.start()):
                continue
            fields_examined += 1
            line = text[: m.start()].count("\n") + 1
            problems.append(
                f"{show(f)}:{line}: \"{m.group(1)}\" is the literal "
                f"{m.group(2)[:40]!r}. An identity field must come from the "
                f"environment, or a self-hosted node publishes this one as its own."
            )

        for m in EMAIL.finditer(text):
            if in_test(m.start()) or GENERIC.search(m.group(0)):
                continue
            line = text[: m.start()].count("\n") + 1
            emails_examined += 1
            problems.append(
                f"{show(f)}:{line}: e-mail literal {m.group(0)} in "
                f"shipped code. Read it from the environment."
            )

    # Count the fields that ARE wired correctly, so the denominator is real
    # rather than "we found no literals because we looked at nothing".
    wired = 0
    for f in SHIPPED:
        text = f.read_text(encoding="utf-8", errors="ignore")
        for fld in IDENTITY_FIELDS:
            wired += len(re.findall(r'"%s":\s*(op_value|operator_|match )' % fld, text))

    # An e-mail literal found in a field this gate never heard of IS something
    # examined. Leaving it out let the vacuity branch return first and report
    # "found no identity field at all" for a file that plainly had one -- the
    # right exit code for the wrong reason, which is the failure a control that
    # only asserts the exit code cannot see.
    examined = fields_examined + wired + emails_examined
    print(f"  {files_examined} shipped file(s); {examined} identity field(s) examined "
          f"({wired} env-driven, {fields_examined} literal)")

    if problems:
        print("\noperator_identity: an identity is compiled in.")
        for pb in problems:
            print(f"  x {pb}")
        return 1

    if examined == 0:
        print("\noperator_identity: VACUOUS -- found no identity field at all.")
        print("  This code serves an agent card, a plugin manifest and a security.txt,")
        print("  every one of which carries a contact. Zero means the field names")
        print("  moved and this gate is reading nothing. Not a pass.")
        return 1

    print("  Every identity field takes its value from the environment, and no")
    print("  shipped file carries an e-mail literal. A node that declares nothing")
    print("  publishes nothing, rather than borrowing whoever built the binary.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
