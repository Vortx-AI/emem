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
    --responder URL        an emem responder holding the corpus, e.g.
                           http://127.0.0.1:5051. Without one the guard signs
                           and logs every verdict but verifies no citation.
    --restrict-cell CELL   a cell64 to refuse references to; repeat for more.
                           Exact match only, never a geocode.
    --module NAME          load a detection module. Repeat for more; order is
                           the pipeline order and is bound into every verdict.
                           `secret-patterns` is built in. `webhook:URL` calls
                           your own classifier, abstaining on timeout.
    --modules-no-text      never show transcript text to any module, whatever
                           it declared. For a relay where text may not cross.
    --claim-gating         deny measurable physical claims that cite nothing.
                           Off by default. Run it with --shadow first and read
                           --report before you enforce it.
    --shadow               evaluate and log every rule, return allow always.
                           The way to measure what enforcing would cost on your
                           own traffic before it costs it.
    --conformance URL      run the deployment conformance suite against a
                           running node and exit non-zero on any failure.
                           This is the gate before pointing a checkpoint at it.
    --audit                verify the existing log and exit
    --report               count what this node has done, from the log, and exit
    -h, --help             this text

ENDPOINTS, OPEN:
    POST /verdict                  any agent, any model, any framework
    POST /verdict/mcp              MCP tools/call, gating a call or a result
    POST /verdict/openai           OpenAI-shaped chat or moderations body
    POST /verdict/cloudevent       CloudEvents 1.0, structured or binary
    POST /verdict/policy           OPA-style {input} -> {result}
    POST /verdict/batch            many transcripts, one request

ENDPOINTS, VENDOR:
    POST /verdict/anthropic-hook   Anthropic Inference hooks
    POST /verdict/claude-code      Claude Code client-side hooks

ENDPOINTS, EVIDENCE:
    GET  /health                   signer, verdict count, active rules
    GET  /.well-known/emem-guard.json   the whole contract, machine-readable
    GET  /log/head                 chain head, for a witness or a mirror
    GET  /log/entry/leaf_7         the signed record a denial named
    GET  /log/entries?start=0      a window, so anyone can mirror the log
    GET  /log/report               what this node has done, counted from disk
    GET  /modules                  the loaded detection modules and their health

Put it behind TLS on a publicly routable host: the platform refuses private
and loopback ranges and does not follow redirects.
";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut bind: Option<String> = None;
    let mut data = std::path::PathBuf::from("./var/guard");
    let mut secrets: Vec<String> = Vec::new();
    let mut require_signature = false;
    let mut audit_only = false;
    let mut report_only = false;
    let mut conformance: Option<String> = None;
    let mut claim_gating = false;
    let mut shadow = false;
    let mut module_specs: Vec<String> = Vec::new();
    let mut modules_may_read_text = true;
    let mut responder: Option<String> = None;
    let mut restricted: std::collections::HashSet<String> = Default::default();

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
            "--responder" => responder = args.next(),
            "--restrict-cell" => {
                if let Some(c) = args.next() {
                    restricted.insert(c);
                }
            }
            "--module" => {
                if let Some(m) = args.next() {
                    module_specs.push(m);
                }
            }
            "--modules-no-text" => modules_may_read_text = false,
            "--claim-gating" => claim_gating = true,
            "--shadow" => shadow = true,
            "--conformance" => conformance = args.next(),
            "--audit" => audit_only = true,
            "--report" => report_only = true,
            other => {
                eprintln!("emem-guard: unknown argument {other:?}\n\n{USAGE}");
                std::process::exit(2);
            }
        }
    }

    if let Some(url) = conformance {
        // Against a URL over the wire, exactly as a checkpoint would reach it.
        // Everything this catches lives between the code and the deployment.
        let outcomes = emem_guard::conformance::run(&url);
        std::process::exit(emem_guard::conformance::report(&outcomes));
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

    if report_only {
        // Counted from the log rather than from memory, so the numbers an
        // operator quotes and the numbers an outsider can verify are the same
        // numbers. Anyone with a copy of the file reproduces this exactly.
        let r = emem_guard::ShadowReport::read(&log_path)?;
        println!("verdicts           {}", r.entries);
        println!("blocked            {}", r.enforced_denies);
        println!("would have blocked {}", r.observed_denies);
        println!("fired rate         {:.4}", r.fired_rate());
        println!("citations checked  {}", r.citations_checked);
        if let (Some(a), Some(b)) = (r.first_unix_s, r.last_unix_s) {
            println!("window             {a} .. {b} unix seconds");
        }
        println!("\nby code            enforced / observed");
        for (code, (e, o)) in &r.by_code {
            println!("  {code:<18} {e:>8} / {o}");
        }
        println!("\nby checkpoint");
        for (cp, n) in &r.by_checkpoint {
            println!("  {cp:<34} {n}");
        }
        if r.entries == 0 {
            println!("\nNothing logged yet. Point a checkpoint at this node first.");
        }
        return Ok(());
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

    // Without a responder the node signs and logs but verifies nothing, and
    // says so on /health rather than implying otherwise.
    let resolver: Box<dyn emem_guard::Resolver> = match responder.as_deref() {
        Some(base) => Box::new(emem_guard::ResponderResolver::new(
            base,
            emem_guard::server::RESOLVE_BUDGET,
        )?),
        None => Box::new(emem_guard::NullResolver),
    };
    let resolver_name = resolver.describe();

    // A restricted list is the only thing that makes the geo rule meaningful,
    // so it turns itself on when one is supplied and stays off otherwise.
    let mut modules = emem_guard::ModuleRegistry::new();
    for spec in &module_specs {
        let loaded: Result<Box<dyn emem_guard::PolicyModule>, String> = match spec.as_str() {
            "secret-patterns" => Ok(Box::new(emem_guard::SecretPatterns::new())),
            other => match other.strip_prefix("webhook:") {
                Some(url) => emem_guard::OrgWebhook::new(
                    "org-webhook",
                    url,
                    std::time::Duration::from_millis(200),
                    // Declared slow, so it cannot block until an operator has
                    // measured their own classifier and said otherwise. A
                    // network call on the enforcing path is a decision, not a
                    // default.
                    emem_guard::LatencyClass::Slow,
                    emem_guard::DataClass::NeedsText,
                )
                .map(|m| Box::new(m.at(url)) as Box<dyn emem_guard::PolicyModule>)
                .map_err(|e| e.to_string()),
                None => Err(format!(
                    "unknown module {other:?}; known: secret-patterns, webhook:<url>"
                )),
            },
        };
        match loaded {
            Ok(m) => {
                if let Err(e) = modules.load(m) {
                    eprintln!("emem-guard: refusing to load {spec}: {e}");
                    std::process::exit(2);
                }
            }
            Err(e) => {
                eprintln!("emem-guard: {e}");
                std::process::exit(2);
            }
        }
    }

    let config = Config {
        geo_restriction: !restricted.is_empty(),
        claim_gating,
        mode: if shadow {
            emem_guard::Mode::Shadow
        } else {
            emem_guard::Mode::Enforce
        },
        ..Config::default()
    };

    let guard = Arc::new(Guard {
        config,
        modules,
        modules_may_read_text,
        resolver,
        restricted_cells: restricted,
        log,
        signing,
        secrets,
        require_signature,
        log_failure_policy: LogFailurePolicy::default(),
    });

    println!("emem-guard listening on {bind}");
    println!("  signer   {signer_b32}");
    println!("  log      {} ({logged} verdicts)", log_path.display());
    println!("  resolve  {resolver_name}");
    if resolver_name == "null" {
        println!("           no responder configured: citations are not verified, only logged.");
        println!("           point one with --responder http://127.0.0.1:5051 to check them.");
    }
    if shadow {
        println!("  mode     shadow: every rule runs and is logged, nothing is blocked.");
        println!(
            "           read it back with emem-guard --report --data {}",
            data.display()
        );
    } else {
        println!("  mode     enforce");
    }
    if claim_gating {
        println!("  claims   on: a measurable claim with no citation is denied.");
        if !shadow {
            println!("           this rule denies on ABSENCE. Measure it with --shadow first.");
        }
    }
    if !module_specs.is_empty() {
        println!(
            "  modules  {} loaded: {}",
            module_specs.len(),
            module_specs.join(", ")
        );
        if !modules_may_read_text {
            println!("           text is withheld from every module (--modules-no-text)");
        }
        println!("           a slow module never blocks; a fast one that misses its");
        println!("           budget is demoted. GET /modules for the current state.");
    }
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
