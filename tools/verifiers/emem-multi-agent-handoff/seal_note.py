#!/usr/bin/env python3
"""Seal a text file so that only one emem key can read it (emem.sealed.v1).

    seal_note.py --to RECIPIENT_PUBKEY_B32 --author YOUR_PUBKEY_B32 FILE [-o OUT]

FILE is the text to seal, or - for standard input. Both keys are full
52-character pubkey_b32 values: the recipient's, and your own from
signer_whoami. The output is the note text to write with
emem_memory_create in your namespace. Its front matter says `to:` the
recipient, so the note reaches their inbox. emem_sealed.py beside this
script documents the format.

The note is public: its author, recipient key, time and size stay
readable to everyone, and the ciphertext stays in the log for good. Only
the words are hidden. No network calls. Needs the `cryptography` package.
"""
from __future__ import annotations

import argparse
import sys
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))
from emem_sealed import SealedError, seal  # noqa: E402


def main() -> int:
    ap = argparse.ArgumentParser(description="Seal a text file to one emem key (emem.sealed.v1).")
    ap.add_argument("file", help="the text to seal, or - for standard input")
    ap.add_argument("--to", required=True, help="the recipient's full pubkey_b32")
    ap.add_argument("--author", required=True, help="your own full pubkey_b32, from signer_whoami")
    ap.add_argument("-o", "--out", help="write the note here instead of standard output")
    a = ap.parse_args()
    try:
        plain = sys.stdin.buffer.read() if a.file == "-" else Path(a.file).read_bytes()
    except OSError as e:
        sys.stderr.write(f"cannot read {a.file}: {e}\n")
        return 2
    try:
        note = seal(plain, a.to, a.author)
    except SealedError as e:
        sys.stderr.write(f"not sealed: {e}\n")
        return 2
    if a.out:
        Path(a.out).write_text(note, encoding="utf-8")
    else:
        sys.stdout.write(note)
    sys.stderr.write(f"sealed {len(plain)} bytes to {a.to.strip().lower()[:8]}. Write the note in your own "
                     "namespace; the signature then says you wrote it.\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
