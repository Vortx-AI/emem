#!/usr/bin/env python3
"""Every colour a palette can put text on a surface with, as a WCAG ratio.

    python3 scripts/contrast_tokens.py            check every palette
    python3 scripts/contrast_tokens.py --report   print the full matrix

Why this is a gate and not a browser check
------------------------------------------
A palette failing WCAG AA is a STATIC property of the CSS. It does not need a
page, a viewport, or a running responder -- it needs the numbers and the
formula, both of which are here. The browser sweep in
`scripts/manual/production_audit.py` finds where a bad pair actually landed on
screen; this stops the pair existing.

It exists because one token, `--mute-2`, sat at 2.26-2.68:1 while carrying 10px
text across the whole site, and nothing said so. Then the same values turned up
in seven pages that keep a private copy of the palette under different names
(`--fg-3`, `--fg-mute`), in `--warn`, and in the scoreboard's status chips,
which are tuned for a dark ground and were never given light-theme values. Six
separate discoveries of one fact. A gate is the difference between finding that
once and finding it every time.

What it checks
--------------
Within each `:root`-ish block, every INK token against every SURFACE token
declared in the same block. Both families are recognised by name, and a name
this file does not recognise is REPORTED rather than skipped -- a palette that
renames its tokens must not silently fall out of the check, which is how the
`--fg-mute` copies escaped the first time.

The threshold is 4.5:1, WCAG AA for body text. Tokens whose names say they are
not text (`--rule`, `--highlight`, `--*-bg`) are surfaces or lines, not ink.

What it does NOT check
----------------------
Which pairs the pages actually USE. It assumes any ink can meet any surface in
its own palette, because on this site they do: --mute sits on --paper-3, chips
sit on --paper-2, and the inverted cards put muted text on --ink. If a palette
ever gains a pair that genuinely cannot co-occur, the honest fix is to name it
in EXEMPT_PAIRS with the reason, not to loosen the threshold.
"""
import argparse
import glob
import math
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
AA = 4.5

# Ink: something text is drawn IN. Surface: something text is drawn ON.
INK = re.compile(r"^--(ink|ink-\d|mute|mute-\d|muted|fg|fg-\w+|accent|accent-dim|warn|ok|bad|"
                  r"absent|machine|pub|ember|forest|sky|amber|dim|faint|info|pending|purple|red|"
                  r"teal|vermilion)$")
SURFACE = re.compile(r"^--(paper|paper-\d|bg|bg-\d|cream|cream-\d|night|panel|panel\d)$")
# Named so the reason is visible: these are lines and washes, never text.
NOT_TEXT = re.compile(r"^--(rule|rule-\d|rule-strong|rule-2|highlight|.*-bg|line|line-\d|edge|"
                      r"warn-line|teal-deep|w-.*|t-.*|s-\d|serif|mono|display)$")

# (file, ink, surface) pairs that cannot co-occur, with the reason. Empty on
# purpose: nothing has needed one yet, and the field exists so that the first
# real case is written down rather than silently tolerated.
EXEMPT_PAIRS: dict[tuple[str, str, str], str] = {
    # arcade.html declares TWO grounds in one block: a dark stage (--night,
    # --panel, --panel2) and one cream card (--paper), used by exactly one rule
    # -- #bubble, a speech bubble that sets its own dark ink. The stage inks
    # never meet the cream, and the cream's ink is not a token. Verified by
    # reading every `background:var(--paper)` and `color:var(--*)` use in that
    # file, not by assuming.
    ("arcade.html", "--teal", "--paper"): "stage ink never meets the one cream card",
    ("arcade.html", "--red", "--paper"): "stage ink never meets the one cream card",
    ("arcade.html", "--amber", "--paper"): "stage ink never meets the one cream card",
    ("arcade.html", "--dim", "--paper"): "stage ink never meets the one cream card",
    ("arcade.html", "--faint", "--paper"): "stage ink never meets the one cream card",
}


def oklch_to_linear_srgb(L: float, C: float, H: float) -> tuple[float, float, float]:
    h = math.radians(H)
    a, b = C * math.cos(h), C * math.sin(h)
    l_ = L + 0.3963377774 * a + 0.2158037573 * b
    m_ = L - 0.1055613458 * a - 0.0638541728 * b
    s_ = L - 0.0894841775 * a - 1.2914855480 * b
    l, m, s = l_ ** 3, m_ ** 3, s_ ** 3
    r = 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s
    g = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s
    bb = -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s
    return (min(1.0, max(0.0, r)), min(1.0, max(0.0, g)), min(1.0, max(0.0, bb)))


def hex_to_linear_srgb(h: str) -> tuple[float, float, float]:
    h = h.lstrip("#")
    if len(h) == 3:
        h = "".join(c * 2 for c in h)
    out = []
    for i in (0, 2, 4):
        v = int(h[i:i + 2], 16) / 255
        out.append(v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4)
    return tuple(out)


def luminance(value: str):
    """Relative luminance, or None if this is not a colour we can evaluate."""
    value = value.strip()
    m = re.match(r"oklch\(\s*([\d.]+)\s+([\d.]+)\s+([\d.]+)", value)
    if m:
        r, g, b = oklch_to_linear_srgb(float(m.group(1)), float(m.group(2)), float(m.group(3)))
    elif re.match(r"^#[0-9a-fA-F]{3,8}$", value):
        r, g, b = hex_to_linear_srgb(value[:7])
    else:
        return None
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def ratio(fg: str, bg: str):
    a, b = luminance(fg), luminance(bg)
    if a is None or b is None:
        return None
    return (max(a, b) + 0.05) / (min(a, b) + 0.05)


# Any block that declares a SURFACE is a palette, not just `:root`. The first
# version matched `:root`-ish selectors only and missed `.inverted` on the
# homepage -- a scoped dark palette on a light page, which redefines --paper,
# --ink and --mute and does NOT redefine --mute-2, so that one token kept its
# light value and ran at 3.31:1 on the dark panel. A scoped palette is exactly
# where this goes wrong, because the author overrides the tokens they were
# thinking about and inherits the ones they were not.
def raw_palette_bodies(path: Path):
    """Block bodies with comments stripped but declarations NOT parsed, so a
    malformed one is still visible as the text it is."""
    text = path.read_text(encoding="utf-8", errors="ignore")
    if path.suffix == ".html":
        text = "\n".join(re.findall(r"<style[^>]*>(.*?)</style>", text, re.S)) or text
    text = re.sub(r"/\*.*?\*/", " ", text, flags=re.S)
    return [(" ".join(m.group(1).split())[:60], m.group(2)) for m in BLOCK.finditer(text)]


BLOCK = re.compile(r"([^{}]+)\{([^}]*--(?:paper|bg|cream|night|panel)[^}]*)\}", re.S)
DECL = re.compile(r"(--[\w-]+)\s*:\s*([^;]+);")


def malformed_declarations(body: str) -> list[str]:
    """Declarations a browser will THROW AWAY, which this file would otherwise read.

    A regex reads `--mute-2:oklch(...)` happily whether or not the declaration
    before it was terminated. CSS does not: a missing semicolon glues two
    declarations into one, the parser cannot make sense of it, and BOTH are
    discarded. So the token is in the file, this checker sees it, and the page
    never gets it.

    That is not hypothetical. Adding `--mute-2` to `.inverted` in index.html, I
    put it after `clip-path:inset(0 -100vw)` -- which had no trailing semicolon,
    because it was the last declaration in the block. The browser dropped the
    clip-path AND the token; the footer heading stayed at 3.31:1; and this gate
    reported the palette clean. A checker that reads what CSS rejects is a
    checker that certifies what the reader never sees.

    Detection: inside one declaration, after parenthesised groups are removed,
    there is exactly one colon. Two means a missing terminator joined two.
    """
    bad = []
    for frag in body.split(";"):
        if not frag.strip():
            continue
        # A fragment carrying a brace is a block boundary the crude BLOCK regex
        # swept in (`@media (...) { :root {`), not a declaration. Judging those
        # as malformed reported four confident findings that were all the
        # parser -- the same shape of mistake this function exists to catch.
        if "{" in frag or "}" in frag:
            continue
        # url(data:...), oklch(...), var(--x, y) -- parens hide legal colons
        flat = re.sub(r"\([^()]*\)", "()", frag)
        for _ in range(3):
            flat = re.sub(r"\([^()]*\)", "()", flat)
        if flat.count(":") > 1:
            bad.append(" ".join(frag.split())[:110])
    return bad


def palettes(path: Path):
    """Every :root-ish block in a file, as (label, {token: value})."""
    text = path.read_text(encoding="utf-8", errors="ignore")
    # only the <style> content for html, to avoid matching prose
    if path.suffix == ".html":
        text = "\n".join(re.findall(r"<style[^>]*>(.*?)</style>", text, re.S)) or text
    # Strip CSS comments BEFORE finding blocks. The selector pattern reaches
    # backwards for whatever precedes the brace, so a comment above a rule was
    # captured as part of its selector -- which made a plain `:root` look like a
    # scoped block, and scoped blocks inherit. Every palette sitting under a
    # comment then inherited the light inks and was checked against its own dark
    # surfaces. Twelve confident findings, all of them the parser.
    text = re.sub(r"/\*.*?\*/", " ", text, flags=re.S)
    out = []
    for m in BLOCK.finditer(text):
        sel = " ".join(m.group(1).split())[:60]
        decls = {k: v.strip() for k, v in DECL.findall(m.group(2))}
        colours = {k: v for k, v in decls.items() if luminance(v) is not None}
        if colours:
            out.append((sel, colours))
    return out


def shared_inks() -> dict:
    """The inks every page inherits from web/tokens.css.

    A scoped palette inherits ACROSS FILES: `.inverted` lives in index.html and
    inherits --mute-2 from tokens.css, which is why computing this per-file left
    it empty and the check vacuous for exactly the block it was written for.
    """
    t = REPO / "web" / "tokens.css"
    if not t.exists():
        return {}
    out = {}
    for sel, toks in palettes(t):
        if sel.strip().startswith(":root"):
            out.update({k: v for k, v in toks.items() if INK.match(k)})
    return out


def check(path: Path, report: bool, shared: dict | None = None):
    fails, unknown, checked = [], set(), 0
    blocks = palettes(path)
    raw_blocks = raw_palette_bodies(path)
    # the page's own root inks, plus the ones every page inherits from tokens.css
    inherited = dict(shared or {})
    for sel, toks in blocks:
        if sel.strip().startswith(":root"):
            inherited.update({k: v for k, v in toks.items() if INK.match(k)})
    for sel, raw_body in raw_blocks:
        for frag in malformed_declarations(raw_body):
            fails.append(f"{path.name} [{sel[:30]}]: this is not one declaration, so CSS "
                         f"discards it and the page never sees either half -- a missing "
                         f"semicolon: {frag}")
    for sel, tokens in blocks:
        inks = {k: v for k, v in tokens.items() if INK.match(k)}
        surfaces = {k: v for k, v in tokens.items() if SURFACE.match(k)}
        # A scoped palette (.inverted, a dark card on a light page) inherits
        # every ink it does not redefine. Those inherited inks land on ITS
        # surfaces, so they belong in this block's check.
        if surfaces and not sel.strip().startswith((":root", "@media")):
            for k, v in inherited.items():
                inks.setdefault(k, v)
        for k in tokens:
            if not INK.match(k) and not SURFACE.match(k) and not NOT_TEXT.match(k):
                unknown.add(k)
        if not inks or not surfaces:
            continue
        for ik, iv in sorted(inks.items()):
            for sk, sv in sorted(surfaces.items()):
                if (path.name, ik, sk) in EXEMPT_PAIRS:
                    continue
                r = ratio(iv, sv)
                if r is None:
                    continue
                checked += 1
                if report:
                    print(f"    {path.name:24} {sel[:28]:28} {ik:12} on {sk:10} {r:5.2f}:1")
                if r < AA:
                    fails.append(f"{path.name} [{sel}] {ik} on {sk} is {r:.2f}:1, under {AA}")
    return fails, unknown, checked


SELF_TEST = [
    # (fg, bg, expected ratio within 0.15) -- values measured in Chrome via a
    # canvas, so this asserts the pure-python conversion agrees with a browser.
    ("oklch(0.516 0.005 85)", "oklch(0.93 0.006 85)", 4.55),
    ("oklch(0.42 0.005 85)", "oklch(0.93 0.006 85)", 6.90),
    ("oklch(0.20 0.005 85)", "oklch(0.975 0.005 85)", 16.88),
    ("oklch(0.68 0.005 85)", "oklch(0.955 0.005 85)", 2.53),
    ("#8a7a64", "#faf6ea", 3.85),
    ("#c9522a", "#faf6ea", 4.11),
]


def self_test() -> list[str]:
    """The conversion, against ratios measured in a real browser."""
    bad = []
    for fg, bg, want in SELF_TEST:
        got = ratio(fg, bg)
        if got is None:
            bad.append(f"could not evaluate {fg} on {bg}")
        elif abs(got - want) > 0.15:
            bad.append(f"{fg} on {bg}: this file says {got:.2f}:1, Chrome measured {want}:1")
    return bad


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--report", action="store_true")
    args = ap.parse_args()

    broken = self_test()
    if broken:
        print("THE COLOUR CONVERSION IS WRONG, so nothing below means anything:")
        for b in broken:
            print(f"  {b}")
        return 1

    shared = shared_inks()
    files = [Path(REPO / "web" / "tokens.css")] + sorted(Path(REPO).glob("web/*.html"))
    all_fails, all_unknown, total = [], set(), 0
    for f in files:
        if not f.exists():
            continue
        fails, unknown, checked = check(f, args.report, shared)
        all_fails += fails
        all_unknown |= unknown
        total += checked

    # Matching nothing is not passing.
    if total == 0:
        print("MATCHED NOTHING: no ink/surface pairs found in any palette. Either "
              "the token names changed or the block regex stopped matching.")
        return 1

    print(f"token contrast: {total} ink-on-surface pair(s) across {len(files)} file(s), "
          f"threshold {AA}:1")
    if all_unknown:
        print(f"  note: {len(all_unknown)} token(s) classified as neither ink nor "
              f"surface and not checked: {', '.join(sorted(all_unknown)[:8])}")
    if all_fails:
        print("\nA PALETTE CAN PUT TEXT ON A SURFACE AND HAVE IT UNREADABLE:")
        for f in all_fails:
            print(f"  {f}")
        return 1
    print("Every ink clears AA on every surface its own palette declares.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
