#!/usr/bin/env python3
"""Build and check emem pointer.v1 trees offline.

    tree_proof.py build FILE [--chunk BYTES] [--source TEXT]
        Cut FILE into units, hash each one, and print a pointer.v1 index
        note on stdout: one `·` row per unit and the Merkle root in the
        front matter. Markdown is cut at its headings (the shallowest level
        that occurs at least twice); anything else into fixed byte ranges
        (default 65536). Nothing is sent anywhere; publishing the note is a
        separate, signed emem_memory_create.

    tree_proof.py check ROW.json NOTE.md [FILE]
        ROW.json is GET /v1/tree/<file_cid>?row=<i>, NOTE.md the note that
        file_cid names. Recomputes the row's leaf from the note's table,
        folds the audit path, and requires both the served root and the
        note's own `root:` to equal the result. With FILE (the source you
        hold), also re-hashes the unit's bytes against the row.

    tree_proof.py --self-test

Rules, from crates/emem-api-rest/src/tree.rs:
  leaf = blake3(url_utf8 || u64_be(offset) || u64_be(length) || hash32)
         (a `·` url hashes as the empty string; a row with no hash as zeros)
  node = blake3(left || right); an odd node is promoted unchanged
Hashes are base32, lowercase, no padding. The tree itself is not signed:
the author's Ed25519 signature over the whole note is what binds the root,
and verify_note.py (emem-multi-agent-handoff) checks it. Exit 0 when every
check passed, 1 when one failed, 2 on bad input. No network calls.
"""
from __future__ import annotations

import base64
import json
import re
import struct
import sys
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "lib"))
try:
    from emem_crypto import blake3, self_test
except ImportError:
    sys.stderr.write("emem_crypto.py not found: it ships in the emem plugin's lib/ directory, three levels above this script\n")
    sys.exit(2)


def b32e(b: bytes) -> str:
    return base64.b32encode(b).decode().rstrip("=").lower()


def b32d(s: str) -> bytes:
    s = s.strip().upper()
    return base64.b32decode(s + "=" * ((8 - len(s) % 8) % 8))


def h(data: bytes) -> bytes:
    return blake3(data).digest()


def leaf(url: str, offset: int, length: int, hash32: bytes) -> bytes:
    return h(url.encode() + struct.pack(">QQ", offset, length) + hash32)


def root(leaves: list[bytes]) -> bytes:
    level = list(leaves)
    while len(level) > 1:
        level = [h(level[i] + level[i + 1]) if i + 1 < len(level) else level[i]
                 for i in range(0, len(level), 2)]
    return level[0]


def front(text: str, key: str):
    lines = text.splitlines()
    if not lines or lines[0].strip() != "---":
        return None
    for line in lines[1:]:
        if line.strip() == "---":
            return None
        if line.startswith(key + ":"):
            return line[len(key) + 1:].strip()
    return None


def rows(text: str):
    """(label, url, offset, length, hash32) per pointer row, in leaf order."""
    out = []
    for line in text.splitlines():
        if not (line.startswith("| ") and line.endswith(" |")):
            continue
        c = line[2:-2].split(" | ")
        if len(c) not in (5, 6):
            continue
        label, url, off, ln, hs = c[0], c[1], c[2], c[3], c[4].strip()
        if not label.strip() or "|" in label or label.strip() == "what":
            continue
        if not url or any(ch.isspace() for ch in url) or not off.isdigit() or not ln.isdigit():
            continue
        hash32 = b32d(hs) if re.fullmatch(r"[a-z2-7]{52}", hs) else bytes(32)
        out.append((label.strip(), "" if url == "·" else url, int(off), int(ln), hash32))
    return out


def units(data: bytes, chunk: int, name: str):
    """(label, offset, length) per unit."""
    if name.lower().endswith((".md", ".markdown")):
        try:
            text = data.decode("utf-8")
        except UnicodeDecodeError:
            text = None
        if text is not None:
            starts, fence = {}, False
            pos = 0
            for line in text.splitlines(keepends=True):
                if line.lstrip().startswith("```"):
                    fence = not fence
                m = None if fence else re.match(r"(#{1,6}) +(\S.*)", line)
                if m:
                    starts.setdefault(len(m.group(1)), []).append((len(text[:pos].encode()), m.group(2).strip()))
                pos += len(line)
            level = min((lv for lv, s in starts.items() if len(s) >= 2), default=None)
            if level is not None:
                cuts = starts[level]
                if cuts[0][0] > 0:
                    cuts = [(0, "preamble")] + cuts
                bounds = [c[0] for c in cuts] + [len(data)]
                return [(re.sub(r"[|\s]+", " ", lab)[:80] or "section", bounds[i], bounds[i + 1] - bounds[i])
                        for i, (_, lab) in enumerate(cuts)]
    n = max(1, -(-len(data) // chunk))
    return [(f"bytes {i * chunk}", i * chunk, min(chunk, len(data) - i * chunk)) for i in range(n)]


def build(path: str, chunk: int, source: str) -> int:
    data = Path(path).read_bytes()
    us = units(data, chunk, path)
    table, leaves = [], []
    for label, off, ln in us:
        hs = h(data[off:off + ln])
        leaves.append(leaf("", off, ln, hs))
        table.append(f"| {label} | · | {off} | {ln} | {b32e(hs)} |")
    note = "\n".join([
        "---",
        "emem: pointer.v1",
        f"source: {source or Path(path).name}, held by its writer (not uploaded)",
        f"bytes: {len(data)}",
        f"blake3: {b32e(h(data))}",
        f"chunks: {len(us)} of {len(us)} hashed",
        f"root: {b32e(root(leaves))}",
        "---",
        "",
        f"# {Path(path).name}",
        "",
        "## Units",
        "",
        "| what | url (· is the source) | offset | length | blake3 |",
        "|---|---|---|---|---|",
        *table,
        "",
    ])
    sys.stdout.write(note)
    return 0


def check(row_path: str, note_path: str, file_path) -> int:
    resp = json.loads(Path(row_path).read_text())
    if resp.get("schema") == "emem.error.v1" or "leaf_b32" not in resp:
        sys.stderr.write(f"not a /v1/tree row response: {resp.get('code')}: {resp.get('message')}\n")
        return 2
    note = Path(note_path).read_bytes()
    text = note.decode("utf-8")
    ok = True

    cid = b32e(h(note)[:16])
    same = cid == resp["file_cid"]
    ok &= same
    print(f"note    {'MATCH   ' if same else 'MISMATCH'}  file_cid={cid}")

    table = rows(text)
    i = int(resp["row"]["index"])
    if i >= len(table):
        print(f"row     MISSING   index {i} of {len(table)} rows in the note")
        return 1
    label, url, off, ln, hash32 = table[i]
    lf = leaf(url, off, ln, hash32)
    same = b32e(lf) == resp["leaf_b32"]
    ok &= same
    print(f"leaf    {'MATCH   ' if same else 'MISMATCH'}  row {i} '{label}' offset={off} length={ln}")

    acc = lf
    for step in resp["path"]:
        sib = b32d(step["hash_b32"])
        acc = h(sib + acc) if step["side"] == "left" else h(acc + sib)
    stated = front(text, "root") or ""
    full = b32e(root([leaf(*r[1:]) for r in table]))
    same = b32e(acc) == resp["root_b32"] == stated == full
    ok &= same
    print(f"root    {'MATCH   ' if same else 'MISMATCH'}  path={len(resp['path'])} steps root={b32e(acc)}")

    if file_path:
        data = Path(file_path).read_bytes()
        same = h(data[off:off + ln]) == hash32
        ok &= same
        print(f"unit    {'MATCH   ' if same else 'MISMATCH'}  blake3(file[{off}:{off + ln}])")
    return 0 if ok else 1


def main() -> int:
    a = sys.argv[1:]
    try:
        if a == ["--self-test"]:
            return 0 if self_test() else 1
        if a and a[0] == "build" and len(a) >= 2:
            chunk = int(a[a.index("--chunk") + 1]) if "--chunk" in a else 65536
            source = a[a.index("--source") + 1] if "--source" in a else ""
            return build(a[1], chunk, source)
        if a and a[0] == "check" and len(a) in (3, 4):
            return check(a[1], a[2], a[3] if len(a) == 4 else None)
    except (OSError, ValueError, KeyError, IndexError) as e:
        sys.stderr.write(f"{e}\n")
        return 2
    sys.stderr.write(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main())
