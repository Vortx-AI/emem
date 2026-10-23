"""Turn a Usage record into dollars using prices.json."""
from __future__ import annotations

import json
from pathlib import Path

DEFAULT_PRICES = Path(__file__).resolve().parent / "prices.json"


def load_prices(path: Path | str = DEFAULT_PRICES) -> dict:
    return json.loads(Path(path).read_text())


def cost_usd(usage, prices: dict) -> float:
    """Dollar cost of one Usage (object or dict). Zero usage costs zero.

    Raises if usage is non-zero and the model has no filled-in price: a missing
    price must never silently become a free run.
    """
    u = usage if isinstance(usage, dict) else usage.as_dict()
    if not (u["input_tokens"] or u["output_tokens"] or u["tool_calls"]):
        return 0.0
    p = prices.get("models", {}).get(u["model_id"])
    if p is None or p.get("input_per_mtok") is None or p.get("output_per_mtok") is None:
        raise ValueError(f"no price for model {u['model_id']!r}: fill prices.json (currently TO BE FILLED)")
    return (u["input_tokens"] * p["input_per_mtok"] + u["output_tokens"] * p["output_per_mtok"]) / 1e6 \
        + u["tool_calls"] * p.get("tool_call_usd", 0.0)
