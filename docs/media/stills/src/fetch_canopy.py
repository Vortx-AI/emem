"""Fetch the signed result notes behind canopy-rules.webp. Read-only calls to emem.dev.

Three experiment results of the methods lab (key mujzmxes), the ones that docs/research/canopy-loss-2026-10.md
reports in sections 5.5, 5.6 and 5.8, read with the MCP tool emem_memory_view.
usage: python fetch_canopy.py    (writes data/canopy/*.json)
"""
import json
import urllib.request
from pathlib import Path

D = Path(__file__).resolve().parent / "data" / "canopy"
D.mkdir(parents=True, exist_ok=True)
API = "https://emem.dev"
ROOT = "/memories/by_attester/mujzmxes/research/experiments"
NOTES = {
    "scaled": "canopy-loss-scaled-2024-2025-2026-10-29",
    "mcc-replication": "canopy-loss-mcc-replication-2024-2025-2026-10-29",
    "third-replication": "canopy-loss-third-replication-2024-2025-2026-10-29",
}


def view(path):
    body = {"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "emem_memory_view", "arguments": {"path": path}}}
    req = urllib.request.Request(f"{API}/mcp", data=json.dumps(body).encode(),
                                 headers={"content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=60) as r:
        res = json.load(r)["result"]
    return res.get("structuredContent") or json.loads(res["content"][0]["text"])


for name, key in NOTES.items():
    v = view(f"{ROOT}/{key}/result.md")
    assert v["path"].endswith(f"{key}/result.md") and v["content"], name
    (D / f"{name}.json").write_text(json.dumps(v, indent=1))
print("wrote", sorted(p.name for p in D.glob("*.json")))
