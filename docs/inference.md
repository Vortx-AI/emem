# emem inference

## What this layer does

Three frozen-pretrained encoders (Prithvi-EO-2.0, Galileo-Tiny, JEPA-v2 dynamics
baseline) and three explicit-method physics solvers (heat 2D, wave 1D, NDVI
AR(2)) running behind `/v1/jepa_predict_v2`, `/v1/jepa_predict`,
`/v1/heat_solve`, `/v1/wave_solve`. GPU work goes via a Python FastAPI sidecar
over a Unix domain socket so the Rust process keeps a small attack surface and
CUDA OOMs surface as 503s instead of crashing the API. The Rust caller's
fallback path is wired only for the closed-form solvers — for ViT-scale
encoders there is no in-process CPU rerun; the responder returns 502/503 and
lets the agent decide.

## Process topology

```
   ┌──────────────┐      Unix socket      ┌────────────────────┐
   │  emem-server │ ── /predict/* ──────► │ jepa-sidecar (py)  │
   │   (Rust)     │ ◄──── JSON ────────── │ FastAPI + uvicorn  │
   └──────────────┘                       │ ┌────────────────┐ │
   gpu_sidecar.rs                         │ │ Registry cache │ │
   handles HTTP/1.1 over UDS              │ │  ├─ Prithvi    │ │
   manually                               │ │  ├─ Galileo    │ │
                                          │ │  └─ JEPA v2    │ │
                                          │ └────────────────┘ │
                                          │ device = cuda|cpu  │
                                          │ vram cap = 10 GB   │
                                          └────────────────────┘
```

Two unrelated death modes are deliberately decoupled:

- Sidecar crash → `SidecarError::Unavailable` → in-process fallback (where
  wired) or honest 502/503 to the agent. emem-server keeps serving recall,
  receipts, and physics primitives that do not touch the sidecar.
- Sidecar refuses (CUDA OOM, missing checkpoint, blake2b mismatch) →
  `SidecarError::Upstream` → 502 to the agent. The Rust path MUST NOT silently
  fall back when the sidecar gave a structured rejection.

## Sidecar protocol

UDS path: `${XDG_RUNTIME_DIR}/emem/jepa_sidecar.sock` (typically
`/run/user/<UID>/emem/jepa_sidecar.sock`). The systemd unit asks for
`RuntimeDirectory=emem` so the directory is created and torn down with the
user session.

Endpoints:

| Method | Path                          | Request                                                    | Response                                                     |
|--------|-------------------------------|------------------------------------------------------------|--------------------------------------------------------------|
| GET    | `/health`                     | —                                                          | `models_loaded`, `device`, `cuda`, `uptime_s`, `pid`         |
| POST   | `/predict/dynamics_v2`        | `DynamicsRequest{lags:[3,128]}`                            | `DynamicsResponse{prediction:[128], model, inference_us, device}` |
| POST   | `/predict/prithvi_eo2_embed`  | `PrithviRequest{chip:[6,224,224], year?, julian_day?, lng?, lat?}` | `PrithviResponse{embedding:[1024], model, inference_us, device}` |
| POST   | `/predict/galileo_embed`      | `GalileoRequest{s2_chip:[1,8,8,10], month?, lng?, lat?}`   | `GalileoResponse{embedding:[192], model, inference_us, device}` |

Every response carries a `model` block containing `model_id`, `version`,
`blake2b_hex`, `via="python_sidecar"`, and `honesty_warnings`. The Rust
caller forwards this object verbatim into the signed receipt — the sidecar
is the single source of truth for what was actually executed. A verifier
re-deriving any signed prediction can hash the on-disk checkpoint with
blake2b-256 and compare to `model.blake2b_hex` byte for byte.

Wire shape, end to end (jepa_v2 example):

1. Agent calls `POST /v1/jepa_predict_v2` with `{cell64, ...}`.
2. emem-server's handler issues 3 internal recall calls for the latest
   geotessera vintages at that cell (these are themselves signed Primary
   facts).
3. The 3×128 fp32 lags are flattened, JSON-encoded, and posted to
   `/predict/dynamics_v2` over the UDS.
4. The sidecar's pydantic validator rejects mis-shaped input with 400.
   Otherwise the registry's cached `DynamicsModel` runs the forward pass.
5. The 128-D prediction returns over the same socket along with the
   `model` block.
6. emem-server signs the prediction as a Primary fact under
   `geotessera.predicted_<next_year>`, embeds the sidecar's `model`
   block in the receipt's derivation args, and returns it to the agent.

## Rust client (`crates/emem-api-rest/src/gpu_sidecar.rs`)

Hand-rolled HTTP/1.1 over the UDS. The client writes the request bytes, reads
the response until EOF (the request always sets `Connection: close`), then
parses status line + body delimiter. The reqwest crate's UDS support requires
a feature flag and a non-trivial connector chain; the sidecar parser is
small enough that the dependency is not worth the surface area.

Configuration:

- `EMEM_SIDECAR_SOCK` — default `/run/emem/jepa_sidecar.sock`.
- `EMEM_SIDECAR_TIMEOUT_MS` — default `5000`.

Error variants:

- `Unavailable(msg)` — connect failed, socket missing, or refused. The Rust
  caller is allowed to chain to a fallback on this variant.
- `Upstream { status, body }` — sidecar returned non-2xx. The caller surfaces
  this as 502 to the agent and does not silently retry without the GPU.
- `Timeout(d)` — the round-trip exceeded `EMEM_SIDECAR_TIMEOUT_MS`.
- `Protocol(msg)` — bad framing, non-UTF-8 status line, or malformed JSON
  body. Treat as 502.

## Models

### Prithvi-EO-2.0-300M-TL — frozen pretrained, no fine-tune

|                  |                                                                                       |
|------------------|---------------------------------------------------------------------------------------|
| HuggingFace      | `ibm-nasa-geospatial/Prithvi-EO-2.0-300M-TL`                                          |
| snapshot pin     | `63adbd39c271da4c42f447e69b1a7c91a338cdc9`                                            |
| input            | `[batch, T=1, H=224, W=224, bands=6]` HLS V2 (Blue, Green, Red, Narrow-NIR, SWIR1, SWIR2) |
| output           | 1024-D CLS token (last block, post-norm)                                              |
| cold             | ~10 s                                                                                 |
| warm             | ~20 ms                                                                                |
| receipt warning  | `frozen_pretrained_encoder`                                                           |

Why frozen: this responder does not fine-tune the encoder. The forward pass
captures whatever HLS V2 masked-autoencoder pretraining captured; quality on
any downstream task depends on how well the task aligns with that objective.
The receipt's `frozen_pretrained_encoder` warning makes that explicit so a
verifier reading the embedding does not assume task-aligned supervision.

Rust chip fetcher: `prithvi_chip.rs` assembles a 224×224 chip at uniform 30 m
sampling — physical extent **6720 m × 6720 m** centred on the cell:

- 10 m S2 bands (B02, B03, B04) — fetch a 672×672 window, mean-pool 3:1.
- 20 m S2 bands (B8A, B11, B12) — fetch a 336×336 window, bilinear 1.5:1.

The chip ships in the upstream's native scale (0–10000 nominal reflectance);
the sidecar applies the model's per-band mean/std normalization before the
forward pass. The receipt cites the source S2 scene's STAC item, the cloud
fraction at pick time, and the cloud/lookback tier the search settled on,
so an auditor can re-derive the chip from the same upstream window.

S2-L2A is a near-substitute for HLS V2 (same Sen2Cor lineage) but not
identical — small Landsat-9 cross-sensor harmonization terms are absent.
The receipt flags this as `s2_l2a_substitute_for_hls_v2`.

### Galileo Tiny — S2-only modality

|                  |                                                                          |
|------------------|--------------------------------------------------------------------------|
| HuggingFace      | `nasaharvest/galileo` (variant via `EMEM_GALILEO_VARIANT`, default `tiny`) |
| input            | `[batch=1, T=1, H=8, W=8, C=10]` (10 S2 bands)                           |
| output           | 192-D average-pool over unmasked tokens                                  |
| cold             | ~4 s                                                                     |
| warm             | ~14 ms                                                                   |
| receipt warning  | `frozen_pretrained_encoder`                                              |

S2 bands in canonical order: B2, B3, B4, B5, B6, B7, B8, B8A, B11, B12.
Native resolutions: 10 m for B2/B3/B4/B8; 20 m for B5/B6/B7/B8A/B11/B12.
The chip is sampled to a uniform 30 m / 8×8 grid — 240 m × 240 m extent —
via 24×24 mean-pool for 10 m bands and 12×12 bilinear for 20 m bands.

The Galileo encoder is multimodal (S1, ERA5, time, climate, VIIRS, SRTM,
DW, WorldCover, LandScan, location) but only the S2 modality is wired here.
The other modality tensors are zero-filled and their group masks stay at
1 (= "not seen by the encoder"). The encoder was trained with the full
mask-ratio schedule and accepts this configuration without retraining. The receipt's
`frozen_pretrained_encoder` warning explicitly notes that a full-multimodal
embedding would carry richer context — wiring more modalities is future work,
not a regression.

### JEPA-v2 dynamics — untrained baseline today

|                   |                                                                                                  |
|-------------------|--------------------------------------------------------------------------------------------------|
| architecture      | residual MLP (4 pre-LN blocks, dim=128, hidden=256, dropout=0.10)                                |
| input             | `[batch, 3, 128]` — three 128-D Tessera lags, oldest first                                       |
| output            | `last_vintage + delta` (zero-init head → identity baseline)                                      |
| on-disk           | `<EMEM_DATA>/jepa_v2/dynamics_v2.onnx` (~8 KB) plus `dynamics_v2.metadata.json`                  |
| inference         | ~50 µs CPU (ort) on the in-process Rust path; ~similar on sidecar CUDA                           |
| receipt warnings  | `untrained_baseline`, `upstream_geotessera_single_vintage`                                       |

Why untrained: training a residual head over Tessera embeddings needs at least
3 lags per cell. As of 2026-05-08 the public `dl2.geotessera.org` bucket only
serves the 2024 vintage reliably (verified live: 2017–2023 return null for
representative cells). All three lags fed into the model collapse to the same
2024 vector, so the head's contribution is degenerate by construction. The
training pipeline is ready (`assemble_data.py` + `train.py`, cosine + L2 loss,
`SEED=42`, `EPOCHS=200`, `BATCH=128`, `LR=3e-4`); the data is not.

The receipt's `untrained_baseline` warning is the only place an LLM reading
the prediction sees the disclosure, so the field is part of the wire contract
and the default is fail-safe: a metadata file that omits `training.trained`
returns `is_trained() == false` and the warning fires anyway. Real
`train.py` output MUST explicitly write `training.trained = true`.

#### Trained-checkpoint loader (`server.py:_Registry.load_dynamics`)

The loader is wired as of 2026-05-08, but no trained checkpoint exists yet.
When `metadata.training.trained == true`:

1. Look for `<EMEM_DATA>/jepa_v2/dynamics_v2.state_dict.pt` next to the
   metadata file. Refuse to load if missing.
2. If `metadata.training.checkpoint_blake2b_hex` (or
   `metadata.model.blake2b_hex`) is set, hash the `.pt` file with blake2b
   and refuse on mismatch. This closes the "swap a `.pt` under a stale
   `metadata.json` and serve a fresh receipt for old weights" hole.
3. `torch.load(path, map_location=device, weights_only=True)`. The
   checkpoint comes from the project's own training pipeline, but the
   sidecar treats any on-disk artifact as untrusted input.
4. `model.load_state_dict(state_dict, strict=True)` — any architecture drift
   between the trained tensors and the `DynamicsModel` class compiled into
   the sidecar surfaces at load time, not as garbage predictions at request
   time.

When `metadata.training.trained == false` (the current shipped state), the
loader rebuilds `DynamicsModel`, zeros the head's weight + bias, and the
residual sums to `last_vintage` exactly. The export script's
`max_diff < 1e-6` invariant guards this at build time.

Either way the receipt's `model.blake2b_hex` reports the on-disk file's
hash so a verifier can re-derive what was actually executed.

Operator workflow when a real checkpoint is ready:

1. Run `python python/jepa_v2/assemble_data.py` to assemble per-cell lags
   into `training_data.npz`. Requires ≥3 reachable Tessera vintages per
   cell — currently blocked on upstream.
2. Run `python python/jepa_v2/train.py`. The script writes the trained
   `dynamics_v2.onnx` (used by the in-process Rust ort path) and
   `dynamics_v2.state_dict.pt` (used by the sidecar PyTorch path) into
   `<EMEM_DATA>/jepa_v2/`. It MUST update `metadata.json` to set
   `training.trained = true` and pin
   `training.checkpoint_blake2b_hex` to the new `.pt` blake2b.
3. Restart the sidecar (`systemctl --user restart emem-jepa-sidecar`).
   The first prediction request triggers `load_dynamics`, which walks
   the trained-checkpoint guards above. If the metadata pin is wrong,
   the sidecar refuses to serve and the receipt under the old metadata
   is preserved.
4. Verifier flow: an agent reading a fresh receipt can re-hash the
   `.pt` and confirm the blake2b matches what the receipt's
   `model.blake2b_hex` claimed.

The 0.0.x receipt's `untrained_baseline` warning continues to fire until
the metadata is rotated to `trained=true`. There is no flag flip that
strips the warning without supplying real weights — that path is
deliberately closed.

## VRAM partitioning (`server.py:_Registry.__init__`)

`EMEM_SIDECAR_VRAM_BUDGET_GB` (default 10):

|                       |        |
|-----------------------|--------|
| `DYNAMICS_BUDGET_GB`  | 0.1    |
| `V_JEPA_2_BUDGET_GB`  | 3.0    |
| `PRITHVI_BUDGET_GB`   | 3.0    |
| reserve               | 3.9    |
| total                 | 10.0   |

`torch.cuda.set_per_process_memory_fraction(TOTAL_BUDGET_GB / device_total_gb)`
is called once at registry init. The per-model budget constants are advisory
accounting — they describe how the cap is allocated, not separate hard caps.
A future allocation that would push the process past the global cap raises
CUDA OOM, which the sidecar surfaces as 503 to Rust.

The `V_JEPA_2_BUDGET_GB` slot is reserved for the eventual V-JEPA 2 model.
The current Phase 3 deployment is Prithvi-EO-2.0; V-JEPA 2 was dropped as
the Phase 3 model (it is a video transformer with 64×256² tubelets — the
wrong fit for annual per-cell embeddings). The budget slot stays so we
don't have to re-balance constants if we later land a video predictor.

## Physics solvers (`crates/emem-api-rest/src/physics.rs`)

These run in-process. The Rust caller does not touch the sidecar; CFL stability
is checked at request time and the receipt cites every fact CID that fed
the discretisation.

### `/v1/heat_solve` — 2D explicit FTCS

|                  |                                                                  |
|------------------|------------------------------------------------------------------|
| input            | `cell, hours_ahead` (cap 168), `diffusivity_m2_per_s` (default 1.0e-6) |
| stencil          | 3×3 MODIS `lst_day_8day` (NW, N, NE, W, centre, E, SW, S, SE)    |
| pitch            | 10 m grid                                                        |
| CFL              | safety 0.20 (max stable 0.25 for 2D heat)                        |
| diagnostics      | uniform-stencil detection, imputed-neighbour count               |

The 5-point Laplacian update at every step is

    u_new = u + α·Δt·(N + S + E + W − 4·centre)/Δx²

with Dirichlet boundaries (corner stencils stay pinned to their initial
condition). Horizon ≤168 h and step count ≤2M iterations — both caps are
defensive against an agent passing a tiny α and a long horizon that would
spend minutes in the handler.

When the 3×3 stencil's range is below 0.01 K (well under the MODIS LST
instrument noise floor of ~0.5 K) the response flags `is_uniform=true`
and the interpretation distinguishes a real "no diffusion expected"
outcome from a stencil that collapsed because the upstream materialiser
populated all 9 cells from one coarser source pixel.

### `/v1/wave_solve` — 1D shallow water

|                            |                                                                           |
|----------------------------|---------------------------------------------------------------------------|
| input                      | `coastal_cell, swell_height_m, swell_period_s`                            |
| profile                    | walk seaward (cardinal-only), pick deepest GMRT cell, build ≥3-cell profile, reverse |
| land-locked rejection      | offshore depth ≥ 5 m AND ≥50% of profile > 1 m                            |
| boundary, offshore         | `H_s · sin(2π·t/T)` (driven swell)                                        |
| boundary, coastal          | hard wall (u = 0)                                                         |
| wave speed                 | `c = √(g·h)`, floored at 0.01 m                                           |
| CFL                        | safety 0.5                                                                |

The land-locked check is intentionally strict. A profile that fails BOTH
criteria (offshore boundary depth ≥ 5 m AND at least half the profile > 1 m)
returns 422 with the actual GMRT depths attached and a hint
(`try_longer_profile` or `try_different_cell`) so an agent can iterate.
Returning a fabricated "wave" for an inland cell would be exactly the kind
of silent hallucination the protocol's honesty culture refuses.

### `/v1/jepa_predict` — closed-form NDVI AR(2)

Coefficients (`physics.rs:1165-1167`): α=0.6, β=0.3, γ=0.1.

    pred = clamp(α · lag_12 + β · (last + trend) + γ · recent_mean, [-1.0, 1.0])

`trend` is the least-squares slope through the last `lookback_months` NDVI
samples. `lag_12` is the same-month value one year ago when the lookback
window includes it; otherwise the α term degrades to `recent_mean`. Default
lookback is 6 months; cap is 24.

This is NOT a learned model. The receipt does not carry `trained` claims
and the response does not pretend to a forecast quality the math cannot
support. It is published as a real, reproducible AR(2) seasonal predictor
with stated coefficients — agents can reimplement it from the receipt alone.

### `/v1/jepa_predict_v2` — Tessera dynamics via sidecar

The handler pulls the 3 latest geotessera vintages at the cell, stacks
them `[1, 3, 128]`, calls `predict_dynamics_v2` on the sidecar, and signs
the resulting 128-D vector as a Primary fact under a future-dated
`geotessera.predicted_<year>` band. The receipt always carries
`untrained_baseline` until a trained checkpoint exists; see the trained-
checkpoint loader section above for what changes when one ships.

## Operational notes

### systemd

`emem-jepa-sidecar.service` is a user unit at
`python/jepa_v2_sidecar/emem-jepa-sidecar.service`. `emem-server.service`
declares `Wants=` and `After=` on it so:

1. Starting `emem-server` pulls the sidecar up.
2. The sidecar's socket is guaranteed to exist before emem-server's first
   request.

A sidecar crash does NOT cascade-stop emem-server. The wired
`/v1/jepa_predict_v2` falls back to the in-process Rust ort path
(closed-form solvers also run in-process unconditionally). For Prithvi
and Galileo, in-process CPU inference at ViT scale is not viable —
recall on those bands returns existing attestations only, and a fresh
materialisation request returns 503 with the sidecar's own error body.

### Cold-start

The first `/predict/prithvi_eo2_embed` after sidecar restart costs ~10 s
while the 1.24 GB checkpoint streams from `<EMEM_DATA>/hf_cache/`.
Subsequent calls are warm. For an agent batching predictions the
recommendation is: send one synthetic warm-up request after the sidecar's
`/health` reports it is reachable, then pipeline the real ones.

Galileo Tiny is ~22 MB on disk and ~4 s cold; warm calls are ~14 ms.

Cold-start budget breakdown (Prithvi):

| stage                                             | wall time |
|---------------------------------------------------|-----------|
| `import torch` + cuda context                     | ~1.5 s    |
| `import prithvi_mae`, build `PrithviMAE` skeleton | ~0.5 s    |
| `torch.load(weights_only=True)` 1.24 GB           | ~5–7 s    |
| `load_state_dict` + `to(device)` + `eval()`       | ~1–2 s    |
| first forward pass (compile + warm caches)        | ~50 ms    |

The model stays resident for the lifetime of the sidecar process — the
operator instruction is "no offload after restart". Process reload is the
only way to re-attempt a load that failed.

### `/health` semantics

`GET /health` is non-mutating and does not load any model. It reports:

- `status` — always `"ok"` if the process is up.
- `models_loaded` — list of model_ids whose registry slot is populated.
  Empty until the first prediction request lazy-loads.
- `device` — `cuda:0` when available, else `cpu`.
- `cuda` — full memory accounting block (total GB, used by all processes,
  this-process allocated/reserved, budget remaining for this process).
- `uptime_s`, `pid`.

Use `/health` from emem-server's startup probe. Do NOT use it as a
warm-up — it deliberately does not trigger model load.

### Offline mode

The systemd unit sets `HF_HOME=<EMEM_DATA>/hf_cache` and pre-pins both
the Prithvi snapshot path and the Galileo snapshot path. `HF_HUB_OFFLINE=1`
is currently commented out in the unit file; uncomment after Phase 3b
weights are pre-cached on the host so the sidecar refuses any network
fetch.

### Receipt's `model.via` field

| `via`              | meaning                                                        |
|--------------------|----------------------------------------------------------------|
| `python_sidecar`   | the sidecar served the prediction                              |
| `in_process_cpu`   | Rust fallback ran the prediction (closed-form solvers only today) |
| (warning surfaced) | `untrained_baseline` indicates the residual-zero sentinel; the field is in `model.honesty_warnings`, not `via` |

## Failure modes

| trigger                                              | error                              | status to agent | recovery                                |
|------------------------------------------------------|------------------------------------|-----------------|------------------------------------------|
| socket missing or refused                            | `SidecarError::Unavailable`        | 503 (or fallback if wired) | restart sidecar / wait for systemd        |
| sidecar 5xx with body                                | `SidecarError::Upstream`           | 502             | inspect sidecar log; do not auto-retry  |
| `EMEM_SIDECAR_TIMEOUT_MS` exceeded                   | `SidecarError::Timeout`            | 504-shaped 502  | bump timeout for cold-start workloads   |
| CUDA OOM inside sidecar                              | uvicorn 503 → `Upstream`           | 502             | reduce batch / wait for other tenants   |
| metadata claims `trained=true` but `.pt` missing     | `FileNotFoundError` → 503 → `Upstream` | 502         | restore the checkpoint or reset metadata |
| `.pt` blake2b mismatches metadata pin                | `RuntimeError` → 503 → `Upstream`  | 502             | rerun `export_baseline.py` / `train.py` |
| `load_state_dict(strict=True)` rejects weight shapes | `RuntimeError` → 503 → `Upstream`  | 502             | architecture drift; rebuild checkpoint  |

The trained-checkpoint failure modes are deliberately load-time, not
inference-time. A serving sidecar that refuses to answer is a better
failure than a serving sidecar that answers with random-init outputs
under a `trained=true` receipt.

### What the in-process Rust ort path does on jepa_v2

`crates/emem-api-rest/src/jepa_v2.rs` holds a parallel runtime to the
sidecar's PyTorch path: it loads `dynamics_v2.onnx` (not the `.pt`)
through ort's CPU execution provider, locks the session in a `Mutex`,
and runs the `[1, 3, 128]` tensor through the same architecture. The
two paths are byte-identical on the zero-init sentinel — the export
script's `max_diff < 1e-6` invariant is the contract.

The Rust path checks `metadata.artifact.size_bytes` against the on-disk
ONNX file size and refuses to load on mismatch. There is no separate
`strict=True` analogue at this layer because ort itself enforces input
shape; a drifted ONNX would fail at `commit_from_file` with a structured
error.

The Rust path does NOT load `dynamics_v2.state_dict.pt`; that file is
PyTorch-specific. When a real trained run ships, both paths must be
re-exported in lockstep — `train.py` writes both the ONNX (for ort) and
the state_dict (for the sidecar) and pins the same blake2b on each in
metadata.

---

See `docs/agents.md` for the call patterns an agent uses to drive these
endpoints, and `docs/protocol.md` for how the receipt's `model` block
fits into the wire shape the verifier consumes.
