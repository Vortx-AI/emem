#!/usr/bin/env python3
"""STE lint: checks Markdown prose against the house convention (CONTRIBUTING.md, "Docs follow the house prose
convention") and a subset of ASD-STE100 Simplified Technical English. Standard library only.

    python3 scripts/ste_lint.py docs/quickstart.md README.md
    python3 scripts/ste_lint.py --changed origin/main          # only lines changed since that ref, in changed .md files
    python3 scripts/ste_lint.py --changed origin/main --fail-on error   # for CI: exit 1 on an error in changed lines

Rules (the severity of each is in scripts/ste_lint_rules.json "severity"):
  dash              an em or en dash in prose (house rule: use a comma, a colon or a full stop)               error
  sentence-length   instructions over 20 words, descriptions over 25 words (STE)                             error
  banned-word       a curated list of words with a simpler STE word (utilize -> use, via -> through)          error
  ai-tell           phrases that read as machine-written filler (delve, seamless, "it's not X, it's Y")     error
  passive           be/get + past participle (heuristic)                                                    warn
  ing-form          an -ing word that opens a sentence or follows by/for/after/when (heuristic)            warn
  phrasal-verb      multi-word verbs (set up, carry out)                                                    warn
  contraction       don't, it's                                                                             warn
  idiom             figurative phrases                                                                      warn
  exclamation       "!" at the end of a prose sentence                                                     warn
Skipped: code fences, inline code, URLs, link targets, HTML tags, tables' pipes, numbers with units (one word),
proper names and credit lines listed in the rules file.
The word lists are a curated subset. They are not the licensed ASD-STE100 dictionary. A heuristic finding is a prompt
to read the sentence again, not proof of a fault. Exit 1 when a finding is at or above --fail-on; with no --fail-on the
lint only reports (exit 0), so the legacy docs do not block a build.
"""
from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from collections import Counter, defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
DEFAULT_RULES = HERE / "ste_lint_rules.json"

IMPERATIVES = set("""add adjust apply ask avoid change check choose click close compare connect copy count do download drag draw
enter find follow get give go hide hover install keep let look make mark measure move note open pan paste pick press put read
remove replace reset run scroll see select send set show start stop switch tap tell toggle turn type use wait write zoom
include examine fill load save view clear calculate disconnect identify push pull release refer record""".split())
IRREG_PART = set("""made done shown seen given taken known found built drawn kept left put set sent held read run cut split chosen
written hidden grown thrown brought bought caught taught told sold paid laid led lost meant met spent won understood begun broken
driven eaten fallen flown forgotten frozen gotten ridden risen shaken spoken stolen torn worn withdrawn mistaken overtaken
undertaken shot fed hit let shut spread struck bound fit sunk sown mown""".split())
NOT_PART = set("need seed red bed shed speed hundred feed bleed breed embed bred indeed weed shred sled".split())
ADV = r"(?:not|also|then|now|only|often|usually|always|never|still|all|first|then|both|each|\w+ly)"
ING_ALLOW = set("""thing nothing something anything everything during morning evening string strings ring spring king wing
building ceiling lightning heading setting meaning warning finding reading drawing opening padding spacing timing rating
ranking marking recording crossing housing landing funding offspring bedding bring sing swing sting sling cling ping
interesting following""".split())
ING_PREP = r"(?:by|for|after|before|when|while|without|upon|on)"
CONTRACTION = re.compile(r"\b(?:\w+n['’]t|\w+['’](?:re|ve|ll|d|m)|(?:it|that|there|what|here|let|he|she|who|where|how|when|why)['’]s)\b",
                         re.I)
ABBREV = re.compile(r"(?:\b(?:e\.g|i\.e|etc|vs|Fig|fig|No|no|approx|ca|cf|al|Dr|Mr|Mrs|Ms|St|viz|resp|incl|Eq|eq|Sec|sec|w\.r\.t)\.|\b[A-Z]\.)$")
WORD = re.compile(r"[\w][\w'’\-]*")


def load_rules(path=None):
    r = json.loads(Path(path or DEFAULT_RULES).read_text())
    r["_banned"] = [(k, _phrase_re(k), v) for k, v in r.get("banned", {}).items()]
    r["_idioms"] = [(k, _phrase_re(k)) for k in r.get("idioms", [])]
    r["_phrasal"] = [(k, _phrasal_re(k)) for k in r.get("phrasal_verbs", [])]
    r["_terms"] = [[(t, _phrase_re(t, plural=True)) for t in grp] for grp in r.get("terms", [])]
    ex = r.setdefault("exempt", {})
    r["_credit"] = [re.compile(p, re.I) for p in ex.get("credit_patterns", [])]
    names = sorted(ex.get("proper_names", []), key=len, reverse=True)
    r["_names"] = re.compile(r"(?<![\w-])(?:" + "|".join(re.escape(n) for n in names) + r")(?![\w-])") if names else None
    units = sorted(ex.get("units", []), key=len, reverse=True)
    u = "|".join(re.escape(x) for x in units)
    r["_num"] = re.compile(r"(?<![\w.])[-+±~]?\d(?:[\d.,:/–−\-]*\d)?(?:\s?(?:" + u + r")(?![\w]))?" if u else r"\d[\d.,:/\-]*")
    r["_allow"] = {w.lower() for w in ex.get("allow_words", [])}
    return r


def _verb_forms(v):
    irr = {"set": ["set", "sets", "setting"], "put": ["put", "puts", "putting"], "cut": ["cut", "cuts", "cutting"],
           "find": ["find", "finds", "found", "finding"], "come": ["come", "comes", "came", "coming"],
           "go": ["go", "goes", "went", "gone", "going"], "take": ["take", "takes", "took", "taken", "taking"],
           "make": ["make", "makes", "made", "making"], "bring": ["bring", "brings", "brought", "bringing"],
           "give": ["give", "gives", "gave", "given", "giving"], "break": ["break", "breaks", "broke", "broken", "breaking"],
           "show": ["show", "shows", "showed", "shown", "showing"], "run": ["run", "runs", "ran", "running"],
           "get": ["get", "gets", "got", "gotten", "getting"], "hold": ["hold", "holds", "held", "holding"],
           "keep": ["keep", "keeps", "kept", "keeping"], "undertake": ["undertake", "undertakes", "undertook", "undertaken", "undertaking"],
           "spearhead": ["spearhead", "spearheads", "spearheaded", "spearheading"]}
    if v in irr:
        return irr[v]
    if v.endswith("y") and v[-2] not in "aeiou":
        return [v, v[:-1] + "ies", v[:-1] + "ied", v + "ing"]
    if v.endswith("e"):
        return [v, v + "s", v + "d", v[:-1] + "ing"]
    if v.endswith(("s", "sh", "ch", "x")):
        return [v, v + "es", v + "ed", v + "ing"]
    if re.search(r"[^aeiou][aeiou][bdgmnpt]$", v) and len(v) <= 4:
        return [v, v + "s", v + v[-1] + "ed", v + v[-1] + "ing"]
    return [v, v + "s", v + "ed", v + "ing"]


def _phrase_re(key, plural=False):
    k = key.strip()
    if k.endswith("~"):
        body = "(?:" + "|".join(re.escape(f) for f in _verb_forms(k[:-1].lower())) + ")"
    elif k.endswith("*"):
        body = re.escape(k[:-1]) + r"[\w-]*"
    else:
        parts = [re.escape(p) for p in k.split()]
        last = k.split()[-1]
        if re.fullmatch(r"[A-Za-z]+", last):
            if last.endswith("y") and plural:
                parts[-1] = re.escape(last[:-1]) + "(?:y|ies)"
            else:
                parts[-1] += "(?:s|es)?"
        body = r"\s+".join(parts)
    end = r"(?![\w-])" if re.search(r"[\w*~]$", k) else ""
    return re.compile(r"(?<![\w-])" + body + end, re.I)


def _phrasal_re(key):
    w = key.split()
    return re.compile(r"(?<![\w-])(?:" + "|".join(_verb_forms(w[0])) + r")\s+" + r"\s+".join(map(re.escape, w[1:])) + r"(?![\w-])", re.I)


# ---------------------------------------------------------------- text helpers
def _blank(s, a, b, ch):
    return s[:a] + ch * (b - a) + s[b:]


def mask(text, rules):
    """Same-length copy of text: credit strings -> spaces; names, URLs, identifiers, numbers with units -> '_' (one word)."""
    m = text
    for rx in rules["_credit"]:
        for x in rx.finditer(m):
            m = _blank(m, x.start(), x.end(), " ")
    for rx in (re.compile(r"https?://\S+|www\.\S+|\S+@\S+\.\w+"),
               re.compile(r"(?<![\w])(?:[\w.-]+/){2,}[\w.\-]*|(?<![\w])[\w-]+/[\w-]*\.\w+|[\w-]+\.(?:py|md|json|html?|js|css|tif|tiff|png|jpe?g|webp|geojson|parquet|csv|sh|txt|gpkg|pmtiles|mp4)\b"),
               re.compile(r"\b\w*_\w+\b")):
        for x in rx.finditer(m):
            m = _blank(m, x.start(), x.end(), "_")
    if rules["_names"] is not None:
        for x in rules["_names"].finditer(m):
            m = _blank(m, x.start(), x.end(), "_")
    for x in rules["_num"].finditer(m):
        m = _blank(m, x.start(), x.end(), "_")
    return m


def sentences(masked):
    """(start, end) spans of sentences. Splits at . ! ? followed by space, and at line breaks / paragraph marks."""
    out, start = [], 0
    for x in re.finditer(r"[.!?…]+[\"'”’)\]]*(?=\s)|\n+|\u2029", masked):
        if x.group().startswith(".") and ABBREV.search(masked[max(0, x.start() - 8):x.start() + 1]):
            continue
        end = x.end() if x.group()[0] in ".!?…\"" else x.start()
        if masked[start:end].strip():
            out.append((start, end))
        start = x.end()
    if masked[start:].strip():
        out.append((start, len(masked)))
    res = []
    for a, b in out:  # trim spaces
        while a < b and masked[a].isspace():
            a += 1
        while b > a and masked[b - 1].isspace():
            b -= 1
        res.append((a, b))
    return res


def words(s):
    return [w for w in WORD.findall(s)]


def is_procedural(sent_masked, ordered=False):
    ws = [w for w in words(sent_masked) if not set(w) <= set("_")]
    if not ws:
        return False
    f = ws[0].lower()
    two = " ".join(x.lower() for x in ws[:2])
    return ordered or f in IMPERATIVES or two in ("do not", "make sure") or f in ("don't", "don’t", "never", "always")


def excerpt(s, n=160):
    s = re.sub(r"\s+", " ", s).strip()
    return s if len(s) <= n else s[:n - 1] + "…"


# ---------------------------------------------------------------- checks on one text unit
def check_unit(orig, rules, loc, ordered=False, line_of=None):
    """Findings for one unit of text (a block element or a markdown paragraph). loc: dict with selector or line."""
    sev = rules.get("severity", {})
    lim = rules.get("limits", {"procedural": 20, "descriptive": 25})
    m = mask(orig, rules)
    out = []

    def add(rule, a, b, text, sugg, sent=None):
        L = dict(loc)
        if line_of is not None:
            L["line"] = line_of(a)
        f = {**L, "rule": rule, "severity": sev.get(rule, "warn"), "text": excerpt(text), "suggestion": sugg}
        if sent is not None and sent != text:
            f["sentence"] = excerpt(sent, 220)
        out.append(f)

    for a, b in sentences(m):
        sm, so = m[a:b], orig[a:b]
        ws = [w for w in words(sm)]
        n = len(ws)
        proc = is_procedural(sm, ordered)
        limit = lim["procedural"] if proc else lim["descriptive"]
        if n > limit:
            add("sentence-length", a, b, so, f"{n} words; {'procedural' if proc else 'descriptive'} limit is {limit}. "
                "Split it into shorter sentences.")
        hits = []
        for key, rx, repl in rules["_banned"]:
            for x in rx.finditer(sm):
                if x.group().lower() not in rules["_allow"]:
                    hits.append((x.start(), x.end(), x.group(), repl))
        kept = []
        for h in sorted(hits, key=lambda h: (h[0] - h[1], h[0])):   # longest first; drop overlaps and duplicates
            if not any(h[0] < k[1] and k[0] < h[1] for k in kept):
                kept.append(h)
        for s0, s1, g, repl in sorted(kept):
            add("banned-word", a + s0, a + s1, g, f"use: {repl}", so)
        for key, rx in rules["_idioms"]:
            for x in rx.finditer(sm):
                add("idiom", a + x.start(), a + x.end(), x.group(), "say it literally", so)
        for key, rx in rules["_phrasal"]:
            for x in rx.finditer(sm):
                add("phrasal-verb", a + x.start(), a + x.end(), x.group(), f"use a one-word verb for '{key}'", so)
        for x in re.finditer(r"\b(am|is|are|was|were|be|been|being|get|gets|got|gotten)\s+(?:" + ADV + r"\s+)?"
                             r"([A-Za-z]{2,}ed|" + "|".join(IRREG_PART) + r")\b", sm, re.I):
            if x.group(2).lower() in NOT_PART:
                continue
            add("passive", a + x.start(), a + x.end(), x.group(), "use the active voice (say who or what does it)", so)
        first = re.match(r"\W*([A-Za-z]+ing)\b", sm)
        if first and first.group(1).lower() not in ING_ALLOW and len(first.group(1)) >= 5:
            add("ing-form", a + first.start(1), a + first.end(1), first.group(1),
                "do not start with an -ing word; use 'When you ...' or 'To ...'", so)
        for x in re.finditer(r"\b" + ING_PREP + r"\s+([A-Za-z]{3,}ing)\b", sm, re.I):
            if x.group(1).lower() in ING_ALLOW:
                continue
            add("ing-form", a + x.start(), a + x.end(), x.group(), "use a verb clause ('when you X', 'to X') or a noun", so)
        for x in CONTRACTION.finditer(sm):
            add("contraction", a + x.start(), a + x.end(), x.group(), "write the full form (do not, it is, ...)", so)
    return out, m


def term_findings(units, rules, file):
    """units: list of (loc, masked_text). One finding per synonym group that the page mixes."""
    sev = rules.get("severity", {}).get("inconsistent-terms", "warn")
    out = []
    for grp in rules["_terms"]:
        hits = defaultdict(list)
        for loc, m in units:
            for t, rx in grp:
                for x in rx.finditer(m):
                    L = dict(loc)
                    if "line_of" in L:
                        L["line"] = L.pop("line_of")(x.start())
                    hits[t].append({k: v for k, v in L.items() if k in ("selector", "line")})
        used = {t: v for t, v in hits.items() if v}
        if len(used) >= 2:
            top = max(used, key=lambda t: len(used[t]))
            out.append({"rule": "inconsistent-terms", "severity": sev,
                        "text": ", ".join(f"{t} x{len(v)}" for t, v in sorted(used.items(), key=lambda kv: -len(kv[1]))),
                        "suggestion": f"use one term for one thing: '{top}' (most used), or say how they differ",
                        "locations": {t: v[:5] for t, v in used.items() if t != top},
                        **({"selector": "(page)"} if not file.endswith(".md") else {"line": 0})})
    return out


# ---------------------------------------------------------------- markdown
def md_units(text):
    """(start_line, text, ordered) per paragraph / list item / heading / table cell, with code and markup removed."""
    lines = text.split("\n")
    units, buf, buf_start, ordered, fence = [], [], None, False, False

    def flush():
        nonlocal buf, buf_start, ordered
        if buf:
            units.append((buf_start, "\n".join(buf), ordered))
        buf, buf_start, ordered = [], None, False

    for i, raw in enumerate(lines, 1):
        s = raw.rstrip()
        if re.match(r"\s*(```|~~~)", s):
            fence = not fence
            flush()
            continue
        if fence or re.match(r"( {4,}|\t)\S", raw) and not buf:
            flush()
            continue
        if not s.strip() or re.match(r"\s*([-*_]\s*){3,}$", s) or re.match(r"\s*\|?\s*:?-{3,}", s):
            flush()
            continue
        s = clean_md_inline(s)
        if "|" in raw and raw.strip().startswith("|"):
            flush()
            for cell in s.strip().strip("|").split("|"):
                if cell.strip():
                    units.append((i, cell.strip(), False))
            continue
        h = re.match(r"\s*#{1,6}\s+(.*)", s)
        li = re.match(r"\s*([-*+]|\d+[.)])\s+(.*)", s)
        if h:
            flush()
            units.append((i, h.group(1), False))
        elif li:
            flush()
            buf, buf_start, ordered = [li.group(2)], i, li.group(1)[0].isdigit()
        else:
            if not buf:
                buf_start = i
            buf.append(re.sub(r"^\s*>\s?", "", s))
    flush()
    return units


def clean_md_inline(s):
    s = re.sub(r"`[^`]*`", lambda x: "_" * max(1, len(x.group()) - 2), s)
    s = re.sub(r"!\[([^\]]*)\]\([^)]*\)", lambda x: " " * len(x.group()), s)
    s = re.sub(r"\[([^\]]*)\]\([^)]*\)", lambda x: x.group(1) + " " * (len(x.group()) - len(x.group(1))), s)
    s = re.sub(r"<[^>]+>", lambda x: " " * len(x.group()), s)
    s = re.sub(r"(\*\*|__|\*|~~)", lambda x: " " * len(x.group()), s)
    return s


def check_md(path, rules):
    text = Path(path).read_text(errors="ignore")
    findings, units = [], []
    for start, utext, ordered in md_units(text):
        line_of = (lambda st, t: (lambda pos: st + t[:pos].count("\n")))(start, utext)
        # wrapped lines are one paragraph: newlines are spaces for the checks (same length, so line_of still works)
        f, m = check_unit(utext.replace("\n", " "), rules, {}, ordered=ordered, line_of=line_of)
        findings += f
        units.append(({"line_of": line_of}, m))
    findings += term_findings(units, rules, str(path))
    return findings



DASH = re.compile(r"[–—]")


def extra_findings(path, rules):
    """dash, ai-tell and exclamation, on the same prose units (code and URLs masked)."""
    sev = rules.get("severity", {})
    text = Path(path).read_text(errors="ignore")
    out = []
    tells = [(k, _phrase_re(k)) for k in rules.get("ai_tells", [])]
    for start, utext, ordered in md_units(text):
        line_of = (lambda st, t: (lambda pos: st + t[:pos].count("\n")))(start, utext)
        flat = utext.replace("\n", " ")
        m = mask(flat, rules)
        for x in DASH.finditer(re.sub(r"https?://\S+", lambda u: "_" * len(u.group()), flat)):   # numbers stay: "3–5" is a dash too
            out.append({"line": line_of(x.start()), "rule": "dash", "severity": sev.get("dash", "error"),
                        "text": excerpt(flat[max(0, x.start() - 40):x.end() + 40]),
                        "suggestion": "use a comma, a colon or a full stop"})
        for k, rx in tells:
            for x in rx.finditer(m):
                out.append({"line": line_of(x.start()), "rule": "ai-tell", "severity": sev.get("ai-tell", "error"),
                            "text": excerpt(flat[max(0, x.start() - 30):x.end() + 30]),
                            "suggestion": f"remove '{k}' or say the fact plainly"})
        for x in re.finditer(r"[A-Za-z)\]]!(?=\s|$)", m):
            out.append({"line": line_of(x.start()), "rule": "exclamation", "severity": sev.get("exclamation", "warn"),
                        "text": excerpt(flat[max(0, x.start() - 40):x.end() + 1]), "suggestion": "end with a full stop"})
    return out


def changed_lines(ref, path):
    """line numbers (1-based) added or changed in path since ref; None = the whole file is new."""
    r = subprocess.run(["git", "diff", "-U0", ref, "--", str(path)], capture_output=True, text=True)
    if r.returncode != 0:
        return None
    if not r.stdout.strip():
        known = subprocess.run(["git", "cat-file", "-e", f"{ref}:{path}"], capture_output=True)
        return None if known.returncode != 0 else set()
    lines = set()
    for m in re.finditer(r"^@@ -\S+ \+(\d+)(?:,(\d+))? @@", r.stdout, re.M):
        a, n = int(m.group(1)), int(m.group(2) or 1)
        lines.update(range(a, a + n))
    return lines


def lint(path, rules, only=None):
    fs = check_md(path, rules) + extra_findings(path, rules)
    if only is not None:
        fs = [f for f in fs if f.get("line") in only]
    by = Counter(f["rule"] for f in fs)
    return {"file": str(path), "errors": sum(f["severity"] == "error" for f in fs),
            "warnings": sum(f["severity"] == "warn" for f in fs), "by_rule": dict(by), "findings": fs}


def summary(results, max_show=40):
    lines = []
    for r in results:
        if not r["findings"]:
            continue
        lines.append(f"== {r['file']}: {r['errors']} errors, {r['warnings']} warnings  "
                     + ", ".join(f"{k} {v}" for k, v in sorted(r["by_rule"].items(), key=lambda kv: -kv[1])))
        fs = sorted(r["findings"], key=lambda f: (f.get("line", 0), f["severity"] != "error"))
        for f in fs[:max_show]:
            lines.append(f"  {f['file'] if 'file' in f else r['file']}:{f.get('line', '?')} [{f['severity']}] {f['rule']}: "
                         f"\"{f['text']}\" -> {f['suggestion']}")
        if len(fs) > max_show:
            lines.append(f"  ... {len(fs) - max_show} more (use --json)")
    te, tw = sum(r["errors"] for r in results), sum(r["warnings"] for r in results)
    lines.append(f"ste lint: {len(results)} file(s), {te} errors, {tw} warnings")
    return "\n".join(lines)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("files", nargs="*", help=".md files (default with --changed: every .md file changed since the ref)")
    ap.add_argument("--changed", metavar="REF", help="check only lines added or changed since this git ref")
    ap.add_argument("--rules", default=str(DEFAULT_RULES))
    ap.add_argument("--fail-on", choices=["error", "warn"], help="exit 1 on a finding at or above this severity")
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--max-show", type=int, default=40)
    a = ap.parse_args(argv)
    rules = load_rules(a.rules)
    files = [Path(f) for f in a.files]
    if a.changed and not files:
        r = subprocess.run(["git", "diff", "--name-only", "--diff-filter=ACMR", a.changed, "--", "*.md"],
                           capture_output=True, text=True, check=True)
        files = [Path(f) for f in r.stdout.split() if Path(f).exists()]
    skip = [re.compile(p) for p in rules.get("skip_paths", [])]
    files = [f for f in files if not any(p.search(str(f)) for p in skip)]
    res = []
    for f in files:
        only = changed_lines(a.changed, f) if a.changed else None
        res.append(lint(f, rules, only))
    print(json.dumps(res, indent=1, ensure_ascii=False) if a.json else summary(res, a.max_show))
    if a.fail_on == "error":
        return 1 if any(r["errors"] for r in res) else 0
    if a.fail_on == "warn":
        return 1 if any(r["errors"] or r["warnings"] for r in res) else 0
    return 0


if __name__ == "__main__":
    sys.exit(main())
