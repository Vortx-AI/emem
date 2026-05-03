//! emem CLI entry point. The HTTP/MCP server lives in the `emem-server`
//! binary; this CLI is for protocol introspection, key management, and
//! one-shot fact verification.
//!
//! ```text
//! emem manifests          → dump active manifest CIDs (bands/functions/sources)
//! emem bands              → dump the active band ontology
//! emem functions          → dump the active function registry
//! emem sources            → dump the active source-connector registry
//! emem errors             → dump the stable error code catalog
//! emem keygen             → generate an attester ed25519 keypair (b32-encoded)
//! emem cell <cell64>      → decode a cell64 string back to its u64 representation
//! emem cell-encode <u64>  → encode a u64 cell ID as cell64
//! emem verify <path|->    → offline-verify a receipt's ed25519 signature
//! ```

#![forbid(unsafe_code)]

use std::io::Read;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use rand::RngCore;

#[derive(Parser, Debug)]
#[command(
    name = "emem",
    version,
    about = "emem agent-native spatial memory protocol"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Dump the active manifest CIDs (bands, functions, sources).
    Manifests,
    /// Dump the active band ontology as JSON.
    Bands,
    /// Dump the active function registry as JSON.
    Functions,
    /// Dump the active source-connector registry as JSON.
    Sources,
    /// Dump the stable error code catalog as JSON.
    Errors,
    /// Generate an attester ed25519 keypair (base32-nopad-lowercase).
    Keygen,
    /// Decode a cell64 string to its u64 representation.
    Cell { cell64: String },
    /// Encode a u64 cell ID as cell64.
    CellEncode { raw: u64 },
    /// Offline-verify a receipt's ed25519 signature. Path may be `-` for
    /// stdin. By default the responder pubkey embedded in the receipt is
    /// used; pass `--pubkey` to override, or `--base-url`/`EMEM_BASE_URL`
    /// to fetch the responder pubkey from `/.well-known/emem.json`.
    Verify {
        /// Path to a JSON file holding the receipt object, or `-` for stdin.
        path: String,
        /// Override pubkey (base32-nopad, lowercase, 32 bytes decoded).
        #[arg(long)]
        pubkey: Option<String>,
        /// Server base URL — fetches `/.well-known/emem.json` to discover
        /// the current responder pubkey. Falls back to `EMEM_BASE_URL`.
        #[arg(long)]
        base_url: Option<String>,
    },
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Manifests => {
            let bands_cid = emem_core::manifest::manifest_cid(&*emem_core::bands::DEFAULT)?;
            let functions_cid = emem_core::manifest::manifest_cid(&*emem_core::functions::DEFAULT)?;
            let sources_cid = emem_core::manifest::manifest_cid(&*emem_core::sources::DEFAULT)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "bands_cid": bands_cid,
                    "functions_cid": functions_cid,
                    "sources_cid": sources_cid,
                }))?
            );
        }
        Cmd::Bands => println!(
            "{}",
            serde_json::to_string_pretty(&*emem_core::bands::DEFAULT)?
        ),
        Cmd::Functions => println!(
            "{}",
            serde_json::to_string_pretty(&*emem_core::functions::DEFAULT)?
        ),
        Cmd::Sources => println!(
            "{}",
            serde_json::to_string_pretty(&*emem_core::sources::DEFAULT)?
        ),
        Cmd::Errors => {
            use emem_core::ErrorCode::*;
            let codes = [
                InvalidCell,
                InvalidResolution,
                TslotMismatch,
                BandNotInRegistry,
                FunctionNotInRegistry,
                SourceSchemeUnknown,
                CidNotFound,
                RegistryCidUnknown,
                SchemaCidUnknown,
                PrivacyRefused,
                LevelTooLow,
                AttesterRevoked,
                Unauthorized,
                ClaimUndecidable,
                BadSignature,
                BadMerkleProof,
                CanonicalEncodingDivergence,
                SourceFetchFailed,
                SourceFormatMismatch,
                ComputeTimeout,
                ComputeQuotaExceeded,
                RateLimited,
                CacheError,
                Internal,
            ];
            println!("{}", serde_json::to_string_pretty(&codes)?);
        }
        Cmd::Keygen => {
            let mut sec = [0u8; 32];
            OsRng.fill_bytes(&mut sec);
            let signing = SigningKey::from_bytes(&sec);
            let secret_b32 = data_encoding::BASE32_NOPAD
                .encode(&signing.to_bytes())
                .to_lowercase();
            let pub_b32 = data_encoding::BASE32_NOPAD
                .encode(signing.verifying_key().as_bytes())
                .to_lowercase();
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "secret_b32": secret_b32,
                    "pubkey_b32": pub_b32,
                    "epoch": 0,
                    "alg": "ed25519",
                }))?
            );
        }
        Cmd::Cell { cell64 } => {
            let c = emem_codec::from_cell64(&cell64)
                .map_err(|e| anyhow::anyhow!("decode error: {e}"))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "cell64": cell64,
                    "raw_u64": c.0,
                    "raw_hex": format!("{:#018x}", c.0),
                    "resolution": c.resolution().0,
                    "base_cell": c.base_cell().0,
                }))?
            );
        }
        Cmd::CellEncode { raw } => {
            let c = emem_core::Cell::from_raw(raw);
            println!("{}", emem_codec::to_cell64(c));
        }
        Cmd::Verify {
            path,
            pubkey,
            base_url,
        } => {
            let exit = run_verify(path, pubkey, base_url)?;
            std::process::exit(exit);
        }
    }
    Ok(())
}

/// Read a receipt from `path` (or stdin if `path == "-"`), determine the
/// verifying pubkey (explicit `--pubkey` > `--base-url`/`EMEM_BASE_URL`'s
/// `/.well-known/emem.json` > the receipt's embedded `responder`), and run
/// the same blake3 preimage + ed25519 strict-verify the server uses for
/// `POST /v1/verify_receipt` (`crates/emem-api-rest/src/lib.rs::post_verify_receipt`).
///
/// Returns the process exit code: 0 = valid, 1 = invalid.
fn run_verify(
    path: String,
    pubkey: Option<String>,
    base_url: Option<String>,
) -> anyhow::Result<i32> {
    let raw = if path == "-" {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        buf
    } else {
        std::fs::read_to_string(PathBuf::from(&path))
            .map_err(|e| anyhow::anyhow!("read {path}: {e}"))?
    };
    let receipt: emem_fact::Receipt =
        serde_json::from_str(&raw).map_err(|e| anyhow::anyhow!("parse receipt JSON: {e}"))?;

    // Pubkey resolution: explicit > well-known fetch > embedded.
    let (pk_bytes, pk_source): ([u8; 32], &'static str) = if let Some(b32) = pubkey {
        (decode_pubkey_b32(&b32)?, "flag")
    } else {
        let url = base_url.or_else(|| std::env::var("EMEM_BASE_URL").ok());
        match url {
            Some(u) => {
                let well_known = format!("{}/.well-known/emem.json", u.trim_end_matches('/'));
                let body = fetch_text(&well_known)?;
                let v: serde_json::Value = serde_json::from_str(&body)
                    .map_err(|e| anyhow::anyhow!("parse {well_known}: {e}"))?;
                let b32 = v
                    .pointer("/responder/pubkey_b32")
                    .and_then(|x| x.as_str())
                    .ok_or_else(|| anyhow::anyhow!("{well_known} missing responder.pubkey_b32"))?;
                (decode_pubkey_b32(b32)?, "well-known")
            }
            None => (receipt.responder.0, "receipt.responder"),
        }
    };

    // Reproduce the server-side preimage byte-for-byte. See
    // `crates/emem-storage/src/server.rs::sign_receipt`.
    let mut h = blake3::Hasher::new();
    h.update(receipt.request_id.as_bytes());
    h.update(b"|");
    h.update(receipt.served_at.as_bytes());
    h.update(b"|");
    h.update(receipt.primitive.as_bytes());
    h.update(b"|");
    for c in &receipt.cells {
        h.update(c.as_bytes());
        h.update(b",");
    }
    h.update(b"|");
    for c in &receipt.fact_cids {
        h.update(c.as_str().as_bytes());
        h.update(b",");
    }
    let msg = h.finalize();

    let pk = ed25519_dalek::VerifyingKey::from_bytes(&pk_bytes)
        .map_err(|e| anyhow::anyhow!("bad pubkey bytes: {e}"))?;
    let sig = ed25519_dalek::Signature::from_bytes(&receipt.signature.0);
    let valid = pk.verify_strict(msg.as_bytes(), &sig).is_ok();

    let signer_b32 = data_encoding::BASE32_NOPAD.encode(&pk_bytes).to_lowercase();

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "valid": valid,
            "signer_pubkey_b32": signer_b32,
            "pubkey_source": pk_source,
            "primitive": receipt.primitive,
            "served_at": receipt.served_at,
            "fact_cids_count": receipt.fact_cids.len(),
            "cells_count": receipt.cells.len(),
            "preimage_blake3_hex": msg.to_hex().to_string(),
        }))?
    );

    Ok(if valid { 0 } else { 1 })
}

/// Fetch a URL as text using a one-shot tokio current-thread runtime, so the
/// rest of the CLI stays sync. Workspace `reqwest` has the `blocking` feature
/// disabled, so we drive an async client through a private runtime.
fn fetch_text(url: &str) -> anyhow::Result<String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| anyhow::anyhow!("tokio runtime: {e}"))?;
    rt.block_on(async {
        let resp = reqwest::Client::new()
            .get(url)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("GET {url}: {e}"))?
            .error_for_status()
            .map_err(|e| anyhow::anyhow!("GET {url}: {e}"))?;
        resp.text()
            .await
            .map_err(|e| anyhow::anyhow!("read {url}: {e}"))
    })
}

fn decode_pubkey_b32(b32: &str) -> anyhow::Result<[u8; 32]> {
    let raw = data_encoding::BASE32_NOPAD
        .decode(b32.to_uppercase().as_bytes())
        .map_err(|e| anyhow::anyhow!("pubkey base32 decode: {e}"))?;
    if raw.len() != 32 {
        anyhow::bail!("pubkey must decode to 32 bytes, got {}", raw.len());
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&raw);
    Ok(arr)
}
