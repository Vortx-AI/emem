# jepa_v2 — small learned dynamics head over Tessera embeddings

Trains and exports the model that powers `jepa_temporal_predictor@2`
(P3-E Phase 1) — the CPU-fast learned predictor that takes a few prior
Tessera vintages of a cell and returns a predicted next-vintage
embedding. Inference runs in Rust via `ort` against the `.onnx` artifact
produced here.

## Architecture
- Input: `[batch, K=3, 128]` — three prior Tessera vectors per cell.
  Tessera ships as int8 + per-pixel f32 scale upstream; the recall
  layer decodes to f32 before this code sees it, so training operates
  on decoded f32 throughout.
- Output: `[batch, 128]` — predicted next-vintage embedding.
- Backbone: 4-layer MLP-mixer-ish residual block, ~200k params total.
- Loss: cosine + L2 in latent space (joint).
- Optimizer: AdamW, 3e-4, weight-decay 1e-4, 200 epochs.

## Data assembly
`assemble_data.py` walks the local emem instance via REST:
1. Calls `/v1/coverage_matrix` → list of (cell, band, last_attested_unix).
2. Filters to bands matching `geotessera.YYYY` for years 2017..2024.
3. For each cell with ≥4 consecutive vintages, calls `/v1/recall` per
   `geotessera.<year>` to fetch the 128-D vector (decoded f32 over the
   wire from the upstream int8+scale storage).
4. Saves to `<EMEM_DATA>/jepa_v2/training_data.npz` as
   `{cells: [str], vintages: [int], vectors: float32[N, Y, 128]}`.

## Training
`train.py` loads `training_data.npz`, builds (input, target) pairs with
input = vectors[:, t-2:t+1, :] and target = vectors[:, t+1, :], holds
out the latest year per cell as validation, and exports the trained
model to `<EMEM_DATA>/jepa_v2/dynamics_v2.onnx` plus a sidecar
`dynamics_v2.metadata.json` carrying validation metrics + training
provenance for the receipt.

## Honesty
With the current cache (~478 cells), the model is BOOTSTRAPPED, not
production-quality. Validation cosine similarity vs the "predict last
vintage" baseline is reported in the metadata sidecar. Re-run after
materialising more cells to improve quality. The receipt's
`model_cid` is the blake3 of the .onnx bytes — an agent can verify
exactly which checkpoint produced any given prediction.

## Reproducibility
- Random seed pinned to 42 in train.py
- All hyperparameters live as constants at the top of train.py
- `<EMEM_DATA>/jepa_v2/dynamics_v2.metadata.json` carries the seed,
  hyperparameters, validation metrics, training-set cell ids, and
  the data_assembly_at_unix timestamp.
