"""Train the *real* `jepa_temporal_predictor@2.scalar` dynamics head.

PIVOT FROM THE V0 SENTINEL
==========================
The original `dynamics_v2.onnx` was a zero-initialised residual MLP that
returned the last input vintage by construction. Its premise — three
historical Tessera 128-D embeddings → next-vintage prediction — fails
because the upstream `dl2.geotessera.org` bucket only serves the 2024
vintage reliably (2017-2023 return null). On this responder all three
"lags" collapse to the same vector and the identity short-circuit
fires.

This module replaces that sentinel with a real, trained multi-band
scalar dynamics predictor:

    Input  : K=6 lags of a 4-band scalar state {ndvi, lst_day,
             lst_night, pm25} → 24 scalars
           + (cell_lat, cell_lng_sin, cell_lng_cos) spatial context
           + (month_sin, month_cos) calendar context  → 5 scalars
           → final flat shape [B, 29]

    Output : next-step (4 mean predictions, 4 log-variance predictions)
             → [B, 8] which the wire wraps as {predictions[4],
             confidence[4]}.

Architecture: small Transformer encoder (2 layers, 4 heads, hidden 32)
operating over the 6 lag steps, with the lag-token sequence augmented
by a learned [CLS] token whose final embedding is projected to the
output head. Parameter count is ~12-15k — fits on CPU, trains in ~2
minutes on this box.

TRAINING DATA — HONEST CAVEAT
=============================
The corpus audit (see `training_data_report.md`) showed that NDVI is
the *only* candidate band with usable temporal depth on this responder
(96 cells with ≥4 monthly tslots; one with 68). LST/PM25/weather have
~1 tslot per cell. So we can't train end-to-end on real multi-tslot
quadruples here.

The training set is therefore predominantly **synthetic** — generated
from a physically-motivated seasonality model documented in
`generate_synthetic_pairs()` below — with the ~96 real multi-tslot
NDVI cells folded in as an additional held-out validation slice. The
model thereby learns physically reasonable dynamics (seasonal cycle,
latitudinal phase shift, soft trend) and is later validated against
the real NDVI history that does exist.

This is honest: the metadata records `training.trained: true` AND
`training.data.synthetic_fraction: ~0.95` so the receipt verifier sees
exactly how much real corpus history grounds the prediction.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import math
import os
import random
import time
from pathlib import Path
from typing import Dict, List, Tuple

import numpy as np
import torch
import torch.nn as nn
import torch.nn.functional as F

# ─── Reproducibility ────────────────────────────────────────────────
SEED = 42
random.seed(SEED)
np.random.seed(SEED)
torch.manual_seed(SEED)

# ─── Model contract ─────────────────────────────────────────────────
K_LAGS = 6
N_BANDS = 4
BAND_KEYS = (
    "indices.ndvi",
    "modis.lst_day_8day",
    "modis.lst_night_8day",
    "cams.pm25",
)
# Physical ranges (used to clamp predictions and normalise input).
BAND_PHYSICAL_RANGES = {
    "indices.ndvi": (-1.0, 1.0),
    "modis.lst_day_8day": (220.0, 340.0),     # K, plausible for any habitable cell
    "modis.lst_night_8day": (210.0, 320.0),
    "cams.pm25": (0.0, 500.0),                # µg/m³, capped (extreme haze)
}
# Per-band normalisation: standardise to a roughly N(0,1) input. Means
# and stds are climatological estimates — not learned, fixed so the
# ONNX wire is stable across re-trains.
BAND_NORM = {
    "indices.ndvi": (0.30, 0.30),
    "modis.lst_day_8day": (290.0, 15.0),
    "modis.lst_night_8day": (280.0, 12.0),
    "cams.pm25": (15.0, 20.0),
}
# Spatial / temporal context: 5 scalars (lat_norm, lng_sin, lng_cos,
# month_sin, month_cos). All in [-1, 1].
N_CONTEXT = 5
# Flat input dimension that the ONNX file expects.
INPUT_DIM = K_LAGS * N_BANDS + N_CONTEXT      # = 29
# Flat output dimension: mean[N_BANDS] || log_var[N_BANDS].
OUTPUT_DIM = 2 * N_BANDS                      # = 8

# ─── cell64 → (lat, lng) decode (Python port) ───────────────────────
# Audit 2026-05-30 fix #5: the real-NDVI eval previously fed lat=0/lng=0
# for every cell because the Rust `latlng_from_cell64` helper isn't bound
# in Python. That biases the spatial-context vector toward Null Island and
# can flatter or flatten the reported skill. The active emem cell64 geo
# encoding is NOT true H3 geometry — it's a plain WGS-84 quantisation grid
# (crates/emem-codec/src/geo.rs), so it decodes in pure Python:
#
#   alphabet : 65,536 CVCV bigrams, deterministic product of 21 consonants
#              × 10 vowels (× same again), padded with z<hex4> entries
#              (crates/emem-codec/src/alphabet.rs).
#   cell u64 : 4 bigrams (base-65,536 digits) → 64-bit raw
#              (crates/emem-codec/src/cell64.rs).
#   geo      : prefix mode=1|res=21|base=0xab; bits 42..22 = lat_q (21b),
#              bits 21..0 = lng_q (22b); lat = lat_q/2^21*180-90,
#              lng = lng_q/2^22*360-180 (crates/emem-codec/src/geo.rs).
_CONS = "bcdfghjklmnpqrstvwxyz"  # 21
_VOWS = "aeiouAEIOU"             # 10


def _build_cell64_alphabet_index() -> Dict[str, int]:
    """Reconstruct the cell64 alphabet → index map exactly as
    crates/emem-codec/src/alphabet.rs::build_alphabet_v0()."""
    out: List[str] = []
    for c1 in _CONS:
        for v1 in _VOWS:
            for c2 in _CONS:
                for v2 in _VOWS:
                    out.append(c1 + v1 + c2 + v2)
    while len(out) < 65_536:
        out.append("z{:04x}".format(len(out)))
    out = out[:65_536]
    return {sym: i for i, sym in enumerate(out)}


_CELL64_ALPHABET_INDEX = _build_cell64_alphabet_index()

# geo.rs constants.
_GEO_LAT_BITS = 21
_GEO_LNG_BITS = 22
_GEO_LAT_MAX = (1 << _GEO_LAT_BITS) - 1
_GEO_LNG_MAX = (1 << _GEO_LNG_BITS) - 1
_GEO_PREFIX_MASK = 0xFFFF_F000_0000_0000
# (mode=1 << 60) | (res=21 << 52) | (base=0xab << 44)
_GEO_PREFIX = (1 << 60) | (21 << 52) | (0xAB << 44)


def latlng_from_cell64(s: str) -> Tuple[float, float] | None:
    """Decode a cell64 string to (lat_deg, lng_deg) bucket centre, or
    None if the string is malformed or is not a geo-aperture cell at the
    active resolution. Mirrors geo.rs::latlng_from_cell64."""
    parts = s.split(".")
    if len(parts) != 4:
        return None
    raw = 0
    for i, sym in enumerate(parts):
        idx = _CELL64_ALPHABET_INDEX.get(sym)
        if idx is None:
            return None
        raw |= idx << (48 - i * 16)
    if (raw & _GEO_PREFIX_MASK) != _GEO_PREFIX:
        return None  # not a geo cell (legacy 305 m / 16-bit grid, edge, etc.)
    lng_q = raw & _GEO_LNG_MAX
    lat_q = (raw >> _GEO_LNG_BITS) & _GEO_LAT_MAX
    lat_deg = (lat_q / _GEO_LAT_MAX) * 180.0 - 90.0
    lng_deg = (lng_q / _GEO_LNG_MAX) * 360.0 - 180.0
    return float(lat_deg), float(lng_deg)


def compute_skill_score(
    mse_model: float | None, mse_baseline: float | None
) -> float | None:
    """Skill score vs the predict-last-value (persistence) baseline:
    `skill = 1 - mse_model / mse_baseline`. Returns None when either MSE
    is missing or the baseline is non-positive (skill undefined). > 0
    means the model beats persistence; ≤ 0 means persistence wins."""
    if mse_model is None or mse_baseline is None:
        return None
    if mse_baseline <= 0.0:
        return None
    return float(1.0 - mse_model / mse_baseline)


# ─── Architecture ───────────────────────────────────────────────────
D_MODEL = 32
N_HEADS = 4
N_LAYERS = 2
DROPOUT = 0.10

# ─── Training ───────────────────────────────────────────────────────
N_TRAIN_SYNTHETIC = 12_000     # synthetic time series (each gives 1 sample)
N_VAL_SYNTHETIC = 2_000
BATCH_SIZE = 256
N_EPOCHS = 25
LR = 1e-3
WEIGHT_DECAY = 1e-5

DEFAULT_AUDIT_DUMP = Path("/tmp/emem_audit_dump")


# ─── Synthetic data generator ───────────────────────────────────────
def _ndvi_seasonal(lat_deg: float, month_idx: float, baseline: float) -> float:
    """A simple cosine-seasonal NDVI model with hemispheric phase flip.

    Northern hemisphere peaks in July (month=7), southern in January.
    Equator amplitude is small. `month_idx` is in [0, 12).
    """
    phase = 0.0 if lat_deg >= 0 else math.pi
    amp = 0.20 * abs(math.tanh(lat_deg / 30.0))
    seasonal = amp * math.cos(2 * math.pi * (month_idx - 7) / 12 + phase)
    return float(np.clip(baseline + seasonal, -1.0, 1.0))


def _lst_day_seasonal(lat_deg: float, month_idx: float, baseline: float) -> float:
    """Daytime LST in Kelvin. Peaks in summer (hemispheric phase), ±15 K swing
    at high latitudes, ±3 K near the equator. Diurnal effects absorbed
    into baseline; this is only the seasonal anomaly."""
    phase = 0.0 if lat_deg >= 0 else math.pi
    amp = 3.0 + 12.0 * abs(math.tanh(lat_deg / 30.0))
    seasonal = amp * math.cos(2 * math.pi * (month_idx - 7) / 12 + phase)
    return float(np.clip(baseline + seasonal, 220.0, 340.0))


def _lst_night_seasonal(lat_deg: float, month_idx: float, baseline: float) -> float:
    return float(np.clip(_lst_day_seasonal(lat_deg, month_idx, baseline) - 8.0, 210.0, 320.0))


def _pm25_seasonal(lat_deg: float, month_idx: float, baseline: float) -> float:
    """PM2.5 µg/m³. Winter-peaked (boundary-layer collapse + heating
    emissions in mid-latitudes); flat near the equator. Hemispheric
    phase flips."""
    phase = math.pi if lat_deg >= 0 else 0.0
    amp = 1.0 + 4.0 * max(0.0, abs(lat_deg) / 30.0)
    seasonal = amp * math.cos(2 * math.pi * (month_idx - 1) / 12 + phase)
    # Multiply baseline so dirty regions amplify; clamp ≥ 0.
    return float(np.clip(baseline + seasonal, 0.0, 500.0))


def _gen_series(lat_deg: float, length: int, rng: np.random.Generator) -> np.ndarray:
    """Generate a `length`-step monthly multi-band series for a cell.

    Returns array of shape [length, 4] in physical units.
    """
    # Per-cell baselines: NDVI baseline correlates with |lat| (tropics
    # greener than poles), PM25 baseline draws from a heavy-tail
    # log-normal so a few "polluted" cells are sampled.
    ndvi_base = 0.55 - 0.40 * (abs(lat_deg) / 90.0) + rng.normal(0.0, 0.08)
    ndvi_base = float(np.clip(ndvi_base, -0.10, 0.85))
    lst_base = 295.0 - 0.55 * abs(lat_deg) + rng.normal(0.0, 3.0)
    lst_base = float(np.clip(lst_base, 230.0, 330.0))
    pm_base = float(np.clip(np.exp(rng.normal(2.3, 0.8)), 1.0, 80.0))

    # AR(1) drift on each baseline so trend signal exists.
    ar = 0.95
    series = np.zeros((length, N_BANDS), dtype=np.float32)
    ndvi_drift = ndvi_base
    lst_drift = lst_base
    pm_drift = pm_base
    start_month = int(rng.integers(0, 12))
    for i in range(length):
        m = (start_month + i) % 12
        # Small AR(1) noise to drift baselines (climate trend).
        ndvi_drift = ar * ndvi_drift + (1 - ar) * ndvi_base + rng.normal(0.0, 0.015)
        lst_drift = ar * lst_drift + (1 - ar) * lst_base + rng.normal(0.0, 0.5)
        pm_drift = ar * pm_drift + (1 - ar) * pm_base + rng.normal(0.0, 0.5)
        series[i, 0] = _ndvi_seasonal(lat_deg, m + 1, ndvi_drift) + rng.normal(0.0, 0.04)
        series[i, 1] = _lst_day_seasonal(lat_deg, m + 1, lst_drift) + rng.normal(0.0, 1.0)
        series[i, 2] = _lst_night_seasonal(lat_deg, m + 1, lst_drift) + rng.normal(0.0, 1.0)
        series[i, 3] = _pm25_seasonal(lat_deg, m + 1, pm_drift) + rng.normal(0.0, 0.5)
    # Clamp to physical ranges.
    series[:, 0] = np.clip(series[:, 0], *BAND_PHYSICAL_RANGES["indices.ndvi"])
    series[:, 1] = np.clip(series[:, 1], *BAND_PHYSICAL_RANGES["modis.lst_day_8day"])
    series[:, 2] = np.clip(series[:, 2], *BAND_PHYSICAL_RANGES["modis.lst_night_8day"])
    series[:, 3] = np.clip(series[:, 3], *BAND_PHYSICAL_RANGES["cams.pm25"])
    return series, start_month


def _build_context(lat_deg: float, lng_deg: float, month_idx: int) -> np.ndarray:
    """5-D context vector for one sample."""
    lat_norm = lat_deg / 90.0
    lng_sin = math.sin(math.radians(lng_deg))
    lng_cos = math.cos(math.radians(lng_deg))
    m_rad = 2 * math.pi * month_idx / 12.0
    month_sin = math.sin(m_rad)
    month_cos = math.cos(m_rad)
    return np.asarray([lat_norm, lng_sin, lng_cos, month_sin, month_cos], dtype=np.float32)


def _normalise_lags(lags_physical: np.ndarray) -> np.ndarray:
    """[K, 4] in physical → [K, 4] z-scored to a stationary distribution."""
    out = np.empty_like(lags_physical)
    for j, key in enumerate(BAND_KEYS):
        mean, std = BAND_NORM[key]
        out[:, j] = (lags_physical[:, j] - mean) / std
    return out


def _normalise_target(target_physical: np.ndarray) -> np.ndarray:
    """[4] physical → [4] z-scored. Inverse applied at inference time
    on the Python side; the ONNX model itself outputs normalised
    predictions + log-variances."""
    out = np.empty_like(target_physical)
    for j, key in enumerate(BAND_KEYS):
        mean, std = BAND_NORM[key]
        out[j] = (target_physical[j] - mean) / std
    return out


def _denormalise_pred(pred_norm: np.ndarray) -> np.ndarray:
    out = np.empty_like(pred_norm)
    for j, key in enumerate(BAND_KEYS):
        mean, std = BAND_NORM[key]
        out[..., j] = pred_norm[..., j] * std + mean
    return out


def generate_synthetic_pairs(
    n_series: int, rng: np.random.Generator
) -> Tuple[np.ndarray, np.ndarray, np.ndarray, np.ndarray]:
    """Generate `n_series` independent monthly time series, sample one
    (K_LAGS → 1) window per series. Returns (lags, contexts, targets,
    target_months) where:
        lags: [N, K_LAGS, N_BANDS]   normalised
        ctx : [N, N_CONTEXT]          (lat_norm, lng_sin, lng_cos, ms, mc)
        tgt : [N, N_BANDS]            normalised
        tgt_month: [N]                int 0..11 (for diagnostics)
    """
    series_len = K_LAGS + 6     # +6 random offset budget
    lags_out = np.zeros((n_series, K_LAGS, N_BANDS), dtype=np.float32)
    ctx_out = np.zeros((n_series, N_CONTEXT), dtype=np.float32)
    tgt_out = np.zeros((n_series, N_BANDS), dtype=np.float32)
    tgt_month_out = np.zeros((n_series,), dtype=np.int32)
    for i in range(n_series):
        lat = float(rng.uniform(-70.0, 70.0))    # avoid polar extremes
        lng = float(rng.uniform(-180.0, 180.0))
        series, start_month = _gen_series(lat, series_len, rng)
        # Pick the offset so the target index is in-range.
        offset = int(rng.integers(0, series_len - K_LAGS))
        lags_phys = series[offset : offset + K_LAGS]
        target_phys = series[offset + K_LAGS - 1 + 1]    # next step
        target_month = (start_month + offset + K_LAGS) % 12
        lags_out[i] = _normalise_lags(lags_phys)
        ctx_out[i] = _build_context(lat, lng, target_month)
        tgt_out[i] = _normalise_target(target_phys)
        tgt_month_out[i] = target_month
    return lags_out, ctx_out, tgt_out, tgt_month_out


# ─── Real-data harvester ────────────────────────────────────────────
def harvest_real_ndvi(
    audit_dump: Path, min_depth: int = 4
) -> List[Dict]:
    """Read the audit CSV `indices_ndvi.csv` and return all (cell, lat,
    lng, monthly_series) tuples that have ≥ `min_depth` observations.
    These are used for the held-out 'real' validation slice.

    Each returned dict carries `series_phys`: a list of (tslot, ndvi)
    pairs sorted by tslot. The other three bands are filled with
    synthetic values calibrated to lat — honest because we don't have
    real LST/PM25 history on this responder.
    """
    csv_path = audit_dump / "indices_ndvi.csv"
    if not csv_path.exists():
        print(f"[harvest_real_ndvi] {csv_path} missing; skip real slice")
        return []
    # Per-cell lat/lng is decoded later (in evaluate_on_real_ndvi) via the
    # pure-Python latlng_from_cell64 ported from crates/emem-codec/src/geo.rs
    # — no external Rust binding needed (audit 2026-05-30 fix #5).
    rows = []
    with csv_path.open() as fh:
        for r in csv.DictReader(fh):
            try:
                rows.append({
                    "cell": r["cell"],
                    "tslot": int(r["tslot"]),
                    "ndvi": float(r["value"]),
                })
            except (KeyError, ValueError):
                continue
    by_cell: Dict[str, List[Tuple[int, float]]] = {}
    for r in rows:
        by_cell.setdefault(r["cell"], []).append((r["tslot"], r["ndvi"]))
    deep = [(c, sorted(set(t))) for c, t in by_cell.items() if len({t for t, _ in t}) >= min_depth]
    print(f"[harvest_real_ndvi] {len(by_cell)} cells, {len(deep)} with >= {min_depth} tslots")
    return [{"cell": c, "obs": sorted(by_cell[c])} for c, _ in deep]


# ─── Model ──────────────────────────────────────────────────────────
class DynamicsV2(nn.Module):
    """Multi-band scalar dynamics head.

    Input: flat tensor `[B, INPUT_DIM]` laid out as
        lags[0..N_BANDS] @ K_LAGS  ‖  ctx[0..N_CONTEXT]

    The lag block is reshaped to `[B, K_LAGS, N_BANDS]`, projected to
    `D_MODEL`, augmented with a learned [CLS] embedding, fed through
    `N_LAYERS` of TransformerEncoder, and the CLS slot is read out as
    the per-step summary. The CLS embedding is concatenated with the
    context vector and projected to `2 * N_BANDS` outputs — first
    half is the next-step mean, second half is the log-variance.

    The ONNX export keeps the wire flat to avoid having to expose the
    reshape in the protocol — the Rust side just passes a 29-vector.
    """

    def __init__(self) -> None:
        super().__init__()
        self.band_proj = nn.Linear(N_BANDS, D_MODEL)
        # Learned positional embedding for lag index 0..K_LAGS-1.
        self.lag_pos = nn.Parameter(torch.zeros(K_LAGS, D_MODEL))
        # Learned [CLS] token.
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
        # Head: mean and log-var, each N_BANDS.
        self.head = nn.Sequential(
            nn.Linear(2 * D_MODEL, D_MODEL),
            nn.GELU(),
            nn.Linear(D_MODEL, 2 * N_BANDS),
        )

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        """Forward.

        Args:
            x: `[B, INPUT_DIM]` flat input
                lags(K_LAGS*N_BANDS) ‖ context(N_CONTEXT).

        Returns:
            `[B, 2 * N_BANDS]` — first N_BANDS are normalised mean,
            second N_BANDS are log-variances.
        """
        B = x.shape[0]
        lags = x[:, : K_LAGS * N_BANDS].reshape(B, K_LAGS, N_BANDS)
        ctx = x[:, K_LAGS * N_BANDS :]
        h = self.band_proj(lags) + self.lag_pos.unsqueeze(0)
        cls = self.cls.expand(B, -1).unsqueeze(1)             # [B, 1, D]
        h = torch.cat([cls, h], dim=1)                         # [B, K+1, D]
        h = self.encoder(h)
        cls_out = h[:, 0, :]                                   # [B, D]
        ctx_h = F.gelu(self.ctx_proj(ctx))
        combined = torch.cat([cls_out, ctx_h], dim=-1)         # [B, 2D]
        return self.head(combined)                             # [B, 2*N_BANDS]


def gaussian_nll(pred: torch.Tensor, target: torch.Tensor) -> torch.Tensor:
    """Mean negative log-likelihood under N(mean, exp(log_var)).
    `pred` is `[B, 2*N_BANDS]`."""
    mean, log_var = pred.chunk(2, dim=-1)
    # Stability: clamp log_var to a sane range.
    log_var = log_var.clamp(min=-6.0, max=4.0)
    inv_var = torch.exp(-log_var)
    sq = (target - mean) ** 2
    return 0.5 * (log_var + sq * inv_var).mean()


def identity_baseline_pred(lags_norm: torch.Tensor) -> torch.Tensor:
    """Baseline: predict next step = last lag's bands. Returns
    `[B, 2*N_BANDS]` so it can be evaluated with the same loss."""
    B = lags_norm.shape[0]
    lags = lags_norm[:, : K_LAGS * N_BANDS].reshape(B, K_LAGS, N_BANDS)
    last = lags[:, -1, :]
    mean = last
    log_var = torch.zeros_like(last)
    return torch.cat([mean, log_var], dim=-1)


def mse_in_physical_units(pred_norm: torch.Tensor, target_norm: torch.Tensor) -> Dict[str, float]:
    """Per-band MSE after denormalising to physical units. Returns a
    dict keyed by band."""
    pred_phys = _denormalise_pred(pred_norm.detach().cpu().numpy())
    tgt_phys = _denormalise_pred(target_norm.detach().cpu().numpy())
    out: Dict[str, float] = {}
    for j, key in enumerate(BAND_KEYS):
        mse = float(np.mean((pred_phys[..., j] - tgt_phys[..., j]) ** 2))
        out[key] = mse
    return out


# ─── Training loop ──────────────────────────────────────────────────
def train(
    *, audit_dump: Path, out_dir: Path, verbose: bool = True
) -> Dict:
    """Train the model + export to ONNX. Returns a dict of stats that
    gets serialised into `dynamics_v2.metadata.json`."""
    out_dir.mkdir(parents=True, exist_ok=True)
    device = torch.device("cpu")    # acceptance criteria: <30 min on CPU
    rng = np.random.default_rng(SEED)
    if verbose:
        print(f"[train] generating {N_TRAIN_SYNTHETIC} synthetic train + {N_VAL_SYNTHETIC} val series")
    train_x_lags, train_x_ctx, train_y, _ = generate_synthetic_pairs(N_TRAIN_SYNTHETIC, rng)
    val_x_lags, val_x_ctx, val_y, _ = generate_synthetic_pairs(N_VAL_SYNTHETIC, rng)
    # Held-out 'real' slice (NDVI cells with ≥4 tslots from the audit).
    # We treat the model's per-band output but evaluate ONLY the NDVI
    # component, since the other three bands are synthetically filled.
    real_eval = harvest_real_ndvi(audit_dump, min_depth=4)
    if verbose:
        print(f"[train] real-NDVI eval cells: {len(real_eval)}")

    def to_flat(x_lags: np.ndarray, x_ctx: np.ndarray) -> torch.Tensor:
        flat = np.concatenate([x_lags.reshape(x_lags.shape[0], -1), x_ctx], axis=-1)
        return torch.from_numpy(flat).float()

    train_x = to_flat(train_x_lags, train_x_ctx)
    train_y_t = torch.from_numpy(train_y).float()
    val_x = to_flat(val_x_lags, val_x_ctx)
    val_y_t = torch.from_numpy(val_y).float()
    if verbose:
        print(f"[train] train_x={tuple(train_x.shape)} val_x={tuple(val_x.shape)}")

    model = DynamicsV2().to(device)
    n_params = sum(p.numel() for p in model.parameters())
    if verbose:
        print(f"[train] model param count = {n_params}")
    opt = torch.optim.AdamW(model.parameters(), lr=LR, weight_decay=WEIGHT_DECAY)
    sched = torch.optim.lr_scheduler.CosineAnnealingLR(opt, T_max=N_EPOCHS)

    history: List[Dict] = []
    t0 = time.time()
    for epoch in range(N_EPOCHS):
        model.train()
        idx = torch.randperm(train_x.shape[0])
        losses = []
        for s in range(0, train_x.shape[0], BATCH_SIZE):
            b = idx[s : s + BATCH_SIZE]
            xb = train_x[b].to(device)
            yb = train_y_t[b].to(device)
            pred = model(xb)
            loss = gaussian_nll(pred, yb)
            opt.zero_grad()
            loss.backward()
            torch.nn.utils.clip_grad_norm_(model.parameters(), max_norm=1.0)
            opt.step()
            losses.append(float(loss.item()))
        sched.step()
        train_loss = float(np.mean(losses))

        # Validation
        model.eval()
        with torch.no_grad():
            val_pred = model(val_x.to(device))
            val_loss = float(gaussian_nll(val_pred, val_y_t.to(device)).item())
            val_pred_mean = val_pred[:, :N_BANDS]
            baseline_pred = identity_baseline_pred(val_x.to(device))
            baseline_loss = float(gaussian_nll(baseline_pred, val_y_t.to(device)).item())
            mse_model = mse_in_physical_units(val_pred_mean, val_y_t.to(device))
            mse_baseline = mse_in_physical_units(
                baseline_pred[:, :N_BANDS], val_y_t.to(device)
            )
        history.append(
            {"epoch": epoch, "train_loss": train_loss, "val_loss": val_loss,
             "baseline_loss": baseline_loss, "val_mse_phys": mse_model,
             "baseline_mse_phys": mse_baseline}
        )
        if verbose:
            print(
                f"  epoch {epoch:>3}  train_nll={train_loss:.4f}  val_nll={val_loss:.4f}"
                f"  baseline_val_nll={baseline_loss:.4f}"
                f"  ndvi_mse_phys={mse_model['indices.ndvi']:.5f}"
                f"  (baseline {mse_baseline['indices.ndvi']:.5f})"
            )
    dt = time.time() - t0
    if verbose:
        print(f"[train] training time: {dt:.1f}s")

    # ── Real-NDVI held-out eval ─────────────────────────────────────
    real_mse_model = None
    real_mse_baseline = None
    n_real_pairs = 0
    if real_eval:
        try:
            real_mse_model, real_mse_baseline, n_real_pairs = evaluate_on_real_ndvi(
                model, real_eval, verbose=verbose
            )
            if verbose:
                print(
                    f"[train] real-NDVI eval: n_pairs={n_real_pairs}  "
                    f"model_mse={real_mse_model:.5f}  baseline_mse={real_mse_baseline:.5f}"
                )
        except Exception as e:
            print(f"[train] real-NDVI eval failed: {e}")

    # ── ONNX export ─────────────────────────────────────────────────
    onnx_path = out_dir / "dynamics_v2.onnx"
    state_dict_path = out_dir / "dynamics_v2.state_dict.pt"
    torch.save(model.state_dict(), state_dict_path)
    model.eval()
    dummy = torch.zeros(1, INPUT_DIM, dtype=torch.float32)
    torch.onnx.export(
        model,
        (dummy,),
        str(onnx_path),
        input_names=["input"],
        output_names=["output"],
        dynamic_axes={"input": {0: "batch"}, "output": {0: "batch"}},
        opset_version=17,
        dynamo=False,
    )
    onnx_bytes = onnx_path.read_bytes()
    artifact_blake2b = hashlib.blake2b(onnx_bytes, digest_size=32).hexdigest()
    ckpt_blake2b = hashlib.blake2b(state_dict_path.read_bytes(), digest_size=32).hexdigest()

    # ── Sanity: ONNX agrees with PyTorch within 1e-4 ───────────────
    try:
        import onnxruntime as ort
        sess = ort.InferenceSession(str(onnx_path), providers=["CPUExecutionProvider"])
        with torch.no_grad():
            torch_out = model(val_x[:16]).numpy()
        ort_out = sess.run(["output"], {"input": val_x[:16].numpy()})[0]
        max_diff = float(np.max(np.abs(torch_out - ort_out)))
        if verbose:
            print(f"[train] ONNX/PyTorch max abs diff on 16 val samples: {max_diff:.2e}")
    except Exception as e:
        print(f"[train] ONNX runtime parity check failed: {e}")
        max_diff = None

    last = history[-1]
    final_val_loss = last["val_loss"]
    final_baseline_loss = last["baseline_loss"]
    # % improvement of val NLL over identity baseline.
    if final_baseline_loss != 0:
        lift_pct = 100.0 * (final_baseline_loss - final_val_loss) / abs(final_baseline_loss)
    else:
        lift_pct = float("nan")
    # NDVI MSE % improvement (most-trusted band).
    base_ndvi_mse = last["baseline_mse_phys"]["indices.ndvi"]
    model_ndvi_mse = last["val_mse_phys"]["indices.ndvi"]
    if base_ndvi_mse > 0:
        ndvi_mse_lift_pct = 100.0 * (base_ndvi_mse - model_ndvi_mse) / base_ndvi_mse
    else:
        ndvi_mse_lift_pct = float("nan")

    # Skill score vs the predict-last-value (persistence / Markov-1)
    # baseline on the held-out REAL NDVI cells (audit 2026-05-30 HIGH #5).
    # skill = 1 - MSE_model / MSE_baseline. skill > 0 ⇔ the model beats
    # persistence; skill ≤ 0 ⇔ persistence is as good or better. This is
    # the single most honest number for whether the trained head adds
    # anything over "tomorrow looks like today" on real ground truth.
    real_ndvi_skill_score = compute_skill_score(real_mse_model, real_mse_baseline)
    beats_persistence_on_real_ndvi = (
        real_ndvi_skill_score is not None and real_ndvi_skill_score > 0.0
    )

    return {
        "history": history,
        "training_time_s": dt,
        "n_params": n_params,
        "onnx_path": str(onnx_path),
        "state_dict_path": str(state_dict_path),
        "artifact_blake2b": artifact_blake2b,
        "checkpoint_blake2b": ckpt_blake2b,
        "onnx_size_bytes": len(onnx_bytes),
        "final_train_loss": last["train_loss"],
        "final_val_loss": final_val_loss,
        "identity_baseline_val_loss": final_baseline_loss,
        "val_lift_pct_over_identity_baseline_nll": lift_pct,
        "val_lift_pct_over_identity_baseline_ndvi_mse": ndvi_mse_lift_pct,
        "val_mse_phys": last["val_mse_phys"],
        "baseline_mse_phys": last["baseline_mse_phys"],
        "real_ndvi_mse_model": real_mse_model,
        "real_ndvi_mse_baseline": real_mse_baseline,
        "real_ndvi_skill_score": real_ndvi_skill_score,
        "beats_persistence_on_real_ndvi": beats_persistence_on_real_ndvi,
        "n_real_ndvi_eval_pairs": n_real_pairs,
        "onnx_vs_pytorch_max_diff": max_diff,
        "train_size": N_TRAIN_SYNTHETIC,
        "val_size": N_VAL_SYNTHETIC,
        "synthetic_fraction_train": 1.0,
        "synthetic_fraction_val": 1.0,
    }


def evaluate_on_real_ndvi(
    model: DynamicsV2, real_eval: List[Dict], verbose: bool = False
) -> Tuple[float, float, int]:
    """For each real NDVI cell with depth ≥ K_LAGS+1, slide a window
    and predict; return (model_mse, baseline_mse, n_pairs) in physical
    NDVI units. The other three bands are filled with NaN-safe
    placeholders since the model only needs the NDVI column for this
    eval — we still feed normalised stand-ins so the model receives a
    coherent input.

    Audit 2026-05-30 fix #5: the per-cell lat/lng is now decoded in
    Python from the cell64 string (latlng_from_cell64 above), so the
    spatial-context vector reflects the cell's REAL position instead of
    biasing every cell to lat=0/lng=0 (Null Island). The number of cells
    that still fell back to (0,0) — because their cell key isn't a
    geo-aperture cell64 (e.g. a legacy 305 m string) — is logged so the
    reported skill can't be silently inflated by a hidden (0,0) bias."""
    model.eval()
    pairs = 0
    err_model = 0.0
    err_baseline = 0.0
    cells_decoded = 0
    cells_fell_back = 0
    for cell in real_eval:
        obs = cell["obs"]
        # Decode the cell's real (lat, lng) from its cell64 key. Fall back
        # to (0, 0) only when the key isn't a decodable geo cell, and count
        # those so the (0,0) bias is visible rather than hidden.
        latlng = latlng_from_cell64(str(cell.get("cell", "")))
        if latlng is None:
            cell_lat, cell_lng = 0.0, 0.0
            cells_fell_back += 1
        else:
            cell_lat, cell_lng = latlng
            cells_decoded += 1
        # `obs` is list of (tslot, ndvi). Sort by tslot.
        obs = sorted(obs, key=lambda x: x[0])
        # Take only NDVI observations, build a "monthly" series by
        # binning to nearest 30-day bucket and averaging.
        # Daily tslots (Tempo::fast in the band registry) → bucket by
        # 30 days.
        by_month: Dict[int, List[float]] = {}
        for t, v in obs:
            mb = t // 30
            by_month.setdefault(int(mb), []).append(float(v))
        monthly = [(mb, float(np.mean(vs))) for mb, vs in sorted(by_month.items())]
        if len(monthly) < K_LAGS + 1:
            continue
        # Sliding windows.
        for i in range(K_LAGS, len(monthly)):
            ndvi_lags = np.asarray([v for _, v in monthly[i - K_LAGS : i]], dtype=np.float32)
            target = monthly[i][1]
            # Fill other bands with their climatological means + small noise,
            # using the cell's real latitude for the LST seasonal shape.
            lst_day = np.asarray([_lst_day_seasonal(cell_lat, (mb % 12) + 1, 290.0) for mb, _ in monthly[i - K_LAGS : i]], dtype=np.float32)
            lst_night = lst_day - 8.0
            pm = np.full(K_LAGS, BAND_NORM["cams.pm25"][0], dtype=np.float32)
            lags_phys = np.stack([ndvi_lags, lst_day, lst_night, pm], axis=-1)
            lags_norm = _normalise_lags(lags_phys)
            ctx = _build_context(cell_lat, cell_lng, monthly[i][0] % 12)
            flat = np.concatenate([lags_norm.reshape(-1), ctx]).astype(np.float32)
            with torch.no_grad():
                pred = model(torch.from_numpy(flat).unsqueeze(0))[0]
            pred_phys = _denormalise_pred(pred[:N_BANDS].numpy())
            baseline_phys = ndvi_lags[-1]
            err_model += (pred_phys[0] - target) ** 2
            err_baseline += (baseline_phys - target) ** 2
            pairs += 1
    if verbose:
        print(
            f"[train] real-NDVI eval cell positions: {cells_decoded} decoded "
            f"from cell64, {cells_fell_back} fell back to (0,0) (non-geo cell key)"
        )
    if pairs == 0:
        return 0.0, 0.0, 0
    return float(err_model / pairs), float(err_baseline / pairs), int(pairs)


def write_metadata(out_dir: Path, stats: Dict) -> None:
    meta = {
        "model_id": "jepa_temporal_predictor@2.scalar",
        "version": "0.0.1-trained-multi-band-scalar",
        "trained_at_unix": int(time.time()),
        "trained_at_iso": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "architecture": {
            "kind": "transformer_encoder",
            "input_lags": K_LAGS,
            "n_bands": N_BANDS,
            "band_keys": list(BAND_KEYS),
            "n_context": N_CONTEXT,
            "context_keys": [
                "lat_norm",
                "lng_sin",
                "lng_cos",
                "month_sin",
                "month_cos",
            ],
            "input_dim": INPUT_DIM,
            "output_dim": 2 * N_BANDS,
            "d_model": D_MODEL,
            "n_heads": N_HEADS,
            "n_layers": N_LAYERS,
            "dropout": DROPOUT,
            "n_parameters": stats["n_params"],
        },
        "normalisation": {
            "band_norm": {k: list(v) for k, v in BAND_NORM.items()},
            "physical_ranges": {k: list(v) for k, v in BAND_PHYSICAL_RANGES.items()},
            "note": "model outputs z-scored predictions; Rust path denormalises with band_norm at inference time and clamps to physical_ranges before signing the receipt",
        },
        "training": {
            "trained": True,
            "training_data_size": stats["train_size"],
            "val_data_size": stats["val_size"],
            "synthetic_fraction_train": stats["synthetic_fraction_train"],
            "synthetic_fraction_val": stats["synthetic_fraction_val"],
            "real_ndvi_eval_pairs": stats["n_real_ndvi_eval_pairs"],
            "n_epochs": N_EPOCHS,
            "batch_size": BATCH_SIZE,
            "lr": LR,
            "weight_decay": WEIGHT_DECAY,
            "loss": "gaussian_negative_log_likelihood",
            "training_time_s": stats["training_time_s"],
            "training_date_iso": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
            "train_loss": stats["final_train_loss"],
            "val_loss": stats["final_val_loss"],
            "identity_baseline_val_loss": stats["identity_baseline_val_loss"],
            "val_lift_pct_over_identity_baseline_nll": stats["val_lift_pct_over_identity_baseline_nll"],
            "val_lift_pct_over_identity_baseline_ndvi_mse": stats["val_lift_pct_over_identity_baseline_ndvi_mse"],
            "val_mse_physical_units": stats["val_mse_phys"],
            "baseline_mse_physical_units": stats["baseline_mse_phys"],
            "real_ndvi_mse_model": stats["real_ndvi_mse_model"],
            "real_ndvi_mse_baseline": stats["real_ndvi_mse_baseline"],
            # Skill vs predict-last-value (persistence) on real NDVI. >0
            # ⇔ beats persistence; ≤0 ⇔ persistence wins (audit 2026-05-30
            # HIGH #5). The sidecar surfaces this in the dynamics response.
            "real_ndvi_skill_score": stats["real_ndvi_skill_score"],
            "beats_persistence_on_real_ndvi": stats["beats_persistence_on_real_ndvi"],
            "skill_reference": "predict-last-value (persistence / Markov-1)",
            "checkpoint_blake2b_hex": stats["checkpoint_blake2b"],
            "honesty_caveats": [
                "training_set_predominantly_synthetic: real corpus history is "
                "currently too sparse on this responder to train end-to-end. "
                "NDVI has ~96 cells with >=4 monthly tslots; LST/PM25/weather "
                "have ~1 tslot per cell. The model is trained on a "
                "physically-motivated seasonality generator (see "
                "train_dynamics_v2.py generate_synthetic_pairs()) and held-out-"
                "validated against the real NDVI cells. Synthetic_fraction "
                "is reported so verifiers see how much real ground truth "
                "anchors the model.",
                "single_step_horizon: this is a one-step-ahead predictor at "
                "monthly cadence. Multi-step horizons MUST be unrolled "
                "auto-regressively at the caller; the receipt does not "
                "implicitly compose multiple steps.",
            ],
        },
        "validation": {
            "by_construction_cosine": None,
            "val_loss": stats["final_val_loss"],
            "identity_baseline_val_loss": stats["identity_baseline_val_loss"],
            "interpretation": "Lower val_loss = better. Identity baseline is the 'predict last lag' Markov-1 predictor — beating it means the model captured at least the seasonal cycle.",
        },
        "artifact": {
            "filename": "dynamics_v2.onnx",
            "size_bytes": stats["onnx_size_bytes"],
            "blake2b_hex": stats["artifact_blake2b"],
            "onnx_opset": 17,
            "onnx_vs_pytorch_max_diff": stats["onnx_vs_pytorch_max_diff"],
        },
        "wire": {
            "input": {
                "shape": [INPUT_DIM],
                "name": "input",
                "dtype": "float32",
                "layout": "lags[K=6, B=4] (flattened row-major) || context[5]",
            },
            "output": {
                "shape": [2 * N_BANDS],
                "name": "output",
                "dtype": "float32",
                "layout": "predictions_normalised[4] || log_variance[4]",
            },
            "preprocessing": "caller normalises each lag scalar with band_norm[band] = (lag - mean) / std, then concatenates with context[5]",
            "postprocessing": "caller denormalises mean[band] = output[band] * std + mean and clamps to physical_ranges; confidence[band] = exp(-output[N_BANDS+band] / 2)",
        },
    }
    meta_path = out_dir / "dynamics_v2.metadata.json"
    meta_path.write_text(json.dumps(meta, indent=2) + "\n")
    print(f"[train] wrote metadata to {meta_path}")


# ════════════════════════════════════════════════════════════════════
# NDVI-ONLY mode (audit 2026-05-30): train a single-band dynamics head
# on the REAL corpus and gate purely on beating persistence on held-out
# REAL NDVI.
#
# WHY a separate path: the 4-band joint model above can't train on real
# data (LST/PM25/weather are ~1 tslot/cell on this responder). NDVI is
# the only band with temporal depth. The refreshed 2026-05-30 audit dump
# shows the deep cells are *annual* peak-NDVI series (median inter-obs
# gap ≈ 365 days; the 798-cell spike is one July observation per year,
# 2020..2026) — NOT monthly. So this path:
#   • models n_bands=1 (NDVI), K configurable (default 3 admits the most
#     cells; 4 is the conservative middle),
#   • treats each cell's sorted (tslot, ndvi) series as the lag window
#     directly (annual cadence; we do NOT fabricate monthly structure),
#   • uses a grouped K-fold over CELLS so no cell leaks between the
#     real-train and the real-eval slice (hold-cells-out CV),
#   • computes skill = 1 - MSE_model / MSE_persistence ONLY on held-out
#     REAL NDVI windows. Synthetic series are optional augmentation in
#     the train split; they never enter the eval.
#
# Persistence = predict-last-value. On annual peak-NDVI this is a very
# strong baseline (summer greenness is stable year-to-year), so a
# negative skill here is the *expected* honest outcome unless the model
# genuinely learns cell-specific trend/recovery dynamics.
# ════════════════════════════════════════════════════════════════════

NDVI_NORM = BAND_NORM["indices.ndvi"]            # (mean, std) = (0.30, 0.30)
NDVI_RANGE = BAND_PHYSICAL_RANGES["indices.ndvi"]  # (-1.0, 1.0)


def _ndvi_norm(v: np.ndarray) -> np.ndarray:
    return (v - NDVI_NORM[0]) / NDVI_NORM[1]


def _ndvi_denorm(v: np.ndarray) -> np.ndarray:
    return v * NDVI_NORM[1] + NDVI_NORM[0]


class NdviDynamics(nn.Module):
    """Single-band (NDVI) one-step-ahead dynamics head.

    Same Transformer backbone as `DynamicsV2` but with `n_bands=1` and a
    configurable lag count `k`. Input is flat `[B, k + N_CONTEXT]`:
    normalised NDVI lags ‖ (lat_norm, lng_sin, lng_cos, month_sin,
    month_cos). Output is `[B, 2]` = (mean, log_var) for next-step NDVI
    (z-scored). Parameter count ≈ 10–12k.
    """

    def __init__(self, k: int, d_model: int = D_MODEL) -> None:
        super().__init__()
        self.k = k
        self.band_proj = nn.Linear(1, d_model)
        self.lag_pos = nn.Parameter(torch.zeros(k, d_model))
        self.cls = nn.Parameter(torch.zeros(1, d_model))
        nn.init.normal_(self.lag_pos, std=0.02)
        nn.init.normal_(self.cls, std=0.02)
        encoder_layer = nn.TransformerEncoderLayer(
            d_model=d_model,
            nhead=N_HEADS,
            dim_feedforward=4 * d_model,
            dropout=DROPOUT,
            batch_first=True,
            norm_first=True,
            activation="gelu",
        )
        self.encoder = nn.TransformerEncoder(encoder_layer, num_layers=N_LAYERS)
        self.ctx_proj = nn.Linear(N_CONTEXT, d_model)
        self.head = nn.Sequential(
            nn.Linear(2 * d_model, d_model),
            nn.GELU(),
            nn.Linear(d_model, 2),
        )

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        B = x.shape[0]
        lags = x[:, : self.k].reshape(B, self.k, 1)
        ctx = x[:, self.k :]
        h = self.band_proj(lags) + self.lag_pos.unsqueeze(0)
        cls = self.cls.expand(B, -1).unsqueeze(1)
        h = torch.cat([cls, h], dim=1)
        h = self.encoder(h)
        cls_out = h[:, 0, :]
        ctx_h = F.gelu(self.ctx_proj(ctx))
        combined = torch.cat([cls_out, ctx_h], dim=-1)
        return self.head(combined)


def _ndvi_gaussian_nll(pred: torch.Tensor, target: torch.Tensor) -> torch.Tensor:
    mean, log_var = pred[:, 0:1], pred[:, 1:2]
    log_var = log_var.clamp(min=-6.0, max=4.0)
    inv_var = torch.exp(-log_var)
    return 0.5 * (log_var + (target - mean) ** 2 * inv_var).mean()


def harvest_real_ndvi_windows(
    audit_dump: Path, k: int, *, drop_zero_tslot: bool = True
) -> List[Dict]:
    """Build real NDVI lag→target windows from the audit dump.

    Each cell's observations are sorted by tslot (de-duplicated, keeping
    the mean value per tslot). A cell with N distinct tslots yields N-k
    sliding windows of (k lags → 1 target). The cell's real (lat, lng)
    is decoded from its cell64 key; cells whose key is not a decodable
    geo cell fall back to (0, 0) and are counted.

    Returns a list of per-cell dicts:
        {"cell", "lat", "lng", "fell_back", "windows": [(lags[k], target)]}
    in PHYSICAL NDVI units (caller normalises).
    """
    csv_path = audit_dump / "indices_ndvi.csv"
    if not csv_path.exists():
        print(f"[ndvi] {csv_path} missing; no real windows")
        return []
    by_cell: Dict[str, Dict[int, List[float]]] = {}
    with csv_path.open() as fh:
        for r in csv.DictReader(fh):
            try:
                cell = r["cell"]
                t = int(r["tslot"])
                v = float(r["value"])
            except (KeyError, ValueError):
                continue
            if drop_zero_tslot and t == 0:
                continue
            if not math.isfinite(v):
                continue
            by_cell.setdefault(cell, {}).setdefault(t, []).append(v)
    out: List[Dict] = []
    n_fell_back = 0
    for cell, tmap in by_cell.items():
        series = [(t, float(np.mean(vs))) for t, vs in sorted(tmap.items())]
        if len(series) < k + 1:
            continue
        latlng = latlng_from_cell64(cell)
        if latlng is None:
            lat, lng = 0.0, 0.0
            fell_back = True
            n_fell_back += 1
        else:
            lat, lng = latlng
            fell_back = False
        windows: List[Tuple[np.ndarray, float, int]] = []
        for i in range(k, len(series)):
            lags = np.asarray([v for _, v in series[i - k : i]], dtype=np.float32)
            target = float(series[i][1])
            # The cell's observations are ~annual and clustered in the
            # growing-season month; use a coarse month-of-year proxy from
            # the tslot (Unix-epoch days) so the calendar context isn't
            # constant. (t % 365) / 365 * 12.
            target_month = int(((series[i][0] % 365) / 365.0) * 12) % 12
            windows.append((lags, target, target_month))
        out.append(
            {"cell": cell, "lat": lat, "lng": lng, "fell_back": fell_back, "windows": windows}
        )
    n_windows = sum(len(c["windows"]) for c in out)
    print(
        f"[ndvi] real windows (k={k}): {len(out)} cells, {n_windows} windows, "
        f"{n_fell_back} cells fell back to (0,0)"
    )
    return out


def _windows_to_tensors(
    cells: List[Dict], k: int
) -> Tuple[torch.Tensor, torch.Tensor, np.ndarray]:
    """Flatten per-cell windows to (X[N, k+N_CONTEXT], y[N,1], persistence[N])."""
    xs, ys, persist = [], [], []
    for c in cells:
        for lags, target, month in c["windows"]:
            lags_norm = _ndvi_norm(lags)
            ctx = _build_context(c["lat"], c["lng"], month)
            xs.append(np.concatenate([lags_norm, ctx]).astype(np.float32))
            ys.append([float(_ndvi_norm(np.asarray(target))[()])])
            persist.append(float(lags[-1]))  # physical predict-last-value
    if not xs:
        return (
            torch.zeros((0, k + N_CONTEXT), dtype=torch.float32),
            torch.zeros((0, 1), dtype=torch.float32),
            np.zeros((0,), dtype=np.float32),
        )
    return (
        torch.from_numpy(np.stack(xs)).float(),
        torch.from_numpy(np.asarray(ys, dtype=np.float32)),
        np.asarray(persist, dtype=np.float32),
    )


def _gen_synthetic_ndvi_windows(
    n_series: int, k: int, rng: np.random.Generator
) -> List[Dict]:
    """Annual-cadence synthetic NDVI windows for optional augmentation.

    Each series samples one observation per year in a fixed growing-
    season month (matching the real corpus, which is peak-season annual),
    with AR(1) baseline drift + observation noise. Returns the same
    per-"cell" dict shape as `harvest_real_ndvi_windows` so the two can
    be concatenated for the train split.
    """
    out: List[Dict] = []
    for _ in range(n_series):
        lat = float(rng.uniform(-70.0, 70.0))
        lng = float(rng.uniform(-180.0, 180.0))
        n_years = k + 1 + int(rng.integers(0, 3))
        peak_month = 7 if lat >= 0 else 1
        ndvi_base = float(np.clip(0.55 - 0.40 * (abs(lat) / 90.0) + rng.normal(0, 0.08), -0.1, 0.85))
        drift = ndvi_base
        vals = []
        for _y in range(n_years):
            drift = 0.9 * drift + 0.1 * ndvi_base + rng.normal(0.0, 0.02)
            v = _ndvi_seasonal(lat, peak_month, drift) + rng.normal(0.0, 0.03)
            vals.append(float(np.clip(v, *NDVI_RANGE)))
        windows = []
        for i in range(k, len(vals)):
            lags = np.asarray(vals[i - k : i], dtype=np.float32)
            windows.append((lags, float(vals[i]), peak_month % 12))
        out.append({"cell": "synthetic", "lat": lat, "lng": lng, "fell_back": False, "windows": windows})
    return out


def _train_one_ndvi(
    train_x: torch.Tensor,
    train_y: torch.Tensor,
    k: int,
    *,
    n_epochs: int,
    batch_size: int,
    seed: int = SEED,
) -> NdviDynamics:
    torch.manual_seed(seed)
    model = NdviDynamics(k)
    opt = torch.optim.AdamW(model.parameters(), lr=LR, weight_decay=WEIGHT_DECAY)
    sched = torch.optim.lr_scheduler.CosineAnnealingLR(opt, T_max=n_epochs)
    n = train_x.shape[0]
    for _epoch in range(n_epochs):
        model.train()
        idx = torch.randperm(n)
        for s in range(0, n, batch_size):
            b = idx[s : s + batch_size]
            pred = model(train_x[b])
            loss = _ndvi_gaussian_nll(pred, train_y[b])
            opt.zero_grad()
            loss.backward()
            torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0)
            opt.step()
        sched.step()
    return model


def _eval_ndvi_skill(
    model: NdviDynamics, eval_x: torch.Tensor, eval_y: torch.Tensor, persist: np.ndarray
) -> Tuple[float, float, int]:
    """Return (model_mse, persistence_mse, n) in PHYSICAL NDVI units."""
    if eval_x.shape[0] == 0:
        return 0.0, 0.0, 0
    model.eval()
    with torch.no_grad():
        pred_norm = model(eval_x)[:, 0].numpy()
    pred_phys = _ndvi_denorm(pred_norm)
    pred_phys = np.clip(pred_phys, *NDVI_RANGE)
    tgt_phys = _ndvi_denorm(eval_y[:, 0].numpy())
    mse_model = float(np.mean((pred_phys - tgt_phys) ** 2))
    mse_persist = float(np.mean((persist - tgt_phys) ** 2))
    return mse_model, mse_persist, int(eval_x.shape[0])


def train_ndvi_only(
    *,
    audit_dump: Path,
    out_dir: Path,
    k: int = 3,
    n_folds: int = 5,
    n_synthetic: int = 4000,
    use_synthetic: bool = True,
    n_epochs: int = 40,
    batch_size: int = 256,
    verbose: bool = True,
) -> Dict:
    """Hold-cells-out CV NDVI dynamics training. Gates on REAL skill."""
    out_dir.mkdir(parents=True, exist_ok=True)
    rng = np.random.default_rng(SEED)
    real_cells = harvest_real_ndvi_windows(audit_dump, k)
    if not real_cells:
        raise SystemExit("no real NDVI windows; refresh the audit dump first")
    n_real_windows = sum(len(c["windows"]) for c in real_cells)
    n_fell_back = sum(1 for c in real_cells if c["fell_back"])

    # Grouped K-fold over cells (deterministic shuffle).
    order = list(range(len(real_cells)))
    random.Random(SEED).shuffle(order)
    folds: List[List[int]] = [order[i::n_folds] for i in range(n_folds)]

    syn_cells = _gen_synthetic_ndvi_windows(n_synthetic, k, rng) if use_synthetic else []
    n_syn_windows = sum(len(c["windows"]) for c in syn_cells)

    fold_model_mse: List[float] = []
    fold_persist_mse: List[float] = []
    fold_n: List[int] = []
    agg_err_model = 0.0
    agg_err_persist = 0.0
    agg_n = 0
    for fi in range(n_folds):
        eval_idx = set(folds[fi])
        eval_cells = [real_cells[i] for i in eval_idx]
        train_real = [real_cells[i] for i in range(len(real_cells)) if i not in eval_idx]
        train_cells = train_real + syn_cells
        tx, ty, _ = _windows_to_tensors(train_cells, k)
        ex, ey, ep = _windows_to_tensors(eval_cells, k)
        if tx.shape[0] == 0 or ex.shape[0] == 0:
            continue
        model = _train_one_ndvi(
            tx, ty, k, n_epochs=n_epochs, batch_size=batch_size, seed=SEED + fi
        )
        m_mse, p_mse, n = _eval_ndvi_skill(model, ex, ey, ep)
        fold_model_mse.append(m_mse)
        fold_persist_mse.append(p_mse)
        fold_n.append(n)
        # Pooled (window-weighted) accumulation for the headline number.
        agg_err_model += m_mse * n
        agg_err_persist += p_mse * n
        agg_n += n
        if verbose:
            sk = compute_skill_score(m_mse, p_mse)
            print(
                f"[ndvi] fold {fi}: n_eval={n:>4} model_mse={m_mse:.5f} "
                f"persist_mse={p_mse:.5f} skill={sk:+.4f}"
            )

    pooled_model_mse = agg_err_model / agg_n if agg_n else None
    pooled_persist_mse = agg_err_persist / agg_n if agg_n else None
    skill = compute_skill_score(pooled_model_mse, pooled_persist_mse)
    beats = skill is not None and skill > 0.0
    if verbose:
        print(
            f"\n[ndvi] POOLED across {n_folds} folds: n={agg_n} "
            f"model_mse={pooled_model_mse:.5f} persist_mse={pooled_persist_mse:.5f}\n"
            f"[ndvi] REAL skill vs persistence = {skill:+.4f}  "
            f"({'BEATS' if beats else 'does NOT beat'} persistence)"
        )

    # Train a FINAL model on ALL real (+ synthetic) windows for the
    # candidate checkpoint. Its skill is the CV estimate above (the final
    # model has seen all cells, so it cannot be honestly self-evaluated).
    all_cells = real_cells + syn_cells
    ax, ay, _ = _windows_to_tensors(all_cells, k)
    final_model = _train_one_ndvi(
        ax, ay, k, n_epochs=n_epochs, batch_size=batch_size, seed=SEED
    )
    n_params = sum(p.numel() for p in final_model.parameters())

    # Export ONNX + state_dict.
    input_dim = k + N_CONTEXT
    onnx_path = out_dir / "dynamics_ndvi.onnx"
    state_dict_path = out_dir / "dynamics_ndvi.state_dict.pt"
    torch.save(final_model.state_dict(), state_dict_path)
    final_model.eval()
    dummy = torch.zeros(1, input_dim, dtype=torch.float32)
    torch.onnx.export(
        final_model,
        (dummy,),
        str(onnx_path),
        input_names=["input"],
        output_names=["output"],
        dynamic_axes={"input": {0: "batch"}, "output": {0: "batch"}},
        opset_version=17,
        dynamo=False,
    )
    onnx_bytes = onnx_path.read_bytes()
    artifact_blake2b = hashlib.blake2b(onnx_bytes, digest_size=32).hexdigest()
    ckpt_blake2b = hashlib.blake2b(state_dict_path.read_bytes(), digest_size=32).hexdigest()

    synthetic_fraction = (
        n_syn_windows / (n_syn_windows + n_real_windows)
        if (n_syn_windows + n_real_windows)
        else 0.0
    )
    return {
        "k": k,
        "n_folds": n_folds,
        "n_real_cells": len(real_cells),
        "n_real_windows": n_real_windows,
        "n_real_cells_fell_back_to_null_island": n_fell_back,
        "n_synthetic_windows": n_syn_windows,
        "use_synthetic_augmentation": use_synthetic,
        "synthetic_fraction": synthetic_fraction,
        "n_params": n_params,
        "input_dim": input_dim,
        "fold_model_mse": fold_model_mse,
        "fold_persist_mse": fold_persist_mse,
        "fold_n": fold_n,
        "real_ndvi_mse_model": pooled_model_mse,
        "real_ndvi_mse_persistence": pooled_persist_mse,
        "real_ndvi_skill_score": skill,
        "beats_persistence": beats,
        "onnx_path": str(onnx_path),
        "state_dict_path": str(state_dict_path),
        "onnx_size_bytes": len(onnx_bytes),
        "artifact_blake2b": artifact_blake2b,
        "checkpoint_blake2b": ckpt_blake2b,
    }


def write_ndvi_metadata(out_dir: Path, stats: Dict) -> None:
    meta = {
        "model_id": "jepa_temporal_predictor@2.ndvi_scalar",
        "version": "0.0.1-candidate-ndvi-only-real-cv",
        "trained_at_unix": int(time.time()),
        "trained_at_iso": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "candidate_only": True,
        "do_not_deploy_unless_beats_persistence": True,
        "architecture": {
            "kind": "transformer_encoder",
            "n_bands": 1,
            "band_keys": ["indices.ndvi"],
            "input_lags": stats["k"],
            "n_context": N_CONTEXT,
            "context_keys": ["lat_norm", "lng_sin", "lng_cos", "month_sin", "month_cos"],
            "input_dim": stats["input_dim"],
            "output_dim": 2,
            "d_model": D_MODEL,
            "n_heads": N_HEADS,
            "n_layers": N_LAYERS,
            "dropout": DROPOUT,
            "n_parameters": stats["n_params"],
        },
        "normalisation": {
            "band_norm": {"indices.ndvi": list(NDVI_NORM)},
            "physical_ranges": {"indices.ndvi": list(NDVI_RANGE)},
        },
        "training": {
            "trained": True,
            "mode": "ndvi_only_real_corpus_grouped_cv",
            "cadence": "annual_peak_ndvi (median inter-obs gap ~365 days on this corpus)",
            "n_folds": stats["n_folds"],
            "cv_scheme": "grouped K-fold over cells (hold-cells-out; no cell leaks train->eval)",
            "n_real_cells": stats["n_real_cells"],
            "n_real_windows": stats["n_real_windows"],
            "n_real_cells_fell_back_to_null_island": stats["n_real_cells_fell_back_to_null_island"],
            "n_synthetic_windows": stats["n_synthetic_windows"],
            "use_synthetic_augmentation": stats["use_synthetic_augmentation"],
            "synthetic_fraction": stats["synthetic_fraction"],
            "n_epochs": N_EPOCHS,
            "loss": "gaussian_negative_log_likelihood",
            "skill_reference": "predict-last-value (persistence / Markov-1)",
            "real_ndvi_mse_model": stats["real_ndvi_mse_model"],
            "real_ndvi_mse_persistence": stats["real_ndvi_mse_persistence"],
            "real_ndvi_skill_score": stats["real_ndvi_skill_score"],
            "beats_persistence": stats["beats_persistence"],
            "fold_model_mse": stats["fold_model_mse"],
            "fold_persist_mse": stats["fold_persist_mse"],
            "fold_n": stats["fold_n"],
            "checkpoint_blake2b_hex": stats["checkpoint_blake2b"],
            "honesty_caveats": [
                "skill computed ONLY on held-out REAL NDVI windows via "
                "grouped cell-wise K-fold; synthetic series (if used) are "
                "train-split augmentation and never enter the eval.",
                "the corpus deep cells are annual peak-season NDVI; "
                "persistence (last-year ~ next-year) is a strong baseline, "
                "so skill <= 0 is an honest and expected outcome absent more "
                "(sub-annual / multi-decade) ground truth.",
            ],
        },
        "artifact": {
            "filename": "dynamics_ndvi.onnx",
            "size_bytes": stats["onnx_size_bytes"],
            "blake2b_hex": stats["artifact_blake2b"],
            "onnx_opset": 17,
        },
    }
    meta_path = out_dir / "dynamics_ndvi.metadata.json"
    meta_path.write_text(json.dumps(meta, indent=2) + "\n")
    print(f"[ndvi] wrote metadata to {meta_path}")


def main() -> None:
    p = argparse.ArgumentParser(description="Train jepa_v2 scalar dynamics head")
    p.add_argument(
        "--out-dir",
        default=os.environ.get("EMEM_JEPA_V2_DIR")
        or str(Path(os.environ.get("EMEM_DATA", "/home/ubuntu/emem/var/emem")) / "jepa_v2"),
    )
    p.add_argument("--audit-dump", default=str(DEFAULT_AUDIT_DUMP))
    p.add_argument("--quiet", action="store_true")
    p.add_argument(
        "--ndvi-only",
        action="store_true",
        help="train the single-band NDVI dynamics head on real corpus with "
        "hold-cells-out CV; gate on beating persistence on held-out real NDVI",
    )
    p.add_argument("--k", type=int, default=3, help="lag window (NDVI-only mode)")
    p.add_argument("--n-folds", type=int, default=5)
    p.add_argument("--n-synthetic", type=int, default=4000)
    p.add_argument(
        "--no-synthetic",
        action="store_true",
        help="NDVI-only: train on real windows ALONE (no synthetic augmentation)",
    )
    args = p.parse_args()
    out_dir = Path(args.out_dir)
    audit_dump = Path(args.audit_dump)

    if args.ndvi_only:
        stats = train_ndvi_only(
            audit_dump=audit_dump,
            out_dir=out_dir,
            k=args.k,
            n_folds=args.n_folds,
            n_synthetic=args.n_synthetic,
            use_synthetic=not args.no_synthetic,
            verbose=not args.quiet,
        )
        write_ndvi_metadata(out_dir, stats)
        sk = stats["real_ndvi_skill_score"]
        print(
            f"\n=== NDVI-ONLY DONE ===\n"
            f"  k                 : {stats['k']}\n"
            f"  real cells        : {stats['n_real_cells']}\n"
            f"  real windows      : {stats['n_real_windows']}\n"
            f"  synthetic frac    : {stats['synthetic_fraction']:.3f}\n"
            f"  model MSE (real)  : {stats['real_ndvi_mse_model']:.5f}\n"
            f"  persist MSE (real): {stats['real_ndvi_mse_persistence']:.5f}\n"
            f"  REAL SKILL        : {sk:+.4f}  "
            f"({'BEATS persistence — DEPLOYABLE' if stats['beats_persistence'] else 'does NOT beat persistence — KEEP FAIL-SAFE'})\n"
            f"  ONNX              : {stats['onnx_path']}  ({stats['onnx_size_bytes']} bytes)\n"
        )
        return

    stats = train(audit_dump=audit_dump, out_dir=out_dir, verbose=not args.quiet)
    write_metadata(out_dir, stats)
    print(
        f"\n=== DONE ===\n"
        f"  ONNX        : {stats['onnx_path']}  ({stats['onnx_size_bytes']} bytes)\n"
        f"  state_dict  : {stats['state_dict_path']}\n"
        f"  final val   : {stats['final_val_loss']:.4f}\n"
        f"  baseline val: {stats['identity_baseline_val_loss']:.4f}\n"
        f"  lift NLL    : {stats['val_lift_pct_over_identity_baseline_nll']:.1f}%\n"
        f"  lift NDVI   : {stats['val_lift_pct_over_identity_baseline_ndvi_mse']:.1f}% (MSE in physical units)\n"
    )


if __name__ == "__main__":
    main()
