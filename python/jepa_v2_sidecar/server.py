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
# (86.5 M params, 330 MB ckpt, 768-D embedding); override
# EMEM_GALILEO_VARIANT to swap to tiny / nano. EMEM_GALILEO_SNAPSHOT
# pins an explicit folder path for air-gapped deployments.
GALILEO_REPO = "nasaharvest/galileo"
GALILEO_VARIANT = os.environ.get("EMEM_GALILEO_VARIANT", "base")
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
        # load actual learned weights (state_dict.pt) before serving —
        # otherwise we'd emit randomly-initialised PyTorch outputs
        # under a "trained" receipt, which would corrupt every
        # downstream comparison + inflate verifier trust. The
        # checkpoint must:
        #   1. exist on disk next to the metadata file,
        #   2. match the architecture currently compiled in (we let
        #      load_state_dict's strict=True surface any layer-name
        #      drift loudly), and
        #   3. carry the blake2b that metadata pinned (so a swapped
        #      .pt file under a stale metadata.json fails closed).
        trained = bool(meta.get("training", {}).get("trained", False))
        model = DynamicsModel().to(self.device)
        if trained:
            ckpt_path = DYNAMICS_DIR / "dynamics_v2.state_dict.pt"
            if not ckpt_path.exists():
                raise FileNotFoundError(
                    f"dynamics_v2 metadata claims trained=true but no checkpoint "
                    f"found at {ckpt_path}. Either drop training.trained back to "
                    "false (residual-zero sentinel) or place the trained state_dict "
                    "alongside the metadata."
                )
            expected_b2b = (
                meta.get("training", {}).get("checkpoint_blake2b_hex")
                or meta.get("model", {}).get("blake2b_hex")
            )
            if expected_b2b:
                import hashlib
                got_b2b = hashlib.blake2b(ckpt_path.read_bytes()).hexdigest()
                if got_b2b != expected_b2b:
                    raise RuntimeError(
                        "dynamics_v2 trained checkpoint blake2b mismatch "
                        f"(got {got_b2b[:16]}..., metadata pinned "
                        f"{expected_b2b[:16]}...). Refusing to serve a swapped "
                        "checkpoint under a stale receipt — re-run "
                        "python/jepa_v2/export_baseline.py to refresh the metadata "
                        "or restore the matching .pt file."
                    )
            # weights_only=True so a malicious .pt cannot side-effect
            # via torch's pickle (defense in depth — the checkpoint
            # comes from our own training pipeline, but the sidecar
            # treats any on-disk artifact as untrusted input).
            state_dict = torch.load(
                ckpt_path, map_location=self.device, weights_only=True
            )
            # strict=True: any architecture drift between the trained
            # checkpoint and the model class compiled into the sidecar
            # is exactly the kind of silent corruption the original
            # guard was protecting against. Surface it as a load-time
            # error, not a runtime garbage-prediction.
            model.load_state_dict(state_dict, strict=True)
        else:
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
app = FastAPI(
    title="emem-jepa-sidecar",
    description="GPU inference sidecar for emem-server",
    version="0.0.1",
)


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
        extensions.append("prithvi-eo-2.0")
    if _REG.galileo is not None:
        models.append(f"galileo_{GALILEO_VARIANT}_v1")
        extensions.append(f"galileo-{GALILEO_VARIANT}")
    if _REG.clay is not None:
        models.append("clay_v1_5")
        extensions.append("clay-v1.5")
    cuda_block = _REG.cuda_props()
    cuda_available = bool(cuda_block.get("available", False))
    if cuda_available:
        extensions.append("gpu")
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
        "name": "emem-jepa-sidecar",
    }


@app.post("/predict/dynamics_v2")
def predict_dynamics(req: DynamicsRequest) -> DynamicsResponse:
    try:
        model, meta = _REG.load_dynamics()
    except FileNotFoundError as e:
        raise HTTPException(status_code=503, detail=str(e)) from e
    except RuntimeError as e:
        # Trained-checkpoint validation failure (architecture drift,
        # blake2b mismatch). 503 so the Rust client surfaces as
        # Upstream (502) — emem-server then refuses to silently fall
        # back to the residual-zero baseline because the user
        # explicitly attested a trained checkpoint.
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


@app.post("/predict/galileo_embed")
def predict_galileo_embed(req: GalileoRequest) -> GalileoResponse:
    """Compute the per-cell Galileo embedding from an S2-only chip.

    Variant (tiny | base | nano) is determined by EMEM_GALILEO_VARIANT
    at sidecar startup; the response carries `model.model_id` so callers
    know which variant produced the embedding.

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
    if req.year is not None and req.month is not None:
        # Approximate week-of-year from year/month/(day or 15).
        from datetime import date  # noqa: WPS433
        d = date(req.year, req.month, req.day or 15)
        woy = d.isocalendar().week
    else:
        woy = 26  # mid-year fallback
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
        latlon_vec = torch.zeros((1, 4), dtype=torch.float32, device=_REG.device)

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
