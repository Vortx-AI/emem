//! Per-stage cold-path latency instrumentation.
//!
//! A cold recall fans out STAC search → COG profile open → tile range
//! read → decode → sign. Until Phase 0 there was no way to see where
//! that wall-clock actually went — the docs quoted `~180 ms cold` as
//! prose with no breakdown. [`StageTimer`] logs one stage's elapsed time
//! on drop (so it covers every early-return / `?` path) under the
//! `emem::latency` target. Get a live breakdown with:
//!
//! ```text
//! RUST_LOG=emem::latency=debug
//! ```
//!
//! It logs at debug level, so it is zero-cost when that target is
//! filtered out (the common case).

use std::time::Instant;

/// RAII timer that emits one `emem::latency` debug event when dropped.
pub struct StageTimer {
    stage: &'static str,
    detail: String,
    started: Instant,
}

impl StageTimer {
    /// Start timing a named stage. `detail` is a short identifier (a URL,
    /// cell, band, or collection) to disambiguate concurrent stages in
    /// the log stream.
    pub fn new(stage: &'static str, detail: impl Into<String>) -> Self {
        Self {
            stage,
            detail: detail.into(),
            started: Instant::now(),
        }
    }
}

impl Drop for StageTimer {
    fn drop(&mut self) {
        tracing::debug!(
            target: "emem::latency",
            stage = self.stage,
            detail = %self.detail,
            elapsed_us = self.started.elapsed().as_micros() as u64,
            "cold-path stage"
        );
    }
}
