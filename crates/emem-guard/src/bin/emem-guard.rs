//! `emem-guard` — run the verdict server.
//!
//! Simple mode is the contract: one command, no config file, shadow-safe
//! defaults. A first run generates a node key, opens a log, and starts
//! answering. Anything an operator has to decide before the first verdict is
//! friction that stops a trial, so nothing here is required.

use std::sync::Arc;

use emem_guard::server::{router, Guard};
use emem_guard::{policy::Config, store::FileLog, LogFailurePolicy};

const USAGE: &str = "\
emem-guard: a signed allow/deny server for claims about the physical world

USAGE:
    emem-guard [--bind ADDR] [--data DIR] [--secret WHSEC] [--require-signature]

OPTIONS:
    --bind ADDR            listen address (default 127.0.0.1:8080, or PORT)
    --data DIR             where the key and verdict log live (default ./var/guard)
    --secret WHSEC         a webhook signing secret; repeat during rotation
    --require-signature    refuse unsigned requests (set once the org has saved
                           its secret; the platform's FIRST connection test
                           arrives unsigned, so this starts off)
    --audit                verify the existing log and exit
    -h, --help             this text

ENDPOINTS:
    POST /verdict/anthropic-hook   Anthropic Inference hooks
    POST /verdict/claude-code      Claude Code client-side hooks
    GET  /health                   signer, verdict count, active rules

Put it behind TLS on a publicly routable host: the platform refuses private
and loopback ranges and does not follow redirects.
";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut bind: Option<String> = None;
    let mut data = std::path::PathBuf::from("./var/guard");
    let mut secrets: Vec<String> = Vec::new();
    let mut require_signature = false;
    let mut audit_only = false;

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            "--bind" => bind = args.next(),
            "--data" => {
                data = args.next().map(Into::into).unwrap_or(data);
            }
            "--secret" => {
                if let Some(s) = args.next() {
                    secrets.push(s);
                }
            }
            "--require-signature" => require_signature = true,
            "--audit" => audit_only = true,
            other => {
                eprintln!("emem-guard: unknown argument {other:?}\n\n{USAGE}");
                std::process::exit(2);
            }
        }
    }

    let log_path = data.join("verdicts.jsonl");

    if audit_only {
        let report = FileLog::audit(&log_path)?;
        println!("entries        {}", report.entries);
        println!("bad signature  {:?}", report.bad_signature);
        println!("broken chain   {:?}", report.broken_chain);
        println!("unparseable    {:?}", report.unparseable);
        println!("intact         {}", report.is_intact());
        // A non-zero exit so a scheduled audit fails a pipeline rather than
        // printing a problem nobody reads.
        std::process::exit(if report.is_intact() { 0 } else { 1 });
    }

    // The node key. Generated on first run and reused after, because a key
    // that rotated on restart would orphan every verdict already signed.
    let key_path = data.join("guard.key");
    let signing = load_or_create_key(&key_path)?;

    let log = FileLog::open(&log_path, signing.verifying_key())?;
    let signer_b32 = log.signer_b32().to_string();
    let logged = log.len();

    // PORT is the convention every PaaS injects.
    let bind = bind
        .or_else(|| std::env::var("PORT").ok().map(|p| format!("0.0.0.0:{p}")))
        .unwrap_or_else(|| "127.0.0.1:8080".to_string());

    if !secrets.is_empty() && !require_signature {
        eprintln!(
            "emem-guard: a secret is configured but --require-signature is off, so unsigned \
             requests are still accepted. Turn it on once your administrator has saved the \
             secret and the connection test has passed."
        );
    }

    let guard = Arc::new(Guard {
        config: Config::default(),
        log,
        signing,
        secrets,
        require_signature,
        log_failure_policy: LogFailurePolicy::default(),
    });

    println!("emem-guard listening on {bind}");
    println!("  signer   {signer_b32}");
    println!("  log      {} ({logged} verdicts)", log_path.display());
    println!("  rules    provenance=on freshness=on geo=off claim_gating=off");
    println!("  verify   emem-guard --audit --data {}", data.display());

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async move {
        let listener = tokio::net::TcpListener::bind(&bind).await?;
        axum::serve(listener, router(guard)).await
    })?;
    Ok(())
}

/// Read the node key, or create one on first run.
///
/// Written 0600. A verdict log is only as good as the key that signed it, and
/// a world-readable key would let anyone forge entries that audit clean.
fn load_or_create_key(
    path: &std::path::Path,
) -> Result<ed25519_dalek::SigningKey, Box<dyn std::error::Error>> {
    if let Ok(raw) = std::fs::read_to_string(path) {
        let bytes = data_encoding::BASE32_NOPAD.decode(raw.trim().to_uppercase().as_bytes())?;
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| "guard.key is not a 32-byte ed25519 seed")?;
        return Ok(ed25519_dalek::SigningKey::from_bytes(&bytes));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut seed = [0u8; 32];
    getrandom(&mut seed)?;
    let encoded = data_encoding::BASE32_NOPAD.encode(&seed).to_lowercase();
    std::fs::write(path, &encoded)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(ed25519_dalek::SigningKey::from_bytes(&seed))
}

/// OS randomness, without adding a dependency for one call.
fn getrandom(buf: &mut [u8; 32]) -> std::io::Result<()> {
    use std::io::Read as _;
    std::fs::File::open("/dev/urandom")?.read_exact(buf)
}
