//! `emem-airgap` — read a directory, sign what arrived, write a directory.
//!
//! Every input is an argument or an environment variable, and the clock is
//! one of them. Nothing is discovered, nothing is fetched, and the process
//! opens no socket.
//!
//! ```bash
//! emem-airgap --input /in --output /out \
//!             --profile orbital.satellite.v1 \
//!             --platform nvidia.jetson-orin \
//!             --observed-at 2026-08-20T09:00:00Z
//! ```

use std::path::PathBuf;

use ed25519_dalek::SigningKey;
use emem_airgap::{decode_dir, key_path, DecodeSettings, JoinRequest, NodeIdentity, NodeKeyFile};

/// Every flag the decoder accepts. The help text is generated from nothing, so
/// this and HELP are checked against each other by a test rather than by
/// whoever edits one of them.
const DECODE_FLAGS: &[&str] = &[
    "--help",
    "--input",
    "--output",
    "--profile",
    "--platform",
    "--observed-at",
    "--data",
    "--max-payload-bytes",
    "--max-files",
    "--max-trace-bytes",
    "--traces",
    "--stage",
    "--hwmodel",
    "--seed-file",
    "--print-seed",
    "--max-depth",
    "--flat",
];

fn env_or(flag: &str, var: &str, args: &[String]) -> Option<String> {
    // Both spellings. `--flag value` and `--flag=value` are the same thing to
    // everyone typing them, and accepting one while reporting the other as
    // missing is the kind of difference that costs an hour on hardware you
    // cannot log into.
    if let Some(i) = args.iter().position(|a| a == flag) {
        // A value that begins with two dashes is a forgotten value, not a
        // value. `--profile --platform orin` would otherwise set the profile
        // to the literal string "--platform" and then complain that the
        // platform was missing, which sends the reader to the wrong flag.
        return args.get(i + 1).filter(|v| !v.starts_with("--")).cloned();
    }
    let prefix = format!("{flag}=");
    if let Some(a) = args.iter().find(|a| a.starts_with(&prefix)) {
        return Some(a[prefix.len()..].to_string());
    }
    std::env::var(var).ok()
}

/// Print failures as text, not as a debug-formatted error.
///
/// Returning Err from main makes Rust Debug-print it, so a multi-line
/// explanation arrives as one line with literal backslash-n in it. These
/// messages exist to be read by whoever is stuck, so main does the printing.
fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("emem-airgap: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        eprintln!("{}", HELP);
        return Ok(());
    }
    emem_airgap::reject_unknown_flags(&args, DECODE_FLAGS)?;

    // Subcommands, with decode as the default so existing invocations are
    // unchanged.
    //
    // These exist because the node was emitting records nobody could check
    // without writing Rust. A signed record whose only verifier is a library
    // is not much of an offer: whoever receives one on the ground has a shell,
    // not a compiler, and the endorser deciding whether to vouch for a node
    // needs to read its request the same way.
    match args.get(1).map(String::as_str) {
        Some("identity") => return cmd_identity(&args),
        Some("keygen") => return cmd_keygen(&args),
        Some("verify") => return cmd_verify(&args),
        Some("verify-join") => return cmd_verify_join(&args),
        _ => {}
    }

    let input = env_or("--input", "EMEM_AIRGAP_INPUT", &args)
        .ok_or("--input (or EMEM_AIRGAP_INPUT) is required")?;
    let output = env_or("--output", "EMEM_AIRGAP_OUTPUT", &args)
        .ok_or("--output (or EMEM_AIRGAP_OUTPUT) is required")?;
    let profile = env_or("--profile", "EMEM_AIRGAP_PROFILE", &args)
        .ok_or("--profile (or EMEM_AIRGAP_PROFILE) is required: the substrate profile this node writes under")?;
    let platform = env_or("--platform", "EMEM_AIRGAP_PLATFORM", &args)
        .ok_or("--platform (or EMEM_AIRGAP_PLATFORM) is required: the device platform id")?;
    // Required, not defaulted to "now". A node with a wrong clock that
    // silently stamped its own time would sign a false statement about when
    // it saw the bytes; making the caller supply it keeps the run honest and
    // reproducible.
    let observed_at = env_or("--observed-at", "EMEM_AIRGAP_OBSERVED_AT", &args).ok_or(
        "--observed-at (or EMEM_AIRGAP_OBSERVED_AT) is required, RFC 3339 UTC. Pass `now` to \
             use this machine's clock deliberately: it is never used by default, because a node \
             with a wrong clock would sign a false statement without anyone choosing that.",
    )?;
    // `now` is spelled out, never assumed.
    //
    // An unattended node on a timer has no operator to type a timestamp, and
    // there was no documented answer for what it should pass: the flag was
    // required and the one obvious value was forbidden. Making the clock an
    // explicit word keeps the property that mattered (nobody gets a
    // self-asserted time by accident) while giving an unattended node
    // something true to say.
    let observed_at = if observed_at == "now" {
        system_clock_utc()?
    } else {
        observed_at
    };
    // Signed, therefore checked. An unvalidated string here is signed into
    // every record of the run, and a record claiming it was observed at
    // "yesterday" verifies perfectly while meaning nothing. A shape check, not
    // a claim the clock is right: only the operator can know that.
    if !looks_like_rfc3339_utc(&observed_at) {
        return Err(format!(
            "--observed-at must be RFC 3339 UTC like 2026-08-20T09:00:00Z, got {observed_at:?}. \
             It is signed into every record, so it is checked rather than trusted."
        )
        .into());
    }

    let data_dir = PathBuf::from(
        env_or("--data", "EMEM_AIRGAP_DATA", &args).unwrap_or_else(|| ".".to_string()),
    );

    let created = observed_at
        .split('T')
        .next()
        .unwrap_or("unknown")
        .to_string();
    let out_path = PathBuf::from(&output);
    let (key, idfile) = load_or_create_identity(&data_dir, &created, Some(&out_path))?;
    let node_key = idfile.pubkey_b32.clone();

    let settings = DecodeSettings {
        input: PathBuf::from(&input),
        output: PathBuf::from(&output),
        node: NodeIdentity {
            node_key: node_key.clone(),
            profile,
            platform,
        },
        traces: env_or("--traces", "EMEM_AIRGAP_TRACES", &args).map(PathBuf::from),
        stage: env_or("--stage", "EMEM_AIRGAP_STAGE", &args),
        observed_at,
        max_payload_bytes: env_or(
            "--max-payload-bytes",
            "EMEM_AIRGAP_MAX_PAYLOAD_BYTES",
            &args,
        )
        .and_then(|v| v.parse().ok())
        .unwrap_or(emem_airgap::DEFAULT_MAX_PAYLOAD_BYTES),
        max_files: env_or("--max-files", "EMEM_AIRGAP_MAX_FILES", &args)
            .and_then(|v| v.parse().ok())
            .unwrap_or(emem_airgap::DEFAULT_MAX_FILES),
        max_trace_bytes: env_or("--max-trace-bytes", "EMEM_AIRGAP_MAX_TRACE_BYTES", &args)
            .and_then(|v| v.parse().ok())
            .unwrap_or(emem_airgap::DEFAULT_MAX_TRACE_BYTES),
        max_depth: env_or("--max-depth", "EMEM_AIRGAP_MAX_DEPTH", &args)
            .and_then(|v| v.parse().ok())
            .unwrap_or(emem_airgap::DEFAULT_MAX_DEPTH),
        flat: args.iter().any(|a| a == "--flat")
            || std::env::var("EMEM_AIRGAP_FLAT").is_ok_and(|v| v == "1" || v == "true"),
    };

    // Written on every run, not just the first. It is small, it is
    // deterministic for a given identity and timestamp, and a node whose
    // endorsement never came back needs the request to still be there for
    // whoever next collects the output directory.
    let hwmodel = env_or("--hwmodel", "EMEM_AIRGAP_HWMODEL", &args)
        .unwrap_or_else(|| settings.node.platform.clone());
    let join = JoinRequest::sign(
        &key,
        &settings.node.profile,
        &settings.node.platform,
        &hwmodel,
        &settings.observed_at,
    );
    std::fs::create_dir_all(&settings.output).map_err(|e| {
        format!(
            "cannot create the output directory {}: {e}. Everything this node emits leaves \
             through it, so nothing can run without it.",
            settings.output.display()
        )
    })?;
    // Per-node for the same reason the run report is: several nodes may share
    // one output mount, and each one's request has to survive the others.
    std::fs::write(
        settings.output.join(format!(
            "join_request.{}.json",
            emem_airgap::short_key(&node_key)
        )),
        serde_json::to_vec_pretty(&join)?,
    )
    .map_err(|e| {
        format!(
            "cannot write the join request into {}: {e}",
            settings.output.display()
        )
    })?;

    let report = decode_dir(&key, &settings)?;
    // stderr, so stdout stays free for the report itself.
    eprintln!("emem-airgap  node {node_key}");
    eprintln!(
        "  {} recorded ({} citing an encoder trace), {} skipped, {} bytes read, {} bytes written",
        report.recorded,
        report.traced,
        report.skipped.len(),
        report.bytes_read,
        report.bytes_written
    );
    for s in &report.skipped {
        eprintln!("  skipped {}: {}", s.name, s.reason);
    }
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

/// Load this node's identity, or create it once.
///
/// The file is the same shape agents already use in `agent_identity.json`:
/// one format for anything that holds a key and answers for what it signs.
///
/// It is never regenerated. A new key would orphan every custody record
/// already signed under the old one and invalidate any endorsement an
/// operator had issued for it, so an unreadable file is an error the operator
/// has to look at rather than something to paper over with a fresh identity.
fn load_or_create_identity(
    data_dir: &std::path::Path,
    created: &str,
    output_dir: Option<&std::path::Path>,
) -> Result<(SigningKey, NodeKeyFile), Box<dyn std::error::Error>> {
    // Supplied out of band, before anything touches a disk.
    //
    // A host may give this node no writable mount at all. The two-mount
    // contract on at least one flight platform is an uplinked input folder and
    // a downlinked results folder, and nothing else: the rootfs is read-only
    // and the process is not root. Told to keep its identity in the results
    // folder, this node would write its ed25519 SEED into the one directory
    // that gets sent to the ground. An operator can instead generate the key
    // on the ground with `keygen`, hold it wherever they hold secrets, and
    // hand it to the node in the environment, where it never lands on storage
    // this node can write or anyone else can read off a downlink.
    if let Some(file) = emem_airgap::seed_from_environment()? {
        let key = file
            .signing_key()
            .ok_or("EMEM_AIRGAP_SEED_HEX is not 32 bytes of hex")?;
        return Ok((key, file));
    }
    let path = key_path(data_dir);
    if let Some(found) = load_identity(&path)? {
        return Ok(found);
    }
    // About to mint one. Refuse if it would land where the output goes.
    //
    // node_identity.json holds seed_hex, the raw ed25519 private seed. The
    // output directory is the one thing that leaves this machine: on a
    // spacecraft it is downlinked, on a shared host it is collected. Writing
    // the key into it publishes the node's private half to everyone who reads
    // a results folder, and it looks like a working configuration right up
    // until it does not. Reported before anything is written, with the two
    // ways out.
    if let Some(out) = output_dir {
        if inside(&path, out) {
            return Err(format!(
                "refusing to create the node identity at {}, because it is inside the output \
                 directory {} and would be published with the records. node_identity.json holds \
                 this node's PRIVATE ed25519 seed.\n\n\
                 Two ways out on a host with no third writable mount:\n  \
                 1. Generate the key on the ground with `emem-airgap keygen --print-seed`, then \
                 pass it as EMEM_AIRGAP_SEED_HEX (or EMEM_AIRGAP_SEED_FILE). Nothing is written \
                 and --data is not needed.\n  \
                 2. Give --data a writable directory that is NOT inside --output.",
                path.display(),
                out.display()
            )
            .into());
        }
    }
    let mut seed = [0u8; 32];
    getrandom(&mut seed)?;
    let file = NodeKeyFile::new(
        seed,
        created,
        "emem air-gapped node: signs custody for payloads that arrive in its input directory",
    );
    let key = file.signing_key().ok_or("generated seed did not decode")?;
    std::fs::create_dir_all(data_dir).map_err(|e| {
        format!(
            "cannot create the data directory {}: {e}. This is where node_identity.json lives, \
             and it must be on storage that survives a restart: a new identity every run orphans \
             every record signed under the last one.",
            data_dir.display()
        )
    })?;
    // Another starter may have claimed the identity between our look and our
    // write.
    //
    // This is load-OR-create and it used to only do one of them: the loser of
    // the race exited with "File exists (os error 17)", which reads as a
    // broken disk rather than as another container getting there first. One
    // run in forty-eight died this way when eight started together on an empty
    // data directory, which is how a host brings up several containers on
    // first boot, not an unusual way to do it.
    let claimed = write_private(&path, serde_json::to_string_pretty(&file)?.as_bytes())
        .map_err(|e| format!("cannot write the node identity to {}: {e}", path.display()))?;
    if !claimed {
        return load_identity(&path)?.ok_or_else(|| {
            format!(
                "another starter claimed {} while this one was starting, and it cannot be read \
                 back. Both are trying to be the same node; run one.",
                path.display()
            )
            .into()
        });
    }
    eprintln!(
        "emem-airgap  new node identity {} written to {}",
        file.pubkey8,
        path.display()
    );
    Ok((key, file))
}

/// Would a file at `path` end up inside `dir`?
///
/// Compares resolved paths where it can, so the same directory reached by a
/// different spelling is still the same directory, and falls back to the
/// lexical form for a path that does not exist yet, which is the normal case
/// for an identity about to be created.
fn inside(path: &std::path::Path, dir: &std::path::Path) -> bool {
    let resolve = |p: &std::path::Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let dir = resolve(dir);
    let parent = path.parent().map(resolve).unwrap_or_default();
    parent.starts_with(&dir)
}

/// Read an existing node identity, or `None` if there is none yet.
///
/// A file that exists but does not parse is an error rather than a reason to
/// mint a new key: a fresh identity would orphan every record already signed
/// under the old one and void any endorsement issued for it.
fn load_identity(
    path: &std::path::Path,
) -> Result<Option<(SigningKey, NodeKeyFile)>, Box<dyn std::error::Error>> {
    let raw = match std::fs::read_to_string(path) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display()).into()),
    };
    let file: NodeKeyFile = serde_json::from_str(&raw).map_err(|e| {
        format!(
            "{} exists but is not a node identity ({e}). Refusing to overwrite it: a new key \
             would orphan every record this node has already signed. Move it aside deliberately \
             if you really mean to start over.",
            path.display()
        )
    })?;
    let key = file
        .signing_key()
        .ok_or("node identity seed_hex is not 32 bytes of hex")?;
    Ok(Some((key, file)))
}

/// Write the node identity so that it is complete, durable, and never
/// overwrites one that already exists.
///
/// All three matter more here than anywhere else in the crate, because every
/// record this node will ever sign depends on this one file surviving.
///
/// **Complete.** The previous version created the destination and then wrote
/// into it, so for a moment the identity existed and was empty. A second
/// process starting at the same time read that empty file and reported the
/// identity as corrupt; had the first process died in that window, the empty
/// file would have stayed and every later run would have refused to start,
/// with no way back that does not involve deleting the node's identity. The
/// content is written to a temporary and linked into place complete.
///
/// **Durable.** fsync on the file before the link and on the directory after
/// it. A brown-out is the ordinary case for this hardware, and an identity
/// that only reached the page cache is an identity the node wakes up without,
/// having already signed records with it.
///
/// **Never clobbering.** `hard_link` fails if the destination exists, which is
/// what makes it the race winner's claim. `rename` would silently replace an
/// existing identity, and replacing this file is never the right outcome.
/// Returns `Ok(false)` when the destination already existed, so the caller can
/// load what is there. An `Err` is a real failure and nothing else.
///
/// Two outcomes used to be one. The function returned `AlreadyExists` both
/// when another writer had claimed the identity and when its own temporary
/// name collided, and the caller could not tell them apart. Concurrent threads
/// of one process hit the second case and were told the first, then failed
/// looking for a file nobody had written yet.
#[cfg(unix)]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<bool> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    // Unique per writer, not per process. `std::process::id()` alone is the
    // same for every thread, so two threads shared a temporary: one truncated
    // or deleted the other's, and the failure surfaced as a nonexistent
    // identity. Separate containers never hit this because their pids differ,
    // which is exactly why it survived the process-level test.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_extension(format!("{}.{n}.tmp", std::process::id()));

    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);

    let linked = std::fs::hard_link(&tmp, path);
    // The temporary goes either way: it has served its purpose if the link
    // took, and it is debris if somebody else won.
    let _ = std::fs::remove_file(&tmp);
    match linked {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
        Err(e) => return Err(e),
    }

    // The directory entry itself, so the link survives the power cut that made
    // the fsync above worth doing.
    if let Some(dir) = path.parent() {
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
    }
    Ok(true)
}

#[cfg(not(unix))]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<bool> {
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut f) => {
            use std::io::Write;
            f.write_all(bytes)?;
            f.sync_all()?;
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(e),
    }
}

/// Fill `buf` with OS randomness, without adding a dependency for it.
fn getrandom(buf: &mut [u8]) -> std::io::Result<()> {
    use std::io::Read;
    let mut f = std::fs::File::open("/dev/urandom")?;
    f.read_exact(buf)
}

/// Create this node's identity, deliberately.
///
/// The `identity` command refuses to make one, and that is still right: a key
/// minted as a side effect of an inspection command is a key nobody meant to
/// create. But an operator has to be able to get a public key BEFORE the first
/// pass, because enrolling a node means handing that key to an endorser and
/// waiting, and a node that only reveals its key after it has already recorded
/// something forces the whole enrolment to trail the first flight.
///
/// So: creation is a command of its own, named for what it does.
///
/// `--print-seed` writes the PRIVATE seed to stdout instead of a file, for a
/// host with no writable mount to keep it in. That is a real need on a
/// two-mount platform, and it is also the one output of this crate that must
/// never be logged, echoed into CI, or downlinked. It says so when it prints.
fn cmd_keygen(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let print_seed = args.iter().any(|a| a == "--print-seed");
    if print_seed {
        // Nothing is written, so no directory is needed and nothing can end up
        // in an output folder by accident.
        let mut seed = [0u8; 32];
        getrandom(&mut seed)?;
        let file = NodeKeyFile::new(
            seed,
            "generated",
            "emem air-gapped node: identity generated for out-of-band provisioning",
        );
        eprintln!(
            "This is a PRIVATE key. Anything holding it can sign as this node.\n\
             Put it in a secret store and hand it to the node as EMEM_AIRGAP_SEED_HEX.\n\
             Do not log it, commit it, or let it reach an output directory.\n"
        );
        println!("EMEM_AIRGAP_SEED_HEX={}", file.seed_hex);
        eprintln!("\nnode {}", file.pubkey_b32);
        eprintln!("Give the endorser the node line, never the seed.");
        return Ok(());
    }

    let data_dir = PathBuf::from(
        env_or("--data", "EMEM_AIRGAP_DATA", args).unwrap_or_else(|| ".".to_string()),
    );
    // No output directory to compare against here: keygen is run deliberately,
    // by a person, against a directory they chose.
    let (_, file) = load_or_create_identity(&data_dir, "keygen", None)?;
    println!("node    {}", file.pubkey_b32);
    println!("short   {}", file.pubkey8);
    println!("at      {}", key_path(&data_dir).display());
    println!();
    println!("Give the endorser the node line. The seed stays in that file, mode 600.");
    Ok(())
}

/// Print this node's public identity. Never the seed.
///
/// The operator needs the public key to endorse the node, and asking them to
/// grep a JSON file for it invites pasting the wrong field. This prints only
/// what is safe to hand out.
fn cmd_identity(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = PathBuf::from(
        env_or("--data", "EMEM_AIRGAP_DATA", args).unwrap_or_else(|| ".".to_string()),
    );
    // The environment first, like every other command.
    //
    // A node whose identity is supplied as EMEM_AIRGAP_SEED_HEX has no file to
    // read, so this reported that it had no identity at all and an operator
    // running that way could not get their own public key to hand to an
    // endorser. Reported from a real deployment.
    if let Some(file) = emem_airgap::seed_from_environment()? {
        println!("node    {}", file.pubkey_b32);
        println!("short   {}", file.pubkey8);
        println!("from    the environment; nothing is stored on this machine");
        return Ok(());
    }
    let path = key_path(&data_dir);
    if !path.exists() {
        return Err(format!(
            "no identity at {}, and no seed in the environment.\n\n\
             Either run a decode pass, which creates one, or supply it: \
             `emem-airgap keygen --print-seed` generates a key and prints it as \
             EMEM_AIRGAP_SEED_HEX, which needs no writable directory at all.\n\n\
             This command does not create one, because a key generated by an inspection command \
             is a key nobody meant to create.",
            path.display()
        )
        .into());
    }
    // The identity is mode 600 and owned by whichever uid wrote it. Run the
    // container as 65532, as the hardened invocation does, and a developer
    // reading it from the host is a different user: they get EACCES and a raw
    // OS error that explains nothing. Say what happened and what to do.
    let raw = std::fs::read_to_string(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            format!(
                "cannot read {}: permission denied.\n\n\
                 The identity is mode 600, owned by the user that created it. If the node ran \
                 in a container as another uid (65532 in the documented invocation), read it \
                 the same way:\n\n  \
                 docker run --rm -v <data-dir>:/data emem-airgap identity --data /data\n\n\
                 That is not a workaround: a private key readable by every user on the host \
                 would be the actual problem.",
                path.display()
            )
        } else {
            format!("cannot read {}: {e}", path.display())
        }
    })?;
    let file: NodeKeyFile = serde_json::from_str(&raw)?;
    println!("node_key   {}", file.pubkey_b32);
    println!("short      {}", file.pubkey8);
    println!("created    {}", file.created);
    println!("role       {}", file.role);
    println!("path       {}", path.display());
    Ok(())
}

/// Read a signed record from disk for one of the ground-side commands.
///
/// Two things the plain read did not do.
///
/// It named nothing. A missing file produced "No such file or directory (os
/// error 2)" with no indication of which file, which is the same defect that
/// was fixed inside the node and had been left in the tools an endorser runs.
/// These commands exist to be used by someone holding a USB stick and a
/// terminal, and an error they cannot act on wastes the trip.
///
/// It had no bound. A custody record is under a kilobyte and a join request is
/// smaller; a 50 MB file named `record.custody.json` was read whole, and the
/// process grew to match it. These files arrive from a node the endorser has
/// not yet decided to trust, which is the whole point of the command, so its
/// input is untrusted by definition.
fn read_record(path: &str, what: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    /// 1 MiB. Three orders of magnitude above any real record, so a legitimate
    /// file cannot hit it, and small enough that a hostile one costs nothing.
    const MAX: u64 = 1024 * 1024;
    let meta =
        std::fs::metadata(path).map_err(|e| format!("cannot read the {what} at {path}: {e}"))?;
    if meta.len() > MAX {
        return Err(format!(
            "{path} is {} bytes, which is not a {what}: they are under a kilobyte. Refusing to \
             read it.",
            meta.len()
        )
        .into());
    }
    std::fs::read(path).map_err(|e| format!("cannot read the {what} at {path}: {e}").into())
}

/// Verify a custody record, and optionally that it covers a given payload.
///
/// Two questions, reported separately, because they are different: whether the
/// record is genuine, and whether the file you have is the one it describes.
fn cmd_verify(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let record_path = args
        .get(2)
        .ok_or("usage: emem-airgap verify <record.custody.json> [payload-file]")?;
    let raw = read_record(record_path, "custody record")?;
    let record: emem_airgap::Custody = serde_json::from_slice(&raw)
        .map_err(|e| format!("{record_path} is not a custody record: {e}"))?;

    match record.verify() {
        Ok(_) => println!("signature  VALID, signed by {}", record.node.node_key),
        Err(e) => {
            println!("signature  INVALID: {e}");
            std::process::exit(1);
        }
    }
    println!("name       {}", record.name);
    println!("digest     {}", record.payload_digest);
    println!("size       {} bytes", record.size_bytes);
    println!("observed   {}", record.observed_at);
    println!("profile    {}", record.node.profile);
    println!("platform   {}", record.node.platform);
    if let Some(st) = &record.stage {
        println!("stage      {st}");
    }
    match &record.trace_cid {
        Some(c) => println!("trace      {c}  (fetch and verify it yourself)"),
        None => println!("trace      none"),
    }
    println!("claims     {}", record.assurance);

    if let Some(payload_path) = args.get(3) {
        // The payload itself has no cap: it is the thing being checked, the
        // caller named it deliberately, and refusing to hash a large frame
        // would defeat the command.
        let payload = std::fs::read(payload_path)
            .map_err(|e| format!("cannot read the payload at {payload_path}: {e}"))?;
        if record.covers(&payload) {
            println!("payload    MATCHES {payload_path}");
        } else {
            println!("payload    DOES NOT MATCH {payload_path}");
            std::process::exit(1);
        }
    } else {
        println!("payload    not checked; pass the file as a second argument to check it");
    }
    Ok(())
}

/// Verify a join request, for the endorser deciding whether to vouch.
///
/// It prints what the signature proves and, just as loudly, what it does not,
/// because the whole risk in endorsing is mistaking a self-signed claim about
/// hardware for evidence about hardware.
fn cmd_verify_join(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let path = args
        .get(2)
        .ok_or("usage: emem-airgap verify-join <join_request.json>")?;
    let raw = read_record(path, "join request")?;
    let j: JoinRequest =
        serde_json::from_slice(&raw).map_err(|e| format!("{path} is not a join request: {e}"))?;
    if j.verify() {
        println!("signature  VALID: the sender holds the private half of this key");
    } else {
        println!("signature  INVALID");
        std::process::exit(1);
    }
    println!("node_key   {}", j.node_key);
    println!("created    {}", j.created_at);
    println!();
    println!("CLAIMED BY THE NODE, and not evidence of anything:");
    println!("  profile   {}", j.profile);
    println!("  platform  {}", j.platform);
    println!("  hwmodel   {}", j.hwmodel);
    println!();
    println!("{}", j.next_step);
    Ok(())
}

/// A shape check for RFC 3339 UTC, without pulling in a date library.
///
/// Deliberately narrow: `YYYY-MM-DDTHH:MM:SSZ`. A node that writes exactly one
/// timestamp format is easier to reason about than one accepting every legal
/// spelling, and whoever supplies it already knows the shape.
/// This machine's clock as RFC 3339 UTC, for an explicit `--observed-at now`.
///
/// Seconds since the epoch, converted by hand: no chrono, because this crate
/// links nothing it does not need and the conversion is arithmetic. Leap
/// seconds are not represented in Unix time, so neither are they here.
fn system_clock_utc() -> Result<String, Box<dyn std::error::Error>> {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "this machine's clock is before 1970, so `now` cannot be trusted")?
        .as_secs();
    Ok(rfc3339_utc(secs))
}

/// Seconds since the epoch as RFC 3339 UTC. Separate from the clock so it can
/// be tested against known dates instead of against whatever today is.
fn rfc3339_utc(secs: u64) -> String {
    let (mut days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);

    // Civil-from-days, the standard algorithm, shifted to a 1st-March era so
    // the leap day falls at the end of the cycle and needs no special case.
    days += 719_468;
    let era = days.div_euclid(146_097);
    let doe = days.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

fn looks_like_rfc3339_utc(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' {
        return false;
    }
    if b[13] != b':' || b[16] != b':' || b[19] != b'Z' {
        return false;
    }
    [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18]
        .iter()
        .all(|&i| b[i].is_ascii_digit())
}

const HELP: &str = "\
emem-airgap  one directory in, one directory out, no network.

COMMANDS
  (none)                 decode: read --input, write custody records to --output
  keygen                 create this node's identity, deliberately
  keygen --print-seed    generate one and print the seed for out-of-band use,
                         writing nothing to disk
  identity               print this node's public key, for the endorser
  verify <rec> [payload] check a custody record, and optionally the payload
  verify-join <req>      check a join request, for whoever is endorsing it

DECODE OPTIONS

  --input        <dir>   payloads arrive here; never modified
  --output       <dir>   custody records are written here
  --profile      <id>    substrate profile this node writes under
  --platform     <id>    device platform id
  --hwmodel      <id>    EAT hwmodel claim for the join request
                         (default: the platform id)
  --observed-at  <ts>    RFC 3339 UTC; required, never defaulted. Pass the
                         literal `now` to use this machine's clock on purpose,
                         which is what an unattended node on a timer wants.
  --data         <dir>   where node_identity.json lives (default: .)
                         Not needed at all when the identity is supplied in
                         the environment; see EMEM_AIRGAP_SEED_HEX below.
  --seed-file    <path>  read the identity's seed from this file instead
                         (same as EMEM_AIRGAP_SEED_FILE)
  --max-payload-bytes <n> refuse payloads larger than this (default 256 MiB)
  --max-trace-bytes <n>  refuse trace files larger than this (default 16 MiB)
  --max-files    <n>     most files in one run (default 10000)
  --max-depth    <n>     how deep to descend into --input (default 32).
                         Subdirectories ARE walked; a payload's signed name is
                         its path relative to --input.
  --flat                 write every record into the top of --output instead of
                         mirroring the input's directories, for a host that
                         collects only top-level files
  --traces       <dir>   emem.os_trace.v1 records from an encoder on this machine;
                         a payload a trace covers gets that trace cited
  --stage        <label> what stage these payloads are at, your vocabulary

Each flag also reads an environment variable: EMEM_AIRGAP_INPUT, _OUTPUT,
_PROFILE, _PLATFORM, _OBSERVED_AT, _DATA, _HWMODEL. Either spelling works,
with a space or an equals sign. A flag this command does not have is refused
rather than ignored, so a typo stops the run instead of quietly changing it.

EMEM_AIRGAP_SEED_HEX supplies this node's identity directly: 64 hex characters,
the raw ed25519 seed, as `keygen --print-seed` prints it. Nothing is written to
disk, so a host that can give this node no writable directory can still run it
with a stable identity. EMEM_AIRGAP_SEED_FILE reads the same 64 characters from
a path, for a platform that mounts secrets but cannot set an environment.

The node REFUSES to create its identity inside --output: that file holds the
private seed, and the output directory is the one that leaves the machine.

Writes one <name>.<node>.custody.json per payload, plus run.<node>.json. Every
output carries the node's short key, so several nodes can share one output
mount without overwriting each other. The payload itself never leaves: only
the record does.";

#[cfg(test)]
mod tests {
    use super::*;

    /// Eight processes starting on one empty data directory must agree on one
    /// identity, and none of them may fail.
    ///
    /// One run in forty-eight used to die with "File exists (os error 17)".
    /// The function is called load-or-create and it only did one of them: the
    /// loser of the race gave up instead of reading the key the winner had
    /// just written, and the message it printed reads as a broken disk rather
    /// than as another container getting there first. Eight containers coming
    /// up together on first boot is not an unusual way to start a host; it is
    /// the normal one.
    #[test]
    fn concurrent_first_runs_agree_on_one_identity() {
        let d = std::env::temp_dir().join("emem-airgap-idrace");
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();

        let keys: Vec<String> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    s.spawn(|| {
                        let (_, file) = load_or_create_identity(&d, "2026-08-20", None)
                            .expect("a process that loses the race must load, not fail");
                        file.pubkey_b32
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });

        let first = &keys[0];
        assert!(
            keys.iter().all(|k| k == first),
            "eight starts produced more than one identity: {keys:?}"
        );
        // And exactly one file, with no temporary left behind.
        let entries: Vec<String> = std::fs::read_dir(&d)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            entries,
            vec!["node_identity.json".to_string()],
            "{entries:?}"
        );
    }

    /// An identity that exists is never replaced.
    ///
    /// Replacing it orphans every record already signed under the old key and
    /// voids any endorsement issued for it, so the write claims the
    /// destination rather than overwriting it.
    #[test]
    fn an_existing_identity_is_never_overwritten() {
        let d = std::env::temp_dir().join("emem-airgap-idclobber");
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let path = d.join("node_identity.json");

        assert!(write_private(&path, b"the original").expect("first write"));
        assert!(
            !write_private(&path, b"a replacement")
                .expect("a second write is refused, not an error"),
            "a second write reported that it had claimed the identity"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"the original");
        // No temporary survives the refusal.
        let leftovers: Vec<_> = std::fs::read_dir(&d)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "node_identity.json")
            .collect();
        assert!(leftovers.is_empty(), "debris left behind: {leftovers:?}");
    }

    /// The identity file is readable only by its owner. It holds the seed.
    #[cfg(unix)]
    #[test]
    fn the_identity_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join("emem-airgap-idperm");
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let path = d.join("node_identity.json");
        write_private(&path, b"seed").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "identity is mode {mode:o}, not 0600");
    }

    /// The ground-side commands read files a node sent, which is to say files
    /// from a party the endorser has not yet decided to trust.
    #[test]
    fn a_record_read_from_disk_is_bounded_and_names_its_path() {
        let d = std::env::temp_dir().join("emem-airgap-readrecord");
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();

        // A file too large to be what it claims to be is refused before it is
        // read. A 50 MB file named record.custody.json used to be read whole,
        // and the process grew to match it.
        let big = d.join("big.custody.json");
        std::fs::write(&big, vec![b'{'; 2 * 1024 * 1024]).unwrap();
        let e = read_record(big.to_str().unwrap(), "custody record").unwrap_err();
        assert!(e.to_string().contains("under a kilobyte"), "{e}");

        // A missing file says which file. It used to say only "No such file or
        // directory (os error 2)", which is the same defect that was fixed
        // inside the node and left in the tools someone runs by hand.
        let gone = d.join("nowhere.custody.json");
        let e = read_record(gone.to_str().unwrap(), "custody record").unwrap_err();
        assert!(e.to_string().contains("nowhere.custody.json"), "{e}");
        assert!(
            e.to_string().contains("cannot read the custody record"),
            "{e}"
        );

        // A real record is read.
        let small = d.join("ok.custody.json");
        std::fs::write(&small, b"{}").unwrap();
        assert_eq!(
            read_record(small.to_str().unwrap(), "custody record").unwrap(),
            b"{}"
        );
    }

    /// Every flag the decoder accepts must appear in the README too.
    ///
    /// The README is what a developer reads before they ever run `--help`, and
    /// it had fallen behind by four flags. That matters more now that an
    /// unknown flag is refused rather than ignored: a README that names a flag
    /// the binary does not have stops the run outright.
    #[test]
    fn the_readme_names_every_flag_the_decoder_accepts() {
        const README: &str = include_str!("../README.md");
        for flag in DECODE_FLAGS {
            if *flag == "--help" {
                continue;
            }
            assert!(
                README.contains(flag),
                "{flag} is accepted but crates/emem-airgap/README.md never names it"
            );
        }
    }

    /// Hand-rolled date arithmetic, checked against dates that break it.
    ///
    /// Leap years, the century rule, the four-hundred-year exception, and the
    /// day either side of each. `--observed-at now` signs whatever this
    /// returns into every record of the run, so a wrong February would be a
    /// wrong claim on a spacecraft nobody can correct.
    #[test]
    fn the_clock_conversion_matches_known_dates() {
        // Expected values computed with an independent implementation, not
        // typed from memory: my first pass had two of them wrong and the code
        // right, which is the wrong way round to learn it.
        for (secs, want) in [
            // the epoch
            (0u64, "1970-01-01T00:00:00Z"),
            (1, "1970-01-01T00:00:01Z"),
            (86399, "1970-01-01T23:59:59Z"),
            (86400, "1970-01-02T00:00:00Z"),
            // 2000 IS a leap year: divisible by 400
            (951782400, "2000-02-29T00:00:00Z"),
            (951868800, "2000-03-01T00:00:00Z"),
            // 2100 is NOT: divisible by 100, not by 400
            (4107456000, "2100-02-28T00:00:00Z"),
            // the day after, with no 29th between them
            (4107542400, "2100-03-01T00:00:00Z"),
            // an ordinary leap year
            (1709164800, "2024-02-29T00:00:00Z"),
            // year boundary
            (1704067199, "2023-12-31T23:59:59Z"),
            (1704067200, "2024-01-01T00:00:00Z"),
            // well past any mission
            (2524608000, "2050-01-01T00:00:00Z"),
        ] {
            assert_eq!(rfc3339_utc(secs), want, "at {secs} seconds");
            assert!(
                looks_like_rfc3339_utc(&rfc3339_utc(secs)),
                "the clock produced a value the validator would reject: {}",
                rfc3339_utc(secs)
            );
        }
    }

    /// The flags the decoder accepts and the flags its help text describes must be
    /// the same set. Two hand-kept lists drift, and the one that drifts silently
    /// is the one nobody runs.
    #[test]
    fn the_help_text_and_the_accepted_flags_agree() {
        let help = HELP;
        for flag in DECODE_FLAGS {
            if *flag == "--help" {
                continue;
            }
            assert!(
                help.contains(flag),
                "{flag} is accepted but the help text never mentions it"
            );
        }
        for line in help.lines() {
            // The options block is indented two spaces. Prose that happens to
            // mention a flag is not an offer of one, and treating it as one
            // made this test fail on its own explanation of --flag=value.
            let Some(rest) = line.strip_prefix("  --") else {
                continue;
            };
            let name = format!(
                "--{}",
                rest.split([' ', '\t', '=']).next().unwrap_or_default()
            );
            assert!(
                DECODE_FLAGS.contains(&name.as_str()),
                "the help text offers {name} but the parser refuses it"
            );
        }
    }
}
