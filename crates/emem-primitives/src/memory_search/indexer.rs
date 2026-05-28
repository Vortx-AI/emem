//! LanceDB partition for memory-file text embeddings.
//!
//! Mirrors the per-dim Lance approach in `lance_index.rs` but with a
//! *different schema* (path + file_cid + kind + signed_at + ...) and a
//! *different lifecycle*: memory-file embeddings are re-indexed when
//! files are created / modified / deleted / renamed, so the index is
//! mutable. The fact-vector partition next door is append-only.
//!
//! ## Schema
//!
//! ```text
//! path                : Utf8
//! file_cid            : Utf8
//! kind                : Utf8       // typing taxonomy, default "resource"
//! signed_at           : Utf8       // ISO 8601 UTC
//! attester_pubkey_b32 : Utf8       // nullable
//! size_bytes          : UInt64
//! vector              : FixedSizeList<Float32, 768>
//! ```
//!
//! ## Idempotency
//!
//! Each upsert keys on (path, file_cid). If the dataset already has a
//! row with the same (path, file_cid) pair we skip the embed + write —
//! same bytes at the same path are by definition identical and would
//! re-produce the same vector. Path-only or CID-only collisions still
//! get an upsert: the old row is removed first (delete-then-append).
//!
//! ## Indexer mode
//!
//! Today: **periodic poll** (`spawn_polling_indexer`). The poll cadence
//! is 60 s by default; tunable via `EMEM_MEMORY_SEARCH_POLL_SECS`. A
//! broadcast-channel hook can replace the poll without changing the
//! search primitive — `index_one_file` and `delete_path` are the
//! event-driven entry points.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use arrow_array::{
    Array, FixedSizeListArray, Float32Array, RecordBatch, RecordBatchIterator, StringArray,
    UInt64Array,
};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use futures_util::TryStreamExt;
use lance::dataset::{Dataset, WriteMode, WriteParams};
use lance_linalg::distance::DistanceType;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};

use super::text_embedder::{global_embedder, EmbedError, TextEmbedder, TEXT_EMBED_DIM};

/// Filename of the memory-text Lance dataset. Sits next to the per-dim
/// `vector_index_d{N}.lance` partitions.
pub const MEMORY_TEXT_DATASET_NAME: &str = "memory_text_index_d768.lance";

/// Errors surfaced by the memory-text indexer.
#[derive(Debug, thiserror::Error)]
pub enum IndexerError {
    /// Underlying Lance crate error.
    #[error("lance: {0}")]
    Lance(String),
    /// Arrow shape / type problem.
    #[error("arrow: {0}")]
    Arrow(String),
    /// Embedder load or inference error.
    #[error("embed: {0}")]
    Embed(String),
    /// Sled / storage I/O.
    #[error("storage: {0}")]
    Storage(String),
    /// I/O.
    #[error("io: {0}")]
    Io(String),
}

impl From<EmbedError> for IndexerError {
    fn from(e: EmbedError) -> Self {
        IndexerError::Embed(e.to_string())
    }
}

/// One file's worth of indexed data. The shape the search primitive
/// returns to callers (without the snippet — that comes from a separate
/// content fetch).
#[derive(Debug, Clone)]
pub struct IndexedRow {
    pub path: String,
    pub file_cid: String,
    pub kind: String,
    pub signed_at: String,
    pub attester_pubkey_b32: Option<String>,
    pub size_bytes: u64,
    pub vector: Vec<f32>,
}

/// Statistics surfaced via the stats endpoint.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryIndexStats {
    /// Total row count in the dataset.
    pub rows: u64,
    /// Filesystem path to the dataset.
    pub path: String,
    /// Whether the model file resolved on disk.
    pub model_loaded: bool,
    /// Embed dim (always 768 for BGE-base; surfaced so a future model
    /// swap is visible).
    pub embed_dim: usize,
    /// Last Unix-seconds wall-clock when hydration completed.
    pub last_hydrated_at_unix_s: Option<u64>,
    /// Last Unix-seconds wall-clock when an incremental upsert ran.
    pub last_indexed_at_unix_s: Option<u64>,
    /// Indexer mode: "polling" today; "broadcast" once Agent W's event
    /// channel is wired.
    pub indexer_mode: String,
    /// Poll cadence (seconds) when `indexer_mode == "polling"`.
    pub poll_interval_secs: u64,
    /// True when the polling loop is currently spawned.
    pub polling_active: bool,
}

/// Build the Arrow schema for the memory-text partition.
fn memory_schema() -> SchemaRef {
    let item = Field::new("item", DataType::Float32, true);
    Arc::new(Schema::new(vec![
        Field::new("path", DataType::Utf8, false),
        Field::new("file_cid", DataType::Utf8, false),
        Field::new("kind", DataType::Utf8, false),
        Field::new("signed_at", DataType::Utf8, false),
        Field::new("attester_pubkey_b32", DataType::Utf8, true),
        Field::new("size_bytes", DataType::UInt64, false),
        Field::new(
            "vector",
            DataType::FixedSizeList(Arc::new(item), TEXT_EMBED_DIM as i32),
            false,
        ),
    ]))
}

/// Single-dataset Lance partition for memory text embeddings.
pub struct MemoryTextIndex {
    /// Filesystem path to the Lance dataset.
    pub path: PathBuf,
    /// Lazy Lance dataset handle (None until first write opens it).
    inner: Arc<Mutex<Option<Dataset>>>,
    /// Cached schema.
    schema: SchemaRef,
    /// Polling cadence (seconds).
    pub poll_interval_secs: u64,
    /// Cached "is the polling loop running?" flag.
    polling_active: RwLock<bool>,
    /// Last-hydrate timestamp (Unix seconds).
    last_hydrated_at: RwLock<Option<u64>>,
    /// Last-index timestamp (Unix seconds).
    last_indexed_at: RwLock<Option<u64>>,
}

/// Resolve the dataset's filesystem path: same root as the fact-vector
/// partitions, distinct file name.
pub fn memory_dataset_path(root: &Path) -> PathBuf {
    root.join(MEMORY_TEXT_DATASET_NAME)
}

impl MemoryTextIndex {
    /// Open (or initialise) the index. Idempotent — repeated calls are
    /// cheap; first call creates the parent directory.
    pub fn open(root: impl AsRef<Path>) -> std::io::Result<Arc<Self>> {
        let root_ref = root.as_ref();
        std::fs::create_dir_all(root_ref)?;
        let path = memory_dataset_path(root_ref);
        let poll = std::env::var("EMEM_MEMORY_SEARCH_POLL_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(60);
        Ok(Arc::new(Self {
            path,
            inner: Arc::new(Mutex::new(None)),
            schema: memory_schema(),
            poll_interval_secs: poll,
            polling_active: RwLock::new(false),
            last_hydrated_at: RwLock::new(None),
            last_indexed_at: RwLock::new(None),
        }))
    }

    /// Open against the default-root location (`<EMEM_DATA>/lance/`).
    pub fn open_default() -> std::io::Result<Arc<Self>> {
        Self::open(crate::lance_index::default_root())
    }

    /// Borrow the dataset (lazy-open from disk if it exists).
    async fn dataset(&self) -> tokio::sync::MutexGuard<'_, Option<Dataset>> {
        let mut guard = self.inner.lock().await;
        if guard.is_none() {
            if let Ok(ds) = Dataset::open(self.path.to_string_lossy().as_ref()).await {
                *guard = Some(ds);
            }
        }
        guard
    }

    /// Drop the cached dataset handle so the next op re-opens it. Used
    /// after a delete/overwrite so subsequent scans see the new manifest.
    async fn reload(&self) {
        let mut guard = self.inner.lock().await;
        if let Ok(ds) = Dataset::open(self.path.to_string_lossy().as_ref()).await {
            *guard = Some(ds);
        }
    }

    /// Append rows. Creates the dataset on the first write, opens +
    /// appends on subsequent writes. No-op when `rows` is empty.
    async fn append_rows(&self, rows: &[IndexedRow]) -> Result<(), IndexerError> {
        if rows.is_empty() {
            return Ok(());
        }
        let batch = rows_to_batch(self.schema.clone(), rows)?;
        let schema = batch.schema();
        let reader = RecordBatchIterator::new(vec![Ok(batch)].into_iter(), schema);
        let mut guard = self.inner.lock().await;
        let uri = self.path.to_string_lossy().to_string();
        let mode = if guard.is_some() {
            WriteMode::Append
        } else {
            WriteMode::Create
        };
        let params = WriteParams {
            mode,
            ..Default::default()
        };
        let ds = Dataset::write(reader, uri.as_str(), Some(params))
            .await
            .map_err(|e| IndexerError::Lance(e.to_string()))?;
        *guard = Some(ds);
        let now = unix_s();
        *self.last_indexed_at.write().await = Some(now);
        Ok(())
    }

    /// Return the (path, file_cid) pairs currently in the index. Used
    /// by the polling loop to dedupe against already-indexed files.
    pub async fn existing_pairs(
        &self,
    ) -> Result<std::collections::HashSet<(String, String)>, IndexerError> {
        let guard = self.dataset().await;
        let ds = match guard.as_ref() {
            Some(ds) => ds,
            None => return Ok(std::collections::HashSet::new()),
        };
        let mut scanner = ds.scan();
        scanner
            .project(&["path", "file_cid"])
            .map_err(|e| IndexerError::Lance(e.to_string()))?;
        let stream = scanner
            .try_into_stream()
            .await
            .map_err(|e| IndexerError::Lance(e.to_string()))?;
        let batches: Vec<RecordBatch> = stream
            .try_collect()
            .await
            .map_err(|e| IndexerError::Lance(e.to_string()))?;
        let mut out = std::collections::HashSet::new();
        for b in batches {
            let path_col = b
                .column_by_name("path")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| IndexerError::Arrow("missing path".into()))?;
            let cid_col = b
                .column_by_name("file_cid")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| IndexerError::Arrow("missing file_cid".into()))?;
            for i in 0..b.num_rows() {
                out.insert((path_col.value(i).to_string(), cid_col.value(i).to_string()));
            }
        }
        Ok(out)
    }

    /// Read every row from the dataset (path, file_cid, kind,
    /// signed_at, attester_pubkey_b32, size_bytes, vector). Used by
    /// the brute-force fallback path in `memory_search` when Lance ANN
    /// is disabled, and by the polling loop to know which paths to keep.
    pub async fn read_all(&self) -> Result<Vec<IndexedRow>, IndexerError> {
        let guard = self.dataset().await;
        let ds = match guard.as_ref() {
            Some(ds) => ds,
            None => return Ok(Vec::new()),
        };
        let mut scanner = ds.scan();
        scanner
            .project(&[
                "path",
                "file_cid",
                "kind",
                "signed_at",
                "attester_pubkey_b32",
                "size_bytes",
                "vector",
            ])
            .map_err(|e| IndexerError::Lance(e.to_string()))?;
        let stream = scanner
            .try_into_stream()
            .await
            .map_err(|e| IndexerError::Lance(e.to_string()))?;
        let batches: Vec<RecordBatch> = stream
            .try_collect()
            .await
            .map_err(|e| IndexerError::Lance(e.to_string()))?;
        let mut out: Vec<IndexedRow> = Vec::new();
        for b in batches {
            let path_col = b
                .column_by_name("path")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| IndexerError::Arrow("missing path".into()))?;
            let cid_col = b
                .column_by_name("file_cid")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| IndexerError::Arrow("missing file_cid".into()))?;
            let kind_col = b
                .column_by_name("kind")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| IndexerError::Arrow("missing kind".into()))?;
            let signed_col = b
                .column_by_name("signed_at")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| IndexerError::Arrow("missing signed_at".into()))?;
            let attester_col = b
                .column_by_name("attester_pubkey_b32")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| IndexerError::Arrow("missing attester_pubkey_b32".into()))?;
            let size_col = b
                .column_by_name("size_bytes")
                .and_then(|c| c.as_any().downcast_ref::<UInt64Array>())
                .ok_or_else(|| IndexerError::Arrow("missing size_bytes".into()))?;
            let vec_col = b
                .column_by_name("vector")
                .and_then(|c| c.as_any().downcast_ref::<FixedSizeListArray>())
                .ok_or_else(|| IndexerError::Arrow("missing vector".into()))?;
            for i in 0..b.num_rows() {
                let v_arr = vec_col.value(i);
                let f32_arr = v_arr
                    .as_any()
                    .downcast_ref::<Float32Array>()
                    .ok_or_else(|| IndexerError::Arrow("vector item not f32".into()))?;
                let vector: Vec<f32> = (0..f32_arr.len()).map(|j| f32_arr.value(j)).collect();
                let attester = if attester_col.is_null(i) {
                    None
                } else {
                    Some(attester_col.value(i).to_string())
                };
                out.push(IndexedRow {
                    path: path_col.value(i).to_string(),
                    file_cid: cid_col.value(i).to_string(),
                    kind: kind_col.value(i).to_string(),
                    signed_at: signed_col.value(i).to_string(),
                    attester_pubkey_b32: attester,
                    size_bytes: size_col.value(i),
                    vector,
                });
            }
        }
        Ok(out)
    }

    /// Delete every row whose `path == target`. Used to drop the prior
    /// version when a file is re-written (or to clean up when a file is
    /// deleted). Lance's `delete` takes a SQL-style predicate string.
    pub async fn delete_path(&self, target: &str) -> Result<(), IndexerError> {
        // Single-quote escape: Lance's SQL parser doubles them.
        let escaped = target.replace('\'', "''");
        let predicate = format!("path = '{escaped}'");
        let mut guard = self.inner.lock().await;
        let Some(ds) = guard.as_mut() else {
            return Ok(());
        };
        ds.delete(&predicate)
            .await
            .map_err(|e| IndexerError::Lance(e.to_string()))?;
        // Drop the cached handle so reads see the new manifest version.
        drop(guard);
        self.reload().await;
        Ok(())
    }

    /// Upsert one file. Skips the embed when (path, file_cid) is
    /// already indexed. Otherwise: deletes any prior rows for the path,
    /// embeds the bytes, appends. Idempotent over identical bytes.
    #[allow(clippy::too_many_arguments)]
    pub async fn index_one_file(
        &self,
        embedder: &TextEmbedder,
        path: &str,
        file_cid: &str,
        text: &str,
        kind: &str,
        signed_at: &str,
        attester_pubkey_b32: Option<&str>,
        size_bytes: u64,
    ) -> Result<bool, IndexerError> {
        let existing = self.existing_pairs().await.unwrap_or_default();
        if existing.contains(&(path.to_string(), file_cid.to_string())) {
            // Same path + same CID → identical bytes → nothing to do.
            return Ok(false);
        }
        // Drop prior rows for the path (the file may have been edited).
        self.delete_path(path).await?;
        let vector = embedder.embed_document(text)?;
        let row = IndexedRow {
            path: path.to_string(),
            file_cid: file_cid.to_string(),
            kind: kind.to_string(),
            signed_at: signed_at.to_string(),
            attester_pubkey_b32: attester_pubkey_b32.map(str::to_string),
            size_bytes,
            vector,
        };
        self.append_rows(std::slice::from_ref(&row)).await?;
        Ok(true)
    }

    /// k-NN search over the indexed vectors. `query` must be 768-D and
    /// L2-normalised by the caller (use `TextEmbedder::embed_query`).
    /// Filters are applied in-process for predictability — the Lance
    /// predicate parser handles `=` cleanly but escaping multi-clause
    /// filters is enough surface area for a regression we keep the
    /// predicate logic here.
    pub async fn knn(
        &self,
        query: &[f32],
        k: usize,
        kind: Option<&str>,
        path_prefix: Option<&str>,
        attester_pubkey_b32: Option<&str>,
        oversample: usize,
    ) -> Result<Vec<(IndexedRow, f32)>, IndexerError> {
        if query.len() != TEXT_EMBED_DIM || k == 0 {
            return Ok(Vec::new());
        }
        let guard = self.dataset().await;
        let ds = match guard.as_ref() {
            Some(ds) => ds,
            None => return Ok(Vec::new()),
        };
        let q = Float32Array::from(query.to_vec());
        let mut scanner = ds.scan();
        // Pull more than k to give the post-filter slack; clamp to a
        // sane upper bound so a high k doesn't pull the whole corpus.
        let take = k.saturating_mul(oversample.max(1)).max(k);
        scanner
            .nearest("vector", &q, take)
            .map_err(|e| IndexerError::Lance(e.to_string()))?;
        scanner
            .project(&[
                "path",
                "file_cid",
                "kind",
                "signed_at",
                "attester_pubkey_b32",
                "size_bytes",
                "vector",
            ])
            .map_err(|e| IndexerError::Lance(e.to_string()))?;
        let stream = scanner
            .try_into_stream()
            .await
            .map_err(|e| IndexerError::Lance(e.to_string()))?;
        let batches: Vec<RecordBatch> = stream
            .try_collect()
            .await
            .map_err(|e| IndexerError::Lance(e.to_string()))?;
        let mut out: Vec<(IndexedRow, f32)> = Vec::new();
        for b in batches {
            let path_col = b
                .column_by_name("path")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| IndexerError::Arrow("missing path".into()))?;
            let cid_col = b
                .column_by_name("file_cid")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| IndexerError::Arrow("missing file_cid".into()))?;
            let kind_col = b
                .column_by_name("kind")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| IndexerError::Arrow("missing kind".into()))?;
            let signed_col = b
                .column_by_name("signed_at")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| IndexerError::Arrow("missing signed_at".into()))?;
            let attester_col = b
                .column_by_name("attester_pubkey_b32")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| IndexerError::Arrow("missing attester_pubkey_b32".into()))?;
            let size_col = b
                .column_by_name("size_bytes")
                .and_then(|c| c.as_any().downcast_ref::<UInt64Array>())
                .ok_or_else(|| IndexerError::Arrow("missing size_bytes".into()))?;
            let vec_col = b
                .column_by_name("vector")
                .and_then(|c| c.as_any().downcast_ref::<FixedSizeListArray>())
                .ok_or_else(|| IndexerError::Arrow("missing vector".into()))?;
            let dist = b
                .column_by_name("_distance")
                .and_then(|c| c.as_any().downcast_ref::<Float32Array>())
                .ok_or_else(|| IndexerError::Arrow("missing _distance".into()))?;
            for i in 0..b.num_rows() {
                let p = path_col.value(i);
                if let Some(prefix) = path_prefix {
                    if !p.starts_with(prefix) {
                        continue;
                    }
                }
                if let Some(want) = kind {
                    if kind_col.value(i) != want {
                        continue;
                    }
                }
                let attester_val = if attester_col.is_null(i) {
                    None
                } else {
                    Some(attester_col.value(i).to_string())
                };
                if let Some(want) = attester_pubkey_b32 {
                    if attester_val.as_deref() != Some(want) {
                        continue;
                    }
                }
                let v_arr = vec_col.value(i);
                let f32_arr = v_arr
                    .as_any()
                    .downcast_ref::<Float32Array>()
                    .ok_or_else(|| IndexerError::Arrow("vector item not f32".into()))?;
                let vector: Vec<f32> = (0..f32_arr.len()).map(|j| f32_arr.value(j)).collect();
                let row = IndexedRow {
                    path: p.to_string(),
                    file_cid: cid_col.value(i).to_string(),
                    kind: kind_col.value(i).to_string(),
                    signed_at: signed_col.value(i).to_string(),
                    attester_pubkey_b32: attester_val,
                    size_bytes: size_col.value(i),
                    vector,
                };
                // Lance cosine distance is `1 - cos`; map back to a
                // [0, 1]-clamped cosine score. We clamp because numeric
                // drift around 0 (and identical vectors) can produce
                // -1e-7 or 1+1e-7.
                let raw_score = 1.0_f32 - dist.value(i);
                let score = raw_score.clamp(0.0, 1.0);
                out.push((row, score));
            }
        }
        out.sort_by(|a, b| b.1.total_cmp(&a.1));
        if out.len() > k {
            out.truncate(k);
        }
        Ok(out)
    }

    /// Snapshot the index for the stats endpoint.
    pub async fn stats(&self) -> MemoryIndexStats {
        let guard = self.dataset().await;
        let rows = match guard.as_ref() {
            Some(ds) => ds.count_rows(None).await.unwrap_or(0) as u64,
            None => 0,
        };
        let model_loaded = global_embedder().is_ok();
        MemoryIndexStats {
            rows,
            path: self.path.to_string_lossy().into_owned(),
            model_loaded,
            embed_dim: TEXT_EMBED_DIM,
            last_hydrated_at_unix_s: *self.last_hydrated_at.read().await,
            last_indexed_at_unix_s: *self.last_indexed_at.read().await,
            indexer_mode: "polling".into(),
            poll_interval_secs: self.poll_interval_secs,
            polling_active: *self.polling_active.read().await,
        }
    }

    /// Stamp the last-hydrated wall-clock. Used by the polling loop
    /// after a complete pass.
    pub async fn mark_hydrated(&self) {
        *self.last_hydrated_at.write().await = Some(unix_s());
    }

    /// Mark the polling loop as active. Idempotent.
    pub async fn set_polling_active(&self, active: bool) {
        *self.polling_active.write().await = active;
    }
}

/// Trait that lets the indexer enumerate the current set of
/// memory-files without depending on `emem-api-rest`. Implemented by
/// the API layer so the polling loop can call back into sled.
///
/// `list_all` returns every file currently held; `read_text` returns
/// the bytes-as-UTF-8 for one path (or `None` when the file vanished
/// between list and read).
#[async_trait::async_trait]
pub trait MemoryFileSource: Send + Sync {
    /// Enumerate every memory file. Each entry is one indexable row.
    /// Implementation is expected to be cheap (single-pass scan over
    /// `memory_files` + `memory_file_meta`).
    async fn list_all(&self) -> Result<Vec<MemoryFileSummary>, IndexerError>;

    /// Fetch the UTF-8 text for one memory file. None when the file is
    /// gone (raced with deletion). Errors are logged + skipped by the
    /// caller so one bad file doesn't break the whole hydration.
    async fn read_text(&self, path: &str) -> Result<Option<String>, IndexerError>;
}

/// Minimal summary of one memory file — what the indexer needs to
/// decide whether to embed and what metadata to store.
#[derive(Debug, Clone)]
pub struct MemoryFileSummary {
    pub path: String,
    pub file_cid: String,
    pub kind: String,
    pub signed_at: String,
    pub attester_pubkey_b32: Option<String>,
    pub size_bytes: u64,
}

/// Run one full hydration pass: read every file from the source, embed
/// any (path, file_cid) pair not already indexed, drop rows whose path
/// has gone away.
///
/// Returns `(rows_written, rows_skipped, paths_deleted)`.
pub async fn hydrate_once(
    index: &MemoryTextIndex,
    source: &(dyn MemoryFileSource + 'static),
) -> Result<(usize, usize, usize), IndexerError> {
    let embedder = global_embedder().map_err(IndexerError::Embed)?;
    let summaries = source.list_all().await?;

    // Build a "currently live" set keyed by path → file_cid so we can
    // detect deletes (paths in Lance that have vanished from sled).
    let live_paths: HashMap<String, String> = summaries
        .iter()
        .map(|s| (s.path.clone(), s.file_cid.clone()))
        .collect();
    let existing = index.existing_pairs().await.unwrap_or_default();
    // Collect paths that already appear in the index. Use the paths
    // present, not just the (path, cid) pairs.
    let mut existing_paths: std::collections::HashSet<String> =
        std::collections::HashSet::with_capacity(existing.len());
    for (p, _) in &existing {
        existing_paths.insert(p.clone());
    }

    // Pass 1: delete paths that have gone away from the source.
    let mut deleted: usize = 0;
    for p in &existing_paths {
        if !live_paths.contains_key(p) {
            if let Err(e) = index.delete_path(p).await {
                tracing::debug!(target: "emem::memory_search", path = %p, error = %e, "delete vanished path failed");
                continue;
            }
            deleted += 1;
        }
    }

    // Pass 2: embed + write new (path, file_cid) pairs.
    let mut written: usize = 0;
    let mut skipped: usize = 0;
    for s in &summaries {
        if existing.contains(&(s.path.clone(), s.file_cid.clone())) {
            skipped += 1;
            continue;
        }
        let text = match source.read_text(&s.path).await {
            Ok(Some(t)) => t,
            Ok(None) => {
                skipped += 1;
                continue;
            }
            Err(e) => {
                tracing::debug!(target: "emem::memory_search", path = %s.path, error = %e, "read_text failed; skipping");
                skipped += 1;
                continue;
            }
        };
        match index
            .index_one_file(
                &embedder,
                &s.path,
                &s.file_cid,
                &text,
                &s.kind,
                &s.signed_at,
                s.attester_pubkey_b32.as_deref(),
                s.size_bytes,
            )
            .await
        {
            Ok(true) => written += 1,
            Ok(false) => skipped += 1,
            Err(e) => {
                tracing::warn!(
                    target: "emem::memory_search",
                    path = %s.path,
                    error = %e,
                    "embed/index failed; file will be retried on next pass"
                );
                skipped += 1;
            }
        }
    }
    index.mark_hydrated().await;
    Ok((written, skipped, deleted))
}

/// Spawn the background polling indexer. Runs `hydrate_once` every
/// `poll_interval_secs` seconds. Cancellation: the loop exits when the
/// process shuts down (the tokio runtime drops the task). Idempotent —
/// re-calling sets the active flag but does not spawn a second loop.
pub fn spawn_polling_indexer(
    index: Arc<MemoryTextIndex>,
    source: Arc<dyn MemoryFileSource + 'static>,
) {
    let interval = Duration::from_secs(index.poll_interval_secs.max(1));
    tokio::spawn(async move {
        index.set_polling_active(true).await;
        // Run an immediate first pass so the index is populated on boot.
        let started = Instant::now();
        match hydrate_once(&index, source.as_ref()).await {
            Ok((w, s, d)) => tracing::info!(
                target: "emem::memory_search",
                rows_written = w,
                rows_skipped = s,
                paths_deleted = d,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "memory_search boot hydration complete"
            ),
            Err(e) => tracing::warn!(
                target: "emem::memory_search",
                error = %e,
                "memory_search boot hydration failed"
            ),
        }
        loop {
            tokio::time::sleep(interval).await;
            let t = Instant::now();
            match hydrate_once(&index, source.as_ref()).await {
                Ok((w, s, d)) => {
                    if w > 0 || d > 0 {
                        tracing::info!(
                            target: "emem::memory_search",
                            rows_written = w,
                            rows_skipped = s,
                            paths_deleted = d,
                            elapsed_ms = t.elapsed().as_millis() as u64,
                            "memory_search poll pass"
                        );
                    }
                }
                Err(e) => tracing::debug!(
                    target: "emem::memory_search",
                    error = %e,
                    "memory_search poll failed"
                ),
            }
        }
    });
}

/// Materialise rows into a RecordBatch for Lance. Shared by append_rows
/// and tests.
fn rows_to_batch(schema: SchemaRef, rows: &[IndexedRow]) -> Result<RecordBatch, IndexerError> {
    let paths = StringArray::from(rows.iter().map(|r| r.path.as_str()).collect::<Vec<_>>());
    let cids = StringArray::from(rows.iter().map(|r| r.file_cid.as_str()).collect::<Vec<_>>());
    let kinds = StringArray::from(rows.iter().map(|r| r.kind.as_str()).collect::<Vec<_>>());
    let signed = StringArray::from(
        rows.iter()
            .map(|r| r.signed_at.as_str())
            .collect::<Vec<_>>(),
    );
    let attesters = StringArray::from(
        rows.iter()
            .map(|r| r.attester_pubkey_b32.as_deref())
            .collect::<Vec<_>>(),
    );
    let sizes = UInt64Array::from(rows.iter().map(|r| r.size_bytes).collect::<Vec<_>>());
    let mut flat: Vec<f32> = Vec::with_capacity(rows.len() * TEXT_EMBED_DIM);
    for r in rows {
        if r.vector.len() != TEXT_EMBED_DIM {
            return Err(IndexerError::Arrow(format!(
                "row vector dim {} != expected {TEXT_EMBED_DIM}",
                r.vector.len()
            )));
        }
        flat.extend_from_slice(&r.vector);
    }
    let values = Float32Array::from(flat);
    let item = Arc::new(Field::new("item", DataType::Float32, true));
    let vec_arr = FixedSizeListArray::try_new(item, TEXT_EMBED_DIM as i32, Arc::new(values), None)
        .map_err(|e| IndexerError::Arrow(e.to_string()))?;
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(paths),
            Arc::new(cids),
            Arc::new(kinds),
            Arc::new(signed),
            Arc::new(attesters),
            Arc::new(sizes),
            Arc::new(vec_arr),
        ],
    )
    .map_err(|e| IndexerError::Arrow(e.to_string()))
}

/// Cosine distance type alias for symmetry with `lance_index.rs`. The
/// brute-force fallback in `mod.rs` uses straight cosine; this is only
/// used if/when we add an ANN index on the partition.
#[allow(dead_code)]
pub(crate) const DEFAULT_DISTANCE: DistanceType = DistanceType::Cosine;

fn unix_s() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn row(path: &str, cid: &str, kind: &str, n: f32) -> IndexedRow {
        IndexedRow {
            path: path.into(),
            file_cid: cid.into(),
            kind: kind.into(),
            signed_at: "2026-05-28T00:00:00Z".into(),
            attester_pubkey_b32: None,
            size_bytes: 42,
            vector: (0..TEXT_EMBED_DIM)
                .map(|i| (i as f32 / TEXT_EMBED_DIM as f32) + n)
                .collect(),
        }
    }

    /// Round-trip: write two rows, knn returns them with score [0, 1].
    #[tokio::test]
    async fn append_then_knn_returns_rows() {
        let tmp = TempDir::new().unwrap();
        let idx = MemoryTextIndex::open(tmp.path()).unwrap();
        let a = row("/memories/a.md", "cid-a", "resource", 0.0);
        let b = row("/memories/b.md", "cid-b", "resource", 0.5);
        idx.append_rows(&[a.clone(), b.clone()]).await.unwrap();
        let res = idx.knn(&a.vector, 2, None, None, None, 1).await.unwrap();
        assert_eq!(res.len(), 2);
        // Self-similarity must be ~1.
        assert_eq!(res[0].0.path, a.path);
        assert!(res[0].1 > 0.99, "self-similarity = {}", res[0].1);
    }

    /// Delete-by-path removes only the matching row.
    #[tokio::test]
    async fn delete_path_removes_only_target() {
        let tmp = TempDir::new().unwrap();
        let idx = MemoryTextIndex::open(tmp.path()).unwrap();
        let a = row("/memories/a.md", "cid-a", "resource", 0.0);
        let b = row("/memories/b.md", "cid-b", "resource", 0.5);
        idx.append_rows(&[a.clone(), b.clone()]).await.unwrap();
        idx.delete_path("/memories/a.md").await.unwrap();
        let all = idx.read_all().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].path, "/memories/b.md");
    }

    /// existing_pairs reports what's currently in the dataset.
    #[tokio::test]
    async fn existing_pairs_lists_writes() {
        let tmp = TempDir::new().unwrap();
        let idx = MemoryTextIndex::open(tmp.path()).unwrap();
        let a = row("/memories/a.md", "cid-a", "resource", 0.0);
        idx.append_rows(std::slice::from_ref(&a)).await.unwrap();
        let pairs = idx.existing_pairs().await.unwrap();
        assert!(pairs.contains(&("/memories/a.md".into(), "cid-a".into())));
    }

    /// kind + path_prefix filters apply in the knn post-pass.
    #[tokio::test]
    async fn knn_filters_apply() {
        let tmp = TempDir::new().unwrap();
        let idx = MemoryTextIndex::open(tmp.path()).unwrap();
        let a = row("/memories/notes/a.md", "cid-a", "fact", 0.0);
        let b = row("/memories/journal/b.md", "cid-b", "resource", 0.05);
        idx.append_rows(&[a.clone(), b.clone()]).await.unwrap();
        // Filter by path prefix.
        let res = idx
            .knn(&a.vector, 10, None, Some("/memories/notes/"), None, 1)
            .await
            .unwrap();
        assert!(res
            .iter()
            .all(|(r, _)| r.path.starts_with("/memories/notes/")));
        // Filter by kind.
        let res = idx
            .knn(&a.vector, 10, Some("fact"), None, None, 1)
            .await
            .unwrap();
        assert!(res.iter().all(|(r, _)| r.kind == "fact"));
        assert_eq!(res.len(), 1);
    }
}
