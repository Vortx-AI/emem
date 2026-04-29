//! Connector implementations per [`emem_core::ConnectorKind`].
//!
//! Anonymous HTTPS + public-bucket GCS work today, including HTTP `Range`
//! reads (vsicurl-style COG window fetch). IPLD / STAC backends are
//! registered separately by operators with native bindings; this crate
//! does not embed them.

use std::time::Duration;

use async_trait::async_trait;
use blake3::Hasher;
use data_encoding::BASE32_NOPAD;

use crate::{FetchError, FetchResponse, SourceConnector};
use emem_core::ConnectorKind;

fn cid_for(bytes: &[u8]) -> String {
    let mut h = Hasher::new();
    h.update(bytes);
    BASE32_NOPAD
        .encode(&h.finalize().as_bytes()[..32])
        .to_lowercase()
}

/// Map a reqwest error to `FetchError::Transport` with a stable string.
fn transport(e: impl std::fmt::Display) -> FetchError {
    FetchError::Transport(e.to_string())
}

/// Inspect the response for `429 Too Many Requests` and return the
/// matching [`FetchError::RateLimited`] if applicable.  Otherwise return
/// the response unchanged.
fn check_rate_limit(resp: reqwest::Response, url: &str) -> Result<reqwest::Response, FetchError> {
    if resp.status().as_u16() != 429 {
        return Ok(resp);
    }
    let retry_after_s = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);
    Err(FetchError::RateLimited {
        provider_id: url.to_string(),
        retry_after_s,
    })
}

/// Anonymous HTTPS GET via reqwest, with `Range` support for COG window
/// reads.  Use the same instance for both whole-object fetches and
/// per-window range reads — the underlying connection pool is shared.
pub struct HttpsConnector {
    client: reqwest::Client,
    kind: ConnectorKind,
}

impl HttpsConnector {
    /// Build with a dedicated client.  We tune timeouts and connection
    /// reuse for the COG access pattern: many small Range reads against
    /// the same hosts, with the occasional multi-MB whole-object pull.
    pub fn new(kind: ConnectorKind) -> Self {
        let client = reqwest::Client::builder()
            .user_agent(concat!("emem/", env!("CARGO_PKG_VERSION")))
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(8)
            .timeout(Duration::from_secs(120))
            .connect_timeout(Duration::from_secs(15))
            // `Accept-Encoding: identity` avoids servers gzipping
            // GeoTIFF bytes — Range offsets must point at the original
            // file or the COG IFD math breaks.
            .build()
            .expect("reqwest client build");
        Self { client, kind }
    }

    async fn build_response(
        resp: reqwest::Response,
        url: &str,
    ) -> Result<FetchResponse, FetchError> {
        let status = resp.status().as_u16();
        let resp = check_rate_limit(resp, url)?;
        if !(200..300).contains(&status) {
            return Err(FetchError::Transport(format!("HTTP {status} from {url}")));
        }
        let captured_at = resp
            .headers()
            .get("last-modified")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let bytes = resp.bytes().await.map_err(transport)?;
        let source_cid = cid_for(&bytes);
        Ok(FetchResponse {
            bytes,
            provider_id: url.to_string(),
            source_cid,
            captured_at,
            status,
        })
    }
}

#[async_trait]
impl SourceConnector for HttpsConnector {
    fn kind(&self) -> ConnectorKind {
        self.kind
    }

    async fn fetch(&self, url: &str, _auth: &str) -> Result<FetchResponse, FetchError> {
        let resp = self
            .client
            .get(url)
            .header(reqwest::header::ACCEPT_ENCODING, "identity")
            .send()
            .await
            .map_err(transport)?;
        Self::build_response(resp, url).await
    }

    async fn fetch_range(
        &self,
        url: &str,
        _auth: &str,
        start: u64,
        end_inclusive: u64,
    ) -> Result<FetchResponse, FetchError> {
        if end_inclusive < start {
            return Err(FetchError::Transport(format!(
                "invalid range [{start},{end_inclusive}] for {url}"
            )));
        }
        let resp = self
            .client
            .get(url)
            .header(reqwest::header::ACCEPT_ENCODING, "identity")
            .header(
                reqwest::header::RANGE,
                format!("bytes={start}-{end_inclusive}"),
            )
            .send()
            .await
            .map_err(transport)?;
        Self::build_response(resp, url).await
    }
}

/// Public-bucket GCS connector.  Anonymous reads of `gs://...` are
/// served by `https://storage.googleapis.com/<bucket>/<object>`; this
/// rewrite lets us reuse [`HttpsConnector`] without a typed GCS client.
/// Authenticated buckets need a separate `GcsConnector` impl that signs
/// with workload identity — that's out of scope until a non-public
/// source lands.
pub struct GcsConnector {
    inner: HttpsConnector,
}

impl GcsConnector {
    /// Build a GCS connector that rewrites `gs://...` to public HTTPS.
    pub fn new() -> Self {
        Self {
            inner: HttpsConnector::new(ConnectorKind::GcsCog),
        }
    }

    fn rewrite(url: &str) -> String {
        if let Some(rest) = url.strip_prefix("gs://") {
            format!("https://storage.googleapis.com/{rest}")
        } else {
            url.to_string()
        }
    }
}

impl Default for GcsConnector {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SourceConnector for GcsConnector {
    fn kind(&self) -> ConnectorKind {
        ConnectorKind::GcsCog
    }

    async fn fetch(&self, url: &str, auth: &str) -> Result<FetchResponse, FetchError> {
        self.inner.fetch(&Self::rewrite(url), auth).await
    }

    async fn fetch_range(
        &self,
        url: &str,
        auth: &str,
        start: u64,
        end_inclusive: u64,
    ) -> Result<FetchResponse, FetchError> {
        self.inner
            .fetch_range(&Self::rewrite(url), auth, start, end_inclusive)
            .await
    }
}

/// IPLD-cid bundle connector — bytes come from a local content store.
/// Operators wire a `BlockstoreConnector` (their preferred IPLD store)
/// to make this kind resolvable; the default registration leaves it
/// unbound and returns `Unauthorized`-equivalent on attempted fetch.
pub struct IpldConnector;

#[async_trait]
impl SourceConnector for IpldConnector {
    fn kind(&self) -> ConnectorKind {
        ConnectorKind::IpldCid
    }

    async fn fetch(&self, url: &str, _auth: &str) -> Result<FetchResponse, FetchError> {
        Err(FetchError::Transport(format!(
            "IPLD connector requires an operator-registered blockstore; requested {url}"
        )))
    }
}

/// Register the default open-data HTTP family connectors on a dispatcher.
/// Covers anonymous HTTPS GeoTIFF and vsicurl-style COG Range reads, plus
/// public GCS bucket rewrites. Operators add authenticated providers
/// (Earthdata, Requester-Pays GCS, etc.) by registering additional connectors.
pub fn register_default_https(disp: &mut crate::Dispatcher) {
    disp.register(Box::new(HttpsConnector::new(ConnectorKind::HttpsGeotiff)));
    disp.register(Box::new(HttpsConnector::new(
        ConnectorKind::HttpsCogVsicurl,
    )));
    disp.register(Box::new(GcsConnector::new()));
}

/// Build a fresh dispatcher with only no-auth, public open-data connectors
/// pre-registered. The default emem build uses this — agents can recall
/// against Copernicus DEM, JRC GSW, Hansen GFC, ESA WorldCover and other
/// open providers without operator credentials.
pub fn open_data_dispatcher() -> crate::Dispatcher {
    let mut d = crate::Dispatcher::new();
    register_default_https(&mut d);
    d
}
