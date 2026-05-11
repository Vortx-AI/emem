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

use arrow::array::{
    Array, BinaryArray, Float32Array, Float64Array, LargeBinaryArray, StringArray, StructArray,
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
    /// In-process memoization of `division_polygon_near` lookups.
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
            division_cache: Mutex::new(Default::default()),
        })
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
            ParquetObjectReader::new(self.store.clone(), path).with_file_size(head.size as u64);
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
        let reader =
            ParquetObjectReader::new(self.store.clone(), path).with_file_size(head.size as u64);

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

    /// Resolve a place's true administrative boundary from Overture's
    /// `divisions/division_area` theme.
    ///
    /// `lat` / `lng` should be a coarse anchor (e.g. the GeoNames
    /// centroid for the place name). The method opens a small search
    /// bbox around the anchor, scans every row group whose bbox
    /// stats overlap it, decodes WKB polygons that actually contain
    /// the anchor point, and returns the *smallest* containing
    /// boundary whose `names.primary` matches `name_hint` (ASCII-
    /// folded, case-insensitive). When no name match is found, the
    /// method returns the smallest containing polygon regardless of
    /// name — useful when the GeoNames record was a sub-locality and
    /// the parent neighborhood/locality is what we actually want.
    ///
    /// Returns `Ok(None)` when no polygon contains the anchor (e.g.
    /// rural area outside any locality boundary) — the caller falls
    /// back to centre-cell-bbox honestly. Returns `Err` only on
    /// transport / parquet decode failure.
    pub async fn division_polygon_near(
        &self,
        lat: f64,
        lng: f64,
        name_hint: &str,
    ) -> Result<Option<DivisionMatch>, OvertureError> {
        // In-process result cache: round the anchor coord to 0.01° so
        // tiny GeoNames-record jitter doesn't bypass the cache. Stores
        // `Option<DivisionMatch>` — Ok(None) results are cached too
        // so the second "look up X in a region Overture doesn't carry"
        // call doesn't re-pay the full shard scan. Static-tempo data;
        // no TTL.
        let normalized_hint = normalize_division_name(name_hint);
        let cache_key: DivisionCacheKey = (
            (lat * 100.0).round() as i32,
            (lng * 100.0).round() as i32,
            normalized_hint.clone(),
        );
        if let Some(hit) = self.division_cache.lock().await.get(&cache_key) {
            return Ok(hit.clone());
        }
        // Half-degree search window around the anchor catches every
        // locality and most regions; countries (which can span ~180°)
        // require the full-shard scan, but row-group bbox pruning on
        // the parquet keeps the I/O small even then. The half-degree
        // pick is empirical: NYC's locality polygon is ~0.3° tall,
        // Greater Tokyo ~0.5°, Berlin ~0.4° — half a degree is the
        // tightest window that contains every locality boundary
        // we'd want to substitute for a Nominatim polygon call.
        let pad = 0.5_f64;
        let s_lat = (lat - pad).max(-90.0);
        let n_lat = (lat + pad).min(90.0);
        let w_lng = (lng - pad).max(-180.0);
        let e_lng = (lng + pad).min(180.0);

        let files = self.list_files(DIVISIONS_AREA).await?;
        let normalized_hint = normalize_division_name(name_hint);
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
            self.division_cache.lock().await.insert(cache_key, None);
            return Ok(None);
        }

        // Pick the polygon that best matches the caller's intent.
        // Ranking, in order of preference (the first non-empty tier wins):
        //
        //   1. EXACT name match (normalized primary == normalized hint) →
        //      take the *highest* admin level. "Manhattan" → borough /
        //      county / region, not "Manhattan Community Board 7".
        //   2. Partial name match (substring either way) → take the
        //      smallest containing polygon. Narrow queries like
        //      "Manhattan Heights, Buffalo" land here.
        //   3. No name match → take the smallest containing polygon
        //      regardless of name. Best-effort behaviour when the
        //      GeoNames record's preferred string doesn't appear in
        //      Overture's primary name (locale / spelling drift).
        //
        // "Highest admin level" follows the Overture-divisions subtype
        // ladder. Bigger number = broader scope. Falling back to the
        // smallest *area* on ties because country boundaries are
        // sometimes split across multiple rows in Overture.
        let exact: Vec<&DivisionMatch> = candidates
            .iter()
            .filter(|m| {
                !normalized_hint.is_empty() && normalize_division_name(&m.name) == normalized_hint
            })
            .collect();
        let partial: Vec<&DivisionMatch> =
            candidates.iter().filter(|m| m.name_matched_hint).collect();

        let chosen = if !exact.is_empty() {
            *exact
                .iter()
                .max_by(|a, b| {
                    division_subtype_rank(&a.subtype)
                        .cmp(&division_subtype_rank(&b.subtype))
                        .then_with(|| {
                            a.bbox_area_deg_sq
                                .partial_cmp(&b.bbox_area_deg_sq)
                                .unwrap_or(std::cmp::Ordering::Equal)
                        })
                })
                .expect("non-empty exact")
        } else if !partial.is_empty() {
            *partial
                .iter()
                .min_by(|a, b| {
                    a.bbox_area_deg_sq
                        .partial_cmp(&b.bbox_area_deg_sq)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .expect("non-empty partial")
        } else {
            candidates
                .iter()
                .min_by(|a, b| {
                    a.bbox_area_deg_sq
                        .partial_cmp(&b.bbox_area_deg_sq)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .expect("non-empty candidates")
        };
        let result = chosen.clone();
        self.division_cache
            .lock()
            .await
            .insert(cache_key, Some(result.clone()));
        Ok(Some(result))
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
            .open_stream(key, rgs, &["id", "names", "subtype"])
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
                let x0 = bb.xmin.val(i);
                let x1 = bb.xmax.val(i);
                let y0 = bb.ymin.val(i);
                let y1 = bb.ymax.val(i);
                let bbox_area_deg_sq = (x1 - x0).abs() * (y1 - y0).abs();
                out.push(DivisionMatch {
                    id,
                    name: primary_name,
                    subtype,
                    geometry,
                    bbox: (y0, y1, x0, x1),
                    bbox_area_deg_sq,
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
#[derive(Debug, Clone)]
pub struct DivisionMatch {
    /// GERS ID — globally stable Overture identifier, citable as
    /// the source CID in receipts.
    pub id: String,
    /// `names.primary` — the canonical localized label.
    pub name: String,
    /// Subtype: `country` / `region` / `county` / `localadmin` /
    /// `locality` / `borough` / `microhood` / `neighborhood` /
    /// `dependency` / `macrohood`.
    pub subtype: String,
    /// WGS84 GeoJSON geometry (`type: "Polygon"` or `"MultiPolygon"`).
    pub geometry: serde_json::Value,
    /// Tuple `(min_lat, max_lat, min_lng, max_lng)` derived from the
    /// row's bbox struct column — surfaced so recall_polygon doesn't
    /// have to re-derive it from the GeoJSON ring.
    pub bbox: (f64, f64, f64, f64),
    /// Approx polygon size (degrees², from the bbox column). Used as
    /// a tie-break to favor the *smallest* containing boundary —
    /// i.e. the locality, not its enclosing region/country.
    pub bbox_area_deg_sq: f64,
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
/// only read `primary`; the rest is available for callers that want
/// to surface localized names later.
struct DivisionNames<'a> {
    primary: Option<&'a StringArray>,
}

impl<'a> DivisionNames<'a> {
    fn new(col: Option<&'a dyn Array>) -> Self {
        let primary = col
            .and_then(|c| c.as_any().downcast_ref::<StructArray>())
            .and_then(|s| s.column_by_name("primary"))
            .and_then(|c| c.as_any().downcast_ref::<StringArray>());
        DivisionNames { primary }
    }
    fn primary(&self, i: usize) -> Option<String> {
        let p = self.primary?;
        if p.is_null(i) {
            None
        } else {
            Some(p.value(i).to_string())
        }
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
            // Exterior ring: do the ray-cast.
            let mut prev_x = f64::NAN;
            let mut prev_y = f64::NAN;
            let mut first_x = 0.0;
            let mut first_y = 0.0;
            for j in 0..np {
                let x = match cur.read_f64() {
                    Some(v) => v,
                    None => return false,
                };
                let y = match cur.read_f64() {
                    Some(v) => v,
                    None => return false,
                };
                if j == 0 {
                    first_x = x;
                    first_y = y;
                } else {
                    let (x0, y0, x1, y1) = (prev_x, prev_y, x, y);
                    if ((y0 > py) != (y1 > py))
                        && (px < (x1 - x0) * (py - y0) / (y1 - y0 + f64::EPSILON) + x0)
                    {
                        inside = !inside;
                    }
                }
                prev_x = x;
                prev_y = y;
            }
            // Close the ring against the first vertex.
            if !prev_x.is_nan() {
                let (x0, y0, x1, y1) = (prev_x, prev_y, first_x, first_y);
                if ((y0 > py) != (y1 > py))
                    && (px < (x1 - x0) * (py - y0) / (y1 - y0 + f64::EPSILON) + x0)
                {
                    inside = !inside;
                }
            }
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

/// Decode a WKB Polygon or MultiPolygon to a GeoJSON geometry value
/// in `[lon, lat]` order with closed rings. Returns `None` for any
/// non-polygon variant or malformed buffer.
pub fn wkb_polygon_to_geojson(buf: &[u8]) -> Option<serde_json::Value> {
    let (mut cur, t) = wkb_type(buf)?;
    match t {
        WKB_POLYGON => {
            let rings = read_polygon_rings(&mut cur)?;
            Some(serde_json::json!({"type": "Polygon", "coordinates": rings}))
        }
        WKB_MULTI_POLYGON => {
            let np = cur.read_u32()?;
            let mut polys = Vec::with_capacity(np as usize);
            for _ in 0..np {
                if cur.read_sub_header() != Some(WKB_POLYGON) {
                    return None;
                }
                let rings = read_polygon_rings(&mut cur)?;
                polys.push(rings);
            }
            Some(serde_json::json!({"type": "MultiPolygon", "coordinates": polys}))
        }
        _ => None,
    }
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
            "Manhattan division: id={}, name={:?}, subtype={}, bbox={:?}, area_deg_sq={:.5}, name_match={}",
            m.id, m.name, m.subtype, m.bbox, m.bbox_area_deg_sq, m.name_matched_hint
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
