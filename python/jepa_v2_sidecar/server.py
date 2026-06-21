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
from contextlib import asynccontextmanager
from pathlib import Path
from typing import Any

import numpy as np
import torch
import torch.nn as nn
import torch.nn.functional as F
from fastapi import FastAPI, HTTPException
from pydantic import BaseModel, Field, ValidationError, field_validator

# ── Hard VRAM budget ──────────────────────────────────────────────────────
# Operator instruction: keep this sidecar under 20 GB on the shared
# A100 so the other co-located GPU services don't get OOM'd. Each model
# allocates a slice and clamps via set_per_process_memory_fraction.
TOTAL_BUDGET_GB = float(os.environ.get("EMEM_SIDECAR_VRAM_BUDGET_GB", "20"))
DYNAMICS_BUDGET_GB = 0.1                    # tiny MLP
V_JEPA_2_BUDGET_GB = 3.0                    # ViT-L on 256x256x64 frames
PRITHVI_BUDGET_GB = 3.0
CLAY_BUDGET_GB = 2.5                        # ViT-L/8 (311 M params) at 256² fp16
RESERVE_BUDGET_GB = TOTAL_BUDGET_GB - (
    DYNAMICS_BUDGET_GB + V_JEPA_2_BUDGET_GB + PRITHVI_BUDGET_GB + CLAY_BUDGET_GB
)
assert RESERVE_BUDGET_GB > 0, "VRAM budget over-allocated; rebalance constants"

EMEM_DATA = Path(os.environ.get("EMEM_DATA", "/home/ubuntu/emem/var/emem"))
DYNAMICS_DIR = Path(os.environ.get("EMEM_JEPA_V2_DIR", str(EMEM_DATA / "jepa_v2")))

# Phase 3 — Prithvi-EO-2.0-300M-TL artifacts. The snapshot SHA is NOT
# hardcoded: we resolve weights via `huggingface_hub.try_to_load_from_cache`
# at load time, which walks `snapshots/<commit_hash>/` for the local
# cache without any network call. Operators wanting an explicit pin
# can set `EMEM_PRITHVI_CKPT` to an absolute checkpoint path; the
# loader uses that path's parent for the config.json sibling.
PRITHVI_REPO = "ibm-nasa-geospatial/Prithvi-EO-2.0-300M-TL"
PRITHVI_CKPT_FILENAME = "Prithvi_EO_V2_300M_TL.pt"
PRITHVI_CFG_FILENAME = "config.json"

# Phase 4 — Galileo multimodal encoder (NASA Harvest, MIT). Same
# offline-first resolver pattern as Prithvi. Variant defaults to base
# (config.json: embedding_size=768, depth=12, num_heads=12; ~86.5 M
# params, 330 MB ckpt, 768-D embedding); override EMEM_GALILEO_VARIANT
# to swap to tiny / nano. EMEM_GALILEO_SNAPSHOT pins an explicit folder
# path for air-gapped deployments.
GALILEO_REPO = "nasaharvest/galileo"
GALILEO_VARIANT = os.environ.get("EMEM_GALILEO_VARIANT", "base")
# S2-band normalization stats from the Galileo pretraining "13"
# normalizing_dict (S1 + S2 + NDVI). We slice out the 10 S2 entries
# in S2_BANDS order (B2/B3/B4/B5/B6/B7/B8/B8A/B11/B12).
#
# NORMALIZATION SCHEME (audit 2026-05-30, HIGH fix). Galileo's upstream
# `Normalizer` (galileo/src/data/dataset.py, std=True path) does NOT
# z-score. It maps each band to roughly [-1, 1] via shift/divide:
#     min_value = mean - std_multiplier * std   (std_multiplier=2 default)
#     max_value = mean + std_multiplier * std
#     shift = min_value = mean - 2*std
#     div   = max_value - min_value = 4*std
#     x_norm = (x - shift) / div
# The released model snapshot
# (`models--nasaharvest--galileo/.../models/<variant>/`) ships only
# encoder.pt/decoder.pt/config.json — there is NO `normalization.json`
# in the snapshot (verified on-disk 2026-05-30). The upstream Normalizer
# derives shift/div from hardcoded mean/std arrays, so the (mean-2σ)/(4σ)
# computation below IS the authoritative path. `_galileo_s2_shift_div`
# will still prefer an on-disk `normalization.json` if a future snapshot
# or operator pin provides one.
# Spec: arXiv:2502.09356 (Galileo) + upstream src/data/dataset.py
# Normalizer; config.json "normalization": "std".
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
# S1 + SRTM normalization stats for the multimodal path. Same upstream
# Normalizer scheme as S2 (std=True, std_multiplier=2): both S1 (VV,VH)
# and SRTM (elevation,slope) are in the Normalizer's `std_bands` set, so
# x_norm = (x - (mean - 2σ)) / (4σ). Stats are the computed mean/std from
# Galileo's `config/normalization.json` (keyed by band-group length): the
# "13"-vector (S1[2] + S2[10] + NDVI[1]) gives S1 at indices 0,1; the
# "16"-vector (SRTM[2] + DW[9] + WC[5]) gives SRTM at indices 0,1.
# Verified against nasaharvest/galileo @ main on 2026-05-30
# (config/normalization.json + src/data/earthengine/{s1,srtm}.py).
# arXiv:2502.09356.
#   S1 VV: mean -11.728724389184965, std 4.887145774840316  (γ0 dB)
#   S1 VH: mean -18.85558188024017,  std 5.730270320384293  (γ0 dB)
GALILEO_S1_MEAN: list[float] = [-11.728724389184965, -18.85558188024017]
GALILEO_S1_STD: list[float] = [4.887145774840316, 5.730270320384293]
#   SRTM elevation: mean 673.0152819503361, std 983.0697298296237  (m)
#   SRTM slope:     mean 5.930092668915115, std 8.167406789813247  (degrees)
GALILEO_SRTM_MEAN: list[float] = [673.0152819503361, 5.930092668915115]
GALILEO_SRTM_STD: list[float] = [983.0697298296237, 8.167406789813247]
# TerraClimate (def, soil, aet) — Galileo's TIME modality. Same upstream
# Normalizer scheme (std=True, std_multiplier=2): all TIME_BANDS are in the
# Normalizer's `std_bands` set (galileo/src/data/dataset.py Normalizer,
# `std_bands[len(TIME_BANDS)] = TIME_BANDS`), so x_norm = (x - (mean-2σ))/(4σ).
# Stats are the mean/std from Galileo's config/normalization.json keyed by
# band-group length: the TIME-group vector has length len(TIME_BANDS)=6, so
# it lives under JSON key "6" (TIME_BANDS = ERA5[2] + TC[3] + VIIRS[1]).
# TerraClimate occupies indices 2,3,4 (def, soil, aet) — verified against
# single_file_galileo.py `TC_BANDS = ["def","soil","aet"]` and
# `TIME_BANDS.index` for each. Values are in raw Google Earth Engine band
# scale (DN = physical_mm / 0.1), matching the upstream ingest
# (galileo/src/data/earthengine/terraclimate.py: select(TC_BANDS).toDouble(),
# no scale applied). Verified against nasaharvest/galileo @ main 2026-05-30.
# arXiv:2502.09356.
#   TC def (idx 2): mean 657.3181260091111, std 704.0008695557707  (GEE DN)
#   TC soil (idx 3): mean 692.1291795806885, std 925.0116126406431 (GEE DN)
#   TC aet (idx 4): mean 562.781331880633,  std 453.2434022278578  (GEE DN)
GALILEO_TC_MEAN: list[float] = [
    657.3181260091111,
    692.1291795806885,
    562.781331880633,
]
GALILEO_TC_STD: list[float] = [
    704.0008695557707,
    925.0116126406431,
    453.2434022278578,
]

# Galileo (base) chip shape — 8×8 pixels sampled at 30 m = 240 m extent
# centred on the cell. Patch_size=2 → 4×4 token grid (16 tokens per
# step). T=1 (single timestep). Fits Galileo's training distribution
# (config.json shape_time_combinations include sizes 4–12).
GALILEO_CHIP_H: int = 8
GALILEO_CHIP_W: int = 8
GALILEO_CHIP_T: int = 1
GALILEO_PATCH_SIZE: int = 2
# Ground sample distance (m) of the chip the Rust side samples. The
# encoder's positional encoding uses gsd_ratio = token_res / BASE_GSD
# where token_res = input_resolution_m * patch_size (single_file_galileo.py
# ~line 711). Omitting input_resolution_m defaults it to BASE_GSD=10,
# which for a 30 m chip yields gsd_ratio = 10*2/10 = 2 instead of the
# correct 30*2/10 = 6 — i.e. wrong positional encoding. Pass this to the
# forward (audit 2026-05-30, HIGH fix #2). MUST agree with the Rust
# chip's sampling resolution.
GALILEO_CHIP_GSD_M: int = 30

# Phase 5 — Clay Foundation Model v1.5 (Made With Clay, Apache-2.0).
# ViT-L/8 MAE + DINOv2 teacher; 311 M params encoder, 1024-D CLS token,
# wavelength-conditioned so one encoder handles S2/S1/Landsat/NAIP/MODIS
# without re-finetuning. Resolution is HF-cache-aware: we ask
# huggingface_hub to look up the checkpoint by repo + filename, which
# walks the real `snapshots/<commit_hash>/` layout instead of the
# bogus `snapshots/main/` literal. Override with EMEM_CLAY_CKPT for
# tests or air-gapped deployments. The metadata.yaml is vendored
# beside server.py so the loader needs zero network at startup.
CLAY_REPO = "made-with-clay/Clay"
CLAY_CKPT_FILENAME = "v1.5/clay-v1.5.ckpt"
CLAY_METADATA_DEFAULT = Path(__file__).parent / "clay_metadata.yaml"
CLAY_METADATA_PATH = Path(
    os.environ.get("EMEM_CLAY_METADATA", str(CLAY_METADATA_DEFAULT))
)


def _hf_cache_dir() -> str:
    """The HF cache directory all loaders share. Honours `EMEM_HF_CACHE`
    so air-gapped deployments can point at a pre-seeded mount, falls
    back to `$EMEM_DATA/hf_cache/hub` (matching `HF_HOME` in the systemd
    unit) and finally to the huggingface_hub default."""
    override = os.environ.get("EMEM_HF_CACHE")
    if override:
        return override
    default = EMEM_DATA / "hf_cache/hub"
    if default.exists():
        return str(default)
    try:
        from huggingface_hub.constants import HF_HUB_CACHE  # noqa: WPS433

        return HF_HUB_CACHE
    except ImportError:
        return str(default)


def _resolve_hf_file(repo_id: str, filename: str) -> Path | None:
    """Walk the HF cache for `filename` under `repo_id`; return its
    absolute path or None when not present. Read-only — never makes
    a network call. Used by every loader so production runs offline
    once the cache is seeded (single bootstrap, then HF_HUB_OFFLINE=1)."""
    try:
        from huggingface_hub import try_to_load_from_cache  # noqa: WPS433
    except ImportError:
        return None
    hit = try_to_load_from_cache(
        repo_id=repo_id, filename=filename, cache_dir=_hf_cache_dir()
    )
    # try_to_load_from_cache returns: str path when present, a
    # _CACHED_NO_EXIST sentinel when the file is known not to exist,
    # or None when not cached. Treat the sentinel as "not found".
    if isinstance(hit, str):
        return Path(hit)
    return None


def _resolve_clay_ckpt() -> Path | None:
    """Return the Clay v1.5 checkpoint path, or None if not cached.

    Resolution order:
    1. EMEM_CLAY_CKPT (explicit override; honoured even if the file
       is missing so the caller can produce a precise error).
    2. `_resolve_hf_file` — walks the real `snapshots/<commit_hash>/`
       tree without network access.
    3. None (caller raises FileNotFoundError with bootstrap text).
    """
    override = os.environ.get("EMEM_CLAY_CKPT")
    if override:
        return Path(override)
    return _resolve_hf_file(CLAY_REPO, CLAY_CKPT_FILENAME)


def _resolve_prithvi_files() -> tuple[Path | None, Path | None]:
    """Resolve the Prithvi `.pt` checkpoint and its sibling
    `config.json`. Both env overrides win over the HF cache walk so
    air-gapped deployments can point at a custom directory. Returns
    `(None, None)` when nothing is cached yet — caller emits the
    bootstrap instruction."""
    ckpt_override = os.environ.get("EMEM_PRITHVI_CKPT")
    snapshot_override = os.environ.get("EMEM_PRITHVI_SNAPSHOT")
    if ckpt_override:
        ckpt = Path(ckpt_override)
        return ckpt, ckpt.parent / PRITHVI_CFG_FILENAME
    if snapshot_override:
        d = Path(snapshot_override)
        return d / PRITHVI_CKPT_FILENAME, d / PRITHVI_CFG_FILENAME
    ckpt = _resolve_hf_file(PRITHVI_REPO, PRITHVI_CKPT_FILENAME)
    cfg = _resolve_hf_file(PRITHVI_REPO, PRITHVI_CFG_FILENAME)
    return ckpt, cfg


def _resolve_galileo_dir() -> Path | None:
    """Resolve the Galileo `models/<variant>/` directory. Honours
    `EMEM_GALILEO_SNAPSHOT` for explicit pins; otherwise walks the HF
    cache and returns the variant subdir of the located encoder."""
    override = os.environ.get("EMEM_GALILEO_SNAPSHOT")
    if override:
        return Path(override)
    encoder = _resolve_hf_file(
        GALILEO_REPO, f"models/{GALILEO_VARIANT}/encoder.pt"
    )
    return encoder.parent if encoder is not None else None


def _galileo_s2_shift_div(
    galileo_dir: Path | None,
) -> tuple[np.ndarray, np.ndarray, str]:
    """Return (shift, div, source) for the 10 S2 bands per Galileo's
    upstream `Normalizer` (std=True): `x_norm = (x - shift) / div` with
    `shift = mean - 2*std`, `div = 4*std` (std_multiplier=2; see
    galileo/src/data/dataset.py Normalizer; arXiv:2502.09356).

    PREFERS an on-disk `normalization.json` in the resolved model dir if
    present (a future snapshot / operator pin may ship one). The released
    `nasaharvest/galileo` snapshot does NOT contain it (verified on-disk
    2026-05-30), so the computed (mean-2σ)/(4σ) path is the authoritative
    default. If a normalization.json is found it must carry explicit
    `shift`/`div` (10-vectors in S2_BANDS order) or `mean`/`std`; anything
    else falls back to the computed values rather than guessing.
    """
    mean = np.asarray(GALILEO_S2_MEAN, dtype=np.float32)
    std = np.asarray(GALILEO_S2_STD, dtype=np.float32)
    computed_shift = mean - 2.0 * std
    computed_div = 4.0 * std
    if galileo_dir is not None:
        nj = galileo_dir / "normalization.json"
        if nj.exists():
            try:
                blob = json.loads(nj.read_text())
                if (
                    isinstance(blob.get("shift"), list)
                    and isinstance(blob.get("div"), list)
                    and len(blob["shift"]) == 10
                    and len(blob["div"]) == 10
                ):
                    return (
                        np.asarray(blob["shift"], dtype=np.float32),
                        np.asarray(blob["div"], dtype=np.float32),
                        f"normalization.json (shift/div) @ {nj}",
                    )
                if (
                    isinstance(blob.get("mean"), list)
                    and isinstance(blob.get("std"), list)
                    and len(blob["mean"]) == 10
                    and len(blob["std"]) == 10
                ):
                    m = np.asarray(blob["mean"], dtype=np.float32)
                    s = np.asarray(blob["std"], dtype=np.float32)
                    return (
                        m - 2.0 * s,
                        4.0 * s,
                        f"normalization.json (mean/std → mean-2σ/4σ) @ {nj}",
                    )
            except (json.JSONDecodeError, KeyError, TypeError, ValueError):
                pass  # fall through to computed
    return (
        computed_shift,
        computed_div,
        "computed (mean-2σ)/(4σ) from GALILEO_S2_MEAN/STD (no normalization.json in snapshot)",
    )


def _galileo_modality_shift_div(
    mean: list[float], std: list[float]
) -> tuple[np.ndarray, np.ndarray]:
    """Galileo's upstream Normalizer shift/div for a `std_bands` modality
    (std=True, std_multiplier=2): `shift = mean - 2σ`, `div = 4σ`. Used for
    S1 (VV,VH) and SRTM (elevation,slope), both of which are in the
    Normalizer's `std_bands` set (galileo/src/data/dataset.py Normalizer).
    The released snapshot ships no normalization.json (verified on-disk
    2026-05-30), so these computed values from config/normalization.json
    are the authoritative path. arXiv:2502.09356."""
    m = np.asarray(mean, dtype=np.float32)
    s = np.asarray(std, dtype=np.float32)
    return m - 2.0 * s, 4.0 * s


# Clay v1.5's S2 L2A platform expects 10 bands at 10 m, in this order
# (verbatim from configs/metadata.yaml). Per-band mean/std + wavelength
# (µm) come from the metadata file; the loader reads them at startup so
# we don't drift from upstream.
CLAY_S2_BAND_ORDER = (
    "blue", "green", "red", "rededge1", "rededge2", "rededge3",
    "nir", "nir08", "swir16", "swir22",
)
# Per the wall-to-wall tutorial, the wave list / mean / std are passed
# directly to the encoder; the chip is normalised in the request handler.
CLAY_CHIP_PIXELS = 256          # 256x256 at 10 m → 2.56 km extent
CLAY_CHIP_GSD_M = 10.0
CLAY_EMBED_DIM = 1024

# ── Dynamics v2 model (multi-band scalar predictor) ───────────────────────
#
# Replaces the pre-2026-05-28 K=3 Tessera-vintage residual MLP. That
# surface was retired because the upstream dl2.geotessera.org bucket
# only serves 2024 reliably (other years return null), so all K lags
# collapsed to the same vector and the residual was the identity.
#
# The new surface predicts the next monthly value for four scalar
# bands {indices.ndvi, modis.lst_day_8day, modis.lst_night_8day,
# cams.pm25} from K=6 prior (already-normalised) lags + a 5-D
# spatial-temporal context vector. The model is a small Transformer
# encoder trained by python/jepa_v2_sidecar/train_dynamics_v2.py;
# see that file for the architecture and training rationale.

# Architecture constants — must match train_dynamics_v2.py.
K_LAGS = 6
N_BANDS = 4
N_CONTEXT = 5
INPUT_DIM = K_LAGS * N_BANDS + N_CONTEXT       # 29
OUTPUT_DIM = 2 * N_BANDS                       # 8 (mean[4] || log_var[4])
D_MODEL = 32
N_HEADS = 4
N_LAYERS = 2
DROPOUT = 0.10
BAND_KEYS_SIDECAR = (
    "indices.ndvi",
    "modis.lst_day_8day",
    "modis.lst_night_8day",
    "cams.pm25",
)
LOW_CONFIDENCE_THRESHOLD = 0.30


class DynamicsModel(nn.Module):
    """Multi-band scalar dynamics head. MUST stay byte-identical to
    `DynamicsV2` in `train_dynamics_v2.py` so the same .onnx loads
    here for a PyTorch parity-check on warm-up if needed.

    Input: flat tensor `[B, INPUT_DIM]` =
        lags_normalised (K_LAGS * N_BANDS) || context (N_CONTEXT).
    Output: `[B, OUTPUT_DIM]` = mean[N_BANDS] || log_var[N_BANDS].
    """

    def __init__(self) -> None:
        super().__init__()
        self.band_proj = nn.Linear(N_BANDS, D_MODEL)
        self.lag_pos = nn.Parameter(torch.zeros(K_LAGS, D_MODEL))
        self.cls = nn.Parameter(torch.zeros(1, D_MODEL))
        nn.init.normal_(self.lag_pos, std=0.02)
        nn.init.normal_(self.cls, std=0.02)
        encoder_layer = nn.TransformerEncoderLayer(
            d_model=D_MODEL,
            nhead=N_HEADS,
            dim_feedforward=4 * D_MODEL,
            dropout=DROPOUT,
            batch_first=True,
            norm_first=True,
            activation="gelu",
        )
        self.encoder = nn.TransformerEncoder(encoder_layer, num_layers=N_LAYERS)
        self.ctx_proj = nn.Linear(N_CONTEXT, D_MODEL)
        self.head = nn.Sequential(
            nn.Linear(2 * D_MODEL, D_MODEL),
            nn.GELU(),
            nn.Linear(D_MODEL, 2 * N_BANDS),
        )

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        B = x.shape[0]
        lags = x[:, : K_LAGS * N_BANDS].reshape(B, K_LAGS, N_BANDS)
        ctx = x[:, K_LAGS * N_BANDS :]
        h = self.band_proj(lags) + self.lag_pos.unsqueeze(0)
        cls = self.cls.expand(B, -1).unsqueeze(1)
        h = torch.cat([cls, h], dim=1)
        h = self.encoder(h)
        cls_out = h[:, 0, :]
        ctx_h = F.gelu(self.ctx_proj(ctx))
        combined = torch.cat([cls_out, ctx_h], dim=-1)
        return self.head(combined)


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
        self.clay: tuple[Any, dict[str, Any]] | None = None
        self.clay_load_at: float | None = None
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
        """Load the trained multi-band scalar dynamics head.

        When `training.trained == true` the loader pulls the
        `dynamics_v2.state_dict.pt` checkpoint alongside the metadata
        and verifies its blake2b against the pinned value (failing
        closed on any mismatch).

        When `training.trained == false` we keep the historical
        residual-zero fallback path so the FastAPI surface is still
        reachable on a half-bootstrapped responder — but the model is
        intentionally a structural no-op for the NEW input shape: the
        head is zero-init and the body forwards anything. The
        Rust-side handler short-circuits to identity before reaching
        the sidecar in that case, so this path effectively only fires
        in tests that explicitly hit the sidecar with an untrained
        artifact.
        """
        if self.dynamics is not None:
            return self.dynamics
        meta_path = DYNAMICS_DIR / "dynamics_v2.metadata.json"
        if not meta_path.exists():
            raise FileNotFoundError(
                f"dynamics_v2 metadata not found at {meta_path}; "
                "run python/jepa_v2_sidecar/train_dynamics_v2.py to seed "
                "the trained artifact + metadata."
            )
        meta = json.loads(meta_path.read_text())
        trained = bool(meta.get("training", {}).get("trained", False))
        model = DynamicsModel().to(self.device)
        if trained:
            ckpt_path = DYNAMICS_DIR / "dynamics_v2.state_dict.pt"
            if not ckpt_path.exists():
                raise FileNotFoundError(
                    f"dynamics_v2 metadata claims trained=true but no checkpoint "
                    f"found at {ckpt_path}. Either drop training.trained back to "
                    "false or re-run train_dynamics_v2.py to ship the matching "
                    ".pt alongside the metadata."
                )
            expected_b2b = (
                meta.get("training", {}).get("checkpoint_blake2b_hex")
                or meta.get("model", {}).get("blake2b_hex")
            )
            if expected_b2b:
                got_b2b = hashlib.blake2b(
                    ckpt_path.read_bytes(), digest_size=32
                ).hexdigest()
                if got_b2b != expected_b2b:
                    raise RuntimeError(
                        "dynamics_v2 trained checkpoint blake2b mismatch "
                        f"(got {got_b2b[:16]}..., metadata pinned "
                        f"{expected_b2b[:16]}...). Refusing to serve a swapped "
                        "checkpoint under a stale receipt — re-run "
                        "python/jepa_v2_sidecar/train_dynamics_v2.py to refresh "
                        "metadata or restore the matching .pt file."
                    )
            # weights_only=True so a malicious .pt cannot side-effect via
            # torch's pickle (defense in depth).
            state_dict = torch.load(
                ckpt_path, map_location=self.device, weights_only=True
            )
            model.load_state_dict(state_dict, strict=True)
        else:
            with torch.no_grad():
                # Zero-init head so the untrained fallback emits a
                # deterministic all-zeros mean (which the wrapping
                # endpoint will denormalise to each band's
                # climatological mean — a sane garbage answer that the
                # receipt clearly marks `untrained_baseline`).
                for layer in model.head:
                    if hasattr(layer, "weight"):
                        layer.weight.zero_()
                    if hasattr(layer, "bias") and layer.bias is not None:
                        layer.bias.zero_()
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
        ckpt_path, cfg_path = _resolve_prithvi_files()
        if (
            ckpt_path is None
            or cfg_path is None
            or not ckpt_path.exists()
            or not cfg_path.exists()
        ):
            raise FileNotFoundError(
                f"Prithvi-EO-2.0 artifacts not found in HF cache "
                f"({_hf_cache_dir()}) and neither EMEM_PRITHVI_CKPT nor "
                f"EMEM_PRITHVI_SNAPSHOT is set. Bootstrap with "
                f"`python -m bootstrap_models prithvi` or "
                f"`huggingface_hub.snapshot_download(repo_id='{PRITHVI_REPO}', "
                f"allow_patterns=['*.py','*.json','*.txt','*.pt'], "
                f"cache_dir='{_hf_cache_dir()}')` — one-time ~1.3 GB. "
                f"Production runs offline once cached (HF_HUB_OFFLINE=1)."
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
        # strict=False is REQUIRED because we deleted the pos_embed buffers
        # (rebuilt deterministically at forward for num_frames=1). But a
        # checkpoint rename / architecture drift could otherwise silently
        # leave real weight tensors un-loaded and we'd sign an embedding
        # from random init. Assert the ONLY missing/unexpected keys are
        # pos_embed buffers (audit 2026-05-30 fix #3).
        incompatible = model.load_state_dict(sd, strict=False)
        bad = [
            k
            for k in (*incompatible.missing_keys, *incompatible.unexpected_keys)
            if "pos_embed" not in k
        ]
        if bad:
            raise RuntimeError(
                "Prithvi checkpoint load_state_dict mismatch on non-pos_embed "
                f"keys (refusing to sign a partially-random embedding): {bad}. "
                f"missing={list(incompatible.missing_keys)} "
                f"unexpected={list(incompatible.unexpected_keys)}"
            )
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
        """Load Galileo (NASA Harvest, MIT) from the pinned local
        snapshot for the variant in EMEM_GALILEO_VARIANT (default base).
        Single-file model code is vendored at
        `python/jepa_v2_sidecar/single_file_galileo.py`. Base variant
        (config.json): embedding_size=768, depth=12, ~86.5 M params,
        ~330 MB checkpoint, 768-D embedding.
        """
        if self.galileo is not None:
            return self.galileo
        sys.path.insert(0, str(Path(__file__).parent))
        from single_file_galileo import Encoder as GalileoEncoder  # noqa: WPS433
        galileo_dir = _resolve_galileo_dir()
        if galileo_dir is None or not (galileo_dir / "encoder.pt").exists():
            raise FileNotFoundError(
                f"Galileo-{GALILEO_VARIANT} artifacts not found in HF cache "
                f"({_hf_cache_dir()}) and EMEM_GALILEO_SNAPSHOT is unset. "
                f"Bootstrap with `python -m bootstrap_models galileo` or "
                f"`huggingface_hub.snapshot_download(repo_id='{GALILEO_REPO}', "
                f"allow_patterns=['models/{GALILEO_VARIANT}/*'], "
                f"cache_dir='{_hf_cache_dir()}')` — one-time ~58 MB. "
                f"Production runs offline once cached (HF_HUB_OFFLINE=1)."
            )
        encoder = GalileoEncoder.load_from_folder(galileo_dir, self.device)
        encoder.to(self.device).eval()
        ckpt_path = galileo_dir / "encoder.pt"
        ckpt_blake2b = hashlib.blake2b(
            ckpt_path.read_bytes(), digest_size=32
        ).hexdigest()
        cfg = json.loads((galileo_dir / "config.json").read_text())
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
                "normalization": "shift_divide (x-shift)/div; shift=mean-2σ, div=4σ (upstream Normalizer std=True)",
                "input_resolution_m": GALILEO_CHIP_GSD_M,
            },
        }
        self.galileo = (encoder, meta)
        self.galileo_load_at = time.time()
        return self.galileo

    def load_clay(self) -> tuple[Any, dict[str, Any]]:
        """Load Clay Foundation Model v1.5 (Made With Clay, Apache-2.0)
        from the pinned local snapshot. Returns (model, meta) where
        model is in eval mode on self.device, meta carries the
        receipt-shape `model` block. Loader follows the wall-to-wall
        tutorial verbatim (https://clay-foundation.github.io/model/).

        Cold-start: ~6-10 s on first call (1.25 GB encoder + 304 MB
        DINOv2 teacher inside the .ckpt; teacher is dropped at load).
        Warm: <30 ms per chip on a modern GPU at fp16, ~3-8 s on CPU.
        """
        if self.clay is not None:
            return self.clay
        ckpt_path = _resolve_clay_ckpt()
        if ckpt_path is None or not ckpt_path.exists():
            raise FileNotFoundError(
                f"Clay v1.5 checkpoint not found in HF cache "
                f"({_hf_cache_dir()}) and EMEM_CLAY_CKPT is unset. "
                f"Bootstrap with `python -m bootstrap_models clay` or "
                f"`huggingface_hub.hf_hub_download(repo_id='{CLAY_REPO}', "
                f"filename='{CLAY_CKPT_FILENAME}', "
                f"cache_dir='{_hf_cache_dir()}')` — one-time ~5.16 GB "
                f"checkpoint; ~1.25 GB encoder weights at runtime. "
                f"Override with EMEM_CLAY_CKPT."
            )
        if not CLAY_METADATA_PATH.exists():
            raise FileNotFoundError(
                f"Clay metadata.yaml not found at {CLAY_METADATA_PATH}. "
                f"Vendor it from "
                f"https://raw.githubusercontent.com/Clay-foundation/model/main/configs/metadata.yaml "
                f"(small, ~10 KB) or override with EMEM_CLAY_METADATA."
            )
        # Heavy imports are deferred to load time so `import server` stays
        # cheap. claymodel is installed via
        # `pip install git+https://github.com/Clay-foundation/model.git`.
        try:
            import yaml  # noqa: WPS433
            from box import Box  # noqa: WPS433
            from claymodel.module import ClayMAEModule  # noqa: WPS433
        except ImportError as e:
            raise FileNotFoundError(
                f"Clay Python deps missing: {e}. Install with "
                f"`pip install git+https://github.com/Clay-foundation/model.git "
                f"python-box pyyaml`."
            ) from e

        meta_yaml = Box(yaml.safe_load(CLAY_METADATA_PATH.read_text()))

        # Wall-to-wall tutorial argument set. mask_ratio=0.0 + shuffle=False
        # is REQUIRED for inference (any other value runs the masking
        # branch trained for pretraining and returns degenerate output).
        # `dolls` are the Matryoshka training-time list — not used at
        # inference, but required by load_from_checkpoint.
        model = ClayMAEModule.load_from_checkpoint(
            str(ckpt_path),
            model_size="large",
            metadata_path=str(CLAY_METADATA_PATH),
            dolls=[16, 32, 64, 128, 256, 768, 1024],
            doll_weights=[1, 1, 1, 1, 1, 1, 1],
            mask_ratio=0.0,
            shuffle=False,
        )
        model.eval().to(self.device)
        ckpt_blake2b = hashlib.blake2b(
            ckpt_path.read_bytes(), digest_size=32
        ).hexdigest()

        # Resolve the per-band stats now so they're cached in `meta`
        # for the receipt — clients can verify the normalisation
        # decoupled from the upstream metadata.yaml at recall time.
        s2 = meta_yaml["sentinel-2-l2a"]
        means = [s2.bands.mean[b] for b in CLAY_S2_BAND_ORDER]
        stds = [s2.bands.std[b] for b in CLAY_S2_BAND_ORDER]
        waves = [s2.bands.wavelength[b] for b in CLAY_S2_BAND_ORDER]
        meta_dict: dict[str, Any] = {
            "model_id": "clay_v1_5",
            "version": "1.5.0",
            "license": "Apache-2.0",
            "source": f"{CLAY_REPO}/{CLAY_CKPT_FILENAME}",
            "paper": "https://clay-foundation.github.io/model/release-notes/specification.html",
            "checkpoint_filename": ckpt_path.name,
            "blake2b_hex": ckpt_blake2b,
            "size_bytes": ckpt_path.stat().st_size,
            "config": {
                "encoder_dim": CLAY_EMBED_DIM,
                "patch_size": 8,
                "chip_pixels": CLAY_CHIP_PIXELS,
                "chip_gsd_m": CLAY_CHIP_GSD_M,
                "platform": "sentinel-2-l2a",
                "bands_in_order": list(CLAY_S2_BAND_ORDER),
                "mean_per_band": means,
                "std_per_band": stds,
                "wavelength_um_per_band": waves,
                "extraction": "encoder.unmsk_patch[:,0,:] (CLS token, 1024-D, post-norm)",
            },
        }
        self.clay = (model, meta_dict)
        self.clay_load_at = time.time()
        return self.clay


_REG = _Registry()


# ── Schemas ──────────────────────────────────────────────────────────────
class DynamicsRequest(BaseModel):
    """Multi-band scalar dynamics request (v0.0.1, 2026-05-28).

    Wire shape:
      lags_normalised : [K_LAGS * N_BANDS] = [24] f32, row-major
                        layout (lag0_band0, lag0_band1, ..., lag5_band3).
                        Each scalar is already z-scored on the
                        caller's side with the band's (mean, std).
      context         : [N_CONTEXT] = [5] f32
                        (lat_norm, lng_sin, lng_cos, month_sin,
                        month_cos).

    The endpoint deliberately accepts pre-normalised inputs so the
    Rust caller can sign the receipt with the exact normalisation
    parameters it used. The sidecar denormalises outputs and clamps
    to physical ranges before returning."""

    lags_normalised: list[float] = Field(
        ..., description=f"shape [{K_LAGS * N_BANDS}], lags in row-major order"
    )
    context: list[float] = Field(
        ..., description=f"shape [{N_CONTEXT}]: lat_norm, lng_sin, lng_cos, month_sin, month_cos"
    )

    @field_validator("lags_normalised")
    @classmethod
    def check_lags(cls, v: list[float]) -> list[float]:
        if len(v) != K_LAGS * N_BANDS:
            raise ValueError(
                f"`lags_normalised` must be {K_LAGS * N_BANDS} floats "
                f"(K_LAGS={K_LAGS} * N_BANDS={N_BANDS}); got {len(v)}"
            )
        return v

    @field_validator("context")
    @classmethod
    def check_context(cls, v: list[float]) -> list[float]:
        if len(v) != N_CONTEXT:
            raise ValueError(
                f"`context` must be {N_CONTEXT} floats; got {len(v)}"
            )
        return v


class DynamicsResponse(BaseModel):
    """Multi-band scalar prediction. `predictions` are in physical
    units after denormalisation + per-band clamp; `confidence` is the
    per-band `exp(-log_var/2)` from the model's Gaussian head.
    `band_keys` is the canonical order so callers don't have to
    rely on positional indices."""

    predictions: list[float]
    confidence: list[float]
    band_keys: list[str]
    low_confidence_bands: list[str]
    model: dict[str, Any]
    inference_us: int
    device: str
    # Additive (audit 2026-05-30 HIGH #5): held-out skill vs the
    # predict-last-value baseline on REAL NDVI, sourced from the training
    # metadata. None when the trained model didn't carry a real-NDVI eval.
    skill_vs_persistence: dict[str, Any] | None = None


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
    """Per-cell Galileo multimodal embedding request.

    `s2_chip` is the 10-band S2 reflectance window in Galileo's
    S2_BANDS order (B2/B3/B4/B5/B6/B7/B8/B8A/B11/B12). Reflectance
    in upstream's native scale (typically 0–10000); the sidecar
    handles per-band normalization with the Galileo pretraining
    stats. Shape: `[T, H, W, 10]` where T must equal
    `GALILEO_CHIP_T` (1) and H, W must equal `GALILEO_CHIP_H/W` (8).

    `s1_chip` (optional) is the Sentinel-1 RTC VV+VH γ0 dB window,
    shape `[T=1, H=8, W=8, 2]` (VV, VH). When present the S1 group
    mask is set to seen and S1's per-modality (mean-2σ)/(4σ) norm is
    applied. NaN pixels (water mask / nodata) are normalized then
    zeroed so they contribute the modality mean rather than NaN.

    `srtm_chip` (optional) is the Copernicus-DEM elevation+slope
    window, shape `[H=8, W=8, 2]` (elevation m, slope degrees) — the
    space tensor has no time dimension. When present the SRTM group
    mask is set to seen.

    Modalities not provided stay zero-filled + masked-absent (the
    encoder is robust to this — it was trained with the full mask
    schedule). This keeps the S2-only path working unchanged.
    """
    s2_chip: list[list[list[list[float]]]] = Field(
        ..., description="reflectance chip, shape [T=1, H=8, W=8, 10]"
    )
    s1_chip: list[list[list[list[float]]]] | None = Field(
        default=None, description="S1 VV+VH γ0 dB chip, shape [T=1, H=8, W=8, 2]"
    )
    srtm_chip: list[list[list[float]]] | None = Field(
        default=None, description="DEM elevation+slope chip, shape [H=8, W=8, 2]"
    )
    tc_chip: list[list[float]] | None = Field(
        default=None,
        description="TerraClimate def+soil+aet chip (raw GEE DN), shape [T=1, C=3]",
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

    @field_validator("s1_chip")
    @classmethod
    def check_s1_shape(
        cls, v: list[list[list[list[float]]]] | None
    ) -> list[list[list[list[float]]]] | None:
        if v is None:
            return v
        if len(v) != GALILEO_CHIP_T:
            raise ValueError(f"s1_chip outer (T) must be {GALILEO_CHIP_T} (got {len(v)})")
        for plane in v:
            if len(plane) != GALILEO_CHIP_H:
                raise ValueError(f"s1_chip H must be {GALILEO_CHIP_H} (got {len(plane)})")
            for row in plane:
                if len(row) != GALILEO_CHIP_W:
                    raise ValueError(f"s1_chip W must be {GALILEO_CHIP_W} (got {len(row)})")
                for px in row:
                    if len(px) != 2:
                        raise ValueError(f"s1_chip must have 2 bands VV,VH (got {len(px)})")
        return v

    @field_validator("srtm_chip")
    @classmethod
    def check_srtm_shape(
        cls, v: list[list[list[float]]] | None
    ) -> list[list[list[float]]] | None:
        if v is None:
            return v
        if len(v) != GALILEO_CHIP_H:
            raise ValueError(f"srtm_chip H must be {GALILEO_CHIP_H} (got {len(v)})")
        for row in v:
            if len(row) != GALILEO_CHIP_W:
                raise ValueError(f"srtm_chip W must be {GALILEO_CHIP_W} (got {len(row)})")
            for px in row:
                if len(px) != 2:
                    raise ValueError(
                        f"srtm_chip must have 2 bands elevation,slope (got {len(px)})"
                    )
        return v

    @field_validator("tc_chip")
    @classmethod
    def check_tc_shape(
        cls, v: list[list[float]] | None
    ) -> list[list[float]] | None:
        if v is None:
            return v
        if len(v) != GALILEO_CHIP_T:
            raise ValueError(f"tc_chip outer (T) must be {GALILEO_CHIP_T} (got {len(v)})")
        for step in v:
            if len(step) != 3:
                raise ValueError(
                    f"tc_chip must have 3 bands def,soil,aet (got {len(step)})"
                )
        return v


class GalileoResponse(BaseModel):
    embedding: list[float]
    embedding_dim: int
    model: dict[str, Any]
    inference_us: int
    device: str


class ClayRequest(BaseModel):
    """Clay Foundation Model v1.5 per-cell embedding request.

    `chip` is a `[10, 256, 256]` reflectance window in S2 L2A band order
    (blue, green, red, rededge1, rededge2, rededge3, nir, nir08, swir16,
    swir22). Reflectance values are raw S2 L2A scaled (0..10000); the
    sidecar normalises with the model's per-band mean/std.

    Optional `year` + `month` engage the temporal encoder via
    sin/cos(week-of-year, hour-of-day); pass None for the no-time path.
    Optional `lng` + `lat` engage the spatial encoder via sin/cos(lat,
    lon); pass None for the no-location path.
    """

    chip: list[list[list[float]]]
    year: int | None = None
    month: int | None = None
    day: int | None = None
    lng: float | None = None
    lat: float | None = None


class ClayResponse(BaseModel):
    embedding: list[float]
    embedding_dim: int
    model: dict[str, Any]
    inference_us: int
    device: str


# ── App ───────────────────────────────────────────────────────────────────
def _warm_load_resolvable_models() -> dict[str, str]:
    """Load every model whose checkpoint is resolvable in the local cache,
    once, at startup, inside the 20 GB VRAM budget (audit 2026-05-30 #6).

    /health previously advertised a model as available the instant its
    checkpoint was cache-resolvable — before any load — so the first cold
    /predict could exceed the caller's timeout. Warming here makes
    `models_loaded[]` honest: a model in that list has actually paid its
    cold-start. Returns {model: "loaded" | "skipped:<reason>"} for logging.
    Set EMEM_SIDECAR_WARM_ON_STARTUP=0 to fall back to lazy load (the
    /health `warming[]` split still keeps callers honest in that mode).
    """
    results: dict[str, str] = {}
    loaders = (
        ("dynamics_v2", _REG.load_dynamics),
        ("prithvi_eo_v2_300m_tl", _REG.load_prithvi),
        (f"galileo_{GALILEO_VARIANT}_v1", _REG.load_galileo),
        ("clay_v1_5", _REG.load_clay),
    )
    for name, loader in loaders:
        try:
            loader()
            results[name] = "loaded"
        except FileNotFoundError as e:
            results[name] = f"skipped:not_resolvable ({e})"
        except Exception as e:  # noqa: BLE001 — warm-load must never crash boot
            results[name] = f"skipped:error ({type(e).__name__}: {e})"
    return results


@asynccontextmanager
async def _lifespan(_app: FastAPI):
    """Warm-load resolvable models at startup so /health's models_loaded[]
    is honest and the first cold /predict doesn't blow the caller timeout
    (audit 2026-05-30 #6). Set EMEM_SIDECAR_WARM_ON_STARTUP=0 to keep the
    old lazy behaviour (the /health loaded[] vs warming[] split still tells
    callers which models are cold)."""
    if os.environ.get("EMEM_SIDECAR_WARM_ON_STARTUP", "1") == "1":
        print(
            "[sidecar] warm-loading resolvable models on startup…",
            file=sys.stderr,
            flush=True,
        )
        for name, status in _warm_load_resolvable_models().items():
            print(f"[sidecar]   {name}: {status}", file=sys.stderr, flush=True)
    else:
        print(
            "[sidecar] EMEM_SIDECAR_WARM_ON_STARTUP=0 → lazy load; "
            "/health splits loaded[] vs warming[]",
            file=sys.stderr,
            flush=True,
        )
    yield


app = FastAPI(
    title="emem-jepa-sidecar",
    description="GPU inference sidecar for emem-server",
    version="0.0.1",
    lifespan=_lifespan,
)


def _resolvable_model_tags() -> list[str]:
    """Model capability tags whose checkpoints are resolvable in the local
    cache (independent of whether they're loaded yet)."""
    tags: list[str] = []
    cuda_available = _REG.device.type == "cuda"
    # dynamics is CPU-or-GPU and always resolvable when its artifacts exist.
    if (DYNAMICS_DIR / "dynamics_v2.onnx").exists() or (
        DYNAMICS_DIR / "dynamics_v2.state_dict.pt"
    ).exists():
        tags.append("dynamics_v2")
    prithvi_ckpt, prithvi_cfg = _resolve_prithvi_files()
    if (
        prithvi_ckpt is not None
        and prithvi_cfg is not None
        and prithvi_ckpt.exists()
        and prithvi_cfg.exists()
    ):
        tags.append("prithvi_eo_v2_300m_tl")
    galileo_dir = _resolve_galileo_dir()
    if galileo_dir is not None and (galileo_dir / "encoder.pt").exists():
        tags.append(f"galileo_{GALILEO_VARIANT}_v1")
    clay_ckpt = _resolve_clay_ckpt()
    if clay_ckpt is not None and clay_ckpt.exists() and CLAY_METADATA_PATH.exists():
        tags.append("clay_v1_5")
    return tags


@app.get("/health")
def health() -> dict[str, Any]:
    """Sidecar health + capability report.

    Adopts the KServe v2 + Triton consensus shape so the Rust
    dispatcher can capability-negotiate at planning time:

      * `live` / `ready` — KServe v2 mandates these flat booleans
        for liveness vs. readiness probes (see KServe v2 spec).
        A model is `live` as soon as the process is up; `ready`
        means at least one GPU model finished its warm pass when
        CUDA is available, or the CPU fallback is functional when
        it isn't.
      * `version` — sidecar package version (KServe v2).
      * `extensions[]` — capability tags the dispatcher reads to
        route algorithm-level GPU gates. The Rust side filters
        algorithms whose `inference.required_extension` isn't in
        this list. `gpu` is included only when CUDA is available
        AND at least one GPU-resident model loaded successfully;
        per-model tags (e.g. `clay-v1.5`) are added once that
        specific load completes.
      * `models[]` / `cuda{}` — Triton-style detail blocks for
        Prometheus / OpenTelemetry scraping.

    This shape is stable; new fields land additively. Don't break
    the `status: "ok"` legacy shape — Rust callers from earlier
    builds still parse it, and the field is preserved here.
    """
    models: list[str] = []
    extensions: list[str] = []
    if _REG.dynamics is not None:
        models.append("dynamics_v2")
        extensions.append("jepa-v2")
    if _REG.prithvi is not None:
        models.append("prithvi_eo_v2_300m_tl")
    if _REG.galileo is not None:
        models.append(f"galileo_{GALILEO_VARIANT}_v1")
    if _REG.clay is not None:
        models.append("clay_v1_5")
    cuda_block = _REG.cuda_props()
    cuda_available = bool(cuda_block.get("available", False))
    if cuda_available:
        extensions.append("gpu")
    # Per-model capability tags advertise *reachability*, not load
    # state. We declare a model "available" the moment its checkpoint
    # is resolvable in the local HF cache — even before the lazy load
    # has kicked in. Otherwise an agent that hits /v1/topics on a cold
    # box sees `available_now=false` for every GPU algorithm and gives
    # up before the first /predict warms the model. The first call
    # pays the cold-start latency, but the dispatcher's planning-time
    # filter no longer mis-classifies cold-but-cacheable as offline.
    if cuda_available:
        prithvi_ckpt, prithvi_cfg = _resolve_prithvi_files()
        if prithvi_ckpt is not None and prithvi_cfg is not None \
                and prithvi_ckpt.exists() and prithvi_cfg.exists():
            extensions.append("prithvi-eo-2.0")
        galileo_dir = _resolve_galileo_dir()
        if galileo_dir is not None and (galileo_dir / "encoder.pt").exists():
            extensions.append(f"galileo-{GALILEO_VARIANT}")
        clay_ckpt = _resolve_clay_ckpt()
        if (
            clay_ckpt is not None
            and clay_ckpt.exists()
            and CLAY_METADATA_PATH.exists()
        ):
            extensions.append("clay-v1.5")
    # Honest loaded-vs-warming split (audit 2026-05-30 #6). `models_loaded`
    # / `models[]` are models that have ACTUALLY completed a load (warm).
    # `warming[]` are resolvable-but-not-yet-loaded — the first /predict
    # on one pays the cold-start, so callers should budget for it. With the
    # default EMEM_SIDECAR_WARM_ON_STARTUP=1 this list is normally empty
    # because the startup hook warm-loads everything resolvable.
    warming = [t for t in _resolvable_model_tags() if t not in models]
    return {
        # Legacy shape — older Rust clients depend on `status` and
        # `models_loaded`. Keep these.
        "status": "ok",
        "models_loaded": models,
        "device": str(_REG.device),
        "cuda": cuda_block,
        "uptime_s": int(time.monotonic()),
        "pid": os.getpid(),
        # KServe v2 / Triton consensus additions (2026-05).
        "live": True,
        "ready": cuda_available or len(models) > 0,
        "version": app.version,
        "extensions": extensions,
        # Additive: resolvable-but-cold models (cold-start budget hint).
        "warming": warming,
        "name": "emem-jepa-sidecar",
    }


@app.post("/predict/dynamics_v2")
def predict_dynamics(req: DynamicsRequest) -> DynamicsResponse:
    """Multi-band scalar next-step prediction.

    Input is K_LAGS*N_BANDS pre-normalised scalars + N_CONTEXT spatial-
    temporal context floats. Output is per-band physical-unit
    predictions + per-band confidence + the canonical band-key list +
    a `low_confidence_bands` slice listing keys below
    `LOW_CONFIDENCE_THRESHOLD`.
    """
    try:
        model, meta = _REG.load_dynamics()
    except FileNotFoundError as e:
        raise HTTPException(status_code=503, detail=str(e)) from e
    except RuntimeError as e:
        # Trained-checkpoint validation failure (architecture drift,
        # blake2b mismatch). 503 so the Rust client surfaces as
        # Upstream (502) — emem-server then refuses to silently fall
        # back to identity because the operator explicitly attested
        # a trained checkpoint and we caught the swap.
        raise HTTPException(status_code=503, detail=str(e)) from e

    # Flatten lags + context into a single [1, INPUT_DIM] tensor.
    flat = np.concatenate(
        [
            np.asarray(req.lags_normalised, dtype=np.float32),
            np.asarray(req.context, dtype=np.float32),
        ]
    )
    x = torch.from_numpy(flat).unsqueeze(0).to(_REG.device)   # [1, INPUT_DIM]
    t0 = time.perf_counter_ns()
    with torch.inference_mode():
        raw = model(x)                                         # [1, OUTPUT_DIM]
    dt_us = max(1, (time.perf_counter_ns() - t0) // 1000)
    raw_row = raw.squeeze(0).cpu().tolist()                    # length OUTPUT_DIM
    mean_norm = raw_row[:N_BANDS]
    log_var = raw_row[N_BANDS:]

    # Denormalise + clamp per-band using the metadata pinned at
    # training time. Keeps the sidecar's response in physical units so
    # the Rust caller doesn't have to duplicate the normalisation table.
    band_norm = meta.get("normalisation", {}).get("band_norm", {})
    phys_range = meta.get("normalisation", {}).get("physical_ranges", {})
    predictions: list[float] = []
    confidence: list[float] = []
    low_confidence: list[str] = []
    for j, band in enumerate(BAND_KEYS_SIDECAR):
        m, s = band_norm.get(band, (0.0, 1.0))
        lo, hi = phys_range.get(band, (-1e30, 1e30))
        phys = mean_norm[j] * float(s) + float(m)
        phys = float(np.clip(phys, lo, hi))
        predictions.append(phys)
        lv = float(np.clip(log_var[j], -6.0, 4.0))
        conf = float(np.exp(-lv / 2.0))
        confidence.append(conf)
        if conf < LOW_CONFIDENCE_THRESHOLD:
            low_confidence.append(band)

    trained = bool(meta.get("training", {}).get("trained", False))
    onnx_path = DYNAMICS_DIR / "dynamics_v2.onnx"
    artifact_blake2b = (
        meta.get("artifact", {}).get("blake2b_hex")
        or hashlib.blake2b(
            onnx_path.read_bytes() if onnx_path.exists() else b"", digest_size=32
        ).hexdigest()
    )

    honesty_warnings: list[str] = []
    if not trained:
        honesty_warnings.append(
            "untrained_baseline: dynamics_v2 metadata flags training.trained=false; "
            "the sidecar is serving a zero-init head whose mean is the climatological "
            "default for each band. Run python/jepa_v2_sidecar/train_dynamics_v2.py "
            "to ship a real trained model."
        )
    synthetic_fraction = float(
        meta.get("training", {}).get("synthetic_fraction_train", 1.0 if trained else 0.0)
    )
    if trained and synthetic_fraction > 0.5:
        honesty_warnings.append(
            f"training_synthetic_fraction={synthetic_fraction:.2f}: the trained model "
            "was fit predominantly on synthesised seasonal data; the held-out "
            "real-NDVI MSE is reported in metadata.training.real_ndvi_mse_model"
        )
    for band in low_confidence:
        honesty_warnings.append(
            f"low_confidence_band:{band}: predictive std-dev exceeded the "
            f"LOW_CONFIDENCE_THRESHOLD ({LOW_CONFIDENCE_THRESHOLD:.2f}); downstream "
            "agents should fall back to other signals or wait for more lags."
        )

    # Skill vs persistence (predict-last-value) on real NDVI, surfaced from
    # the training metadata (audit 2026-05-30 HIGH #5). When skill < 0 the
    # trained head is WORSE than "tomorrow looks like today" on real ground
    # truth — emit a loud honesty warning so callers prefer persistence.
    training_meta = meta.get("training", {})
    skill_vs_persistence: dict[str, Any] | None = None
    real_skill = training_meta.get("real_ndvi_skill_score")
    if real_skill is not None:
        beats = bool(training_meta.get("beats_persistence_on_real_ndvi", real_skill > 0.0))
        skill_vs_persistence = {
            "real_ndvi": float(real_skill),
            "beats_persistence": beats,
            "reference": "predict-last-value",
        }
        if real_skill < 0.0:
            honesty_warnings.append(
                f"NEGATIVE_SKILL: worse than predict-last-value on real NDVI "
                f"(skill={float(real_skill):.4f}); prefer persistence"
            )

    return DynamicsResponse(
        predictions=predictions,
        confidence=confidence,
        band_keys=list(BAND_KEYS_SIDECAR),
        low_confidence_bands=low_confidence,
        model={
            "model_id": meta.get("model_id", "jepa_temporal_predictor@2.scalar"),
            "version": meta.get("version", "0.0.1-trained-multi-band-scalar"),
            "blake2b_hex": artifact_blake2b,
            "trained": trained,
            "synthetic_fraction_train": synthetic_fraction,
            "via": "python_sidecar",
            "honesty_warnings": honesty_warnings,
        },
        inference_us=dt_us,
        device=str(_REG.device),
        skill_vs_persistence=skill_vs_persistence,
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


@app.post("/predict/galileo_embed")
def predict_galileo_embed(req: GalileoRequest) -> GalileoResponse:
    """Compute the per-cell Galileo embedding (multimodal-capable).

    Variant (tiny | base | nano) is determined by EMEM_GALILEO_VARIANT
    at sidecar startup; the response carries `model.model_id` so callers
    know which variant produced the embedding.

    S2 is always fed. S1 (VV,VH γ0 dB), SRTM (elevation, slope) and TC
    (TerraClimate def/soil/aet, the TIME group) are fed when `s1_chip` /
    `srtm_chip` / `tc_chip` are present in the request — their group masks
    flip to 0 (seen) and each gets its per-modality (mean-2σ)/(4σ)
    normalization (Galileo's upstream Normalizer scheme, std=True;
    arXiv:2502.09356). Modalities not supplied (and ERA5, VIIRS, DW, WC,
    LandScan, location which emem doesn't yet wire) stay zero-filled +
    masked-absent. The encoder is robust to this — it was
    trained with the full mask schedule. Returns the average-pooled token
    output (768-D base; config.json embedding_size) per the canonical
    Galileo embedding recipe (cf. visualizing_embeddings.py upstream).
    """
    try:
        encoder, meta = _REG.load_galileo()
    except FileNotFoundError as e:
        raise HTTPException(status_code=503, detail=str(e)) from e

    sys.path.insert(0, str(Path(__file__).parent))
    from single_file_galileo import (  # noqa: WPS433
        S1_BANDS,
        SPACE_BANDS,
        SPACE_BAND_GROUPS_IDX,
        SPACE_TIME_BANDS,
        SPACE_TIME_BANDS_GROUPS_IDX,
        SRTM_BANDS,
        STATIC_BANDS,
        STATIC_BAND_GROUPS_IDX,
        S2_BANDS,
        TIME_BANDS,
        TIME_BAND_GROUPS_IDX,
    )

    device = _REG.device
    arr = np.asarray(req.s2_chip, dtype=np.float32)  # [T, H, W, 10]
    # Apply per-band normalization with Galileo's upstream Normalizer
    # scheme (shift/divide, NOT z-score): x_norm = (x - shift) / div with
    # shift = mean - 2σ, div = 4σ (audit 2026-05-30 HIGH fix #1; cf.
    # galileo/src/data/dataset.py Normalizer std=True, arXiv:2502.09356).
    galileo_dir = _resolve_galileo_dir()
    shift, div, norm_source = _galileo_s2_shift_div(galileo_dir)
    s2_n = (arr - shift) / div
    # Reorder to [H, W, T, 10] which is what construct_galileo_input expects.
    s2_t = torch.from_numpy(s2_n).permute(1, 2, 0, 3).contiguous().to(device)

    # Build the masked-output tuple inline. S2 is always seen; S1 + SRTM
    # are seen only when provided. The remaining modalities are zero-filled
    # and their group masks stay at 1 (= "not seen by the encoder").
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

    modality_subset = ["s2"]

    # ── S1 (VV, VH) into the space-time tensor when provided ────────────
    if req.s1_chip is not None:
        s1_arr = np.asarray(req.s1_chip, dtype=np.float32)  # [T, H, W, 2]
        s1_shift, s1_div = _galileo_modality_shift_div(
            GALILEO_S1_MEAN, GALILEO_S1_STD
        )
        s1_n = (s1_arr - s1_shift) / s1_div
        # NaN pixels (water mask / nodata, marked NaN by the Rust fetcher)
        # → 0 after normalization. 0 in normalized space ≈ the modality
        # mean (since shift centres near the mean), the least-biased fill,
        # and the patch's other pixels still carry signal. The S1 group is
        # still marked seen — Galileo has no per-pixel mask, only per-group.
        s1_n = np.nan_to_num(s1_n, nan=0.0, posinf=0.0, neginf=0.0)
        s1_t = torch.from_numpy(s1_n).permute(1, 2, 0, 3).contiguous().to(device)
        s1_indices = [SPACE_TIME_BANDS.index(b) for b in S1_BANDS]
        s_t_x[:, :, :, s1_indices] = s1_t
        s1_group = [
            i for i, k in enumerate(SPACE_TIME_BANDS_GROUPS_IDX) if k == "S1"
        ]
        s_t_m[:, :, :, s1_group] = 0  # seen
        modality_subset.append("s1")

    sp_x = torch.zeros((h, w, len(SPACE_BANDS)), dtype=torch.float, device=device)
    sp_m = torch.ones(
        (h, w, len(SPACE_BAND_GROUPS_IDX)), dtype=torch.float, device=device
    )

    # ── SRTM (elevation, slope) into the space tensor when provided ─────
    if req.srtm_chip is not None:
        srtm_arr = np.asarray(req.srtm_chip, dtype=np.float32)  # [H, W, 2]
        srtm_shift, srtm_div = _galileo_modality_shift_div(
            GALILEO_SRTM_MEAN, GALILEO_SRTM_STD
        )
        srtm_n = (srtm_arr - srtm_shift) / srtm_div
        srtm_n = np.nan_to_num(srtm_n, nan=0.0, posinf=0.0, neginf=0.0)
        srtm_t = torch.from_numpy(srtm_n).contiguous().to(device)  # [H, W, 2]
        srtm_indices = [SPACE_BANDS.index(b) for b in SRTM_BANDS]
        sp_x[:, :, srtm_indices] = srtm_t
        srtm_group = [
            i for i, k in enumerate(SPACE_BAND_GROUPS_IDX) if k == "SRTM"
        ]
        sp_m[:, :, srtm_group] = 0  # seen
        modality_subset.append("srtm")

    t_x = torch.zeros((t, len(TIME_BANDS)), dtype=torch.float, device=device)
    t_m = torch.ones((t, len(TIME_BAND_GROUPS_IDX)), dtype=torch.float, device=device)
    st_x = torch.zeros((len(STATIC_BANDS),), dtype=torch.float, device=device)
    st_m = torch.ones(
        (len(STATIC_BAND_GROUPS_IDX),), dtype=torch.float, device=device
    )

    # ── TerraClimate (def, soil, aet) into the TIME tensor when provided ─
    # The TIME tensor has no spatial dimension (`t_x` is [T, len(TIME_BANDS)]).
    # We fill the three TC band slots and flip the TC group mask to seen;
    # ERA5 + VIIRS stay masked-absent (emem doesn't wire them). Same upstream
    # Normalizer scheme as the other std_bands: x_norm = (x - (mean-2σ))/(4σ).
    tc_fed = False
    if req.tc_chip is not None:
        from single_file_galileo import TC_BANDS  # noqa: WPS433

        tc_arr = np.asarray(req.tc_chip, dtype=np.float32)  # [T, 3] (def,soil,aet)
        tc_shift, tc_div = _galileo_modality_shift_div(GALILEO_TC_MEAN, GALILEO_TC_STD)
        tc_n = (tc_arr - tc_shift) / tc_div
        tc_n = np.nan_to_num(tc_n, nan=0.0, posinf=0.0, neginf=0.0)
        tc_t = torch.from_numpy(tc_n).contiguous().to(device)  # [T, 3]
        tc_indices = [TIME_BANDS.index(b) for b in TC_BANDS]  # [2, 3, 4]
        t_x[:, tc_indices] = tc_t
        tc_group = [i for i, k in enumerate(TIME_BAND_GROUPS_IDX) if k == "TC"]
        t_m[:, tc_group] = 0  # seen
        modality_subset.append("tc")
        tc_fed = True

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

    # Time mask: when TerraClimate is fed, `t_m` already has the TC group
    # flipped to seen (0) with ERA5/VIIRS still masked-absent (1), so pass
    # it through. When no TIME modality is fed, force all-ones so the whole
    # time group is excluded from the token pool (the historical behaviour).
    time_mask = t_m if tc_fed else torch.ones_like(t_m)

    t0 = time.perf_counter_ns()
    with torch.inference_mode():
        # The recipe per visualizing_embeddings.py: pass the seen-modality
        # masks (S2 always; S1 in s_t_m, SRTM in sp_m, TC in t_m when
        # provided), keep the static mask all-ones (no LandScan/location
        # wired here), call forward, then average over unmasked tokens.
        model_output = encoder(
            s_t_x.float(),
            sp_x.float(),
            t_x.float(),
            st_x.float(),
            s_t_m,
            sp_m,
            time_mask,
            torch.ones_like(st_m),
            months.long(),
            patch_size=GALILEO_PATCH_SIZE,
            # Chip is sampled at 30 m; without this the encoder defaults
            # to BASE_GSD=10 → gsd_ratio = 10*2/10 = 2 instead of the
            # correct 30*2/10 = 6, mis-setting the spatial positional
            # encoding (audit 2026-05-30 HIGH fix #2; single_file_galileo.py
            # Encoder.forward input_resolution_m, gsd_ratio math ~line 711).
            input_resolution_m=GALILEO_CHIP_GSD_M,
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
            "config": {
                **meta["config"],
                "normalization_source": norm_source,
                "modality_subset": modality_subset,
            },
            "honesty_warnings": _galileo_honesty_warnings(
                norm_source, modality_subset
            ),
        },
        inference_us=dt_us,
        device=str(device),
    )


def _galileo_honesty_warnings(
    norm_source: str, modality_subset: list[str]
) -> list[str]:
    """Honest disclosure of which Galileo modalities were fed vs masked.

    S2 is always present; S1 + SRTM + TC (TerraClimate def/soil/aet) are
    present only when the caller supplied the chips. The remaining
    modalities (ERA5/VIIRS/DW/WC/LandScan/location) are not wired in emem
    and are always masked-absent. TC fills Galileo's TIME group; ERA5 and
    VIIRS (the other two TIME-group members) stay masked-absent.
    """
    fed = ", ".join(modality_subset)
    all_galileo = {"s2", "s1", "srtm", "tc"}
    not_fed = sorted(all_galileo - set(modality_subset))
    warns = [
        f"normalization_source: S2={norm_source}; S1/SRTM/TC use computed "
        "(mean-2σ)/(4σ) from config/normalization.json (galileo @ main, "
        "verified 2026-05-30)",
        f"modality_subset: [{fed}] fed to the encoder; per-modality "
        "(mean-2σ)/(4σ) normalization applied to each",
    ]
    if not_fed:
        warns.append(
            "frozen_pretrained_encoder: per-cell forward through the frozen "
            f"Galileo encoder. Wired but not provided at this cell: "
            f"[{', '.join(not_fed)}]; never-wired in emem: "
            "[ERA5, VIIRS, DW, WC, LandScan, location] — all "
            "zero-filled + masked-absent (not zero-filled-and-claimed)."
        )
    else:
        warns.append(
            "frozen_pretrained_encoder: per-cell forward through the frozen "
            "Galileo encoder with S2+S1+SRTM+TC all fed. Never-wired in emem: "
            "[ERA5, VIIRS, DW, WC, LandScan, location] — masked-absent."
        )
    return warns


@app.post("/predict/clay_embed")
def predict_clay_embed(req: ClayRequest) -> ClayResponse:
    """Compute the per-cell Clay Foundation Model v1.5 embedding.

    Returns the encoder's CLS token (1024-D) suitable for cosine
    similarity, downstream linear probes, or k-NN retrieval. The chip
    is normalised with the model's per-band mean/std before the
    forward pass; the temporal / spatial encoders are engaged when
    year+month and lng+lat are supplied (recommended — Clay's
    sensor-agnostic strength comes from those conditioning vectors).

    Recipe is the wall-to-wall tutorial verbatim:
      1. v2.Normalize(mean, std) over [B, 10, 256, 256] reflectance
      2. Build datacube with platform / time / latlon / pixels / gsd / waves
      3. encoder(datacube) → unmsk_patch [B, 1024+1, 1024]
      4. embedding = unmsk_patch[:, 0, :]   # CLS token at position 0
    """
    try:
        model, meta = _REG.load_clay()
    except FileNotFoundError as e:
        raise HTTPException(status_code=503, detail=str(e)) from e

    import math  # noqa: WPS433
    from torchvision.transforms import v2  # noqa: WPS433

    arr = np.asarray(req.chip, dtype=np.float32)  # [10, 256, 256]
    if arr.shape != (10, CLAY_CHIP_PIXELS, CLAY_CHIP_PIXELS):
        raise HTTPException(
            status_code=400,
            detail=(
                f"clay chip must be [10, {CLAY_CHIP_PIXELS}, {CLAY_CHIP_PIXELS}]; "
                f"got {arr.shape}"
            ),
        )
    pixels = torch.from_numpy(arr).unsqueeze(0).to(_REG.device)
    # Per-band normalize. v2.Normalize wants mean / std as lists.
    mean = meta["config"]["mean_per_band"]
    std = meta["config"]["std_per_band"]
    pixels = v2.Normalize(mean=mean, std=std)(pixels)

    # Temporal encoding: sin/cos of week-of-year + hour-of-day. Default
    # to mid-year noon when caller doesn't pass year/month so the model
    # still receives a coherent vector rather than zeros (which would
    # land it in an undertrained corner of the conditioning space).
    fallback_warnings: list[str] = []
    if req.year is not None and req.month is not None:
        # Approximate week-of-year from year/month/(day or 15).
        from datetime import date  # noqa: WPS433
        d = date(req.year, req.month, req.day or 15)
        woy = d.isocalendar().week
    else:
        woy = 26  # mid-year fallback
        # Honesty (audit 2026-05-30 fix #4): the time-conditioning vector
        # is NOT the scene's real acquisition time — it encodes a mid-year
        # noon default. The embedding is therefore conditioned on a guessed
        # season. (Live Rust path always sends real year+month, so this
        # fires only on a degraded caller.)
        fallback_warnings.append(
            "time_defaulted: scene year/month absent; week-of-year defaulted "
            "to 26 (mid-year) and hour to solar noon. The time-conditioning "
            "vector does NOT reflect the real acquisition time."
        )
    week_rad = woy * 2 * math.pi / 52
    hour_rad = 12 * 2 * math.pi / 24  # solar noon
    time_vec = torch.tensor(
        [[math.sin(week_rad), math.cos(week_rad), math.sin(hour_rad), math.cos(hour_rad)]],
        dtype=torch.float32,
        device=_REG.device,
    )

    if req.lng is not None and req.lat is not None:
        lat_rad = req.lat * math.pi / 180
        lon_rad = req.lng * math.pi / 180
        latlon_vec = torch.tensor(
            [[math.sin(lat_rad), math.cos(lat_rad), math.sin(lon_rad), math.cos(lon_rad)]],
            dtype=torch.float32,
            device=_REG.device,
        )
    else:
        # NB: zero-filling here is NOT "unknown location" — sin(0)=0,
        # cos(0)=1 encodes lat=0,lng=0 (Null Island), a concrete point the
        # encoder treats as real. Flag it (audit 2026-05-30 fix #4) so the
        # Rust side never threads a Null-Island-conditioned embedding into a
        # signed fact as if the location were known. (Live path always sends
        # real lat/lng, so this fires only on a degraded caller.)
        latlon_vec = torch.zeros((1, 4), dtype=torch.float32, device=_REG.device)
        fallback_warnings.append(
            "location_defaulted: scene lat/lng absent; latlon-conditioning "
            "vector zero-filled, which encodes lat=0,lng=0 (Null Island) as a "
            "REAL point — not 'unknown'. The embedding is conditioned on (0,0)."
        )

    waves = torch.tensor(
        meta["config"]["wavelength_um_per_band"],
        dtype=torch.float32,
        device=_REG.device,
    )
    datacube = {
        "platform": "sentinel-2-l2a",
        "time": time_vec,
        "latlon": latlon_vec,
        "pixels": pixels,
        "gsd": torch.tensor(CLAY_CHIP_GSD_M, device=_REG.device),
        "waves": waves,
    }

    t0 = time.perf_counter_ns()
    with torch.inference_mode():
        unmsk_patch, _unmsk_idx, _msk_idx, _msk_matrix = model.model.encoder(
            datacube
        )
    dt_us = max(1, (time.perf_counter_ns() - t0) // 1000)

    # CLS token at position 0 — the per-chip foundation embedding.
    embedding = unmsk_patch[:, 0, :].squeeze(0).detach().cpu().tolist()

    return ClayResponse(
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
                *fallback_warnings,
                "frozen_pretrained_encoder: this is a per-cell forward "
                "pass through the frozen Clay v1.5 encoder (no fine-tuning "
                "at this responder). The CLS embedding captures whatever "
                "Clay's MAE+DINOv2 multi-sensor pretraining captured; "
                "downstream task fit depends on that alignment.",
                "wavelength_conditioned: the embedding is sensitive to the "
                "(platform, wavelengths, gsd) triple supplied at encode "
                "time. Two recalls with different platform tags produce "
                "deliberately different vectors — always cite "
                "model_blake2b alongside the fact."
            ],
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

    sock = os.environ.get("EMEM_SIDECAR_SOCK", "/run/emem/jepa_sidecar.sock")
    print(f"emem-jepa-sidecar listening on {sock}", file=sys.stderr, flush=True)
    uvicorn.run(app, uds=sock, log_level="info")


if __name__ == "__main__":
    main()
