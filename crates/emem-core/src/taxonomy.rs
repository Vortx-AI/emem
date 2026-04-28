//! lcv-1 — emem land-cover taxonomy.
//!
//! Spec §4. 64 leaf classes hierarchically grouped into 8 root families.
//! Each leaf has a canonical structural identifier (`lcv-1.fN.lM`).
//! Operators that prefer mnemonic class labels publish a separate label
//! manifest and reference its CID alongside the taxonomy.

use serde::{Deserialize, Serialize};

/// Root family of the lcv-1 taxonomy (8 families × 8 leaves = 64 classes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum LcvFamily {
    /// f0: Vegetation (closed canopy)
    VegClosed = 0,
    /// f1: Vegetation (open / shrub)
    VegOpen = 1,
    /// f2: Cropland (annual)
    CropAnnual = 2,
    /// f3: Cropland (perennial / orchard)
    CropPerennial = 3,
    /// f4: Built / sealed
    Built = 4,
    /// f5: Bare / sparse
    Bare = 5,
    /// f6: Water (inland + coastal)
    Water = 6,
    /// f7: Snow / ice / wetland
    Cryo = 7,
}

/// A single lcv-1 leaf class. The family lives in the high 3 bits, the
/// intra-family leaf in the low 3 bits (0..7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Lcv1(pub u8);

impl Lcv1 {
    /// Construct from family + intra-family leaf 0..7.
    pub const fn new(family: LcvFamily, leaf: u8) -> Self {
        debug_assert!(leaf < 8);
        Lcv1((family as u8) << 3 | (leaf & 0x07))
    }

    /// Extract family.
    pub fn family(self) -> LcvFamily {
        match self.0 >> 3 {
            0 => LcvFamily::VegClosed,
            1 => LcvFamily::VegOpen,
            2 => LcvFamily::CropAnnual,
            3 => LcvFamily::CropPerennial,
            4 => LcvFamily::Built,
            5 => LcvFamily::Bare,
            6 => LcvFamily::Water,
            _ => LcvFamily::Cryo,
        }
    }

    /// Canonical structural identifier, e.g. `"lcv-1.f3.l5"`.
    pub fn id(self) -> String {
        let fam_idx = self.0 >> 3;
        let leaf_idx = self.0 & 0x07;
        format!("lcv-1.f{fam_idx}.l{leaf_idx}")
    }
}
