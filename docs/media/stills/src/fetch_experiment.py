"""Fetch the signed records behind experiment-chain.webp. Read-only calls to emem.dev.

One experiment of the methods lab (key mujzmxes), canopy-loss-scaled-2024-2025-2026-10-29, the one that
docs/research/canopy-loss-2026-10.md reports in section 5.5: its plan note, its result note, its experiment@1
record (JSON and CBOR), and a log inclusion proof for each of the three.
usage: python fetch_experiment.py    (writes data/experiment/*)
"""
import json
import time
import urllib.request
from pathlib import Path

D = Path(__file__).resolve().parent / "data" / "experiment"
D.mkdir(parents=True, exist_ok=True)
API = "https://emem.dev"
EXP = "/memories/by_attester/mujzmxes/research/experiments/canopy-loss-scaled-2024-2025-2026-10-29"
RECORD = "mw6r3fd4cwvn6giqrk43a6hj4fnaklsku5fk3nsie7azsqinal3a"
UA = {"user-agent": "emem-docs-media-still"}


def get(path, accept="application/json"):
    req = urllib.request.Request(API + path, headers={**UA, "accept": accept})
    with urllib.request.urlopen(req, timeout=60) as r:
        return r.read()


def view(path):
    body = {"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "emem_memory_view", "arguments": {"path": path}}}
    req = urllib.request.Request(f"{API}/mcp", data=json.dumps(body).encode(),
                                 headers={**UA, "content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=60) as r:
        res = json.load(r)["result"]
    return res.get("structuredContent") or json.loads(res["content"][0]["text"])


for name in ("plan", "result"):
    v = view(f"{EXP}/{name}.md")
    assert v["path"] == f"{EXP}/{name}.md" and v["content"], name
    (D / f"{name}.json").write_text(json.dumps(v, indent=1))
    time.sleep(0.5)
    inc = get(v["log"]["inclusion"])
    (D / f"inclusion_{name}.json").write_bytes(inc)
    time.sleep(0.5)
(D / "record.json").write_bytes(get(f"/v1/facts/{RECORD}"))
(D / "record.cbor").write_bytes(get(f"/v1/facts/{RECORD}", "application/cbor"))
(D / "inclusion_record.json").write_bytes(get(f"/v1/log/inclusion?fact_cid={RECORD}"))
print("wrote", sorted(p.name for p in D.iterdir()))
