#!/usr/bin/env python3
"""Every link that leaves emem opens in a new tab.

A reader who clicks a directory listing, a DOI or a video from the middle of a
page has not asked to abandon the page. 320 of 321 off-site links across 26
pages were taking them away from it.

`noreferrer` rides along with `noopener`: a `target="_blank"` link without it
hands the destination our URL for free, and `noopener` alone does not stop that.

GENERATED PAGES ARE SKIPPED, not fixed. web/channel.html, web/tools.html and
web/whitepaper-v2.html are rewritten by redeploy.sh before every build, so an
edit here would be reverted by the next deploy and look like the fix had not
taken. Their generators are fixed instead, and this script FAILS if one of them
still emits a bare off-site anchor, so the skip cannot quietly become a hole.
"""
import re
import sys
import pathlib

REPO = pathlib.Path(__file__).resolve().parent.parent
WEB = REPO / "web"

# Rewritten by redeploy.sh; fix the generator, never the file.
GENERATED = {"channel.html", "tools.html", "whitepaper-v2.html"}

ANCHOR = re.compile(r'<a\b([^>]*?)>', re.I)
HREF = re.compile(r'href="([^"]+)"', re.I)


def offsite(url: str) -> bool:
    return url.startswith("http") and "emem.dev" not in url


def fix_attrs(attrs: str) -> str:
    if 'target=' in attrs:
        return attrs
    if 'rel="' in attrs:
        attrs = re.sub(r'rel="([^"]*)"',
                       lambda m: 'rel="%s"' % " ".join(
                           dict.fromkeys(m.group(1).split() + ["noopener", "noreferrer"])),
                       attrs)
        return attrs.rstrip() + ' target="_blank"'
    return attrs.rstrip() + ' target="_blank" rel="noopener noreferrer"'


def main() -> int:
    check = "--check" in sys.argv
    examined = changed = 0
    stale_generated = []
    for f in sorted(WEB.glob("*.html")):
        s = f.read_text(encoding="utf-8", errors="ignore")
        hits = []
        for m in ANCHOR.finditer(s):
            h = HREF.search(m.group(1))
            if h and offsite(h.group(1)):
                examined += 1
                # Same predicate the fixer uses. These disagreed: the hit test
                # looked for target="_blank" and the fixer for target=, so an
                # anchor written target='_blank' entered `hits`, came back
                # unchanged, and tripped the grew-the-file assertion. A gate
                # that raises is indistinguishable from one nobody ran.
                if "target=" not in m.group(1):
                    hits.append(m)
        if not hits:
            continue
        if f.name in GENERATED:
            stale_generated.append(f"{f.name}: {len(hits)} bare off-site anchor(s); "
                                   f"fix the generator, this file is rewritten on deploy")
            continue
        out, last = [], 0
        for m in hits:
            out.append(s[last:m.start()])
            out.append("<a" + fix_attrs(m.group(1)) + ">")
            last = m.end()
        out.append(s[last:])
        new = "".join(out)
        if len(new) <= len(s):
            print(f"  x {f.name}: rewrite changed nothing; the anchor shapes here "
                  f"are not what this script recognises")
            stale_generated.append(f"{f.name}: unrecognised anchor shape")
            continue
        changed += len(hits)
        if not check:
            f.write_text(new, encoding="utf-8")

    print(f"  {examined} off-site link(s) examined across {len(list(WEB.glob('*.html')))} page(s)")
    if examined == 0:
        print("\noffsite_new_tab: VACUOUS -- found no off-site link at all. These")
        print("  pages carry hundreds. Zero means the anchor scan broke, not that")
        print("  the site stopped linking out. Not a pass.")
        return 1
    for line in stale_generated:
        print(f"  x {line}")
    if check:
        if changed or stale_generated:
            print(f"\noffsite_new_tab: {changed} link(s) would change, "
                  f"{len(stale_generated)} generated page(s) stale.")
            return 1
        print("  Every off-site link opens in a new tab.")
        return 0
    print(f"  rewrote {changed} link(s)")
    return 1 if stale_generated else 0


if __name__ == "__main__":
    sys.exit(main())
