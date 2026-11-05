#!/usr/bin/env python3
"""Verify who wrote an emem memory note, offline, then read its author's key card.

    verify_note.py VIEW.json [NOTE_FILE] [--offline] [--origin URL]
    verify_note.py --self-test      # check the bundled crypto

VIEW.json is a saved emem_memory_view result: either the whole JSON-RPC
response from https://emem.dev/mcp, or the JSON object inside its
result.content[0].text. A note too large for one MCP result arrives with
`content` omitted; download it (GET https://emem.dev<signed_path>) and pass
the file as NOTE_FILE. The receipt on that response proves the responder
stored and served the note; this script checks the other claim, that the
key in `authorship` signed these bytes at this path:

  body    blake3(content) == body_hash_hex    (create, str_replace, insert)
  sig     ed25519 over the per-verb memory-write digest verifies against
          authorship.attester_pubkey_b32
  banner  when the view carries a `_superseded` banner, its signature
          verifies against the banner's attester_pubkey_b32, over the
          digest with verb "supersede", the note's path, body_hash =
          blake3(replaced_by + "|" + reason) and a base. The base is the
          banner's signed_base when it has one, else the note's own
          file_cid. Supersedes signed before v1 was retired used v1 (no
          base), so v1 is tried second. No banner is not a failure.

The digest, from `caller_signed_objects` in GET /v1/verifier_spec. The
server now refuses v1 for every write; notes signed with v1 before that
still verify here:

  v1  blake3("emem.memory_write|" + verb + "|" + signed_path + "|" + body_hash)
  v2  blake3("emem.memory_write.v2|" + verb + "|" + signed_path + "|" + body_hash
             + "|" + base)

body_hash enters as 32 raw bytes; base is the replaced file_cid or the
literal "absent". Exit 0 when every check passed, 1 when one failed (a
banner signature that does not verify counts), 2 on bad input or a note
with no caller signature. The checks make no network
calls; BLAKE3 and Ed25519 come from emem_crypto.py beside this script, plain
Python shipped with the plugin. A valid signature says which key wrote the
note, never that what it says is true.

When the note verifies, the script reads the author's key card,
/memories/by_attester/<pubkey8>/key-card.md, with one read-only GET to URL
(default https://emem.dev). A key card is the key's own statement of its
role and operator. The script shows it only when the same key signed it,
and labels it as the author's own claim: the signature proves who wrote
the card, not that it is true. A missing card or a network failure changes
nothing else, because the exit code comes from the note alone. --offline
skips the read, and then the script makes no network call.
"""
from __future__ import annotations

import argparse
import base64
import http.client
import json
import re
import sys
import urllib.error
import urllib.request
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))
try:
    from emem_crypto import blake3, ed25519_verify, self_test
except ImportError:
    sys.stderr.write("emem_crypto.py not found: it ships beside this script in the skill's scripts/ directory\n")
    sys.exit(2)

ORIGIN = "https://emem.dev"
BODY_VERBS = ("create", "str_replace", "insert")
CARD_FIELD = re.compile(r"\s*(role|operator|contact|rotates|other_keys_of_this_operator)\s*:\s*(.*?)\s*$", re.I)


def b32d(s: str) -> bytes:
    s = s.strip().upper()
    return base64.b32decode(s + "=" * ((8 - len(s) % 8) % 8))


def load_view(path: str) -> dict:
    doc = json.loads(Path(path).read_text())
    if isinstance(doc, dict) and "result" in doc:
        doc = doc["result"]
    if isinstance(doc, dict) and isinstance(doc.get("content"), list):
        doc = json.loads(doc["content"][0]["text"])
    if not isinstance(doc, dict) or "authorship" not in doc:
        raise ValueError("not an emem_memory_view result: no authorship block")
    return doc


def signature_valid(a: dict) -> tuple[bool, int]:
    """(valid, preimage version) for an authorship block."""
    head = a["verb"].encode() + b"|" + a["signed_path"].encode() + b"|" + bytes.fromhex(a["body_hash_hex"])
    version = int(a.get("preimage_version") or 1)
    if version == 2:
        base = a.get("base") or "absent"
        preimage = b"emem.memory_write.v2|" + head + b"|" + base.encode()
    else:
        preimage = b"emem.memory_write|" + head
    return ed25519_verify(b32d(a["attester_pubkey_b32"]), b32d(a["sig_b32"]), blake3(preimage).digest()), version


def supersede_valid(pubkey: str, sig: str, path: str, replaced_by: str, reason: str, base: str):
    """The preimage version a supersede signature verifies under, or None."""
    body_hash = blake3(f"{replaced_by}|{reason}".encode("utf-8")).digest()
    head = b"supersede|" + path.encode() + b"|" + body_hash
    for version, preimage in ((2, b"emem.memory_write.v2|" + head + b"|" + base.encode()),
                              (1, b"emem.memory_write|" + head)):
        if ed25519_verify(b32d(pubkey), b32d(sig), blake3(preimage).digest()):
            return version
    return None


def banner(view: dict, note_signer: str) -> tuple[bool, str]:
    """(ok, line) for the `_superseded` banner. ok is False only for a signature that does not verify."""
    b = view.get("_superseded")
    if not isinstance(b, dict):
        return True, "banner  NONE      no _superseded banner: the note is not marked as replaced"
    target = b.get("this_note_was_replaced_by")
    pk, sig, reason = b.get("attester_pubkey_b32"), b.get("sig_b32"), b.get("reason")
    if not pk or not sig:
        return True, (f"banner  UNSIGNED  replaced by {shown(str(target))}; the banner has no signature, "
                      "so only the responder vouches for it")
    paths = []
    for p in (view.get("path"), view["authorship"].get("signed_path")):
        if isinstance(p, str) and p not in paths:
            paths.append(p)
    signed_base = b.get("signed_base")
    if isinstance(signed_base, str) and signed_base:
        base, base_from = signed_base, "the banner's signed_base"
    else:
        base, base_from = view.get("file_cid"), "this note's file_cid"
    version = None
    if isinstance(target, str) and isinstance(reason, str) and isinstance(base, str):
        try:
            version = next((v for v in (supersede_valid(pk, sig, p, target, reason, base)
                                        for p in paths) if v), None)
        except ValueError:
            version = None
    if version is None:
        return False, (f"banner  INVALID   the supersede signature does not verify over replaced_by|reason "
                       f"with base from {base_from}: the target, the reason or the base may have changed")
    who = "this note's key" if pk.lower() == note_signer.lower() else f"another key {shown(pk)}"
    how = f"v2 with base from {base_from}" if version == 2 else "v1, which has no base"
    return True, f"banner  VALID     supersede {how}, by {who}; replaced by {shown(target)}"


def shown(value: str) -> str:
    """Author text, made safe to print: no control characters, at most 120 characters."""
    return "".join(c for c in value if c.isprintable())[:120]


def key_card(origin: str, signer: str) -> list[str]:
    """Lines that report the signer's key card. Never raises: the card is advice."""
    path = f"/memories/by_attester/{signer[:8].lower()}/key-card.md"
    req = urllib.request.Request(origin + path, headers={"accept": "application/json",
                                                         "user-agent": "emem-plugin-verify-note"})
    try:
        with urllib.request.urlopen(req, timeout=15) as r:
            doc = json.loads(r.read(1 << 20))
    except urllib.error.HTTPError as e:
        if e.code in (404, 410):
            return [f"card    NONE      no key card at {path}"]
        return [f"card    UNCHECKED the key card read answered HTTP {e.code}"]
    except (OSError, ValueError, http.client.HTTPException) as e:
        return [f"card    UNCHECKED could not read the key card: {shown(str(getattr(e, 'reason', e)))}"]
    try:
        a, content = doc["authorship"], doc["content"]
        if not a.get("caller_signed") or not a.get("sig_b32"):
            return [f"card    UNSIGNED  {path} carries no caller signature, so it is not shown"]
        if a["attester_pubkey_b32"].lower() != signer.lower():
            return [f"card    FOREIGN   {path} is signed by {shown(a['attester_pubkey_b32'])}, "
                    "not by this note's key, so it is not shown"]
        bound = a["verb"] in BODY_VERBS and blake3(content.encode("utf-8")).digest() == bytes.fromhex(a["body_hash_hex"])
        if not (bound and signature_valid(a)[0]):
            return [f"card    INVALID   {path} does not verify, so it is not shown"]
    except (KeyError, TypeError, ValueError, AttributeError):
        return [f"card    UNCHECKED {path} did not come back as a memory view"]
    fields: dict[str, str] = {}
    for line in content.splitlines():
        m = CARD_FIELD.match(line)
        if m and m.group(1).lower() not in fields:
            fields[m.group(1).lower()] = shown(m.group(2))
    out = [f"card    VALID     {path}, signed by this note's key. It proves only who wrote it."]
    out.append(f"        role      {fields.get('role') or '(not stated)'}  (the author's own claim)")
    out.append(f"        operator  {fields.get('operator') or '(not stated)'}  (the author's own claim)")
    if fields.get("other_keys_of_this_operator"):
        out.append(f"        others    {fields['other_keys_of_this_operator']}  (the author's own claim)")
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description="Verify who wrote an emem memory note, then read its author's key card.")
    ap.add_argument("view", nargs="?", help="a saved emem_memory_view result")
    ap.add_argument("note_file", nargs="?", help="the note's bytes, when the view omits `content`")
    ap.add_argument("--self-test", action="store_true", help="check the bundled crypto and exit")
    ap.add_argument("--offline", action="store_true", help="do not read the author's key card")
    ap.add_argument("--origin", default=ORIGIN, help=f"where to read the key card (default {ORIGIN})")
    args = ap.parse_args()
    if args.self_test:
        return 0 if self_test() else 1
    origin = args.origin.rstrip("/")
    if not args.view or not re.match(r"https?://", origin):
        sys.stderr.write(__doc__)
        return 2
    try:
        view = load_view(args.view)
    except (OSError, ValueError, KeyError, IndexError) as e:
        sys.stderr.write(f"cannot read the view: {e}\n")
        return 2
    a = view["authorship"]
    if not a.get("caller_signed") or not a.get("sig_b32"):
        sys.stderr.write("this note carries no caller signature; only the responder's receipt vouches for it\n")
        return 2

    ok = True
    verb = a["verb"]
    if verb in BODY_VERBS:
        if args.note_file:
            body = Path(args.note_file).read_bytes()
        elif isinstance(view.get("content"), str):
            body = view["content"].encode("utf-8")
        else:
            sys.stderr.write("the view omits `content` (too large for one MCP result); download "
                             f"{origin}{a['signed_path']} and pass it as the second argument\n")
            return 2
        got = blake3(body).digest()
        same = got == bytes.fromhex(a["body_hash_hex"])
        ok &= same
        print(f"body    {'MATCH   ' if same else 'MISMATCH'}  blake3(content)={got.hex()}")
    else:
        print(f"body    SKIPPED   verb={verb} signs no body bytes")

    good, version = signature_valid(a)
    ok &= good
    signer = a["attester_pubkey_b32"]
    print(f"sig     {'VALID   ' if good else 'INVALID '}  v{version} verb={verb} signer={signer}")
    print(f"path    {a['signed_path']}")
    banner_ok, line = banner(view, signer)
    print(line)
    if args.offline:
        print("card    SKIPPED   --offline")
    elif not ok:
        print("card    SKIPPED   the note did not verify")
    else:
        for line in key_card(origin, signer):
            print(line)
    return 0 if ok and banner_ok else 1


if __name__ == "__main__":
    sys.exit(main())
