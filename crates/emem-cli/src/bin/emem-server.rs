//! emem-server — bind the HTTP/MCP surface on a single port.
//!
//! Defaults: bind 0.0.0.0:5051, hot cache + merkle log under `./var/emem/`,
//! identity ed25519 key generated at startup (printed once for verification).
//!
//! Env knobs:
//!   - `EMEM_BIND` (default `0.0.0.0:5051`)
//!   - `EMEM_DATA` (default `./var/emem`); pass `:memory:` for ephemeral
//!   - `EMEM_SECRET_B32` (optional 32-byte ed25519 secret in base32-nopad)
//!   - When unset, the server reads `<EMEM_DATA>/identity.secret.b32` if
//!     present, else generates a fresh key and persists it (0600). This
//!     keeps the responder pubkey stable across restarts so receipts
//!     verify long-term.
//!   - `EMEM_TLS_DOMAINS` (comma-separated, e.g. `emem.dev,www.emem.dev`)
//!     — when set, the server listens on the TLS bind (default `0.0.0.0:443`)
//!     and obtains a Let's Encrypt cert via TLS-ALPN-01. Only port 443 is
//!     needed; no Cloudflare, no Caddy, no nginx.
//!   - `EMEM_TLS_BIND` (default `0.0.0.0:443`) — TLS bind address.
//!   - `EMEM_TLS_CONTACT` (default `mailto:avijeet@vortx.ai`) — ACME contact.
//!   - `EMEM_TLS_STAGING=1` — use Let's Encrypt staging directory (rate-limit
//!     friendly while testing the deploy path).

use std::sync::Arc;

use emem_api_rest::default_manifest_cids;
use emem_core::manifest::manifest_cid;
use emem_fact::{RegistryCid, SchemaCid};
use emem_storage::server::ManifestCids;
use emem_storage::{server::ResponderIdentity, MaterializingStorage, Server};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let bind = std::env::var("EMEM_BIND").unwrap_or_else(|_| "0.0.0.0:5051".into());
    let data = std::env::var("EMEM_DATA").unwrap_or_else(|_| "./var/emem".into());

    let bands = Arc::new((*emem_core::bands::DEFAULT).clone());
    let functions = Arc::new((*emem_core::functions::DEFAULT).clone());
    let sources = Arc::new((*emem_core::sources::DEFAULT).clone());

    let storage = if data == ":memory:" {
        tracing::info!("opening ephemeral storage");
        MaterializingStorage::ephemeral(bands.clone(), functions.clone(), sources.clone())?
    } else {
        tracing::info!(%data, "opening persistent storage");
        MaterializingStorage::rooted(&data, bands.clone(), functions.clone(), sources.clone())?
    };

    let functions_cid = manifest_cid(&*functions).unwrap_or_default();
    let schema_cid = manifest_cid(&*emem_core::schema::DEFAULT).unwrap_or_default();
    let (bands_cid, sources_cid) = default_manifest_cids();

    let identity = load_or_create_identity(&data)?;

    tracing::info!(
        responder_pubkey_b32 = %data_encoding::BASE32_NOPAD.encode(&identity.pubkey.0).to_lowercase(),
        responder_key_epoch = identity.epoch.0,
        "responder identity"
    );

    let started_at_unix_s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let server = Arc::new(Server {
        storage: Arc::new(storage),
        identity,
        manifests: ManifestCids {
            registry_cid: RegistryCid::new(functions_cid),
            schema_cid: SchemaCid::new(schema_cid),
            bands_cid,
            sources_cid,
        },
        started_at_unix_s,
    });

    let app = emem_api_rest::router(server);

    eprintln!("  GET  /health");
    eprintln!("  GET  /openapi.json");
    eprintln!("  GET  /.well-known/emem.json");
    eprintln!("  POST /v1/recall, /v1/compare, /v1/find_similar, /v1/diff, ...");
    eprintln!("  POST /mcp  (MCP JSON-RPC 2.0)");

    let tls_domains = std::env::var("EMEM_TLS_DOMAINS")
        .ok()
        .map(|s| {
            s.split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if tls_domains.is_empty() {
        // Plain HTTP path (the default; behind a reverse proxy in production).
        let listener = tokio::net::TcpListener::bind(&bind).await?;
        tracing::info!(%bind, "emem listening (plain HTTP)");
        eprintln!("emem listening on http://{bind}");
        let shutdown = shutdown_signal();
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown)
            .await?;
    } else {
        // Native TLS via rustls + Let's Encrypt (TLS-ALPN-01). Only :443 needed.
        let tls_bind: std::net::SocketAddr = std::env::var("EMEM_TLS_BIND")
            .unwrap_or_else(|_| "0.0.0.0:443".into())
            .parse()
            .map_err(|e| anyhow::anyhow!("EMEM_TLS_BIND parse failed: {e}"))?;
        let contact =
            std::env::var("EMEM_TLS_CONTACT").unwrap_or_else(|_| "mailto:avijeet@vortx.ai".into());
        let staging = std::env::var("EMEM_TLS_STAGING").ok().as_deref() == Some("1");
        let cache_dir = std::path::Path::new(&data).join("acme.cache");
        std::fs::create_dir_all(&cache_dir).ok();

        tracing::info!(?tls_domains, %tls_bind, %contact, staging, cache=%cache_dir.display(),
            "emem listening (HTTPS, ACME via TLS-ALPN-01)");
        eprintln!(
            "emem listening on https://{tls_bind} for {:?}  (staging={})",
            tls_domains, staging
        );

        use futures_util::StreamExt;
        use rustls_acme::axum::AxumAcceptor;
        use rustls_acme::caches::DirCache;
        use rustls_acme::AcmeConfig;

        let mut state = AcmeConfig::new(tls_domains.clone())
            .contact_push(contact)
            .cache(DirCache::new(cache_dir))
            .directory_lets_encrypt(!staging)
            .state();
        let rustls_cfg = state.default_rustls_config();
        let acceptor: AxumAcceptor = state.axum_acceptor(rustls_cfg);

        // Background ACME event drainer: must be polled for the cert flow to
        // make progress. Logs ok / err per renewal.
        tokio::spawn(async move {
            while let Some(ev) = state.next().await {
                match ev {
                    Ok(ok) => tracing::info!(?ok, "acme event"),
                    Err(e) => tracing::error!(error = %e, "acme error"),
                }
            }
        });

        // Optional plain HTTP listener kept up so existing 5051 callers (MCP
        // clients on local nets, the live-demo binary) keep working.
        let app_for_http = app.clone();
        if !bind.is_empty() {
            tokio::spawn(async move {
                if let Ok(listener) = tokio::net::TcpListener::bind(&bind).await {
                    tracing::info!(%bind, "emem also listening on plain HTTP for local agents");
                    let _ = axum::serve(listener, app_for_http)
                        .with_graceful_shutdown(shutdown_signal())
                        .await;
                }
            });
        }

        // TLS server. axum-server handles graceful shutdown via tokio signals.
        axum_server::bind(tls_bind)
            .acceptor(acceptor)
            .serve(app.into_make_service())
            .await?;
    }
    Ok(())
}

async fn shutdown_signal() {
    use tokio::signal;
    let ctrl_c = async {
        let _ = signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = signal::unix::signal(signal::unix::SignalKind::terminate()) {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => tracing::info!("ctrl_c received — draining"),
        _ = term   => tracing::info!("SIGTERM received — draining"),
    }
}

fn load_or_create_identity(data_dir: &str) -> anyhow::Result<ResponderIdentity> {
    if let Ok(s) = std::env::var("EMEM_SECRET_B32") {
        return decode_secret(&s);
    }
    if data_dir == ":memory:" {
        return Ok(ResponderIdentity::fresh());
    }
    let id_path = std::path::Path::new(data_dir).join("identity.secret.b32");
    if id_path.exists() {
        let s = std::fs::read_to_string(&id_path)?.trim().to_string();
        let id = decode_secret(&s)?;
        tracing::info!(path = %id_path.display(), "loaded persisted identity");
        return Ok(id);
    }
    let id = ResponderIdentity::fresh();
    std::fs::create_dir_all(data_dir).ok();
    let secret_b32 = id.export_secret_b32();
    std::fs::write(&id_path, &secret_b32)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = std::fs::metadata(&id_path)?.permissions();
        perm.set_mode(0o600);
        std::fs::set_permissions(&id_path, perm)?;
    }
    tracing::info!(path = %id_path.display(), "generated and persisted new identity");
    Ok(id)
}

fn decode_secret(s: &str) -> anyhow::Result<ResponderIdentity> {
    let bytes = data_encoding::BASE32_NOPAD
        .decode(s.trim().to_uppercase().as_bytes())
        .map_err(|e| anyhow::anyhow!("ed25519 secret must be base32-nopad: {e}"))?;
    if bytes.len() != 32 {
        anyhow::bail!(
            "ed25519 secret must decode to 32 bytes, got {}",
            bytes.len()
        );
    }
    let mut sec = [0u8; 32];
    sec.copy_from_slice(&bytes);
    Ok(ResponderIdentity::from_secret(sec, 0))
}
