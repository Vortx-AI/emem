"""Golden-vector + correctness tests for the Galileo S2 normalization.

Audit 2026-05-30 (HIGH fix #1): the sidecar previously z-scored S2
(`(x-mean)/std`). Galileo's upstream `Normalizer` (galileo/src/data/
dataset.py, std=True path; arXiv:2502.09356) instead maps each band to
roughly [-1, 1] via shift/divide:

    shift = mean - std_multiplier*std   (std_multiplier=2 default)
    div   = (mean + 2*std) - (mean - 2*std) = 4*std
    x_norm = (x - shift) / div

The released `nasaharvest/galileo` snapshot ships NO normalization.json
(verified on-disk 2026-05-30), so the computed (mean-2σ)/(4σ) path is the
authoritative one. These tests pin the exact normalized value for a known
input and prove the corrected scheme differs from the old z-score.

Importing server.py needs the runtime deps (fastapi/torch/numpy/pydantic),
which are present in the sidecar venv. If they're missing (bare checkout)
the whole module is skipped so `pytest -q` stays green.
"""

from __future__ import annotations

import os
from pathlib import Path

try:
    import pytest

    HAVE_PYTEST = True
except ImportError:  # pragma: no cover
    HAVE_PYTEST = False

    class _PytestStub:  # minimal shim for plain `python` invocation
        @staticmethod
        def skip(reason: str) -> None:
            raise SystemExit(0)

        class mark:  # noqa: N801
            @staticmethod
            def skipif(condition: bool, reason: str = ""):
                def _decorator(fn):
                    return fn

                return _decorator

    pytest = _PytestStub()  # type: ignore[assignment]

try:
    import numpy as np

    import server  # noqa: WPS433 — the sidecar module under test
    from server import (
        GALILEO_S2_MEAN,
        GALILEO_S2_STD,
        _galileo_s2_shift_div,
    )

    HAVE_SERVER = True
    IMPORT_ERR = ""
except Exception as e:  # noqa: BLE001 — record why so the skip is honest
    HAVE_SERVER = False
    IMPORT_ERR = f"{type(e).__name__}: {e}"


@pytest.mark.skipif(not HAVE_SERVER, reason=f"server import failed: {IMPORT_ERR}")
def test_shift_div_is_computed_from_mean_std() -> None:
    """With no normalization.json in the snapshot, shift/div MUST be the
    upstream (mean-2σ)/(4σ) values, NOT a z-score's (mean, std)."""
    shift, div, source = _galileo_s2_shift_div(None)
    mean = np.asarray(GALILEO_S2_MEAN, dtype=np.float32)
    std = np.asarray(GALILEO_S2_STD, dtype=np.float32)
    assert np.allclose(shift, mean - 2.0 * std), "shift must be mean - 2σ"
    assert np.allclose(div, 4.0 * std), "div must be 4σ"
    assert "computed" in source, source
    # And it MUST differ from a z-score (shift==mean, div==std).
    assert not np.allclose(shift, mean), "shift must not equal mean (that's z-score)"
    assert not np.allclose(div, std), "div must not equal std (that's z-score)"


@pytest.mark.skipif(not HAVE_SERVER, reason=f"server import failed: {IMPORT_ERR}")
def test_golden_normalized_value_band0() -> None:
    """Pin the exact shift/divide-normalized value for a known input on
    band 0 (B2). mean=1395.3408730676722, std=917.7041440370853.
        shift = mean - 2*std = -440.0674150065  (≈)
        div   = 4*std        =  3670.81657615   (≈)
        x=2000 → (2000 - shift)/div ≈ 0.6647206
    This is the load-bearing golden: the OLD z-score would give
    (2000-mean)/std ≈ 0.6588824, a materially different number that gets
    SIGNED into the embedding."""
    shift, div, _ = _galileo_s2_shift_div(None)
    x = 2000.0
    got = float((x - shift[0]) / div[0])
    expected_shiftdiv = 0.664720604908779
    expected_zscore = 0.6588824196351161
    assert abs(got - expected_shiftdiv) < 1e-6, f"shift/divide value drifted: {got}"
    assert abs(got - expected_zscore) > 1e-3, (
        "normalized value collapsed back to the (wrong) z-score scheme"
    )


@pytest.mark.skipif(not HAVE_SERVER, reason=f"server import failed: {IMPORT_ERR}")
def test_normalization_prefers_on_disk_json(tmp_path: Path) -> None:
    """If a normalization.json with shift/div is present in the model dir,
    it overrides the computed values (forward-compat for a future snapshot
    or operator pin)."""
    import json

    d = tmp_path
    shift_vals = [float(i) for i in range(10)]
    div_vals = [float(i + 1) for i in range(10)]
    (d / "normalization.json").write_text(
        json.dumps({"shift": shift_vals, "div": div_vals})
    )
    shift, div, source = _galileo_s2_shift_div(d)
    assert list(shift) == shift_vals
    assert list(div) == div_vals
    assert "normalization.json" in source


@pytest.mark.skipif(not HAVE_SERVER, reason=f"server import failed: {IMPORT_ERR}")
def test_chip_gsd_is_30m() -> None:
    """The chip GSD constant must be 30 m so the forward passes
    input_resolution_m=30 (gsd_ratio = 30*2/10 = 6, not the wrong 2)."""
    assert server.GALILEO_CHIP_GSD_M == 30


if __name__ == "__main__":
    if not HAVE_SERVER:
        print(f"[galileo] server import failed; skipping ({IMPORT_ERR})")
        raise SystemExit(0)
    test_shift_div_is_computed_from_mean_std()
    test_golden_normalized_value_band0()
    import tempfile

    with tempfile.TemporaryDirectory() as td:
        test_normalization_prefers_on_disk_json(Path(td))
    test_chip_gsd_is_30m()
    print("[galileo] all normalization tests passed")
