#!/usr/bin/env python3
"""Does every page say what it is, once, in a length that survives?

    scripts/page_metadata.py [--max 175] [--verbose]

Why this exists
---------------
Eighteen of twenty pages carried a description longer than a search result
shows, the worst at 407 characters and several over 330. None of them was
wrong. They were paragraphs in a slot that displays a sentence, so the half a
reader saw was chosen by a truncation rule rather than by us, and the page most
in need of a good first line (`/reference`, the read surface) had the longest.

That is not a rendering bug, which is why nothing caught it: every page looked
correct in a browser, and the only surface where the length mattered was one we
do not render. So it is measured here instead.

What it checks
--------------
  present     every served page has a `<title>` and a `<meta name=description>`.
              A page with no description is described by whatever a search
              engine scrapes.
  length      descriptions MAX_CHARS or under and at least MIN_CHARS, because a
              stub is the same failure from the other end; titles MAX_TITLE or
              under, which is where a result stops showing them.
  distinct    no two pages share a description. Two pages with one sentence
              between them means one of them is unlabelled, and the duplicate
              usually arrives by copying a template and forgetting the slot.

What it does NOT check
----------------------
Whether the sentence is any good. That is a judgement, it belongs in review,
and a gate that tried would either be trivially satisfiable or wrong. This
holds the mechanical floor: present, sized, and not a copy of another page's.
"""
import argparse
import html as H
import pathlib
import re
import sys

MAX_CHARS = 175
MIN_CHARS = 50
# A result shows about sixty characters of a title. Seventy is the slack this
# holds to, so a page has room to be specific without the end being cut off.
MAX_TITLE = 70

# Not a browsed page. `mcp-fact-card.html` is `ui://emem/fact-card`, an MCP
# Apps view (SEP-1865) rendered inside a host's chrome from a resource read,
# never fetched by a browser and never indexed. A description there would
# describe nothing to nobody. Exempt by name and with the reason, rather than by
# a pattern that would quietly swallow the next real page.
NOT_BROWSED = {"mcp-fact-card.html"}

# Generated pages are checked like any other: they are what a visitor gets.
# When one fails, the fix belongs in its generator, and the message says so.
GENERATED = {
    "channel.html": "scripts/build_channel.py",
    "tools.html": "scripts/gen_tools_page.py",
    "whitepaper-v2.html": "scripts/render_whitepaper.py",
}

TITLE_RE = re.compile(r"<title>(.*?)</title>", re.S | re.I)

DESC_RE = re.compile(
    r'<meta\s+name=["\']?description["\']?\s+content=["\'](.*?)["\']\s*/?>', re.S | re.I
)


def described(path: pathlib.Path) -> str | None:
    m = DESC_RE.search(path.read_text(encoding="utf-8", errors="replace"))
    return " ".join(H.unescape(m.group(1)).split()) if m else None


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--max", type=int, default=MAX_CHARS)
    ap.add_argument("--verbose", action="store_true")
    args = ap.parse_args()

    web = pathlib.Path(__file__).resolve().parent.parent / "web"
    pages = sorted(p for p in web.glob("*.html") if p.name not in NOT_BROWSED)
    if not pages:
        # Matching nothing is not passing.
        print(f"  no pages found under {web}; this gate had nothing to check.")
        return 1

    problems: list[str] = []
    seen: dict[str, str] = {}
    for p in pages:
        raw = p.read_text(encoding="utf-8", errors="replace")
        tm = TITLE_RE.search(raw)
        where = f"(fix in {GENERATED[p.name]})" if p.name in GENERATED else ""
        if not tm:
            problems.append(f"{p.name}: no <title> {where}")
        else:
            t = " ".join(H.unescape(tm.group(1)).split())
            if len(t) > MAX_TITLE:
                problems.append(f"{p.name}: title is {len(t)} chars, over {MAX_TITLE} {where}")
        d = described(p)
        if d is None:
            problems.append(f"{p.name}: no meta description {where}")
            continue
        if len(d) > args.max:
            problems.append(
                f"{p.name}: {len(d)} chars, over {args.max} {where}\n"
                f"        {d[:100]}..."
            )
        elif len(d) < MIN_CHARS:
            problems.append(f"{p.name}: {len(d)} chars, under {MIN_CHARS} {where}")
        if d in seen and len(d) >= MIN_CHARS:
            problems.append(f"{p.name}: same description as {seen[d]} {where}")
        else:
            seen[d] = p.name
        if args.verbose:
            print(f"    {p.name:28} {len(d):4}")

    print(f"page metadata: {len(pages)} page(s) checked, "
          f"title <= {MAX_TITLE}, description {MIN_CHARS}-{args.max} chars")
    if problems:
        print("\nA PAGE DESCRIBES ITSELF IN MORE THAN A RESULT WILL SHOW:")
        for pr in problems:
            print(f"  {pr}")
        return 1
    print("Every page says what it is, once, in a length a result will show.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
