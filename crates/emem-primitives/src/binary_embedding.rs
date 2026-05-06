// Box-Muller, Gram-Schmidt, and several tests are inherent index-keyed
// linear-algebra loops; rewriting them with iterators would hide the
// matrix structure that's the point of the file.
#![allow(clippy::needless_range_loop)]

//! Binary-quantized embeddings for fast triage k-NN.
//!
//! The protocol's default foundation embedding (`geotessera`) is a
//! 128-D vector stored upstream as int8 + per-pixel f32 scale
//! (~144 B/cell on disk: 128 int8 + 4 B scale + tile metadata),
//! decoded to f32 by the recall path so all comparisons run in
//! fp32 cosine. Two changes here cut both the storage and the
//! scoring cost by ~16× without losing the ability to re-rank with
//! the full vector when an answer needs it:
//!
//!   1. **Sign-bit packing** to a 128-bit binary embedding (16 B/cell).
//!      The bit at position `i` is `1` iff the (rotated) value at
//!      dimension `i` is non-negative.
//!
//!   2. **TurboQuant rotation** — a fixed random orthogonal 128×128
//!      matrix applied *before* sign-bit extraction. This redistributes
//!      whatever variance the upstream embedding concentrates in a few
//!      axes across all 128 dims, so a single bit per dim carries
//!      meaningful information rather than collapsing to "is this the
//!      one big-magnitude axis or not". The trick is from Isaac
//!      Corley's *TerraBit* writeup
//!      (<https://geospatialml.com/posts/terrabit/>).
//!
//! Hamming distance over the packed bits scores via XOR + popcount,
//! which is one CPU instruction per 64-bit word. On commodity x86 a
//! single core does ~10⁹ scored pairs/sec — three orders of magnitude
//! faster than the fp32 cosine path. For corpora large enough to need
//! a Lance / FAISS sidecar, the binary fast path also degrades more
//! gracefully because it scans 16 bytes instead of 256 per candidate.
//!
//! ## Determinism
//!
//! The rotation is generated from a fixed seed (`ROT_SEED_TEXT`) via a
//! BLAKE3-keyed Gaussian draw + classical Gram-Schmidt. Every responder
//! that uses this module produces the **same** matrix bit-for-bit, so
//! a binary fact materialised on responder A is content-comparable
//! against one materialised on responder B without coordination —
//! and the rotation's content address can be pinned via
//! [`rotation_cid`] for verifier round-trip.
//!
//! The chosen seed is recorded in the band's derivation `fn_key`
//! (`turboquant_geotessera_bin128_v1@1`) so the receipt is
//! reproducible: a verifier can re-derive the rotation, re-pack the
//! geotessera vector, and check the resulting bytes match.

#![forbid(unsafe_code)]

use std::sync::LazyLock;

/// Seed text that derives the TurboQuant rotation. Bumping the suffix
/// (`v1` → `v2`) MUST also rename the band key (e.g. `bin128.v2`) so
/// that no two responders produce different binary facts under the
/// same band name.
pub const ROT_SEED_TEXT: &str = "emem.binary_embedding.turboquant.v1";

/// Bits per binary embedding (matches the geotessera native dim).
pub const BIN_DIMS: usize = 128;
/// Bytes per packed binary embedding (`BIN_DIMS / 8`).
pub const BIN_BYTES: usize = BIN_DIMS / 8;

/// Process-wide cached rotation matrix. Generated once on first use.
pub static ROTATION: LazyLock<[[f32; BIN_DIMS]; BIN_DIMS]> = LazyLock::new(build_rotation);

/// BLAKE3-derived content address of the rotation matrix bytes.
/// Surfacing this CID alongside the binary fact lets verifiers
/// re-derive the rotation without trusting the responder's identity —
/// the matrix is fully determined by the seed text and the algorithm
/// in [`build_rotation`].
pub fn rotation_cid() -> String {
    let r = &*ROTATION;
    let mut h = blake3::Hasher::new();
    for row in r.iter() {
        for &v in row {
            h.update(&v.to_le_bytes());
        }
    }
    h.finalize().to_hex().to_string()
}

/// Apply rotation, then sign-bit pack into 16 bytes. Bit ordering is
/// MSB-first per byte — bit `i` of dim `d` lives at byte `d / 8`,
/// shift `7 - (d % 8)`. This matches the natural "big-endian within
/// byte" layout that JS / Python clients producing the same encoding
/// in WebAssembly / NumPy will emit by default, so cross-language
/// round-trips byte-compare cleanly.
pub fn pack_bin128(vec: &[f32; BIN_DIMS]) -> [u8; BIN_BYTES] {
    let r = &*ROTATION;
    let mut rotated = [0.0f32; BIN_DIMS];
    for i in 0..BIN_DIMS {
        let mut acc = 0.0f64;
        let row = &r[i];
        for j in 0..BIN_DIMS {
            acc += (row[j] as f64) * (vec[j] as f64);
        }
        rotated[i] = acc as f32;
    }
    let mut out = [0u8; BIN_BYTES];
    for d in 0..BIN_DIMS {
        if rotated[d] >= 0.0 {
            out[d / 8] |= 1u8 << (7 - (d % 8));
        }
    }
    out
}

/// Variant of [`pack_bin128`] that accepts an arbitrary-length slice.
/// Returns `None` if the slice does not have exactly `BIN_DIMS`
/// elements — callers should only pass through after they've
/// confirmed the band is the 128-D `geotessera` family.
pub fn pack_bin128_slice(vec: &[f32]) -> Option<[u8; BIN_BYTES]> {
    if vec.len() != BIN_DIMS {
        return None;
    }
    let mut buf = [0.0f32; BIN_DIMS];
    buf.copy_from_slice(vec);
    Some(pack_bin128(&buf))
}

/// Hamming distance (number of differing bits) between two packed
/// binary embeddings. Lower = more similar. Uses Rust's
/// `u128::count_ones` which compiles to `POPCNT` on x86-64 with the
/// `popcnt` feature (or to a constant-time table fallback on older
/// targets — still O(1) per pair).
pub fn hamming_distance(a: &[u8; BIN_BYTES], b: &[u8; BIN_BYTES]) -> u32 {
    let ua = u128::from_be_bytes(*a);
    let ub = u128::from_be_bytes(*b);
    (ua ^ ub).count_ones()
}

/// Convert a Hamming distance to a similarity score in `[-1.0, 1.0]`
/// using the same convention as cosine similarity (1.0 = identical,
/// 0.0 = orthogonal-equivalent, -1.0 = anti-aligned). Exact mapping:
/// `score = 1 - 2 * dist / BIN_DIMS`. Useful for ordering binary
/// neighbours alongside cosine neighbours in mixed-mode results.
pub fn hamming_score(dist: u32) -> f32 {
    1.0 - 2.0 * (dist as f32) / (BIN_DIMS as f32)
}

// ── Deterministic rotation builder ────────────────────────────────

/// Build a 128×128 orthogonal matrix from `ROT_SEED_TEXT`. Algorithm:
/// 1. Use BLAKE3 keyed by the seed as a CSPRNG to fill 16 384 f32
///    Gaussian samples (Box-Muller from uniform draws).
/// 2. Classical Gram-Schmidt orthonormalisation in fp64. The pivot
///    rejection guard (`norm > 1e-9`) is paranoia — Gaussian columns
///    are linearly independent w.p. 1, but we'd rather fail loud than
///    silently emit a degenerate basis.
fn build_rotation() -> [[f32; BIN_DIMS]; BIN_DIMS] {
    let mut hasher = blake3::Hasher::new_keyed(&derive_key(ROT_SEED_TEXT));
    hasher.update(b"turboquant_rotation");
    let mut xof = hasher.finalize_xof();
    // Two u64 draws per matrix cell (one Box-Muller pass needs two
    // uniform inputs), 8 bytes per draw.
    let needed_bytes = BIN_DIMS * BIN_DIMS * 2 * 8;
    let mut buf = vec![0u8; needed_bytes];
    xof.fill(&mut buf);

    let mut a: [[f64; BIN_DIMS]; BIN_DIMS] = [[0.0; BIN_DIMS]; BIN_DIMS];
    let mut idx = 0usize;
    for i in 0..BIN_DIMS {
        for j in 0..BIN_DIMS {
            let u1 = u64_to_unit(read_u64(&buf, idx));
            idx += 1;
            let u2 = u64_to_unit(read_u64(&buf, idx));
            idx += 1;
            // Box-Muller transform: two uniform → one Gaussian.
            // We discard the second output (sin component) — it'd be
            // correlated with future cells of the same row otherwise.
            let g =
                (-2.0_f64 * u1.max(1e-300).ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
            a[i][j] = g;
        }
    }

    // Classical Gram-Schmidt. Operate row-wise (each row of `a` is one
    // basis vector). For the rotation to redistribute variance the
    // matrix only needs orthogonality + unit norm — we do NOT need
    // the modified-GS variant's extra numerical stability for
    // n=128 with double precision input.
    for i in 0..BIN_DIMS {
        for j in 0..i {
            let dot: f64 = (0..BIN_DIMS).map(|k| a[i][k] * a[j][k]).sum();
            for k in 0..BIN_DIMS {
                a[i][k] -= dot * a[j][k];
            }
        }
        let norm: f64 = (0..BIN_DIMS).map(|k| a[i][k] * a[i][k]).sum::<f64>().sqrt();
        assert!(
            norm > 1e-9,
            "TurboQuant Gram-Schmidt degenerate at row {i} (norm={norm:.3e}) — \
             rebuilding from a different seed is the right next step",
        );
        let inv = 1.0 / norm;
        for k in 0..BIN_DIMS {
            a[i][k] *= inv;
        }
    }

    let mut out = [[0.0f32; BIN_DIMS]; BIN_DIMS];
    for i in 0..BIN_DIMS {
        for j in 0..BIN_DIMS {
            out[i][j] = a[i][j] as f32;
        }
    }
    out
}

fn derive_key(seed_text: &str) -> [u8; 32] {
    let h = blake3::hash(seed_text.as_bytes());
    *h.as_bytes()
}

fn read_u64(buf: &[u8], i: usize) -> u64 {
    let off = i * 8;
    let mut b = [0u8; 8];
    b.copy_from_slice(&buf[off..off + 8]);
    u64::from_le_bytes(b)
}

/// Map a u64 to (0, 1] uniform. The +1 / +2 in numerator/denominator
/// avoids both endpoints; Box-Muller's `ln(u)` is undefined at 0.
fn u64_to_unit(x: u64) -> f64 {
    let n = (x >> 11) as f64; // 53-bit precision (matches f64 mantissa)
    (n + 1.0) / (((1u64 << 53) + 2) as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_is_orthogonal() {
        let r = &*ROTATION;
        // R · R^T should be the identity (within fp32 tolerance).
        let mut max_err: f32 = 0.0;
        for i in 0..BIN_DIMS {
            for j in 0..BIN_DIMS {
                let dot: f32 = (0..BIN_DIMS).map(|k| r[i][k] * r[j][k]).sum();
                let expected = if i == j { 1.0 } else { 0.0 };
                let err = (dot - expected).abs();
                if err > max_err {
                    max_err = err;
                }
            }
        }
        assert!(
            max_err < 5e-5,
            "rotation orthogonality breach: max_err={max_err:.3e}",
        );
    }

    #[test]
    fn pack_is_deterministic() {
        let mut v = [0.0f32; BIN_DIMS];
        for i in 0..BIN_DIMS {
            v[i] = ((i as f32) - 64.0) * 0.01;
        }
        let a = pack_bin128(&v);
        let b = pack_bin128(&v);
        assert_eq!(a, b);
    }

    #[test]
    fn pack_uses_all_dims() {
        let mut v = [0.0f32; BIN_DIMS];
        for i in 0..BIN_DIMS {
            v[i] = if i % 3 == 0 { 1.0 } else { -1.0 };
        }
        let packed = pack_bin128(&v);
        // After rotation, expect a roughly-balanced bit pattern (not
        // all zero, not all one). 128 bits with a quasi-uniform sign
        // distribution should land in [40, 88] popcount.
        let pop = u128::from_be_bytes(packed).count_ones();
        assert!(
            (40..=88).contains(&pop),
            "expected ~64-bit popcount after rotation, got {pop}",
        );
    }

    #[test]
    fn hamming_self_zero() {
        let mut v = [0.0f32; BIN_DIMS];
        for i in 0..BIN_DIMS {
            v[i] = (i as f32).sin();
        }
        let p = pack_bin128(&v);
        assert_eq!(hamming_distance(&p, &p), 0);
        assert!((hamming_score(0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn hamming_inverse_max() {
        let mut v = [0.0f32; BIN_DIMS];
        for i in 0..BIN_DIMS {
            v[i] = (i as f32).sin();
        }
        let p = pack_bin128(&v);
        let mut q = p;
        for byte in q.iter_mut() {
            *byte = !*byte;
        }
        assert_eq!(hamming_distance(&p, &q), BIN_DIMS as u32);
        assert!((hamming_score(BIN_DIMS as u32) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn similar_vectors_have_low_hamming() {
        // Two vectors that differ only in noise should share most bits
        // after the rotation+sign step.
        let mut v1 = [0.0f32; BIN_DIMS];
        for i in 0..BIN_DIMS {
            v1[i] = ((i * 17 + 3) % 19) as f32 - 9.0;
        }
        let mut v2 = v1;
        // Tiny perturbation — well below the per-dim signal magnitude.
        for i in 0..BIN_DIMS {
            v2[i] += (i as f32).sin() * 0.001;
        }
        let p1 = pack_bin128(&v1);
        let p2 = pack_bin128(&v2);
        let d = hamming_distance(&p1, &p2);
        assert!(d < 20, "expected near-identical packs, got hamming={d}");
    }

    #[test]
    fn rotation_cid_is_stable() {
        let a = rotation_cid();
        let b = rotation_cid();
        assert_eq!(a, b);
        assert_eq!(a.len(), 64); // BLAKE3 hex
    }

    #[test]
    fn pack_bin128_slice_rejects_wrong_length() {
        assert!(pack_bin128_slice(&[1.0f32; 64]).is_none());
        assert!(pack_bin128_slice(&[1.0f32; 200]).is_none());
        assert!(pack_bin128_slice(&[1.0f32; 128]).is_some());
    }
}
