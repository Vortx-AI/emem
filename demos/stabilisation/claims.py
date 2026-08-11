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
]
