# Hot-path latency baseline (Phase 0)

The first committed perf baseline for emem. Before this, every latency
figure in the docs was prose. These numbers are the regression gate and
the target the embedding/throughput upgrade has to beat.

Reproduce:

```bash
cargo bench -p emem-primitives --bench hot_paths
```

Numbers below are from one dev host (single run, criterion's
[lo mean hi] reduced to the mean). **They are machine-specific** — what
matters is the *ratios* and re-running on the same host before/after a
change. Re-run and overwrite this file when the reference host changes.

## 1. Vector-op inner loop

| op | 128-D (Tessera) | 768-D (Galileo) | 1024-D (Clay/Prithvi) | 1152-D (Tessera ×9 yr) |
|---|---|---|---|---|
| `cosine` (fp32, scalar fp64 accum) | 131 ns | 728 ns | 967 ns | 1.09 µs |
| `cosine_finite` (NaN-masked) | 87 ns¹ | 935 ns | 1.27 µs | 1.44 µs |
| `decode_then_score` (CBOR→Vec<f32> + cosine) | **523 ns** | 3.05 µs | 4.04 µs | 4.53 µs |

¹ the 128-D `cosine_finite` case is all-NaN (one masked vintage block)
and returns early, so it's not comparable to the others.

**Read:** `decode_then_score` is **~4× `cosine`** at 128-D (523 ns vs
131 ns) and **~4×** at 1024-D (4.04 µs vs 0.97 µs). The CBOR `as_vec_f32`
decode — allocate a `Vec<f32>`, coerce every element from a boxed
`ciborium::Value::Float(f64)` — dominates per-candidate cost. This is the
tax fp16-resident vectors + SIMD remove in Phase 2. `cosine` itself runs
at ~1 Gelem/s scalar; SIMD is the lever there.

## 2. `find_similar` brute-force scaling (cosine, 128-D, k=10)

No Lance index installed → the O(N) `iter_index` linear scan.

| corpus | time | per-cell |
|---|---|---|
| 1 000 | 1.95 ms | ~1.9 µs |
| 10 000 | 21.2 ms | ~2.1 µs |
| 100 000 | 233 ms | ~2.3 µs |

**Read:** dead-linear — ~2.1 µs/cell (decode + score + heap churn),
extrapolating to **~2.3 s at 1 M cells**. 233 ms at 100k already blows
any interactive budget. This is the wall the ANN fast-path (cover the
`filter`/`as_of`/`scope` bypass cases) and the per-candidate decode/SIMD
work in Phase 2 must break.

## What this gates

- Phase 2 SIMD + fp16-resident vectors → target: `decode_then_score`
  collapses toward `cosine`, `cosine` itself drops 4–8×.
- Phase 2 ANN coverage → target: `find_similar` goes sub-linear on the
  query shapes that currently fall back to brute force.
