#!/usr/bin/env python3
"""Record this repo's claims as a signed emem note; fail when one stops being true.

Three subcommands:

  record   probe every claim in claims.py, write the answers into one ledger,
           sign it with an ed25519 key, POST it to emem, and pin its content
           address in assertions.lock.json.
  check    the gate. Re-fetch the ledger, verify its signature offline, rebuild
           it from the repo, and re-run every probe. Non-zero exit on drift.
  demo     run check green, then apply a one-character edit and run the SAME
           check again so you can see it go red.

Exit codes match the other gates in this repo: 0 clean, 1 drift, 2 unreachable.

Requires: pip install blake3 pynacl
"""
from __future__ import annotations

import argparse
import base64
import json
import os
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

import blake3
from nacl.exceptions import BadSignatureError
from nacl.signing import SigningKey, VerifyKey

sys.path.insert(0, str(Path(__file__).resolve().parent))
from claims import CLAIMS, RESPONDER, post  # noqa: E402

HERE = Path(__file__).resolve().parent
LOCK = HERE / "assertions.lock.json"
IDENTITY = Path(os.environ.get("EMEM_IDENTITY", HERE / ".identity.json"))

WRITE_DOMAIN = b"emem.memory_write|create|"


def b32(raw: bytes) -> str:
    return base64.b32encode(raw).decode().rstrip("=").lower()


def file_cid(body: bytes) -> str:
    """emem's content address for a memory file: base32 of the FIRST 16 BYTES
    of the body's blake3, so 26 characters. A fact_cid is the full 32 bytes,
    52 characters. Confusing the two is how a schema ends up declaring 26."""
    return b32(blake3.blake3(body).digest()[:16])


def write_digest(path: str, body: bytes) -> bytes:
    return blake3.blake3(WRITE_DOMAIN + path.encode() + b"|"
                         + blake3.blake3(body).digest()).digest()


# ------------------------------------------------------------------ identity
def identity() -> tuple[SigningKey, str, str]:
    """A local ed25519 key. There is no registration: the namespace
    /memories/by_attester/<first 8 chars of the pubkey>/ belongs to whoever
    writes to it first and is then held by that key alone."""
    if IDENTITY.exists():
        seed = bytes.fromhex(json.loads(IDENTITY.read_text())["seed_hex"])
    else:
        seed = os.urandom(32)
        IDENTITY.write_text(json.dumps({"seed_hex": seed.hex(),
                                        "note": "local demo key; delete to get a new namespace"}))
        IDENTITY.chmod(0o600)
    sk = SigningKey(seed)
    pub = b32(bytes(sk.verify_key))
    return sk, pub, pub[:8]


# -------------------------------------------------------------------- ledger
def ledger_text(claims: list[dict], observed: dict[str, str],
                recorded_at: str, attester: str) -> str:
    """The exact bytes that get signed. Deterministic given the repo plus the
    two values the lock remembers, so `check` can rebuild it and diff."""
    out = [
        "# stabilisation ledger",
        "",
        "Claims this repo makes about things that live outside it. Each was true",
        "against the responder named below at the moment it was recorded.",
        "",
        f"responder: {RESPONDER}",
        f"recorded_at: {recorded_at}",
        f"attester: {attester}",
        "source: demos/stabilisation/claims.py",
        "",
    ]
    for c in claims:
        out += [f"## {c['id']}",
                f"claim: {c['claim']}",
                f"how: {c['how']}",
                f"observed: {observed[c['id']]}"]
        if c.get("token"):
            out += [f"token: {c['token']}", f"quotes: {c['quotes']}"]
        out.append("")
    return "\n".join(out)


def memory_create(sk: SigningKey, pub: str, path: str, body: str) -> dict:
    sig = b32(sk.sign(write_digest(path, body.encode())).signature)
    req = urllib.request.Request(
        RESPONDER + "/mcp",
        data=json.dumps({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                         "params": {"name": "emem_memory_create", "arguments": {
                             "path": path, "file_text": body,
                             "attester": {"pubkey_b32": pub, "sig_b32": sig}}}}).encode(),
        headers={"Content-Type": "application/json", "Accept": "application/json"})
    r = json.load(urllib.request.urlopen(req, timeout=90))["result"]
    if r.get("isError"):
        raise SystemExit("memory_create refused: "
                         + (r.get("content") or [{}])[0].get("text", "")[:300])
    return r["structuredContent"]


def fetch_note(path: str) -> dict:
    req = urllib.request.Request(RESPONDER + path, headers={"Accept": "application/json"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.load(r)


# -------------------------------------------------------------------- record
def cmd_record(_a) -> int:
    sk, pub, pk8 = identity()
    observed = {}
    for c in CLAIMS:
        observed[c["id"]] = c["probe"](c)
        print(f"  probed {c['id']}: {observed[c['id']]}")
    for c in CLAIMS:
        if c.get("quotes") and c["quotes"] not in c["claim"]:
            raise SystemExit(f"{c['id']}: quotes={c['quotes']!r} does not appear in its own "
                             f"claim text. Refusing to record a claim that misquotes itself.")

    recorded_at = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    slug = recorded_at.replace(":", "").replace("-", "")
    path = f"/memories/by_attester/{pk8}/stabilise/{slug}-ledger.md"
    body = ledger_text(CLAIMS, observed, recorded_at, pub)

    res = memory_create(sk, pub, path, body)
    if res["file_cid"] != file_cid(body.encode()):
        raise SystemExit(f"the responder addressed the note as {res['file_cid']} but these "
                         f"bytes hash to {file_cid(body.encode())}; it did not store what was sent")

    LOCK.write_text(json.dumps({
        "responder": RESPONDER,
        "recorded_at": recorded_at,
        "attester_pubkey_b32": pub,
        "ledger_path": path,
        "ledger_file_cid": res["file_cid"],
        "observed": observed,
    }, indent=2, sort_keys=True) + "\n")
    print(f"\nrecorded {len(CLAIMS)} claims")
    print(f"  ledger    {RESPONDER}{path}")
    print(f"  file_cid  {res['file_cid']}")
    print(f"  lock      {LOCK}")
    return 0


# --------------------------------------------------------------------- check
def run_check(claims: list[dict], lock: dict) -> list[str]:
    """Every failure this gate can report. Returns a list of problems.

    Split out so `demo` can call it on a deliberately edited copy and get a red
    from the same code path that produces the green, rather than from a second
    implementation that could be wrong in a flattering direction.
    """
    problems: list[str] = []
    if RESPONDER != lock["responder"]:
        problems.append(f"the lock was recorded against {lock['responder']} and this run "
                        f"is pointed at {RESPONDER}; a claim recorded elsewhere is not "
                        f"evidence about this responder")

    # 1. The signed record is intact, and we do not take the responder's word
    #    for that: the bytes are re-hashed here and the signature is checked
    #    against the attester's own key, not against anything emem asserts.
    try:
        note = fetch_note(lock["ledger_path"])
    except urllib.error.HTTPError as e:
        # A missing ledger is drift, not an outage. Reporting it as "responder
        # unreachable" would exit 2 and read as a skip, which is how a gate
        # ends up green about a record that is no longer there.
        return problems + [f"the ledger at {lock['ledger_path']} is gone: HTTP {e.code}"]
    served = note["content"]
    a = note.get("authorship") or {}
    local_cid = file_cid(served.encode())
    if local_cid != lock["ledger_file_cid"]:
        problems.append(
            f"the ledger at {lock['ledger_path']} now hashes to {local_cid}, but the lock "
            f"pins {lock['ledger_file_cid']}: the recorded assertions were rewritten")
    if a.get("attester_pubkey_b32") != lock["attester_pubkey_b32"]:
        problems.append(f"the ledger is now signed by {a.get('attester_pubkey_b32')}, "
                        f"not {lock['attester_pubkey_b32']}")
    elif a.get("body_hash_hex") != blake3.blake3(served.encode()).hexdigest():
        problems.append("the signature on the ledger is bound to a different body than "
                        "the bytes served")
    else:
        try:
            VerifyKey(base64.b32decode(a["attester_pubkey_b32"].upper() + "====")).verify(
                write_digest(a["signed_path"], served.encode()),
                base64.b32decode(a["sig_b32"].upper() + "="))
        except (BadSignatureError, ValueError, KeyError) as e:
            problems.append(f"the ledger's ed25519 signature does not verify: {e}")

    # 2. The repo still says what was signed. Rebuilding from claims.py means an
    #    edit to a claim's prose is caught even when the world has not moved.
    rebuilt = ledger_text(claims, lock["observed"], lock["recorded_at"],
                          lock["attester_pubkey_b32"])
    if rebuilt != served:
        s, r = served.split("\n"), rebuilt.split("\n")
        first = next((i for i in range(max(len(s), len(r)))
                      if (s[i:i + 1] or [""])[0] != (r[i:i + 1] or [""])[0]), 0)
        problems.append(
            f"claims.py no longer matches the signed ledger, first difference at line "
            f"{first + 1}:\n      signed: {(s[first:first + 1] or [''])[0][:110]}\n"
            f"      repo:   {(r[first:first + 1] or [''])[0][:110]}")

    # 3. The world has not moved under the claims.
    for c in claims:
        was = lock["observed"].get(c["id"])
        if was is None:
            problems.append(f"{c['id']}: present in claims.py, absent from the lock; "
                            f"it has never been recorded")
            continue
        try:
            now = c["probe"](c)
        except Exception as e:  # a probe that cannot run is not a pass
            problems.append(f"{c['id']}: probe failed: {type(e).__name__}: {str(e)[:100]}")
            continue
        if now != was:
            problems.append(f"{c['id']}: recorded {was!r}, now {now!r}  ({c['how']})")

    # 4. Claims that carry a citation: the token dereferences, and the number in
    #    the prose is the number the signed fact carries.
    for c in claims:
        if not c.get("token"):
            continue
        if c["quotes"] not in c["claim"]:
            problems.append(f"{c['id']}: the claim no longer contains {c['quotes']!r}, "
                            f"which is the value its citation is checked against")
            continue
        status, body = post("/v1/echo_verify", {"token": c["token"],
                                                "claimed_value": c["quotes"], "strict": True})
        if status != 200:
            problems.append(f"{c['id']}: its citation does not resolve: HTTP {status} "
                            f"{body.get('code')} for {c['token'][-16:]}")
        elif not body.get("matches"):
            problems.append(f"{c['id']}: the prose says {c['quotes']} but the signed fact "
                            f"says {body.get('resolved_value_verbatim')} "
                            f"(drift: {body.get('drift')})")
    return problems


def cmd_check(_a) -> int:
    if not LOCK.exists():
        print(f"no lock at {LOCK}; run `stabilise.py record` first")
        return 2
    lock = json.loads(LOCK.read_text())
    try:
        problems = run_check(CLAIMS, lock)
    except urllib.error.URLError as e:
        print(f"  (skipped: {RESPONDER} unreachable: {e})")
        return 2
    return report(problems, len(CLAIMS), lock)


def report(problems: list[str], n: int, lock: dict) -> int:
    print(f"{n} claims, recorded {lock['recorded_at']} against {lock['responder']}")
    if problems:
        print("\nDRIFT:")
        for p in problems:
            print(f"  x {p}")
        print("\nEither the claim was true and stopped being true, or someone changed"
              "\nwhat it says. Fix the claim, then re-run `record` to sign the new one.")
        return 1
    print("\nEvery claim still holds, and the signed record of them is intact.")
    return 0


# ---------------------------------------------------------------------- demo
def cmd_demo(_a) -> int:
    if not LOCK.exists():
        print(f"no lock at {LOCK}; run `stabilise.py record` first")
        return 2
    lock = json.loads(LOCK.read_text())

    print("=" * 72)
    print("1. the recorded claims, checked against the live responder")
    print("=" * 72)
    clean = run_check(CLAIMS, lock)
    report(clean, len(CLAIMS), lock)
    if clean:
        print("\nThe green half of this demo did not come out green, so the red half "
              "\nbelow proves nothing. Re-run `record` and try again.")
        return 1

    print()
    print("=" * 72)
    print("2. the same check, after one digit of the quoted value is edited")
    print("=" * 72)
    edited = [dict(c) for c in CLAIMS]
    target = next(c for c in edited if c.get("token"))
    old = target["quotes"]
    i = old.index(".") + 1
    new = old[:i] + str((int(old[i]) + 1) % 10) + old[i + 1:]
    print(f"editing {target['id']}: {old} -> {new}")
    target["claim"] = target["claim"].replace(old, new)
    target["quotes"] = new
    rc2 = report(run_check(edited, lock), len(edited), lock)

    print()
    print("=" * 72)
    print("3. the same check, after one character of the citation is edited")
    print("=" * 72)
    edited = [dict(c) for c in CLAIMS]
    target = next(c for c in edited if c.get("token"))
    old = target["token"]
    target["token"] = old[:-1] + ("a" if old[-1] != "a" else "b")
    print(f"editing {target['id']}: last character of the token, "
          f"...{old[-8:]} -> ...{target['token'][-8:]}")
    rc3 = report(run_check(edited, lock), len(edited), lock)

    if rc2 == 0 or rc3 == 0:
        print("\nAn edit was NOT caught. That is a failure of this demo, not a pass.")
        return 1
    print("\nOne digit changed and the number stopped matching the fact emem signed."
          "\nOne character of the citation changed and the citation stopped resolving."
          "\nBoth also broke the content address of the signed ledger, which is the"
          "\ncheck that works even when the world has not moved at all.")
    return 0


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("record", help="probe, sign, publish, pin").set_defaults(fn=cmd_record)
    sub.add_parser("check", help="the gate; non-zero on drift").set_defaults(fn=cmd_check)
    sub.add_parser("demo", help="show a pass and a fail").set_defaults(fn=cmd_demo)
    a = p.parse_args()
    return a.fn(a)


if __name__ == "__main__":
    raise SystemExit(main())
