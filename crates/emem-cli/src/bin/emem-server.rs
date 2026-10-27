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
//!   - `EMEM_TLS_CONTACT` (NO DEFAULT, required when TLS is on) — ACME
//!     contact. Must be the operator's own address: it is the account of
//!     record for every certificate this node registers.
//!   - `EMEM_CONTACT` (optional) — contact placed in the outbound
//!     User-Agent sent to data providers; defaults to the project URL.
//!   - `EMEM_TLS_STAGING=1` — use Let's Encrypt staging directory (rate-limit
//!     friendly while testing the deploy path).

use std::sync::Arc;

use emem_api_rest::default_manifest_cids;
use emem_core::manifest::manifest_cid;
use emem_fact::{RegistryCid, SchemaCid};
use emem_storage::server::ManifestCids;
use emem_storage::{server::ResponderIdentity, MaterializingStorage, Server};

// See the dependency's note in Cargo.toml: glibc kept freed arena heaps
// resident, and the responder's RSS climbed until the page cache starved.
#[cfg(target_os = "linux")]
#[global_allocator]
static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

/// One line every ten minutes: what the program holds live (`allocated`)
/// against what the kernel is charging for it (`resident`). A gap that grows
/// is fragmentation; an `allocated` that grows is a leak.
#[cfg(target_os = "linux")]
fn log_allocator_stats() {
    tokio::spawn(async {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(600));
        loop {
            tick.tick().await;
            use tikv_jemalloc_ctl::{epoch, stats};
            if epoch::advance().is_err() {
                continue;
            }
            let mib = |r: Result<usize, tikv_jemalloc_ctl::Error>| r.map(|b| b >> 20).unwrap_or(0);
            tracing::info!(
                target: "emem::alloc",
                allocated_mib = mib(stats::allocated::read()),
                active_mib = mib(stats::active::read()),
                resident_mib = mib(stats::resident::read()),
                retained_mib = mib(stats::retained::read()),
                "allocator"
            );
        }
    });
}

/// Keep whole local copies of the static forest rasters under
/// `<EMEM_DATA>/tile_store`, up to `EMEM_TILE_STORE_GB` (default 40; 0 turns
/// the store off). See `emem_fetch::tile_store`.
fn enable_tile_store(data: &str) {
    let gb: u64 = std::env::var("EMEM_TILE_STORE_GB")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(40);
    if gb == 0 {
        return;
    }
    let dir = std::path::Path::new(data).join("tile_store");
    match emem_fetch::tile_store::enable(dir.clone(), gb << 30) {
        Ok(n) => {
            tracing::info!(target: "emem::tile_store", dir = %dir.display(), budget_gb = gb, copies = n, "tile store on")
        }
        Err(e) => {
            tracing::warn!(target: "emem::tile_store", dir = %dir.display(), error = %e, "tile store off: its directory is not usable")
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    #[cfg(target_os = "linux")]
    log_allocator_stats();

    // Wall clock from the top of `main`. Every step below reports its own
    // duration against it, because "startup is slow" is not actionable and
    // "storage open 1.9 s, topic router 7.6 s" is. `boot_step` is one line
    // per phase on stderr, so the breakdown is readable straight out of
    // `journalctl` without turning on a debug filter.
    let boot = std::time::Instant::now();
    let boot_step = |name: &str, t: std::time::Instant| {
        let ms = t.elapsed().as_millis() as u64;
        tracing::info!(target: "emem::boot", step = name, elapsed_ms = ms, since_start_ms = boot.elapsed().as_millis() as u64, "boot step");
        eprintln!("boot: {name} {ms} ms (t+{} ms)", boot.elapsed().as_millis());
    };

    // `bind` is the plain-HTTP listener. When TLS is also active (see
    // EMEM_TLS_DOMAINS below) we default the plain listener to
    // 127.0.0.1:5051 instead of 0.0.0.0:5051 so writes (/v1/attest,
    // /v1/attest_cbor) and signed receipts never traverse the public
    // interface in cleartext. An operator who genuinely wants the plain
    // listener exposed (e.g. on a private VLAN) still sets EMEM_BIND
    // explicitly. With no TLS, the historical 0.0.0.0:5051 default
    // applies — that's the "plain HTTP behind a reverse proxy" path.
    let bind_explicit = std::env::var("EMEM_BIND").ok();
    let data = emem_core::data_dir::data_dir()
        .to_string_lossy()
        .into_owned();
    if data != ":memory:" {
        enable_tile_store(&data);
    }

    // The topic router is started FIRST and awaited last.
    //
    // It has to be loaded before the listener binds (see the await below for
    // why), and it is the single longest step in startup. It also depends on
    // nothing else here: it reads its model straight off disk under
    // EMEM_TOPIC_MODEL_DIR / EMEM_DATA and never touches storage, the
    // identity or the manifests. Kicking it off now runs it against the
    // blocking pool while the main thread opens storage and hashes the
    // manifests, so the two costs overlap instead of adding. Before this the
    // model load did not begin until storage had finished, and startup was
    // the sum of both.
    let warmup_started = std::time::Instant::now();
    let warmup = tokio::task::spawn_blocking(|| {
        let t0 = std::time::Instant::now();
        let r = emem_api_rest::topic_router::TopicRouter::global();
        let backend = r.backend_name();
        tracing::info!(
            target: "emem::topic_router",
            backend = backend,
            elapsed_ms = t0.elapsed().as_millis() as u64,
            "topic router warmup complete"
        );
        backend
    });

    let t = std::time::Instant::now();
    let bands = Arc::new((*emem_core::bands::DEFAULT).clone());
    let functions = Arc::new((*emem_core::functions::DEFAULT).clone());
    let sources = Arc::new((*emem_core::sources::DEFAULT).clone());
    boot_step("registries", t);

    let t = std::time::Instant::now();
    let storage = if data == ":memory:" {
        tracing::info!("opening ephemeral storage");
        MaterializingStorage::ephemeral(bands.clone(), functions.clone(), sources.clone())?
    } else {
        tracing::info!(%data, "opening persistent storage");
        let _heartbeat = storage_open_heartbeat(t);
        MaterializingStorage::rooted(&data, bands.clone(), functions.clone(), sources.clone())?
    };
    boot_step("storage_open", t);

    let t = std::time::Instant::now();
    let functions_cid = manifest_cid(&*functions).unwrap_or_default();
    let schema_cid = manifest_cid(&*emem_core::schema::DEFAULT).unwrap_or_default();
    let (bands_cid, sources_cid) = default_manifest_cids();
    boot_step("manifest_cids", t);

    let t = std::time::Instant::now();
    let identity = load_or_create_identity(&data)?;
    boot_step("identity", t);

    tracing::info!(
        responder_pubkey_b32 = %data_encoding::BASE32_NOPAD.encode(&identity.pubkey.0).to_lowercase(),
        responder_key_epoch = identity.epoch.0,
        "responder identity"
    );

    // Optional NASA OPERA DIST-ALERT (near-real-time disturbance). The
    // Earthdata Login token is read from the environment at request time by
    // the materializer; here we just log whether it is provisioned so the
    // operator can confirm the optional NRT feature is on/off. The token
    // value is NEVER logged — only its presence. When unset, the
    // opera_dist.* bands sign an honest Absence (no regression).
    if emem_fetch::opera_dist::is_enabled() {
        tracing::info!(
            "OPERA DIST-ALERT enabled (Earthdata token provisioned); opera_dist.* bands will fetch live NRT disturbance"
        );
    } else {
        tracing::info!(
            "OPERA DIST-ALERT disabled (no EMEM_EARTHDATA_TOKEN / EMEM_EARTHDATA_TOKEN_FILE); opera_dist.* bands sign honest Absence"
        );
    }

    let started_at_unix_s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // The fact plane is closed by default; only now, with the identity loaded,
    // is the responder's own key known, so the policy is installed here. The
    // log line is the operator's evidence of what this node admits.
    let fact_plane = emem_storage::FactPlanePolicy::for_responder(identity.pubkey.0);
    tracing::info!(policy = %fact_plane.describe(), "fact plane");
    if fact_plane.open {
        tracing::warn!(
            "EMEM_FACT_PLANE_OPEN=1: any signing key may occupy an address on this node"
        );
    }
    storage.set_fact_plane_policy(fact_plane);

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

    // Kept so the shutdown path can flush the sled index after the listener
    // has stopped; `router` takes the Arc.
    let server_for_shutdown = server.clone();
    let t = std::time::Instant::now();
    let app = emem_api_rest::router(server);
    boot_step("router_build", t);

    // Wait for the topic router (started at the top of `main`) BEFORE we
    // start accepting requests. The router uses a `OnceLock` that
    // synchronously runs the model load inside whichever thread first
    // calls `global()` — when that thread is an axum handler and the
    // load takes >RST/keepalive timeouts (or hangs on CUDA EP init),
    // every subsequent request that touches the router is wedged for
    // the rest of the process lifetime. Loading it on the dedicated
    // blocking thread pool with a hard 90-second timeout keeps that path
    // off the live request handlers — if it hangs or exceeds the budget
    // we abort, log, and let the in-process fallback (`Backend::Keyword`)
    // take over for `/v1/ask` until the operator investigates.
    //
    // It is still awaited before the bind rather than left running behind
    // an open port, and that is deliberate. `global()` is not reached only
    // by `/v1/ask`: `GET /v1/agent_card` calls `backend_name()` to publish
    // `manifests.topic_router_backend`, so a listener opened early would
    // move the same wait onto a descriptor read, and any request that did
    // touch the router would park a runtime worker on a `OnceLock` for the
    // remainder of the load. Overlapping it with storage open takes the
    // same time off startup without opening a port in front of a responder
    // that cannot yet answer.
    {
        match tokio::time::timeout(std::time::Duration::from_secs(90), warmup).await {
            Ok(Ok(backend)) => {
                eprintln!("topic router ready ({backend})");
            }
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "topic router warmup task panicked");
                eprintln!("topic router warmup panicked: {e}");
            }
            Err(_) => {
                tracing::warn!(
                    "topic router warmup exceeded 90s — server will start, /v1/ask may stall on first call until OnceLock initializer returns"
                );
                eprintln!("warning: topic router warmup exceeded 90s; continuing");
            }
        }
        boot_step("topic_router_warmup", warmup_started);
    }

    // Wire the sled-backed persistent cache for Overture division-
    // polygon lookups before kicking off warm-up. Once installed the
    // lookup path consults sled before scanning parquet and writes
    // back on miss, so a process restart doesn't re-pay the 6–15 s
    // cold-cache cost per previously-seen place. Idempotent; safe to
    // skip on sled failure (the in-memory cache still works).
    let t = std::time::Instant::now();
    emem_api_rest::wire_overture_persistent_cache();
    boot_step("overture_cache_wire", t);

    // NOTE: JRC GFC2020 no longer needs a boot-time COG pre-warm. The
    // connector now reads 10°×10° tiles (jrc_gfc2020::tile_url_for) whose
    // IFD opens in ~1 s cold, instead of the 41 GB single-COG whose
    // ~hundreds-of-MB tile index could not finish inside the request
    // budget — the wedge that took /v1/eudr_dds down on 2026-06-02. Tiles
    // warm on-demand and per-region via cog::PROFILE_CACHE / TILE_CACHE,
    // exactly like Hansen GFC. No global pre-warm is possible (the tile
    // depends on the requested region) or needed.

    // Pre-warm the Overture divisions cache so the first /v1/locate
    // (and every place-based boring endpoint behind it) doesn't pay
    // the 6–15 s cold-cache penalty for S3 list + per-shard parquet
    // footer fetch. Runs in the background so server start isn't
    // blocked on a slow S3 round-trip; locate requests that race the
    // warm-up just take the cold path themselves once. Disable with
    // `EMEM_OVERTURE_SKIP_WARMUP=1` for offline / air-gapped runs.
    if std::env::var("EMEM_OVERTURE_SKIP_WARMUP").ok().as_deref() != Some("1") {
        tokio::spawn(async {
            let t0 = std::time::Instant::now();
            match emem_fetch::overture::OvertureClient::shared()
                .warm_start()
                .await
            {
                Ok(stats) => {
                    tracing::info!(
                        target: "emem::overture::warm",
                        overture_warm_files = stats.file_count,
                        overture_warm_footer_errors = stats.footer_errors,
                        overture_warm_elapsed_ms = stats.elapsed_ms,
                        "overture warm-up complete"
                    );
                    eprintln!(
                        "overture warm-up: {} shards, {} footer errors, {} ms",
                        stats.file_count, stats.footer_errors, stats.elapsed_ms
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        target: "emem::overture::warm",
                        overture_warm_elapsed_ms = t0.elapsed().as_millis() as u64,
                        error = %e,
                        "overture warm-up failed; first locate call will pay cold cost"
                    );
                    eprintln!("warning: overture warm-up failed ({e}); first locate will be slow");
                }
            }
        });
    }

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
        let bind = bind_explicit.unwrap_or_else(|| "0.0.0.0:5051".into());
        let listener = tokio::net::TcpListener::bind(&bind).await?;
        tracing::info!(%bind, boot_ms = boot.elapsed().as_millis() as u64, "emem listening (plain HTTP)");
        eprintln!(
            "emem listening on http://{bind}  (boot {} ms)",
            boot.elapsed().as_millis()
        );
        serve_plain(listener, app, REQUEST_START_DEADLINE).await?;
        flush_and_exit(&server_for_shutdown).await
    } else {
        // Native TLS via rustls + Let's Encrypt (TLS-ALPN-01). Only :443 needed.
        let tls_bind: std::net::SocketAddr = std::env::var("EMEM_TLS_BIND")
            .unwrap_or_else(|_| "0.0.0.0:443".into())
            .parse()
            .map_err(|e| anyhow::anyhow!("EMEM_TLS_BIND parse failed: {e}"))?;
        // ACME registration contact. NO DEFAULT, deliberately.
        //
        // This used to fall back to a hardcoded personal address, in a public
        // repository. Anyone enabling TLS without setting the variable would
        // have registered Let's Encrypt certificates for THEIR domains against
        // A STRANGER'S EMAIL -- who would then receive the expiry notices and
        // the revocation mail for hosts they have never heard of, and would be
        // the account of record for certificates they do not control.
        //
        // Refusing to start is the correct failure. A missing contact is a
        // question only the operator can answer, and substituting someone
        // else's answer is worse than any outage: the outage is visible at
        // once and this would not be visible for ninety days.
        let contact = std::env::var("EMEM_TLS_CONTACT").map_err(|_| {
            anyhow::anyhow!(
                "EMEM_TLS_CONTACT is not set. ACME registers certificates against \
                 a contact address, so this must be YOUR address -- set it to \
                 e.g. `mailto:you@example.org`. Set EMEM_TLS_DOMAINS=\"\" to \
                 serve plain HTTP instead."
            )
        })?;
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

        // Plain HTTP listener — kept up so the live-demo binary and local
        // MCP clients keep working without going through TLS. When TLS is
        // active and the operator hasn't explicitly set EMEM_BIND we
        // default to 127.0.0.1:5051: signed writes (/v1/attest,
        // /v1/attest_cbor) and the responder's identity must never be
        // reachable in cleartext over the public interface. An operator
        // who wants public plain HTTP (e.g. on a private VLAN) sets
        // EMEM_BIND=0.0.0.0:5051 explicitly.
        let plain_bind = bind_explicit
            .clone()
            .unwrap_or_else(|| "127.0.0.1:5051".into());
        if !plain_bind.is_empty() {
            let app_for_http = app.clone();
            tokio::spawn(async move {
                if let Ok(listener) = tokio::net::TcpListener::bind(&plain_bind).await {
                    tracing::info!(bind=%plain_bind, "emem also listening on plain HTTP (loopback unless EMEM_BIND set)");
                    if let Err(e) =
                        serve_plain(listener, app_for_http, REQUEST_START_DEADLINE).await
                    {
                        tracing::warn!(bind=%plain_bind, error=%e, "plain HTTP listener stopped");
                    }
                }
            });
        }

        // TLS server.
        //
        // The comment that used to sit here said axum-server handled graceful
        // shutdown via tokio signals. It does not: without a `Handle` the
        // serve future has nothing to complete on, so SIGTERM was simply never
        // acted upon and `main` never returned. systemd waited out
        // TimeoutStopSec and SIGKILLed — measured on this host as 90 s of
        // refused connections on every single deploy, with
        // `State 'stop-sigterm' timed out. Killing.` in the journal each time.
        // A `Handle` is what turns the signal into "stop accepting, give
        // in-flight work `grace`, then close what is left".
        //
        // `into_make_service_with_connect_info` is required so the
        // rate-limit middleware can read the peer SocketAddr — without
        // it, every anonymous request collapses into one shared
        // "unknown" bucket and the per-IP limit becomes a global limit.
        let handle = axum_server::Handle::new();
        drain_on_signal(handle.clone());
        tracing::info!(
            target: "emem::boot",
            boot_ms = boot.elapsed().as_millis() as u64,
            "binding TLS listener"
        );
        eprintln!("boot: binding :443 at t+{} ms", boot.elapsed().as_millis());
        let mut tls_server = axum_server::bind(tls_bind)
            .acceptor(ConnectionDeadlines::new(acceptor))
            .handle(handle);
        idle_connection_timers(tls_server.http_builder(), REQUEST_START_DEADLINE);
        tls_server
            .serve(app.into_make_service_with_connect_info::<std::net::SocketAddr>())
            .await?;
        flush_and_exit(&server_for_shutdown).await
    }
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
        _ = ctrl_c => tracing::info!("ctrl_c received, draining"),
        _ = term   => tracing::info!("SIGTERM received, draining"),
    }
}

/// How long in-flight requests get to finish after the listener stops
/// accepting, before their connections are closed underneath them.
///
/// Three seconds because the drain only has to cover a request already being
/// served: the boring endpoints answer in milliseconds and the long ones
/// (materialize, polygon fan-out) are the very requests that must NOT be
/// allowed to hold a deploy open. Long-lived streams (`/v1/memory/sse`,
/// `/v1/stream`) never end on their own, so without a deadline "graceful"
/// means "never".
fn shutdown_grace() -> std::time::Duration {
    let s = std::env::var("EMEM_SHUTDOWN_GRACE_S")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(3)
        .min(30);
    std::time::Duration::from_secs(s)
}

/// Flush the sled index, close redb, then leave.
///
/// `std::process::exit` skips destructors, so sled's own drop-time flush never
/// runs and anything still only in its in-memory log would be lost. This
/// flushes explicitly first, under its own timeout so a wedged flush cannot
/// reintroduce the hang this whole path exists to remove. sled fsyncs the
/// whole `Db`, so one call covers every tree.
///
/// redb needs the same treatment and did not get it when it was added. It
/// writes its "shut down cleanly" header in `Database::drop`, which this
/// function skips, so EVERY boot since the redb cutover opened a database
/// still marked `recovery_required` and rebuilt the allocator state by walking
/// the file. Measured 2026-09-14: sled opened in 29,357 ms on 43.8 GB while
/// the redb open ran for tens of minutes on 53.7 GB, and the whole cost had
/// been attributed to sled page-faulting.
///
/// Exiting explicitly rather than returning from `main` is deliberate: the
/// runtime's drop waits for blocking-pool tasks to finish, and this process
/// keeps long sled scans and COG range-reads on that pool.
async fn flush_and_exit(server: &Arc<Server>) -> ! {
    let t0 = std::time::Instant::now();
    // Citation counts are coalesced in memory between 30 s writes.
    if let Some(reg) = server.storage.attesters() {
        if let Err(e) = reg.flush_citations() {
            tracing::error!(error = %e, "citation flush failed");
        }
    }
    if let Some(db) = server.storage.hot_sled_db() {
        let db = db.clone();
        match tokio::time::timeout(std::time::Duration::from_secs(10), db.flush_async()).await {
            Ok(Ok(bytes)) => tracing::info!(
                flushed_bytes = bytes,
                flush_ms = t0.elapsed().as_millis() as u64,
                "sled flushed"
            ),
            Ok(Err(e)) => tracing::error!(error = %e, "sled flush failed"),
            Err(_) => tracing::error!("sled flush did not finish in 10s; exiting anyway"),
        }
    }
    if let Some(r) = server.storage.redb() {
        // Bounded by what is LEFT of the container's stop grace, not by a
        // constant. `ExecStop=docker stop -t 60` SIGKILLs 60 s after SIGTERM,
        // and a kill inside the close is worse than not waiting at all: the
        // deferred close never runs, the file stays marked for recovery, and
        // the next open walks it (40 minutes for 56 GB on 2026-09-15). The
        // drain and the sled flush have already spent part of the budget, and
        // on that incident `close()` itself blocked 37 s on the db lock behind
        // a write transaction that was stuck on a wedged disk.
        let stop_grace = std::time::Duration::from_secs(
            std::env::var("EMEM_STOP_GRACE_S")
                .ok()
                .and_then(|v| v.trim().parse::<u64>().ok())
                .unwrap_or(60)
                .clamp(5, 3600),
        );
        let margin = std::time::Duration::from_secs(5);
        let spent = shutdown_grace() + t0.elapsed();
        let left = stop_grace.saturating_sub(spent + margin);
        let budget = left.min(emem_cache::RedbFacts::close_wait_default());
        tracing::info!(
            close_budget_ms = budget.as_millis() as u64,
            spent_ms = spent.as_millis() as u64,
            "closing redb"
        );
        r.close_with_budget(budget);
    }
    eprintln!("emem: shutdown complete in {} ms", t0.elapsed().as_millis());
    std::process::exit(0);
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
    // Write the secret atomically with 0600 from the start. The previous
    // "fs::write then chmod 0600" pattern opened a brief window where any
    // process on the host could read the 0644-mode file before the chmod
    // landed — a co-tenant could exfiltrate the responder signing key.
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&id_path)?;
        f.write_all(secret_b32.as_bytes())?;
        f.sync_all().ok();
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&id_path, &secret_b32)?;
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

/// The watchdog does not count a missed `/live` while `emem::boot` lines are
/// still arriving, but the storage open is one long call that logged nothing:
/// 286 s of silence on 2026-10-22, counted as two misses of a healthy boot.
/// That silence is why the watchdog threshold had to cover a whole cold boot.
/// Logs every 30 s until the returned sender is dropped, i.e. the open returns.
fn storage_open_heartbeat(started: std::time::Instant) -> std::sync::mpsc::Sender<()> {
    let (done, _thread) = heartbeat_every(std::time::Duration::from_secs(30), move || {
        tracing::info!(
            target: "emem::boot",
            elapsed_ms = started.elapsed().as_millis() as u64,
            "still opening persistent storage"
        );
    });
    done
}

/// Runs `tick` every `every` on its own thread until the returned sender is
/// dropped; the thread then exits.
fn heartbeat_every(
    every: std::time::Duration,
    tick: impl Fn() + Send + 'static,
) -> (std::sync::mpsc::Sender<()>, std::thread::JoinHandle<()>) {
    let (done, wait) = std::sync::mpsc::channel::<()>();
    let thread = std::thread::spawn(move || {
        while let Err(std::sync::mpsc::RecvTimeoutError::Timeout) = wait.recv_timeout(every) {
            tick();
        }
    });
    (done, thread)
}

/// On SIGTERM or ctrl-c: stop accepting, give open connections
/// `shutdown_grace()`, then close what is left. A keep-alive SSE stream never
/// ends on its own, so a drain with no deadline would never finish.
fn drain_on_signal(handle: axum_server::Handle) {
    let grace = shutdown_grace();
    tokio::spawn(async move {
        shutdown_signal().await;
        tracing::info!(
            grace_s = grace.as_secs(),
            "no longer accepting connections, draining"
        );
        eprintln!("emem: draining, {}s deadline", grace.as_secs());
        handle.graceful_shutdown(Some(grace));
    });
}

/// Plain HTTP, with the same connection deadlines and drain as the TLS
/// listener. `axum::serve` exposes no hyper builder, so it could not close
/// an idle connection either.
async fn serve_plain(
    listener: tokio::net::TcpListener,
    app: axum::Router,
    request_start: std::time::Duration,
) -> std::io::Result<()> {
    let handle = axum_server::Handle::new();
    drain_on_signal(handle.clone());
    let mut server = axum_server::from_tcp(listener.into_std()?)
        .acceptor(ConnectionDeadlines {
            inner: axum_server::accept::DefaultAcceptor,
            handshake: TLS_HANDSHAKE_DEADLINE,
            request_start,
        })
        .handle(handle);
    idle_connection_timers(server.http_builder(), request_start);
    server
        .serve(app.into_make_service_with_connect_info::<std::net::SocketAddr>())
        .await
}

/// How long a client gets to finish the TLS handshake.
const TLS_HANDSHAKE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);
/// How long a connection may stay silent after the handshake before its
/// first request byte, and between requests on an HTTP/1 keep-alive.
const REQUEST_START_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);

/// Neither rustls-acme's acceptor nor axum-server's builder closes a
/// connection that goes quiet: no handshake deadline, no timer for hyper's
/// header-read timeout, and none on the read that tells HTTP/1 from HTTP/2.
/// Clients that open a connection and walk away left one socket each until
/// the process hit its file limit (2,403 "Too many open files" on
/// 2026-10-22, 158 sockets from one address idle for over 6 h). These
/// deadlines cover only waiting for a request to start, so a slow request
/// already running (an EUDR run, an SSE stream) is never cut off.
fn idle_connection_timers(
    builder: &mut hyper_util::server::conn::auto::Builder<hyper_util::rt::TokioExecutor>,
    request_start: std::time::Duration,
) {
    builder
        .http1()
        .timer(hyper_util::rt::TokioTimer::new())
        .header_read_timeout(request_start);
    builder
        .http2()
        .timer(hyper_util::rt::TokioTimer::new())
        .keep_alive_interval(std::time::Duration::from_secs(60))
        .keep_alive_timeout(std::time::Duration::from_secs(20));
}

#[derive(Clone)]
struct ConnectionDeadlines<A> {
    inner: A,
    handshake: std::time::Duration,
    request_start: std::time::Duration,
}

impl<A> ConnectionDeadlines<A> {
    fn new(inner: A) -> Self {
        Self {
            inner,
            handshake: TLS_HANDSHAKE_DEADLINE,
            request_start: REQUEST_START_DEADLINE,
        }
    }
}

impl<A, I, S> axum_server::accept::Accept<I, S> for ConnectionDeadlines<A>
where
    A: axum_server::accept::Accept<I, S>,
    A::Future: Send + 'static,
    A::Stream: Send + 'static,
    A::Service: Send + 'static,
{
    type Stream = FirstByteDeadline<A::Stream>;
    type Service = A::Service;
    type Future = std::pin::Pin<
        Box<
            dyn std::future::Future<Output = std::io::Result<(Self::Stream, Self::Service)>> + Send,
        >,
    >;

    fn accept(&self, stream: I, service: S) -> Self::Future {
        let handshake = self.inner.accept(stream, service);
        let (limit, request_start) = (self.handshake, self.request_start);
        Box::pin(async move {
            let (stream, service) =
                tokio::time::timeout(limit, handshake).await.map_err(|_| {
                    std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "TLS handshake did not finish in time",
                    )
                })??;
            Ok((FirstByteDeadline::new(stream, request_start), service))
        })
    }
}

/// Fails the read with `TimedOut` if no byte arrives before `after`; after
/// the first byte it only passes through.
struct FirstByteDeadline<T> {
    io: T,
    deadline: Option<std::pin::Pin<Box<tokio::time::Sleep>>>,
}

impl<T> FirstByteDeadline<T> {
    fn new(io: T, after: std::time::Duration) -> Self {
        Self {
            io,
            deadline: Some(Box::pin(tokio::time::sleep(after))),
        }
    }
}

impl<T: tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for FirstByteDeadline<T> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        use std::future::Future;
        use std::task::Poll;
        let this = &mut *self;
        let before = buf.filled().len();
        match std::pin::Pin::new(&mut this.io).poll_read(cx, buf) {
            Poll::Ready(result) => {
                if buf.filled().len() > before {
                    this.deadline = None;
                }
                Poll::Ready(result)
            }
            Poll::Pending => {
                let expired = this
                    .deadline
                    .as_mut()
                    .is_some_and(|deadline| deadline.as_mut().poll(cx).is_ready());
                if expired {
                    Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "no request arrived on the connection in time",
                    )))
                } else {
                    Poll::Pending
                }
            }
        }
    }
}

impl<T: tokio::io::AsyncWrite + Unpin> tokio::io::AsyncWrite for FirstByteDeadline<T> {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.io).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.io).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.io.is_write_vectored()
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.io).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.io).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod heartbeat_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// The watchdog skips a missed /live only while boot lines keep coming,
    /// so the heartbeat must tick for as long as the open runs and stop the
    /// moment it returns. Timed windows made this flaky on a loaded macOS
    /// runner, so it waits for ticks and for the thread to exit instead.
    #[test]
    fn the_heartbeat_ticks_until_its_sender_drops() {
        let ticks = Arc::new(AtomicUsize::new(0));
        let seen = ticks.clone();
        let (done, thread) = heartbeat_every(Duration::from_millis(5), move || {
            seen.fetch_add(1, Ordering::SeqCst);
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while ticks.load(Ordering::SeqCst) < 3 {
            assert!(
                std::time::Instant::now() < deadline,
                "no 3 ticks within 10 s"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        drop(done);
        let (exited, wait) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = thread.join();
            let _ = exited.send(());
        });
        assert!(
            wait.recv_timeout(Duration::from_secs(10)).is_ok(),
            "the heartbeat thread kept running after the open returned"
        );
    }
}

#[cfg(test)]
mod connection_deadline_tests {
    use super::*;
    use std::time::{Duration, Instant};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const SHORT: Duration = Duration::from_secs(1);

    #[derive(Clone)]
    struct NeverHandshakes;

    /// Holds the socket and never completes, like a client that connects
    /// and sends no ClientHello.
    impl<I: Send + 'static, S: Send + 'static> axum_server::accept::Accept<I, S> for NeverHandshakes {
        type Stream = I;
        type Service = S;
        type Future =
            std::pin::Pin<Box<dyn std::future::Future<Output = std::io::Result<(I, S)>> + Send>>;

        fn accept(&self, stream: I, _service: S) -> Self::Future {
            Box::pin(async move {
                let _held = stream;
                std::future::pending().await
            })
        }
    }

    fn app() -> axum::Router {
        axum::Router::new()
            .route("/fast", axum::routing::get(|| async { "fast" }))
            .route(
                "/slow",
                axum::routing::get(|| async {
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    "slow"
                }),
            )
    }

    async fn start<A>(inner: A) -> std::net::SocketAddr
    where
        A: axum_server::accept::Accept<
                tokio::net::TcpStream,
                axum::Router,
                Stream = tokio::net::TcpStream,
                Service = axum::Router,
            > + Clone
            + Send
            + Sync
            + 'static,
        A::Future: Send + 'static,
    {
        let handle = axum_server::Handle::new();
        let mut server = axum_server::bind("127.0.0.1:0".parse().unwrap())
            .acceptor(ConnectionDeadlines {
                inner,
                handshake: SHORT,
                request_start: SHORT,
            })
            .handle(handle.clone());
        idle_connection_timers(server.http_builder(), SHORT);
        tokio::spawn(server.serve(app().into_make_service()));
        handle.listening().await.unwrap()
    }

    async fn start_plain() -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(serve_plain(listener, app(), SHORT));
        addr
    }

    /// Reads until the server closes the connection or `limit` passes;
    /// returns what arrived and whether it was closed.
    async fn read_until_closed(s: &mut tokio::net::TcpStream, limit: Duration) -> (String, bool) {
        let mut got = Vec::new();
        let mut buf = [0u8; 4096];
        let deadline = Instant::now() + limit;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match tokio::time::timeout(left, s.read(&mut buf)).await {
                Ok(Ok(0)) | Ok(Err(_)) => return (String::from_utf8_lossy(&got).into(), true),
                Ok(Ok(n)) => got.extend_from_slice(&buf[..n]),
                Err(_) => return (String::from_utf8_lossy(&got).into(), false),
            }
        }
    }

    fn get(path: &str) -> Vec<u8> {
        format!("GET {path} HTTP/1.1\r\nHost: t\r\nConnection: keep-alive\r\n\r\n").into_bytes()
    }

    #[tokio::test]
    async fn a_silent_connection_is_closed() {
        let addr = start_plain().await;
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        let t = Instant::now();
        let (_, closed) = read_until_closed(&mut s, Duration::from_secs(6)).await;
        assert!(closed, "a connection that sent nothing stayed open");
        assert!(
            t.elapsed() < Duration::from_secs(4),
            "closed only after {:?}",
            t.elapsed()
        );
    }

    #[tokio::test]
    async fn an_idle_keep_alive_is_closed_after_its_answer() {
        let addr = start_plain().await;
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        s.write_all(&get("/fast")).await.unwrap();
        let (got, closed) = read_until_closed(&mut s, Duration::from_secs(6)).await;
        assert!(
            got.starts_with("HTTP/1.1 200"),
            "no answer before the close: {got:?}"
        );
        assert!(got.ends_with("fast"), "{got:?}");
        assert!(closed, "an idle keep-alive connection stayed open");
    }

    #[tokio::test]
    async fn a_request_slower_than_the_deadline_still_gets_its_answer() {
        let addr = start_plain().await;
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        s.write_all(&get("/slow")).await.unwrap();
        let (got, _) = read_until_closed(&mut s, Duration::from_secs(8)).await;
        assert!(got.starts_with("HTTP/1.1 200"), "{got:?}");
        assert!(got.contains("slow"), "{got:?}");
    }

    #[tokio::test]
    async fn a_second_request_inside_the_window_is_served() {
        let addr = start_plain().await;
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        for _ in 0..2 {
            s.write_all(&get("/fast")).await.unwrap();
            let mut buf = [0u8; 4096];
            let n = tokio::time::timeout(Duration::from_secs(3), s.read(&mut buf))
                .await
                .unwrap()
                .unwrap();
            let got = String::from_utf8_lossy(&buf[..n]);
            assert!(got.starts_with("HTTP/1.1 200"), "{got:?}");
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    }

    #[tokio::test]
    async fn a_handshake_that_never_finishes_is_cut_off() {
        let addr = start(NeverHandshakes).await;
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        let t = Instant::now();
        let (_, closed) = read_until_closed(&mut s, Duration::from_secs(6)).await;
        assert!(closed, "a stalled handshake held its connection");
        assert!(
            t.elapsed() < Duration::from_secs(4),
            "closed only after {:?}",
            t.elapsed()
        );
    }
}
