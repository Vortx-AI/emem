# Hot-path latency baseline (Phase 0)

This file holds the first committed latency baseline for emem.
Before it, every latency figure in the docs was prose only.
Use these numbers as the regression gate.
The planned embedding and throughput work must beat them.

Run the benchmarks again with this command (it needs a Rust toolchain):

```bash
cargo bench -p emem-primitives --bench hot_paths
```

The bench source is `hot_paths.rs` in this directory.

## Where the numbers come from

All numbers come from one run on one development host.
Each value is the mean of criterion's `[lo mean hi]` output.
The file does not record the run date or the host.
It first appears in commit `7456f7e3`, dated 29 October 2026.
We did not run the benchmarks again for this page (7 November 2026).

The numbers are specific to that machine.
Compare ratios, not absolute times.
To measure a change, run the bench on the same host before and after it.
When the reference host changes, run the bench again and replace the numbers in this file.

The bench names each vector size after an embedding model: Tessera, Galileo, Clay and Prithvi.
emem.dev no longer runs the Clay, Prithvi and Galileo encoders.
The sizes stay useful as test shapes.

## 1. Vector operation inner loop

| operation | 128-D (Tessera) | 768-D (Galileo) | 1024-D (Clay/Prithvi) | 1152-D (Tessera, 9 years) |
|---|---|---|---|---|
| `cosine` (fp32 input, fp64 sum, scalar) | 131 ns | 728 ns | 967 ns | 1.09 µs |
| `cosine_finite` (skips NaN values) | 87 ns (see note) | 935 ns | 1.27 µs | 1.44 µs |
| `decode_then_score` (CBOR to `Vec<f32>`, then `cosine`) | 523 ns | 3.05 µs | 4.04 µs | 4.53 µs |

Note: in the 128-D `cosine_finite` case, every value is NaN.
The bench masks one 128-value year block, and at 128-D that block is the full vector.
The function returns early, so do not compare this value with the others.

What the table shows:

- `decode_then_score` takes about 4 times as long as `cosine`.
  At 128-D it is 523 ns against 131 ns. At 1024-D it is 4.04 µs against 0.97 µs.
- The CBOR decode in `as_vec_f32` causes most of the cost per candidate.
  It allocates a `Vec<f32>` and converts each element from a boxed `ciborium::Value::Float(f64)`.
- Phase 2 plans to remove this cost with fp16 vectors held in memory and SIMD.
- `cosine` alone runs at about 1 billion elements per second without SIMD.
  SIMD is the next step for it.

## 2. `find_similar` brute-force scaling (cosine, 128-D, k=10)

This bench installs no Lance index.
So `find_similar` uses the O(N) linear scan through `iter_index`.

| corpus (cells) | time | per cell |
|---|---|---|
| 1,000 | 1.95 ms | about 1.9 µs |
| 10,000 | 21.2 ms | about 2.1 µs |
| 100,000 | 233 ms | about 2.3 µs |

What the table shows:

- Time grows linearly, at about 2.1 µs per cell.
  This covers decode, score and heap updates.
- At 1 million cells, this rate gives about 2.3 s.
- At 100,000 cells, 233 ms is already too slow for an interactive call.
- Phase 2 must remove this limit with two changes.
  The first is an ANN path for queries with `filter`, `as_of` or `scope`, which skipped the index.
  The second is faster decode and SIMD scoring per candidate.

## 3. `find_similar` ANN pool and exact rerank (Phase 2, filtered queries)

Before Phase 2, a filtered, scoped or bi-temporal query used the full scan in section 2.
Now it takes a bounded pool of nearest vectors from the Lance index.
Then it computes the exact cosine for each candidate that passes the predicate.
The response names this method as `ranking_method: ann_oversampled_then_exact`.

| query on a 100,000-cell corpus, filtered | time | compared with brute force |
|---|---|---|
| brute force (exact, O(N)) | 233 ms | 1x |
| ANN pool and exact rerank | 23.3 ms | about 10 times faster |

What the table shows:

- The filtered query no longer scans every cell.
  The scores are exact. The recall is approximate, and the response says so.
- The 23.3 ms figure is high.
  In the bench, `MemStorage::scan_cell` is O(N), so each `as_of` check scans all 100,000 cells again.
  Production storage (sled) uses a prefix scan, so the real gain is larger.
- The time grows with the over-fetch factor, not with corpus size.
  `EMEM_FIND_SIMILAR_RERANK_OVERSAMPLE` sets that factor (default 32, range 4 to 512).

The code paths are in `crates/emem-primitives/src/find_similar.rs`.
A cosine query with `scope`, `as_of` or `filter` goes to the filtered rerank path.
If Lance is not present or no candidate passes, the query falls back to the full scan.
On `emem-server`, `EMEM_DISABLE_LANCE=1` turns Lance off, so every query uses the full scan.

## What this baseline gates

| item | status |
|---|---|
| Phase 2 ANN coverage | Done. `find_similar` is sub-linear for queries with `filter`, `scope` or `as_of`: 233 ms to 23 ms. |
| SIMD in the canonical `cosine` | Ruled out. `cosine` feeds the signed `triple_consensus` value, which must be bit-identical on every host. |
| fp16 vectors held in memory | Not done. This is the remaining option for the vector inner loop. It applies only to the derived index or a scorer used for ranking, not to the content-addressed path. |
| ANN partition for `mode=Hamming` | Not done. Hamming mode still scans every cell. |
