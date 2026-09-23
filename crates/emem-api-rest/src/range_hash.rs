//! `POST /v1/range_hash`: BLAKE3 of an exact byte range of a public object, as
//! fetched by this responder, under a signed receipt.
//!
//! A pointer note names a large object by the hash of every chunk its author
//! read, and until now every one of those bytes crossed the network into
//! someone's browser. This lets the responder read a range next to the data and
//! say, under its own key, "these `length` bytes at `offset` of this url hashed
//! to this, at this time, with this ETag". That is a witness a pointer's author
//! does not control, which a count of browser keys is not.
//!
//! It is caller-directed egress from our address, which is server-side request
//! forgery unless bounded, and an amplifier unless metered. So:
//!   - https on port 443 only, no credentials in the url, no IP literals, no
//!     local names;
//!   - the name is resolved once, every address must be publicly routable, and
//!     the connection is PINNED to those addresses, so a second resolution
//!     cannot rebind it somewhere private;
//!   - at most three redirects, each target admitted and pinned by the same
//!     rules before it is fetched (a redirect followed by the client would be
//!     a destination the check never saw); the receipt names both the url
//!     asked and the url the bytes came from;
//!   - the upstream must answer 206 with a Content-Range for exactly the bytes
//!     asked. A 200 means it ignored the range, and is refused rather than
//!     truncated, because we would be hashing bytes we chose and not bytes it
//!     named;
//!   - at most `EMEM_RANGE_HASH_MAX_BYTES` (16 MiB) per call, read as a stream
//!     and abandoned at the first byte over;
//!   - a per-IP daily quota and a small global concurrency cap.

use std::net::SocketAddr;
use std::sync::OnceLock;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use ed25519_dalek::Signer;
use serde::Deserialize;
use serde_json::json;

use crate::{AppState, ErrorBody, ErrorCode};

pub(crate) const RANGE_HASH_DOMAIN: &str = "emem.range_hash.v1";

pub(crate) mod tag {
    pub const URL: u8 = 1;
    pub const OFFSET: u8 = 2;
    pub const LENGTH: u8 = 3;
    pub const BLAKE3: u8 = 4;
    pub const ETAG: u8 = 5;
    pub const FETCHED_AT: u8 = 6;
    pub const RESPONDER_PUBKEY: u8 = 7;
    pub const FETCHED_URL: u8 = 8;
}

pub(crate) const PREIMAGE: &str = "PreimageV1(\"emem.range_hash.v1\"){1:url, 2:u64_be offset, 3:u64_be length, 4:blake3 (32 raw bytes), 5:etag or \"absent\", 6:fetched_at, 7:responder_pubkey, 8:fetched_url (after redirects)}";

fn max_bytes() -> u64 {
    std::env::var("EMEM_RANGE_HASH_MAX_BYTES")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|n| *n >= 1)
        .unwrap_or(16 << 20)
        .min(256 << 20)
}

fn daily_quota() -> u32 {
    std::env::var("EMEM_RANGE_HASH_DAILY_QUOTA")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(500)
}

fn permits() -> &'static tokio::sync::Semaphore {
    static P: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    P.get_or_init(|| tokio::sync::Semaphore::new(4))
}

#[derive(Debug, Deserialize)]
pub(crate) struct RangeHashReq {
    url: String,
    offset: u64,
    length: u64,
}

/// The host of a url this route may fetch, or why not.
pub(crate) fn admit_url(raw: &str) -> Result<(reqwest::Url, String), String> {
    let url = reqwest::Url::parse(raw).map_err(|e| format!("not a url: {e}"))?;
    if url.scheme() != "https" {
        return Err("only https urls are fetched".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("a url carrying credentials is refused".into());
    }
    if url.port().is_some_and(|p| p != 443) {
        return Err("only port 443 is fetched".into());
    }
    let host = url
        .host_str()
        .ok_or("the url has no host")?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    crate::enlistment::host_is_publicly_routable(&host)?;
    Ok((url, host))
}

/// `Content-Range: bytes a-b/total`, as `(a, b, total)`; total `None` for `*`.
pub(crate) fn parse_content_range(v: &str) -> Option<(u64, u64, Option<u64>)> {
    let rest = v.trim().strip_prefix("bytes ")?;
    let (span, total) = rest.split_once('/')?;
    let (a, b) = span.split_once('-')?;
    let total = if total == "*" {
        None
    } else {
        Some(total.parse().ok()?)
    };
    Some((a.parse().ok()?, b.parse().ok()?, total))
}

// One argument per signed segment; a struct would only rename them.
#[allow(clippy::too_many_arguments)]
pub(crate) fn range_hash_preimage(
    url: &str,
    offset: u64,
    length: u64,
    hash: &[u8; 32],
    etag: Option<&str>,
    fetched_at: &str,
    responder_pk: &[u8; 32],
    fetched_url: &str,
) -> [u8; 32] {
    let mut pb = emem_attest::PreimageV1::new(RANGE_HASH_DOMAIN);
    pb.seg(tag::URL, url.as_bytes());
    pb.seg(tag::OFFSET, &offset.to_be_bytes());
    pb.seg(tag::LENGTH, &length.to_be_bytes());
    pb.seg(tag::BLAKE3, hash);
    pb.seg(tag::ETAG, etag.unwrap_or("absent").as_bytes());
    pb.seg(tag::FETCHED_AT, fetched_at.as_bytes());
    pb.seg(tag::RESPONDER_PUBKEY, responder_pk);
    pb.seg(tag::FETCHED_URL, fetched_url.as_bytes());
    pb.finalize()
}

fn refuse(status: StatusCode, wire: &str, message: String) -> Response {
    crate::ApiError(
        status,
        ErrorBody {
            code: if status == StatusCode::BAD_GATEWAY {
                ErrorCode::SourceFetchFailed
            } else {
                ErrorCode::InvalidArgument
            },
            message,
            details: Some(json!({ "code": wire })),
        },
    )
    .into_response()
}

/// One GET of `range` from `url`, connected only to `host`'s addresses as
/// resolved and checked here, never following a redirect itself.
// The Err is a finished response returned straight to the handler, once.
#[allow(clippy::result_large_err)]
async fn fetch_pinned(
    url: &reqwest::Url,
    host: &str,
    range: &str,
) -> Result<reqwest::Response, Response> {
    let addrs: Vec<SocketAddr> = match tokio::net::lookup_host((host, 443u16)).await {
        Ok(a) => a.collect(),
        Err(e) => {
            return Err(refuse(
                StatusCode::BAD_GATEWAY,
                "unresolvable",
                format!("{host} did not resolve: {e}"),
            ))
        }
    };
    if addrs.is_empty() {
        return Err(refuse(
            StatusCode::BAD_GATEWAY,
            "unresolvable",
            format!("{host} resolved to no addresses"),
        ));
    }
    if let Some(bad) = addrs
        .iter()
        .find(|a| !crate::enlistment::addr_is_publicly_routable(a.ip()))
    {
        return Err(refuse(
            StatusCode::BAD_REQUEST,
            "url_refused",
            format!(
                "{host} resolves to {}, which is not publicly routable",
                bad.ip()
            ),
        ));
    }
    let client = reqwest::Client::builder()
        .resolve_to_addrs(host, &addrs)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent(format!(
            "emem-range-hash/1 (+{})",
            crate::public_origin().unwrap_or_else(|| crate::CANONICAL_ORIGIN.to_string())
        ))
        .build()
        .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, "client", e.to_string()))?;
    client
        .get(url.clone())
        .header("range", range)
        .header("accept-encoding", "identity")
        .send()
        .await
        .map_err(|e| {
            refuse(
                StatusCode::BAD_GATEWAY,
                "fetch_failed",
                format!("{host}: {e}"),
            )
        })
}

pub(crate) async fn post_range_hash(
    axum::extract::State(s): axum::extract::State<AppState>,
    req: axum::http::Request<axum::body::Body>,
) -> Response {
    let ip = crate::client_ip(&req).unwrap_or_else(|| "unknown".to_string());
    let Ok(body) = axum::body::to_bytes(req.into_body(), 8 * 1024).await else {
        return refuse(
            StatusCode::BAD_REQUEST,
            "invalid_argument",
            "body over 8 KiB".into(),
        );
    };
    let r: RangeHashReq = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return refuse(
                StatusCode::BAD_REQUEST,
                "invalid_argument",
                format!("expected {{url, offset, length}}: {e}"),
            )
        }
    };
    let cap = max_bytes();
    if r.length == 0 || r.length > cap {
        return refuse(
            StatusCode::BAD_REQUEST,
            "range_too_large",
            format!("length must be 1..={cap} bytes; hash a larger object as several ranges"),
        );
    }
    let Some(last) = r.offset.checked_add(r.length - 1) else {
        return refuse(
            StatusCode::BAD_REQUEST,
            "invalid_argument",
            "offset + length overflows".into(),
        );
    };
    let (url, host) = match admit_url(&r.url) {
        Ok(v) => v,
        Err(e) => return refuse(StatusCode::BAD_REQUEST, "url_refused", e),
    };
    if !crate::check_daily_quota(&ip, "range_hash", daily_quota()) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [("retry-after", "86400")],
            Json(json!({
                "code": "range_hash_quota",
                "error": format!("range_hash daily quota hit ({}/day per IP)", daily_quota()),
            })),
        )
            .into_response();
    }
    let Ok(_permit) = permits().acquire().await else {
        return refuse(
            StatusCode::SERVICE_UNAVAILABLE,
            "busy",
            "no fetch slot".into(),
        );
    };

    let range = format!("bytes={}-{last}", r.offset);
    let (mut target, mut target_host) = (url.clone(), host.clone());
    let mut hops = 0;
    let mut resp = loop {
        let resp = match fetch_pinned(&target, &target_host, &range).await {
            Ok(x) => x,
            Err(e) => return e,
        };
        if !resp.status().is_redirection() {
            break resp;
        }
        hops += 1;
        let next = resp
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .and_then(|l| target.join(l).ok());
        let Some(next) = next.filter(|_| hops <= 3) else {
            return refuse(
                StatusCode::BAD_GATEWAY,
                "too_many_redirects",
                format!("{target_host} redirected {hops} times or without a Location"),
            );
        };
        match admit_url(next.as_str()) {
            Ok((u, h)) => (target, target_host) = (u, h),
            Err(e) => {
                return refuse(
                    StatusCode::BAD_GATEWAY,
                    "redirect_refused",
                    format!("{target_host} redirected to a url this route does not fetch: {e}"),
                )
            }
        }
    };
    let host = target_host;
    if resp.status() != StatusCode::PARTIAL_CONTENT {
        return refuse(
            StatusCode::BAD_GATEWAY,
            "range_not_honoured",
            format!(
                "{host} answered {} to a byte-range request; only a 206 for exactly the bytes asked is hashed (redirects are not followed)",
                resp.status()
            ),
        );
    }
    let header = |k: &str| {
        resp.headers()
            .get(k)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let etag = header("etag");
    let total = match header("content-range")
        .as_deref()
        .and_then(parse_content_range)
    {
        Some((a, b, total)) if a == r.offset && b == last => total,
        other => {
            return refuse(
                StatusCode::BAD_GATEWAY,
                "range_not_honoured",
                format!(
                    "{host} answered 206 with Content-Range {other:?}, not bytes {}-{last}",
                    r.offset
                ),
            )
        }
    };
    let mut hasher = blake3::Hasher::new();
    let mut got: u64 = 0;
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                got += chunk.len() as u64;
                if got > r.length {
                    return refuse(
                        StatusCode::BAD_GATEWAY,
                        "range_not_honoured",
                        format!("{host} sent more than {} bytes", r.length),
                    );
                }
                hasher.update(&chunk);
            }
            Ok(None) => break,
            Err(e) => {
                return refuse(
                    StatusCode::BAD_GATEWAY,
                    "fetch_failed",
                    format!("{host}: {e}"),
                )
            }
        }
    }
    if got != r.length {
        return refuse(
            StatusCode::BAD_GATEWAY,
            "short_read",
            format!("{host} sent {got} of {} bytes", r.length),
        );
    }
    let hash = *hasher.finalize().as_bytes();
    let fetched_at = crate::chrono_iso8601_utc();
    let pk = s.identity.pubkey.0;
    let url_s = url.as_str();
    let preimage = range_hash_preimage(
        url_s,
        r.offset,
        r.length,
        &hash,
        etag.as_deref(),
        &fetched_at,
        &pk,
        target.as_str(),
    );
    let sig = s.identity.signing.sign(&preimage).to_bytes();
    let b32 = |b: &[u8]| data_encoding::BASE32_NOPAD.encode(b).to_ascii_lowercase();
    Json(json!({
        "url": url_s,
        "fetched_url": target.as_str(),
        "redirects": hops,
        "offset": r.offset,
        "length": r.length,
        "blake3_b32": b32(&hash),
        "etag": etag,
        "content_total": total,
        "fetched_at": fetched_at,
        // The pointer-row leaf for this range, as a pointer note's Merkle tree
        // commits to it (see /v1/tree). A row at the note's own source is
        // hashed with an empty url, so both are given.
        "leaf_b32": b32(&crate::tree::chunk_leaf(url_s, r.offset, r.length, &hash)),
        "leaf_at_source_b32": b32(&crate::tree::chunk_leaf("", r.offset, r.length, &hash)),
        "receipt": {
            "domain": RANGE_HASH_DOMAIN,
            "preimage": PREIMAGE,
            "responder_pubkey_b32": b32(&pk),
            "signature_b32": b32(&sig),
            "note": "this responder read these bytes from the url at fetched_at and they hashed to blake3_b32; it says nothing about what the bytes mean, and the source may change after (compare etag)",
        },
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_public_https_names_are_admitted() {
        assert!(admit_url("https://huggingface.co/x/resolve/main/model.safetensors").is_ok());
        for bad in [
            "http://example.org/a",
            "https://127.0.0.1/a",
            "https://[::1]/a",
            "https://169.254.169.254/latest/meta-data",
            "https://localhost/a",
            "https://metadata.internal/a",
            "https://user:pw@example.org/a",
            "https://example.org:8443/a",
            "file:///etc/passwd",
            "https://intranet/a",
        ] {
            assert!(admit_url(bad).is_err(), "{bad}");
        }
    }

    /// Against the network: the GPT-2 header chunk hashes to what an ememdemo
    /// pointer recorded for it from a browser. `cargo test -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn hashes_a_published_pointer_chunk_like_the_browser_did() {
        let s = crate::tests::test_app_state();
        let body = serde_json::to_vec(&json!({
            "url": "https://huggingface.co/openai-community/gpt2/resolve/main/model.safetensors",
            "offset": 0,
            "length": 14291,
        }))
        .unwrap();
        let req = axum::http::Request::post("/v1/range_hash")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body))
            .unwrap();
        let resp = post_range_hash(axum::extract::State(s), req).await;
        let status = resp.status();
        let b = axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap();
        let j: serde_json::Value = serde_json::from_slice(&b).unwrap();
        assert_eq!(status, StatusCode::OK, "{j}");
        assert_eq!(
            j["blake3_b32"], "vtqobol7vtartdjlje4rsv4aqxogghursbpy3lt3n6wvqnzzzhqq",
            "{j}"
        );
    }

    #[test]
    fn content_range_is_read_exactly() {
        assert_eq!(
            parse_content_range("bytes 0-99/1000"),
            Some((0, 99, Some(1000)))
        );
        assert_eq!(parse_content_range("bytes 5-9/*"), Some((5, 9, None)));
        assert_eq!(parse_content_range("items 0-9/10"), None);
        assert_eq!(parse_content_range("bytes 0-x/10"), None);
    }

    #[test]
    fn receipt_signs_every_field() {
        use ed25519_dalek::{Signer, SigningKey, Verifier};
        let sk = SigningKey::from_bytes(&[9u8; 32]);
        let pk = sk.verifying_key().to_bytes();
        let h = *blake3::hash(b"bytes").as_bytes();
        let base = range_hash_preimage(
            "https://a.org/x",
            0,
            5,
            &h,
            Some("\"e\""),
            "t",
            &pk,
            "https://cdn.a.org/x",
        );
        let sig = sk.sign(&base);
        sk.verifying_key().verify(&base, &sig).unwrap();
        // Change any one field and the signed digest changes.
        for other in [
            range_hash_preimage(
                "https://a.org/y",
                0,
                5,
                &h,
                Some("\"e\""),
                "t",
                &pk,
                "https://cdn.a.org/x",
            ),
            range_hash_preimage(
                "https://a.org/x",
                1,
                5,
                &h,
                Some("\"e\""),
                "t",
                &pk,
                "https://cdn.a.org/x",
            ),
            range_hash_preimage(
                "https://a.org/x",
                0,
                6,
                &h,
                Some("\"e\""),
                "t",
                &pk,
                "https://cdn.a.org/x",
            ),
            range_hash_preimage(
                "https://a.org/x",
                0,
                5,
                &[0u8; 32],
                Some("\"e\""),
                "t",
                &pk,
                "https://cdn.a.org/x",
            ),
            range_hash_preimage(
                "https://a.org/x",
                0,
                5,
                &h,
                None,
                "t",
                &pk,
                "https://cdn.a.org/x",
            ),
            range_hash_preimage(
                "https://a.org/x",
                0,
                5,
                &h,
                Some("\"e\""),
                "u",
                &pk,
                "https://cdn.a.org/x",
            ),
            range_hash_preimage(
                "https://a.org/x",
                0,
                5,
                &h,
                Some("\"e\""),
                "t",
                &pk,
                "https://a.org/x",
            ),
        ] {
            assert_ne!(other, base);
        }
    }
}
