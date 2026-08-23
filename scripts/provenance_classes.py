#!/usr/bin/env python3
"""Every provenance_class we emit must be one we declare.

Why this is a gate and not a fix
--------------------------------
The upstream found an undeclared class, `measurement`, on eleven of their
surfaces -- more often than one of the real ones appeared. The correct diagnosis
was already written down in their own tree, in full, under a heading explaining
why that word was wrong. It had reached exactly one call site.

Ours was `rendering_of_measurement`, on the ground-postcard descriptor in the
ask payload. A hybrid name implies a hybrid guarantee, and there is none: a
drawing does not recompute and carries no hash. It is retired for `none`, which
is a statement that the thing sits outside the provenance system rather than an
eighth class.

That fix, on its own, is the fix that already failed once for both of us. So the
rule is checked rather than remembered.

What counts as declared
-----------------------
The seven in `emem_core::substrates::KNOWN_PROVENANCE`, plus `none`, which is
admitted EXPLICITLY as an out-of-system declaration and not as a class. A value
outside that set is either a typo or a new guarantee nobody has defined, and
both should stop a build.
"""
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
SUBSTRATES = REPO / "crates" / "emem-core" / "src" / "substrates.rs"


def declared() -> set[str]:
    """Read the canonical list out of the source, never a copy of it."""
    src = SUBSTRATES.read_text(encoding="utf-8")
    m = re.search(r"const KNOWN_PROVENANCE:\s*\[&str;\s*\d+\]\s*=\s*\[(.*?)\];", src, re.S)
    if not m:
        sys.exit("could not find KNOWN_PROVENANCE in substrates.rs; "
                 "this gate reads the list rather than restating it, so it "
                 "cannot run if the list moves")
    return set(re.findall(r'"([a-z_]+)"', m.group(1)))


def main() -> int:
    allowed = declared()
    # `none` is not a class. It is the statement that the thing being described
    # is outside the provenance system, and it is admitted so that saying so
    # stays possible without minting a class to say it with.
    admitted = allowed | {"none"}

    # Every literal assigned to a provenance_class field, in Rust or in JSON.
    pat = re.compile(r'"provenance_class"\s*:\s*"([^"]+)"')
    bad: list[tuple[str, int, str]] = []
    seen: dict[str, int] = {}
    for f in sorted(REPO.glob("crates/**/*.rs")):
        text = f.read_text(encoding="utf-8", errors="ignore")
        for m in pat.finditer(text):
            val = m.group(1)
            seen[val] = seen.get(val, 0) + 1
            if val not in admitted:
                line = text[: m.start()].count("\n") + 1
                bad.append((str(f.relative_to(REPO)), line, val))

    print(f"provenance classes: {len(allowed)} declared, "
          f"{len(seen)} distinct emitted, {sum(seen.values())} sites")
    for val, n in sorted(seen.items(), key=lambda kv: -kv[1]):
        mark = "" if val in admitted else "   <-- NOT DECLARED"
        note = "  (out-of-system statement, not a class)" if val == "none" else ""
        print(f"  {val:26} {n:3}{note}{mark}")

    if bad:
        print("\nUNDECLARED PROVENANCE CLASS EMITTED:")
        for rel, line, val in bad:
            print(f"  {rel}:{line}: {val!r}")
        print("\nA class outside the declared set is either a typo or a guarantee")
        print("nobody has defined. If the thing genuinely has no provenance, say")
        print("`none` and name what does carry it; do not mint a hybrid name,")
        print("because a hybrid name implies a hybrid guarantee.")
        return 1

    print("\nEvery emitted provenance class is one we declare.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
