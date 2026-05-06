# jepa_v2_sidecar — GPU inference service for emem

A FastAPI process that holds GPU-resident models (V-JEPA 2, Prithvi,
…) and exposes them over a Unix socket. emem-server (Rust) calls in.

## Why a sidecar

The libonnxruntime-1.22-cuda12 binary at /opt deadlocks at session
create on this host (verified 2026-05-06 with isolated reproducer). A
Python sidecar sidesteps the in-process ort + CUDA path entirely while
keeping the Rust orchestrator + protocol logic untouched.

## API

All endpoints are POST `application/json`, served over Unix socket
`/run/emem/jepa_sidecar.sock` (or `EMEM_SIDECAR_SOCK` if set).

- `GET  /health` → `{"status":"ok", "models":[...], "vram":{...}}`
- `POST /predict/dynamics_v2` body `{"lags":[[...128];3]}` → `{"prediction":[...128], "model":{...}}`
  - Phase 1's small dynamics MLP. Mirrors the Rust ort path so an
    operator can A/B Rust-CPU vs Python-GPU on the same input.
- `POST /predict/v_jepa_2` (Phase 3) body `{"frames_url":"..."}` → `{"prediction":[...], "model":{...}}`
- `POST /predict/prithvi_eo2` (Phase 4) body `{"hls_window":...}` → `{...}`

## VRAM budget

Hard ceiling: **10 GB** (per operator's instruction; the A100 has
40 GB total but is shared with other workloads). Each model session
declares `torch.cuda.set_per_process_memory_fraction()` proportional
to its budget so an over-allocating bug fails fast instead of
clobbering geoqa-models / intruder.

| Model | Budget | Notes |
|---|---|---|
| dynamics_v2 (Phase 1 mirror) | <100 MB | tiny MLP; CUDA only for parity |
| V-JEPA 2 ViT-L (Phase 3) | 3 GB | weights + activations |
| Prithvi-EO-2.0-300M (Phase 4) | 3 GB | weights + activations |
| Reserve (peaks) | 4 GB | |

## Deployment

systemd user unit `emem-jepa-sidecar.service` (Phase 2 ships the
unit file). Starts before `emem-server.service`. Restart policy
`Restart=on-failure`, `RestartSec=10`.

## Local dev

```
.venv/bin/python -m uvicorn server:app \
  --uds /tmp/emem-jepa.sock \
  --log-level info
```

Then from another shell:

```
curl --unix-socket /tmp/emem-jepa.sock http://localhost/health
```
