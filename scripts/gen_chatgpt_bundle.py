#!/usr/bin/env python3
"""Generate and validate the ChatGPT submission bundle against the live server.

Why
---
integrations/chatgpt/tools.md documented FOUR tools that do not exist:
`emem_ask_place`, `emem_locate_place`, `emem_recall_facts`, `emem_get_receipt`.
The submission JSON beside it declares nine real ones. A reviewer reading the
two files together would have found the app describing a surface the server
does not serve, and the example CIDs were `bafy...`, which is IPFS's format and
not ours.

Nothing in the bundle was generated, so nothing kept it honest. tools.md is
generated now, from the tool catalogue the responder actually serves, for
exactly the nine tools the submission declares. The rest of the bundle is
prose and stays hand-written, but the FACTS in it are checked here.

Checks (all of them fail loudly rather than warning):
  - every tool the submission declares exists in the live catalogue
  - its readOnlyHint / destructiveHint / openWorldHint match the live server
  - no justification contradicts the flag it justifies
  - the contact address matches the agent card
  - the tool count in the prose matches the live count

Usage:
  python3 scripts/gen_chatgpt_bundle.py
  python3 scripts/gen_chatgpt_bundle.py --check
"""
from __future__ import annotations

import argparse
import json
import pathlib
import re
import sys
import urllib.request

REPO = pathlib.Path(__file__).resolve().parent.parent
BUNDLE = REPO / "integrations" / "chatgpt"
SUBMISSION = BUNDLE / "chatgpt-app-submission.json"
TOOLS_MD = BUNDLE / "tools.md"


def rpc(url: str, method: str, params: dict) -> dict:
    req = urllib.request.Request(
        url, data=json.dumps({"jsonrpc": "2.0", "id": 1,
                              "method": method, "params": params}).encode(),
        headers={"content-type": "application/json",
                 "accept": "application/json, text/event-stream"})
    raw = urllib.request.urlopen(req, timeout=90).read().decode()
    if raw.lstrip().startswith("data:"):
        raw = "\n".join(l[5:].strip() for l in raw.splitlines() if l.startswith("data:"))
    return json.loads(raw)["result"]


def catalogue(origin: str) -> dict:
    """Every tool the responder serves. /mcp advertises the core tier only, so
    this reads /mcp/full and follows nextCursor: reading one page and calling it
    the catalogue is how a tool comes to look absent when it is merely later."""
    out, cursor, pages = {}, None, 0
    while True:
        r = rpc(origin.rstrip("/") + "/mcp/full", "tools/list",
                {} if not cursor else {"cursor": cursor})
        for t in r["tools"]:
            out[t["name"]] = t
        cursor = r.get("nextCursor")
        pages += 1
        if not cursor or pages > 20:
            break
    return out


# How many read-only COUNT claims the last check() call actually examined.
# main() prints it: "0 problems" over 0 claims examined is not the same result
# as "0 problems" over nine, and the two must not print the same line.
CLAIMS_CHECKED = 0


# Validate against the PUBLISHED schema, not against rules typed here.
#
# Two review comments arrived on the same day for the same reason: the subtitle
# was 39 characters against a 30 limit, and every tool carried its annotations
# FLAT when the schema requires an `annotations` object. Both are stated plainly
# in the schema this file has always named in its own `$schema` field, and
# nothing ever read it. A hand-kept list of limits would have caught the first
# and never the second, because the second is a shape and not a number.
#
# The copy is vendored so CI does not depend on that host being up, and the
# fetch below compares the two so a drifted vendor announces itself instead of
# quietly validating against last month's rules.
SCHEMA_PATH = BUNDLE / "submission.schema.json"
SCHEMA_URL = ("https://developers.openai.com/plugins/schemas/"
              "chatgpt-app-submission.v1.json")


def schema_findings(sub: dict) -> list[str]:
    """Every way the submission violates the published schema."""
    try:
        import jsonschema
    except ImportError:
        return ["jsonschema is not installed, so the submission was NOT validated "
                "against its schema this run (pip install jsonschema)"]
    if not SCHEMA_PATH.exists():
        return [f"{SCHEMA_PATH.name} is missing; the submission was not validated"]
    schema = json.loads(SCHEMA_PATH.read_text(encoding="utf-8"))
    out = []
    try:
        import urllib.request
        with urllib.request.urlopen(SCHEMA_URL, timeout=20) as r:
            live = json.load(r)
        if live != schema:
            out.append(f"{SCHEMA_PATH.name} differs from {SCHEMA_URL}; re-vendor it "
                       f"before trusting this validation")
    except Exception:
        pass  # offline is fine: the vendored copy still validates
    for e in sorted(jsonschema.Draft202012Validator(schema).iter_errors(sub),
                    key=lambda e: list(e.path)):
        where = ".".join(str(x) for x in e.path) or "<root>"
        out.append(f"schema: {where}: {e.message}")
    return out


def check(sub: dict, live: dict, card: dict) -> list[str]:
    global CLAIMS_CHECKED
    bad = []
    for name, decl in sub["tools"].items():
        t = live.get(name)
        if not t:
            bad.append(f"{name}: declared in the submission, not served by the responder")
            continue
        ann = t.get("annotations") or {}
        # The published schema nests these: {"annotations": {...},
        # "justifications": {read_only_justification, ...}}. The bundle carried
        # them FLAT, which is why a reviewer had to tell us "emem_ask must
        # include an annotations object" -- the file had the values and not the
        # shape, and nothing here compared it to the schema that says so.
        d_ann = decl.get("annotations") or {}
        d_just = decl.get("justifications") or {}
        just_key = {
            "readOnlyHint": "read_only_justification",
            "destructiveHint": "destructive_justification",
            "openWorldHint": "open_world_justification",
        }
        for flag in ("readOnlyHint", "destructiveHint", "openWorldHint"):
            if flag in d_ann and flag in ann and d_ann[flag] != ann[flag]:
                bad.append(f"{name}: declares {flag}={d_ann[flag]}, server says {ann[flag]}")
        # a justification that contradicts the flag beside it
        for flag in ("readOnlyHint", "destructiveHint", "openWorldHint"):
            for j in (d_just.get(just_key[flag]), ):
                if not j:
                    continue
                for other in ("readOnlyHint", "destructiveHint"):
                    m = re.search(rf"{other} is (true|false)", j)
                    if m and d_ann.get(other) is not (m.group(1) == "true"):
                        bad.append(f"{name}: {just_key[flag]} says "
                                   f"'{other} is {m.group(1)}' but {other} is {d_ann.get(other)}")
    # Every COUNT the bundle states about read-only tools, checked against the
    # live annotations.
    #
    # This is the hole the scope line used to name and not fill. On 2026-08-27
    # the prose said "Seven of the nine tools are strictly read-only" while the
    # annotations said four, and the same wrong sentence sat in SEVEN
    # openWorldHint_justifications inside the submission JSON -- three of them
    # on tools that are themselves among the five that can add state, so the
    # sentence contradicted itself in place. The flags were right the whole
    # time and agreed with the server; only the sentences counting them were
    # wrong, and nothing compared a sentence to a flag.
    #
    # Scanned over every FILE in the bundle, not *.md, because the worst copies
    # were in the .json.
    ro_true = sum(1 for n in sub["tools"]
                  if ((live.get(n) or {}).get("annotations") or {}).get("readOnlyHint") is True)
    total = len(sub["tools"])
    rw_true = total - ro_true
    claims = 0

    def _num(tok: str):
        words = {"one": 1, "two": 2, "three": 3, "four": 4, "five": 5,
                 "six": 6, "seven": 7, "eight": 8, "nine": 9, "ten": 10}
        tok = tok.lower()
        return words.get(tok, int(tok) if tok.isdigit() else None)

    for f in sorted(x for x in BUNDLE.iterdir() if x.is_file()):
        text = f.read_text(encoding="utf-8")
        for m in re.finditer(r"(\w+)(?:\s+of the (\w+) tools?)?[^.]{0,80}?strictly read-only",
                             text, re.I):
            n = _num(m.group(1))
            if n is None:
                continue
            claims += 1
            if n != ro_true:
                bad.append(f"{f.name}: says {m.group(1)} tools are strictly read-only, "
                           f"the live annotations say {ro_true}")
            t = _num(m.group(2)) if m.group(2) else None
            if t is not None and t != total:
                bad.append(f"{f.name}: says 'of the {m.group(2)} tools', "
                           f"the submission declares {total}")
        # "the other five ... materialise" -- only where it is plainly about
        # the writing half, so an unrelated "the other two" cannot be accused.
        for m in re.finditer(r"the other (\w+)[^.]{0,160}?(materialis|readOnlyHint|add state)",
                             text, re.I):
            n = _num(m.group(1))
            if n is None:
                continue
            claims += 1
            if n != rw_true:
                bad.append(f"{f.name}: says the other {m.group(1)} can add state, "
                           f"the live annotations say {rw_true}")

    CLAIMS_CHECKED = claims
    if claims == 0:
        bad.append("no read-only COUNT claim was found in any bundle file, and these "
                   "files do state one; the wording moved and this check is now "
                   "reading nothing")

    bad.extend(schema_findings(sub))

    contact = (card.get("emem") or {}).get("contact")
    for f in sorted(BUNDLE.glob("*.md")):
        text = f.read_text(encoding="utf-8")
        for addr in set(re.findall(r"[\w.+-]+@[\w.-]+\.\w+", text)):
            if contact and addr != contact:
                bad.append(f"{f.name}: contact {addr} does not match the agent card ({contact})")
    return bad


def render(sub: dict, live: dict, origin: str) -> str:
    lines = [
        "# Tools",
        "",
        "<!-- GENERATED by scripts/gen_chatgpt_bundle.py from the tool catalogue this",
        "     responder actually serves. Do not edit by hand: run the script.",
        "",
        "     This file previously documented four tools that do not exist",
        "     (emem_ask_place, emem_locate_place, emem_recall_facts, emem_get_receipt)",
        "     while the submission JSON beside it declared nine real ones. Nothing",
        "     generated it, so nothing kept it true. -->",
        "",
        f"The app declares **{len(sub['tools'])} tools**. Each one below is checked against "
        f"`{origin}/mcp/full` at generation time: the name exists, and the MCP annotations "
        "here are the annotations the server sends.",
        "",
        "Reads need no key and no account. None of emem's write verbs is exposed in this app.",
        "",
    ]
    for name in sub["tools"]:
        t = live[name]
        decl = sub["tools"][name]
        ann = t.get("annotations") or {}
        schema = t.get("inputSchema") or {}
        props = schema.get("properties") or {}
        required = schema.get("required") or []
        desc = " ".join((t.get("description") or "").split())
        if len(desc) > 600:
            desc = desc[:597].rsplit(" ", 1)[0] + "…"
        lines += [f"## `{name}`", "", desc, ""]
        ro = ann.get("readOnlyHint")
        lines += [
            f"**Read-only:** {'yes' if ro else 'no'}. "
            + (" ".join((decl.get("readOnlyHint_justification") or "").split())[:400]
               if not ro else "It reads and returns; it adds nothing another reader would see."),
            "",
        ]
        if props:
            # Required keys first: the first version listed six optional fields
            # and omitted the one required field directly above a line saying it
            # was required, which is a worked example that does not work.
            order = [k for k in required if k in props] + \
                    [k for k in props if k not in required]
            body = []
            for k in order[:7]:
                spec = props[k] or {}
                ex = spec.get("example")
                if ex is None:
                    ex = {"string": f"<{k}>", "number": 0, "integer": 0,
                          "boolean": False, "array": [], "object": {}}.get(spec.get("type"), f"<{k}>")
                # ensure_ascii=False, or a placeholder renders as \u2026 in a code block
                body.append((k, json.dumps(ex, ensure_ascii=False), k in required))
            width = max(len(json.dumps(k, ensure_ascii=False)) + len(v) for k, v, _ in body)
            rows = []
            for i, (k, v, req) in enumerate(body):
                pair = f'{json.dumps(k, ensure_ascii=False)}: {v}'
                comma = "," if i < len(body) - 1 else ""
                note = "" if req else f'{" " * max(1, width + 2 - len(pair) - len(comma))}// optional'
                rows.append(f"  {pair}{comma}{note}")
            lines += ["**Input**", "", "```json", "{"] + rows + ["}", "```", ""]
            if required:
                lines += ["Required: " + ", ".join(f"`{r}`" for r in required), ""]
        lines += ["---", ""]
    lines += [
        "## Verifying any answer",
        "",
        "Every response carries an ed25519 receipt. `emem_verify_receipt` checks it against "
        f"the responder's published key at `{origin}/.well-known/emem.json`, so an answer can "
        "be checked without trusting the responder that produced it. Fact identifiers are "
        "base32 blake3 content addresses over canonical CBOR; they are not IPFS CIDs and do "
        "not begin with `bafy`.",
        "",
    ]
    return "\n".join(lines)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--origin", default="https://emem.dev")
    ap.add_argument("--check", action="store_true")
    a = ap.parse_args()

    sub = json.loads(SUBMISSION.read_text(encoding="utf-8"))
    try:
        live = catalogue(a.origin)
        card = json.loads(urllib.request.urlopen(
            a.origin.rstrip("/") + "/.well-known/agent-card.json", timeout=45).read().decode())
    except Exception as e:
        print(f"could not read the responder at {a.origin}: {type(e).__name__}: {e}")
        print("Undetermined, not clean: this bundle is validated AGAINST the server,")
        print("so an unreachable server means nothing here was checked.")
        return 2

    problems = check(sub, live, card)
    if problems:
        print(f"the submission bundle disagrees with {a.origin}:")
        for p in problems:
            print("  ", p)
        return 1

    body = render(sub, live, a.origin)
    if a.check:
        if not TOOLS_MD.exists() or TOOLS_MD.read_text(encoding="utf-8") != body:
            print("integrations/chatgpt/tools.md is stale; run scripts/gen_chatgpt_bundle.py")
            return 1
        print(f"bundle agrees with {a.origin}: {len(sub['tools'])} tools declared, all served, "
              f"annotations match, no contradictory justifications, contact matches the card")
        print(f"  scope: the {len(sub['tools'])} tools this submission DECLARES, checked against")
        print(f"  the live catalogue, plus tools.md and the e-mail addresses in "
              f"{len(list(BUNDLE.glob('*.md')))} .md file(s).")
        print(f"  Cross-checked {CLAIMS_CHECKED} read-only COUNT claim(s) across every file "
              f"in the bundle against the live annotations.")
        print(f"  Validated against {SCHEMA_PATH.name}, the published submission schema: "
              f"shape, required fields and every length limit it states.")
        print("  NOT covered: the rest of the prose, whether the declared set is the")
        print("  right set, and the domain-verification token, which the portal")
        print("  reissues per submission and no check here can know.")
        return 0
    TOOLS_MD.write_text(body, encoding="utf-8")
    print(f"wrote {TOOLS_MD.relative_to(REPO)} from {len(live)} live tools "
          f"({len(sub['tools'])} declared)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
