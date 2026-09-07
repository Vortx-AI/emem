#!/usr/bin/env python3
"""Check a node against the join bar in docs/federation.md §8e.

Two roles, because they need different things published.

A WITNESS attests that it saw a head. It has to prove it exists, that its key
is stable, and that a third party can bind that key to its name without asking
either party. It does NOT have to publish a corpus: witnessing gets stronger
the less the witness holds, and a bar that demands data keeps out exactly the
parties whose independence is worth most.

A RESOLVER additionally asks readers to fetch facts from it, so it must publish
what a reader needs to check those facts: the registries they were signed
against, and an enumerable surface.

The first version of §8e had one bar with the registry CIDs in it, and geo.qa
failed it while co-signing this log every fifteen minutes. That is the bug this
script exists to stop repeating: an acceptance test no participant passes is not
a standard, it is a sentence.

Everything here is fetched anonymously. If a check needs a credential, it is
not a check a third party can run, and the whole point is that they can.

    python3 scripts/verify_node.py https://geo.qa/emem
    python3 scripts/verify_node.py https://emem.dev --role resolver

Exit 0 the node meets the role's bar, 1 it does not, 2 nothing answered at all.
"""
from __future__ import annotations

import argparse
import json
import sys
import urllib.request

UA = "emem-verify-node/1 (+https://emem.dev)"
ROWS: list[tuple[str, bool, str]] = []


def get(url: str, timeout: float = 25.0):
    req = urllib.request.Request(url, headers={"user-agent": UA})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, r.read()
    except urllib.error.HTTPError as e:
        return e.code, b""
    except Exception:  # noqa: BLE001
        return 0, b""


def row(name: str, ok: bool, detail: str):
    ROWS.append((name, ok, detail))
    return ok


def host_of(origin: str) -> str:
    return origin.split("://", 1)[-1].split("/", 1)[0].split(":", 1)[0]


def dns_txt(name: str) -> list[str]:
    """Over DoH, so this works on a box with no resolver tooling and matches
    the path enlistment.rs uses for _emem-agent."""
    try:
        req = urllib.request.Request(
            f"https://cloudflare-dns.com/dns-query?name={name}&type=TXT",
            headers={"accept": "application/dns-json"})
        with urllib.request.urlopen(req, timeout=15) as r:
            doc = json.load(r)
    except Exception:  # noqa: BLE001
        return []
    out = []
    for ans in doc.get("Answer") or []:
        if ans.get("type") == 16:
            raw = ans.get("data") or ""
            out.append("".join(p for p in raw.split('"') if p.strip()))
    return out


def check_witness(origin: str) -> str | None:
    """The witness bar. Returns the responder key if it could be established."""
    st, body = get(f"{origin}/v1/log/sth")
    key = None
    if st == 200:
        try:
            sth = json.loads(body)["sth"]
            key = sth.get("responder_pubkey_b32")
            row("/v1/log/sth", True,
                f"tree_size {sth.get('tree_size')}, signer {(key or '')[:12]}...")
        except Exception:  # noqa: BLE001
            row("/v1/log/sth", False, "200 but not a parseable STH")
    else:
        row("/v1/log/sth", False, f"http {st or 'no response'}")

    st, body = get(f"{origin}/live")
    live_key = None
    if st == 200:
        try:
            d = json.loads(body)
            live_key = d.get("responder_pubkey_b32")
            row("/live carries a stable responder key", bool(live_key),
                f"{(live_key or 'absent')[:12]}... version {d.get('version')}")
        except Exception:  # noqa: BLE001
            row("/live carries a stable responder key", False, "200 but not JSON")
    else:
        row("/live carries a stable responder key", False, f"http {st or 'no response'}")

    # The two keys must agree, or "stable" means nothing.
    if key and live_key:
        row("the STH key and the /live key agree", key == live_key,
            "same key" if key == live_key else f"{key[:10]}... vs {live_key[:10]}...")

    host = host_of(origin)
    recs = dns_txt(f"_emem-node.{host}")
    if not recs:
        row(f"_emem-node.{host} TXT", False, "no TXT record")
    else:
        found = None
        for rec in recs:
            fields = {}
            for part in rec.split(";"):
                k, _, v = part.strip().partition("=")
                if k:
                    fields[k.strip().lower()] = v.strip()
            if fields.get("v") == "emem1" and fields.get("k"):
                found = fields["k"].lower()
                break
        if not found:
            row(f"_emem-node.{host} TXT", False,
                f"{len(recs)} record(s), none in v=emem1 form")
        elif key and found != key.lower():
            row(f"_emem-node.{host} TXT", False,
                f"publishes {found[:12]}... but the node signs with {key[:12]}...")
        else:
            row(f"_emem-node.{host} TXT", True, "matches the signing key")

    st, body = get(f"{origin}/.well-known/did.json")
    if st == 200:
        try:
            doc = json.loads(body)
            n = len(doc.get("verificationMethod") or [])
            row("/.well-known/did.json", bool(doc.get("id")),
                f"{doc.get('id')}, {n} verification method(s)")
        except Exception:  # noqa: BLE001
            row("/.well-known/did.json", False, "200 but not JSON")
    else:
        row("/.well-known/did.json", False, f"http {st or 'no response'}")
    return key


def check_resolver(origin: str) -> None:
    """The resolver bar, on top of the witness bar."""
    st, body = get(f"{origin}/health")
    cids = ("algorithms_cid", "bands_cid", "schema_cid", "sources_cid")
    if st == 200:
        try:
            d = json.loads(body)
            have = [c for c in cids if d.get(c)]
            row("the four registry CIDs are public", len(have) == 4,
                f"{len(have)}/4 present" if have else "none present")
        except Exception:  # noqa: BLE001
            row("the four registry CIDs are public", False, "200 but not JSON")
    elif st in (401, 403):
        row("the four registry CIDs are public", False,
            f"http {st}: private deployment, a reader cannot check what it serves")
    else:
        row("the four registry CIDs are public", False, f"http {st or 'no response'}")

    st, _ = get(f"{origin}/openapi.json")
    row("/openapi.json", st == 200, f"http {st or 'no response'}")

    st, body = get(f"{origin}/.well-known/emem.json")
    if st == 200:
        try:
            fed = json.loads(body).get("federation")
            row("/.well-known/emem.json has a federation block", bool(fed),
                "present" if fed else "served, but no federation block")
        except Exception:  # noqa: BLE001
            row("/.well-known/emem.json has a federation block", False,
                "200 but not JSON")
    else:
        row("/.well-known/emem.json has a federation block", False,
            f"http {st or 'no response'}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("origin", help="e.g. https://geo.qa/emem")
    ap.add_argument("--role", choices=("witness", "resolver"), default="witness",
                    help="resolver also checks what a READER needs (default: witness)")
    a = ap.parse_args()
    origin = a.origin.rstrip("/")

    print(f"{origin} against docs/federation.md §8e, role: {a.role}\n")
    check_witness(origin)
    n_witness = len(ROWS)
    if a.role == "resolver":
        check_resolver(origin)

    if not any(ok for _, ok, _ in ROWS):
        print("  nothing answered; the origin may be wrong or the node down")
        return 2

    w = max(len(n) for n, _, _ in ROWS)
    for i, (name, ok, detail) in enumerate(ROWS):
        if i == n_witness:
            print(f"  {'-' * (w + 8)}  resolver bar")
        print(f"  {'ok  ' if ok else 'FAIL'}  {name:<{w}}  {detail}")

    failed = [n for n, ok, _ in ROWS if not ok]
    print()
    if failed:
        print(f"{len(ROWS) - len(failed)} of {len(ROWS)} met. Not joined as "
              f"{a.role}: {', '.join(failed[:3])}"
              + (" ..." if len(failed) > 3 else ""))
        return 1
    print(f"all {len(ROWS)} met: joined as {a.role}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
