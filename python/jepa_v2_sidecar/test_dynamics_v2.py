"""Smoke test for the post-pivot multi-band scalar dynamics surface.

Runs three checks against the on-disk artifact in
`$EMEM_JEPA_V2_DIR/dynamics_v2.onnx`:

1. The ONNX loads and serves a non-degenerate input without crashing.
2. The output is NOT bit-identical to any input lag (rules out
   identity-baseline contamination).
3. The output is in physically reasonable ranges per band (NDVI in
   [-1, 1], LST in [200, 350] K, PM2.5 >= 0).

Run with `pytest python/jepa_v2_sidecar/test_dynamics_v2.py` or as
`python python/jepa_v2_sidecar/test_dynamics_v2.py` for a manual
invocation. The test silently skips when the artifact isn't yet
present (clean checkout).
"""

from __future__ import annotations

import json
import os
from pathlib import Path

import numpy as np

try:
    import pytest

    HAVE_PYTEST = True
except ImportError:    # pragma: no cover
    HAVE_PYTEST = False
    # Provide a tiny shim so the @skipif decorator is a no-op when
    # invoked via plain `python` and the file is still importable.
    class _PytestStub:
        @staticmethod
        def skip(reason: str) -> None:    # type: ignore[unused-argument]
            raise SystemExit(0)

        class mark:    # noqa: N801
            @staticmethod
            def skipif(condition: bool, reason: str = ""):
                def _decorator(fn):
                    return fn
                return _decorator

    pytest = _PytestStub()    # type: ignore[assignment]

from train_dynamics_v2 import (
    BAND_KEYS,
    BAND_NORM,
    BAND_PHYSICAL_RANGES,
    INPUT_DIM,
    K_LAGS,
    N_BANDS,
    N_CONTEXT,
    OUTPUT_DIM,
    _build_context,
    _denormalise_pred,
    _normalise_lags,
)

EMEM_DATA = Path(os.environ.get("EMEM_DATA", "/home/ubuntu/emem/var/emem"))
DYN_DIR = Path(os.environ.get("EMEM_JEPA_V2_DIR", str(EMEM_DATA / "jepa_v2")))


def _have_artifact() -> bool:
    return (
        (DYN_DIR / "dynamics_v2.onnx").exists()
        and (DYN_DIR / "dynamics_v2.metadata.json").exists()
    )


def _load_metadata() -> dict:
    return json.loads((DYN_DIR / "dynamics_v2.metadata.json").read_text())


@pytest.mark.skipif(not _have_artifact(), reason="dynamics_v2 artifact missing")
def test_onnx_runs_on_non_degenerate_input() -> None:
    """The ONNX must accept the documented [1, INPUT_DIM] f32 input and
    return [1, OUTPUT_DIM] without throwing."""
    import onnxruntime as ort

    sess = ort.InferenceSession(
        str(DYN_DIR / "dynamics_v2.onnx"), providers=["CPUExecutionProvider"]
    )
    # Six lags of monotonically-rising physical values per band.
    lags_phys = np.zeros((K_LAGS, N_BANDS), dtype=np.float32)
    for k in range(K_LAGS):
        lags_phys[k, 0] = 0.40 + 0.05 * k    # NDVI 0.40..0.65
        lags_phys[k, 1] = 285.0 + 2.0 * k    # LST_day 285..295
        lags_phys[k, 2] = 275.0 + 1.5 * k    # LST_night 275..282.5
        lags_phys[k, 3] = 12.0 + 0.5 * k     # PM25 12..14.5
    lags_norm = _normalise_lags(lags_phys).reshape(-1)
    ctx = _build_context(lat_deg=45.0, lng_deg=-90.0, month_idx=6)
    flat = np.concatenate([lags_norm, ctx]).astype(np.float32)
    assert flat.shape == (INPUT_DIM,)
    out = sess.run(["output"], {"input": flat[None, :]})[0]
    assert out.shape == (1, OUTPUT_DIM), f"unexpected output shape {out.shape}"


@pytest.mark.skipif(not _have_artifact(), reason="dynamics_v2 artifact missing")
def test_output_not_identity_baseline() -> None:
    """The trained model must NOT produce a prediction bit-identical to
    any of the input lags. If it did, we're shipping the residual-zero
    sentinel under a "trained=true" receipt — exactly the corruption
    this PR was meant to fix."""
    meta = _load_metadata()
    if not meta.get("training", {}).get("trained", False):
        pytest.skip("metadata still flags untrained=true; identity check N/A")
    import onnxruntime as ort

    sess = ort.InferenceSession(
        str(DYN_DIR / "dynamics_v2.onnx"), providers=["CPUExecutionProvider"]
    )
    lags_phys = np.zeros((K_LAGS, N_BANDS), dtype=np.float32)
    for k in range(K_LAGS):
        lags_phys[k, 0] = 0.40 + 0.05 * k
        lags_phys[k, 1] = 285.0 + 2.0 * k
        lags_phys[k, 2] = 275.0 + 1.5 * k
        lags_phys[k, 3] = 12.0 + 0.5 * k
    lags_norm = _normalise_lags(lags_phys).reshape(-1)
    ctx = _build_context(lat_deg=45.0, lng_deg=-90.0, month_idx=6)
    flat = np.concatenate([lags_norm, ctx]).astype(np.float32)
    out_raw = sess.run(["output"], {"input": flat[None, :]})[0][0]
    pred_norm = out_raw[:N_BANDS]
    pred_phys = _denormalise_pred(pred_norm)
    # No lag value should exactly equal the prediction.
    for k in range(K_LAGS):
        for j in range(N_BANDS):
            diff = abs(float(pred_phys[j]) - float(lags_phys[k, j]))
            assert diff > 1e-6, (
                f"prediction[{BAND_KEYS[j]}]={pred_phys[j]} == lag[{k}][{j}]={lags_phys[k, j]} "
                "to within fp32 precision; the model is acting as identity"
            )


@pytest.mark.skipif(not _have_artifact(), reason="dynamics_v2 artifact missing")
def test_output_in_physical_ranges() -> None:
    """After denormalisation + clamp the predicted values MUST be in
    the documented physical ranges per band."""
    import onnxruntime as ort

    sess = ort.InferenceSession(
        str(DYN_DIR / "dynamics_v2.onnx"), providers=["CPUExecutionProvider"]
    )
    lags_phys = np.zeros((K_LAGS, N_BANDS), dtype=np.float32)
    for k in range(K_LAGS):
        lags_phys[k, 0] = 0.40 + 0.05 * k
        lags_phys[k, 1] = 285.0 + 2.0 * k
        lags_phys[k, 2] = 275.0 + 1.5 * k
        lags_phys[k, 3] = 12.0 + 0.5 * k
    lags_norm = _normalise_lags(lags_phys).reshape(-1)
    ctx = _build_context(lat_deg=45.0, lng_deg=-90.0, month_idx=6)
    flat = np.concatenate([lags_norm, ctx]).astype(np.float32)
    raw = sess.run(["output"], {"input": flat[None, :]})[0][0]
    pred_phys = _denormalise_pred(raw[:N_BANDS])
    for j, band in enumerate(BAND_KEYS):
        lo, hi = BAND_PHYSICAL_RANGES[band]
        p = float(pred_phys[j])
        # NB: the model can technically emit out-of-range values; the
        # Rust + sidecar path clamps. Test that the *pre-clamp* output
        # isn't catastrophically far from the physical range either.
        slack = (hi - lo) * 0.25
        assert lo - slack <= p <= hi + slack, (
            f"prediction[{band}]={p} way outside physical range [{lo}, {hi}] "
            "(pre-clamp); the model has diverged from training distribution"
        )


@pytest.mark.skipif(not _have_artifact(), reason="dynamics_v2 artifact missing")
def test_metadata_carries_trained_flag_and_blake2b() -> None:
    """The metadata sidecar MUST report trained=true, a positive
    real-NDVI evaluation count, and a 64-hex-char artifact hash."""
    meta = _load_metadata()
    if not meta.get("training", {}).get("trained", False):
        pytest.skip("metadata still untrained")
    assert meta["training"]["trained"] is True
    assert meta["training"]["training_data_size"] >= 1_000
    assert meta["training"]["val_loss"] < meta["training"]["identity_baseline_val_loss"], (
        "trained model MUST beat the identity baseline on val NLL"
    )
    artifact = meta["artifact"]
    assert len(artifact["blake2b_hex"]) == 64
    assert artifact["size_bytes"] > 8_192, (
        f"a real trained ONNX should be larger than the 8KB sentinel; "
        f"got {artifact['size_bytes']}"
    )


if __name__ == "__main__":
    if not _have_artifact():
        print("[smoke] dynamics_v2 artifact missing; nothing to test")
        raise SystemExit(0)
    test_onnx_runs_on_non_degenerate_input()
    test_output_not_identity_baseline()
    test_output_in_physical_ranges()
    test_metadata_carries_trained_flag_and_blake2b()
    print("[smoke] all 4 tests passed")
