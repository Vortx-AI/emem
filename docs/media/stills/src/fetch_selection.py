"""Fetch the signed result notes behind method-selection.webp. Read-only calls to emem.dev.

The two method-selection results of the methods lab (key mujzmxes) that docs/research/method-selection.md reports in
sections 9 and 10, read with the MCP tool emem_memory_view.
usage: python fetch_selection.py    (writes data/selection/*.json)
"""
import json
import urllib.request
from pathlib import Path

D = Path(__file__).resolve().parent / "data" / "selection"
D.mkdir(parents=True, exist_ok=True)
API = "https://emem.dev"
ROOT = "/memories/by_attester/mujzmxes/research/experiments"
NOTES = {
    "first": "canopy-loss-method-selection-2022-2025-2026-10-30",
    "second": "canopy-loss-method-selection2-2022-2025-2026-11-05",
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
