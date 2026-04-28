#!/usr/bin/env python3
"""Measure the cell64 alphabet (spec §3.2 / OQ-1) from tokenizer corpora.

Produces `crates/emem-codec/data/cell64-alphabet-v0.bin`: 65,536 bigram strings,
each encoded as a length-prefixed UTF-8 byte string, in Hilbert-curve order.

Algorithm:
  1. For each tokenizer (cl100k_base, o200k_base, llama-3-bpe, claude),
     enumerate single-token strings of length 2-4 chars.
  2. Intersect: keep strings that are single-token in ALL four tokenizers.
  3. Filter to BPE-friendly bigram patterns (CV / VC / CVC / VCV).
  4. Score each candidate by mean rank across tokenizers; keep top 65,536.
  5. Order along a 256x256 Hilbert curve so adjacent indices map to spatially
     adjacent cell-children (spec §3.2 locality property).
  6. Write binary blob.

Usage:
  python tools/measure_alphabet.py --out crates/emem-codec/data/cell64-alphabet-v0.bin

Note: v0.0.2 ships a synthetic placeholder alphabet in
crates/emem-codec/src/alphabet.rs. This tool replaces it in v0.0.3 once
tokenizer access is wired up.
"""
from __future__ import annotations

import argparse
import struct
import sys
from pathlib import Path

TARGET_SIZE = 65_536


def hilbert_order(side: int = 256) -> list[int]:
    """Return [0..side*side) reordered along a Hilbert curve."""
    out = [0] * (side * side)
    for d in range(side * side):
        x, y = _d_to_xy(side, d)
        out[d] = y * side + x
    return out


def _d_to_xy(side: int, d: int) -> tuple[int, int]:
    x, y, t = 0, 0, d
    s = 1
    while s < side:
        rx = 1 & (t // 2)
        ry = 1 & (t ^ rx)
        if ry == 0:
            if rx == 1:
                x = s - 1 - x
                y = s - 1 - y
            x, y = y, x
        x += s * rx
        y += s * ry
        t //= 4
        s *= 2
    return x, y


def synthesize_placeholder() -> list[str]:
    """Synthesize a placeholder 65,536-entry alphabet with the same
    surface shape as the real one. Real impl plugs in tokenizer corpora.
    """
    cons = "bcdfghjklmnpqrstvwxyz"
    vows = "aeiouAEIOU"
    out: list[str] = []
    for c1 in cons:
        for v1 in vows:
            for c2 in cons:
                for v2 in vows:
                    out.append(f"{c1}{v1}{c2}{v2}")
                    if len(out) >= TARGET_SIZE:
                        return out
    return out


def write_blob(alphabet: list[str], path: Path) -> None:
    with path.open("wb") as f:
        for sym in alphabet:
            data = sym.encode("utf-8")
            if len(data) > 255:
                raise ValueError(f"symbol too long: {sym}")
            f.write(struct.pack("B", len(data)))
            f.write(data)


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--out", required=True, type=Path,
                   help="Output binary blob path.")
    p.add_argument("--placeholder", action="store_true",
                   help="Emit the synthetic placeholder alphabet (default until v0.0.3).")
    args = p.parse_args(argv)

    if not args.placeholder:
        print("Real measurement not yet wired up. Use --placeholder for now.",
              file=sys.stderr)
        return 2

    alphabet = synthesize_placeholder()
    if len(alphabet) != TARGET_SIZE:
        print(f"alphabet size {len(alphabet)} != target {TARGET_SIZE}",
              file=sys.stderr)
        return 1

    order = hilbert_order(256)
    reordered = [alphabet[i] for i in order]

    args.out.parent.mkdir(parents=True, exist_ok=True)
    write_blob(reordered, args.out)
    print(f"Wrote {len(reordered)} symbols to {args.out} "
          f"({args.out.stat().st_size} bytes).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
