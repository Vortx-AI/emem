"""LangChain tools for emem, the Earth memory protocol.

Copy this file next to your agent code as `langchain_emem.py`. Do not keep
the name `langchain.py`: it would shadow the `langchain` package. The nine
tools wrap the emem REST API with the LangChain `@tool` decorator. Any
LangChain or LangGraph agent can call them. The hosted responder at
https://emem.dev uses HTTPS only. Reads need no key.

Install (langchain-openai is for the model in the usage example):
    pip install langchain langchain-core requests langchain-openai

Usage:
    from langchain_emem import emem_tools
    from langchain.agents import create_agent

    agent = create_agent(model="openai:gpt-4.1-mini", tools=emem_tools)
    agent.invoke({"messages": [{"role": "user",
        "content": "What is the elevation at cell defi.zb592.nUkO.zEzE?"}]})

Tested on 7 November 2026 with langchain 1.4.9 and langchain-core 1.7.6.
Each tool returned a live answer from emem.dev. The agent reached the
OpenAI call; we used a test key, so the model part did not run.
Band names are full ids such as `copdem30m.elevation_mean` (GET /v1/bands).
A tslot is a day number since 1970-01-01.
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
    """Compare two cells. Returns per-band delta (b minus a) and a signed receipt.

    Cosine is set only when both cells hold the same vector band.
    """
    body: dict[str, Any] = {"a": a, "b": b}
    if family: body["family"] = family
    return _post("/v1/compare", body)

@tool
def emem_find_similar(key: str, k: int = 10, band: str = "geotessera") -> dict:
    """k-NN over the corpus. key = cell64 or 'inline:[x,y,...]'.

    The geotessera band is retired on emem.dev. Its index is frozen, so a
    cell with no stored vector gets no neighbours.
    """
    return _post("/v1/find_similar", {"key": key, "k": k, "band": band})

@tool
def emem_diff(cell: str, band: str, tslot_a: int, tslot_b: int) -> dict:
    """Compute a signed delta fact between two tslots at one cell and band.

    Both tslots must hold a fact. Get them from emem_trajectory first.
    """
    return _post("/v1/diff", {"cell": cell, "band": band, "tslot_a": tslot_a, "tslot_b": tslot_b})

@tool
def emem_trajectory(cell: str, band: str, window: list[int]) -> dict:
    """Time series for (cell, band) over an inclusive [start, end] tslot window."""
    return _post("/v1/trajectory", {"cell": cell, "band": band, "window": window})

@tool
def emem_verify(claim: dict, cell: str, mode: str = "fast") -> dict:
    """Verify a claim {band, op, value, tslot} against the facts at a cell.

    Returns verdict (true or false) and evidence fact ids. Empty evidence
    means the cell holds no fact for that band, so nothing was checked.
    """
    return _post("/v1/verify", {"claim": claim, "cell": cell, "mode": mode})

@tool
def emem_intent(intent: dict) -> dict:
    """Send a typed intent. emem plans the calls, runs them and returns results.

    The intent must have a `type`: where_is, what_is_here, is_like,
    did_change, find_like, confirm, ask or area_over_time. Example:
    {"type": "what_is_here", "place": "Mount Fuji"}.
    """
    return _post("/v1/intent", intent)

@tool
def emem_bands() -> dict:
    """List the active bands. Call once at session start."""
    return requests.get(f"{EMEM}/v1/bands", timeout=TIMEOUT).json()

@tool
def emem_agent_card() -> dict:
    """Rich agent-onboarding card: tools, when-to-use, manifests, surfaces."""
    return requests.get(f"{EMEM}/v1/agent_card", timeout=TIMEOUT).json()

emem_tools = [
    emem_recall, emem_compare, emem_find_similar, emem_diff,
    emem_trajectory, emem_verify, emem_intent, emem_bands, emem_agent_card,
]
