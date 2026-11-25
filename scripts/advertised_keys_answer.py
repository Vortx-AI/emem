#!/usr/bin/env python3
"""Every surface that tells an agent what it can ask names only keys that answer.

Why this exists
---------------
emem.dev says "Every answer is signed." On 2026-11-20, `GET /v1/bands` listed
49 registered scalar keys with no reader (`scalar_keys_unwired`), and the
surfaces an agent reads to choose a key still named some of them as if they
answered: `emem_soil` and the `/v1/soil` OpenAPI text promised silt, a skill
told agents to read `ghsl.built_surface_m2`, and the served tool text gave the
retired `geotessera` as a default with no word that it was retired. A recall of
such a key then came back empty, and the promise broke on the first call.

`/v1/bands` already says which keys answer. This gate holds every other surface
to it: a sentence that names a key from `scalar_keys_unwired` (or a band that is
retired here) must also say that it does not answer, or the same paragraph must
carry a sentence that names that key, or its bare band, and says so.

What it reads
-------------
From the responder (the one `--origin` names): the MCP `initialize`
instructions and every tool from `tools/list` on /mcp/full and from
/v1/tools, the description
and summary strings of /openapi.json, and every string of /v1/sources,
/v1/capabilities, /v1/materializers (all pages) and the `data_at_this_cell`
block of /v1/locate. From the tree: docs/agents.md, web/skills.md and the
plugin skills under plugins/emem/skills/.

What it does not read
---------------------
Fenced code blocks and HTML lines in markdown: they are requests and records,
not claims about what answers. /v1/bands itself, which is the source of truth.
Prose that names a quantity without its key ("silt"): a word list cannot tell a
claim from a mention, so those were read by a person (see the commit that added
this gate).

    python3 scripts/advertised_keys_answer.py
    python3 scripts/advertised_keys_answer.py --origin http://127.0.0.1:5112

Exit: 0 every mention is marked, 1 a surface names a key that does not answer
without saying so, 2 the responder could not be reached, 3 the self-test failed
(the checker would pass anything).
"""

from __future__ import annotations

import argparse
import glob
import json
import os
import re
import sys
import urllib.error
import urllib.request

from lib_patience import patient

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_ORIGIN = os.environ.get("EMEM_RESPONDER") or os.environ.get("EMEM_ORIGIN") or "https://emem.dev"

# A sentence says the key does not answer when it carries one of these.
MARKER = re.compile(
    r"retire|withdrawn|no reader|not wired|unwired|no longer|does not answer|do not answer|"
    r"not answer|no materializer|band_not_wired|no_materializer|stored facts only|stored vectors|"
    r"only from stored|signed before|frozen|does not serve|not serve|no new (?:fact|ones|vector|embedding)|"
    r"declared_no_connector|no connector|has no reader",
    re.I,
)
# JSON fields that hold a name or an address, not a claim about what answers.
ID_FIELDS = {"scheme", "id", "url", "url_template", "upstream_scheme", "upstream_endpoint",
             "derivation_fn_key", "fn_key", "license", "kind", "auth", "responder_pubkey_b32"}
# Words in a JSON path that mean the list it holds is the list of keys that do not answer.
PATH_MARKER = re.compile(r"unwired|not_wired|retired|no_reader|declared_but_no|withdrawn|missing|unknown", re.I)
SENTENCE = re.compile(r"(?<=[.!?])\s+")
PLACE = {"lat": 18.5196, "lng": 73.8553}  # Pune


def key_pattern(keys: list[str]) -> re.Pattern:
    alts = "|".join(re.escape(k) for k in sorted(keys, key=len, reverse=True))
    return re.compile(r"(?<![\w.])(" + alts + r")(?![\w])")


def root_named(sentence: str, root: str) -> bool:
    """The bare band name, not a dotted key under it."""
    return re.search(r"(?<![\w.])" + re.escape(root) + r"(?![\w.])", sentence) is not None


def split_sentences(text: str) -> list[str]:
    return [s for s in SENTENCE.split(text) if s.strip()]


def check_sentences(sentences: list[str], pat: re.Pattern) -> list[tuple[str, str]]:
    """(key, sentence) for every unmarked mention of a key in one unit."""
    declared: set[str] = set()
    for s in sentences:
        if MARKER.search(s):
            for m in pat.finditer(s):
                declared.add(m.group(1))
    out = []
    for s in sentences:
        if MARKER.search(s):
            continue
        for m in pat.finditer(s):
            k = m.group(1)
            root = k.split(".")[0]
            if k in declared:
                continue
            if any(MARKER.search(o) and root_named(o, root) for o in sentences):
                continue
            out.append((k, " ".join(s.split())[:240]))
    return out


def check_unit(text: str, pat: re.Pattern) -> list[tuple[str, str]]:
    return check_sentences(split_sentences(text), pat)


def self_test() -> bool:
    pat = key_pattern(["landcover.crop", "geotessera", "geotessera.bin128"])
    must_fail = [
        "Use landcover.crop to read the crop fraction.",
        "The default band is `geotessera`.",
        "Pass `geotessera.bin128` for speed. Recall indices.ndvi.",
    ]
    must_pass = [
        "landcover.crop is registered, no reader yet.",
        "The default band is `geotessera`. At this responder `geotessera` is retired.",
        "Pass `geotessera.bin128` for speed. On emem.dev `geotessera` is retired.",
        "Read landcover.cropland or xlandcover.crop, which are other names.",
    ]
    return all(check_unit(t, pat) for t in must_fail) and not any(check_unit(t, pat) for t in must_pass)


# ---------------------------------------------------------------- reading


def _req(origin: str, path: str, body: dict | None = None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(
        origin + path, data=data,
        headers={"Content-Type": "application/json", "Accept": "application/json, text/event-stream"},
    )
    with patient(req, timeout=120) as r:
        return json.load(r)


def strings(node, path=""):
    if isinstance(node, dict):
        for k, v in node.items():
            yield from strings(v, f"{path}/{k}")
    elif isinstance(node, list):
        for i, v in enumerate(node):
            yield from strings(v, f"{path}/{i}")
    elif isinstance(node, str):
        yield path, node


def json_findings(doc, pat, surface: str):
    """Each string is a unit. A string that IS a key (a list entry) passes only
    when its path, or a string beside it, says the list does not answer."""
    out = []

    def walk(node, path, parent):
        if isinstance(node, dict):
            for k, v in node.items():
                walk(v, f"{path}/{k}", node)
        elif isinstance(node, list):
            for i, v in enumerate(node):
                walk(v, f"{path}/{i}", parent)
        elif isinstance(node, str):
            if path.rsplit("/", 1)[-1] in ID_FIELDS or re.match(r"^[a-z0-9]+://", node):
                return
            full = pat.fullmatch(node.strip("`"))
            if full:
                beside = " ".join(v for v in (parent or {}).values() if isinstance(v, str))
                if not (PATH_MARKER.search(path) or MARKER.search(beside)):
                    out.append((surface, path, full.group(1), node))
                return
            for k, s in check_unit(node, pat):
                out.append((surface, path, k, s))

    walk(doc, "", None)
    return out


def markdown_units(text: str):
    """Paragraphs, list items and table rows as lists of sentences, with line
    numbers. A table row is one sentence, read with the paragraph above the
    table, which is what introduces it."""
    units, buf, start, in_code = [], [], 0, False
    lines = text.split("\n")

    def flush():
        if buf:
            units.append((start, split_sentences(" ".join(buf))))
        buf.clear()

    prev_para: tuple[int, str] | None = None
    for i, line in enumerate(lines, 1):
        st = line.strip()
        if st.startswith("```"):
            in_code = not in_code
            flush()
            continue
        if in_code or st.startswith("<"):
            continue
        if not st:
            flush()
            continue
        if st.startswith("|"):
            if set(st) <= set("|-: "):
                continue
            flush()
            # A row is one statement: its cells are fragments of it.
            intro = split_sentences(prev_para[1]) if prev_para else []
            units.append((i, intro + [" ".join(st.replace("|", " ").split())]))
            continue
        if re.match(r"^([-*+]|\d+\.)\s", st):
            flush()
            start = i
        if not buf:
            start = i
        buf.append(st)
        prev_para = (start, " ".join(buf))
    flush()
    return units


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--origin", default=DEFAULT_ORIGIN)
    args = ap.parse_args()
    origin = args.origin.rstrip("/")

    if not self_test():
        print("self-test failed: the checker passes a sentence it must flag, or flags one it must pass")
        return 3

    try:
        bands = _req(origin, "/v1/bands")
    except (urllib.error.URLError, OSError) as e:
        print(f"responder {origin} unreachable ({e}); nothing asserted")
        return 2

    keys: set[str] = set()
    for b in bands.get("bands", []):
        keys.update(b.get("scalar_keys_unwired") or [])
        if (b.get("materializer") or {}).get("kind") == "retired":
            keys.add(b["key"])
    if not keys:
        print("/v1/bands lists no unwired key; nothing to hold the surfaces to")
        return 0
    pat = key_pattern(sorted(keys))

    findings: list[tuple[str, str, str, str]] = []
    units = 0

    # MCP: the instructions, then each tool as one unit (description and every
    # string of its input schema).
    try:
        init = _req(origin, "/mcp", {"jsonrpc": "2.0", "id": 1, "method": "initialize",
                                     "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                                                "clientInfo": {"name": "advertised-keys-gate", "version": "1"}}})
        instr = (init.get("result") or {}).get("instructions") or ""
        units += 1
        findings += [("mcp instructions", "", k, s) for k, s in check_unit(instr, pat)]
        tools, cursor, seen = [], None, set()
        while True:
            params = {"tier": "all"} if cursor is None else {"tier": "all", "cursor": cursor}
            r = _req(origin, "/mcp/full", {"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": params})["result"]
            tools += r.get("tools") or []
            cursor = r.get("nextCursor")
            if not cursor or cursor in seen:
                break
            seen.add(cursor)
        for t in tools:
            parts = [t.get("title") or "", t.get("description") or ""]
            parts += [s for p, s in strings(t.get("inputSchema") or {}) if p.endswith("/description")]
            units += 1
            text = "\n\n".join(parts)
            findings += [(f"tool {t.get('name')}", "", k, s) for k, s in check_unit(text, pat)]
        tool_count = len(tools)
        # The REST mirror of the same registry, built by another function.
        for t in _req(origin, "/v1/tools?tier=all").get("tools") or []:
            parts = [t.get("title") or "", t.get("description") or "", t.get("when_to_use") or ""]
            parts += [s for p, s in strings(t.get("input_schema") or {}) if p.endswith("/description")]
            units += 1
            text = "\n\n".join(parts)
            findings += [(f"/v1/tools {t.get('name')}", "", k, s) for k, s in check_unit(text, pat)]
    except (urllib.error.URLError, OSError, KeyError) as e:
        print(f"MCP surface unreadable at {origin} ({e})")
        return 2

    # OpenAPI: the prose only.
    spec = _req(origin, "/openapi.json")
    for path, s in strings(spec):
        if path.endswith("/description") or path.endswith("/summary"):
            units += 1
            findings += [("openapi", path, k, t) for k, t in check_unit(s, pat)]

    # JSON surfaces an agent reads to pick a key.
    for path in ("/v1/sources", "/v1/capabilities"):
        doc = _req(origin, path)
        units += sum(1 for _ in strings(doc))
        findings += json_findings(doc, pat, path)
    page, mats = 1, 0
    while True:
        doc = _req(origin, f"/v1/materializers?page={page}&page_size=100")
        mats += len(doc.get("materializers") or [])
        units += sum(1 for _ in strings(doc))
        findings += json_findings(doc, pat, f"/v1/materializers?page={page}")
        if not (doc.get("pagination") or {}).get("has_next"):
            break
        page += 1
    try:
        loc = _req(origin, "/v1/locate", PLACE)
        block, where = loc.get("data_at_this_cell"), "/v1/locate data_at_this_cell"
    except urllib.error.HTTPError:
        block, where = None, ""
    if block is None:
        block, where = _req(origin, "/v1/topics"), "/v1/topics (locate failed; same block)"
    units += sum(1 for _ in strings(block))
    findings += json_findings(block, pat, where)

    # The tree.
    files = [os.path.join(REPO, "docs/agents.md"), os.path.join(REPO, "web/skills.md")]
    files += sorted(glob.glob(os.path.join(REPO, "plugins/emem/skills/**/*.md"), recursive=True))
    for f in files:
        text = open(f, encoding="utf-8").read()
        for line, sentences in markdown_units(text):
            units += 1
            rel = os.path.relpath(f, REPO)
            findings += [(rel, f"line {line}", k, s) for k, s in check_sentences(sentences, pat)]

    print(f"{len(keys)} keys that do not answer at {origin}; {units} units read across "
          f"{tool_count} tools (tools/list and /v1/tools), OpenAPI, 3 JSON surfaces ({mats} materializers), {where.split(' ')[0]} "
          f"and {len(files)} documents")
    if not findings:
        print("advertised_keys_answer: every mention of a key that does not answer says so")
        return 0
    print(f"advertised_keys_answer: {len(findings)} mention(s) of a key that does not answer, unmarked:")
    for surface, loc, k, s in findings:
        print(f"  {surface} {loc}: `{k}`: {s}")
    print("Mark it in the same sentence (\"registered, no reader yet\", \"retired\", \"withdrawn\"), "
          "or name a key that answers. GET /v1/bands lists both per band.")
    return 1


if __name__ == "__main__":
    sys.exit(main())
