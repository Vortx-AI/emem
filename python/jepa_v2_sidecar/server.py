"""GPU inference sidecar for emem-server.

Holds GPU-resident models in process, exposes them over a Unix socket.
The Rust process calls in for any inference that would otherwise hit
the broken in-process ort+CUDA path.

Phase 2 ships:
- /health with VRAM accounting
- /predict/dynamics_v2 (Phase 1 model executed via PyTorch on CUDA
  for parity with the Rust CPU path; both produce identical output
  on the zero-init sentinel)

Phase 3+4 add /predict/v_jepa_2 and /predict/prithvi_eo2.
"""

from __future__ import annotations

import hashlib
import json
import os
import sys
import time
from pathlib import Path
from typing import Any

import numpy as np
import torch
import torch.nn as nn
import torch.nn.functional as F
from fastapi import FastAPI, HTTPException
from pydantic import BaseModel, Field, ValidationError, field_validator

# ── Hard VRAM budget ──────────────────────────────────────────────────────
# Operator instruction: keep this sidecar under 10 GB on the shared
# A100 so geoqa-models and intruder don't get OOM'd. Each model
# allocates a slice and clamps via set_per_process_memory_fraction.
TOTAL_BUDGET_GB = float(os.environ.get("EMEM_SIDECAR_VRAM_BUDGET_GB", "10"))
DYNAMICS_BUDGET_GB = 0.1                    # tiny MLP
V_JEPA_2_BUDGET_GB = 3.0                    # ViT-L on 256x256x64 frames
PRITHVI_BUDGET_GB = 3.0
RESERVE_BUDGET_GB = TOTAL_BUDGET_GB - (
    DYNAMICS_BUDGET_GB + V_JEPA_2_BUDGET_GB + PRITHVI_BUDGET_GB
)
assert RESERVE_BUDGET_GB > 0, "VRAM budget over-allocated; rebalance constants"

EMEM_DATA = Path(os.environ.get("EMEM_DATA", "/home/ubuntu/emem/var/emem"))
DYNAMICS_DIR = Path(os.environ.get("EMEM_JEPA_V2_DIR", str(EMEM_DATA / "jepa_v2")))

# ── Phase 1 dynamics model (mirror of the Rust path) ──────────────────────
INPUT_LAGS = 3
TESSERA_DIM = 128
HIDDEN = 256
N_BLOCKS = 4


class ResidualBlock(nn.Module):
    """Pre-LN residual block — must match python/jepa_v2/train.py byte-for-byte
    so the same .onnx artifact loads in both places."""

    def __init__(self, dim: int, hidden: int, dropout: float = 0.10):
        super().__init__()
        self.norm = nn.LayerNorm(dim)
        self.fc1 = nn.Linear(dim, hidden)
        self.fc2 = nn.Linear(hidden, dim)
        self.drop = nn.Dropout(dropout)

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        return x + self.fc2(self.drop(F.gelu(self.fc1(self.norm(x)))))


class DynamicsModel(nn.Module):
    def __init__(
        self,
        dim: int = TESSERA_DIM,
        lags: int = INPUT_LAGS,
        hidden: int = HIDDEN,
        n_blocks: int = N_BLOCKS,
    ):
        super().__init__()
        self.proj_in = nn.Linear(dim * lags, dim)
        self.blocks = nn.ModuleList(
            [ResidualBlock(dim, hidden) for _ in range(n_blocks)]
        )
        self.head = nn.Linear(dim, dim)

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        last = x[:, -1, :]
        h = self.proj_in(x.flatten(1))
        for blk in self.blocks:
            h = blk(h)
        return last + self.head(h)


# ── Lazy model registry ───────────────────────────────────────────────────
class _Registry:
    """Process-local registry of loaded models. Lazy: load on first
    use, keep resident for the process lifetime (matches the operator
    instruction "no model offload after restart")."""

    def __init__(self) -> None:
        self.dynamics: tuple[DynamicsModel, dict[str, Any]] | None = None
        self.dynamics_load_at: float | None = None
        self.device = torch.device("cuda" if torch.cuda.is_available() else "cpu")

    def cuda_props(self) -> dict[str, Any]:
        if self.device.type != "cuda":
            return {"available": False}
        free_b, total_b = torch.cuda.mem_get_info(0)
        used_b = total_b - free_b
        return {
            "available": True,
            "device": torch.cuda.get_device_name(0),
            "total_gb": round(total_b / 1024**3, 2),
            "used_by_all_processes_gb": round(used_b / 1024**3, 2),
            "free_gb": round(free_b / 1024**3, 2),
            "this_process_allocated_gb": round(
                torch.cuda.memory_allocated(0) / 1024**3, 4
            ),
            "this_process_reserved_gb": round(
                torch.cuda.memory_reserved(0) / 1024**3, 4
            ),
            "budget_total_gb": TOTAL_BUDGET_GB,
            "budget_remaining_for_this_process_gb": round(
                TOTAL_BUDGET_GB
                - torch.cuda.memory_reserved(0) / 1024**3,
                2,
            ),
        }

    def load_dynamics(self) -> tuple[DynamicsModel, dict[str, Any]]:
        if self.dynamics is not None:
            return self.dynamics
        # Bound this model's slice of VRAM so an OOM here doesn't
        # spill into the others' budgets.
        if self.device.type == "cuda":
            torch.cuda.set_per_process_memory_fraction(
                DYNAMICS_BUDGET_GB / float(TOTAL_BUDGET_GB), device=0
            )
        meta_path = DYNAMICS_DIR / "dynamics_v2.metadata.json"
        if not meta_path.exists():
            raise FileNotFoundError(
                f"dynamics_v2 metadata not found at {meta_path}; "
                "run python/jepa_v2/export_baseline.py to seed the sentinel"
            )
        meta = json.loads(meta_path.read_text())
        # CORRECTNESS GUARD: this branch supports the zero-init
        # baseline ONLY. We rebuild the PyTorch architecture and
        # zero out the head so output == last_input_vintage by
        # construction — byte-identical to the Rust ort path on
        # the sentinel.
        #
        # When metadata claims `training.trained == true` we MUST
        # load actual learned weights (state_dict.pt or onnx →
        # torch import) before serving — otherwise we'd emit
        # randomly-initialised PyTorch outputs under a "trained"
        # receipt, which would corrupt every downstream
        # comparison + inflate verifier trust. That loader hasn't
        # shipped yet, so refuse-to-serve until it does.
        trained = bool(meta.get("training", {}).get("trained", False))
        if trained:
            raise RuntimeError(
                "dynamics_v2 metadata claims trained=true but the sidecar's "
                "trained-checkpoint loader has not shipped yet. Refusing to "
                "serve random PyTorch weights under a 'trained' receipt. "
                "Either set training.trained=false in the metadata to fall "
                "back to the residual-zero sentinel, or implement the ONNX "
                "→ torch state_dict loader at server.py:_Registry.load_dynamics."
            )
        model = DynamicsModel().to(self.device)
        with torch.no_grad():
            model.head.weight.zero_()
            model.head.bias.zero_()
        model.eval()
        self.dynamics = (model, meta)
        self.dynamics_load_at = time.time()
        return self.dynamics


_REG = _Registry()


# ── Schemas ──────────────────────────────────────────────────────────────
class DynamicsRequest(BaseModel):
    """K=3 prior 128-D Tessera vintages, oldest first."""

    lags: list[list[float]] = Field(..., description=f"shape [{INPUT_LAGS}, {TESSERA_DIM}]")

    @field_validator("lags")
    @classmethod
    def check_shape(cls, v: list[list[float]]) -> list[list[float]]:
        if len(v) != INPUT_LAGS:
            raise ValueError(
                f"`lags` must have exactly {INPUT_LAGS} entries (got {len(v)})"
            )
        for i, row in enumerate(v):
            if len(row) != TESSERA_DIM:
                raise ValueError(
                    f"`lags[{i}]` must have {TESSERA_DIM} dims (got {len(row)})"
                )
        return v


class DynamicsResponse(BaseModel):
    prediction: list[float]
    model: dict[str, Any]
    inference_us: int
    device: str


# ── App ───────────────────────────────────────────────────────────────────
app = FastAPI(
    title="emem-jepa-sidecar",
    description="GPU inference sidecar for emem-server",
    version="0.0.1",
)


@app.get("/health")
def health() -> dict[str, Any]:
    models: list[str] = []
    if _REG.dynamics is not None:
        models.append("dynamics_v2")
    return {
        "status": "ok",
        "models_loaded": models,
        "device": str(_REG.device),
        "cuda": _REG.cuda_props(),
        "uptime_s": int(time.monotonic()),
        "pid": os.getpid(),
    }


@app.post("/predict/dynamics_v2")
def predict_dynamics(req: DynamicsRequest) -> DynamicsResponse:
    try:
        model, meta = _REG.load_dynamics()
    except FileNotFoundError as e:
        raise HTTPException(status_code=503, detail=str(e)) from e
    except RuntimeError as e:
        # The "trained-but-no-loader" guard. 503 so the Rust client
        # surfaces it as Upstream (502) — emem-server then refuses
        # to silently fall back to the in-process CPU path because
        # the user explicitly attested a trained checkpoint.
        raise HTTPException(status_code=503, detail=str(e)) from e

    arr = np.asarray(req.lags, dtype=np.float32)            # [3, 128]
    x = torch.from_numpy(arr).unsqueeze(0).to(_REG.device)  # [1, 3, 128]
    t0 = time.perf_counter_ns()
    with torch.inference_mode():
        pred = model(x)                                     # [1, 128]
    dt_us = max(1, (time.perf_counter_ns() - t0) // 1000)
    pred_list = pred.squeeze(0).cpu().tolist()

    # Receipt-shape model block — mirrors what the Rust runtime
    # surfaces. The sidecar is the single source of truth when it's
    # answering; the Rust caller forwards this verbatim into the
    # signed receipt.
    onnx_path = DYNAMICS_DIR / "dynamics_v2.onnx"
    artifact_blake2b = (
        meta.get("artifact", {}).get("blake2b_hex")
        or hashlib.blake2b(
            onnx_path.read_bytes() if onnx_path.exists() else b"", digest_size=32
        ).hexdigest()
    )
    return DynamicsResponse(
        prediction=pred_list,
        model={
            "model_id": meta.get("model_id", "jepa_temporal_predictor@2"),
            "version": meta.get("version", "0.0.0-untrained-baseline"),
            "blake2b_hex": artifact_blake2b,
            "trained": meta.get("training", {}).get("trained", False),
            "via": "python_sidecar",
            "honesty_warnings": (
                []
                if meta.get("training", {}).get("trained", False)
                else [
                    "untrained_baseline: this jepa_v2 model is the residual-zero-init "
                    "sentinel that returns last_input_vintage by construction. "
                    "Quality is the 'predict last vintage' baseline, NOT a learned "
                    "forecast. Run python/jepa_v2/train.py to ship a real model."
                ]
            ),
        },
        inference_us=dt_us,
        device=str(_REG.device),
    )


@app.exception_handler(ValidationError)
def on_validation_error(_request, exc: ValidationError):
    """Pydantic validation error → 400 with structured detail."""
    raise HTTPException(status_code=400, detail=str(exc))


def main() -> None:
    """Entrypoint when run as `python server.py`. Production uses
    `uvicorn server:app --uds ...` so this main() exists for dev.
    """
    import uvicorn

    sock = os.environ.get("EMEM_SIDECAR_SOCK", "/tmp/emem-jepa.sock")
    print(f"emem-jepa-sidecar listening on {sock}", file=sys.stderr, flush=True)
    uvicorn.run(app, uds=sock, log_level="info")


if __name__ == "__main__":
    main()
