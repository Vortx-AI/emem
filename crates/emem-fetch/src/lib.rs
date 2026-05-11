//! emem-fetch — source-connector framework for **lazy fact materialization**.
//!
//! emem is a global memory: agents recall `(cell, band, tslot)` triples and
//! the protocol either returns a cached fact or fetches the canonical
//! upstream sources, computes the band value, attests, caches forever, and
//! returns. This crate is the *fetch* half of that pipeline.
//!
//! Connectors are pluggable per `ConnectorKind` (spec §15). The default
//! sources manifest (`emem-core::sources`) drives URL templating; operators
//! may swap manifests to add mirrors, auth, or new providers without
//! recompiling.
//!
//! Connectors implement `SourceConnector`. The dispatcher routes a fetch
//! request through the registry, picks a provider (failover-aware), and
//! returns raw bytes + metadata. Decoding (GeoTIFF → array) lives in
//! a separate decoder layer per source kind.

#![forbid(unsafe_code)]

use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub mod cache_window;
pub mod chirps;
pub mod cog;
pub mod connectors;
pub mod dmsp_ols;
pub mod firms;
pub mod ftw;
pub mod hansen_gfc;
pub mod koppen;
pub mod overture;
pub mod proj;
pub mod stac;
pub mod template;
pub mod terraclimate;
pub mod wdpa;
pub mod worldpop;

/// A single fetch request.
///
/// `cell` is the cache key (content-addressed hex tessellation); `bbox`
/// is the geographic shape connectors use to compute upstream tile URLs
/// and Range windows.  Most production calls carry both — `cell` decides
/// hit/miss in the cache, `bbox` decides what to fetch on miss.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchRequest {
    /// Source scheme key (must exist in the active sources manifest).
    pub scheme: String,
    /// Cell whose data we want; the connector resolves which provider tile
    /// covers this cell.
    pub cell: emem_core::Cell,
    /// WGS84 bounding box for the request region.  Required for any
    /// scheme whose template references bbox-derived variables (most
    /// global COG sources).  When omitted, only schemes whose templates
    /// reference no geographic vars (or only caller-supplied vars) will
    /// resolve.
    #[serde(default)]
    pub bbox: Option<emem_core::Bbox>,
    /// Time slot.
    pub tslot: emem_core::Tslot,
    /// Specific channels (e.g. ["B04", "B08"] for Sentinel-2).
    pub channels: Vec<String>,
    /// Optional caller-provided template variables (override defaults).
    #[serde(default)]
    pub vars: HashMap<String, String>,
}

/// Bytes + metadata returned from a fetch.
#[derive(Debug, Clone)]
pub struct FetchResponse {
    /// Raw payload bytes (typically a GeoTIFF tile or a window of one).
    pub bytes: bytes::Bytes,
    /// Provider that served the response.
    pub provider_id: String,
    /// Source CID derived from `blake3(bytes)` — used as `Source.cid` in
    /// downstream attestation.
    pub source_cid: String,
    /// Provider-reported capture timestamp (ISO 8601), if available.
    pub captured_at: Option<String>,
    /// HTTP/equivalent status. Useful for rate-limit backoff hints.
    pub status: u16,
}

/// Errors during fetch.
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    /// Scheme not in the active sources manifest.
    #[error("unknown source scheme: {0}")]
    UnknownScheme(String),
    /// All providers for this scheme failed.
    #[error("all providers failed for {scheme}: {detail}")]
    AllProvidersFailed { scheme: String, detail: String },
    /// Network or auth error.
    #[error("transport: {0}")]
    Transport(String),
    /// Provider rate-limited.
    #[error("rate limited by {provider_id}; retry after {retry_after_s}s")]
    RateLimited {
        provider_id: String,
        retry_after_s: u32,
    },
    /// Template variable was not provided.
    #[error("missing template variable: {0}")]
    MissingVariable(String),
}

/// A connector implementation for one ConnectorKind.
#[async_trait]
pub trait SourceConnector: Send + Sync {
    /// Connector kind this implementation handles.
    fn kind(&self) -> emem_core::ConnectorKind;

    /// Fetch the full resolved URL (after template expansion).
    async fn fetch(&self, url: &str, auth: &str) -> Result<FetchResponse, FetchError>;

    /// Fetch a byte range `[start, end_inclusive]` via HTTP `Range`.  The
    /// default impl returns `Transport("range not supported")`; HTTP-based
    /// connectors override this to enable cheap COG window reads.  Range
    /// reads are *the* mechanism that makes vsicurl-style global lazy
    /// fetch viable: a Sentinel-2 scene is ~1 GB but a 5x5 km AOI
    /// touches only a few hundred KB of IFD + tile data.
    async fn fetch_range(
        &self,
        url: &str,
        _auth: &str,
        _start: u64,
        _end_inclusive: u64,
    ) -> Result<FetchResponse, FetchError> {
        Err(FetchError::Transport(format!(
            "range not supported for connector serving {url}"
        )))
    }
}

/// Dispatcher that picks a connector by ConnectorKind and a provider by
/// failover order from the sources manifest.
pub struct Dispatcher {
    connectors: HashMap<emem_core::ConnectorKind, Box<dyn SourceConnector>>,
}

impl Dispatcher {
    /// Build a new dispatcher with no registered connectors.
    pub fn new() -> Self {
        Self {
            connectors: HashMap::new(),
        }
    }

    /// Register a connector for its `kind()`. Last-registered wins.
    pub fn register(&mut self, c: Box<dyn SourceConnector>) {
        self.connectors.insert(c.kind(), c);
    }

    /// Fetch a single request, trying each provider in failover order.
    pub async fn fetch(
        &self,
        sources: &emem_core::SourceRegistry,
        req: &FetchRequest,
    ) -> Result<FetchResponse, FetchError> {
        let scheme = sources
            .lookup(&req.scheme)
            .ok_or_else(|| FetchError::UnknownScheme(req.scheme.clone()))?;
        let mut last_err: Option<String> = None;
        for prov in &scheme.providers {
            let connector = match self.connectors.get(&prov.kind) {
                Some(c) => c,
                None => {
                    last_err = Some(format!("no connector for {:?}", prov.kind));
                    continue;
                }
            };
            let url = match &prov.url_template {
                Some(t) => template::expand(t, req)?,
                None => match &prov.cid {
                    Some(cid) => format!("ipld:{cid}"),
                    None => {
                        last_err = Some("no url_template or cid".into());
                        continue;
                    }
                },
            };
            match connector.fetch(&url, &prov.auth).await {
                Ok(resp) => return Ok(resp),
                Err(e) => {
                    last_err = Some(format!("{}: {e}", prov.id));
                }
            }
        }
        Err(FetchError::AllProvidersFailed {
            scheme: req.scheme.clone(),
            detail: last_err.unwrap_or_else(|| "no providers".into()),
        })
    }
}

impl Default for Dispatcher {
    fn default() -> Self {
        Self::new()
    }
}
