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

/// Convert a (d, order) Hilbert curve distance to a 2D point.
/// Adapted from Wikipedia's Hilbert curve pseudocode.
pub fn d_to_xy(order: u32, mut d: u32) -> (u32, u32) {
    let n = 1u32 << order;
    let (mut x, mut y) = (0u32, 0u32);
    let mut s = 1u32;
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

/// Inverse of `d_to_xy`.
pub fn xy_to_d(order: u32, mut x: u32, mut y: u32) -> u32 {
    let n = 1u32 << order;
    let mut d: u32 = 0;
    let mut s = n / 2;
    while s > 0 {
        let rx = u32::from((x & s) > 0);
        let ry = u32::from((y & s) > 0);
        d += s * s * ((3 * rx) ^ ry);
        if ry == 0 {
            if rx == 1 {
                // Standard Hilbert rotate: wrapping is intentional — the
                // result is reduced mod 2^32 and the higher bits cancel
                // through the subsequent swap and segment-size descent.
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
}
