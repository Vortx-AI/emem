# emem inference

## What this layer does

Four foundation encoders co-resident on a 20 GB VRAM budget — Clay
v1.5, Prithvi-EO-2.0-300M-TL, Galileo (variant via
`EMEM_GALILEO_VARIANT`, default `base`), JEPA v2 (untrained
baseline) — plus three explicit-method physics solvers (heat 2D,
wave 1D, NDVI AR(2)) running behind `/v1/jepa_predict_v2`,
`/v1/jepa_predict`, `/v1/heat_solve`, `/v1/wave_solve`. GPU work goes
via a Python FastAPI sidecar over a Unix domain socket so the Rust
process keeps a small attack surface and CUDA OOMs surface as 503
instead of crashing the API. The Rust caller's fallback path is wired
for the closed-form solvers and for JEPA v2's untrained short-circuit;
for the ViT-scale encoders there is no in-process CPU rerun.

## Process topology

```
   ┌──────────────┐      Unix socket      ┌────────────────────┐
   │  emem-server │ ── /predict/* ──────► │ jepa-sidecar (py)  │
   │   (Rust)     │ ◄──── JSON ────────── │ FastAPI + uvicorn  │
   └──────────────┘                       │ ┌────────────────┐ │
   gpu_sidecar.rs                         │ │ Registry cache │ │
   handles HTTP/1.1 over UDS              │ │  ├─ Clay v1.5  │ │
   manually                               │ │  ├─ Prithvi    │ │
                                          │ │  ├─ Galileo    │ │
                                          │ │  └─ JEPA v2    │ │
                                          │ └────────────────┘ │
                                          │ device = cuda|cpu  │
                                          │ vram cap = 20 GB   │
                                          └────────────────────┘
```

Two unrelated death modes are decoupled:

- Sidecar crash → `SidecarError::Unavailable` → in-process fallback
  (where wired) or honest 502/503 to the agent. `emem-server` keeps
  serving recall, receipts, and physics primitives that do not touch
  the sidecar.
- Sidecar refuses (CUDA OOM, missing checkpoint, blake2b mismatch) →
  `SidecarError::Upstream` → 502 to the agent. The Rust path MUST NOT
  silently fall back when the sidecar gave a structured rejection.

## Sidecar protocol

UDS path: `${XDG_RUNTIME_DIR}/emem/jepa_sidecar.sock` (typically
`/run/user/<UID>/emem/jepa_sidecar.sock`). The systemd unit asks for
`RuntimeDirectory=emem` so the directory is created and torn down
with the user session.

Endpoints:

| Method | Path                          | Request                                                              | Response                                                          |
|--------|-------------------------------|----------------------------------------------------------------------|-------------------------------------------------------------------|
| GET    | `/health`                     | —                                                                    | `models_loaded`, `device`, `cuda`, `uptime_s`, `pid`              |
| POST   | `/predict/clay_embed`         | `ClayRequest{chip_bands: {key→[H,W]}, lng?, lat?, capture_date?}`    | `ClayResponse{embedding:[1024], model, inference_us, device}`     |
| POST   | `/predict/prithvi_eo2_embed`  | `PrithviRequest{chip:[6,224,224], year?, julian_day?, lng?, lat?}`   | `PrithviResponse{embedding:[1024], model, inference_us, device}`  |
| POST   | `/predict/galileo_embed`      | `GalileoRequest{s2_chip:[1,8,8,10], month?, lng?, lat?}`             | `GalileoResponse{embedding:[D], model, inference_us, device}` where `D` depends on the loaded variant (`EMEM_GALILEO_VARIANT`) |
| POST   | `/predict/dynamics_v2`        | `DynamicsRequest{lags:[3,128]}`                                      | `DynamicsResponse{prediction:[128], model, inference_us, device}` |

Every response carries a `model` block containing `model_id`,
`version`, `blake2b_hex`, `via="python_sidecar"`, and
`honesty_warnings`. The Rust caller forwards this object verbatim
into the signed receipt — the sidecar is the single source of truth
for what was executed. A verifier re-deriving any signed prediction
hashes the on-disk checkpoint with blake2b-256 and compares to
`model.blake2b_hex` byte-for-byte.

Wire shape, end to end (jepa_v2 example):

1. Agent calls `POST /v1/jepa_predict_v2` with `{cell64, ...}`.
2. The handler issues 3 internal recall calls for the latest
   geotessera vintages at that cell (these are themselves signed
   Primary facts).
3. The 3×128 fp32 lags flatten, JSON-encode, and post to
   `/predict/dynamics_v2` over the UDS — **except** when
   `is_trained() == false`, in which case the v2 handler
   short-circuits before the network call (see §JEPA v2 below).
4. The sidecar's pydantic validator rejects mis-shaped input with
   400. Otherwise the registry's cached `DynamicsModel` runs the
   forward pass.
5. The 128-D prediction returns over the same socket along with the
   `model` block.
6. `emem-server` signs the prediction as a Primary fact under
   `geotessera.predicted_<next_year>`, embeds the sidecar's `model`
   block in the receipt's derivation args, and returns it.

## Rust client (`crates/emem-api-rest/src/gpu_sidecar.rs`)

Hand-rolled HTTP/1.1 over the UDS. The client writes the request
bytes, reads the response until EOF (the request always sets
`Connection: close`), then parses status line + body delimiter. The
reqwest crate's UDS support requires a feature flag and a non-trivial
connector chain; the sidecar parser is small enough that the
dependency is not worth the surface area.

Configuration:

- `EMEM_SIDECAR_SOCK` — default `/run/emem/jepa_sidecar.sock`. The
  user-mode systemd unit sets `%t/emem/jepa_sidecar.sock` (resolves
  to `/run/user/<UID>/emem/jepa_sidecar.sock`); production must set
  the env explicitly.
- `EMEM_SIDECAR_TIMEOUT_MS` — default `5000`.

Error variants:

- `Unavailable(msg)` — connect failed, socket missing, or refused.
  The Rust caller chains to a fallback on this variant.
- `Upstream { status, body }` — sidecar returned non-2xx. Surfaces as
  502 to the agent; does not silently retry without the GPU.
- `Timeout(d)` — round-trip exceeded `EMEM_SIDECAR_TIMEOUT_MS`.
- `Protocol(msg)` — bad framing, non-UTF-8 status line, or malformed
  JSON body. Treat as 502.

## Models

### Clay v1.5 — wavelength-conditioned ViT-L/8

|                   |                                                                                  |
|-------------------|----------------------------------------------------------------------------------|
| HuggingFace       | `made-with-clay/Clay` (`v1.5/clay-v1.5.ckpt`, Apache-2.0)                        |
| pinned via        | `Clay-foundation/model` git package SHA `f14e698f3c237cabf8d28dec669a362d66625381` |
| input             | `[B, C, 256, 256]` wavelength-conditioned (S1 / S2 / Landsat / NAIP / multi-sensor) |
| output            | 1024-D CLS token (last block, post-norm)                                         |
| cold              | ~6 s                                                                             |
| warm              | ~18 ms                                                                           |
| disk              | ~1.4 GB checkpoint                                                               |
| receptive field   | ~2.56 km at 10 m S2                                                              |
| receipt warning   | `frozen_pretrained_encoder`                                                      |

Clay v1.5 is a ViT-L/8 trained with MAE + a DINOv2 teacher. The
sidecar's `load_clay` reads `metadata.yaml` from
`python/jepa_v2_sidecar/clay_metadata.yaml` for the per-band
wavelength + mean + std priors; the chip ships in native upstream
scale and the encoder applies normalization internally.

Why frozen: this responder does not fine-tune. The CLS token captures
whatever the MAE + DINOv2 objective captured; downstream quality
depends on task alignment. The
`frozen_pretrained_encoder` warning makes the disclosure explicit.

Rust chip fetcher: `clay_chip.rs` assembles the wavelength-keyed band
dictionary at uniform 10 m sampling. The dolls embedding configuration
ships sizes `[16, 32, 64, 128, 256, 768, 1024]`; the default 1024-D
output is the post-norm CLS at position 0.

### Prithvi-EO-2.0-300M-TL — multi-temporal HLS MAE

|                  |                                                                                       |
|------------------|---------------------------------------------------------------------------------------|
| HuggingFace      | `ibm-nasa-geospatial/Prithvi-EO-2.0-300M-TL`                                          |
| snapshot pin     | `63adbd39c271da4c42f447e69b1a7c91a338cdc9`                                            |
| input            | `[B, T=1, H=224, W=224, bands=6]` HLS V2 (Blue, Green, Red, Narrow-NIR, SWIR1, SWIR2) |
| output           | 1024-D CLS token (last block, post-norm)                                              |
| cold             | ~10 s                                                                                 |
| warm             | ~20 ms                                                                                |
| disk             | ~1.24 GB checkpoint                                                                   |
| receptive field  | ~6.7 km (chip-scale, 30 m × 224)                                                      |
| receipt warning  | `frozen_pretrained_encoder`                                                           |

Why frozen: as with Clay, this responder does not fine-tune the
encoder. The forward pass captures whatever HLS V2 masked-autoencoder
pretraining captured.

Rust chip fetcher: `prithvi_chip.rs` assembles a 224×224 chip at
uniform 30 m sampling — physical extent **6720 m × 6720 m** centred
on the cell:

- 10 m S2 bands (B02, B03, B04) — fetch a 672×672 window, mean-pool 3:1.
- 20 m S2 bands (B8A, B11, B12) — fetch a 336×336 window, bilinear 1.5:1.

The chip ships in upstream native scale (0-10000 nominal
reflectance); the sidecar applies the model's per-band mean / std
normalization. The receipt cites the source S2 scene's STAC item,
the cloud fraction at pick time, and the cloud / lookback tier the
search settled on. S2-L2A is a near-substitute for HLS V2 (same
Sen2Cor lineage) but not identical — small Landsat-9 cross-sensor
harmonization terms are absent. The receipt flags this as
`s2_l2a_substitute_for_hls_v2`.

### Galileo — S2-only modality wired

|                  |                                                                          |
|------------------|--------------------------------------------------------------------------|
| HuggingFace      | `nasaharvest/galileo` (variant via `EMEM_GALILEO_VARIANT`, default `base`; override to `tiny` / `nano`) |
| input            | `[batch=1, T=1, H=8, W=8, C=10]` (10 S2 bands)                           |
| output           | tiny: **192-D** (avg-pool over unmasked tokens) · base: 768-D            |
| cold             | tiny ~4 s · base ~10 s                                                   |
| warm             | tiny ~14 ms · base ~25 ms                                                |
| disk             | tiny ~22 MB · base ~330 MB                                               |
| receipt warning  | `frozen_pretrained_encoder`                                              |

S2 bands in canonical order: B2, B3, B4, B5, B6, B7, B8, B8A, B11,
B12. Native resolutions: 10 m for B2/B3/B4/B8; 20 m for
B5/B6/B7/B8A/B11/B12. The chip is sampled to a uniform 30 m / 8×8
grid — 240 m × 240 m extent — via 24×24 mean-pool for 10 m bands and
12×12 bilinear for 20 m bands.

The Galileo encoder is multimodal (S1, ERA5, time, climate, VIIRS,
SRTM, DW, WorldCover, LandScan, location) but only the S2 modality
is wired here. The other modality tensors are zero-filled and their
group masks stay at 1 (= "not seen by the encoder"). The encoder was
trained with the full mask-ratio schedule and accepts this
configuration without retraining.

### JEPA v2 dynamics — untrained baseline today

|                   |                                                                                                  |
|-------------------|--------------------------------------------------------------------------------------------------|
| architecture      | residual MLP (4 pre-LN blocks, dim=128, hidden=256, dropout=0.10)                                |
| input             | `[batch, 3, 128]` — three 128-D Tessera lags, oldest first                                       |
| output            | `last_vintage + delta` (zero-init head → identity baseline)                                      |
| on-disk           | `<EMEM_DATA>/jepa_v2/dynamics_v2.onnx` (~8 KB) plus `dynamics_v2.metadata.json`                  |
| inference         | short-circuits when untrained (no ORT or sidecar call); ~50 µs ORT CPU when a trained `.onnx` ships |
| receipt warnings  | `untrained_baseline`, `upstream_geotessera_single_vintage`                                        |

The v2 handler reads `is_trained()` from a metadata-only cache
(`ensure_metadata`) before spinning up the ONNX session or hitting
the sidecar. When `training.trained == false` (the shipped state in
0.0.6) the handler returns `last_input_vintage` directly and attaches
both honesty warnings on the receipt. This costs nothing — no ONNX
init, no sidecar round-trip — and makes the untrained baseline
indistinguishable in cost from a pure recall.

Training is gated on upstream Tessera publishing ≥ 3 vintages per
cell. The upstream `dl2.geotessera.org` bucket ships eight annual
vintages (2017-2024) and the responder materialises them as
`geotessera.{2017..2024}` plus the 1024-D `geotessera.multi_year`
stack, but most cells in `/v1/coverage` have only the latest vintage
attested locally. The training pipeline is ready
(`python/jepa_v2/assemble_data.py` + `train.py`, smooth-L1 + cosine
loss, K=5 + positional encoding, cell-level split, `SEED=42`,
`EPOCHS=200`, `BATCH=128`, `LR=3e-4`); the candidate pool is the
bottleneck.

The receipt's `untrained_baseline` warning is the only place an LLM
reading the prediction sees the disclosure, so the field is part of
the wire contract. The default is fail-safe: a metadata file that
omits `training.trained` returns `is_trained() == false` and the
warning fires anyway. Real `train.py` output MUST explicitly write
`training.trained = true`.

#### Trained-checkpoint loader (`server.py:_Registry.load_dynamics`)

Wired since 2026-05-08; no trained checkpoint exists yet. When
`metadata.training.trained == true`:

1. Look for `<EMEM_DATA>/jepa_v2/dynamics_v2.state_dict.pt` next to
   the metadata file. Refuse to load if missing.
2. If `metadata.training.checkpoint_blake2b_hex` (or
   `metadata.model.blake2b_hex`) is set, hash the `.pt` file with
   blake2b and refuse on mismatch. Closes the "swap a `.pt` under a
   stale `metadata.json` and serve a fresh receipt for old weights"
   hole.
3. `torch.load(path, map_location=device, weights_only=True)`. The
   checkpoint comes from the project's own training pipeline, but
   the sidecar treats any on-disk artifact as untrusted input.
4. `model.load_state_dict(state_dict, strict=True)` — any architecture
   drift between trained tensors and the `DynamicsModel` class
   surfaces at load time, not as garbage predictions at request time.

When `metadata.training.trained == false` the loader rebuilds
`DynamicsModel`, zeros the head's weight + bias, and the residual
sums to `last_vintage` exactly. The export script's
`max_diff < 1e-6` invariant guards this at build time.

Either way the receipt's `model.blake2b_hex` reports the on-disk
file's hash so a verifier re-derives what was actually executed.

Operator workflow when a real checkpoint is ready:

1. `python python/jepa_v2/assemble_data.py` → `training_data.npz`.
   Requires ≥ 3 reachable Tessera vintages per cell.
2. `python python/jepa_v2/train.py` writes `dynamics_v2.onnx` (used
   by the Rust ORT path) and `dynamics_v2.state_dict.pt` (used by
   the sidecar PyTorch path) into `<EMEM_DATA>/jepa_v2/`. It MUST
   update `metadata.json` to set `training.trained = true` and pin
   `training.checkpoint_blake2b_hex` to the new `.pt` blake2b.
3. Restart the sidecar (`systemctl --user restart emem-jepa-sidecar`).
4. Verifier flow: an agent reading a fresh receipt re-hashes the
   `.pt` and confirms blake2b matches the receipt's
   `model.blake2b_hex`.

The 0.0.x receipt's `untrained_baseline` warning continues to fire
until the metadata rotates to `trained=true`. No flag flip strips
the warning without supplying real weights.

## VRAM partitioning (`server.py:_Registry.__init__`)

`EMEM_SIDECAR_VRAM_BUDGET_GB` default 20 (the systemd user unit sets
this explicitly):

|                       |        |
|-----------------------|--------|
| `DYNAMICS_BUDGET_GB`  | 0.1    |
| `V_JEPA_2_BUDGET_GB`  | 3.0    |
| `PRITHVI_BUDGET_GB`   | 3.0    |
| `CLAY_BUDGET_GB`      | 2.5    |
| reserve               | 11.4   |
| total                 | 20.0   |

`torch.cuda.set_per_process_memory_fraction(TOTAL_BUDGET_GB / device_total_gb)`
is called once at registry init. Per-model budget constants are
advisory accounting — they describe how the cap is allocated, not
separate hard caps. A future allocation that would push the process
past the global cap raises CUDA OOM, which the sidecar surfaces as
503 to Rust.

## Physics solvers (`crates/emem-api-rest/src/physics.rs`)

These run in-process. The Rust caller does not touch the sidecar;
CFL stability is checked at request time and the receipt cites every
fact CID that fed the discretisation.

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

with Dirichlet boundaries. Horizon ≤ 168 h and step count ≤ 2 × 10⁶
iterations; both caps are defensive against an agent passing a tiny
α and a long horizon that would spend minutes in the handler.

When the 3×3 stencil's range is below 0.01 K (well under the MODIS
LST instrument noise floor of ~0.5 K) the response flags
`is_uniform=true` and distinguishes a real "no diffusion expected"
outcome from a stencil that collapsed because the upstream
materialiser populated all 9 cells from one coarser source pixel.

### `/v1/wave_solve` — 1D shallow water

|                            |                                                                           |
|----------------------------|---------------------------------------------------------------------------|
| input                      | `coastal_cell, swell_height_m, swell_period_s`                            |
| profile                    | walk seaward (cardinal-only), pick deepest GMRT cell, build ≥ 3-cell profile, reverse |
| land-locked rejection      | offshore depth ≥ 5 m AND ≥ 50 % of profile > 1 m                          |
| boundary, offshore         | `H_s · sin(2π·t/T)` (driven swell)                                        |
| boundary, coastal          | hard wall (u = 0)                                                         |
| wave speed                 | `c = √(g·h)`, floored at 0.01 m                                           |
| CFL                        | safety 0.5                                                                |

A profile that fails the land-locked check returns 422 with the
actual GMRT depths attached and a hint (`try_longer_profile` or
`try_different_cell`) so an agent can iterate. Returning a fabricated
"wave" for an inland cell is the kind of silent hallucination the
protocol refuses.

### `/v1/jepa_predict` — closed-form NDVI AR(2)

Coefficients (`physics.rs:1165-1167`): α=0.6, β=0.3, γ=0.1.

    pred = clamp(α · lag_12 + β · (last + trend) + γ · recent_mean, [-1.0, 1.0])

`trend` is the least-squares slope through the last `lookback_months`
NDVI samples. `lag_12` is the same-month value one year ago when the
lookback window includes it; otherwise the α term degrades to
`recent_mean`. Default lookback is 6 months; cap is 24.

This is NOT a learned model. The receipt does not carry `trained`
claims and the response does not pretend to a forecast quality the
math cannot support. It is published as a reproducible AR(2)
seasonal predictor with stated coefficients — agents can reimplement
it from the receipt alone.

### `/v1/jepa_predict_v2` — Tessera dynamics via sidecar

The handler pulls the 3 latest geotessera vintages at the cell,
stacks them `[1, 3, 128]`, and either:

- short-circuits when `is_trained() == false` and returns
  `last_input_vintage` directly with `untrained_baseline` and
  `upstream_geotessera_single_vintage` honesty warnings, or
- calls `predict_dynamics_v2` on the sidecar (or falls back to the
  in-process ORT CPU path) and signs the resulting 128-D vector.

The signed fact lives under a future-dated
`geotessera.predicted_<year>` band.

## Operational notes

### systemd

`emem-jepa-sidecar.service` is a user unit at
`python/jepa_v2_sidecar/emem-jepa-sidecar.service`.
`emem-server.service` declares `Wants=` and `After=` on it so:

1. Starting `emem-server` pulls the sidecar up.
2. The sidecar's socket exists before emem-server's first request.

A sidecar crash does NOT cascade-stop emem-server. The wired
`/v1/jepa_predict_v2` falls back to the in-process Rust ORT path
(closed-form solvers run in-process unconditionally). For Clay,
Prithvi, and Galileo, in-process CPU inference at ViT scale is not
viable — recall on those bands returns existing attestations only,
and a fresh materialisation request returns 503 with the sidecar's
error body.

### Cold-start

The first `/predict/clay_embed` or `/predict/prithvi_eo2_embed`
after sidecar restart costs the model's documented cold latency
while the checkpoint streams from `<EMEM_DATA>/hf_cache/`. Subsequent
calls are warm. For an agent batching predictions: send one
synthetic warm-up after the sidecar's `/health` reports it is
reachable, then pipeline real calls.

Cold-start budget breakdown (Prithvi):

| stage                                             | wall time |
|---------------------------------------------------|-----------|
| `import torch` + cuda context                     | ~1.5 s    |
| `import prithvi_mae`, build `PrithviMAE` skeleton | ~0.5 s    |
| `torch.load(weights_only=True)` 1.24 GB           | ~5-7 s    |
| `load_state_dict` + `to(device)` + `eval()`       | ~1-2 s    |
| first forward pass (compile + warm caches)        | ~50 ms    |

The model stays resident for the lifetime of the sidecar process.
Process reload is the only way to re-attempt a load that failed.

### `/health` semantics

`GET /health` is non-mutating and does not load any model. It reports:

- `status` — always `"ok"` if the process is up.
- `models_loaded` — list of model_ids whose registry slot is populated.
- `device` — `cuda:0` when available, else `cpu`.
- `cuda` — full memory accounting block (total GB, used by all
  processes, this-process allocated / reserved, budget remaining).
- `uptime_s`, `pid`.

Use `/health` from emem-server's startup probe. Do NOT use it as a
warm-up — it deliberately does not trigger model load.

### Offline mode

The systemd unit sets `HF_HOME=<EMEM_DATA>/hf_cache` and pre-pins
the model snapshot paths. `HF_HUB_OFFLINE=1` is currently commented
out in the unit; uncomment after all weights are pre-cached on the
host so the sidecar refuses any network fetch.

### Receipt's `model.via` field

| `via`              | meaning                                                        |
|--------------------|----------------------------------------------------------------|
| `python_sidecar`   | the sidecar served the prediction                              |
| `in_process_cpu`   | Rust fallback ran the prediction (closed-form solvers + JEPA v2 ORT) |
| `short_circuit`    | JEPA v2 untrained baseline — no inference tier touched         |

`untrained_baseline` is a `model.honesty_warnings` entry, not a `via`
value; it can co-exist with any `via`.

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

Trained-checkpoint failure modes are deliberately load-time, not
inference-time. A serving sidecar that refuses to answer is a better
failure than a serving sidecar that answers with random-init outputs
under a `trained=true` receipt.

### What the in-process Rust ORT path does on jepa_v2

`crates/emem-api-rest/src/jepa_v2.rs` holds a parallel runtime to the
sidecar's PyTorch path: it loads `dynamics_v2.onnx` (not the `.pt`)
through ORT's CPU execution provider, locks the session in a
`Mutex`, and runs the `[1, 3, 128]` tensor through the same
architecture. The two paths are byte-identical on the zero-init
sentinel — the export script's `max_diff < 1e-6` invariant is the
contract.

The Rust path checks `metadata.artifact.size_bytes` against the
on-disk ONNX file size and refuses to load on mismatch. There is no
separate `strict=True` analogue because ORT itself enforces input
shape; a drifted ONNX fails at `commit_from_file` with a structured
error.

The Rust path does NOT load `dynamics_v2.state_dict.pt`; that file is
PyTorch-specific. When a real trained run ships, both paths must be
re-exported in lockstep — `train.py` writes both the ONNX (for ORT)
and the state_dict (for the sidecar) and pins the same blake2b on
each in metadata.

---

See `docs/agents.md` for the call patterns an agent uses to drive
these endpoints, and `docs/protocol.md` for how the receipt's `model`
block fits into the wire shape the verifier consumes.
