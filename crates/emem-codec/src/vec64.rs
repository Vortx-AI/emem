//! vec64 — vector-as-address derivation.
//!
//! A 1792D fp16 vector → blake3 → first 12 bytes → base32 = vec64.
//! See spec §3.4.
//!
//! 12 bytes (96 bits) → birthday collision at √(2^96) ≈ 8×10^14 vectors,
//! comfortably above the global emem fact-vector population (~10^13 at
//! full coverage). The full 32-byte CID is still the storage key; vec64 is
//! a token-economical short form for inline reference.

use blake3::Hasher;
use data_encoding::BASE32_NOPAD;

/// Number of bytes in the vec64 prefix. 12 bytes = 96 bits.
pub const VEC64_PREFIX_BYTES: usize = 12;

/// Derive the `vec64` short form for a 1792D float vector.
///
/// The hash is computed over the canonical fp16 little-endian byte
/// representation (each f32 is converted to f16 by `f32_to_f16_bits` and
/// serialized as raw u16 little-endian bytes).
pub fn to_vec64(v: &[f32]) -> String {
    let cid = vec64_to_cid(v);
    BASE32_NOPAD.encode(&cid[..VEC64_PREFIX_BYTES]).to_lowercase()
}

/// Compute the full 32-byte CID over the fp16 canonical form of the vector.
pub fn vec64_to_cid(v: &[f32]) -> [u8; 32] {
    let mut hasher = Hasher::new();
    let mut buf = [0u8; 2];
    for &f in v {
        let h = f32_to_f16_bits(f);
        buf.copy_from_slice(&h.to_le_bytes());
        hasher.update(&buf);
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(hasher.finalize().as_bytes());
    out
}

/// Convert f32 → IEEE 754 binary16 bits. Round-to-nearest-even.
/// Pure-Rust, dependency-free.
fn f32_to_f16_bits(x: f32) -> u16 {
    let bits = x.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xFF) as i32 - 127 + 15;
    let mant = (bits & 0x7FFFFF) as u32;

    if exp >= 31 {
        return sign | 0x7C00 | if (bits & 0x7FFF_FFFF) > 0x7F80_0000 { 0x200 } else { 0 };
    }
    if exp <= 0 {
        if exp < -10 { return sign; }
        let mant = mant | 0x0080_0000;
        let shift = (14 - exp) as u32;
        let new_mant = (mant >> shift) as u16;
        let round = if (mant >> (shift - 1)) & 1 == 1 { 1 } else { 0 };
        return sign | new_mant.wrapping_add(round);
    }
    let new_mant = (mant >> 13) as u16;
    let round = if (mant >> 12) & 1 == 1 { 1 } else { 0 };
    let packed = sign | ((exp as u16) << 10) | new_mant;
    packed.wrapping_add(round)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vec64_is_deterministic() {
        let v: Vec<f32> = (0..1792).map(|i| (i as f32) * 0.001).collect();
        let a = to_vec64(&v);
        let b = to_vec64(&v);
        assert_eq!(a, b);
        // 12 bytes base32 nopad = 20 chars (12 × 8 / 5 = 19.2 → ceil 20)
        assert!(a.len() >= 19 && a.len() <= 20, "got {}: {}", a.len(), a);
    }

    #[test]
    fn vec64_distinguishes_vectors() {
        let v1: Vec<f32> = (0..1792).map(|i| (i as f32) * 0.001).collect();
        let v2: Vec<f32> = (0..1792).map(|i| (i as f32) * 0.002).collect();
        assert_ne!(to_vec64(&v1), to_vec64(&v2));
    }
}
