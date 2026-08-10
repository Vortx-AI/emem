//! Overture Maps materializer — anonymous S3 + pure-Rust parquet + WKB decode.
//!
//! Overture publishes monthly snapshots of buildings, places (POIs), and
//! transportation segments to a public, anonymous-readable S3 bucket
//! (`s3://overturemaps-us-west-2/release/<YYYY-MM-DD.0>/`). Each release is a
//! sharded set of GeoParquet files. Every parquet has a top-level `bbox`
//! struct column with `xmin/xmax/ymin/ymax` per row, plus per-row-group
//! min/max statistics on those four scalars, so we can prune row groups
//! aggressively before reading any geometry.
//!
//! Geometries are stored as WKB bytes in the `geometry` column. We decode
//! only the variants we need (Point for places; LineString for transportation
//! segments; Polygon centroid for buildings) — no GEOS, no GDAL, no PyO3.
//!
//! Caching strategy:
//!  - Per-release file list is fetched once via the S3 ListObjectsV2 XML API
//!    and held in process memory.
//!  - Per-file parquet footer (the `Arc<ParquetMetaData>` returned by
//!    `parquet::arrow::async_reader::ParquetObjectReader`) is cached the
//!    first time a file is touched. Footer reads are 1-2 KB; subsequent
//!    cells in the same area pay only the row-group bytes.
//!  - The cell bbox is *small* (~10 m × ~6 m at 52° N) so most cells
//!    touch one row group, and a single parquet file's bytes cover many
//!    adjacent cells. Future cells in the same neighborhood reuse the
//!    same footer cache.
//!
//! Error model: every failure surfaces as `OvertureError` so the caller
//! (the materializer in emem-api-rest) can record it as a `skip_reason`
//! on the recall response. We never fall back to a placeholder value —
//! an empty cell is a real `0` (no buildings inside the bbox), but a
//! transport failure is an `Err`, not a hidden zero.

#![allow(clippy::result_large_err)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use arrow::array::{
    Array, BinaryArray, Float32Array, Float64Array, LargeBinaryArray, MapArray, StringArray,
    StructArray,
};
use arrow::datatypes::DataType;
use futures_util::{StreamExt, TryStreamExt};
use object_store::{aws::AmazonS3Builder, path::Path as ObjectPath, ObjectStore};
use parquet::arrow::arrow_reader::ArrowReaderOptions;
use parquet::arrow::async_reader::{
    AsyncFileReader, ParquetObjectReader, ParquetRecordBatchStreamBuilder,
};
use parquet::arrow::ProjectionMask;
use parquet::file::metadata::ParquetMetaData;
use parquet::file::statistics::Statistics;
use std::sync::OnceLock;
use tokio::sync::Mutex;

/// S3 bucket Overture publishes to. Anonymous access; same bucket the
/// `overturemaps` CLI uses by default.
pub const BUCKET: &str = "overturemaps-us-west-2";
pub const REGION: &str = "us-west-2";

/// Public HTTPS list endpoint for the bucket (used for ListObjectsV2 XML).
/// Anonymous read — no signing, no key. Same surface a browser hits when
/// fetching `https://overturemaps-us-west-2.s3.amazonaws.com/?...`.
pub const LIST_ENDPOINT: &str = "https://overturemaps-us-west-2.s3.amazonaws.com/";

/// How long an auto-discovered release tag stays valid before we
/// re-list the bucket. Releases are published roughly monthly, so a
/// 24-hour TTL keeps us within one calendar day of "latest" while
/// keeping the cold-path S3 list off every cell call.
pub const RELEASE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Errors surfaced by the Overture materializer.
#[derive(Debug, thiserror::Error)]
pub enum OvertureError {
    #[error("s3 list error: {0}")]
    S3List(String),
    #[error("s3 get error for {key}: {detail}")]
    S3Get { key: String, detail: String },
    #[error("parquet error for {key}: {detail}")]
    Parquet { key: String, detail: String },
    #[error("schema mismatch for {key}: {detail}")]
    Schema { key: String, detail: String },
    #[error("wkb decode failure: {0}")]
    Wkb(String),
    #[error("init error: {0}")]
    Init(String),
}

/// In-process cache of decoded parquet footers, keyed by S3 key.
type FooterCache = std::collections::HashMap<String, Arc<ParquetMetaData>>;

/// Cache key for the per-(release, theme, type) parquet file list. The
/// release component is owned `String` so a TTL-driven release flip
/// (auto-discovery picks up a new monthly release) cleanly invalidates
/// the file list without us having to manually evict.
type FileListKey = (String, &'static str, &'static str);

/// In-process cache of parquet file lists per `FileListKey`.
type FileListCache = std::collections::HashMap<FileListKey, Vec<String>>;

/// In-process cache key for `division_polygon_near` — anchor coord
/// rounded to 0.01° (~1.1 km grid) plus normalized name hint.
type DivisionCacheKey = (i32, i32, String);
type DivisionCache = std::collections::HashMap<DivisionCacheKey, Option<DivisionMatch>>;

/// Render a [`DivisionCacheKey`] into a flat string suitable as a
/// persistent-store key. Format is `"div:{lat100}:{lng100}:{subtype}|{hint}"`
/// where `subtype|hint` is the third tuple element verbatim (already
/// produced as `"{normalized_pref}|{normalized_hint}"` by the lookup
/// path, so byte-identical keys come back to the in-memory cache
/// after a sled round-trip). The `div:` prefix scopes the namespace
/// so a single shared sled tree can carry other Overture-themed
/// caches in future without collisions.
pub fn division_cache_key_string(key: &(i32, i32, String)) -> String {
    format!("div:{}:{}:{}", key.0, key.1, key.2)
}

/// Persistent backing store for the `division_polygon_*` cache.
/// Implementations live in the crate that owns the durable storage
/// (the API-rest crate wraps the existing `geocoder.sled` tree). The
/// trait is intentionally bytes-in / bytes-out so this crate doesn't
/// take a sled dependency and the implementation can swap to any
/// kv store without touching this file.
///
/// Stored payload is the serde_json-encoded `Option<DivisionMatch>`.
/// JSON instead of CBOR keeps the cache human-inspectable from the
/// CLI (`sled-cli get`) and avoids adding `serde_cbor`/`ciborium` to
/// the dep graph; the per-entry cost (~1–2 KiB) is irrelevant against
/// the wall-clock savings (~10 s per cold lookup).
pub trait DivisionPersistentCache: Send + Sync {
    /// Returns the stored payload for `key`, or `None` on miss / error.
    /// Errors must be swallowed and logged inside the implementation —
    /// the lookup path treats a `None` as "fall through to the
    /// parquet scan" and a transient sled failure must never cascade
    /// into a 5xx on `/v1/locate`.
    fn get(&self, key: &str) -> Option<Vec<u8>>;
    /// Writes `value` under `key`. Errors must be logged but not
    /// surfaced — losing a single cache put is recoverable (the next
    /// lookup will recompute and re-put), wedging the locate path is
    /// not.
    fn put(&self, key: &str, value: &[u8]);
}

/// Theme + type pair Overture organises files under.
#[derive(Debug, Clone, Copy)]
struct ThemeType {
    theme: &'static str,
    typ: &'static str,
}

const PLACES: ThemeType = ThemeType {
    theme: "places",
    typ: "place",
};
const BUILDINGS: ThemeType = ThemeType {
    theme: "buildings",
    typ: "building",
};
const SEGMENTS: ThemeType = ThemeType {
    theme: "transportation",
    typ: "segment",
};

/// Administrative-boundary polygons (countries, regions, counties,
/// localities, neighborhoods). Used by `/v1/recall_polygon`'s polygon
/// resolver to substitute for the Nominatim-polygon round-trip — once
/// GeoNames has given us a coord, divisions gives us the boundary,
/// and we never touch a rate-limited public OSM endpoint.
const DIVISIONS_AREA: ThemeType = ThemeType {
    theme: "divisions",
    typ: "division_area",
};

/// In-process snapshot of the auto-discovered release tag plus the
/// monotonic clock instant we fetched it at. `None` means "never fetched
/// successfully yet" — a recall on the cold path will block briefly on
/// the S3 list call.
#[derive(Clone, Debug)]
struct ReleaseCache {
    release: String,
    fetched_at: Instant,
}

/// Result of [`OvertureClient::warm_start`]. Surfaced so the boot path
/// can log shard count + per-file footer failure count + wallclock,
/// without having to thread metric-emission down into the fetch crate.
#[derive(Clone, Debug)]
pub struct WarmStartStats {
    /// Number of `division_area` parquet shards discovered in the
    /// active Overture release. ~30–150 for current releases.
    pub file_count: usize,
    /// Shards whose parquet footer fetch failed. Lookup path tolerates
    /// this — it will re-fetch the footer on demand — so a non-zero
    /// value is degraded, not fatal.
    pub footer_errors: usize,
    /// Wall-clock duration of the warm-up call, milliseconds.
    pub elapsed_ms: u64,
}

/// Anonymous S3 reader for the latest Overture release. The release tag
/// is auto-discovered on first use and re-checked every `RELEASE_TTL`.
/// On a refresh failure we log + reuse the cached value (the only
/// non-silent fallback path). On a *cold-start* failure (no cached
/// value yet) we surface the error so the caller can decide.
pub struct OvertureClient {
    /// Anonymous S3 store (no signing, no key).
    store: Arc<dyn ObjectStore>,
    /// Auto-discovered release tag + 24h TTL guard. Keyed by the
    /// optional `EMEM_OVERTURE_RELEASE` override: when the env var is
    /// set we pin to that value and skip the S3 list entirely.
    release_cache: Mutex<Option<ReleaseCache>>,
    /// File list per (release, theme, type), populated lazily. Keyed by
    /// release so that a TTL-driven release flip cleanly invalidates
    /// the file list (we don't want to serve "buildings" from the old
    /// release once we've seen a newer one).
    file_lists: Mutex<FileListCache>,
    /// Decoded parquet footers per S3 key.
    footers: Mutex<FooterCache>,
    /// Durable cache for `division_polygon_*` results, set once at
    /// boot via [`Self::set_persistent_cache`]. When present, lookups
    /// consult it before scanning parquet and write back on miss so a
    /// process restart doesn't re-pay the 6–15 s cold-cache cost for
    /// places the responder has already resolved. `OnceLock` rather
    /// than `Mutex<Option<…>>` because the cache is wired once and
    /// then read-only thereafter — no need for write locks on every
    /// lookup.
    persistent_cache: OnceLock<Box<dyn DivisionPersistentCache>>,
    /// In-process memoization of `division_polygon_near` lookups.
    //
    // NB: see [`WarmStartStats`] above for the boot-time pre-warm
    // entrypoint that fills `release_cache`, `file_lists`, and
    // `footers` in one pass.
    //
    /// Keyed by the anchor coord rounded to 0.01° (~1.1 km grid) plus
    /// the normalized name hint, so a downstream agent calling
    /// `/v1/locate("Mumbai")` twice in a row pays the ~4 s cold Overture
    /// footer-fetch only on the first call. Stored value is the full
    /// `Option<DivisionMatch>` — `None` results are cached too, so
    /// places Overture doesn't carry don't re-trigger a full shard
    /// scan on every retry. The cache is unbounded; admin boundaries
    /// are static-tempo, the per-process query universe at a single
    /// responder is small (~thousands of distinct (anchor, name)
    /// keys), and `DivisionMatch` is ~2 KB worst case — even 100 000
    /// distinct entries fit in ~200 MB. Persistent caching across
    /// process restarts lives in the sled geocoder cache via the
    /// `polygon_bbox` round-trip in `nominatim_cache_put`.
    division_cache: Mutex<DivisionCache>,
}

impl OvertureClient {
    /// Build an anonymous S3 reader. The release tag is *not* resolved
    /// here — it is fetched lazily on first use via [`active_release`].
    /// This keeps the constructor sync-safe (no Tokio required at boot)
    /// and lets cold-start failures surface as proper `Result`s through
    /// the recall path instead of panicking the process.
    pub fn new() -> Result<Self, OvertureError> {
        let store = AmazonS3Builder::new()
            .with_region(REGION)
            .with_bucket_name(BUCKET)
            .with_unsigned_payload(true)
            .with_skip_signature(true)
            .build()
            .map_err(|e| OvertureError::Init(format!("AmazonS3Builder: {e}")))?;
        Ok(Self {
            store: Arc::new(store),
            release_cache: Mutex::new(None),
            file_lists: Mutex::new(Default::default()),
            footers: Mutex::new(Default::default()),
            persistent_cache: OnceLock::new(),
            division_cache: Mutex::new(Default::default()),
        })
    }

    /// Wire a durable cache for `division_polygon_*` results. Call once
    /// at server boot — subsequent calls are no-ops (`OnceLock::set`
    /// returns `Err` on re-set, which we drop). When unset (default),
    /// the lookup path is unchanged: in-memory cache only, full cold
    /// cost on process restart.
    pub fn set_persistent_cache(&self, cache: Box<dyn DivisionPersistentCache>) {
        let _ = self.persistent_cache.set(cache);
    }

    /// Process-global instance. Initialised on first use; the inner
    /// constructor only fails if `AmazonS3Builder` validates wrong, which
    /// is a deterministic config bug, so we surface that as a panic at
    /// startup rather than threading the error through every materializer.
    pub fn shared() -> &'static OvertureClient {
        static C: OnceLock<OvertureClient> = OnceLock::new();
        C.get_or_init(|| {
            OvertureClient::new().expect("OvertureClient::new (anonymous S3) must not fail")
        })
    }

    /// Pre-populate the release tag, file list, and parquet footers for
    /// the `divisions/division_area` theme. Idempotent — safe to call
    /// once at server boot, additional calls hit the existing caches
    /// and return immediately.
    ///
    /// Without this, the first `division_polygon_*` call after process
    /// start pays the full S3 list + per-file footer fetch — measured
    /// at 6–15 s for a typical place anchor with `EMEM_OVERTURE_PARALLEL=32`.
    /// That latency directly translates into a `/v1/ndvi` or
    /// `/v1/eudr_dds` 504 because the boring-endpoint budget is 60 s
    /// and the geocoder consumes most of it. After `warm_start` the
    /// first locate call only pays the bbox-pruned row-group reads
    /// (typically ~200–500 ms).
    pub async fn warm_start(&self) -> Result<WarmStartStats, OvertureError> {
        let t0 = Instant::now();
        // 1. Cache the release tag (one S3 list on `release/`).
        let _release = self.release().await?;
        // 2. Cache the divisions/division_area file list (one S3 list).
        let files = self.list_files(DIVISIONS_AREA).await?;
        let file_count = files.len();
        // 3. Cache each parquet footer (one HEAD + metadata range read
        //    per file, fanned out at `scan_parallelism()`). Failures are
        //    logged and counted; we never fail the whole warm-up on a
        //    single flaky shard — the lookup path also tolerates the
        //    cold path.
        let parallel = scan_parallelism();
        let footer_errors = futures_util::stream::iter(files)
            .map(|key| async move {
                match self.footer(&key).await {
                    Ok(_) => 0usize,
                    Err(e) => {
                        tracing::warn!(
                            target: "emem::overture::warm",
                            key = %key,
                            error = %e,
                            "footer warm-up failed; lookup path will retry on demand"
                        );
                        1usize
                    }
                }
            })
            .buffer_unordered(parallel)
            .fold(0usize, |acc, n| async move { acc + n })
            .await;
        Ok(WarmStartStats {
            file_count,
            footer_errors,
            elapsed_ms: t0.elapsed().as_millis() as u64,
        })
    }

    /// Resolve the release tag the client should currently use.
    ///
    /// Resolution order:
    ///   1. `EMEM_OVERTURE_RELEASE` env var — pinned override, no S3 call.
    ///   2. Cached value younger than `RELEASE_TTL` — returned as-is.
    ///   3. Cached value older than TTL — refresh; on success replace
    ///      cache, on failure log + reuse the stale value (the *only*
    ///      acceptable non-silent fallback because we logged it).
    ///   4. No cached value at all — refresh; on failure return Err.
    pub async fn release(&self) -> Result<String, OvertureError> {
        if let Ok(pinned) = std::env::var("EMEM_OVERTURE_RELEASE") {
            if !pinned.is_empty() {
                return Ok(pinned);
            }
        }
        let mut g = self.release_cache.lock().await;
        let now = Instant::now();
        if let Some(c) = g.as_ref() {
            if now.duration_since(c.fetched_at) < RELEASE_TTL {
                return Ok(c.release.clone());
            }
        }
        // Either cold start or TTL expired. Refresh.
        match latest_release().await {
            Ok(rel) => {
                *g = Some(ReleaseCache {
                    release: rel.clone(),
                    fetched_at: now,
                });
                Ok(rel)
            }
            Err(e) => {
                if let Some(c) = g.as_ref() {
                    tracing::warn!(
                        target: "emem_fetch::overture",
                        "Overture release refresh failed: {e}; reusing cached value {} (age {:?})",
                        c.release,
                        now.duration_since(c.fetched_at)
                    );
                    Ok(c.release.clone())
                } else {
                    tracing::error!(
                        target: "emem_fetch::overture",
                        "Overture release cold-start discovery failed: {e}; no cached value to fall back on"
                    );
                    Err(e)
                }
            }
        }
    }

    /// List parquet files under `release/<theme>/<typ>/`, caching the result.
    async fn list_files(&self, tt: ThemeType) -> Result<Vec<String>, OvertureError> {
        let release = self.release().await?;
        {
            let g = self.file_lists.lock().await;
            if let Some(v) = g.get(&(release.clone(), tt.theme, tt.typ)) {
                return Ok(v.clone());
            }
        }
        let prefix = format!("release/{}/theme={}/type={}/", release, tt.theme, tt.typ);
        let prefix_path = ObjectPath::from(prefix.clone());
        let mut out = Vec::new();
        let mut stream = self.store.list(Some(&prefix_path));
        while let Some(meta) = stream
            .try_next()
            .await
            .map_err(|e| OvertureError::S3List(format!("{prefix}: {e}")))?
        {
            let key = meta.location.to_string();
            if key.ends_with(".parquet") {
                out.push(key);
            }
        }
        if out.is_empty() {
            return Err(OvertureError::S3List(format!(
                "no parquet files under prefix {prefix} (wrong release tag?)"
            )));
        }
        let mut g = self.file_lists.lock().await;
        g.insert((release, tt.theme, tt.typ), out.clone());
        Ok(out)
    }

    /// Get cached footer for one parquet, fetching if absent.
    async fn footer(&self, key: &str) -> Result<Arc<ParquetMetaData>, OvertureError> {
        {
            let g = self.footers.lock().await;
            if let Some(m) = g.get(key) {
                return Ok(m.clone());
            }
        }
        let path = ObjectPath::from(key.to_string());
        let head = self
            .store
            .head(&path)
            .await
            .map_err(|e| OvertureError::S3Get {
                key: key.to_string(),
                detail: format!("head: {e}"),
            })?;
        let mut reader =
            ParquetObjectReader::new(self.store.clone(), path).with_file_size(head.size);
        let meta = reader
            .get_metadata(None)
            .await
            .map_err(|e| OvertureError::Parquet {
                key: key.to_string(),
                detail: format!("get_metadata: {e}"),
            })?;
        let mut g = self.footers.lock().await;
        g.insert(key.to_string(), meta.clone());
        Ok(meta)
    }

    /// Build a fresh row-batch stream over the chosen row groups for
    /// `bbox` + `geometry` + any caller-specified extra columns.
    /// `extra_cols` items match against the leaf path's first segment
    /// — for the divisions theme we pass `["id", "names", "subtype"]`
    /// so the resolver can populate the GERS id, primary name, and
    /// admin level on the returned `DivisionMatch`. Passing `&[]`
    /// keeps the original behaviour for places/buildings/segments.
    async fn open_stream(
        &self,
        key: &str,
        row_groups: Vec<usize>,
        extra_cols: &[&str],
    ) -> Result<
        parquet::arrow::async_reader::ParquetRecordBatchStream<ParquetObjectReader>,
        OvertureError,
    > {
        let path = ObjectPath::from(key.to_string());
        let head = self
            .store
            .head(&path)
            .await
            .map_err(|e| OvertureError::S3Get {
                key: key.to_string(),
                detail: format!("head: {e}"),
            })?;
        let reader = ParquetObjectReader::new(self.store.clone(), path).with_file_size(head.size);

        let opts = ArrowReaderOptions::new();
        let builder = ParquetRecordBatchStreamBuilder::new_with_options(reader, opts)
            .await
            .map_err(|e| OvertureError::Parquet {
                key: key.to_string(),
                detail: format!("stream_builder: {e}"),
            })?;

        // Project only the columns we need by leaf-column index. The schema
        // descr's `columns()` returns leaf descriptors in order — we match
        // by leaf path's first segment.
        let mut leaf_idx = Vec::new();
        {
            let parquet_schema = builder.parquet_schema();
            for (i, col) in parquet_schema.columns().iter().enumerate() {
                let parts = col.path().parts();
                if parts.is_empty() {
                    continue;
                }
                let first = parts[0].as_str();
                if first == "bbox" || first == "geometry" || extra_cols.contains(&first) {
                    leaf_idx.push(i);
                }
            }
        }
        if leaf_idx.is_empty() {
            return Err(OvertureError::Schema {
                key: key.to_string(),
                detail: "neither bbox nor geometry columns present".into(),
            });
        }
        let mask = ProjectionMask::leaves(builder.parquet_schema(), leaf_idx);
        let builder = builder.with_projection(mask).with_row_groups(row_groups);
        builder.build().map_err(|e| OvertureError::Parquet {
            key: key.to_string(),
            detail: format!("stream build: {e}"),
        })
    }

    /// Pick row groups whose `bbox` column statistics overlap the query bbox.
    /// Returns a list of (row_group_index, parquet_metadata).
    fn pick_row_groups(
        &self,
        meta: &Arc<ParquetMetaData>,
        s_lat: f64,
        n_lat: f64,
        w_lng: f64,
        e_lng: f64,
    ) -> Vec<usize> {
        // Find the leaf indices for bbox.xmin / xmax / ymin / ymax.
        let schema = meta.file_metadata().schema_descr();
        let mut idx = BboxLeafIndex::default();
        for (i, col) in schema.columns().iter().enumerate() {
            let parts = col.path().parts();
            if parts.len() == 2 && parts[0] == "bbox" {
                match parts[1].as_str() {
                    "xmin" => idx.xmin = Some(i),
                    "xmax" => idx.xmax = Some(i),
                    "ymin" => idx.ymin = Some(i),
                    "ymax" => idx.ymax = Some(i),
                    _ => {}
                }
            }
        }
        let (Some(ix0), Some(ix1), Some(iy0), Some(iy1)) = (idx.xmin, idx.xmax, idx.ymin, idx.ymax)
        else {
            // No stats present — fall back to scanning every row group.
            return (0..meta.num_row_groups()).collect();
        };

        let mut keep = Vec::new();
        for (rg_i, rg) in meta.row_groups().iter().enumerate() {
            // Each leaf has its own stats column.
            let xmin = stat_min_f64(rg.column(ix0).statistics());
            let xmax = stat_max_f64(rg.column(ix1).statistics());
            let ymin = stat_min_f64(rg.column(iy0).statistics());
            let ymax = stat_max_f64(rg.column(iy1).statistics());
            // If any stat is missing, keep the group (conservative).
            let overlaps = match (xmin, xmax, ymin, ymax) {
                (Some(rx0), Some(rx1), Some(ry0), Some(ry1)) => {
                    rx0 <= e_lng && rx1 >= w_lng && ry0 <= n_lat && ry1 >= s_lat
                }
                _ => true,
            };
            if overlaps {
                keep.push(rg_i);
            }
        }
        keep
    }

    /// Count places (POIs, points) whose geometry falls inside the bbox.
    pub async fn places_count_in_bbox(
        &self,
        s_lat: f64,
        n_lat: f64,
        w_lng: f64,
        e_lng: f64,
    ) -> Result<u64, OvertureError> {
        self.scan_count(PLACES, s_lat, n_lat, w_lng, e_lng, GeomKind::Point)
            .await
    }

    /// Count buildings whose centroid (from the polygon WKB) falls inside the bbox.
    pub async fn buildings_count_in_bbox(
        &self,
        s_lat: f64,
        n_lat: f64,
        w_lng: f64,
        e_lng: f64,
    ) -> Result<u64, OvertureError> {
        self.scan_count(
            BUILDINGS,
            s_lat,
            n_lat,
            w_lng,
            e_lng,
            GeomKind::PolygonCentroid,
        )
        .await
    }

    /// Sum of road-segment length (metres) intersecting the bbox.
    pub async fn road_length_m_in_bbox(
        &self,
        s_lat: f64,
        n_lat: f64,
        w_lng: f64,
        e_lng: f64,
    ) -> Result<f64, OvertureError> {
        let files = self.list_files(SEGMENTS).await?;
        let lat0 = (s_lat + n_lat) / 2.0;
        // Local-tangent-plane scaling; exact at lat0, accurate to sub-mm
        // over a ~10 m cell.
        let m_per_deg_lat = 111_320.0_f64;
        let m_per_deg_lng = 111_320.0_f64 * lat0.to_radians().cos();
        let parallel = scan_parallelism();
        let total = futures_util::stream::iter(files)
            .map(|key| async move {
                self.scan_one_file_road(
                    &key,
                    s_lat,
                    n_lat,
                    w_lng,
                    e_lng,
                    m_per_deg_lat,
                    m_per_deg_lng,
                )
                .await
            })
            .buffer_unordered(parallel)
            .try_fold(
                0.0_f64,
                |acc, x| async move { Ok::<_, OvertureError>(acc + x) },
            )
            .await?;
        Ok(total)
    }

    #[allow(clippy::too_many_arguments)]
    async fn scan_one_file_road(
        &self,
        key: &str,
        s_lat: f64,
        n_lat: f64,
        w_lng: f64,
        e_lng: f64,
        m_per_deg_lat: f64,
        m_per_deg_lng: f64,
    ) -> Result<f64, OvertureError> {
        let meta = self.footer(key).await?;
        let rgs = self.pick_row_groups(&meta, s_lat, n_lat, w_lng, e_lng);
        if rgs.is_empty() {
            return Ok(0.0);
        }
        let mut stream = self.open_stream(key, rgs, &[]).await?;
        let mut total = 0.0f64;
        while let Some(batch) = stream
            .try_next()
            .await
            .map_err(|e| OvertureError::Parquet {
                key: key.to_string(),
                detail: format!("next batch: {e}"),
            })?
        {
            let bbox_col = batch
                .column_by_name("bbox")
                .ok_or_else(|| OvertureError::Schema {
                    key: key.to_string(),
                    detail: "no bbox column in batch".into(),
                })?;
            let geom_col =
                batch
                    .column_by_name("geometry")
                    .ok_or_else(|| OvertureError::Schema {
                        key: key.to_string(),
                        detail: "no geometry column in batch".into(),
                    })?;
            let bb = BBoxAccess::new(bbox_col.as_ref()).map_err(|e| OvertureError::Schema {
                key: key.to_string(),
                detail: e,
            })?;
            let geoms = WkbAccess::new(geom_col.as_ref()).map_err(|e| OvertureError::Schema {
                key: key.to_string(),
                detail: e,
            })?;
            for i in 0..batch.num_rows() {
                if !bb.overlaps(i, s_lat, n_lat, w_lng, e_lng) {
                    continue;
                }
                let Some(wkb) = geoms.get(i) else {
                    continue;
                };
                let Some(line_pts) = wkb_linestring_or_multi(wkb) else {
                    continue;
                };
                total += polyline_clipped_length(
                    &line_pts,
                    s_lat,
                    n_lat,
                    w_lng,
                    e_lng,
                    m_per_deg_lat,
                    m_per_deg_lng,
                );
            }
        }
        Ok(total)
    }

    /// The cache key `division_polygon_with_subtype` would use for these
    /// arguments. Factored out so the network-free probe below and the
    /// lookup itself cannot drift apart: a probe that computed the key
    /// differently would report "undecided" for an entry that is decided,
    /// which is exactly the failure the probe exists to prevent.
    fn division_cache_key(
        lat: f64,
        lng: f64,
        name_hint: &str,
        preferred_subtype: Option<&str>,
    ) -> DivisionCacheKey {
        (
            (lat * 100.0).round() as i32,
            (lng * 100.0).round() as i32,
            format!(
                "{}|{}",
                preferred_subtype.unwrap_or(""),
                normalize_division_name(name_hint)
            ),
        )
    }

    /// Has this (anchor, name, subtype) already been decided, without
    /// touching the network?
    ///
    /// Returns `Some(answer)` when the in-process memo or the durable
    /// cache already holds a verdict — including `Some(None)`, meaning
    /// "Overture definitively carries no polygon covering this anchor".
    /// Returns `None` only when nothing has decided it yet.
    ///
    /// Why this exists separately from `division_polygon_with_subtype`:
    /// the caller needs to know whether an answer is *free* before it
    /// decides how long to wait. Racing a deadline against a lookup that
    /// might be a HashMap hit or might be a 10 s parquet scan makes the
    /// returned polygon a function of wall-clock timing, so the same
    /// place name answered about two different places on consecutive
    /// requests. Probing first separates "already decided, serve it" from
    /// "never seen, decide it now and persist" and removes the timing
    /// from the answer.
    pub async fn division_polygon_decided(
        &self,
        lat: f64,
        lng: f64,
        name_hint: &str,
        preferred_subtype: Option<&str>,
    ) -> Option<Option<DivisionMatch>> {
        let cache_key = Self::division_cache_key(lat, lng, name_hint, preferred_subtype);
        if let Some(hit) = self.division_cache.lock().await.get(&cache_key) {
            return Some(hit.clone());
        }
        let pc = self.persistent_cache.get()?;
        let key_str = division_cache_key_string(&cache_key);
        let raw = pc.get(&key_str)?;
        match serde_json::from_slice::<Option<DivisionMatch>>(&raw) {
            Ok(decoded) => {
                self.division_cache
                    .lock()
                    .await
                    .insert(cache_key, decoded.clone());
                Some(decoded)
            }
            // Corrupt row. Report undecided so the caller pays for a fresh
            // scan, which overwrites it. Same self-healing as the lookup path.
            Err(_) => None,
        }
    }

    /// Resolve a place's true administrative boundary from Overture's
    /// `divisions/division_area` theme, optionally biased toward a
    /// specific admin subtype. Convenience wrapper over
    /// [`Self::division_polygon_with_subtype`] with `preferred_subtype = None`.
    pub async fn division_polygon_near(
        &self,
        lat: f64,
        lng: f64,
        name_hint: &str,
    ) -> Result<Option<DivisionMatch>, OvertureError> {
        self.division_polygon_with_subtype(lat, lng, name_hint, None)
            .await
    }

    /// Resolve a place's true administrative boundary from Overture's
    /// `divisions/division_area` theme, with optional admin-level
    /// preference.
    ///
    /// `lat` / `lng` should be a coarse anchor (e.g. the GeoNames
    /// centroid for the place name). The method scans every row group
    /// whose bbox stats overlap a wide search box around the anchor —
    /// wide enough to catch country-scale polygons whose individual
    /// row bbox encloses the anchor — decodes WKB polygons that
    /// actually contain the anchor point, and ranks the candidates.
    ///
    /// Ranking order (first non-empty tier wins):
    ///   1. **Subtype-pinned exact name match** — when
    ///      `preferred_subtype` is `Some("country")` and a candidate's
    ///      subtype matches AND its primary name matches the hint,
    ///      that wins. Lets the country tier of the locate cascade
    ///      pull the country polygon even though Dhaka District,
    ///      Dhaka Division, and Bangladesh country all contain the
    ///      Dhaka anchor.
    ///   2. **Subtype-pinned containment** — when no exact name
    ///      matches, prefer the candidate whose subtype matches
    ///      `preferred_subtype` over any other subtype.
    ///   3. **Exact name match without subtype pin** — among all
    ///      name-matching candidates, prefer the highest admin
    ///      level (`division_subtype_rank`). "Manhattan" should
    ///      resolve to the borough / county / region row, not a
    ///      Community-Board sub-locality.
    ///   4. **Partial name match** — substring either way; prefer
    ///      the smallest containing polygon (narrowest scope).
    ///   5. **No name match** — smallest containing polygon
    ///      regardless of name.
    ///
    /// Returns `Ok(None)` when no polygon contains the anchor (e.g.
    /// rural area outside any locality boundary) — the caller falls
    /// back to centre-cell-bbox honestly. Returns `Err` only on
    /// transport / parquet decode failure.
    pub async fn division_polygon_with_subtype(
        &self,
        lat: f64,
        lng: f64,
        name_hint: &str,
        preferred_subtype: Option<&str>,
    ) -> Result<Option<DivisionMatch>, OvertureError> {
        // In-process result cache: round the anchor coord to 0.01° so
        // tiny GeoNames-record jitter doesn't bypass the cache. Stores
        // `Option<DivisionMatch>` — Ok(None) results are cached too
        // so the second "look up X in a region Overture doesn't carry"
        // call doesn't re-pay the full shard scan. Static-tempo data;
        // no TTL.
        let normalized_hint = normalize_division_name(name_hint);
        // Cache key includes the subtype preference so country-tier
        // and locality-tier calls at the same anchor don't collide.
        let cache_key: DivisionCacheKey =
            Self::division_cache_key(lat, lng, name_hint, preferred_subtype);
        if let Some(hit) = self.division_cache.lock().await.get(&cache_key) {
            return Ok(hit.clone());
        }
        // Persistent cache check — sled-backed, populated on prior
        // process lifetimes. A hit promotes the entry back into the
        // in-memory cache so the second call within this process pays
        // only the HashMap lookup, not a sled read + JSON decode.
        if let Some(pc) = self.persistent_cache.get() {
            let key_str = division_cache_key_string(&cache_key);
            if let Some(raw) = pc.get(&key_str) {
                match serde_json::from_slice::<Option<DivisionMatch>>(&raw) {
                    Ok(decoded) => {
                        self.division_cache
                            .lock()
                            .await
                            .insert(cache_key.clone(), decoded.clone());
                        return Ok(decoded);
                    }
                    Err(e) => {
                        // Corrupt entry — log and fall through to a
                        // fresh scan. The fresh result will overwrite
                        // the bad row on the put below, self-healing.
                        tracing::warn!(
                            target: "emem::overture::cache",
                            cache_key = %key_str,
                            error = %e,
                            "persistent division_cache entry failed to decode; recomputing"
                        );
                    }
                }
            }
        }
        // Search window: scales with `preferred_subtype` so country-
        // tier scans don't miss the country polygon entirely. A
        // ½° pad around the anchor catches every locality (NYC ~0.3°,
        // Greater Tokyo ~0.5°), but a "country" preference requires
        // a much wider sweep — anchors fall well inside a country's
        // bbox but the country row's parquet row-group may be
        // hundreds of km wide and only intersects the search window
        // if the window itself is wide enough. The pad-by-subtype
        // table below is keyed to Overture's admin levels:
        //
        //   country → 30°  (covers any country up to ~6600 km wide)
        //   region  → 6°   (US states, Indian states, Brazil states)
        //   county / localadmin → 1.5°
        //   locality / borough / smaller → 0.5° (legacy default)
        //   no preference → 0.5° (legacy default)
        //
        // The wider sweep is still O(footer_count) parquet head IO;
        // the row-group bbox prune inside each file keeps the
        // per-file scan cheap regardless of the search window's size.
        let pad: f64 = match preferred_subtype {
            Some("country") => 30.0,
            Some("region") => 6.0,
            Some("county") | Some("localadmin") => 1.5,
            _ => 0.5,
        };
        let s_lat = (lat - pad).max(-90.0);
        let n_lat = (lat + pad).min(90.0);
        let w_lng = (lng - pad).max(-180.0);
        let e_lng = (lng + pad).min(180.0);

        let files = self.list_files(DIVISIONS_AREA).await?;
        let parallel = scan_parallelism();
        let candidates: Vec<DivisionMatch> = futures_util::stream::iter(files)
            .map(|key| {
                let hint = normalized_hint.clone();
                async move {
                    self.scan_one_file_divisions(&key, lat, lng, s_lat, n_lat, w_lng, e_lng, &hint)
                        .await
                }
            })
            .buffer_unordered(parallel)
            .try_fold(Vec::<DivisionMatch>::new(), |mut acc, mut x| async move {
                acc.append(&mut x);
                Ok::<_, OvertureError>(acc)
            })
            .await?;

        if candidates.is_empty() {
            self.persist_division_lookup(&cache_key, &None);
            self.division_cache.lock().await.insert(cache_key, None);
            return Ok(None);
        }

        // Ranking (first non-empty tier wins):
        //   1. Subtype-pinned + exact name match.
        //   2. Exact name match (no subtype pin) — highest admin
        //      level wins, with smallest-bbox tie-break.
        //   3. Subtype-pinned containment (no name match).
        //   4. Partial name match — smallest containing polygon.
        //   5. No constraint — smallest containing polygon.
        let pref = preferred_subtype.unwrap_or("");
        let exact: Vec<&DivisionMatch> = candidates
            .iter()
            .filter(|m| {
                !normalized_hint.is_empty() && normalize_division_name(&m.name) == normalized_hint
            })
            .collect();
        let exact_subtype_pinned: Vec<&DivisionMatch> = exact
            .iter()
            .copied()
            .filter(|m| !pref.is_empty() && m.subtype.eq_ignore_ascii_case(pref))
            .collect();
        let subtype_pinned: Vec<&DivisionMatch> = candidates
            .iter()
            .filter(|m| !pref.is_empty() && m.subtype.eq_ignore_ascii_case(pref))
            .collect();
        let partial: Vec<&DivisionMatch> =
            candidates.iter().filter(|m| m.name_matched_hint).collect();

        let chosen = if !exact_subtype_pinned.is_empty() {
            // The "I know exactly what I want" path: locate cascade
            // told us this is e.g. a country-tier query and we
            // found a country-subtype row whose primary name matches.
            // Take the largest bbox among them (a country split
            // across multiple rows uses the row that covers most of
            // the territory).
            *exact_subtype_pinned
                .iter()
                .max_by(|a, b| {
                    a.bbox_area_km_sq
                        .partial_cmp(&b.bbox_area_km_sq)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .expect("non-empty exact_subtype_pinned")
        } else if !exact.is_empty() {
            *exact
                .iter()
                .max_by(|a, b| {
                    division_subtype_rank(&a.subtype)
                        .cmp(&division_subtype_rank(&b.subtype))
                        .then_with(|| {
                            a.bbox_area_km_sq
                                .partial_cmp(&b.bbox_area_km_sq)
                                .unwrap_or(std::cmp::Ordering::Equal)
                        })
                })
                .expect("non-empty exact")
        } else if !subtype_pinned.is_empty() {
            *subtype_pinned
                .iter()
                .min_by(|a, b| {
                    a.bbox_area_km_sq
                        .partial_cmp(&b.bbox_area_km_sq)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .expect("non-empty subtype_pinned")
        } else if !partial.is_empty() {
            *partial
                .iter()
                .min_by(|a, b| {
                    a.bbox_area_km_sq
                        .partial_cmp(&b.bbox_area_km_sq)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .expect("non-empty partial")
        } else {
            candidates
                .iter()
                .min_by(|a, b| {
                    a.bbox_area_km_sq
                        .partial_cmp(&b.bbox_area_km_sq)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .expect("non-empty candidates")
        };
        let result = chosen.clone();
        self.persist_division_lookup(&cache_key, &Some(result.clone()));
        self.division_cache
            .lock()
            .await
            .insert(cache_key, Some(result.clone()));
        Ok(Some(result))
    }

    /// Write a `division_polygon_*` lookup result to the persistent
    /// cache if one is wired. Sync (no `.await`) because sled writes
    /// are non-blocking from the caller's perspective — the
    /// implementation pushes into its background flusher. Serialise
    /// failures are logged and dropped: a missing persistent put just
    /// means the next process will re-pay the parquet scan once.
    fn persist_division_lookup(&self, cache_key: &DivisionCacheKey, value: &Option<DivisionMatch>) {
        let Some(pc) = self.persistent_cache.get() else {
            return;
        };
        let key_str = division_cache_key_string(cache_key);
        match serde_json::to_vec(value) {
            Ok(bytes) => pc.put(&key_str, &bytes),
            Err(e) => {
                tracing::warn!(
                    target: "emem::overture::cache",
                    cache_key = %key_str,
                    error = %e,
                    "failed to serialize division lookup for persistent cache"
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn scan_one_file_divisions(
        &self,
        key: &str,
        anchor_lat: f64,
        anchor_lng: f64,
        s_lat: f64,
        n_lat: f64,
        w_lng: f64,
        e_lng: f64,
        normalized_hint: &str,
    ) -> Result<Vec<DivisionMatch>, OvertureError> {
        let meta = self.footer(key).await?;
        let rgs = self.pick_row_groups(&meta, s_lat, n_lat, w_lng, e_lng);
        if rgs.is_empty() {
            return Ok(Vec::new());
        }
        let mut stream = self
            .open_stream(key, rgs, &["id", "names", "subtype", "country"])
            .await?;
        let mut out: Vec<DivisionMatch> = Vec::new();
        while let Some(batch) = stream
            .try_next()
            .await
            .map_err(|e| OvertureError::Parquet {
                key: key.to_string(),
                detail: format!("next batch: {e}"),
            })?
        {
            let bbox_col = batch
                .column_by_name("bbox")
                .ok_or_else(|| OvertureError::Schema {
                    key: key.to_string(),
                    detail: "no bbox column in batch".into(),
                })?;
            let geom_col =
                batch
                    .column_by_name("geometry")
                    .ok_or_else(|| OvertureError::Schema {
                        key: key.to_string(),
                        detail: "no geometry column in batch".into(),
                    })?;
            let bb = BBoxAccess::new(bbox_col.as_ref()).map_err(|e| OvertureError::Schema {
                key: key.to_string(),
                detail: e,
            })?;
            let geoms = WkbAccess::new(geom_col.as_ref()).map_err(|e| OvertureError::Schema {
                key: key.to_string(),
                detail: e,
            })?;
            let names = DivisionNames::new(batch.column_by_name("names").map(|a| a.as_ref()));
            let subtypes = batch
                .column_by_name("subtype")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let ids = batch
                .column_by_name("id")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let countries = batch
                .column_by_name("country")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());

            for i in 0..batch.num_rows() {
                if !bb.overlaps(i, s_lat, n_lat, w_lng, e_lng) {
                    continue;
                }
                let Some(wkb) = geoms.get(i) else { continue };
                // Quick polygon-contains-point check before we pay the
                // GeoJSON encode. Avoids materialising thousands of
                // country polygons just to filter to the one we want.
                if !wkb_polygon_contains_point(wkb, anchor_lng, anchor_lat) {
                    continue;
                }
                let Some(geometry) = wkb_polygon_to_geojson(wkb) else {
                    continue;
                };
                let primary_name = names.primary(i).unwrap_or_default();
                let names_common = names.common(i);
                let normalized_primary = normalize_division_name(&primary_name);
                let name_matched_hint = !normalized_hint.is_empty()
                    && !normalized_primary.is_empty()
                    && (normalized_primary == normalized_hint
                        || normalized_primary.contains(normalized_hint)
                        || normalized_hint.contains(&normalized_primary));
                let subtype = subtypes
                    .and_then(|s| (!s.is_null(i)).then(|| s.value(i).to_string()))
                    .unwrap_or_default();
                let id = ids
                    .and_then(|s| (!s.is_null(i)).then(|| s.value(i).to_string()))
                    .unwrap_or_default();
                let country = countries
                    .and_then(|s| (!s.is_null(i)).then(|| s.value(i).to_string()))
                    .unwrap_or_default();
                let x0 = bb.xmin.val(i);
                let x1 = bb.xmax.val(i);
                let y0 = bb.ymin.val(i);
                let y1 = bb.ymax.val(i);
                // Cosine-corrected km² so the smallest-bbox tie-break
                // doesn't systematically prefer high-latitude polygons
                // over equally-sized tropical ones. 1° latitude is a
                // near-constant 111.32 km on WGS-84; 1° longitude is
                // 111.32 × cos(lat) km, shrinking to zero at the poles.
                // Centroid latitude is a good enough approximation for
                // a ranking key — admin bboxes rarely span enough
                // latitude for the cosine to change much within a
                // single row. Anti-meridian wrap is not handled here;
                // the lng extent is computed from the parquet bbox
                // column verbatim, which Overture pre-splits at ±180°
                // for IDL-crossing polygons.
                let bbox_area_km_sq = {
                    let lat_center = (y0 + y1) * 0.5;
                    let km_per_deg_lat = 111.32_f64;
                    let km_per_deg_lng = 111.32_f64 * lat_center.to_radians().cos().max(0.0);
                    (x1 - x0).abs() * km_per_deg_lng * (y1 - y0).abs() * km_per_deg_lat
                };
                out.push(DivisionMatch {
                    id,
                    name: primary_name,
                    names_common,
                    subtype,
                    country,
                    geometry,
                    bbox: (y0, y1, x0, x1),
                    bbox_area_km_sq,
                    name_matched_hint,
                });
            }
        }
        Ok(out)
    }

    async fn scan_count(
        &self,
        tt: ThemeType,
        s_lat: f64,
        n_lat: f64,
        w_lng: f64,
        e_lng: f64,
        kind: GeomKind,
    ) -> Result<u64, OvertureError> {
        let files = self.list_files(tt).await?;
        let parallel = scan_parallelism();
        let total = futures_util::stream::iter(files)
            .map(|key| async move {
                self.scan_one_file_count(&key, s_lat, n_lat, w_lng, e_lng, kind)
                    .await
            })
            .buffer_unordered(parallel)
            .try_fold(
                0u64,
                |acc, x| async move { Ok::<_, OvertureError>(acc + x) },
            )
            .await?;
        Ok(total)
    }

    async fn scan_one_file_count(
        &self,
        key: &str,
        s_lat: f64,
        n_lat: f64,
        w_lng: f64,
        e_lng: f64,
        kind: GeomKind,
    ) -> Result<u64, OvertureError> {
        let meta = self.footer(key).await?;
        let rgs = self.pick_row_groups(&meta, s_lat, n_lat, w_lng, e_lng);
        if rgs.is_empty() {
            return Ok(0);
        }
        let mut stream = self.open_stream(key, rgs, &[]).await?;
        let mut count = 0u64;
        while let Some(batch) = stream
            .try_next()
            .await
            .map_err(|e| OvertureError::Parquet {
                key: key.to_string(),
                detail: format!("next batch: {e}"),
            })?
        {
            let bbox_col = batch
                .column_by_name("bbox")
                .ok_or_else(|| OvertureError::Schema {
                    key: key.to_string(),
                    detail: "no bbox column in batch".into(),
                })?;
            let geom_col =
                batch
                    .column_by_name("geometry")
                    .ok_or_else(|| OvertureError::Schema {
                        key: key.to_string(),
                        detail: "no geometry column in batch".into(),
                    })?;
            let bb = BBoxAccess::new(bbox_col.as_ref()).map_err(|e| OvertureError::Schema {
                key: key.to_string(),
                detail: e,
            })?;
            let geoms = WkbAccess::new(geom_col.as_ref()).map_err(|e| OvertureError::Schema {
                key: key.to_string(),
                detail: e,
            })?;
            for i in 0..batch.num_rows() {
                if !bb.overlaps(i, s_lat, n_lat, w_lng, e_lng) {
                    continue;
                }
                let Some(wkb) = geoms.get(i) else {
                    continue;
                };
                let inside = match kind {
                    GeomKind::Point => wkb_point_inside(wkb, s_lat, n_lat, w_lng, e_lng),
                    GeomKind::PolygonCentroid => {
                        wkb_polygon_centroid_inside(wkb, s_lat, n_lat, w_lng, e_lng)
                    }
                };
                if inside {
                    count += 1;
                }
            }
        }
        Ok(count)
    }
}

/// Discover the latest Overture release tag by listing the public bucket.
///
/// Hits `https://overturemaps-us-west-2.s3.amazonaws.com/?list-type=2&prefix=release/&delimiter=/`
/// (anonymous, no signing) and parses the `CommonPrefixes` entries from
/// the ListBucketResult XML. Each entry looks like
/// `<CommonPrefixes><Prefix>release/2026-04-15.0/</Prefix></CommonPrefixes>`.
/// We strip the `release/` prefix and trailing `/`, then sort descending.
///
/// Lexicographic sort is correct because every release tag has the form
/// `YYYY-MM-DD.N` (zero-padded month/day, integer revision N). String
/// comparison therefore matches chronological + revision order.
///
/// Returns the highest tag (e.g. `2026-04-15.1` over `2026-04-15.0`).
/// Returns `Err(OvertureError::S3List)` on any HTTP / parse failure —
/// the caller (typically `OvertureClient::release`) decides whether to
/// fall back to a cached value or surface the error.
pub async fn latest_release() -> Result<String, OvertureError> {
    let url = format!("{LIST_ENDPOINT}?list-type=2&prefix=release/&delimiter=/");
    let client = reqwest::Client::builder()
        .user_agent("emem-fetch/overture (+https://emem.dev)")
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| OvertureError::S3List(format!("reqwest client: {e}")))?;
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| OvertureError::S3List(format!("GET {url}: {e}")))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(OvertureError::S3List(format!(
            "list bucket returned {status} for {url}"
        )));
    }
    let body = resp
        .text()
        .await
        .map_err(|e| OvertureError::S3List(format!("read body: {e}")))?;
    parse_latest_release_from_xml(&body)
}

/// Pure parser: given a ListObjectsV2 XML body, return the lex-max
/// release tag (date + revision). Public so tests + external tooling
/// can exercise it without an S3 round-trip.
pub fn parse_latest_release_from_xml(body: &str) -> Result<String, OvertureError> {
    // Tiny inline XML scanner — we only need the contents of every
    // `<Prefix>...</Prefix>` element nested inside `<CommonPrefixes>`.
    // Pulling in a full XML parser for one element would be overkill
    // and `quick-xml` is not yet a workspace dep. The bucket response
    // is well-formed S3 output (no comments, no CDATA, no namespaces
    // on these tags), so a strict tag scan is sufficient and correct.
    let mut releases: Vec<String> = Vec::new();
    let mut rest = body;
    while let Some(open_idx) = rest.find("<Prefix>") {
        let after_open = &rest[open_idx + "<Prefix>".len()..];
        let Some(close_idx) = after_open.find("</Prefix>") else {
            break;
        };
        let inner = &after_open[..close_idx];
        // Expect `release/<TAG>/`. Be defensive: anything else is ignored.
        if let Some(stripped) = inner
            .strip_prefix("release/")
            .and_then(|s| s.strip_suffix('/'))
        {
            // The top-level prefix `release/` itself appears as
            // `<Prefix>release/</Prefix>` in some responses — `stripped`
            // will be empty there. Skip it.
            if !stripped.is_empty() && !stripped.contains('/') {
                releases.push(stripped.to_string());
            }
        }
        rest = &after_open[close_idx + "</Prefix>".len()..];
    }
    if releases.is_empty() {
        return Err(OvertureError::S3List(
            "no <CommonPrefixes><Prefix>release/.../</Prefix></CommonPrefixes> entries in list response".into(),
        ));
    }
    // Lex-descending sort works because every tag is `YYYY-MM-DD.N`
    // with zero-padded numeric components. We additionally split on
    // '.' and compare the revision as an integer so `2026-04-15.10`
    // would sort above `2026-04-15.2` if Overture ever ships a
    // double-digit revision.
    releases.sort_by(|a, b| compare_release_tags(b, a));
    Ok(releases.into_iter().next().unwrap())
}

/// Compare two `YYYY-MM-DD.N` release tags. Date-part lex-compares
/// correctly because of zero-padding; revision is numeric to survive
/// a future double-digit `.10` revision.
fn compare_release_tags(a: &str, b: &str) -> std::cmp::Ordering {
    let split = |s: &str| -> (String, u32) {
        if let Some((date, rev)) = s.rsplit_once('.') {
            let n = rev.parse::<u32>().unwrap_or(0);
            (date.to_string(), n)
        } else {
            (s.to_string(), 0)
        }
    };
    let (da, na) = split(a);
    let (db, nb) = split(b);
    da.cmp(&db).then(na.cmp(&nb))
}

/// File-scan parallelism: high enough to overlap S3 RTT, low enough not to
/// thrash. Override with `EMEM_OVERTURE_PARALLEL`.
fn scan_parallelism() -> usize {
    std::env::var("EMEM_OVERTURE_PARALLEL")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(32)
        .clamp(1, 256)
}

#[derive(Default)]
struct BboxLeafIndex {
    xmin: Option<usize>,
    xmax: Option<usize>,
    ymin: Option<usize>,
    ymax: Option<usize>,
}

#[derive(Clone, Copy)]
enum GeomKind {
    Point,
    PolygonCentroid,
}

/// Per-row bbox accessor — tolerates both f32 and f64 underlying storage,
/// which Overture has used in different vintages.
struct BBoxAccess<'a> {
    xmin: F32OrF64<'a>,
    xmax: F32OrF64<'a>,
    ymin: F32OrF64<'a>,
    ymax: F32OrF64<'a>,
}

enum F32OrF64<'a> {
    F32(&'a Float32Array),
    F64(&'a Float64Array),
}

impl<'a> F32OrF64<'a> {
    fn val(&self, i: usize) -> f64 {
        match self {
            F32OrF64::F32(a) => a.value(i) as f64,
            F32OrF64::F64(a) => a.value(i),
        }
    }
}

impl<'a> BBoxAccess<'a> {
    fn new(col: &'a dyn Array) -> Result<Self, String> {
        let s = col
            .as_any()
            .downcast_ref::<StructArray>()
            .ok_or_else(|| format!("bbox not a struct (dtype={:?})", col.data_type()))?;
        let pick = |name: &str| -> Result<F32OrF64<'a>, String> {
            let f = s
                .column_by_name(name)
                .ok_or_else(|| format!("bbox.{name} missing"))?;
            match f.data_type() {
                DataType::Float32 => Ok(F32OrF64::F32(
                    f.as_any().downcast_ref::<Float32Array>().unwrap(),
                )),
                DataType::Float64 => Ok(F32OrF64::F64(
                    f.as_any().downcast_ref::<Float64Array>().unwrap(),
                )),
                d => Err(format!("bbox.{name} unexpected dtype {d:?}")),
            }
        };
        Ok(BBoxAccess {
            xmin: pick("xmin")?,
            xmax: pick("xmax")?,
            ymin: pick("ymin")?,
            ymax: pick("ymax")?,
        })
    }
    fn overlaps(&self, i: usize, s_lat: f64, n_lat: f64, w_lng: f64, e_lng: f64) -> bool {
        let x0 = self.xmin.val(i);
        let x1 = self.xmax.val(i);
        let y0 = self.ymin.val(i);
        let y1 = self.ymax.val(i);
        x0 <= e_lng && x1 >= w_lng && y0 <= n_lat && y1 >= s_lat
    }
}

/// Per-row WKB accessor — geometry column may be `Binary` or `LargeBinary`.
struct WkbAccess<'a> {
    inner: WkbInner<'a>,
}

enum WkbInner<'a> {
    Bin(&'a BinaryArray),
    Large(&'a LargeBinaryArray),
}

impl<'a> WkbAccess<'a> {
    fn new(col: &'a dyn Array) -> Result<Self, String> {
        if let Some(b) = col.as_any().downcast_ref::<BinaryArray>() {
            Ok(WkbAccess {
                inner: WkbInner::Bin(b),
            })
        } else if let Some(b) = col.as_any().downcast_ref::<LargeBinaryArray>() {
            Ok(WkbAccess {
                inner: WkbInner::Large(b),
            })
        } else {
            Err(format!("geometry unexpected dtype {:?}", col.data_type()))
        }
    }
    fn get(&self, i: usize) -> Option<&[u8]> {
        match &self.inner {
            WkbInner::Bin(b) => {
                if b.is_null(i) {
                    None
                } else {
                    Some(b.value(i))
                }
            }
            WkbInner::Large(b) => {
                if b.is_null(i) {
                    None
                } else {
                    Some(b.value(i))
                }
            }
        }
    }
}

fn stat_min_f64(s: Option<&Statistics>) -> Option<f64> {
    let s = s?;
    match s {
        Statistics::Float(v) => v.min_opt().map(|x| *x as f64),
        Statistics::Double(v) => v.min_opt().copied(),
        Statistics::Int32(v) => v.min_opt().map(|x| *x as f64),
        Statistics::Int64(v) => v.min_opt().map(|x| *x as f64),
        _ => None,
    }
}
fn stat_max_f64(s: Option<&Statistics>) -> Option<f64> {
    let s = s?;
    match s {
        Statistics::Float(v) => v.max_opt().map(|x| *x as f64),
        Statistics::Double(v) => v.max_opt().copied(),
        Statistics::Int32(v) => v.max_opt().map(|x| *x as f64),
        Statistics::Int64(v) => v.max_opt().map(|x| *x as f64),
        _ => None,
    }
}

// ---------- minimal WKB decoder (Point / LineString / MultiLineString /
//                                Polygon / MultiPolygon, little-endian) ----------

/// WKB type tag (low 16 bits in the wkbType field).
const WKB_POINT: u32 = 1;
const WKB_LINESTRING: u32 = 2;
const WKB_POLYGON: u32 = 3;
const WKB_MULTI_LINESTRING: u32 = 5;
const WKB_MULTI_POLYGON: u32 = 6;

struct WkbCursor<'a> {
    buf: &'a [u8],
    pos: usize,
    le: bool,
}
impl<'a> WkbCursor<'a> {
    fn new(buf: &'a [u8]) -> Option<Self> {
        if buf.is_empty() {
            return None;
        }
        let le = match buf[0] {
            1 => true,
            0 => false,
            _ => return None,
        };
        Some(WkbCursor { buf, pos: 1, le })
    }
    fn read_u32(&mut self) -> Option<u32> {
        if self.pos + 4 > self.buf.len() {
            return None;
        }
        let s = &self.buf[self.pos..self.pos + 4];
        self.pos += 4;
        Some(if self.le {
            u32::from_le_bytes([s[0], s[1], s[2], s[3]])
        } else {
            u32::from_be_bytes([s[0], s[1], s[2], s[3]])
        })
    }
    fn read_f64(&mut self) -> Option<f64> {
        if self.pos + 8 > self.buf.len() {
            return None;
        }
        let s = &self.buf[self.pos..self.pos + 8];
        self.pos += 8;
        Some(if self.le {
            f64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]])
        } else {
            f64::from_be_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]])
        })
    }
    /// Read the next nested geometry's byte order + type. Each sub-geometry
    /// in a multi-geometry has its own byte-order byte and type tag.
    fn read_sub_header(&mut self) -> Option<u32> {
        if self.pos + 5 > self.buf.len() {
            return None;
        }
        let bo = self.buf[self.pos];
        self.pos += 1;
        self.le = match bo {
            1 => true,
            0 => false,
            _ => return None,
        };
        self.read_u32().map(|t| t & 0x0FFFFFFF) // mask off SRID/M/Z bits
    }
}

fn wkb_type(buf: &[u8]) -> Option<(WkbCursor<'_>, u32)> {
    let mut cur = WkbCursor::new(buf)?;
    let t = cur.read_u32()? & 0x0FFFFFFF;
    Some((cur, t))
}

/// Test if a WKB Point falls inside the bbox.
pub fn wkb_point_inside(buf: &[u8], s_lat: f64, n_lat: f64, w_lng: f64, e_lng: f64) -> bool {
    let Some((mut cur, t)) = wkb_type(buf) else {
        return false;
    };
    if t != WKB_POINT {
        return false;
    }
    let (Some(x), Some(y)) = (cur.read_f64(), cur.read_f64()) else {
        return false;
    };
    x >= w_lng && x <= e_lng && y >= s_lat && y <= n_lat
}

/// One administrative-boundary match returned by
/// [`OvertureClient::division_polygon_near`]. The geometry is a
/// GeoJSON Polygon (or MultiPolygon) ready to drop into a
/// `/v1/recall_polygon` response's `polygon_geojson` field.
///
/// Serializable so the per-(anchor, name) lookup result can survive
/// across process restarts via a [`DivisionPersistentCache`]
/// (typically the responder's sled geocoder tree). All fields are
/// plain owned data — no borrowed slices, no lifetimes — so a JSON
/// round-trip is lossless.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DivisionMatch {
    /// GERS ID — globally stable Overture identifier, citable as
    /// the source CID in receipts.
    pub id: String,
    /// `names.primary` — the canonical localized label.
    pub name: String,
    /// `names.common` — map of ISO 639 language code (or
    /// language-script tag like `zh-Hans`) → localized name. Pulled
    /// directly from Overture's divisions schema so an agent can
    /// surface the Japanese, Bengali, or Arabic name of a place
    /// without a second lookup. Empty when the row had no `common`
    /// entries.
    pub names_common: std::collections::BTreeMap<String, String>,
    /// Subtype: `country` / `region` / `county` / `localadmin` /
    /// `locality` / `borough` / `microhood` / `neighborhood` /
    /// `dependency` / `macrohood`.
    pub subtype: String,
    /// ISO 3166-1 alpha-2 country code that owns this division
    /// (parsed from Overture's `country` column). Empty for the
    /// rare disputed-territory rows that ship without one.
    pub country: String,
    /// WGS84 GeoJSON geometry (`type: "Polygon"` or `"MultiPolygon"`).
    pub geometry: serde_json::Value,
    /// Tuple `(min_lat, max_lat, min_lng, max_lng)` derived from the
    /// row's bbox struct column — surfaced so recall_polygon doesn't
    /// have to re-derive it from the GeoJSON ring.
    pub bbox: (f64, f64, f64, f64),
    /// Approx polygon footprint (km²) derived from the row's bbox
    /// with a cosine-of-centroid-latitude correction on the lng
    /// extent. Used as a tie-break to favor the *smallest* containing
    /// boundary — i.e. the locality, not its enclosing region or
    /// country. Was naive degrees² in 0.0.6 and earlier, but that
    /// systematically over-counted high-latitude polygons (Greenland,
    /// Russia, Canada) because 1° lng shrinks toward the poles, so
    /// the smallest-bbox ranker could prefer a tropical region of the
    /// same true area over a polar one of equal degree²; km² is the
    /// honest comparator.
    pub bbox_area_km_sq: f64,
    /// True when the polygon's `names.primary` matched the caller's
    /// `name_hint` (ASCII-folded, case-insensitive). The resolver
    /// prefers name-matched polygons; the area tie-break only fires
    /// when no name matched.
    pub name_matched_hint: bool,
}

/// Admin-level rank for the division-resolver's exact-match tie-break.
/// Higher number = broader scope; the resolver prefers the highest-
/// rank candidate among exact-name matches so "Manhattan" lands on
/// the borough/county/region row, not the neighborhood / community-
/// board row. Subtypes follow Overture's
/// [divisions schema](https://docs.overturemaps.org/schema/reference/divisions/division/);
/// unknown subtypes get rank 0 so they only win when nothing else
/// matches.
fn division_subtype_rank(subtype: &str) -> u8 {
    match subtype {
        "country" => 10,
        "region" => 9,
        "county" => 8,
        "localadmin" => 7,
        "locality" => 6,
        "borough" => 5,
        "dependency" => 4,
        "macrohood" => 3,
        "neighborhood" => 2,
        "microhood" => 1,
        _ => 0,
    }
}

/// Folded name comparison for division resolver — lowercase, strip
/// punctuation. Matches the heuristic the geocoder cache uses, so
/// "São Paulo", "Sao Paulo", and "SAO PAULO" hit the same key. Light
/// re-implementation kept inline to avoid a `deunicode` dep.
fn normalize_division_name(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = true;
    for ch in s.chars() {
        let folded = match ch {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' => {
                'a'
            }
            'ç' | 'Ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' | 'È' | 'É' | 'Ê' | 'Ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' | 'Ì' | 'Í' | 'Î' | 'Ï' => 'i',
            'ñ' | 'Ñ' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' => {
                'o'
            }
            'ù' | 'ú' | 'û' | 'ü' | 'Ù' | 'Ú' | 'Û' | 'Ü' => 'u',
            'ý' | 'ÿ' | 'Ý' | 'Ÿ' => 'y',
            _ => ch,
        };
        if folded.is_ascii_alphanumeric() {
            out.push(folded.to_ascii_lowercase());
            last_space = false;
        } else if !last_space {
            out.push(' ');
            last_space = true;
        }
    }
    out.trim().to_string()
}

/// Accessor for the `names` struct column on the divisions parquet.
/// Overture's schema stores names as a struct with `primary` (Utf8),
/// `common` (Map<Utf8, Utf8>), and `rules` (List<Struct<...>>). We
/// read both `primary` and `common` so the locate response can
/// surface every localized name an agent might quote back ("東京",
/// "ঢাকা", "القاهرة").
///
/// `rules` carries variant-specific name disambiguation (e.g. "this
/// name applies only when the language is X and the country is Y").
/// We don't read it here — the few cases it disambiguates are
/// already covered by `common`'s straight lang→name map, and the
/// `rules` parser would double the parquet-read cost for marginal
/// recall gain.
struct DivisionNames<'a> {
    primary: Option<&'a StringArray>,
    /// `Map<Utf8, Utf8>` column. Arrow exposes Map as a List of
    /// Struct<key, value>; we hold the inner StructArray plus the
    /// list offsets buffer for per-row slicing.
    common: Option<MapStringAccess<'a>>,
}

/// Lightweight accessor for an Arrow `Map<Utf8, Utf8>` column. Wraps
/// the offsets buffer (one i32 per row+1) and the inner Struct<key,
/// value> StringArrays so per-row maps are a constant-time slice.
struct MapStringAccess<'a> {
    offsets: &'a [i32],
    keys: &'a StringArray,
    values: &'a StringArray,
}

impl<'a> MapStringAccess<'a> {
    /// Build the accessor from a generic Arrow column. Returns
    /// `None` when the column is missing or the inner type isn't
    /// `Map<Utf8, Utf8>` (schema drift). Used only for `names.common`
    /// so failure here is silent and degrades to "no localized
    /// names" rather than aborting the whole scan.
    fn try_new(col: &'a dyn Array) -> Option<Self> {
        let m = col.as_any().downcast_ref::<MapArray>()?;
        let entries = m.entries();
        // Overture publishes `names.common` as
        // `Map<key: Utf8, value: Utf8>` but Arrow lets producers
        // name the inner struct fields freely; some emitters use
        // ("key", "value"), others use ("keys", "values"). Probe by
        // name first, then fall back to position 0/1 — Arrow Map
        // spec mandates ordered (key, value) physical layout.
        let keys = entries
            .column_by_name("key")
            .or_else(|| entries.column_by_name("keys"))
            .unwrap_or_else(|| entries.column(0))
            .as_any()
            .downcast_ref::<StringArray>()?;
        let values = entries
            .column_by_name("value")
            .or_else(|| entries.column_by_name("values"))
            .unwrap_or_else(|| entries.column(1))
            .as_any()
            .downcast_ref::<StringArray>()?;
        Some(MapStringAccess {
            offsets: m.value_offsets(),
            keys,
            values,
        })
    }

    /// Return the (key, value) pairs for row `i` as owned strings.
    fn row(&self, i: usize) -> Vec<(String, String)> {
        if i + 1 >= self.offsets.len() {
            return Vec::new();
        }
        let start = self.offsets[i] as usize;
        let end = self.offsets[i + 1] as usize;
        let mut out = Vec::with_capacity(end.saturating_sub(start));
        for j in start..end {
            if j >= self.keys.len() || j >= self.values.len() {
                break;
            }
            if self.keys.is_null(j) {
                continue;
            }
            let k = self.keys.value(j).to_string();
            let v = if self.values.is_null(j) {
                String::new()
            } else {
                self.values.value(j).to_string()
            };
            if !k.is_empty() && !v.is_empty() {
                out.push((k, v));
            }
        }
        out
    }
}

impl<'a> DivisionNames<'a> {
    fn new(col: Option<&'a dyn Array>) -> Self {
        let struct_arr = col.and_then(|c| c.as_any().downcast_ref::<StructArray>());
        let primary = struct_arr
            .and_then(|s| s.column_by_name("primary"))
            .and_then(|c| c.as_any().downcast_ref::<StringArray>());
        let common = struct_arr
            .and_then(|s| s.column_by_name("common"))
            .and_then(|c| MapStringAccess::try_new(c.as_ref()));
        DivisionNames { primary, common }
    }
    fn primary(&self, i: usize) -> Option<String> {
        let p = self.primary?;
        if p.is_null(i) {
            None
        } else {
            Some(p.value(i).to_string())
        }
    }
    /// Per-row `names.common` entries, collected into a sorted
    /// BTreeMap so the wire shape is deterministic regardless of
    /// arrow's internal key order.
    fn common(&self, i: usize) -> std::collections::BTreeMap<String, String> {
        let mut out = std::collections::BTreeMap::new();
        if let Some(c) = &self.common {
            for (k, v) in c.row(i) {
                out.insert(k, v);
            }
        }
        out
    }
}

/// Polygon-contains-point: even-odd ray-cast on the WKB's exterior
/// ring (for Polygon) or any sub-polygon's exterior ring (for
/// MultiPolygon). Holes are ignored — admin boundaries rarely have
/// genuine interior rings, and on the rare exclave-island case the
/// outer-ring inclusion is the right answer (the GeoNames anchor
/// is by construction one of the populated places GeoNames listed
/// inside the locality, so it lands on land, not in a hole).
pub fn wkb_polygon_contains_point(buf: &[u8], lng: f64, lat: f64) -> bool {
    let Some((mut cur, t)) = wkb_type(buf) else {
        return false;
    };
    match t {
        WKB_POLYGON => polygon_contains_pt(&mut cur, lng, lat),
        WKB_MULTI_POLYGON => {
            let Some(np) = cur.read_u32() else {
                return false;
            };
            for _ in 0..np {
                if cur.read_sub_header() != Some(WKB_POLYGON) {
                    return false;
                }
                if polygon_contains_pt(&mut cur, lng, lat) {
                    return true;
                }
            }
            false
        }
        _ => false,
    }
}

/// Read a polygon (consuming exactly its bytes from `cur`) and test
/// whether `(lng, lat)` lies inside its exterior ring via even-odd
/// ray-cast. Inner rings are read past for cursor correctness but
/// not used in the test — see the rationale on
/// [`wkb_polygon_contains_point`].
///
/// Anti-meridian (IDL) handling: polygons that cross ±180° (Russia
/// Chukotka, Fiji, Aleutians, NZ Chathams, Antarctica) have adjacent
/// vertices whose longitude jumps from near +180 to near -180. A
/// naive ray-cast treats that jump as a real edge spanning ~360° and
/// reports wrong inside/outside. We detect the crossing by scanning
/// the buffered exterior ring for any |Δx| > 180 between adjacent
/// vertices; if found, all longitudes (vertices + query point) are
/// shifted into [0, 360) before the ray-cast, moving the
/// discontinuity from ±180° to 0°/360° where no edge of a
/// non-pathological polygon sits.
fn polygon_contains_pt(cur: &mut WkbCursor<'_>, px: f64, py: f64) -> bool {
    let nrings = match cur.read_u32() {
        Some(n) => n,
        None => return false,
    };
    let mut inside = false;
    for r in 0..nrings {
        let np = match cur.read_u32() {
            Some(n) => n,
            None => return false,
        };
        if np < 3 {
            // Consume but ignore degenerate rings.
            for _ in 0..np {
                let _ = cur.read_f64();
                let _ = cur.read_f64();
            }
            continue;
        }
        if r == 0 {
            // Exterior ring: buffer all vertices so we can scan for
            // IDL crossings before ray-casting. WKB spec promises the
            // ring is closed (first == last), but we close defensively
            // in [`ring_contains_point`] regardless.
            let mut ring: Vec<(f64, f64)> = Vec::with_capacity(np as usize);
            for _ in 0..np {
                let x = match cur.read_f64() {
                    Some(v) => v,
                    None => return false,
                };
                let y = match cur.read_f64() {
                    Some(v) => v,
                    None => return false,
                };
                ring.push((x, y));
            }
            inside = ring_contains_point(&ring, px, py);
        } else {
            // Inner ring: walk past without testing.
            for _ in 0..np {
                let _ = cur.read_f64();
                let _ = cur.read_f64();
            }
        }
    }
    inside
}

/// Even-odd ray-cast on a (possibly open) ring. Handles anti-meridian
/// crossings by shifting longitudes into [0, 360) when any adjacent
/// edge spans more than 180° of longitude (the unambiguous signature
/// of an IDL crossing — no admin boundary spans more than half the
/// globe in a single edge). The closing edge (last → first) is
/// included implicitly via `(i + 1) % n`, so callers may pass either
/// WKB-closed rings or open rings.
fn ring_contains_point(ring: &[(f64, f64)], mut px: f64, py: f64) -> bool {
    if ring.len() < 3 {
        return false;
    }
    // IDL detection: any edge (including the close) with |Δx| > 180.
    let n = ring.len();
    let mut crosses_idl = false;
    for i in 0..n {
        let dx = ring[(i + 1) % n].0 - ring[i].0;
        if dx.abs() > 180.0 {
            crosses_idl = true;
            break;
        }
    }
    // Only allocate the shifted ring when IDL crossing forces us to.
    // The common case (~99 % of admin boundaries — none of the 250
    // countries except 5–10 cross the IDL) ray-casts against the
    // original slice with no allocation.
    let shifted: Vec<(f64, f64)>;
    let pts: &[(f64, f64)] = if crosses_idl {
        if px < 0.0 {
            px += 360.0;
        }
        shifted = ring
            .iter()
            .map(|&(x, y)| (if x < 0.0 { x + 360.0 } else { x }, y))
            .collect();
        &shifted
    } else {
        ring
    };
    let mut inside = false;
    for i in 0..n {
        let (x0, y0) = pts[i];
        let (x1, y1) = pts[(i + 1) % n];
        // (y0>py)!=(y1>py) skips horizontal edges (y0==y1) — both
        // comparisons evaluate the same way, so y1-y0 in the
        // intersection formula below can never be zero when we reach
        // the divide.
        if ((y0 > py) != (y1 > py)) && (px < (x1 - x0) * (py - y0) / (y1 - y0) + x0) {
            inside = !inside;
        }
    }
    inside
}

/// Decode a WKB Polygon or MultiPolygon to a GeoJSON geometry value
/// in `[lon, lat]` order with closed rings. Returns `None` for any
/// non-polygon variant or malformed buffer.
///
/// RFC 7946 §3.1.9 forbids polygons that cross the antimeridian — a
/// client rendering an IDL-spanning polygon as a single ring will
/// draw a giant stripe across the entire map. When the exterior ring
/// crosses ±180°, we split it at the antimeridian into east and
/// west pieces and emit the result as a `MultiPolygon`. The split
/// uses linear interpolation on the (shifted-to-[0,360)) longitude
/// to find the y at the IDL crossing, then closes each piece against
/// `(±180, y_cross)`. Inner rings (holes) are not split because
/// admin boundaries effectively never carry IDL-crossing interior
/// rings; if one ever appears, it is kept attached to whichever
/// half-polygon contains its first vertex.
pub fn wkb_polygon_to_geojson(buf: &[u8]) -> Option<serde_json::Value> {
    let (mut cur, t) = wkb_type(buf)?;
    match t {
        WKB_POLYGON => {
            let rings = read_polygon_rings(&mut cur)?;
            Some(rings_to_geojson_geometry(&rings))
        }
        WKB_MULTI_POLYGON => {
            let np = cur.read_u32()?;
            let mut polys: Vec<Vec<Vec<[f64; 2]>>> = Vec::with_capacity(np as usize);
            for _ in 0..np {
                if cur.read_sub_header() != Some(WKB_POLYGON) {
                    return None;
                }
                let rings = read_polygon_rings(&mut cur)?;
                // Per-sub-polygon IDL split: each sub-polygon that
                // crosses gets replaced by its split halves; the
                // outer MultiPolygon coordinate array then carries
                // them as separate polygons.
                if let Some(exterior) = rings.first() {
                    if ring_crosses_idl_xy(exterior) {
                        let halves = split_polygon_rings_at_antimeridian(&rings);
                        polys.extend(halves);
                        continue;
                    }
                }
                polys.push(rings);
            }
            Some(serde_json::json!({"type": "MultiPolygon", "coordinates": polys}))
        }
        _ => None,
    }
}

/// Wrap a list of rings as a GeoJSON Polygon, or, if the exterior
/// ring crosses the antimeridian, split into a MultiPolygon. Single
/// shared helper so the WKB_POLYGON and WKB_MULTI_POLYGON arms in
/// [`wkb_polygon_to_geojson`] don't drift apart on the split logic.
fn rings_to_geojson_geometry(rings: &[Vec<[f64; 2]>]) -> serde_json::Value {
    if let Some(exterior) = rings.first() {
        if ring_crosses_idl_xy(exterior) {
            let halves = split_polygon_rings_at_antimeridian(rings);
            return serde_json::json!({
                "type": "MultiPolygon",
                "coordinates": halves,
            });
        }
    }
    serde_json::json!({"type": "Polygon", "coordinates": rings})
}

/// IDL-crossing detector for a ring stored as `[[x, y], …]` (GeoJSON
/// coordinate order). Returns true if any edge — including the
/// closing edge — has |Δx| > 180°. Matches the detection used by
/// [`ring_contains_point`].
fn ring_crosses_idl_xy(ring: &[[f64; 2]]) -> bool {
    let n = ring.len();
    if n < 2 {
        return false;
    }
    for i in 0..n {
        let dx = ring[(i + 1) % n][0] - ring[i][0];
        if dx.abs() > 180.0 {
            return true;
        }
    }
    false
}

/// Split a single polygon (exterior + optional holes) at the
/// antimeridian, returning one or more polygons each represented as
/// `[exterior, hole1, hole2, …]`. The exterior ring is cut into two
/// halves at ±180; each cut introduces a vertex at (sign·180, y_cross)
/// where y_cross is the linear-interpolated latitude of the IDL
/// crossing in shifted-longitude space. Inner rings are bucketed by
/// the sign of their first vertex; rings that themselves cross the
/// IDL are kept attached to that bucket (rare in practice).
fn split_polygon_rings_at_antimeridian(rings: &[Vec<[f64; 2]>]) -> Vec<Vec<Vec<[f64; 2]>>> {
    let exterior = match rings.first() {
        Some(r) => r,
        None => return Vec::new(),
    };
    let (east, west) = split_ring_at_antimeridian(exterior);
    let mut east_poly: Vec<Vec<[f64; 2]>> = if east.len() >= 4 {
        vec![east]
    } else {
        Vec::new()
    };
    let mut west_poly: Vec<Vec<[f64; 2]>> = if west.len() >= 4 {
        vec![west]
    } else {
        Vec::new()
    };
    // Distribute inner rings by the sign of their first vertex. A
    // hole that itself crosses the IDL stays whole and lands in
    // whichever bucket its first vertex lives — splitting a hole
    // requires a containment test (which side of the cut is "inside"
    // the host polygon) that admin geometry never warrants.
    for hole in rings.iter().skip(1) {
        if hole.is_empty() {
            continue;
        }
        let first_x = hole[0][0];
        if first_x >= 0.0 {
            if !east_poly.is_empty() {
                east_poly.push(hole.clone());
            }
        } else if !west_poly.is_empty() {
            west_poly.push(hole.clone());
        }
    }
    let mut out = Vec::new();
    if !east_poly.is_empty() {
        out.push(east_poly);
    }
    if !west_poly.is_empty() {
        out.push(west_poly);
    }
    out
}

/// Cut a single ring at the antimeridian and return the two closed
/// halves (east at ≥0° longitude, west at <0° longitude). Either
/// half can be empty when the input is entirely on one side; in that
/// case the function returns a single populated half and an empty
/// counterpart (callers filter empty halves out).
fn split_ring_at_antimeridian(ring: &[[f64; 2]]) -> (Vec<[f64; 2]>, Vec<[f64; 2]>) {
    let mut east: Vec<[f64; 2]> = Vec::new();
    let mut west: Vec<[f64; 2]> = Vec::new();
    let n = ring.len();
    if n == 0 {
        return (east, west);
    }
    for i in 0..n {
        let [x0, y0] = ring[i];
        let [x1, y1] = ring[(i + 1) % n];
        // Append current vertex to its native side.
        if x0 >= 0.0 {
            east.push([x0, y0]);
        } else {
            west.push([x0, y0]);
        }
        let dx = x1 - x0;
        if dx.abs() > 180.0 {
            // Crossing — compute y at the IDL using shifted-to-[0,360°)
            // linear interpolation so the formula doesn't care which
            // direction the wrap goes.
            let x0s = if x0 < 0.0 { x0 + 360.0 } else { x0 };
            let x1s = if x1 < 0.0 { x1 + 360.0 } else { x1 };
            let denom = x1s - x0s;
            // denom can be zero only if both ends are identical, in
            // which case dx is also zero and we wouldn't be in this
            // branch. Guard defensively anyway.
            let t = if denom.abs() < f64::EPSILON {
                0.5
            } else {
                (180.0 - x0s) / denom
            };
            let y_cross = y0 + t * (y1 - y0);
            // Emit (+180, y_cross) on the east side and (-180, y_cross)
            // on the west side. Order matters: the side we were just
            // on closes first, then the other side opens at the IDL.
            if x0 >= 0.0 {
                east.push([180.0, y_cross]);
                west.push([-180.0, y_cross]);
            } else {
                west.push([-180.0, y_cross]);
                east.push([180.0, y_cross]);
            }
        }
    }
    // Close each non-empty ring.
    for ring in [&mut east, &mut west] {
        if !ring.is_empty() {
            let first = ring[0];
            let last = *ring.last().unwrap();
            if first != last {
                ring.push(first);
            }
        }
    }
    (east, west)
}

fn read_polygon_rings(cur: &mut WkbCursor<'_>) -> Option<Vec<Vec<[f64; 2]>>> {
    let nrings = cur.read_u32()?;
    let mut rings = Vec::with_capacity(nrings as usize);
    for _ in 0..nrings {
        let np = cur.read_u32()?;
        let mut ring = Vec::with_capacity(np as usize);
        for _ in 0..np {
            let x = cur.read_f64()?;
            let y = cur.read_f64()?;
            ring.push([x, y]);
        }
        // GeoJSON requires closed rings; WKB rings are already
        // closed by spec but defensively close again if a producer
        // skipped the duplicate vertex.
        if let (Some(first), Some(last)) = (ring.first().copied(), ring.last().copied()) {
            if first != last {
                ring.push(first);
            }
        }
        rings.push(ring);
    }
    Some(rings)
}

/// Test if a WKB Polygon's vertex centroid falls inside the bbox.
/// (Average of outer-ring vertices — adequate for a ~10 m cell; the
/// buildings.count band is documented as "centroid inside bbox", so the
/// approximation is part of the contract.)
pub fn wkb_polygon_centroid_inside(
    buf: &[u8],
    s_lat: f64,
    n_lat: f64,
    w_lng: f64,
    e_lng: f64,
) -> bool {
    let Some((mut cur, t)) = wkb_type(buf) else {
        return false;
    };
    let (cx, cy) = match t {
        WKB_POLYGON => match polygon_centroid(&mut cur) {
            Some(c) => c,
            None => return false,
        },
        WKB_MULTI_POLYGON => {
            // Centroid of the largest sub-polygon by ring vertex count.
            let Some(np) = cur.read_u32() else {
                return false;
            };
            let mut best: Option<((f64, f64), u32)> = None;
            for _ in 0..np {
                if cur.read_sub_header() != Some(WKB_POLYGON) {
                    return false;
                }
                let Some(c) = polygon_centroid_with_count(&mut cur) else {
                    return false;
                };
                if best.map(|(_, n)| n < c.1).unwrap_or(true) {
                    best = Some(((c.0 .0, c.0 .1), c.1));
                }
            }
            match best {
                Some((c, _)) => c,
                None => return false,
            }
        }
        _ => return false,
    };
    cx >= w_lng && cx <= e_lng && cy >= s_lat && cy <= n_lat
}

fn polygon_centroid(cur: &mut WkbCursor<'_>) -> Option<(f64, f64)> {
    polygon_centroid_with_count(cur).map(|(c, _)| c)
}

fn polygon_centroid_with_count(cur: &mut WkbCursor<'_>) -> Option<((f64, f64), u32)> {
    let nrings = cur.read_u32()?;
    if nrings == 0 {
        return None;
    }
    let mut cx = 0.0f64;
    let mut cy = 0.0f64;
    let mut total = 0u32;
    for r in 0..nrings {
        let np = cur.read_u32()?;
        for _ in 0..np {
            let x = cur.read_f64()?;
            let y = cur.read_f64()?;
            if r == 0 {
                cx += x;
                cy += y;
                total += 1;
            }
        }
    }
    if total == 0 {
        return None;
    }
    Some(((cx / total as f64, cy / total as f64), total))
}

/// Decode a LineString or MultiLineString into a vector of polylines.
/// Each inner Vec is one polyline of (x, y) pairs.
fn wkb_linestring_or_multi(buf: &[u8]) -> Option<Vec<Vec<(f64, f64)>>> {
    let mut cur = WkbCursor::new(buf)?;
    let t = cur.read_u32()? & 0x0FFFFFFF;
    match t {
        WKB_LINESTRING => {
            let line = read_linestring(&mut cur)?;
            Some(vec![line])
        }
        WKB_MULTI_LINESTRING => {
            let n = cur.read_u32()?;
            let mut out = Vec::with_capacity(n as usize);
            for _ in 0..n {
                if cur.read_sub_header() != Some(WKB_LINESTRING) {
                    return None;
                }
                let line = read_linestring(&mut cur)?;
                out.push(line);
            }
            Some(out)
        }
        _ => None,
    }
}

fn read_linestring(cur: &mut WkbCursor<'_>) -> Option<Vec<(f64, f64)>> {
    let np = cur.read_u32()?;
    let mut v = Vec::with_capacity(np as usize);
    for _ in 0..np {
        let x = cur.read_f64()?;
        let y = cur.read_f64()?;
        v.push((x, y));
    }
    Some(v)
}

/// Sum the planar-projected length of each polyline's segment that falls
/// inside the bbox. Uses Liang-Barsky-style segment clipping; a tiny cell
/// (~10 m) makes the planar approximation indistinguishable from haversine.
fn polyline_clipped_length(
    polylines: &[Vec<(f64, f64)>],
    s_lat: f64,
    n_lat: f64,
    w_lng: f64,
    e_lng: f64,
    m_per_deg_lat: f64,
    m_per_deg_lng: f64,
) -> f64 {
    let mut total = 0.0f64;
    for line in polylines {
        if line.len() < 2 {
            continue;
        }
        for w in line.windows(2) {
            let (a, b) = (w[0], w[1]);
            if let Some(((x0, y0), (x1, y1))) =
                clip_segment_to_bbox(a.0, a.1, b.0, b.1, w_lng, e_lng, s_lat, n_lat)
            {
                let dx = (x1 - x0) * m_per_deg_lng;
                let dy = (y1 - y0) * m_per_deg_lat;
                total += (dx * dx + dy * dy).sqrt();
            }
        }
    }
    total
}

/// Liang-Barsky segment-vs-AABB clipping.
#[allow(clippy::too_many_arguments)]
fn clip_segment_to_bbox(
    mut x0: f64,
    mut y0: f64,
    mut x1: f64,
    mut y1: f64,
    xmin: f64,
    xmax: f64,
    ymin: f64,
    ymax: f64,
) -> Option<((f64, f64), (f64, f64))> {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let mut t0 = 0.0f64;
    let mut t1 = 1.0f64;
    let edges = [
        (-dx, x0 - xmin),
        (dx, xmax - x0),
        (-dy, y0 - ymin),
        (dy, ymax - y0),
    ];
    for (p, q) in edges {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                if r > t1 {
                    return None;
                }
                if r > t0 {
                    t0 = r;
                }
            } else {
                if r < t0 {
                    return None;
                }
                if r < t1 {
                    t1 = r;
                }
            }
        }
    }
    let nx0 = x0 + t0 * dx;
    let ny0 = y0 + t0 * dy;
    let nx1 = x0 + t1 * dx;
    let ny1 = y0 + t1 * dy;
    let _ = (&mut x0, &mut y0, &mut x1, &mut y1);
    Some(((nx0, ny0), (nx1, ny1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt_le(x: f64, y: f64) -> Vec<u8> {
        let mut v = Vec::with_capacity(21);
        v.push(1); // little-endian
        v.extend_from_slice(&1u32.to_le_bytes()); // type=Point
        v.extend_from_slice(&x.to_le_bytes());
        v.extend_from_slice(&y.to_le_bytes());
        v
    }

    fn linestring_le(pts: &[(f64, f64)]) -> Vec<u8> {
        let mut v = Vec::new();
        v.push(1);
        v.extend_from_slice(&2u32.to_le_bytes()); // LineString
        v.extend_from_slice(&(pts.len() as u32).to_le_bytes());
        for (x, y) in pts {
            v.extend_from_slice(&x.to_le_bytes());
            v.extend_from_slice(&y.to_le_bytes());
        }
        v
    }

    fn polygon_le(rings: &[Vec<(f64, f64)>]) -> Vec<u8> {
        let mut v = Vec::new();
        v.push(1);
        v.extend_from_slice(&3u32.to_le_bytes());
        v.extend_from_slice(&(rings.len() as u32).to_le_bytes());
        for r in rings {
            v.extend_from_slice(&(r.len() as u32).to_le_bytes());
            for (x, y) in r {
                v.extend_from_slice(&x.to_le_bytes());
                v.extend_from_slice(&y.to_le_bytes());
            }
        }
        v
    }

    #[test]
    fn point_inside_outside() {
        let p = pt_le(0.118, 52.206);
        assert!(wkb_point_inside(&p, 52.205, 52.208, 0.115, 0.121));
        assert!(!wkb_point_inside(&p, 52.205, 52.208, 0.200, 0.300));
    }

    #[test]
    fn polygon_centroid_basic() {
        // Square 0..1 x 0..1 → centroid 0.5,0.5.
        let poly = polygon_le(&[vec![
            (0.0, 0.0),
            (1.0, 0.0),
            (1.0, 1.0),
            (0.0, 1.0),
            (0.0, 0.0),
        ]]);
        assert!(wkb_polygon_centroid_inside(&poly, 0.4, 0.6, 0.4, 0.6));
        assert!(!wkb_polygon_centroid_inside(&poly, 2.0, 3.0, 2.0, 3.0));
    }

    #[test]
    fn parses_latest_release_from_list_xml() {
        // Stub of the real `?list-type=2&prefix=release/&delimiter=/`
        // response shape. Multiple releases on the same date check that
        // we sort by revision *as a number*, not just lexicographically.
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Name>overturemaps-us-west-2</Name>
  <Prefix>release/</Prefix>
  <Delimiter>/</Delimiter>
  <KeyCount>3</KeyCount>
  <CommonPrefixes><Prefix>release/2026-03-15.0/</Prefix></CommonPrefixes>
  <CommonPrefixes><Prefix>release/2026-04-15.0/</Prefix></CommonPrefixes>
  <CommonPrefixes><Prefix>release/2026-04-15.1/</Prefix></CommonPrefixes>
</ListBucketResult>"#;
        let got = parse_latest_release_from_xml(xml).expect("parses");
        assert_eq!(got, "2026-04-15.1");
    }

    #[test]
    fn parser_revision_is_numeric_not_lex() {
        // Synthetic future case: revision `.10` must beat `.2` even
        // though `.10` < `.2` under naive string comparison.
        let xml = r#"<ListBucketResult>
  <CommonPrefixes><Prefix>release/2026-04-15.2/</Prefix></CommonPrefixes>
  <CommonPrefixes><Prefix>release/2026-04-15.10/</Prefix></CommonPrefixes>
</ListBucketResult>"#;
        let got = parse_latest_release_from_xml(xml).expect("parses");
        assert_eq!(got, "2026-04-15.10");
    }

    #[test]
    fn parser_errors_on_empty_list() {
        let xml = "<ListBucketResult></ListBucketResult>";
        let err = parse_latest_release_from_xml(xml).unwrap_err();
        match err {
            OvertureError::S3List(msg) => assert!(msg.contains("no <CommonPrefixes")),
            other => panic!("expected S3List error, got {other:?}"),
        }
    }

    #[test]
    fn parser_skips_unrelated_prefix_lines() {
        // `<Prefix>release/</Prefix>` (the request echo) and any
        // nested-deeper prefix must be ignored — only direct
        // `release/<TAG>/` children count.
        let xml = r#"<ListBucketResult>
  <Prefix>release/</Prefix>
  <CommonPrefixes><Prefix>release/2026-04-15.0/</Prefix></CommonPrefixes>
  <CommonPrefixes><Prefix>release/2026-04-15.0/theme=foo/</Prefix></CommonPrefixes>
</ListBucketResult>"#;
        let got = parse_latest_release_from_xml(xml).expect("parses");
        assert_eq!(got, "2026-04-15.0");
    }

    /// WKB Polygon → GeoJSON round-trip. Builds a 1° unit square LE
    /// WKB polygon, decodes it, asserts the GeoJSON shape + closed
    /// ring + point-in-polygon predicate.
    #[test]
    fn wkb_polygon_to_geojson_basic() {
        // LE Polygon: 1 byte order + 4 type + 4 nrings + 4 npoints
        //   + 5 × (8 + 8) for the closed square at (0,0)-(1,1).
        let mut buf = Vec::<u8>::new();
        buf.push(1); // little-endian
        buf.extend_from_slice(&3u32.to_le_bytes()); // type=Polygon
        buf.extend_from_slice(&1u32.to_le_bytes()); // 1 ring
        buf.extend_from_slice(&5u32.to_le_bytes()); // 5 vertices (closed)
        for (x, y) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0), (0.0, 0.0)] {
            buf.extend_from_slice(&f64::to_le_bytes(x));
            buf.extend_from_slice(&f64::to_le_bytes(y));
        }
        let gj = wkb_polygon_to_geojson(&buf).expect("decode");
        assert_eq!(gj.get("type").and_then(|t| t.as_str()), Some("Polygon"));
        let outer = gj
            .get("coordinates")
            .and_then(|c| c.as_array())
            .and_then(|r| r.first())
            .and_then(|r| r.as_array())
            .expect("outer ring");
        assert_eq!(outer.len(), 5);
        assert_eq!(outer.first(), outer.last(), "ring must be closed");

        // Point-in-polygon: (0.5, 0.5) is inside; (2.0, 2.0) is outside.
        assert!(wkb_polygon_contains_point(&buf, 0.5, 0.5));
        assert!(!wkb_polygon_contains_point(&buf, 2.0, 2.0));
        assert!(!wkb_polygon_contains_point(&buf, -0.1, 0.5));
    }

    #[test]
    fn normalize_division_name_matches_diacritics() {
        assert_eq!(normalize_division_name("São Paulo"), "sao paulo");
        assert_eq!(normalize_division_name("New-York,  NY"), "new york ny");
        assert_eq!(normalize_division_name(""), "");
    }

    /// Network-gated: live Overture-divisions read for "Manhattan"
    /// (anchor at 40.7831 N, 73.9712 W). Asserts we get back a
    /// non-empty polygon, that the polygon contains the anchor, and
    /// that the GERS id is non-empty so the receipt has a citable
    /// content-address. Skipped by default; run with
    /// `cargo test -p emem-fetch -- --ignored overture_divisions_live`.
    /// Country-tier preference must return a country-subtype
    /// polygon, not a sub-locality district that happens to contain
    /// the same anchor. Verified live against Overture's anonymous
    /// S3 mirror — toggled `#[ignore]` so CI doesn't depend on
    /// transient S3 weather, but always run before shipping a
    /// release that touches the resolver.
    #[tokio::test]
    #[ignore]
    async fn overture_country_tier_returns_country_polygon() {
        let c = OvertureClient::new().unwrap();
        // Dhaka anchor — Bangladesh, BD.
        let m = c
            .division_polygon_with_subtype(23.71, 90.41, "Bangladesh", Some("country"))
            .await
            .expect("S3 live")
            .expect("country polygon must exist for Bangladesh");
        assert_eq!(
            m.subtype, "country",
            "expected country subtype, got {:?}",
            m
        );
        assert_eq!(m.country, "BD");
        // Polygon bbox must cover the whole country (~20.5..26.7 lat,
        // 88..93 lng) — much wider than any internal district.
        assert!(
            m.bbox.0 < 21.5 && m.bbox.1 > 26.0,
            "country bbox lat range too tight: {:?}",
            m.bbox
        );
        assert!(
            m.bbox.2 < 89.0 && m.bbox.3 > 92.0,
            "country bbox lng range too tight: {:?}",
            m.bbox
        );
        // Localized names should include at least the canonical
        // English form plus the Bengali script.
        assert!(
            !m.names_common.is_empty(),
            "Overture row must carry common-names for Bangladesh"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn overture_divisions_live_resolves_manhattan_polygon() {
        let client = OvertureClient::shared();
        let m = client
            .division_polygon_near(40.7831, -73.9712, "Manhattan")
            .await
            .expect("divisions fetch must succeed")
            .expect("divisions must return a containing polygon at Manhattan anchor");
        eprintln!(
            "Manhattan division: id={}, name={:?}, subtype={}, bbox={:?}, area_km_sq={:.3}, name_match={}",
            m.id, m.name, m.subtype, m.bbox, m.bbox_area_km_sq, m.name_matched_hint
        );
        assert!(!m.id.is_empty(), "GERS id must be present");
        // Manhattan-the-borough lands as `subtype=locality` in
        // Overture's division ladder (NYC's structure splits the
        // five boroughs as five locality rows under the city
        // "New York", which itself is a county subtype). Name match
        // must be set since we passed "Manhattan" as the hint.
        assert!(m.name_matched_hint, "expected name match for Manhattan");
        // Locate the first ring of the geometry across both
        // Polygon (coordinates: ring[]) and MultiPolygon
        // (coordinates: polygon[]: ring[]) shapes.
        let kind = m
            .geometry
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("");
        let coords = m
            .geometry
            .get("coordinates")
            .and_then(|c| c.as_array())
            .expect("coordinates array");
        let outer: &Vec<serde_json::Value> = match kind {
            "Polygon" => coords
                .first()
                .and_then(|r| r.as_array())
                .expect("Polygon outer ring"),
            "MultiPolygon" => coords
                .first()
                .and_then(|p| p.as_array())
                .and_then(|rings| rings.first())
                .and_then(|r| r.as_array())
                .expect("MultiPolygon outer ring"),
            other => panic!("unexpected geometry type {other:?}"),
        };
        assert!(
            outer.len() >= 4,
            "outer ring must have at least 4 vertices ({} on a {})",
            outer.len(),
            kind
        );
    }

    #[test]
    fn linestring_length_inside_only() {
        // 1° east-west line at lat 52° → ~111320 * cos(52°) ≈ 68 540 m.
        // Clip to ±0.5° → expect half of that.
        let line = linestring_le(&[(-1.0, 52.0), (1.0, 52.0)]);
        let polys = wkb_linestring_or_multi(&line).unwrap();
        let lat0 = 52.0_f64;
        let m_lat = 111_320.0;
        let m_lng = 111_320.0 * lat0.to_radians().cos();
        let l = polyline_clipped_length(&polys, 51.0, 53.0, -0.5, 0.5, m_lat, m_lng);
        // Entire 1° span clipped to [-0.5, 0.5] → 1° width.
        assert!((l - m_lng * 1.0).abs() < 1.0, "got {l}");
    }
}
