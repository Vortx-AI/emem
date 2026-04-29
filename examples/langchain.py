"""LangChain tools for emem — Earth memory protocol.

Drop this file next to your agent code. The tools wrap the live emem
HTTPS REST surface with type-checked LangChain Tool decorators so any
LangGraph / LangChain agent can call them directly. The hosted
responder at https://emem.dev is HTTPS-only; no keys required.

Install:
    pip install langchain langchain-core requests

Usage:
    from langchain_emem import emem_tools
    from langchain.agents import create_react_agent

    agent = create_react_agent(model="...", tools=emem_tools)
"""
import os
from typing import Any
import requests
from langchain_core.tools import tool

EMEM = os.environ.get("EMEM_URL", "https://emem.dev")
TIMEOUT = 30

def _post(path: str, body: dict[str, Any]) -> dict[str, Any]:
    r = requests.post(f"{EMEM}{path}", json=body, timeout=TIMEOUT)
    r.raise_for_status()
    return r.json()

@tool
def emem_recall(cell: str, bands: list[str] | None = None, tslot: int | None = None) -> dict:
    """Recall facts at an emem cell64 string. Returns signed receipt + fact list.

    Use first when the user names a place / cell. Cite the receipt's fact_cids
    in your reply.
    """
    body: dict[str, Any] = {"cell": cell}
    if bands is not None: body["bands"] = bands
    if tslot is not None: body["tslot"] = tslot
    return _post("/v1/recall", body)

@tool
def emem_compare(a: str, b: str, family: str | None = None) -> dict:
    """Compare two cells. Returns cosine + per-band delta + signed receipt."""
    body: dict[str, Any] = {"a": a, "b": b}
    if family: body["family"] = family
    return _post("/v1/compare", body)

@tool
def emem_find_similar(key: str, k: int = 10, band: str = "alphaearth") -> dict:
    """k-NN over the corpus. key = cell64 or 'inline:[x,y,...]'."""
    return _post("/v1/find_similar", {"key": key, "k": k, "band": band})

@tool
def emem_diff(cell: str, band: str, tslot_a: int, tslot_b: int) -> dict:
    """Compute a DerivativeFact (delta) between two tslots at one cell+band."""
    return _post("/v1/diff", {"cell": cell, "band": band, "tslot_a": tslot_a, "tslot_b": tslot_b})

@tool
def emem_trajectory(cell: str, band: str, window: list[int]) -> dict:
    """Time series for (cell, band) over an inclusive [start, end] tslot window."""
    return _post("/v1/trajectory", {"cell": cell, "band": band, "window": window})

@tool
def emem_verify(claim: dict, cell: str, mode: str = "fast") -> dict:
    """Verify a structured claim against a cell's facts. Returns verdict + evidence CIDs."""
    return _post("/v1/verify", {"claim": claim, "cell": cell, "mode": mode})

@tool
def emem_intent(intent: dict) -> dict:
    """Submit a typed Intent; the emem planner returns a Plan you can execute."""
    return _post("/v1/intent", intent)

@tool
def emem_bands() -> dict:
    """List the active band ontology — call once at session start."""
    return requests.get(f"{EMEM}/v1/bands", timeout=TIMEOUT).json()

@tool
def emem_agent_card() -> dict:
    """Rich agent-onboarding card: tools, when-to-use, manifests, surfaces."""
    return requests.get(f"{EMEM}/v1/agent_card", timeout=TIMEOUT).json()

emem_tools = [
    emem_recall, emem_compare, emem_find_similar, emem_diff,
    emem_trajectory, emem_verify, emem_intent, emem_bands, emem_agent_card,
]
