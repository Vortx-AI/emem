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
import urllib.error
import urllib.request

RESPONDER = os.environ.get("EMEM_RESPONDER", "https://emem.dev").rstrip("/")

# A fact this responder holds, pinned as a citation. Claim 5 quotes its value.
NDVI_CELL = "defi.zb4e3.zaeed.fEya"
NDVI_TOKEN = f"emem:fact:{NDVI_CELL}:qtv2bco56qw4pmlohk56dotoxyl3atmnjpmzrijj2kazw2mj57oq"


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


# --------------------------------------------------------------------------- #

CLAIMS = [
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
