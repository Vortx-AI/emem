#!/usr/bin/env python3
"""Check every on-screen claim a film's film.json names against emem.dev.

    film_claims.py FILM.json

The half of docs/media/films/STANDARD.md that film_qc.py cannot do offline. For each claim:

  emem:fact:<cell>:<cid>          POST /v1/memory_token/resolve; it must resolve with its cell, and a `value`
                                   in the claim must equal the signed value rounded to the precision shown.
  emem:tree:<file_cid>#row=<i>    a number no single fact holds, pinned as one unit of the author's signed
                                   pointer.v1 index. The claim names the note in `index_note` (a path under
                                   /memories/by_attester/). GET /v1/tree/<file_cid>?row=<i>, the note and the
                                   source it indexes (on emem.dev only), then tools/verifiers/.../tree_proof.py
                                   check: note, leaf, root and unit must MATCH. Then the unit's `- value:` line
                                   must equal the claim's `value`.

  a computed claim (explainers only)  `reproduce` names a public script that computes the value offline, and
                                   `output` the file it writes; the file must hash to `output_sha256`.

  a pass label                     "Sentinel-2, 30 May 2026, 11:06 UTC" names the picture on screen, and a wrong
                                   date misleads as much as a wrong count. `stac_item` is the item's URL on one
                                   of the two public catalogues below, `shown_utc` the date or minute shown
                                   (2026-05-30 or 2026-05-30T11:06Z). The item's `datetime`, cut to that
                                   precision as a clock shows it, must equal it; the text must name the same day;
                                   and when film.json has segments, an evidence segment must list the item id.
  emem:bundle:<cid> + `rule`        a share or count over an area, such as "72 % of the park reads as snow". The
                                   bundle binds the signed per-cell facts; `rule` is {band, op, threshold} with op
                                   one of ge, gt, le, lt; `statistic` is count, share or percent. The gate resolves
                                   every member, applies the rule and recounts. All members must resolve, read the
                                   rule's band on one date, and carry no erratum. `total`, when shown, must equal
                                   the number of members.
  a conversion                     "99 football pitches" from "71 hectares": `of` is the index of the claim it
                                   converts, `multiply` and `divide` the factors, `formula` the words. The gate
                                   converts the SIGNED value of that claim, not the rounded one on screen.

Exit 0 when every claim passes, 1 when one fails, 2 on bad input. Reads emem.dev, and the catalogue item a pass
label names, and nothing else.
"""
import hashlib, json, re, subprocess, sys, tempfile, urllib.parse, urllib.request
from decimal import ROUND_HALF_EVEN, ROUND_HALF_UP, Decimal, InvalidOperation
from pathlib import Path

EMEM = "https://emem.dev"
REPO = Path(__file__).resolve().parent.parent
TREE_PROOF = Path(__file__).resolve().parent / "verifiers" / "emem-tokenise-files" / "tree_proof.py"
FACT = re.compile(r"^emem:fact:[A-Za-z0-9.]+:[a-z2-7]{20,64}$")
TREE = re.compile(r"^emem:tree:([a-z2-7]{20,64})#row=(\d{1,6})$")
BUNDLE = re.compile(r"^emem:bundle:[a-z2-7]{20,64}$")
RULE_OPS = {"ge": lambda v, t: v >= t, "gt": lambda v, t: v > t, "le": lambda v, t: v <= t, "lt": lambda v, t: v < t}
NOTE_PATH = re.compile(r"^/memories/by_attester/[a-z2-7]{8}/[A-Za-z0-9._/-]{1,200}\.md$")
STAC_HOSTS = ("planetarycomputer.microsoft.com", "earth-search.aws.element84.com")
SHOWN_UTC = re.compile(r"^(\d{4})-(\d{2})-(\d{2})(?:T(\d{2}):(\d{2})Z)?$")
MONTHS = ("jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec")


def get(url, data=None):
    req = urllib.request.Request(url, data=json.dumps(data).encode() if data is not None else None,
                                 headers={"content-type": "application/json", "user-agent": "emem-film-claims"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return r.read()


def num(x):
    """Decimal, not float: 0.125 rounds the same way on screen and here."""
    t = str(x)
    for sep in (",", " ", "\u00a0", "\u202f"):
        t = t.replace(sep, "")
    try:
        return Decimal(t.strip().rstrip("%"))
    except InvalidOperation:
        return None


def same(shown, signed):
    """The value on screen is the signed value rounded to the precision shown: 3539.04 for 3539.0422 passes,
    1,285 for 1284 does not. A tolerance would let a wrong count through. At an exact half either common rule
    (half up, or half to even as Python's format does) passes."""
    a, b = num(shown), num(signed)
    if a is None or b is None or not a.is_finite() or not b.is_finite():
        return str(shown).strip() == str(signed).strip()
    digits = re.search(r"\.(\d+)", str(shown).replace(",", ""))
    q = Decimal(1).scaleb(-(len(digits.group(1)) if digits else 0))
    return a in (b.quantize(q, ROUND_HALF_UP), b.quantize(q, ROUND_HALF_EVEN))


def fact(c):
    r = json.loads(get(f"{EMEM}/v1/memory_token/resolve", {"token": c["token"]}))
    if not r.get("resolved"):
        return False, f"does not resolve: {r.get('message', 'no reason given')}"
    if r.get("cell_matches") is False:
        return False, "resolves only without its cell; cite it as emem:fact:<cell>:<cid>"
    if r.get("erratum"):
        return False, f"carries an erratum ({r['erratum'].get('id')}); cite the fact in replaced_by"
    if "value" in c and not same(c["value"], r.get("value")):
        return False, f"shown {c['value']!r}, signed {r.get('value')!r}"
    v = r.get("value")
    if isinstance(v, (dict, list)):
        return True, "signed record (a structure; no value compared)"
    return True, f"signed {v!r}", v


def tree(c, m):
    cid, row = m.group(1), int(m.group(2))
    note_path = str(c.get("index_note", ""))
    if not NOTE_PATH.match(note_path):
        return False, "a tree claim needs `index_note`, the path of its pointer.v1 note under /memories/by_attester/"
    with tempfile.TemporaryDirectory() as d:
        d = Path(d)
        (d / "row.json").write_bytes(get(f"{EMEM}/v1/tree/{cid}?row={row}"))
        note = get(f"{EMEM}{note_path}")
        (d / "note.md").write_bytes(note)
        src = re.search(rb"^source:\s*(\S+)", note, re.M)
        if not src or not src.group(1).decode().startswith(EMEM + "/memories/"):
            return False, "the note's source is not a note on emem.dev, so its bytes cannot be read here"
        body = get(src.group(1).decode())
        (d / "source.md").write_bytes(body)
        r = subprocess.run([sys.executable, str(TREE_PROOF), "check", str(d / "row.json"), str(d / "note.md"),
                            str(d / "source.md")], capture_output=True, text=True)
        lines = [l for l in r.stdout.splitlines() if l.split()[:1] and l.split()[0] in ("note", "leaf", "root", "unit")]
        if r.returncode != 0 or len(lines) != 4 or any("MISMATCH" in l for l in lines):
            return False, "tree_proof: " + " / ".join(" ".join(l.split()[:2]) for l in lines)
        lf = re.search(r"offset=(\d+) length=(\d+)", r.stdout)
        unit = body[int(lf.group(1)):int(lf.group(1)) + int(lf.group(2))].decode("utf-8", "replace")
        v = re.search(r"^- value:\s*(.+)$", unit, re.M)
        if "value" in c:
            if not v:
                return False, "the unit has no `- value:` line to compare"
            if not same(c["value"], v.group(1)):
                return False, f"shown {c['value']!r}, the signed unit says {v.group(1).strip()!r}"
        unit_value = v.group(1).strip() if v else None
        return True, f"note, leaf, root and unit MATCH; unit value {unit_value or '(none)'}", unit_value


def bundle(c):
    rule, stat = c.get("rule") or {}, c.get("statistic")
    op, band, thr = rule.get("op"), rule.get("band"), num(rule.get("threshold"))
    if op not in RULE_OPS or not band or thr is None or stat not in ("count", "share", "percent"):
        return False, "a bundle claim needs `rule` {band, op: ge|gt|le|lt, threshold} and `statistic` count|share|percent"
    if "value" not in c:
        return False, "a bundle claim needs the `value` shown"
    b = json.loads(get(f"{EMEM}/v1/memory_bundle/{c['token']}"))
    cites = b.get("citations") or []
    if not cites or len(cites) > 256:
        return False, f"the bundle has {len(cites)} citations; a share needs 1 to 256"
    if b.get("resolved") != b.get("members") or b.get("members") != len(cites):
        return False, f"{b.get('resolved')} of {b.get('members')} members resolved; a share over part of a set says more than it holds"
    if {x.get("band") for x in cites} != {band}:
        return False, f"the bundle holds {sorted({str(x.get('band')) for x in cites})}, and the rule reads {band}"
    days = {x.get("resolved_tslot") for x in cites}
    if len(days) != 1:
        return False, f"the members span {len(days)} dates; a share is of one date"
    hits = 0
    for x in cites:
        r = json.loads(get(f"{EMEM}/v1/memory_token/resolve", {"token": x["memory_token"]}))
        if not r.get("resolved") or r.get("cell_matches") is False:
            return False, f"member {x['memory_token'][:60]} does not resolve with its cell"
        if r.get("erratum"):
            return False, f"member at {x.get('cell')} carries an erratum ({r['erratum'].get('id')}); bundle its replacement"
        v = num(r.get("value"))
        if v is None or not v.is_finite():
            return False, f"member at {x.get('cell')} has no numeric value ({r.get('value')!r})"
        hits += RULE_OPS[op](v, thr)
    n = len(cites)
    got = {"count": Decimal(hits), "share": Decimal(hits) / n, "percent": Decimal(100 * hits) / n}[stat]
    if "total" in c and num(c["total"]) != n:
        return False, f"shown total {c['total']!r}, the bundle has {n} members"
    rule_txt = f"{band} {op} {rule.get('threshold')}"
    if not same(c["value"], got):
        return False, f"shown {c['value']!r}; {hits} of {n} members meet {rule_txt}, so the {stat} is {got:.6g}"
    return True, f"{hits} of {n} members meet {rule_txt} on one date: {stat} {got:.6g}"


def label(c, meta):
    url = str(c.get("stac_item", ""))
    u = urllib.parse.urlparse(url)
    if u.scheme != "https" or u.hostname not in STAC_HOSTS:
        return False, "a pass label needs `stac_item`, the https URL of the item on " + " or ".join(STAC_HOSTS)
    m = SHOWN_UTC.match(str(c.get("shown_utc", "")))
    if not m:
        return False, "a pass label needs `shown_utc`, the date or minute shown: 2026-05-30 or 2026-05-30T11:06Z"
    y, mo, d, hh, mi = m.groups()
    if not 1 <= int(mo) <= 12:
        return False, f"`shown_utc` has no month {mo}"
    item = json.loads(get(url))
    dt = str((item.get("properties") or {}).get("datetime") or "")
    want = f"{y}-{mo}-{d}" + (f"T{hh}:{mi}" if hh else "")
    if not dt.startswith(want):
        return False, f"shown {want}, the catalogue item says {dt or 'no datetime'}"
    text = str(c.get("text", "")).lower()
    named = y in text and re.search(rf"(?<!\d)0?{int(d)}(?!\d)", text) and (MONTHS[int(mo) - 1] in text or f"-{mo}-" in text)
    if not named or (hh and f"{hh}:{mi}" not in text):
        return False, f"the text does not say {want}; `shown_utc` must be what the screen shows"
    segs = meta.get("segments") or []
    listed = {str(s) for g in segs if g.get("role") == "evidence" for s in (g.get("scenes") or [])}
    if segs and item.get("id") not in listed:
        return False, f"no evidence segment lists {item.get('id')!r}; name scenes by their catalogue item id"
    return True, f"catalogue datetime {dt}"


def convert(c, i, claims, signed):
    of, mul, div = c.get("of"), num(c.get("multiply", 1)), num(c.get("divide", 1))
    if not isinstance(of, int) or isinstance(of, bool) or not 0 <= of < len(claims) or of == i:
        return False, "a conversion needs `of`, the index of another claim in this film.json"
    if mul is None or div is None or not mul.is_finite() or not div.is_finite() or div == 0:
        return False, "`multiply` and `divide` must be finite numbers, `divide` not 0"
    if not str(c.get("formula", "")).strip():
        return False, "a conversion needs `formula`, the arithmetic in words"
    base = signed.get(of)
    lead = re.match(r"\s*([-+]?[\d,]*\.?\d+(?:[eE][-+]?\d+)?)", str(base)) if base is not None else None
    b = num(lead.group(1)) if lead else None
    if b is None:
        return False, f"claim {of} passed with no numeric signed value, so there is nothing to convert"
    if "value" not in c:
        return False, "a conversion needs the `value` shown"
    v = b * mul / div
    if not same(c["value"], v):
        return False, f"shown {c['value']!r}; {c['formula']} on the signed {base} gives {v:.8g}"
    return True, f"{c['formula']} on the signed {base} gives {v:.8g}"


def main():
    if len(sys.argv) != 2:
        print(__doc__)
        return 2
    try:
        meta = json.loads(Path(sys.argv[1]).read_text())
    except (OSError, ValueError) as e:
        print(f"cannot read film.json: {e}")
        return 2
    claims, bad, signed, results = meta.get("claims") or [], 0, {}, {}
    if not claims:
        print("FAIL no claims in film.json")
        return 1
    # Conversions last: each reads the signed value of the claim it converts.
    order = sorted(range(len(claims)), key=lambda k: "of" in claims[k])
    for i in order:
        c = claims[i]
        tok = str(c.get("token", ""))
        try:
            if "of" in c:
                ok, why = convert(c, i, claims, signed)
            elif c.get("stac_item") or c.get("shown_utc"):
                ok, why = label(c, meta)
            elif not tok and c.get("reproduce") and c.get("output_sha256"):
                out = REPO / str(c.get("output", ""))
                if REPO not in out.resolve().parents or not out.is_file():
                    ok, why = False, "a computed claim needs `output`, a file in this repository"
                else:
                    h = hashlib.sha256(out.read_bytes()).hexdigest()
                    ok = h == c["output_sha256"]
                    why = f"computed by {c['reproduce']}; output {'matches' if ok else 'differs from'} its sha256"
            elif BUNDLE.match(tok):
                ok, why = bundle(c)
            elif FACT.match(tok) or TREE.match(tok):
                m = TREE.match(tok)
                ok, why, *val = tree(c, m) if m else fact(c)
                if ok and val and val[0] is not None:
                    signed[i] = val[0]
            else:
                ok, why = False, "not an emem:fact, emem:tree#row or emem:bundle token, a pass label or a conversion"
        except Exception as e:  # noqa: BLE001
            ok, why = False, f"{type(e).__name__}: {str(e)[:160]}"
        bad += not ok
        results[i] = f"{'pass' if ok else 'FAIL'}  {i:>2} {c.get('text', '')[:46]!r:50} {(tok or c.get('stac_item', ''))[:72]}  {why}"
    for i in range(len(claims)):
        print(results[i])
    print(f"{len(claims) - bad} of {len(claims)} claims pass")
    return 0 if not bad else 1


if __name__ == "__main__":
    sys.exit(main())
