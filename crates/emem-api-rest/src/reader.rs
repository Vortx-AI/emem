//! A signed reader for web pages and images: `POST /v1/read` and `POST /v1/ocr`.
//!
//! A browser that cannot fetch a page (no CORS) used to read it through a
//! third-party text reader, so the text an agent cited was whatever that reader
//! returned, and the OCR it ran came from worker and model bytes nobody pinned.
//! Here the responder fetches under the same bounds as `range_hash` (https
//! only, DNS pinned to public addresses, redirects re-admitted per hop, a size
//! cap, a per-IP quota), hashes the exact bytes it received, and signs what it
//! read. The text is derived from those bytes; the hashes let anyone who
//! fetches the same url check whether they got the same page.
//!
//! OCR runs open-source Tesseract on the responder. Its text is `model_output`:
//! the receipt proves which image, engine and language produced it, never that
//! the reading is right.

// Every Err here is a finished response handed straight back to the handler, once.
#![allow(clippy::result_large_err)]

use std::sync::OnceLock;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use ed25519_dalek::Signer;
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};
use sha2::Digest;

use crate::range_hash::{admit_url, fetch_following, refuse};
use crate::AppState;

pub(crate) const READ_DOMAIN: &str = "emem.read.v1";
pub(crate) const OCR_DOMAIN: &str = "emem.ocr.v1";

pub(crate) mod read_tag {
    pub const URL: u8 = 1;
    pub const FETCHED_URL: u8 = 2;
    pub const BODY_BLAKE3: u8 = 3;
    pub const BODY_SHA256: u8 = 4;
    pub const ETAG: u8 = 5;
    pub const TEXT_BLAKE3: u8 = 6;
    pub const FETCHED_AT: u8 = 7;
    pub const RESPONDER_PUBKEY: u8 = 8;
}

pub(crate) mod ocr_tag {
    pub const IMAGE_BLAKE3: u8 = 1;
    pub const SOURCE: u8 = 2;
    pub const LANG: u8 = 3;
    pub const ENGINE: u8 = 4;
    pub const TEXT_BLAKE3: u8 = 5;
    pub const READ_AT: u8 = 6;
    pub const RESPONDER_PUBKEY: u8 = 7;
}

pub(crate) const DOC_PARSE_DOMAIN: &str = "emem.doc_parse.v1";

pub(crate) mod doc_tag {
    pub const KIND: u8 = 1;
    pub const TEXT_BLAKE3: u8 = 2;
    pub const RESULT_BLAKE3: u8 = 3;
    pub const PARSER: u8 = 4;
    pub const PARSED_AT: u8 = 5;
    pub const RESPONDER_PUBKEY: u8 = 6;
}

const READ_MAX_BYTES: usize = 4 << 20;
const OCR_MAX_BYTES: usize = 8 << 20;
const TEXT_MAX_CHARS: usize = 200_000;

fn b32(b: &[u8]) -> String {
    data_encoding::BASE32_NOPAD.encode(b).to_ascii_lowercase()
}

fn daily_quota(var: &str, default: u32) -> u32 {
    std::env::var(var)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn quota_refused(what: &str, n: u32) -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        [("retry-after", "86400")],
        Json(json!({"code": format!("{what}_quota"), "error": format!("{what} daily quota hit ({n}/day per IP)")})),
    )
        .into_response()
}

/// Read a response body, abandoning it at the first byte over `cap`.
async fn body_capped(
    mut resp: reqwest::Response,
    cap: usize,
    host: &str,
) -> Result<Vec<u8>, Response> {
    let mut out = Vec::new();
    loop {
        match resp.chunk().await {
            Ok(Some(c)) => {
                if out.len() + c.len() > cap {
                    return Err(refuse(
                        StatusCode::BAD_GATEWAY,
                        "too_large",
                        format!("{host} sent more than {cap} bytes; this route reads at most that"),
                    ));
                }
                out.extend_from_slice(&c);
            }
            Ok(None) => return Ok(out),
            Err(e) => {
                return Err(refuse(
                    StatusCode::BAD_GATEWAY,
                    "fetch_failed",
                    format!("{host}: {e}"),
                ))
            }
        }
    }
}

/// Fetch `raw` under the reader's bounds: admitted, pinned, redirect-checked,
/// 2xx only, capped.
async fn fetch_bytes(
    raw: &str,
    cap: usize,
) -> Result<(Vec<u8>, reqwest::Url, Option<String>, Option<String>, u32), Response> {
    let (url, host) =
        admit_url(raw).map_err(|e| refuse(StatusCode::BAD_REQUEST, "url_refused", e))?;
    let (resp, fetched, host, hops) =
        fetch_following(&url, &host, &[("accept-encoding", "identity".into())]).await?;
    if !resp.status().is_success() {
        return Err(refuse(
            StatusCode::BAD_GATEWAY,
            "upstream_status",
            format!("{host} answered {}", resp.status()),
        ));
    }
    let header = |k: &str| {
        resp.headers()
            .get(k)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let (etag, ctype) = (header("etag"), header("content-type"));
    let bytes = body_capped(resp, cap, &host).await?;
    Ok((bytes, fetched, etag, ctype, hops))
}

/// Visible text of an html document: script, style and noscript bodies
/// dropped, block ends turned into line breaks, tags removed, the common
/// entities decoded, runs of blank space collapsed. Deliberately simple: the
/// hashes of the bytes are the evidence, and this is a reading of them.
pub(crate) fn html_text(html: &str) -> (Option<String>, String) {
    let lower = html.to_ascii_lowercase();
    let title = lower.find("<title").and_then(|i| {
        let start = lower[i..].find('>')? + i + 1;
        let end = lower[start..].find("</title")? + start;
        Some(decode_entities(html[start..end].trim()))
    });
    let mut out = String::with_capacity(html.len() / 3);
    let mut i = 0;
    let bytes = html.as_bytes();
    while i < html.len() {
        if bytes[i] == b'<' {
            let rest = &lower[i..];
            let skip_to = ["script", "style", "noscript"].iter().find_map(|t| {
                (rest.starts_with(&format!("<{t}"))).then(|| {
                    rest.find(&format!("</{t}"))
                        .and_then(|e| rest[e..].find('>').map(|g| i + e + g + 1))
                        .unwrap_or(html.len())
                })
            });
            let end = match skip_to {
                Some(e) => e,
                None => rest.find('>').map(|g| i + g + 1).unwrap_or(html.len()),
            };
            let tag = &lower[i..end];
            if [
                "<br", "</p", "</div", "</li", "</h1", "</h2", "</h3", "</h4", "</tr", "<hr",
            ]
            .iter()
            .any(|b| tag.starts_with(b))
            {
                out.push('\n');
            } else {
                out.push(' ');
            }
            i = end;
        } else {
            let next = html[i..].find('<').map(|n| i + n).unwrap_or(html.len());
            out.push_str(&html[i..next]);
            i = next;
        }
    }
    let decoded = decode_entities(&out);
    let mut text = String::with_capacity(decoded.len());
    for line in decoded.lines() {
        let l = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if !l.is_empty() {
            text.push_str(&l);
            text.push('\n');
        }
    }
    (title, text)
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i..];
        let Some(semi) = after.find(';').filter(|n| *n <= 10) else {
            out.push('&');
            rest = &after[1..];
            continue;
        };
        let ent = &after[1..semi];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            e if e.starts_with("#x") || e.starts_with("#X") => u32::from_str_radix(&e[2..], 16)
                .ok()
                .and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &after[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &after[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[derive(Debug, Deserialize)]
pub(crate) struct ReadReq {
    url: String,
}

pub(crate) async fn post_read(
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
    let r: ReadReq = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return refuse(
                StatusCode::BAD_REQUEST,
                "invalid_argument",
                format!("expected {{url}}: {e}"),
            )
        }
    };
    let quota = daily_quota("EMEM_READ_DAILY_QUOTA", 500);
    if !crate::check_daily_quota(&ip, "read", quota) {
        return quota_refused("read", quota);
    }
    let (bytes, fetched, etag, ctype, hops) = match fetch_bytes(&r.url, READ_MAX_BYTES).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let ctype_l = ctype.as_deref().unwrap_or("").to_ascii_lowercase();
    let raw = String::from_utf8_lossy(&bytes);
    let (title, mut text, reading) = if ctype_l.contains("html")
        || raw.trim_start().starts_with('<')
    {
        let (t, x) = html_text(&raw);
        (t, x, "html_visible_text")
    } else if ctype_l.starts_with("text/")
        || ctype_l.contains("json")
        || ctype_l.contains("xml")
        || ctype_l.is_empty()
    {
        (None, raw.into_owned(), "as_is")
    } else {
        return refuse(
            StatusCode::UNPROCESSABLE_ENTITY,
            "not_text",
            format!("content-type {ctype_l:?} is not text; use /v1/ocr for an image or /v1/range_hash to hash bytes"),
        );
    };
    let truncated = text.chars().count() > TEXT_MAX_CHARS;
    if truncated {
        text = text.chars().take(TEXT_MAX_CHARS).collect();
    }
    let body_b3 = *blake3::hash(&bytes).as_bytes();
    let body_sha: [u8; 32] = sha2::Sha256::digest(&bytes).into();
    let text_b3 = *blake3::hash(text.as_bytes()).as_bytes();
    let fetched_at = crate::chrono_iso8601_utc();
    let pk = s.identity.pubkey.0;
    let url_s = r.url.trim();
    let mut pb = emem_attest::PreimageV1::new(READ_DOMAIN);
    pb.seg(read_tag::URL, url_s.as_bytes());
    pb.seg(read_tag::FETCHED_URL, fetched.as_str().as_bytes());
    pb.seg(read_tag::BODY_BLAKE3, &body_b3);
    pb.seg(read_tag::BODY_SHA256, &body_sha);
    pb.seg(
        read_tag::ETAG,
        etag.as_deref().unwrap_or("absent").as_bytes(),
    );
    pb.seg(read_tag::TEXT_BLAKE3, &text_b3);
    pb.seg(read_tag::FETCHED_AT, fetched_at.as_bytes());
    pb.seg(read_tag::RESPONDER_PUBKEY, &pk);
    let sig = s.identity.signing.sign(&pb.finalize()).to_bytes();
    Json(json!({
        "url": url_s,
        "fetched_url": fetched.as_str(),
        "redirects": hops,
        "content_type": ctype,
        "bytes": bytes.len(),
        "body_blake3_b32": b32(&body_b3),
        "body_sha256_hex": data_encoding::HEXLOWER.encode(&body_sha),
        "etag": etag,
        "title": title,
        "reading": reading,
        "text": text,
        "text_truncated": truncated,
        "text_blake3_b32": b32(&text_b3),
        "fetched_at": fetched_at,
        "provenance_class": "deterministic_index",
        "receipt": {
            "domain": READ_DOMAIN,
            "preimage": "PreimageV1(\"emem.read.v1\"){1:url, 2:fetched_url, 3:body_blake3, 4:body_sha256, 5:etag or \"absent\", 6:text_blake3, 7:fetched_at, 8:responder_pubkey}",
            "responder_pubkey_b32": b32(&pk),
            "signature_b32": b32(&sig),
            "note": "this responder fetched these bytes from fetched_url at fetched_at; the text is derived from them. It says nothing about whether the page is true, and the page may change (compare etag and body hashes)",
        },
    }))
    .into_response()
}

#[derive(Debug, Deserialize)]
pub(crate) struct OcrReq {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    image_b64: Option<String>,
    #[serde(default)]
    lang: Option<String>,
}

fn tesseract_bin() -> String {
    std::env::var("EMEM_TESSERACT").unwrap_or_else(|_| "tesseract".into())
}

/// The engine's own version line, or `None` when no Tesseract is installed.
async fn tesseract_version() -> Option<String> {
    static V: OnceLock<Option<String>> = OnceLock::new();
    if let Some(v) = V.get() {
        return v.clone();
    }
    let v = tokio::process::Command::new(tesseract_bin())
        .arg("--version")
        .output()
        .await
        .ok()
        .and_then(|o| {
            let all = [o.stdout, o.stderr].concat();
            String::from_utf8_lossy(&all)
                .lines()
                .next()
                .map(|l| l.trim().to_string())
        })
        .filter(|l| l.to_ascii_lowercase().starts_with("tesseract"));
    let _ = V.set(v.clone());
    v
}

/// The image formats Tesseract reads here, by their first bytes.
pub(crate) fn image_kind(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpeg")
    } else if b.starts_with(b"II*\0") || b.starts_with(b"MM\0*") {
        Some("tiff")
    } else if b.len() > 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some("webp")
    } else if b.starts_with(b"GIF8") {
        Some("gif")
    } else if b.starts_with(b"BM") {
        Some("bmp")
    } else {
        None
    }
}

pub(crate) fn lang_ok(l: &str) -> bool {
    (3..=32).contains(&l.len())
        && l.bytes()
            .all(|b| b.is_ascii_lowercase() || b == b'_' || b == b'+')
}

fn ocr_permits() -> &'static tokio::sync::Semaphore {
    static P: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    P.get_or_init(|| tokio::sync::Semaphore::new(2))
}

pub(crate) async fn post_ocr(
    axum::extract::State(s): axum::extract::State<AppState>,
    req: axum::http::Request<axum::body::Body>,
) -> Response {
    let ip = crate::client_ip(&req).unwrap_or_else(|| "unknown".to_string());
    let Ok(body) = axum::body::to_bytes(req.into_body(), OCR_MAX_BYTES * 4 / 3 + 4096).await else {
        return refuse(
            StatusCode::BAD_REQUEST,
            "invalid_argument",
            "body too large".into(),
        );
    };
    let r: OcrReq = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return refuse(
                StatusCode::BAD_REQUEST,
                "invalid_argument",
                format!("expected {{url}} or {{image_b64}}: {e}"),
            )
        }
    };
    match ocr_value(&s, &ip, r).await {
        Ok(v) => Json(v).into_response(),
        Err(resp) => resp,
    }
}

/// Run OCR for `r` as `post_ocr` does (quota, engine, signed receipt).
pub(crate) async fn ocr_value(s: &AppState, ip: &str, r: OcrReq) -> Result<JsonValue, Response> {
    let lang = r.lang.clone().unwrap_or_else(|| "eng".into());
    if !lang_ok(&lang) {
        return Err(refuse(
            StatusCode::BAD_REQUEST,
            "invalid_argument",
            format!("lang {lang:?} is not a tesseract language code"),
        ));
    }
    let Some(engine) = tesseract_version().await else {
        return Err(refuse(
            StatusCode::NOT_IMPLEMENTED,
            "ocr_unavailable",
            "this responder has no OCR engine installed".into(),
        ));
    };
    let quota = daily_quota("EMEM_OCR_DAILY_QUOTA", 200);
    if !crate::check_daily_quota(ip, "ocr", quota) {
        return Err(quota_refused("ocr", quota));
    }
    let (image, source) = match (r.url.as_deref(), r.image_b64.as_deref()) {
        (Some(u), None) => match fetch_bytes(u, OCR_MAX_BYTES).await {
            Ok((b, fetched, _, _, _)) => (b, fetched.to_string()),
            Err(e) => return Err(e),
        },
        (None, Some(b64)) => match data_encoding::BASE64.decode(b64.trim().as_bytes()) {
            Ok(b) if b.len() <= OCR_MAX_BYTES => (b, "upload".to_string()),
            Ok(_) => {
                return Err(refuse(
                    StatusCode::BAD_REQUEST,
                    "too_large",
                    format!("image over {OCR_MAX_BYTES} bytes"),
                ))
            }
            Err(e) => {
                return Err(refuse(
                    StatusCode::BAD_REQUEST,
                    "invalid_argument",
                    format!("image_b64: {e}"),
                ))
            }
        },
        _ => {
            return Err(refuse(
                StatusCode::BAD_REQUEST,
                "invalid_argument",
                "pass exactly one of url or image_b64".into(),
            ))
        }
    };
    let Some(kind) = image_kind(&image) else {
        return Err(refuse(
            StatusCode::UNPROCESSABLE_ENTITY,
            "not_an_image",
            "not a png, jpeg, tiff, webp, gif or bmp".into(),
        ));
    };
    let Ok(_permit) = ocr_permits().acquire().await else {
        return Err(refuse(
            StatusCode::SERVICE_UNAVAILABLE,
            "busy",
            "no OCR slot".into(),
        ));
    };
    let text = match run_tesseract(&image, &lang).await {
        Ok(t) => t,
        Err(e) => return Err(refuse(StatusCode::UNPROCESSABLE_ENTITY, "ocr_failed", e)),
    };
    let image_b3 = *blake3::hash(&image).as_bytes();
    let text_b3 = *blake3::hash(text.as_bytes()).as_bytes();
    let read_at = crate::chrono_iso8601_utc();
    let pk = s.identity.pubkey.0;
    let mut pb = emem_attest::PreimageV1::new(OCR_DOMAIN);
    pb.seg(ocr_tag::IMAGE_BLAKE3, &image_b3);
    pb.seg(ocr_tag::SOURCE, source.as_bytes());
    pb.seg(ocr_tag::LANG, lang.as_bytes());
    pb.seg(ocr_tag::ENGINE, engine.as_bytes());
    pb.seg(ocr_tag::TEXT_BLAKE3, &text_b3);
    pb.seg(ocr_tag::READ_AT, read_at.as_bytes());
    pb.seg(ocr_tag::RESPONDER_PUBKEY, &pk);
    let sig = s.identity.signing.sign(&pb.finalize()).to_bytes();
    Ok(json!({
        "source": source,
        "image_kind": kind,
        "image_bytes": image.len(),
        "image_blake3_b32": b32(&image_b3),
        "lang": lang,
        "engine": engine,
        "text": text,
        "text_blake3_b32": b32(&text_b3),
        "read_at": read_at,
        "provenance_class": "model_output",
        "receipt": {
            "domain": OCR_DOMAIN,
            "preimage": "PreimageV1(\"emem.ocr.v1\"){1:image_blake3, 2:source url or \"upload\", 3:lang, 4:engine, 5:text_blake3, 6:read_at, 7:responder_pubkey}",
            "responder_pubkey_b32": b32(&pk),
            "signature_b32": b32(&sig),
            "note": "model_output: this signs which image, engine and language produced this text, not that the text is a correct reading",
        },
    }))
}

#[derive(Debug, Deserialize)]
pub(crate) struct DocParseReq {
    #[serde(default)]
    text: Option<String>,
    /// A `/v1/ocr` response: its text is checked against its own blake3.
    #[serde(default)]
    ocr: Option<JsonValue>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    image_b64: Option<String>,
    #[serde(default)]
    lang: Option<String>,
    /// Lab reports: the EU product (a name like "sweet peppers" or its EU
    /// code) whose MRLs in force each row is read against.
    #[serde(default)]
    product: Option<String>,
}

/// Check an `emem.ocr.v1` receipt: this responder's key, over the preimage
/// rebuilt from the reply's own fields and `text`.
fn verify_ocr_receipt(s: &AppState, o: &JsonValue, text: &str) -> Result<(), String> {
    let dec = |f: &str| -> Result<Vec<u8>, String> {
        let v = o
            .pointer(f)
            .and_then(|v| v.as_str())
            .ok_or(format!("ocr{f} is missing"))?;
        data_encoding::BASE32_NOPAD
            .decode(v.to_ascii_uppercase().as_bytes())
            .map_err(|e| format!("ocr{f}: {e}"))
    };
    let field = |f: &str| o[f].as_str().ok_or(format!("ocr.{f} is missing"));
    let pk = dec("/receipt/responder_pubkey_b32")?;
    if pk.as_slice() != s.identity.pubkey.0.as_slice() {
        return Err("the OCR receipt is not signed by this responder".into());
    }
    let sig: [u8; 64] = dec("/receipt/signature_b32")?
        .try_into()
        .map_err(|_| "ocr.receipt.signature_b32 is not 64 bytes".to_string())?;
    let image_b3 = dec("/image_blake3_b32")?;
    let text_b3 = *blake3::hash(text.as_bytes()).as_bytes();
    let mut pb = emem_attest::PreimageV1::new(OCR_DOMAIN);
    pb.seg(ocr_tag::IMAGE_BLAKE3, &image_b3);
    pb.seg(ocr_tag::SOURCE, field("source")?.as_bytes());
    pb.seg(ocr_tag::LANG, field("lang")?.as_bytes());
    pb.seg(ocr_tag::ENGINE, field("engine")?.as_bytes());
    pb.seg(ocr_tag::TEXT_BLAKE3, &text_b3);
    pb.seg(ocr_tag::READ_AT, field("read_at")?.as_bytes());
    pb.seg(ocr_tag::RESPONDER_PUBKEY, &pk);
    let vk = ed25519_dalek::VerifyingKey::from_bytes(&s.identity.pubkey.0)
        .map_err(|e| format!("responder key: {e}"))?;
    vk.verify_strict(&pb.finalize(), &ed25519_dalek::Signature::from_bytes(&sig))
        .map_err(|_| {
            "the OCR receipt's signature does not verify over its image, text, engine and time"
                .to_string()
        })
}

const DOC_PARSER: &str = "doc_parse@1";

/// `lab_report_parse@1` and `land_record_parse@1`: the document's text,
/// from the caller, a signed `/v1/ocr` reply, or OCR run here, parsed
/// deterministically and signed over the text's hash and the result's.
async fn doc_parse(
    s: AppState,
    req: axum::http::Request<axum::body::Body>,
    kind: &'static str,
) -> Response {
    let ip = crate::client_ip(&req).unwrap_or_else(|| "unknown".to_string());
    let Ok(body) = axum::body::to_bytes(req.into_body(), OCR_MAX_BYTES * 4 / 3 + 4096).await else {
        return refuse(
            StatusCode::BAD_REQUEST,
            "invalid_argument",
            "body too large".into(),
        );
    };
    let r: DocParseReq = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return refuse(
                StatusCode::BAD_REQUEST,
                "invalid_argument",
                format!("expected {{text}}, {{ocr}}, {{url}} or {{image_b64}}: {e}"),
            )
        }
    };
    let default_lang = if kind == "land_record_parse@1" {
        "eng+hin+mar"
    } else {
        "eng"
    };
    let (text, ocr) = if let Some(t) = r.text {
        // The text path does no OCR but still costs a parse: its own quota.
        let quota = daily_quota("EMEM_DOC_PARSE_DAILY_QUOTA", 2_000);
        if !crate::check_daily_quota(&ip, "doc_parse", quota) {
            return quota_refused("doc_parse", quota);
        }
        (t, None)
    } else if let Some(o) = r.ocr {
        let Some(t) = o["text"].as_str().map(str::to_string) else {
            return refuse(
                StatusCode::BAD_REQUEST,
                "invalid_argument",
                "ocr.text is missing".into(),
            );
        };
        // The receipt must be one this responder signed over this text: a
        // caller-built object with a made-up signature was echoed back
        // under a real one, as though emem had read that image.
        if let Err(why) = verify_ocr_receipt(&s, &o, &t) {
            return refuse(StatusCode::UNPROCESSABLE_ENTITY, "ocr_receipt_invalid", why);
        }
        if o["text_blake3_b32"].as_str()
            != Some(b32(blake3::hash(t.as_bytes()).as_bytes()).as_str())
        {
            return refuse(
                StatusCode::UNPROCESSABLE_ENTITY,
                "text_hash_mismatch",
                "ocr.text does not hash to ocr.text_blake3_b32, so it is not the text that receipt signs".into(),
            );
        }
        (t, Some(o))
    } else if r.url.is_some() || r.image_b64.is_some() {
        let o = OcrReq {
            url: r.url,
            image_b64: r.image_b64,
            lang: Some(r.lang.unwrap_or_else(|| default_lang.to_string())),
        };
        match ocr_value(&s, &ip, o).await {
            Ok(v) => (v["text"].as_str().unwrap_or("").to_string(), Some(v)),
            Err(resp) => return resp,
        }
    } else {
        return refuse(
            StatusCode::BAD_REQUEST,
            "invalid_argument",
            "pass text, ocr, url or image_b64".into(),
        );
    };
    if text.len() > TEXT_MAX_CHARS * 4 {
        return refuse(StatusCode::BAD_REQUEST, "too_large", "text too long".into());
    }
    // CPU work off the async workers.
    let parse_text = text.clone();
    let mut result = tokio::task::spawn_blocking(move || match kind {
        "lab_report_parse@1" => {
            serde_json::to_value(crate::doc_parse::parse_lab_report(&parse_text))
        }
        _ => serde_json::to_value(crate::doc_parse::parse_land_record(&parse_text)),
    })
    .await
    .ok()
    .and_then(Result::ok)
    .unwrap_or(JsonValue::Null);
    // The limit in force, not only the one the report prints.
    if kind == "lab_report_parse@1" {
        if let Some(product) = r.product.as_deref().filter(|p| !p.trim().is_empty()) {
            let lookup = crate::eu_mrl::annotate(&mut result, product).await;
            if lookup["exceedances"].as_u64().unwrap_or(0) > 0 {
                result["verdict"] = json!("exceeds_eu_mrl");
            }
            result["eu_mrl_lookup"] = lookup;
        }
    }
    let text_b3 = *blake3::hash(text.as_bytes()).as_bytes();
    let result_b3 = *blake3::hash(&serde_json::to_vec(&result).unwrap_or_default()).as_bytes();
    let parsed_at = crate::chrono_iso8601_utc();
    let pk = s.identity.pubkey.0;
    let mut pb = emem_attest::PreimageV1::new(DOC_PARSE_DOMAIN);
    pb.seg(doc_tag::KIND, kind.as_bytes());
    pb.seg(doc_tag::TEXT_BLAKE3, &text_b3);
    pb.seg(doc_tag::RESULT_BLAKE3, &result_b3);
    pb.seg(doc_tag::PARSER, DOC_PARSER.as_bytes());
    pb.seg(doc_tag::PARSED_AT, parsed_at.as_bytes());
    pb.seg(doc_tag::RESPONDER_PUBKEY, &pk);
    let sig = s.identity.signing.sign(&pb.finalize()).to_bytes();
    Json(json!({
        "schema": "emem.doc_parse.v1",
        "signal": kind,
        "parser": DOC_PARSER,
        "text_blake3_b32": b32(&text_b3),
        "result_blake3_b32": b32(&result_b3),
        "result": result,
        "ocr": ocr,
        "parsed_at": parsed_at,
        "provenance_class": "deterministic_index",
        "reading": "Each value carries its line and byte offset in the text; the text hashes to text_blake3_b32, which the OCR receipt signs against the image. The parser reads what the document prints. It does not check a residue against the EU MRL database, a land record against its registry, or a signature for authenticity.",
        "receipt": {
            "domain": DOC_PARSE_DOMAIN,
            "preimage": "PreimageV1(\"emem.doc_parse.v1\"){1:kind, 2:text_blake3, 3:result_blake3 (blake3 of the serialised result), 4:parser, 5:parsed_at, 6:responder_pubkey}",
            "responder_pubkey_b32": b32(&pk),
            "signature_b32": b32(&sig),
        },
    }))
    .into_response()
}

pub(crate) async fn post_lab_report_parse(
    axum::extract::State(s): axum::extract::State<AppState>,
    req: axum::http::Request<axum::body::Body>,
) -> Response {
    doc_parse(s, req, "lab_report_parse@1").await
}

pub(crate) async fn post_land_record_parse(
    axum::extract::State(s): axum::extract::State<AppState>,
    req: axum::http::Request<axum::body::Body>,
) -> Response {
    doc_parse(s, req, "land_record_parse@1").await
}

async fn run_tesseract(image: &[u8], lang: &str) -> Result<String, String> {
    use tokio::io::AsyncWriteExt;
    let mut child = tokio::process::Command::new(tesseract_bin())
        .args(["stdin", "stdout", "-l", lang])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("could not start tesseract: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("tesseract stdin unavailable")?;
    let input = image.to_vec();
    let writer = tokio::spawn(async move {
        let _ = stdin.write_all(&input).await;
    });
    let out = tokio::time::timeout(std::time::Duration::from_secs(45), child.wait_with_output())
        .await
        .map_err(|_| "tesseract ran past 45 s and was stopped".to_string())?
        .map_err(|e| format!("tesseract: {e}"))?;
    let _ = writer.await;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "tesseract failed: {}",
            err.lines().last().unwrap_or("")
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_text_keeps_what_a_reader_sees() {
        let html = "<html><head><title>A &amp; B</title><style>p{color:red}</style>\
                    <script>var x='<p>not text</p>';</script></head><body><h1>Head</h1>\
                    <p>One &lt;two&gt; &#233;t&eacute;</p><div>Three<br>Four</div></body></html>";
        let (title, text) = html_text(html);
        assert_eq!(title.as_deref(), Some("A & B"));
        assert!(text.contains("Head\n"));
        assert!(text.contains("One <two> ét&eacute;"), "{text}");
        assert!(text.contains("Three\nFour"));
        assert!(!text.contains("color") && !text.contains("not text"));
    }

    #[test]
    fn only_real_images_and_plain_language_codes_pass() {
        assert_eq!(image_kind(b"\x89PNG\r\n\x1a\nrest"), Some("png"));
        assert_eq!(image_kind(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpeg"));
        assert_eq!(image_kind(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(image_kind(b"<svg>"), None);
        assert!(lang_ok("eng") && lang_ok("eng+deu") && lang_ok("chi_sim"));
        assert!(!lang_ok("../etc") && !lang_ok("e") && !lang_ok("eng;rm"));
    }

    /// Against a real engine when one is installed: `cargo test -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn tesseract_reads_a_rendered_word() {
        let v = tesseract_version().await.expect("tesseract installed");
        assert!(v.to_ascii_lowercase().starts_with("tesseract"));
        let png =
            std::fs::read(std::env::var("EMEM_OCR_TEST_PNG").expect("EMEM_OCR_TEST_PNG")).unwrap();
        let text = run_tesseract(&png, "eng").await.unwrap();
        assert!(text.to_ascii_lowercase().contains("emem"), "{text}");
    }
}
