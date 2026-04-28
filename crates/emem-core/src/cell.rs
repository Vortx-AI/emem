//! emem cell — recursive icosahedral aperture-7 hex tessellation.
//!
//! Spec §3.1. The cell ID is a 64-bit packed integer with the H3-equivalent
//! geometry layout, normatively defined here without referring to H3 in the
//! wire format. Implementations MAY use H3 ≥4.0 as a backend if they pass
//! the cell test vectors in `spec/test_vectors/cell64/`.
//!
//! ```text
//! [63]      reserved (must be 0)
//! [62..59]  mode (4 bits, 16 modes; see [`Mode`])
//! [58..56]  edge/vertex disambiguation (3 bits)
//! [55..52]  resolution (4 bits, 0..=15)
//! [51..45]  base cell (7 bits, 0..=121 valid)
//! [44..0]   path: 15 × 3-bit child digits, level 1 highest, level 15 lowest.
//!           Unused trailing levels (for resolutions < 15) are filled with
//!           the sentinel digit 0b111 (=7) which is never a valid child.
//! ```

use serde::{Deserialize, Serialize};

// ── Bit layout constants ─────────────────────────────────────────────────

const RESERVED_SHIFT: u32 = 63;
const RESERVED_MASK: u64  = 1u64 << RESERVED_SHIFT;

const MODE_SHIFT: u32 = 59;
const MODE_BITS: u32  = 4;
const MODE_MASK: u64  = ((1u64 << MODE_BITS) - 1) << MODE_SHIFT;

const DISAMBIG_SHIFT: u32 = 56;
const DISAMBIG_BITS: u32  = 3;
const DISAMBIG_MASK: u64  = ((1u64 << DISAMBIG_BITS) - 1) << DISAMBIG_SHIFT;

const RES_SHIFT: u32 = 52;
const RES_BITS: u32  = 4;
const RES_MASK: u64  = ((1u64 << RES_BITS) - 1) << RES_SHIFT;

const BASE_SHIFT: u32 = 45;
const BASE_BITS: u32  = 7;
const BASE_MASK: u64  = ((1u64 << BASE_BITS) - 1) << BASE_SHIFT;

/// 3 bits per path level.
pub const PATH_BITS_PER_LEVEL: u32 = 3;
/// Number of subdivision levels (res 1..=15).
pub const MAX_PATH_LEVELS: u32 = 15;
/// Sentinel digit for unused trailing path levels.
pub const PATH_SENTINEL: u8 = 0b111;

/// Number of icosahedral base cells (110 hex + 12 pentagon = 122).
pub const BASE_CELL_COUNT: u8 = 122;

/// Default fact resolution (≈3.4 m edge).
pub const DEFAULT_RESOLUTION: Resolution = Resolution(13);

/// Maximum resolution permitted by v0 (sub-meter sensors reserved at v0.2).
pub const MAX_RESOLUTION: Resolution = Resolution(15);

// ── Types ────────────────────────────────────────────────────────────────

/// A single emem cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Cell(pub u64);

/// Cell mode (4 bits, 16 modes; spec §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum Mode {
    /// A cell at some resolution.
    Cell = 1,
    /// A directed edge between two cells.
    DirectedEdge = 2,
    /// An undirected edge between two cells.
    UndirectedEdge = 3,
    /// A vertex shared by surrounding cells.
    Vertex = 4,
    /// A compressed set of cells (range encoding).
    Set = 5,
}

/// Resolution level 0..=15.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Resolution(pub u8);

/// Base cell ID 0..=121.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BaseCell(pub u8);

// ── Methods ──────────────────────────────────────────────────────────────

/// Bit position of the digit for a given path level (1..=15).
const fn level_bit_pos(level: u32) -> u32 {
    PATH_BITS_PER_LEVEL * (MAX_PATH_LEVELS - level)
}

impl Cell {
    /// Construct from raw u64. No validation; callers must use [`Cell::pack`]
    /// for validated construction.
    pub const fn from_raw(raw: u64) -> Self { Cell(raw) }

    /// Pack mode/resolution/base/path into a Cell. Trailing path digits are
    /// auto-filled with the sentinel.
    pub fn pack(mode: Mode, res: Resolution, base: BaseCell, path: &[u8]) -> Self {
        debug_assert!(res.0 <= MAX_RESOLUTION.0);
        debug_assert!(base.0 < BASE_CELL_COUNT);
        debug_assert!(path.len() <= res.0 as usize);

        let mut raw: u64 = 0;
        raw |= (mode as u64) << MODE_SHIFT;
        raw |= (res.0 as u64) << RES_SHIFT;
        raw |= (base.0 as u64) << BASE_SHIFT;

        // Fill all 15 levels with the sentinel first, then overwrite the
        // populated ones.
        for level in 1..=MAX_PATH_LEVELS {
            raw |= (PATH_SENTINEL as u64) << level_bit_pos(level);
        }
        for (i, &digit) in path.iter().enumerate() {
            let level = (i as u32) + 1;
            let pos = level_bit_pos(level);
            // clear sentinel, set digit
            raw &= !(0b111u64 << pos);
            raw |= ((digit & 0b111) as u64) << pos;
        }
        Cell(raw)
    }

    /// Extract mode.
    pub fn mode(&self) -> Option<Mode> {
        match (self.0 & MODE_MASK) >> MODE_SHIFT {
            1 => Some(Mode::Cell),
            2 => Some(Mode::DirectedEdge),
            3 => Some(Mode::UndirectedEdge),
            4 => Some(Mode::Vertex),
            5 => Some(Mode::Set),
            _ => None,
        }
    }

    /// Extract resolution.
    pub fn resolution(&self) -> Resolution {
        Resolution(((self.0 & RES_MASK) >> RES_SHIFT) as u8)
    }

    /// Extract base cell.
    pub fn base_cell(&self) -> BaseCell {
        BaseCell(((self.0 & BASE_MASK) >> BASE_SHIFT) as u8)
    }

    /// Extract the path digit at `level` (1..=15). Returns `None` if the
    /// digit is unset (sentinel) or `level` exceeds this cell's resolution.
    pub fn path_digit(&self, level: u32) -> Option<u8> {
        if level == 0 || level > MAX_PATH_LEVELS { return None; }
        if level > self.resolution().0 as u32 { return None; }
        let d = ((self.0 >> level_bit_pos(level)) & 0b111) as u8;
        if d == PATH_SENTINEL { None } else { Some(d) }
    }

    /// Parent cell at one coarser resolution. None when already at res 0.
    pub fn parent(&self) -> Option<Cell> {
        let r = self.resolution().0;
        if r == 0 { return None; }
        let pos = level_bit_pos(r as u32);
        // set this level's digit to sentinel, then decrement res
        let mut raw = self.0 & !(0b111u64 << pos);
        raw |= (PATH_SENTINEL as u64) << pos;
        raw = (raw & !RES_MASK) | (((r - 1) as u64) << RES_SHIFT);
        Some(Cell(raw))
    }

    /// Construct a child cell at one finer resolution with the given digit
    /// (0..=6). Returns `None` if already at MAX_RESOLUTION or digit invalid.
    pub fn child(&self, digit: u8) -> Option<Cell> {
        let r = self.resolution().0;
        if r >= MAX_RESOLUTION.0 || digit > 6 { return None; }
        let new_r = r + 1;
        let pos = level_bit_pos(new_r as u32);
        let mut raw = self.0 & !(0b111u64 << pos);
        raw |= (digit as u64) << pos;
        raw = (raw & !RES_MASK) | ((new_r as u64) << RES_SHIFT);
        Some(Cell(raw))
    }

    /// True iff the reserved bit is zero and the mode is one of the defined
    /// values.
    pub fn is_well_formed(&self) -> bool {
        if self.0 & RESERVED_MASK != 0 { return false; }
        self.mode().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_and_extract() {
        let c = Cell::pack(Mode::Cell, Resolution(3), BaseCell(42), &[1, 2, 3]);
        assert_eq!(c.mode(), Some(Mode::Cell));
        assert_eq!(c.resolution(), Resolution(3));
        assert_eq!(c.base_cell(), BaseCell(42));
        assert_eq!(c.path_digit(1), Some(1));
        assert_eq!(c.path_digit(2), Some(2));
        assert_eq!(c.path_digit(3), Some(3));
        assert_eq!(c.path_digit(4), None);
    }

    #[test]
    fn parent_chain() {
        let c = Cell::pack(Mode::Cell, Resolution(3), BaseCell(42), &[1, 2, 3]);
        let p = c.parent().unwrap();
        assert_eq!(p.resolution(), Resolution(2));
        assert_eq!(p.path_digit(3), None);
        assert_eq!(p.parent().unwrap().resolution(), Resolution(1));
    }

    #[test]
    fn parent_at_root_is_none() {
        let c = Cell::pack(Mode::Cell, Resolution(0), BaseCell(5), &[]);
        assert!(c.parent().is_none());
    }

    #[test]
    fn child_extends_path() {
        let c = Cell::pack(Mode::Cell, Resolution(2), BaseCell(7), &[3, 4]);
        let ch = c.child(5).unwrap();
        assert_eq!(ch.resolution(), Resolution(3));
        assert_eq!(ch.path_digit(3), Some(5));
    }

    #[test]
    fn base_cell_holds_122() {
        let c = Cell::pack(Mode::Cell, Resolution(0), BaseCell(121), &[]);
        assert_eq!(c.base_cell(), BaseCell(121));
    }

    #[test]
    fn well_formed_for_valid_cells() {
        let c = Cell::pack(Mode::Cell, Resolution(13), BaseCell(0), &[]);
        assert!(c.is_well_formed());
    }
}
