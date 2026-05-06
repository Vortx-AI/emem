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

# Phase 3 — Prithvi-EO-2.0-300M-TL artifacts. We pin the snapshot path
# at startup; HF_HOME is set by the systemd unit. Future loads will
# read from this exact pinned snapshot (no HF lookup), so production
# is offline by construction once the cache is seeded. To upgrade,
# re-run the snapshot_download bootstrap and update PRITHVI_SNAPSHOT.
PRITHVI_REPO = "ibm-nasa-geospatial/Prithvi-EO-2.0-300M-TL"
PRITHVI_SNAPSHOT_DEFAULT = (
    EMEM_DATA / "hf_cache/hub/models--ibm-nasa-geospatial--Prithvi-EO-2.0-300M-TL/"
    "snapshots/63adbd39c271da4c42f447e69b1a7c91a338cdc9"
)
PRITHVI_SNAPSHOT = Path(
    os.environ.get("EMEM_PRITHVI_SNAPSHOT", str(PRITHVI_SNAPSHOT_DEFAULT))
)

# Phase 4 — Galileo-Tiny multimodal encoder (NASA Harvest, MIT).
# Tiny variant: 5.7 M params, 22 MB checkpoint, 192-D embedding. Pinned
# to the local HF snapshot path for offline-once-cached operation.
GALILEO_REPO = "nasaharvest/galileo"
GALILEO_VARIANT = "tiny"
GALILEO_SNAPSHOT_DEFAULT = (
    EMEM_DATA / "hf_cache/hub/models--nasaharvest--galileo/"
    "snapshots/f039dd5dde966a931baeda47eb680fa89b253e4e/models/tiny"
)
GALILEO_SNAPSHOT = Path(
    os.environ.get("EMEM_GALILEO_SNAPSHOT", str(GALILEO_SNAPSHOT_DEFAULT))
)
# S2-band normalization stats from the Galileo pretraining "13"
# normalizing_dict (S1 + S2 + NDVI). We slice out the 10 S2 entries
# in S2_BANDS order (B2/B3/B4/B5/B6/B7/B8/B8A/B11/B12).
GALILEO_S2_MEAN: list[float] = [
    1395.3408730676722,
    1338.4026921784578,
    1343.09883810357,
    1543.8607982512297,
    2186.2022069512263,
    2525.0932853316694,
    2410.3377187373408,
    2750.2854646886753,
    2234.911100061487,
    1474.5311266077113,
]
GALILEO_S2_STD: list[float] = [
    917.7041440370853,
    913.2988423581528,
    1092.678723527555,
    1047.2206083460424,
    1048.0101611156767,
    1143.6903026819996,
    1098.979177731649,
    1204.472755085893,
    1145.9774063078878,
    980.2429840007796,
]
# Galileo-Tiny chip shape — 8×8 spatial tokens at 30 m = 240 m extent
# centred on the cell. Patch_size=2 → 4×4 token grid (16 tokens per
# step). T=1 (single timestep). Fits Galileo's training distribution
# (shape_time_combinations include sizes 4–12).
GALILEO_CHIP_H: int = 8
GALILEO_CHIP_W: int = 8
GALILEO_CHIP_T: int = 1
GALILEO_PATCH_SIZE: int = 2

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
        self.prithvi: tuple[Any, dict[str, Any]] | None = None
        self.prithvi_load_at: float | None = None
        self.galileo: tuple[Any, dict[str, Any]] | None = None
        self.galileo_load_at: float | None = None
        self.device = torch.device("cuda" if torch.cuda.is_available() else "cpu")
        # Hard per-process VRAM cap, set ONCE at startup. The argument
        # to set_per_process_memory_fraction is a fraction of total
        # device memory, NOT of our budget — so we compute
        # TOTAL_BUDGET_GB / total_device_gb. Any future allocation that
        # would push the process past this cap raises CUDA OOM, which
        # surfaces as 503 to the Rust caller and the in-process CPU
        # fallback takes over. Per-model BUDGET_GB constants below are
        # advisory accounting only — they sum to TOTAL_BUDGET_GB and
        # describe how the cap is allocated, not separate hard caps.
        if self.device.type == "cuda":
            _, total_b = torch.cuda.mem_get_info(0)
            total_gb = total_b / 1024**3
            fraction = min(1.0, TOTAL_BUDGET_GB / total_gb)
            torch.cuda.set_per_process_memory_fraction(fraction, device=0)

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
        # No per-load set_per_process_memory_fraction — the global cap
        # is set once in __init__. Calling it again here would REPLACE
        # the global cap with one scaled to a single model's slice and
        # break the budget for any subsequent loads.
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

    def load_prithvi(self) -> tuple[Any, dict[str, Any]]:
        """Load Prithvi-EO-2.0-300M-TL from the pinned local snapshot.
        Returns (model, meta) where model is in eval mode on self.device,
        meta carries the receipt-shape `model` block (model_id, blake2b_hex,
        bands, mean/std, source paper) the Rust caller forwards into the
        signed receipt.

        We build with `num_frames=1` to match our per-cell input shape
        (T=1 timestep). The pos_embed reconstruction path in
        prithvi_mae.py constructs new tensors via numpy → CPU when T
        differs from grid_size; building with num_frames=1 keeps every
        tensor on `self.device` from the start.
        """
        if self.prithvi is not None:
            return self.prithvi
        # Vendor — `prithvi_mae.py` lives next to this file (Apache-2.0
        # from ibm-nasa-geospatial). Importing once at load time keeps
        # the cold-start cost on the first request, not on `import`.
        sys.path.insert(0, str(Path(__file__).parent))
        from prithvi_mae import PrithviMAE  # noqa: WPS433
        ckpt_path = PRITHVI_SNAPSHOT / "Prithvi_EO_V2_300M_TL.pt"
        cfg_path = PRITHVI_SNAPSHOT / "config.json"
        if not ckpt_path.exists() or not cfg_path.exists():
            raise FileNotFoundError(
                f"Prithvi-EO-2.0 artifacts not found at {PRITHVI_SNAPSHOT}. "
                f"Bootstrap with `huggingface_hub.snapshot_download(repo_id="
                f"'{PRITHVI_REPO}', allow_patterns=['*.py','*.json','*.txt','*.pt'], "
                f"cache_dir='{EMEM_DATA / 'hf_cache/hub'}')` (one-time, "
                f"~1.3 GB; production runs offline once cached)."
            )
        cfg = json.loads(cfg_path.read_text())["pretrained_cfg"]
        # T=1 — single timestep per request. The chip-fetch upgrade
        # (Phase 3b) may eventually feed multi-vintage stacks; when it
        # does, raise this and re-derive pos_embed on `self.device`.
        cfg["num_frames"] = 1
        ks = (
            "img_size", "num_frames", "patch_size", "in_chans", "embed_dim",
            "depth", "num_heads", "decoder_embed_dim", "decoder_depth",
            "decoder_num_heads", "mlp_ratio", "coords_encoding",
            "coords_scale_learn", "mask_ratio",
        )
        model = PrithviMAE(**{k: cfg[k] for k in ks})
        sd = torch.load(ckpt_path, map_location="cpu", weights_only=True)
        # Strip pos_embed buffers — the model rebuilds them deterministically
        # from img_size + patch_size + num_frames at forward time. Keeping
        # the upstream pos_embed (which assumes num_frames=4) would mismatch
        # our num_frames=1 build.
        for k in list(sd):
            if "pos_embed" in k:
                del sd[k]
        model.load_state_dict(sd, strict=False)
        model.eval().to(self.device)
        # Hash the .pt for the model_cid in the receipt — same blake2b-256
        # convention the protocol uses for fact CIDs.
        ckpt_blake2b = hashlib.blake2b(ckpt_path.read_bytes(), digest_size=32).hexdigest()
        meta = {
            "model_id": "prithvi_eo_v2_300m_tl",
            "version": "2.0.0",
            "license": "Apache-2.0",
            "source": "ibm-nasa-geospatial/Prithvi-EO-2.0-300M-TL (HF)",
            "paper": "arXiv:2412.02732 — Prithvi-EO-2.0",
            "checkpoint_filename": ckpt_path.name,
            "blake2b_hex": ckpt_blake2b,
            "size_bytes": ckpt_path.stat().st_size,
            "config": {
                "img_size": cfg["img_size"],
                "num_frames": cfg["num_frames"],
                "patch_size": cfg["patch_size"],
                "in_chans": cfg["in_chans"],
                "embed_dim": cfg["embed_dim"],
                "depth": cfg["depth"],
                "num_heads": cfg["num_heads"],
                "bands_in_order": [
                    "Blue (S2.B02)",
                    "Green (S2.B03)",
                    "Red (S2.B04)",
                    "Narrow NIR (S2.B8A)",
                    "SWIR1 (S2.B11)",
                    "SWIR2 (S2.B12)",
                ],
                "mean_per_band": cfg["mean"],
                "std_per_band": cfg["std"],
            },
        }
        self.prithvi = (model, meta)
        self.prithvi_load_at = time.time()
        return self.prithvi

    def load_galileo(self) -> tuple[Any, dict[str, Any]]:
        """Load Galileo-Tiny (NASA Harvest, MIT) from the pinned local
        snapshot. Single-file model code is vendored at
        `python/jepa_v2_sidecar/single_file_galileo.py`. Tiny variant:
        5.7 M params, 22 MB checkpoint, 192-D embedding.
        """
        if self.galileo is not None:
            return self.galileo
        sys.path.insert(0, str(Path(__file__).parent))
        from single_file_galileo import Encoder as GalileoEncoder  # noqa: WPS433
        if not (GALILEO_SNAPSHOT / "encoder.pt").exists():
            raise FileNotFoundError(
                f"Galileo-Tiny artifacts not found at {GALILEO_SNAPSHOT}. "
                f"Bootstrap with `huggingface_hub.snapshot_download(repo_id="
                f"'{GALILEO_REPO}', allow_patterns=['models/{GALILEO_VARIANT}/*'], "
                f"cache_dir='{EMEM_DATA / 'hf_cache/hub'}')` (one-time, "
                f"~58 MB; production runs offline once cached)."
            )
        encoder = GalileoEncoder.load_from_folder(GALILEO_SNAPSHOT, self.device)
        encoder.to(self.device).eval()
        ckpt_path = GALILEO_SNAPSHOT / "encoder.pt"
        ckpt_blake2b = hashlib.blake2b(
            ckpt_path.read_bytes(), digest_size=32
        ).hexdigest()
        cfg = json.loads((GALILEO_SNAPSHOT / "config.json").read_text())
        meta = {
            "model_id": f"galileo_{GALILEO_VARIANT}_v1",
            "version": "1.0.0",
            "license": "MIT",
            "source": f"{GALILEO_REPO} ({GALILEO_VARIANT})",
            "paper": "arXiv:2502.09356 — Galileo: Learning Global and Local Features in Pretrained Remote Sensing Models",
            "checkpoint_filename": ckpt_path.name,
            "blake2b_hex": ckpt_blake2b,
            "size_bytes": ckpt_path.stat().st_size,
            "config": {
                "embedding_size": encoder.embedding_size,
                "depth": cfg["model"]["encoder"]["depth"],
                "num_heads": cfg["model"]["encoder"]["num_heads"],
                "patch_size_used": GALILEO_PATCH_SIZE,
                "chip_h": GALILEO_CHIP_H,
                "chip_w": GALILEO_CHIP_W,
                "chip_t": GALILEO_CHIP_T,
                "modalities_used": ["S2"],
                "s2_bands_in_order": ["B2","B3","B4","B5","B6","B7","B8","B8A","B11","B12"],
                "s2_mean": GALILEO_S2_MEAN,
                "s2_std": GALILEO_S2_STD,
            },
        }
        self.galileo = (encoder, meta)
        self.galileo_load_at = time.time()
        return self.galileo


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


class PrithviRequest(BaseModel):
    """Per-cell Prithvi-EO-2.0 embedding request.

    `chip` is the 6-band reflectance window centred on the cell, in
    the order Blue / Green / Red / Narrow-NIR / SWIR1 / SWIR2 (matches
    HLS V2 product). Reflectance values in the upstream's native scale
    (typically 0–10000); the sidecar applies the model's mean/std
    normalization. Shape: `[6, H, W]` — H and W must equal the
    model's `img_size` (224 in Prithvi-EO-2.0).
    """

    chip: list[list[list[float]]] = Field(
        ..., description="reflectance chip, shape [6, 224, 224], HLS V2 band order"
    )
    # Optional metadata — Prithvi-EO-2.0-TL was pretrained with these,
    # so passing them sharpens the embedding when known. Both default
    # to None; the model handles the dropout case at inference.
    year: int | None = Field(default=None, description="acquisition year, e.g. 2024")
    julian_day: int | None = Field(
        default=None, ge=1, le=366, description="day-of-year 1..366"
    )
    lng: float | None = Field(default=None, description="centre longitude (WGS-84)")
    lat: float | None = Field(default=None, description="centre latitude (WGS-84)")

    @field_validator("chip")
    @classmethod
    def check_chip_shape(
        cls, v: list[list[list[float]]]
    ) -> list[list[list[float]]]:
        # Validate against the model's expected img_size (224). We don't
        # import the model just to read this — Prithvi-EO-2.0 fixes it
        # at 224 in config.json and the band count is locked to 6.
        if len(v) != 6:
            raise ValueError(f"`chip` must have 6 bands (got {len(v)})")
        for i, plane in enumerate(v):
            if len(plane) != 224:
                raise ValueError(
                    f"`chip[{i}]` must have 224 rows (got {len(plane)})"
                )
            for j, row in enumerate(plane):
                if len(row) != 224:
                    raise ValueError(
                        f"`chip[{i}][{j}]` must have 224 cols (got {len(row)})"
                    )
        return v


class PrithviResponse(BaseModel):
    embedding: list[float]
    embedding_dim: int
    model: dict[str, Any]
    inference_us: int
    device: str


class GalileoRequest(BaseModel):
    """Per-cell Galileo-Tiny S2-only embedding request.

    `s2_chip` is the 10-band S2 reflectance window in Galileo's
    S2_BANDS order (B2/B3/B4/B5/B6/B7/B8/B8A/B11/B12). Reflectance
    in upstream's native scale (typically 0–10000); the sidecar
    handles per-band normalization with the Galileo pretraining
    stats. Shape: `[T, H, W, 10]` where T must equal
    `GALILEO_CHIP_T` (1) and H, W must equal `GALILEO_CHIP_H/W` (8).
    """
    s2_chip: list[list[list[list[float]]]] = Field(
        ..., description="reflectance chip, shape [T=1, H=8, W=8, 10]"
    )
    month: int | None = Field(
        default=None, ge=1, le=12, description="month-of-year 1..12"
    )
    lng: float | None = Field(default=None, description="centre longitude (WGS-84)")
    lat: float | None = Field(default=None, description="centre latitude (WGS-84)")

    @field_validator("s2_chip")
    @classmethod
    def check_shape(
        cls, v: list[list[list[list[float]]]]
    ) -> list[list[list[list[float]]]]:
        if len(v) != GALILEO_CHIP_T:
            raise ValueError(f"s2_chip outer (T) must be {GALILEO_CHIP_T} (got {len(v)})")
        for ti, plane in enumerate(v):
            if len(plane) != GALILEO_CHIP_H:
                raise ValueError(
                    f"s2_chip[{ti}] (H) must be {GALILEO_CHIP_H} (got {len(plane)})"
                )
            for hi, row in enumerate(plane):
                if len(row) != GALILEO_CHIP_W:
                    raise ValueError(
                        f"s2_chip[{ti}][{hi}] (W) must be {GALILEO_CHIP_W} (got {len(row)})"
                    )
                for wi, px in enumerate(row):
                    if len(px) != 10:
                        raise ValueError(
                            f"s2_chip[{ti}][{hi}][{wi}] must have 10 bands (got {len(px)})"
                        )
        return v


class GalileoResponse(BaseModel):
    embedding: list[float]
    embedding_dim: int
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
    if _REG.prithvi is not None:
        models.append("prithvi_eo_v2_300m_tl")
    if _REG.galileo is not None:
        models.append(f"galileo_{GALILEO_VARIANT}_v1")
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


@app.post("/predict/prithvi_eo2_embed")
def predict_prithvi_eo2_embed(req: PrithviRequest) -> PrithviResponse:
    """Compute the per-cell Prithvi-EO-2.0-300M-TL embedding.

    Returns the encoder's CLS token from the last transformer block
    (post-norm) — a 1024-D foundation embedding suitable for cosine
    similarity, downstream linear probes, or k-NN retrieval. The chip
    is normalized with the model's per-band mean/std before the
    forward pass.
    """
    try:
        model, meta = _REG.load_prithvi()
    except FileNotFoundError as e:
        raise HTTPException(status_code=503, detail=str(e)) from e

    # Build [1, 6, 1, 224, 224] tensor from the request chip + normalize.
    arr = np.asarray(req.chip, dtype=np.float32)  # [6, 224, 224]
    x = torch.from_numpy(arr).unsqueeze(0).unsqueeze(2).to(_REG.device)
    mean = torch.tensor(meta["config"]["mean_per_band"], device=_REG.device)
    std = torch.tensor(meta["config"]["std_per_band"], device=_REG.device)
    x = (x - mean.view(1, 6, 1, 1, 1)) / std.view(1, 6, 1, 1, 1)

    # Optional metadata tensors. The model accepts None for either,
    # falling back to the dropout-trained no-metadata path.
    temporal_coords = None
    location_coords = None
    if req.year is not None and req.julian_day is not None:
        temporal_coords = torch.tensor(
            [[[float(req.year), float(req.julian_day)]]],
            device=_REG.device,
        )  # [B=1, T=1, 2]
    if req.lng is not None and req.lat is not None:
        location_coords = torch.tensor(
            [[float(req.lng), float(req.lat)]], device=_REG.device
        )  # [B=1, 2]

    t0 = time.perf_counter_ns()
    with torch.inference_mode():
        # forward_features returns one tensor per encoder block, post-norm
        # on the last entry. Shape: [B, num_patches+1, embed_dim].
        feats = model.forward_features(x, temporal_coords, location_coords)
    dt_us = max(1, (time.perf_counter_ns() - t0) // 1000)

    # CLS token of the last block IS the per-image foundation embedding.
    embedding = feats[-1][:, 0, :].squeeze(0).cpu().tolist()

    return PrithviResponse(
        embedding=embedding,
        embedding_dim=len(embedding),
        model={
            "model_id": meta["model_id"],
            "version": meta["version"],
            "license": meta["license"],
            "blake2b_hex": meta["blake2b_hex"],
            "via": "python_sidecar",
            "source": meta["source"],
            "paper": meta["paper"],
            "config": meta["config"],
            "honesty_warnings": [
                "frozen_pretrained_encoder: this is a per-cell forward "
                "pass through the frozen Prithvi-EO-2.0-300M-TL encoder "
                "(no fine-tuning at this responder). The embedding "
                "captures whatever HLS V2 pretraining captured; quality "
                "for any downstream task depends on how well that "
                "task aligns with masked-autoencoder objectives."
            ],
        },
        inference_us=dt_us,
        device=str(_REG.device),
    )


@app.post("/predict/galileo_tiny_embed")
def predict_galileo_tiny_embed(req: GalileoRequest) -> GalileoResponse:
    """Compute the per-cell Galileo-Tiny embedding from an S2-only chip.

    All other Galileo modalities (S1, ERA5, TC, VIIRS, SRTM, DW, WC,
    LandScan, location) are passed as zeros + masked-as-absent. The
    encoder is robust to this configuration — it was trained with the
    full mask-ratio schedule. Returns the average-pooled token output
    (192-D for Tiny) which matches the canonical Galileo embedding
    extraction recipe (cf. visualizing_embeddings.py upstream).
    """
    try:
        encoder, meta = _REG.load_galileo()
    except FileNotFoundError as e:
        raise HTTPException(status_code=503, detail=str(e)) from e

    sys.path.insert(0, str(Path(__file__).parent))
    from single_file_galileo import (  # noqa: WPS433
        SPACE_BANDS,
        SPACE_BAND_GROUPS_IDX,
        SPACE_TIME_BANDS,
        SPACE_TIME_BANDS_GROUPS_IDX,
        STATIC_BANDS,
        STATIC_BAND_GROUPS_IDX,
        S2_BANDS,
        TIME_BANDS,
        TIME_BAND_GROUPS_IDX,
    )

    device = _REG.device
    arr = np.asarray(req.s2_chip, dtype=np.float32)  # [T, H, W, 10]
    # Apply per-band normalization with the Galileo pretraining stats.
    mean = np.asarray(GALILEO_S2_MEAN, dtype=np.float32)
    std = np.asarray(GALILEO_S2_STD, dtype=np.float32)
    s2_n = (arr - mean) / std
    # Reorder to [H, W, T, 10] which is what construct_galileo_input expects.
    s2_t = torch.from_numpy(s2_n).permute(1, 2, 0, 3).contiguous().to(device)

    # Build the masked-output tuple inline (S2-only mode). The other
    # modalities are zero-filled and their group masks stay at 1
    # (= "not seen by the encoder").
    h, w, t, _ = s2_t.shape
    s_t_x = torch.zeros(
        (h, w, t, len(SPACE_TIME_BANDS)), dtype=torch.float, device=device
    )
    s_t_m = torch.ones(
        (h, w, t, len(SPACE_TIME_BANDS_GROUPS_IDX)), dtype=torch.float, device=device
    )
    s2_indices = [SPACE_TIME_BANDS.index(b) for b in S2_BANDS]
    s_t_x[:, :, :, s2_indices] = s2_t
    s2_groups = [
        i for i, k in enumerate(SPACE_TIME_BANDS_GROUPS_IDX) if k.startswith("S2_")
    ]
    s_t_m[:, :, :, s2_groups] = 0  # 0 = seen by encoder
    sp_x = torch.zeros((h, w, len(SPACE_BANDS)), dtype=torch.float, device=device)
    sp_m = torch.ones(
        (h, w, len(SPACE_BAND_GROUPS_IDX)), dtype=torch.float, device=device
    )
    t_x = torch.zeros((t, len(TIME_BANDS)), dtype=torch.float, device=device)
    t_m = torch.ones((t, len(TIME_BAND_GROUPS_IDX)), dtype=torch.float, device=device)
    st_x = torch.zeros((len(STATIC_BANDS),), dtype=torch.float, device=device)
    st_m = torch.ones(
        (len(STATIC_BAND_GROUPS_IDX),), dtype=torch.float, device=device
    )

    month = int(req.month) if req.month is not None else 7  # July default
    months = torch.full((t,), month, dtype=torch.long, device=device)

    # The encoder expects a batch dimension on every tensor — unsqueeze
    # to `[B=1, ...]` before the forward. construct_galileo_input in the
    # upstream repo omits the batch and the caller normally adds it via
    # DataLoader collation; we do it explicitly here.
    s_t_x = s_t_x.unsqueeze(0)
    s_t_m = s_t_m.unsqueeze(0)
    sp_x = sp_x.unsqueeze(0)
    sp_m = sp_m.unsqueeze(0)
    t_x = t_x.unsqueeze(0)
    t_m = t_m.unsqueeze(0)
    st_x = st_x.unsqueeze(0)
    st_m = st_m.unsqueeze(0)
    months = months.unsqueeze(0)

    t0 = time.perf_counter_ns()
    with torch.inference_mode():
        # The recipe per visualizing_embeddings.py: pass S2 modality
        # masks unmasked, set time/static masks all-ones (forces
        # average_tokens to ignore them in the pool), call forward, then
        # average over unmasked tokens.
        model_output = encoder(
            s_t_x.float(),
            sp_x.float(),
            t_x.float(),
            st_x.float(),
            s_t_m,
            sp_m,
            torch.ones_like(t_m),
            torch.ones_like(st_m),
            months.long(),
            patch_size=GALILEO_PATCH_SIZE,
        )
        # average_tokens takes the first 7 outputs (4 data + 3 masks
        # for S2/SP/T) — discard the static-mask. Result is [B, embed_dim].
        embedding_t = encoder.average_tokens(*model_output[:-1])
    dt_us = max(1, (time.perf_counter_ns() - t0) // 1000)

    # Squeeze the implicit batch dim if present (encoder runs single-batch
    # for now). Output shape is [embedding_size].
    if embedding_t.dim() > 1:
        embedding_t = embedding_t.squeeze(0)
    embedding = embedding_t.cpu().tolist()

    return GalileoResponse(
        embedding=embedding,
        embedding_dim=len(embedding),
        model={
            "model_id": meta["model_id"],
            "version": meta["version"],
            "license": meta["license"],
            "blake2b_hex": meta["blake2b_hex"],
            "via": "python_sidecar",
            "source": meta["source"],
            "paper": meta["paper"],
            "config": meta["config"],
            "honesty_warnings": [
                "frozen_pretrained_encoder: this is a per-cell forward "
                "pass through the frozen Galileo-Tiny encoder using "
                "the S2-only modality (S1/ERA5/TC/VIIRS/SRTM/DW/WC/"
                "LandScan/location all zero-masked). The full multimodal "
                "embedding would carry richer context — this responder "
                "ships S2-only as the lowest-friction mode that uses "
                "data already wired here."
            ],
        },
        inference_us=dt_us,
        device=str(device),
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
