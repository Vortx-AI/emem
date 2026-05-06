"""Train the jepa_temporal_predictor@2 dynamics head.

Loads `training_data.npz` (assemble_data.py output), trains a small
residual MLP that predicts vintage[t+1] from [t-2, t-1, t], holds out
the latest year per cell as validation, and exports `dynamics_v2.onnx`
plus a metadata sidecar with hyperparameters + validation metrics.

Usage:
    python train.py [--data PATH] [--out-dir PATH]

Defaults: <EMEM_DATA>/jepa_v2/{training_data.npz, dynamics_v2.onnx}.
"""

import argparse
import datetime
import hashlib
import json
import os
import random
from pathlib import Path

import numpy as np
import torch
import torch.nn as nn
import torch.nn.functional as F

# Pinned hyperparameters — bumping these is a model-version change.
SEED = 42
INPUT_LAGS = 3                # how many past vintages feed the predictor
TESSERA_DIM = 128
HIDDEN = 256
N_BLOCKS = 4
DROPOUT = 0.10
LR = 3e-4
WEIGHT_DECAY = 1e-4
EPOCHS = 200
BATCH_SIZE = 128


class ResidualBlock(nn.Module):
    """Pre-LN residual MLP block — trains stably even on tiny datasets."""

    def __init__(self, dim: int, hidden: int, dropout: float):
        super().__init__()
        self.norm = nn.LayerNorm(dim)
        self.fc1 = nn.Linear(dim, hidden)
        self.fc2 = nn.Linear(hidden, dim)
        self.drop = nn.Dropout(dropout)

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        return x + self.fc2(self.drop(F.gelu(self.fc1(self.norm(x)))))


class DynamicsModel(nn.Module):
    """Predicts the next-vintage Tessera vector from a window of K prior
    vintages. Architecture: project the K×128 input to a single 128-D
    state, run N residual blocks, then read out 128-D delta vs the most
    recent input vintage. Predicting a delta (residual) makes the
    "predict last vintage" baseline fall out for free at zero-init."""

    def __init__(
        self,
        dim: int = TESSERA_DIM,
        lags: int = INPUT_LAGS,
        hidden: int = HIDDEN,
        n_blocks: int = N_BLOCKS,
        dropout: float = DROPOUT,
    ):
        super().__init__()
        self.lags = lags
        self.proj_in = nn.Linear(dim * lags, dim)
        self.blocks = nn.ModuleList(
            [ResidualBlock(dim, hidden, dropout) for _ in range(n_blocks)]
        )
        self.head = nn.Linear(dim, dim)

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        # x: [batch, K, dim]
        last = x[:, -1, :]                       # most-recent vintage
        h = self.proj_in(x.flatten(1))           # [batch, dim]
        for blk in self.blocks:
            h = blk(h)
        delta = self.head(h)                     # [batch, dim]
        return last + delta                      # residual prediction


def cosine_similarity(a: torch.Tensor, b: torch.Tensor) -> torch.Tensor:
    return F.cosine_similarity(a, b, dim=-1)


def loss_fn(pred: torch.Tensor, target: torch.Tensor) -> torch.Tensor:
    """Cosine + L2 in latent space. Cosine alone fixes scale-invariant
    direction but ignores magnitude; L2 alone is dominated by the
    high-variance dimensions. Equal-weight combination matches what
    Tessera's own training loss does for embedding-space objectives."""
    cos_loss = (1.0 - cosine_similarity(pred, target)).mean()
    l2_loss = F.mse_loss(pred, target)
    return cos_loss + l2_loss, cos_loss.detach(), l2_loss.detach()


def build_pairs(vectors: np.ndarray, mask: np.ndarray, lags: int):
    """Returns lists of (cell_idx, t_target, x [lags,128], y [128]). The
    `t_target` is the year-offset within the cell's run that we predict."""
    pairs = []
    for ci in range(vectors.shape[0]):
        valid_T = int(mask[ci].sum())
        # Need lags valid years before t_target, i.e. t_target ∈ [lags, valid_T-1]
        for t in range(lags, valid_T):
            x = vectors[ci, t - lags : t]      # [lags, 128]
            y = vectors[ci, t]                 # [128]
            pairs.append((ci, t, x, y))
    return pairs


def split_pairs(pairs, mask):
    """Hold out each cell's LATEST training pair as validation. This
    matches the production use-case (predict the most-recent next-year
    vintage from prior history) without leaking future information."""
    train, val = [], []
    seen_cell_latest = {}
    for ci, t, x, y in pairs:
        prev = seen_cell_latest.get(ci)
        if prev is None or t > prev[1]:
            seen_cell_latest[ci] = (ci, t, x, y)
    val_keys = {(ci, t) for ci, (_, t, _, _) in seen_cell_latest.items()}
    for ci, t, x, y in pairs:
        if (ci, t) in val_keys:
            val.append((ci, t, x, y))
        else:
            train.append((ci, t, x, y))
    return train, val


def to_batches(pairs, batch_size, shuffle=True):
    idx = list(range(len(pairs)))
    if shuffle:
        random.shuffle(idx)
    for i in range(0, len(idx), batch_size):
        chunk = [pairs[j] for j in idx[i : i + batch_size]]
        x = torch.from_numpy(np.stack([p[2] for p in chunk])).float()  # [B, lags, 128]
        y = torch.from_numpy(np.stack([p[3] for p in chunk])).float()  # [B, 128]
        yield x, y


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--data", type=Path, default=None)
    parser.add_argument("--out-dir", type=Path, default=None)
    args = parser.parse_args()

    emem_data = Path(os.environ.get("EMEM_DATA", "/home/ubuntu/emem/var/emem"))
    if args.data is None:
        args.data = emem_data / "jepa_v2" / "training_data.npz"
    if args.out_dir is None:
        args.out_dir = emem_data / "jepa_v2"
    args.out_dir.mkdir(parents=True, exist_ok=True)

    # Determinism
    random.seed(SEED)
    np.random.seed(SEED)
    torch.manual_seed(SEED)
    torch.use_deterministic_algorithms(False)  # determinism nice-to-have, not required for v2 bootstrap

    print(f"loading {args.data}", flush=True)
    z = np.load(args.data, allow_pickle=True)
    vectors = z["vectors"]                                 # [N_cells, max_run, 128]
    mask = z["mask"]                                       # [N_cells, max_run]
    cells = [str(c) for c in z["cells"]]
    print(f"  {vectors.shape[0]} cells, max_run={vectors.shape[1]}", flush=True)

    pairs = build_pairs(vectors, mask, INPUT_LAGS)
    print(f"built {len(pairs)} (input, target) pairs", flush=True)

    train_pairs, val_pairs = split_pairs(pairs, mask)
    print(f"split: {len(train_pairs)} train / {len(val_pairs)} val", flush=True)

    if len(train_pairs) == 0 or len(val_pairs) == 0:
        raise RuntimeError(
            "training-data assembly produced no train/val pairs. "
            "Each cell needs ≥4 consecutive Tessera vintages: "
            "INPUT_LAGS+1 = 4. Run assemble_data.py against more cells."
        )

    # CPU is fine for this tiny model; CUDA available but unnecessary.
    device = torch.device("cpu")
    model = DynamicsModel().to(device)
    n_params = sum(p.numel() for p in model.parameters())
    print(f"model: {n_params} params", flush=True)

    opt = torch.optim.AdamW(model.parameters(), lr=LR, weight_decay=WEIGHT_DECAY)
    best_val_cos = -2.0
    best_state = None
    for epoch in range(EPOCHS):
        model.train()
        train_losses = []
        for x, y in to_batches(train_pairs, BATCH_SIZE, shuffle=True):
            x = x.to(device)
            y = y.to(device)
            opt.zero_grad()
            pred = model(x)
            loss, cos_l, l2_l = loss_fn(pred, y)
            loss.backward()
            opt.step()
            train_losses.append(loss.item())

        model.eval()
        with torch.no_grad():
            xs = torch.from_numpy(np.stack([p[2] for p in val_pairs])).float().to(device)
            ys = torch.from_numpy(np.stack([p[3] for p in val_pairs])).float().to(device)
            preds = model(xs)
            val_cos = cosine_similarity(preds, ys).mean().item()
            # Baseline: "predict last vintage"
            last = xs[:, -1, :]
            base_cos = cosine_similarity(last, ys).mean().item()

        if val_cos > best_val_cos:
            best_val_cos = val_cos
            best_state = {k: v.detach().clone() for k, v in model.state_dict().items()}

        if epoch % 20 == 0 or epoch == EPOCHS - 1:
            print(
                f"  epoch {epoch:4d}  train_loss={np.mean(train_losses):.4f}  "
                f"val_cos={val_cos:.4f}  baseline_cos={base_cos:.4f}  "
                f"lift={val_cos - base_cos:+.4f}",
                flush=True,
            )

    assert best_state is not None
    model.load_state_dict(best_state)
    model.eval()

    # Final eval
    with torch.no_grad():
        xs = torch.from_numpy(np.stack([p[2] for p in val_pairs])).float().to(device)
        ys = torch.from_numpy(np.stack([p[3] for p in val_pairs])).float().to(device)
        preds = model(xs)
        final_cos = cosine_similarity(preds, ys).mean().item()
        final_mse = F.mse_loss(preds, ys).item()
        baseline_cos = cosine_similarity(xs[:, -1, :], ys).mean().item()
        baseline_mse = F.mse_loss(xs[:, -1, :], ys).item()

    onnx_path = args.out_dir / "dynamics_v2.onnx"
    print(f"exporting to {onnx_path}", flush=True)
    dummy = torch.randn(1, INPUT_LAGS, TESSERA_DIM)
    torch.onnx.export(
        model,
        dummy,
        str(onnx_path),
        input_names=["lags"],
        output_names=["next_vintage"],
        dynamic_axes={"lags": {0: "batch"}, "next_vintage": {0: "batch"}},
        opset_version=17,
    )

    onnx_bytes = onnx_path.read_bytes()
    model_blake3 = hashlib.blake2b(onnx_bytes, digest_size=32).hexdigest()
    metadata = {
        "model_id": "jepa_temporal_predictor@2",
        "version": "0.0.1-bootstrap",
        "trained_at_unix": int(datetime.datetime.now(datetime.UTC).timestamp()),
        "trained_at_iso": datetime.datetime.now(datetime.UTC).isoformat(),
        "architecture": {
            "input_lags": INPUT_LAGS,
            "tessera_dim": TESSERA_DIM,
            "hidden": HIDDEN,
            "n_blocks": N_BLOCKS,
            "dropout": DROPOUT,
            "n_params": n_params,
            "kind": "residual_mlp",
            "residual_target": "delta vs last input vintage",
        },
        "training": {
            "seed": SEED,
            "lr": LR,
            "weight_decay": WEIGHT_DECAY,
            "epochs": EPOCHS,
            "batch_size": BATCH_SIZE,
            "loss": "cosine + L2",
            "n_train_pairs": len(train_pairs),
            "n_val_pairs": len(val_pairs),
            "n_train_cells": len(set(p[0] for p in train_pairs)),
        },
        "validation": {
            "cosine_similarity": final_cos,
            "mse": final_mse,
            "baseline_predict_last_vintage_cosine": baseline_cos,
            "baseline_predict_last_vintage_mse": baseline_mse,
            "cosine_lift_vs_baseline": final_cos - baseline_cos,
            "mse_reduction_vs_baseline": baseline_mse - final_mse,
        },
        "artifact": {
            "filename": "dynamics_v2.onnx",
            "size_bytes": len(onnx_bytes),
            "blake2b_hex": model_blake3,
        },
        "honesty_note": (
            "v2-bootstrap: trained on a small sample of cells with ≥4 "
            "consecutive Tessera vintages. Quality is bounded by training-set "
            "diversity. Cosine lift over the 'predict last vintage' baseline "
            "is the honest measure of whether the learned head is doing "
            "anything useful — if lift is ~0 the model has not learned and "
            "the receipt should warn the agent."
        ),
    }
    sidecar = args.out_dir / "dynamics_v2.metadata.json"
    sidecar.write_text(json.dumps(metadata, indent=2))
    print(f"wrote {sidecar}", flush=True)
    print(json.dumps(metadata["validation"], indent=2), flush=True)


if __name__ == "__main__":
    main()
