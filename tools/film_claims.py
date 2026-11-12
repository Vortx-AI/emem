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

Exit 0 when every claim passes, 1 when one fails, 2 on bad input. Reads emem.dev and nothing else.
"""
import json, re, subprocess, sys, tempfile, urllib.request
from pathlib import Path

EMEM = "https://emem.dev"
TREE_PROOF = Path(__file__).resolve().parent / "verifiers" / "emem-tokenise-files" / "tree_proof.py"
FACT = re.compile(r"^emem:fact:[A-Za-z0-9.]+:[a-z2-7]{20,64}$")
TREE = re.compile(r"^emem:tree:([a-z2-7]{20,64})#row=(\d{1,6})$")
NOTE_PATH = re.compile(r"^/memories/by_attester/[a-z2-7]{8}/[A-Za-z0-9._/-]{1,200}\.md$")


def get(url, data=None):
    req = urllib.request.Request(url, data=json.dumps(data).encode() if data is not None else None,
                                 headers={"content-type": "application/json", "user-agent": "emem-film-claims"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return r.read()


def num(x):
    try:
        return float(str(x).replace(",", "").replace(" ", "").strip().rstrip("%"))
    except ValueError:
        return None


def same(shown, signed):
    """The value on screen is the signed value rounded to the precision shown: 3539.04 for 3539.0422 passes,
    1,285 for 1284 does not. A tolerance would let a wrong count through."""
    a, b = num(shown), num(signed)
    if a is None or b is None:
        return str(shown).strip() == str(signed).strip()
    digits = re.search(r"\.(\d+)", str(shown).replace(",", ""))
    places = len(digits.group(1)) if digits else 0
    return round(b, places) == round(a, places)


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
    return True, f"signed {r.get('value')!r}"


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
        return True, f"note, leaf, root and unit MATCH; unit value {v.group(1).strip() if v else '(none)'}"


def main():
    if len(sys.argv) != 2:
        print(__doc__)
        return 2
    try:
        meta = json.loads(Path(sys.argv[1]).read_text())
    except (OSError, ValueError) as e:
        print(f"cannot read film.json: {e}")
        return 2
    claims, bad = meta.get("claims") or [], 0
    if not claims:
        print("FAIL no claims in film.json")
        return 1
    for c in claims:
        tok = str(c.get("token", ""))
        try:
            if FACT.match(tok):
                ok, why = fact(c)
            elif (m := TREE.match(tok)):
                ok, why = tree(c, m)
            else:
                ok, why = False, "not an emem:fact or emem:tree#row token"
        except Exception as e:  # noqa: BLE001
            ok, why = False, f"{type(e).__name__}: {str(e)[:160]}"
        bad += not ok
        print(f"{'pass' if ok else 'FAIL'}  {c.get('text', '')[:48]!r:52} {tok[:72]}  {why}")
    print(f"{len(claims) - bad} of {len(claims)} claims pass")
    return 0 if not bad else 1


if __name__ == "__main__":
    sys.exit(main())
