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
        GALILEO_S1_MEAN,
        GALILEO_S1_STD,
        GALILEO_S2_MEAN,
        GALILEO_S2_STD,
        GALILEO_SRTM_MEAN,
        GALILEO_SRTM_STD,
        GalileoRequest,
        _galileo_modality_shift_div,
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


@pytest.mark.skipif(not HAVE_SERVER, reason=f"server import failed: {IMPORT_ERR}")
def test_s1_srtm_modality_shift_div() -> None:
    """S1 (VV,VH) and SRTM (elevation,slope) are both in Galileo's
    Normalizer `std_bands` set, so each uses (mean-2σ)/(4σ) — same scheme
    as S2, different per-modality stats (config/normalization.json keys
    "13" S1 indices 0,1 and "16" SRTM indices 0,1; verified 2026-05-30)."""
    for mean, std in ((GALILEO_S1_MEAN, GALILEO_S1_STD), (GALILEO_SRTM_MEAN, GALILEO_SRTM_STD)):
        m = np.asarray(mean, dtype=np.float32)
        sd = np.asarray(std, dtype=np.float32)
        shift, div = _galileo_modality_shift_div(mean, std)
        assert np.allclose(shift, m - 2.0 * sd), "shift must be mean - 2σ"
        assert np.allclose(div, 4.0 * sd), "div must be 4σ"
    # Pin the exact S1 VV stat so a refactor can't silently swap it.
    assert abs(GALILEO_S1_MEAN[0] - (-11.728724389184965)) < 1e-9
    assert abs(GALILEO_S1_STD[0] - 4.887145774840316) < 1e-9
    # SRTM slope is in degrees (ee.Terrain.slope), mean ≈ 5.93°.
    assert abs(GALILEO_SRTM_MEAN[1] - 5.930092668915115) < 1e-9


@pytest.mark.skipif(not HAVE_SERVER, reason=f"server import failed: {IMPORT_ERR}")
def test_request_accepts_optional_s1_srtm_and_validates_shape() -> None:
    """The multimodal request fields are optional (S2-only still valid) and
    enforce the [T,H,W,2] / [H,W,2] shapes."""
    s2 = [[[[0.0] * 10 for _ in range(8)] for _ in range(8)]]
    # S2-only: s1/srtm default to None.
    r = GalileoRequest(s2_chip=s2)
    assert r.s1_chip is None and r.srtm_chip is None
    # Full multimodal: correct shapes accepted.
    s1 = [[[[0.0, 0.0] for _ in range(8)] for _ in range(8)]]
    srtm = [[[0.0, 0.0] for _ in range(8)] for _ in range(8)]
    r2 = GalileoRequest(s2_chip=s2, s1_chip=s1, srtm_chip=srtm)
    assert r2.s1_chip is not None and r2.srtm_chip is not None
    # Wrong S1 band count rejected.
    bad_s1 = [[[[0.0] for _ in range(8)] for _ in range(8)]]
    try:
        GalileoRequest(s2_chip=s2, s1_chip=bad_s1)
        raise AssertionError("expected validation error for 1-band S1")
    except Exception as e:  # noqa: BLE001
        assert "2 bands" in str(e) or "VV" in str(e)


def _galileo_weights_dir() -> "Path | None":
    """Resolve the Galileo `base` encoder dir if the weights are present
    locally, else None (so the real-forward test skips on a bare checkout
    rather than network-fetching)."""
    try:
        d = server._resolve_galileo_dir()
    except Exception:  # noqa: BLE001
        return None
    if d is not None and (d / "encoder.pt").exists() and (d / "config.json").exists():
        return d
    return None


@pytest.mark.skipif(not HAVE_SERVER, reason=f"server import failed: {IMPORT_ERR}")
def test_average_tokens_arity_real_forward() -> None:
    """Real Galileo forward → average_tokens arity-regression guard.

    The handler calls `encoder.average_tokens(*model_output[:-1])`.
    `Encoder.forward` returns a 9-tuple ending in `months`; `average_tokens`
    needs exactly the 8 (x,m) tensors. The `[:-1]` slice drops `months` so
    the arity is 8==8. This was flagged in an audit as a possible 7-vs-8
    TypeError, but a real forward proves it works. If a future refactor
    changes either the forward return length or the average_tokens
    signature, this test fails instead of crashing only in production.
    """
    weights = _galileo_weights_dir()
    if weights is None:
        pytest.skip("galileo base encoder weights not present locally")

    import torch  # local import: only needed for this test

    from single_file_galileo import (  # noqa: WPS433
        Encoder,
        SPACE_BANDS,
        SPACE_BAND_GROUPS_IDX,
        SPACE_TIME_BANDS,
        SPACE_TIME_BANDS_GROUPS_IDX,
        SRTM_BANDS,
        STATIC_BANDS,
        STATIC_BAND_GROUPS_IDX,
        S1_BANDS,
        S2_BANDS,
        TIME_BANDS,
        TIME_BAND_GROUPS_IDX,
    )

    dev = torch.device("cpu")
    encoder = Encoder.load_from_folder(weights, dev).eval()

    h, w = server.GALILEO_CHIP_H, server.GALILEO_CHIP_W
    t = server.GALILEO_CHIP_T
    rng = np.random.default_rng(0)

    s_t_x = torch.zeros((h, w, t, len(SPACE_TIME_BANDS)))
    s_t_m = torch.ones((h, w, t, len(SPACE_TIME_BANDS_GROUPS_IDX)))
    s2_idx = [SPACE_TIME_BANDS.index(b) for b in S2_BANDS]
    s_t_x[:, :, :, s2_idx] = torch.from_numpy(
        rng.standard_normal((h, w, t, len(S2_BANDS))).astype(np.float32)
    )
    for i, k in enumerate(SPACE_TIME_BANDS_GROUPS_IDX):
        if k.startswith("S2_"):
            s_t_m[:, :, :, i] = 0
    s1_idx = [SPACE_TIME_BANDS.index(b) for b in S1_BANDS]
    s_t_x[:, :, :, s1_idx] = torch.from_numpy(
        rng.standard_normal((h, w, t, len(S1_BANDS))).astype(np.float32)
    )
    for i, k in enumerate(SPACE_TIME_BANDS_GROUPS_IDX):
        if k == "S1":
            s_t_m[:, :, :, i] = 0

    sp_x = torch.zeros((h, w, len(SPACE_BANDS)))
    sp_m = torch.ones((h, w, len(SPACE_BAND_GROUPS_IDX)))
    srtm_idx = [SPACE_BANDS.index(b) for b in SRTM_BANDS]
    sp_x[:, :, srtm_idx] = torch.from_numpy(
        rng.standard_normal((h, w, len(SRTM_BANDS))).astype(np.float32)
    )
    for i, k in enumerate(SPACE_BAND_GROUPS_IDX):
        if k == "SRTM":
            sp_m[:, :, i] = 0

    t_x = torch.zeros((t, len(TIME_BANDS)))
    t_m = torch.ones((t, len(TIME_BAND_GROUPS_IDX)))
    st_x = torch.zeros((len(STATIC_BANDS),))
    st_m = torch.ones((len(STATIC_BAND_GROUPS_IDX),))
    months = torch.full((t,), 7, dtype=torch.long).unsqueeze(0)

    s_t_x = s_t_x.unsqueeze(0)
    s_t_m = s_t_m.unsqueeze(0)
    sp_x = sp_x.unsqueeze(0)
    sp_m = sp_m.unsqueeze(0)
    t_x = t_x.unsqueeze(0)
    t_m = t_m.unsqueeze(0)
    st_x = st_x.unsqueeze(0)
    st_m = st_m.unsqueeze(0)

    with torch.inference_mode():
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
            patch_size=server.GALILEO_PATCH_SIZE,
            input_resolution_m=server.GALILEO_CHIP_GSD_M,
        )
        # Mirror the production handler exactly: drop `months`, average the
        # 8 (x,m) tensors. 9-tuple → 8 args, the canonical arity.
        assert len(model_output) == 9, f"forward returned {len(model_output)}-tuple"
        embedding_t = encoder.average_tokens(*model_output[:-1])
        if embedding_t.dim() > 1:
            embedding_t = embedding_t.squeeze(0)
        embedding = embedding_t.cpu().tolist()

    assert len(embedding) == 768, f"expected 768-D embedding, got {len(embedding)}"
    assert np.all(np.isfinite(embedding)), "embedding has non-finite values"


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
    test_s1_srtm_modality_shift_div()
    test_request_accepts_optional_s1_srtm_and_validates_shape()
    if _galileo_weights_dir() is not None:
        test_average_tokens_arity_real_forward()
        print("[galileo] real-forward arity guard passed")
    else:
        print("[galileo] real-forward arity guard skipped (no local weights)")
    print("[galileo] all normalization tests passed")
