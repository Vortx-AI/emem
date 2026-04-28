//! emem-cubes — loaders for the AgriSynth 1792D bootstrap cubes.
//!
//! The bootstrap corpus is `farms/<NAME>/cube_10m.npz` from the agri repo,
//! shape `[N_pixels, 1792]` float16 (or float32 in v1 cubes).
//!
//! Bootstrap loading runs out-of-band of the protocol (the agri repo's
//! Python tooling produces the cubes). This crate is a stable Rust handle
//! for paths to those cubes; the on-disk parser intentionally lives in
//! `tools/load_cube.py`. Callers that need pixel-level access in Rust
//! should invoke the Python tool to produce a parquet-backed dataset that
//! emem-storage can ingest as Primary facts.

#![forbid(unsafe_code)]

use std::path::Path;

/// A loaded cube backed by an mmap'd `.npz`.
#[derive(Debug)]
pub struct Cube {
    /// Number of pixels.
    pub n_pixels: usize,
    /// Per-pixel dimensionality (always 1792 in v0).
    pub dims: usize,
    /// File path.
    pub path: std::path::PathBuf,
}

impl Cube {
    /// Open a cube handle by path. Returns a metadata-only descriptor
    /// (the on-disk parser lives in `tools/load_cube.py`); errors out
    /// if the path does not exist on the filesystem.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, CubeError> {
        let path = path.as_ref().to_path_buf();
        let meta = std::fs::metadata(&path)?;
        // Approximate pixel count assuming float16 storage (2 bytes × 1792 dims).
        let n_pixels = (meta.len() as usize).saturating_div(2 * 1792);
        Ok(Cube { n_pixels, dims: 1792, path })
    }
}

/// Cube-loading errors.
#[derive(Debug, thiserror::Error)]
pub enum CubeError {
    /// Disk I/O failed.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// File was not a valid `.npz`.
    #[error("not a valid npz: {0}")]
    BadNpz(String),
}
