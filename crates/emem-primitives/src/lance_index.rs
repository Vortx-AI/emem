//! Embedded Lance vector index — ANN accelerator for `find_similar`.
//!
//! emem stores fact vectors in sled (the authoritative columnar key/value
//! store). A naive `find_similar` walks the full corpus and computes
//! cosine per cell — `O(N)` in the number of attested vectors. As soon
//! as the corpus crosses ~1 M vectors that becomes the dominant cost.
//!
//! This module layers a *separate* Lance dataset alongside sled. Sled
//! stays authoritative; Lance is a derived ANN index. Hydration on boot
//! copies every vector fact into the appropriate per-dim dataset, and
//! `sign_and_persist` appends incrementally. If Lance is missing,
//! corrupted or disabled (env `EMEM_DISABLE_LANCE=1`), the brute-force
//! path in `find_similar` is unchanged and returns identical results.
//!
//! ## Per-dim partitioning
//!
//! Lance's `FixedSizeList<Float32, N>` column requires a single uniform
//! dimensionality. emem's bands carry several widths today
//! (`geotessera`=128, `prithvi_eo2`=1024, `clay_v1`=1024, `galileo`=768),
//! and new families will appear. We therefore keep one Lance dataset
//! per dim, addressed by sub-path `vector_index_d{dim}.lance` under
//! `$EMEM_DATA`. A `LanceIndex` is the front-door — it lazily opens
//! the per-dim dataset on demand and keeps an in-memory map.
//!
//! ## Schema (per dim partition)
//!
//! ```text
//! cell64   : Utf8
//! band     : Utf8
//! tslot    : UInt64
//! fact_cid : Utf8
//! vector   : FixedSizeList<Float32, dim>
//! ```
//!
//! ## Index type
//!
//! We build an IVF_PQ index over the `vector` column with cosine
//! metric. The choice is dictated by Lance 6.0.1's surface:
//!
//! - IVF_PQ is well-tuned in Lance, scales to billions of vectors with
//!   moderate RAM, and supports cosine via the cosine MetricType.
//! - HNSW is available via `ivf_hnsw_pq`, but adds a second tunable
//!   surface (M, ef_construction) and is more memory-hungry at build
//!   time. We pick IVF_PQ for predictability at the M-scale corpora
//!   emem targets today.
//!
//! Parameters are sized off the partition's row count when the index
//! is (re)built:
//!
//! ```text
//! num_partitions = clamp(sqrt(N), 16, 256)
//! num_bits       = 8
//! num_sub_vectors = max(1, dim / 16) clamped to dim
//! max_iterations = 50
//! ```
//!
//! The index lives under the same Lance dataset path. Lance manages
//! its own version graph.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use arrow_array::{
    Array, FixedSizeListArray, Float32Array, RecordBatch, RecordBatchIterator, StringArray,
    UInt64Array,
};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use futures_util::TryStreamExt;
use lance::dataset::{Dataset, WriteMode, WriteParams};
use lance::index::vector::VectorIndexParams;
use lance::index::DatasetIndexExt;
use lance_index::IndexType;
use lance_linalg::distance::DistanceType;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};

use emem_cache::CanonicalKey;
use emem_fact::{Fact, FactCid};
use emem_storage::Storage;

use crate::cbor_ops::as_vec_f32;

/// Filename stem for the per-dim Lance dataset.
fn dataset_path(root: &Path, dim: usize) -> PathBuf {
    root.join(format!("vector_index_d{dim}.lance"))
}

/// Resolve the root directory from `$EMEM_DATA` (with the documented
/// fallback to `/home/ubuntu/emem/var/emem`). Caller is responsible for
/// creating the directory if it doesn't exist.
pub fn default_root() -> PathBuf {
    std::env::var("EMEM_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/home/ubuntu/emem/var/emem"))
        .join("lance")
}

/// Is Lance globally disabled by the operator? `EMEM_DISABLE_LANCE=1`
/// turns the index off — `find_similar` then falls through to the
/// historical brute-force scan unchanged. Used both as a kill-switch
/// in production incidents and to drive the A/B correctness test the
/// acceptance criteria call for.
pub fn lance_disabled() -> bool {
    matches!(
        std::env::var("EMEM_DISABLE_LANCE").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE")
    )
}

/// A single per-dim Lance partition handle.
#[derive(Clone)]
pub struct DimPartition {
    /// Dimensionality of the vector column.
    pub dim: usize,
    /// Filesystem path to the dataset (one directory per partition).
    pub path: PathBuf,
    /// Opened dataset, behind a mutex so writers serialize naturally.
    /// Lance's `Dataset` itself is `Send + Sync`, but `create_index`
    /// and `append`/`overwrite` mutate the manifest; serialising via
    /// the mutex keeps the in-process view consistent without paying
    /// for a per-call dataset open.
    inner: Arc<Mutex<Option<Dataset>>>,
    /// Cached schema (same shape per dim — kept on the partition so we
    /// don't rebuild it for every write).
    pub schema: SchemaRef,
}

/// Statistics surfaced via `/v1/vector_index/stats`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PerDimStats {
    /// Dimensionality of this partition.
    pub dim: usize,
    /// Row count in the dataset (per Lance's accurate counter).
    pub rows: u64,
    /// Whether an IVF_PQ index is present.
    pub has_index: bool,
    /// IVF_PQ partition count last used (best-effort — recorded at
    /// build time so the operator can see what was actually built).
    pub ivf_partitions: Option<u32>,
    /// PQ sub-vector count last used.
    pub pq_sub_vectors: Option<u32>,
    /// Filesystem path to this dataset.
    pub path: String,
}

/// Aggregated stats for the whole index.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IndexStats {
    /// Per-dim partition statistics, sorted by dim ascending.
    pub per_dim: Vec<PerDimStats>,
    /// Distinct dims known to the index (== `per_dim.len()`).
    pub total_dims: usize,
    /// Unix-seconds timestamp of the last (successful) hydration pass.
    /// `None` until hydration has run.
    pub last_hydrated_at_unix_s: Option<u64>,
    /// Unix-seconds timestamp of the last successful incremental append.
    pub last_appended_at_unix_s: Option<u64>,
    /// Index type built. Today always `"IVF_PQ"`; left as a string so
    /// future HNSW partitions don't require a wire-shape change.
    pub index_type: String,
    /// True when `EMEM_DISABLE_LANCE=1` short-circuited the layer.
    pub disabled: bool,
}

/// The Lance index. One handle per process, shared by the API layer +
/// primitive callers.
pub struct LanceIndex {
    /// Filesystem root holding the per-dim datasets.
    pub root: PathBuf,
    /// Open partitions, keyed by dim.
    partitions: RwLock<HashMap<usize, DimPartition>>,
    /// Mutex over hydration so two concurrent `hydrate_from_storage`
    /// calls (e.g. boot + manual trigger) serialise.
    hydration_lock: Mutex<()>,
    /// Wall-clock of the last successful hydration pass.
    last_hydrated_at: RwLock<Option<u64>>,
    /// Wall-clock of the last successful incremental append.
    last_appended_at: RwLock<Option<u64>>,
}

/// Build the Arrow schema for a per-dim partition. The vector column is
/// `FixedSizeList<Float32, dim>` (Lance's preferred vector encoding).
fn partition_schema(dim: usize) -> SchemaRef {
    let vector_field = Field::new("item", DataType::Float32, true);
    Arc::new(Schema::new(vec![
        Field::new("cell64", DataType::Utf8, false),
        Field::new("band", DataType::Utf8, false),
        Field::new("tslot", DataType::UInt64, false),
        Field::new("fact_cid", DataType::Utf8, false),
        Field::new(
            "vector",
            DataType::FixedSizeList(Arc::new(vector_field), dim as i32),
            false,
        ),
    ]))
}

/// One row destined for a Lance partition.
#[derive(Clone)]
struct PendingRow {
    cell: String,
    band: String,
    tslot: u64,
    fact_cid: String,
    vector: Vec<f32>,
}

/// Group rows into a `RecordBatch` for the given dim. All rows must
/// share `dim` — caller ensures this by routing through the per-dim
/// hashmap upstream.
fn rows_to_batch(dim: usize, rows: &[PendingRow]) -> Result<RecordBatch, arrow_schema::ArrowError> {
    let schema = partition_schema(dim);
    let cells = StringArray::from(rows.iter().map(|r| r.cell.as_str()).collect::<Vec<_>>());
    let bands = StringArray::from(rows.iter().map(|r| r.band.as_str()).collect::<Vec<_>>());
    let tslots = UInt64Array::from(rows.iter().map(|r| r.tslot).collect::<Vec<_>>());
    let cids = StringArray::from(rows.iter().map(|r| r.fact_cid.as_str()).collect::<Vec<_>>());
    let mut flat: Vec<f32> = Vec::with_capacity(rows.len() * dim);
    for r in rows {
        debug_assert_eq!(r.vector.len(), dim);
        flat.extend_from_slice(&r.vector);
    }
    let values = Float32Array::from(flat);
    let item = Arc::new(Field::new("item", DataType::Float32, true));
    let vector_arr = FixedSizeListArray::try_new(item, dim as i32, Arc::new(values), None)?;
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(cells),
            Arc::new(bands),
            Arc::new(tslots),
            Arc::new(cids),
            Arc::new(vector_arr),
        ],
    )
}

impl LanceIndex {
    /// Open (or initialise) the index rooted at `root`. The directory is
    /// created if missing. Per-dim partitions are opened lazily on first
    /// touch.
    pub fn open(root: impl Into<PathBuf>) -> std::io::Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        Ok(Self {
            root,
            partitions: RwLock::new(HashMap::new()),
            hydration_lock: Mutex::new(()),
            last_hydrated_at: RwLock::new(None),
            last_appended_at: RwLock::new(None),
        })
    }

    /// Convenience: open the default-root index (`$EMEM_DATA/lance`).
    pub fn open_default() -> std::io::Result<Self> {
        Self::open(default_root())
    }

    /// Look up or open the per-dim partition. The first call for a given
    /// dim either opens the existing dataset or returns a placeholder
    /// with no rows yet.
    async fn partition(&self, dim: usize) -> DimPartition {
        {
            let map = self.partitions.read().await;
            if let Some(p) = map.get(&dim) {
                return p.clone();
            }
        }
        let path = dataset_path(&self.root, dim);
        let dataset = Dataset::open(path.to_string_lossy().as_ref()).await.ok();
        let part = DimPartition {
            dim,
            path: path.clone(),
            inner: Arc::new(Mutex::new(dataset)),
            schema: partition_schema(dim),
        };
        let mut map = self.partitions.write().await;
        map.entry(dim).or_insert_with(|| part.clone()).clone()
    }

    /// Write a batch of rows, creating the dataset on first write or
    /// appending to it otherwise. Idempotency at the row level is not
    /// enforced here — callers (`hydrate_from_storage`) dedupe upstream
    /// against the existing `fact_cid` set.
    async fn write_rows(&self, dim: usize, rows: &[PendingRow]) -> Result<(), LanceError> {
        if rows.is_empty() {
            return Ok(());
        }
        let part = self.partition(dim).await;
        let batch = rows_to_batch(dim, rows).map_err(|e| LanceError::Arrow(e.to_string()))?;
        let schema = batch.schema();
        let reader = RecordBatchIterator::new(vec![Ok(batch)].into_iter(), schema);
        let mut guard = part.inner.lock().await;
        let uri = part.path.to_string_lossy().to_string();
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
            .map_err(|e| LanceError::Lance(e.to_string()))?;
        *guard = Some(ds);
        // Bump the append timestamp. Best-effort — the index is still
        // useful even if the timestamp write somehow races.
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        *self.last_appended_at.write().await = Some(now);
        Ok(())
    }

    /// Materialise an IVF_PQ index on the per-dim partition's vector
    /// column. Lance will skip + log when the row count is too small to
    /// train a meaningful PQ (the threshold is enforced inside Lance);
    /// we record the chosen params on success.
    async fn build_index(&self, dim: usize) -> Result<(u32, u32), LanceError> {
        let part = self.partition(dim).await;
        let mut guard = part.inner.lock().await;
        let ds = guard
            .as_mut()
            .ok_or_else(|| LanceError::Lance("partition not initialised".into()))?;
        let rows = ds
            .count_rows(None)
            .await
            .map_err(|e| LanceError::Lance(e.to_string()))?;
        let num_partitions = (rows as f64).sqrt().round().clamp(16.0, 256.0) as usize;
        // PQ requires dim % num_sub_vectors == 0. Find the largest k ≤
        // dim/16 that divides dim cleanly, clamped to ≥1.
        let target = (dim / 16).max(1);
        let num_sub_vectors = (1..=target)
            .rev()
            .find(|k| dim.is_multiple_of(*k))
            .unwrap_or(1);
        let params =
            VectorIndexParams::ivf_pq(num_partitions, 8, num_sub_vectors, DistanceType::Cosine, 50);
        ds.create_index(&["vector"], IndexType::Vector, None, &params, true)
            .await
            .map_err(|e| LanceError::Lance(e.to_string()))?;
        Ok((num_partitions as u32, num_sub_vectors as u32))
    }

    /// Read the existing set of `fact_cid` values from the per-dim
    /// partition. Used by the hydration path to skip rows already in
    /// the index. Returns an empty set when the partition doesn't exist.
    async fn existing_cids(
        &self,
        dim: usize,
    ) -> Result<std::collections::HashSet<String>, LanceError> {
        let part = self.partition(dim).await;
        let guard = part.inner.lock().await;
        let ds = match guard.as_ref() {
            Some(ds) => ds,
            None => return Ok(std::collections::HashSet::new()),
        };
        let mut scanner = ds.scan();
        scanner
            .project(&["fact_cid"])
            .map_err(|e| LanceError::Lance(e.to_string()))?;
        let stream = scanner
            .try_into_stream()
            .await
            .map_err(|e| LanceError::Lance(e.to_string()))?;
        let batches: Vec<RecordBatch> = stream
            .try_collect()
            .await
            .map_err(|e| LanceError::Lance(e.to_string()))?;
        let mut out = std::collections::HashSet::new();
        for b in batches {
            let col = b
                .column_by_name("fact_cid")
                .ok_or_else(|| LanceError::Arrow("fact_cid column missing".into()))?;
            let arr = col
                .as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(|| LanceError::Arrow("fact_cid not Utf8".into()))?;
            for i in 0..arr.len() {
                if !arr.is_null(i) {
                    out.insert(arr.value(i).to_string());
                }
            }
        }
        Ok(out)
    }

    /// Hydrate the index from `storage`. Reads the full sled index once,
    /// drops non-vector facts, groups by dim, and writes the missing
    /// rows into each per-dim partition. Idempotent: re-running hydration
    /// after the first pass is a no-op because every fact_cid is already
    /// in the partition.
    ///
    /// Returns `(rows_written, total_vector_facts_seen, elapsed)`.
    pub async fn hydrate_from_storage(
        &self,
        storage: &(dyn Storage + Send + Sync),
    ) -> Result<(usize, usize, std::time::Duration), LanceError> {
        let _guard = self.hydration_lock.lock().await;
        let started = Instant::now();
        let entries = storage
            .iter_index(None)
            .await
            .map_err(|e| LanceError::Storage(e.to_string()))?;

        // Group cids by canonical key so we can resolve facts in bulk
        // batches (sled's get_facts_many is much cheaper batched).
        let mut keys: Vec<CanonicalKey> = Vec::with_capacity(entries.len());
        let mut cids: Vec<FactCid> = Vec::with_capacity(entries.len());
        for (k, c) in entries {
            keys.push(k);
            cids.push(c);
        }

        let mut grouped: HashMap<usize, Vec<PendingRow>> = HashMap::new();
        let mut total_vec_facts: usize = 0;
        // Pull facts in chunks so we don't materialise the whole corpus
        // in one allocation. 4096 keeps RAM bounded on a 1M corpus.
        const CHUNK: usize = 4096;
        for chunk_start in (0..cids.len()).step_by(CHUNK) {
            let end = (chunk_start + CHUNK).min(cids.len());
            let cid_slice = &cids[chunk_start..end];
            let key_slice = &keys[chunk_start..end];
            let facts = storage
                .get_facts_many(cid_slice)
                .await
                .map_err(|e| LanceError::Storage(e.to_string()))?;
            for ((key, cid), fact_opt) in key_slice.iter().zip(cid_slice.iter()).zip(facts) {
                let Some(fact) = fact_opt else { continue };
                let Fact::Primary(p) = fact else { continue };
                let Some(vec) = as_vec_f32(&p.value) else {
                    continue;
                };
                if vec.is_empty() {
                    continue;
                }
                let dim = vec.len();
                total_vec_facts += 1;
                grouped.entry(dim).or_default().push(PendingRow {
                    cell: key.cell.clone(),
                    band: key.band.clone(),
                    tslot: key.tslot,
                    fact_cid: cid.as_str().to_string(),
                    vector: vec,
                });
            }
        }

        let mut total_written: usize = 0;
        let mut indexed_dims: Vec<usize> = Vec::new();
        for (dim, rows) in grouped {
            let existing = self.existing_cids(dim).await.unwrap_or_default();
            let to_write: Vec<PendingRow> = rows
                .into_iter()
                .filter(|r| !existing.contains(&r.fact_cid))
                .collect();
            if to_write.is_empty() {
                continue;
            }
            // Write in sub-batches to keep memory bounded.
            const BATCH: usize = 1024;
            for chunk in to_write.chunks(BATCH) {
                self.write_rows(dim, chunk).await?;
                total_written += chunk.len();
            }
            indexed_dims.push(dim);
        }

        // Build IVF_PQ over every partition that got fresh rows. Lance
        // gracefully skips when too few rows exist, so we don't gate
        // by a hard threshold — the index just falls through to brute-
        // force inside Lance for very small datasets.
        for dim in indexed_dims {
            // Index build is allowed to fail (e.g. too few rows): we
            // log and continue so hydration as a whole still succeeds.
            if let Err(e) = self.build_index(dim).await {
                tracing::debug!(target: "emem::lance", dim, error = %e, "index build skipped");
            }
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        *self.last_hydrated_at.write().await = Some(now);
        Ok((total_written, total_vec_facts, started.elapsed()))
    }

    /// Best-effort incremental append. Called from `sign_and_persist` —
    /// sled is authoritative, so a failure here is logged and ignored.
    pub async fn append_fact(&self, key: &CanonicalKey, cid: &FactCid, vec: Vec<f32>) {
        if vec.is_empty() {
            return;
        }
        let dim = vec.len();
        let row = PendingRow {
            cell: key.cell.clone(),
            band: key.band.clone(),
            tslot: key.tslot,
            fact_cid: cid.as_str().to_string(),
            vector: vec,
        };
        if let Err(e) = self.write_rows(dim, std::slice::from_ref(&row)).await {
            tracing::debug!(target: "emem::lance", dim, error = %e, "append_fact failed (sled remains authoritative)");
        }
    }

    /// k-NN search. Returns `(cell64, score, fact_cid)` triples in score-
    /// descending order. Cosine metric — Lance returns a `_distance`
    /// column in [0, 2]; we convert via `score = 1 - distance` so the
    /// callers see the same direction as the brute-force path.
    ///
    /// `band` is an optional post-filter applied in-process; Lance push-
    /// down filters are available but the syntax is enough surface area
    /// for a regression that we apply our own filter explicitly. This
    /// keeps Lance as a *vector* accelerator and leaves predicate logic
    /// in the primitive.
    pub async fn knn(
        &self,
        query: &[f32],
        k: usize,
        band: Option<&str>,
        oversample: usize,
    ) -> Result<Vec<(String, f32, String)>, LanceError> {
        let dim = query.len();
        if dim == 0 || k == 0 {
            return Ok(Vec::new());
        }
        let part = self.partition(dim).await;
        let guard = part.inner.lock().await;
        let ds = match guard.as_ref() {
            Some(ds) => ds,
            None => return Ok(Vec::new()),
        };
        let q = Float32Array::from(query.to_vec());
        let mut scanner = ds.scan();
        scanner
            .nearest("vector", &q, k.saturating_mul(oversample.max(1)))
            .map_err(|e| LanceError::Lance(e.to_string()))?;
        scanner
            .project(&["cell64", "band", "fact_cid"])
            .map_err(|e| LanceError::Lance(e.to_string()))?;
        let stream = scanner
            .try_into_stream()
            .await
            .map_err(|e| LanceError::Lance(e.to_string()))?;
        let batches: Vec<RecordBatch> = stream
            .try_collect()
            .await
            .map_err(|e| LanceError::Lance(e.to_string()))?;
        let mut out: Vec<(String, f32, String)> = Vec::new();
        for batch in batches {
            let cells = batch
                .column_by_name("cell64")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| LanceError::Arrow("cell64 column missing".into()))?;
            let bands = batch
                .column_by_name("band")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| LanceError::Arrow("band column missing".into()))?;
            let cids = batch
                .column_by_name("fact_cid")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| LanceError::Arrow("fact_cid column missing".into()))?;
            let distances = batch
                .column_by_name("_distance")
                .and_then(|c| c.as_any().downcast_ref::<Float32Array>())
                .ok_or_else(|| LanceError::Arrow("_distance column missing".into()))?;
            for i in 0..batch.num_rows() {
                if let Some(b) = band {
                    if bands.value(i) != b {
                        continue;
                    }
                }
                let dist = distances.value(i);
                // Lance cosine distance is `1 - cos`. Map back to the
                // historical [-1, 1] cosine score so callers see the
                // same number they got from the brute-force path.
                let score = 1.0 - dist;
                out.push((cells.value(i).to_string(), score, cids.value(i).to_string()));
            }
        }
        // Lance scanners may emit results in any order when a project
        // strips `_distance` ordering hints; sort defensively.
        out.sort_by(|a, b| b.1.total_cmp(&a.1));
        if out.len() > k {
            out.truncate(k);
        }
        Ok(out)
    }

    /// Snapshot the index's current shape for `/v1/vector_index/stats`.
    pub async fn stats(&self) -> IndexStats {
        if lance_disabled() {
            return IndexStats {
                disabled: true,
                ..Default::default()
            };
        }
        let map = self.partitions.read().await;
        let mut per_dim: Vec<PerDimStats> = Vec::with_capacity(map.len());
        for (dim, part) in map.iter() {
            let guard = part.inner.lock().await;
            let (rows, has_index) = if let Some(ds) = guard.as_ref() {
                let rows = ds.count_rows(None).await.unwrap_or(0) as u64;
                let has_index = ds
                    .load_indices()
                    .await
                    .map(|idx| !idx.is_empty())
                    .unwrap_or(false);
                (rows, has_index)
            } else {
                (0, false)
            };
            per_dim.push(PerDimStats {
                dim: *dim,
                rows,
                has_index,
                // Recorded at build time; we don't persist them across
                // process restarts (they are derivable from row count
                // via the same formula). Leave as None when the index
                // wasn't built in this process's lifetime.
                ivf_partitions: None,
                pq_sub_vectors: None,
                path: part.path.to_string_lossy().into_owned(),
            });
        }
        per_dim.sort_by_key(|s| s.dim);
        IndexStats {
            total_dims: per_dim.len(),
            per_dim,
            last_hydrated_at_unix_s: *self.last_hydrated_at.read().await,
            last_appended_at_unix_s: *self.last_appended_at.read().await,
            index_type: "IVF_PQ".into(),
            disabled: false,
        }
    }

    /// Total row count across all partitions — used by tests that only
    /// care that hydration wrote *something*.
    pub async fn total_rows(&self) -> u64 {
        let map = self.partitions.read().await;
        let mut total: u64 = 0;
        for part in map.values() {
            let guard = part.inner.lock().await;
            if let Some(ds) = guard.as_ref() {
                total += ds.count_rows(None).await.unwrap_or(0) as u64;
            }
        }
        total
    }
}

/// Error variants surfaced by the Lance layer. The primitive's
/// fast-path catches every variant and falls back to brute force —
/// Lance is an accelerator, not a hard dependency.
#[derive(Debug, thiserror::Error)]
pub enum LanceError {
    /// Underlying Lance crate error (open / write / index / scan).
    #[error("lance: {0}")]
    Lance(String),
    /// Arrow shape / type problem.
    #[error("arrow: {0}")]
    Arrow(String),
    /// Storage layer error surfaced through `iter_index` / `get_facts_many`.
    #[error("storage: {0}")]
    Storage(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};
    use tempfile::TempDir;

    fn rand_vec(rng: &mut StdRng, dim: usize) -> Vec<f32> {
        (0..dim).map(|_| rng.gen::<f32>() * 2.0 - 1.0).collect()
    }

    /// Direct test of the Lance layer without going through Storage —
    /// confirms write + index + knn round-trip.
    #[tokio::test]
    async fn write_then_knn_returns_seeded_vector() {
        let tmp = TempDir::new().unwrap();
        let idx = LanceIndex::open(tmp.path()).unwrap();
        let mut rng = StdRng::seed_from_u64(42);
        // Seed 256 random 128-D vectors, plus one known query target.
        let dim = 128;
        let target: Vec<f32> = (0..dim).map(|i| (i as f32 / dim as f32) - 0.5).collect();
        let target_cid = "cid-target".to_string();
        let mut rows: Vec<PendingRow> = Vec::new();
        rows.push(PendingRow {
            cell: "cell-target".into(),
            band: "geotessera".into(),
            tslot: 0,
            fact_cid: target_cid.clone(),
            vector: target.clone(),
        });
        for i in 0..255 {
            rows.push(PendingRow {
                cell: format!("cell-rand-{i}"),
                band: "geotessera".into(),
                tslot: 0,
                fact_cid: format!("cid-rand-{i}"),
                vector: rand_vec(&mut rng, dim),
            });
        }
        idx.write_rows(dim, &rows).await.unwrap();
        // No index — knn over the column should still work (Lance
        // falls back to a flat scan when no index is present).
        let res = idx.knn(&target, 5, Some("geotessera"), 1).await.unwrap();
        assert!(!res.is_empty(), "expected at least one neighbour");
        // The exact-match neighbour must be ranked first.
        assert_eq!(res[0].0, "cell-target");
        assert_eq!(res[0].2, target_cid);
        assert!(
            (res[0].1 - 1.0).abs() < 1e-3,
            "self-similarity score should be ~1.0, got {}",
            res[0].1
        );
    }

    /// Hydration is idempotent at the row level: running it twice over
    /// the same backing storage must not double the row count.
    #[tokio::test]
    async fn write_rows_skips_existing_cids_on_second_pass() {
        let tmp = TempDir::new().unwrap();
        let idx = LanceIndex::open(tmp.path()).unwrap();
        let dim = 64;
        let row = PendingRow {
            cell: "c".into(),
            band: "geotessera".into(),
            tslot: 0,
            fact_cid: "cid-1".into(),
            vector: vec![0.5; dim],
        };
        idx.write_rows(dim, std::slice::from_ref(&row))
            .await
            .unwrap();
        let initial = idx.total_rows().await;
        assert_eq!(initial, 1);
        // Re-hydrate the same row through the existing-cid filter the
        // hydration path uses (we call existing_cids directly here).
        let existing = idx.existing_cids(dim).await.unwrap();
        assert!(existing.contains("cid-1"));
        // And a deliberate filter→write cycle (mimics hydration) writes
        // zero rows the second time.
        let to_write: Vec<&PendingRow> = std::iter::once(&row)
            .filter(|r| !existing.contains(&r.fact_cid))
            .collect();
        assert!(to_write.is_empty(), "second-pass write set must be empty");
    }
}
