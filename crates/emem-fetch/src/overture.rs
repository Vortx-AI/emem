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
//!  - The cell bbox is *small* (~305 m × ~190 m at 52° N) so most cells
//!    touch one or two row groups across one or two parquet files. Future
//!    cells in the same neighborhood reuse the same footer cache.
//!
//! Error model: every failure surfaces as `OvertureError` so the caller
//! (the materializer in emem-api-rest) can record it as a `skip_reason`
//! on the recall response. We never fall back to a placeholder value —
//! an empty cell is a real `0` (no buildings inside the bbox), but a
//! transport failure is an `Err`, not a hidden zero.

#![allow(clippy::result_large_err)]

use std::sync::Arc;

use arrow::array::{Array, BinaryArray, Float32Array, Float64Array, LargeBinaryArray, StructArray};
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

/// Default Overture release. Each release is immutable; we pin the stable
/// vintage so derivation hashes stay reproducible. Override with the env
/// var `EMEM_OVERTURE_RELEASE` if a downstream pipeline needs a different
/// snapshot.
pub const DEFAULT_RELEASE: &str = "2026-04-15.0";

/// S3 bucket Overture publishes to. Anonymous access; same bucket the
/// `overturemaps` CLI uses by default.
pub const BUCKET: &str = "overturemaps-us-west-2";
pub const REGION: &str = "us-west-2";

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

/// Anonymous S3 reader for one Overture release.
pub struct OvertureClient {
    /// Release tag, e.g. "2026-04-15.0".
    release: String,
    /// Anonymous S3 store (no signing, no key).
    store: Arc<dyn ObjectStore>,
    /// File list per (theme, type), populated lazily.
    file_lists: Mutex<std::collections::HashMap<(&'static str, &'static str), Vec<String>>>,
    /// Decoded parquet footers per S3 key.
    footers: Mutex<FooterCache>,
}

impl OvertureClient {
    /// Build an anonymous S3 reader for the configured release.
    pub fn new() -> Result<Self, OvertureError> {
        let release =
            std::env::var("EMEM_OVERTURE_RELEASE").unwrap_or_else(|_| DEFAULT_RELEASE.to_string());
        let store = AmazonS3Builder::new()
            .with_region(REGION)
            .with_bucket_name(BUCKET)
            .with_unsigned_payload(true)
            .with_skip_signature(true)
            .build()
            .map_err(|e| OvertureError::Init(format!("AmazonS3Builder: {e}")))?;
        Ok(Self {
            release,
            store: Arc::new(store),
            file_lists: Mutex::new(Default::default()),
            footers: Mutex::new(Default::default()),
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

    /// Public release tag the client is reading from.
    pub fn release(&self) -> &str {
        &self.release
    }

    /// List parquet files under `release/<theme>/<typ>/`, caching the result.
    async fn list_files(&self, tt: ThemeType) -> Result<Vec<String>, OvertureError> {
        {
            let g = self.file_lists.lock().await;
            if let Some(v) = g.get(&(tt.theme, tt.typ)) {
                return Ok(v.clone());
            }
        }
        let prefix = format!(
            "release/{}/theme={}/type={}/",
            self.release, tt.theme, tt.typ
        );
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
        g.insert((tt.theme, tt.typ), out.clone());
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

    /// Build a fresh row-batch stream over the chosen row groups for the
    /// columns we need: `bbox` (struct) + `geometry` (WKB binary).
    async fn open_stream(
        &self,
        key: &str,
        row_groups: Vec<usize>,
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
                match parts[0].as_str() {
                    "bbox" | "geometry" => leaf_idx.push(i),
                    _ => {}
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
        // Local-tangent-plane scaling; exact at lat0, accurate to a few cm
        // over a 305 m cell.
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
        let mut stream = self.open_stream(key, rgs).await?;
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
        let mut stream = self.open_stream(key, rgs).await?;
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

/// Test if a WKB Polygon's vertex centroid falls inside the bbox.
/// (Average of outer-ring vertices — adequate for a ~305 m cell; the
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
/// (~305 m) makes the planar approximation indistinguishable from haversine.
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
