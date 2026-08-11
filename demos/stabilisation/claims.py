#!/usr/bin/env python3
"""The claims this repo makes about things git cannot check.

This is the file you edit. Everything else is machinery.

A claim is a sentence plus the code that decides it. `claim` is the prose a
human reads; `probe` is a function that asks the live responder and returns the
answer as a string. `how` is one line saying what the probe does, so a reader
who does not want to read Python still knows what was asked.

Nothing here hardcodes an expected value. The expected value is whatever the
probe returned at the moment the claim was recorded, and that is stored in
assertions.lock.json alongside the content address of the signed record.
Hardcoding it is how you get "163 recipes" surviving four commits past 168.
"""
from __future__ import annotations

import json
import os
import re
import urllib.error
import urllib.request

RESPONDER = os.environ.get("EMEM_RESPONDER", "https://emem.dev").rstrip("/")

# A fact this responder holds, pinned as a citation. One claim quotes its value.
NDVI_CELL = "defi.zb4e3.zaeed.fEya"
NDVI_TOKEN = f"emem:fact:{NDVI_CELL}:qtv2bco56qw4pmlohk56dotoxyl3atmnjpmzrijj2kazw2mj57oq"

# The payload ceiling MCP clients enforce on a single response.
CLIENT_CAP_BYTES = 102_400


def get(path: str, timeout: int = 60):
    with urllib.request.urlopen(RESPONDER + path, timeout=timeout) as r:
        return json.load(r)


def post(path: str, body: dict, timeout: int = 90) -> tuple[int, dict]:
    """POST returning (status, body). A 4xx is data here, not an exception:
    several claims below are claims ABOUT the error the responder returns."""
    req = urllib.request.Request(
        RESPONDER + path, data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json", "Accept": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, json.load(r)
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read().decode() or "{}")


def widest_tools_page(endpoint: str = "/mcp/full") -> str:
    """Walk the whole tools/list cursor chain and report whether every page fits
    the client cap. Returns a verdict, not a byte count, on purpose: the size of
    a page moves every time anyone edits a tool description, and a claim that
    goes red on an edit nobody cares about gets switched off. The property is
    "it fits". That is what gets pinned."""
    cursor, seen, page = None, set(), 0
    while True:
        page += 1
        req = urllib.request.Request(
            RESPONDER + endpoint,
            data=json.dumps({"jsonrpc": "2.0", "id": 1, "method": "tools/list",
                             "params": {} if cursor is None else {"cursor": cursor}}).encode(),
            headers={"Content-Type": "application/json",
                     "Accept": "application/json, text/event-stream"})
        with urllib.request.urlopen(req, timeout=90) as r:
            raw = r.read()
        # The whole HTTP body, because that is the number the client measures.
        if len(raw) > CLIENT_CAP_BYTES:
            return f"over on page {page}: {len(raw)} > {CLIENT_CAP_BYTES}"
        cursor = json.loads(raw)["result"].get("nextCursor")
        if not cursor:
            return "fits"
        if cursor in seen:  # a cursor that repeats is a loop, not a last page
            return f"cursor loop at page {page}: {cursor}"
        seen.add(cursor)


def dead_sdk_doc_links() -> str:
    """Every emem.dev URL a shipped SDK points a user at, fetched from the live
    site. A package's Documentation link is a promise made to someone who has
    already installed it, and neither pip nor npm ever re-checks it: the 2.1.0
    release of emem-langmem shipped a Documentation URL that had never resolved,
    and nothing anywhere went red. Docs render as .html, so a /docs/*.md URL is
    the shape that 404s. Reports the dead ones by name, because a bare count
    tells you something broke without telling you what to fix."""
    root = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")
    urls: set[str] = set()
    for sub in ("sdks", "python", "integrations"):
        for dirpath, dirnames, names in os.walk(os.path.join(root, sub)):
            dirnames[:] = [d for d in dirnames
                           if d not in {"node_modules", ".venv", "dist", "target"}]
            for n in names:
                if not n.endswith((".toml", ".md", ".json", ".py", ".ts", ".js")):
                    continue
                try:
                    with open(os.path.join(dirpath, n), encoding="utf-8") as fh:
                        text = fh.read()
                except (OSError, UnicodeDecodeError):
                    continue
                urls.update(re.findall(r"https://emem\.dev/[A-Za-z0-9/._#-]+", text))
    dead = []
    for u in sorted(urls):
        target = u.split("#", 1)[0].rstrip(".,)")
        try:
            req = urllib.request.Request(target, method="HEAD")
            with urllib.request.urlopen(req, timeout=30) as r:
                status = r.status
        except urllib.error.HTTPError as e:
            status = e.code
        except OSError:
            # A network fault is not a dead link. Say so rather than
            # reporting a false positive that someone then "fixes".
            return "unreachable: could not reach the responder"
        # Only 404 and 410 mean gone. A POST-only route answers 405 to the HEAD
        # this probe sends, and /v1/ask does exactly that while serving 200 to
        # the method it documents — counting that as dead is how a claim earns
        # a reputation for crying wolf and gets switched off.
        if status in (404, 410):
            dead.append(target)
    if not dead:
        return f"0 dead of {len(urls)}"
    return f"{len(dead)} dead of {len(urls)}: " + ", ".join(dead)


def core_profile_is_whole_on_page_one() -> str:
    """A no-cursor tools/list on /mcp must return the COMPLETE core profile and
    end the chain there.

    Directory scanners and several hosts take page one and stop. When the page
    budget split the core tier they got 12 of a profile the same response
    declared as 16, so the advertised surface was unreachable by construction
    for exactly the clients most likely to read it. The other half of the same
    property: the chain from /mcp must not hand a patient client on to the
    extended tier, or the cheap endpoint is the expensive one and the byte
    figure a directory was promised is wrong in the direction that matters.

    Reports a verdict rather than a count for the same reason widest_tools_page
    does: tool counts move on purpose, and a claim that reddens on an intended
    edit gets switched off."""
    req = urllib.request.Request(
        RESPONDER + "/mcp",
        data=json.dumps({"jsonrpc": "2.0", "id": 1, "method": "tools/list",
                         "params": {}}).encode(),
        headers={"Content-Type": "application/json",
                 "Accept": "application/json, text/event-stream"})
    with urllib.request.urlopen(req, timeout=90) as r:
        raw = r.read()
    result = json.loads(raw)["result"]
    shown = len(result.get("tools", []))
    declared = ((result.get("_meta") or {}).get("dev.emem/profiles") or {}).get("core")
    if declared is None:
        return "no core profile declared in _meta"
    if shown != declared:
        return f"page one shows {shown} of a {declared}-tool core profile"
    if result.get("nextCursor"):
        return f"chain continues past core: nextCursor={result['nextCursor']}"
    if len(raw) > CLIENT_CAP_BYTES:
        return f"page one is {len(raw)} > {CLIENT_CAP_BYTES}"
    return "whole core profile in one page, chain ends"


def unknown_recall_field_is_refused() -> str:
    """`determinstic: true`, one transposed letter, used to return every fact
    while `deterministic: true` filtered. The tamper-provenance filter silently
    did nothing and the response was indistinguishable from success.

    Compares the two spellings against each other rather than pinning a fact
    count, because the corpus grows: the property is that the typo is REFUSED
    and the correct spelling still filters."""
    typo_status, typo_body = post("/v1/recall",
                                  {"place": "Bengaluru", "determinstic": True})
    if typo_status == 200:
        return f"typo accepted: {len(typo_body.get('facts', []))} facts returned"
    code = (typo_body.get("details") or {}).get("code") or typo_body.get("code")
    unfiltered = len(post("/v1/recall", {"place": "Bengaluru"})[1].get("facts", []))
    filtered = len(post("/v1/recall",
                        {"place": "Bengaluru", "deterministic": True})[1].get("facts", []))
    if filtered >= unfiltered:
        return f"correct spelling did not filter: {filtered} of {unfiltered}"
    return f"{typo_status} {code}, and the correct spelling still filters"


def cell_matches_reports_a_check_that_ran() -> str:
    """`cell_matches` was hardcoded true on every 200 while the guard behind it
    only ran when the token was well-formed, so a bare cid, which asserts no
    cell at all, came back claiming the address had been checked.

    Asserts both directions: true on a canonical token where the comparison
    runs, false on a bare cid where there is nothing to compare."""
    ok = post("/v1/memory_token/resolve", {"token": NDVI_TOKEN})[1]
    bare = post("/v1/memory_token/resolve", {"token": NDVI_TOKEN.split(":")[-1]})[1]
    return (f"canonical={ok.get('cell_matches')} "
            f"bare_cid={bare.get('cell_matches')} degraded={bare.get('degraded')}")

# --------------------------------------------------------------------------- #

CLAIMS = [
    {
        "id": "sdk_doc_links_resolve",
        "claim": "Every emem.dev URL a shipped SDK points a user at resolves. "
                 "Docs render as .html; a /docs/*.md URL 404s, and a published "
                 "package's dead Documentation link is invisible to pip, to npm "
                 "and to the test suite.",
        "how": "collect every emem.dev URL under sdks/, python/ and "
               "integrations/, HEAD each one, name the dead",
        "probe": lambda c: dead_sdk_doc_links(),
    },
    {
        "id": "fact_cid_is_52_chars",
        "claim": "A fact_cid is 52 characters: a 256-bit blake3 in base32-nopad.",
        "how": "resolve a known-good token and measure len(fact_cid)",
        "probe": lambda c: str(len(post("/v1/memory_token/resolve",
                                       {"token": NDVI_TOKEN})[1]["fact_cid"])),
    },
    {
        "id": "short_cid_is_refused_as_malformed",
        "claim": "A 26-character cid is refused as malformed, not answered "
                 "cid_not_found. 26 is a memory file_cid (16 bytes truncated); "
                 "the two are different addresses and must not be confused.",
        "how": "resolve a 26-char cid, report status and details.code",
        "probe": lambda c: (lambda s, b: f"{s} {(b.get('details') or {}).get('code')}")(
            *post("/v1/memory_token/resolve",
                  {"token": f"emem:fact:{NDVI_CELL}:duhrfe62ymvuvdqvazb4b4f3fq"})),
    },
    {
        "id": "algorithm_registry_total",
        "claim": "The algorithm registry serves 168 entries. /v1/algorithms "
                 "returns one page of 20, so pagination.total is the count and "
                 "len(algorithms) is not.",
        "how": "GET /v1/algorithms and read pagination.total",
        "probe": lambda c: str(get("/v1/algorithms")["pagination"]["total"]),
    },
    {
        "id": "guard_verdict_is_advisory",
        "claim": "POST /v1/guard/verdict answers action=allow on a transcript "
                 "whose citation does not resolve. It is advisory. The only "
                 "honest discriminator is receipt.fact_cids, which is empty.",
        "how": "send a forged citation, report action and len(receipt.fact_cids)",
        "probe": lambda c: (lambda b: f"{b['action']} fact_cids={len(b['receipt']['fact_cids'])}")(
            post("/v1/guard/verdict", {"texts": [
                "NDVI at that cell is 0.4253807106598985, per "
                f"emem:fact:{NDVI_CELL}:qtv2bco56qw4pmlohk56dotoxyl3atmnjpmzrijj2kazw2mj57zz."
            ]})[1]),
    },
    {
        "id": "tools_list_pages_fit_client_cap",
        "claim": "Every page of tools/list on /mcp/full fits the 102,400-byte "
                 "payload cap MCP clients enforce. Measured over the whole HTTP "
                 "body, which is what the client measures.",
        "how": "walk the tools/list cursor chain, compare each response body "
               "against the cap, report a verdict rather than a size",
        "probe": lambda c: widest_tools_page(),
    },
    {
        "id": "ndvi_value_quoted_in_prose",
        "claim": f"NDVI at cell {NDVI_CELL} is 0.4253807106598985.",
        "how": "dereference the cited token and read value_verbatim",
        # The one claim that carries its own citation. `quotes` is the number
        # the prose above states; `check` asserts it appears verbatim in the
        # prose AND that emem's signed fact says the same digits. So editing
        # the number, or editing the token, both fail.
        "token": NDVI_TOKEN,
        "quotes": "0.4253807106598985",
        "probe": lambda c: (lambda s, b: b["value_verbatim"] if s == 200
                            else f"unresolved ({b.get('code')})")(
            *post("/v1/memory_token/resolve", {"token": c["token"]})),
    },
    {
        "id": "core_profile_whole_on_page_one",
        "claim": "A no-cursor tools/list on /mcp returns the complete core "
                 "profile and the chain ends there. A scanner that takes page "
                 "one and stops gets a callable profile, not a fragment.",
        "how": "one tools/list with no cursor; compare the tools shown against "
               "the core count the same response declares, and require a null "
               "nextCursor",
        "probe": lambda c: core_profile_is_whole_on_page_one(),
    },
    {
        "id": "unknown_recall_field_is_refused",
        "claim": "A misspelled recall field is refused, not dropped. "
                 "`determinstic: true` returned every fact while reporting "
                 "success, so a safety filter could be disabled by a typo.",
        "how": "send the typo and the correct spelling; require a typed refusal "
               "for one and real filtering from the other",
        "probe": lambda c: unknown_recall_field_is_refused(),
    },
    {
        "id": "cell_matches_reports_a_check_that_ran",
        "claim": "`cell_matches` is true only when the address comparison "
                 "actually ran. A bare cid asserts no cell, so it reports "
                 "false: not checked, rather than checked and agreed.",
        "how": "resolve a canonical token and a bare cid, report both flags",
        "probe": lambda c: cell_matches_reports_a_check_that_ran(),
    },
]
