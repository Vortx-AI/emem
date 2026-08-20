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

fn env_or(flag: &str, var: &str, args: &[String]) -> Option<String> {
    if let Some(i) = args.iter().position(|a| a == flag) {
        return args.get(i + 1).cloned();
    }
    std::env::var(var).ok()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        eprintln!("{}", HELP);
        return Ok(());
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
    let observed_at = env_or("--observed-at", "EMEM_AIRGAP_OBSERVED_AT", &args)
        .ok_or("--observed-at (or EMEM_AIRGAP_OBSERVED_AT) is required, RFC 3339 UTC")?;
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
    let (key, idfile) = load_or_create_identity(&data_dir, &created)?;
    let node_key = idfile.pubkey_b32.clone();

    let settings = DecodeSettings {
        input: PathBuf::from(&input),
        output: PathBuf::from(&output),
        node: NodeIdentity {
            node_key: node_key.clone(),
            profile,
            platform,
        },
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
    std::fs::create_dir_all(&settings.output)?;
    // Per-node for the same reason the run report is: several nodes may share
    // one output mount, and each one's request has to survive the others.
    std::fs::write(
        settings.output.join(format!(
            "join_request.{}.json",
            emem_airgap::short_key(&node_key)
        )),
        serde_json::to_vec_pretty(&join)?,
    )?;

    let report = decode_dir(&key, &settings)?;
    // stderr, so stdout stays free for the report itself.
    eprintln!("emem-airgap  node {node_key}");
    eprintln!(
        "  {} recorded, {} skipped, {} bytes read, {} bytes written",
        report.recorded,
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
) -> Result<(SigningKey, NodeKeyFile), Box<dyn std::error::Error>> {
    let path = key_path(data_dir);
    if path.exists() {
        let raw = std::fs::read_to_string(&path)?;
        let file: NodeKeyFile = serde_json::from_str(&raw).map_err(|e| {
            format!(
                "{} exists but is not a node identity ({e}). Refusing to overwrite it: \
                 a new key would orphan every record this node has already signed. \
                 Move it aside deliberately if you really mean to start over.",
                path.display()
            )
        })?;
        let key = file
            .signing_key()
            .ok_or("node identity seed_hex is not 32 bytes of hex")?;
        return Ok((key, file));
    }
    let mut seed = [0u8; 32];
    getrandom(&mut seed)?;
    let file = NodeKeyFile::new(
        seed,
        created,
        "emem air-gapped node: signs custody for payloads that arrive in its input directory",
    );
    let key = file.signing_key().ok_or("generated seed did not decode")?;
    std::fs::create_dir_all(data_dir)?;
    write_private(&path, serde_json::to_string_pretty(&file)?.as_bytes())?;
    eprintln!(
        "emem-airgap  new node identity {} written to {}",
        file.pubkey8,
        path.display()
    );
    Ok((key, file))
}

#[cfg(unix)]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)
}

#[cfg(not(unix))]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

/// Fill `buf` with OS randomness, without adding a dependency for it.
fn getrandom(buf: &mut [u8]) -> std::io::Result<()> {
    use std::io::Read;
    let mut f = std::fs::File::open("/dev/urandom")?;
    f.read_exact(buf)
}

/// A shape check for RFC 3339 UTC, without pulling in a date library.
///
/// Deliberately narrow: `YYYY-MM-DDTHH:MM:SSZ`. A node that writes exactly one
/// timestamp format is easier to reason about than one accepting every legal
/// spelling, and whoever supplies it already knows the shape.
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

  --input        <dir>   payloads arrive here; never modified
  --output       <dir>   custody records are written here
  --profile      <id>    substrate profile this node writes under
  --platform     <id>    device platform id
  --observed-at  <ts>    RFC 3339 UTC; required, never defaulted to now
  --data         <dir>   where node_identity.json lives (default: .)
  --max-payload-bytes <n> refuse payloads larger than this (default 256 MiB)
  --max-files    <n>     most files in one run (default 10000)

Each flag also reads an environment variable: EMEM_AIRGAP_INPUT, _OUTPUT,
_PROFILE, _PLATFORM, _OBSERVED_AT, _DATA.

Writes one <name>.custody.json per payload plus run.json. The payload itself
never leaves: only the record does.";
