"""Minimal end-to-end smoke example: locate → recall → cite a receipt.

Run against the public hosted instance:

    python examples/recall.py

Or against a local responder:

    EMEM_BASE_URL=http://localhost:5051 python examples/recall.py
"""

from __future__ import annotations

from emem import Client


def main() -> None:
    with Client() as em:
        loc = em.locate("Mount Fuji")
        cell = loc["cell64"]
        print(f"resolved Mount Fuji → cell64 = {cell}")

        facts = em.recall(cell, bands=["copdem30m.elevation_mean"])
        for f in facts.get("facts", []):
            print(
                f"  {f.get('band')} = {f.get('value')} "
                f"(cid={f.get('cid')[:12]}…, tslot={f.get('tslot')})"
            )

        receipt = facts.get("receipt") or {}
        print(
            f"  receipt: signed by {receipt.get('responder_pubkey', '?')[:16]}…"
            f" over {len(receipt.get('fact_cids') or [])} fact CID(s)"
        )


if __name__ == "__main__":
    main()
