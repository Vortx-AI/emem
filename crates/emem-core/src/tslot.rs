//! tslot — token-economical temporal addressing.
//!
//! Spec §3.3. An unsigned integer offset from the emem epoch
//! (2026-01-01T00:00:00Z UTC) in tempo-class-implied units.

use serde::{Deserialize, Serialize};

/// A time slot, encoded as a bare u64. The unit is determined by the band's
/// declared tempo class — see [`Tempo`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Tslot(pub u64);

/// Band tempo class. Drives slot duration, cache TTL, and refinement scheduling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tempo {
    /// Bands that never change: DEM, Köppen.
    Static,
    /// Annual cadence: Tessera v1, soil.
    Slow,
    /// Monthly cadence: NDVI composites.
    Medium,
    /// Daily cadence: raw S2 NDVI.
    Fast,
    /// Hourly cadence: weather, traffic.
    UltraFast,
}

/// Epoch sentinel: emem epoch is 2026-01-01T00:00:00Z UTC.
/// Stored as Unix epoch seconds for comparison only.
pub const EMEM_EPOCH_UNIX: i64 = 1_767_225_600;

impl Tempo {
    /// Slot duration in seconds; Static returns 0 (the slot is meaningless).
    pub const fn slot_seconds(self) -> u64 {
        match self {
            Tempo::Static    => 0,
            Tempo::Slow      => 365 * 24 * 60 * 60,
            Tempo::Medium    => 30  * 24 * 60 * 60,
            Tempo::Fast      =>       24 * 60 * 60,
            Tempo::UltraFast =>            60 * 60,
        }
    }
}

impl Tslot {
    /// Snap a Unix timestamp (seconds) to the slot for a given tempo.
    pub fn from_unix(unix_seconds: i64, tempo: Tempo) -> Self {
        if matches!(tempo, Tempo::Static) {
            return Tslot(0);
        }
        let dur = tempo.slot_seconds() as i64;
        let off = unix_seconds - EMEM_EPOCH_UNIX;
        Tslot(((off / dur).max(0)) as u64)
    }
}
