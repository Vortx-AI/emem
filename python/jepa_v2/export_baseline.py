"""Export an untrained sentinel `dynamics_v2.onnx` so the Rust runtime
can ship before real training data is assembled.

By construction (residual MLP with zero-init head), the untrained model
returns last_input_vintage (cosine ≈ 1.0 against itself) — the
"predict last vintage" baseline. The receipt metadata flags this as
`untrained_residual_baseline` so an agent never reads it as a real
prediction.

Replace by running `assemble_data.py` then `train.py` once enough cells
have ≥4 consecutive Tessera vintages cached.

Usage:
    python export_baseline.py [--out-dir PATH]
"""

import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path

import torch

from train import DynamicsModel, INPUT_LAGS, TESSERA_DIM


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--out-dir", type=Path, default=None)
    args = parser.parse_args()

    if args.out_dir is None:
        emem_data = Path(os.environ.get("EMEM_DATA", "/home/ubuntu/emem/var/emem"))
        args.out_dir = emem_data / "jepa_v2"
    args.out_dir.mkdir(parents=True, exist_ok=True)

    # Reproducible: seeded model, then immediately zero out the head so
    # the residual prediction collapses to last_input_vintage exactly.
    torch.manual_seed(42)
    model = DynamicsModel()
    with torch.no_grad():
        model.head.weight.zero_()
        model.head.bias.zero_()
    model.eval()

    # Sanity check: residual sum is exactly the last input.
    dummy = torch.randn(2, INPUT_LAGS, TESSERA_DIM)
    pred = model(dummy)
    last = dummy[:, -1, :]
    max_diff = (pred - last).abs().max().item()
    if max_diff > 1e-6:
        raise RuntimeError(
            f"baseline sanity-check failed: |pred - last_input| = {max_diff} (expected ~0). "
            f"The residual-head-zero invariant is broken."
        )

    onnx_path = args.out_dir / "dynamics_v2.onnx"
    print(f"exporting baseline to {onnx_path}", flush=True)
    torch.onnx.export(
        model,
        torch.randn(1, INPUT_LAGS, TESSERA_DIM),
        str(onnx_path),
        input_names=["lags"],
        output_names=["next_vintage"],
        dynamic_axes={"lags": {0: "batch"}, "next_vintage": {0: "batch"}},
        opset_version=17,
    )

    onnx_bytes = onnx_path.read_bytes()
    digest = hashlib.blake2b(onnx_bytes, digest_size=32).hexdigest()
    metadata = {
        "model_id": "jepa_temporal_predictor@2",
        "version": "0.0.0-untrained-baseline",
        "trained_at_unix": int(datetime.datetime.now(datetime.UTC).timestamp()),
        "trained_at_iso": datetime.datetime.now(datetime.UTC).isoformat(),
        "architecture": {
            "input_lags": INPUT_LAGS,
            "tessera_dim": TESSERA_DIM,
            "kind": "residual_mlp",
            "residual_target": "delta vs last input vintage",
            "head_initialised": "zero (untrained)",
        },
        "training": {
            "trained": False,
            "rationale": (
                "shipped untrained because Tessera vintages were not yet cached "
                "on this responder when v2 went live. With head zero-initialised, "
                "the residual MLP returns last_input_vintage exactly — i.e. the "
                "'predict last vintage' baseline. Receipt MUST surface this as "
                "untrained so agents do not treat it as a learned prediction."
            ),
        },
        "validation": {
            "by_construction_cosine": 1.0,
            "by_construction_mse": 0.0,
            "interpretation": (
                "this is identity (output == last_input_vintage). Any non-zero "
                "lift over the baseline requires running train.py against real "
                "data; the metadata's `training.trained` flag becomes true and "
                "real validation_cosine / mse are reported."
            ),
        },
        "artifact": {
            "filename": "dynamics_v2.onnx",
            "size_bytes": len(onnx_bytes),
            "blake2b_hex": digest,
        },
    }
    sidecar = args.out_dir / "dynamics_v2.metadata.json"
    sidecar.write_text(json.dumps(metadata, indent=2))
    print(f"wrote {sidecar}", flush=True)
    print(json.dumps(metadata, indent=2), flush=True)


if __name__ == "__main__":
    main()
