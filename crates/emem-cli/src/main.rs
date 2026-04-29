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
//! ```

#![forbid(unsafe_code)]

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
    }
    Ok(())
}
