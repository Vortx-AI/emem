//! tslot — token-economical temporal addressing.
//!
//! Spec §3.3. An unsigned integer bucket of the Unix timeline at the
//! band's tempo cadence — `tslot = floor(unix_seconds / tempo.slot_seconds())`.
//!
//! `EMEM_EPOCH_UNIX` (2026-01-01T00:00:00Z UTC) is retained as a
//! reference epoch for protocol metadata, but `Tslot::from_unix` no
//! longer subtracts it from the input. The pre-v0.0.3 behavior
//! (offset-from-2026-epoch) made every pre-epoch historical observation
//! collapse to `Tslot(0)`, which structurally broke per-tslot historical
//! backfill — the natural reading of "5 years of MODIS NDVI" can't be
//! addressed if the addressing scheme starts in the future. Buckets-of-
//! Unix matches how every other Earth-observation system stores time.

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
            Tempo::Static => 0,
            Tempo::Slow => 365 * 24 * 60 * 60,
            Tempo::Medium => 30 * 24 * 60 * 60,
            Tempo::Fast => 24 * 60 * 60,
            Tempo::UltraFast => 60 * 60,
        }
    }
}

impl Tslot {
    /// Snap a Unix timestamp (seconds) to the slot for a given tempo.
    /// Buckets are anchored at the Unix epoch (1970-01-01T00:00:00Z),
    /// not the emem epoch, so historical observations from any past
    /// year have a natural per-tempo address. Pre-1970 timestamps
    /// (negative Unix seconds) clamp to `Tslot(0)`.
    pub fn from_unix(unix_seconds: i64, tempo: Tempo) -> Self {
        if matches!(tempo, Tempo::Static) {
            return Tslot(0);
        }
        let dur = tempo.slot_seconds() as i64;
        Tslot((unix_seconds.max(0) / dur) as u64)
    }

    /// Inverse of `from_unix`: the Unix epoch second at the start of
    /// this slot for the given tempo. For `Static`, returns 0.
    pub fn to_unix_start(self, tempo: Tempo) -> i64 {
        if matches!(tempo, Tempo::Static) {
            return 0;
        }
        (self.0 as i64) * (tempo.slot_seconds() as i64)
    }
}
