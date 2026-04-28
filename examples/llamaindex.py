"""LlamaIndex tools for emem — Earth memory protocol.

Install:
    pip install llama-index-core requests

Usage:
    from llamaindex_emem import emem_tool_spec
    from llama_index.core.agent import ReActAgent
    from llama_index.llms.openai import OpenAI  # or any provider

    agent = ReActAgent.from_tools(
        tools=emem_tool_spec(),
        llm=OpenAI(model="gpt-4.1-mini"),
        verbose=True,
    )
    agent.chat("What's at cell ento.bria.calo.tris?")
"""
import os
from typing import Any
import requests
from llama_index.core.tools import FunctionTool

EMEM = os.environ.get("EMEM_URL", "https://emem.dev")
TIMEOUT = 30

def _post(path: str, body: dict[str, Any]) -> dict[str, Any]:
    r = requests.post(f"{EMEM}{path}", json=body, timeout=TIMEOUT)
    r.raise_for_status()
    return r.json()

def emem_recall(cell: str) -> dict:
    """Recall facts at an emem cell64 string. Always cite receipt.fact_cids."""
    return _post("/v1/recall", {"cell": cell})

def emem_compare(a: str, b: str) -> dict:
    """Compare two cells; cosine similarity + per-band delta + signed receipt."""
    return _post("/v1/compare", {"a": a, "b": b})

def emem_find_similar(key: str, k: int = 10) -> dict:
    """Find k cells most similar to `key` (a cell64 or 'inline:[...]')."""
    return _post("/v1/find_similar", {"key": key, "k": k})

def emem_diff(cell: str, band: str, tslot_a: int, tslot_b: int) -> dict:
    """Compute the band delta between two tslots at a cell."""
    return _post("/v1/diff", {"cell": cell, "band": band, "tslot_a": tslot_a, "tslot_b": tslot_b})

def emem_verify(claim: dict, cell: str) -> dict:
    """Verify a structured claim {band, op, value, tslot|window} against a cell."""
    return _post("/v1/verify", {"claim": claim, "cell": cell})

def emem_agent_card() -> dict:
    """Rich agent-onboarding card — call once to learn tools + surfaces."""
    return requests.get(f"{EMEM}/v1/agent_card", timeout=TIMEOUT).json()

def emem_tool_spec() -> list[FunctionTool]:
    return [
        FunctionTool.from_defaults(fn=emem_recall, name="emem_recall",
            description="Recall facts at an emem cell64. Returns signed receipt."),
        FunctionTool.from_defaults(fn=emem_compare, name="emem_compare",
            description="Compare two emem cells; cosine + per-band delta."),
        FunctionTool.from_defaults(fn=emem_find_similar, name="emem_find_similar",
            description="k-NN over the emem corpus by cell or inline vector."),
        FunctionTool.from_defaults(fn=emem_diff, name="emem_diff",
            description="Delta in a band's value between two tslots at a cell."),
        FunctionTool.from_defaults(fn=emem_verify, name="emem_verify",
            description="Verify a structured spatial claim; returns verdict + evidence CIDs."),
        FunctionTool.from_defaults(fn=emem_agent_card, name="emem_agent_card",
            description="Discover the emem protocol surface; call once at session start."),
    ]
