//! Hilbert curve traversal helpers for the cell64 alphabet construction.
//!
//! The cell64 alphabet is ordered along a Hilbert curve over the 16×16 child
//! lattice at each subdivision step, so that adjacent alphabet indices map
//! to spatially adjacent cells (spec §3.2 locality property).
//!
//! Runtime helpers used by codec round-trip tests. The construction
//! algorithm for measuring the BPE-optimal alphabet lives in
//! `tools/measure_alphabet.py`; this module exposes the in-process
//! Hilbert-curve walks the codec uses.
//!
//! Note: order can run up to 31 here (axis side n = 2^31; total
//! Hilbert distance d = 2^62, fits u64). The geo encoder uses
//! order=22 (~5–10 m grid at the equator); the older order=16
//! encoder (~305 m grid) read the same helpers when it was active.

/// Convert a (d, order) Hilbert curve distance to a 2D point.
/// Adapted from Wikipedia's Hilbert curve pseudocode. `order ≤ 31`;
/// `d` ranges [0, 2^(2·order)).
pub fn d_to_xy(order: u32, mut d: u64) -> (u64, u64) {
    let n: u64 = 1u64 << order;
    let (mut x, mut y) = (0u64, 0u64);
    let mut s: u64 = 1;
    while s < n {
        let rx = 1 & (d / 2);
        let ry = 1 & (d ^ rx);
        if ry == 0 {
            if rx == 1 {
                x = s.wrapping_sub(1).wrapping_sub(x);
                y = s.wrapping_sub(1).wrapping_sub(y);
            }
            std::mem::swap(&mut x, &mut y);
        }
        x += s * rx;
        y += s * ry;
        d /= 4;
        s *= 2;
    }
    (x, y)
}

/// Inverse of `d_to_xy`. `order ≤ 31`; output `d ∈ [0, 2^(2·order))`.
pub fn xy_to_d(order: u32, mut x: u64, mut y: u64) -> u64 {
    let n: u64 = 1u64 << order;
    let mut d: u64 = 0;
    let mut s: u64 = n / 2;
    while s > 0 {
        let rx = u64::from((x & s) > 0);
        let ry = u64::from((y & s) > 0);
        d += s * s * ((3 * rx) ^ ry);
        if ry == 0 {
            if rx == 1 {
                // Standard Hilbert rotate: wrapping is intentional — the
                // result is reduced and higher bits cancel through the
                // subsequent swap and segment-size descent.
                x = s.wrapping_sub(1).wrapping_sub(x);
                y = s.wrapping_sub(1).wrapping_sub(y);
            }
            std::mem::swap(&mut x, &mut y);
        }
        s /= 2;
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hilbert_roundtrip_order_4() {
        for d in 0..256 {
            let (x, y) = d_to_xy(4, d);
            assert_eq!(xy_to_d(4, x, y), d);
        }
    }

    /// Order 22 is the active geo encoding (~5 m lat / ~10 m lng at
    /// equator). Sanity-spot a handful of points across the full
    /// 2^44-d distance range — exhaustive would be 17 trillion.
    #[test]
    fn hilbert_roundtrip_order_22() {
        let max_d: u64 = 1u64 << 44;
        for &d in &[
            0,
            1,
            42,
            12345,
            1u64 << 20,
            1u64 << 30,
            1u64 << 40,
            max_d - 1,
        ] {
            let (x, y) = d_to_xy(22, d);
            assert!(x < (1u64 << 22), "x out of range at d={d}: {x}");
            assert!(y < (1u64 << 22), "y out of range at d={d}: {y}");
            assert_eq!(xy_to_d(22, x, y), d, "round-trip failed at d={d}");
        }
    }
}
