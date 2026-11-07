"""Values for the film emem-log-climb, computed offline from saved live log reads. Nothing is sent to emem.dev.

Input: docs/media/stills/src/data/log/ (live responses from emem.dev on 7 November 2026, the same files that
docs/media/stills/log-proof.webp uses): inclusion_fuji.json, sth.json, consistency.json, recall.json, and
entry_fuji.json + entry_fuji_summary.json (GET /v1/log/entries?start=2463431&end=2463432, with and without
summary=1). A log entry is an attestation, not a fact; this one attestation holds exactly one fact, the Fuji one.
Every hash is computed with verify_log.py from the emem-transparency-log skill, and the script stops if the
climb does not end at the signed root, or if the tampered proof still ends there.
usage: python3 log_climb_data.py   (writes log-climb-data.js next to this script)
"""
import base64
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
DATA = ROOT / "docs/media/stills/src/data/log"
sys.path.insert(0, str(ROOT / "plugins/emem/skills/emem-transparency-log/scripts"))
import verify_log as vl  # noqa: E402


def load(name):
    return json.loads((DATA / name).read_text())


def b32(b: bytes) -> str:
    return base64.b32encode(b).decode().lower().rstrip("=")


def ranges(lo, hi, i):
    """RFC 6962 audit path as leaf ranges, bottom first: the subtree each path hash stands for."""
    if hi - lo == 1:
        return []
    k = 1
    while k * 2 < hi - lo:
        k *= 2
    if i < lo + k:
        return ranges(lo, lo + k, i) + [(lo + k, hi)]
    return ranges(lo + k, hi, i) + [(lo, lo + k)]


inc, sth0, cons, rec, ent, summ = (load(f) for f in (
    "inclusion_fuji.json", "sth.json", "consistency.json", "recall.json", "entry_fuji.json", "entry_fuji_summary.json"))
fact = rec["facts"][0]
sth1 = inc["sth"]
n, m = int(inc["tree_size"]), int(inc["leaf_index"])
path_b32 = inc["audit_path_b32"]
path = [vl.b32d(p) for p in path_b32]
leaf = vl.b32d(inc["leaf_hash_b32"])
root = vl.b32d(sth1["root_b32"])
assert inc["fact_cid"] == fact["fact_cid"] and inc["matched"] == "fact_cid"
assert leaf == vl.blake3(b"\x00" + vl.b32d(inc["entry_hash_b32"])).digest()
assert vl.verify_sth(sth1) and vl.verify_sth(sth0["sth"])
assert vl.root_from_inclusion(leaf, m, n, path) == root
row, srow = ent["entries"][0], summ["entries"][0]
assert row["leaf_index"] == srow["leaf_index"] == m and row["entry_kind"] == "attestation"
assert vl.blake3(vl.b32d(row["entry_cbor_b32"])).digest() == vl.b32d(inc["entry_hash_b32"])
assert srow["facts"] == 1 and srow["band"] == fact["band"] and srow["cell"] == fact["cell"]

sib = ranges(0, n, m)
assert len(sib) == len(path)


def climb(hashes):
    acc, lo, hi, out = leaf, m, m + 1, []
    for (a, b), h in zip(sib, hashes):
        left = b <= m
        acc = vl.node(h, acc) if left else vl.node(acc, h)
        lo, hi = min(lo, a), max(hi, b)
        out.append({"a": a, "b": b, "left": left, "lo": lo, "hi": hi, "acc": b32(acc)})
    return acc, out


top, steps = climb(path)
assert top == root and (steps[-1]["lo"], steps[-1]["hi"]) == (0, n)

bad_i = 9
s = path_b32[bad_i]
bad_s = s[:20] + ("a" if s[20] != "a" else "b") + s[21:]
bad = list(path)
bad[bad_i] = vl.b32d(bad_s)
bad_top, bad_steps = climb(bad)
assert bad_top != root

first, second = int(sth0["sth"]["tree_size"]), n
proof = [vl.b32d(p) for p in cons["consistency_proof_b32"]]
assert cons["first_size"] == first and cons["second_size"] == second
assert vl.consistent(first, vl.b32d(sth0["sth"]["root_b32"]), second, root, proof)

out = {
    "fact": {"value": fact["value"], "band": fact["band"], "cell": fact["cell"], "fact_cid": fact["fact_cid"]},
    "leaf_index": m, "tree_size": n, "entry_bytes": srow["entry_bytes"], "attested_at": srow["attested_at"], "leaf": inc["leaf_hash_b32"], "entry_hash": inc["entry_hash_b32"],
    "path": path_b32, "steps": steps,
    "sth": {"root": sth1["root_b32"], "signed_at": sth1["signed_at"], "pubkey": sth1["responder_pubkey_b32"]},
    "bad": {"i": bad_i, "orig": s, "hash": bad_s, "pos": 20, "root": b32(bad_top),
            "acc": [x["acc"] for x in bad_steps]},
    "cons": {"first": first, "first_root": sth0["sth"]["root_b32"], "first_at": sth0["sth"]["signed_at"],
             "second": second, "hashes": len(proof)},
}
(HERE / "log-climb-data.js").write_text("window.LOG = " + json.dumps(out, indent=1) + ";\n")
print("steps", len(steps), "root ok, tamper breaks at", b32(bad_top)[:8], "consistency ok", first, "->", second)
