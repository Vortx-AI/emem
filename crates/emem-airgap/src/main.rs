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
use emem_airgap::{decode_dir, key_path, DecodeSettings, NodeIdentity};

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
    let data_dir = PathBuf::from(
        env_or("--data", "EMEM_AIRGAP_DATA", &args).unwrap_or_else(|| ".".to_string()),
    );

    let key = load_or_create_key(&data_dir)?;
    let node_key = data_encoding::BASE32_NOPAD
        .encode(key.verifying_key().as_bytes())
        .to_lowercase();

    let settings = DecodeSettings {
        input: PathBuf::from(&input),
        output: PathBuf::from(&output),
        node: NodeIdentity {
            node_key: node_key.clone(),
            profile,
            platform,
        },
        observed_at,
    };

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

/// Load the node key, or make one on first run.
///
/// Written 0600 and never printed. The public half is printed, because the
/// operator needs it to endorse this node.
fn load_or_create_key(
    data_dir: &std::path::Path,
) -> Result<SigningKey, Box<dyn std::error::Error>> {
    let path = key_path(data_dir);
    if path.exists() {
        let s = std::fs::read_to_string(&path)?;
        let raw = data_encoding::BASE32_NOPAD.decode(s.trim().to_uppercase().as_bytes())?;
        let bytes: [u8; 32] = raw
            .as_slice()
            .try_into()
            .map_err(|_| "node.secret.b32 is not a 32-byte key")?;
        return Ok(SigningKey::from_bytes(&bytes));
    }
    // No OS RNG dependency beyond what ed25519-dalek already needs.
    let mut seed = [0u8; 32];
    getrandom(&mut seed)?;
    let key = SigningKey::from_bytes(&seed);
    std::fs::create_dir_all(data_dir)?;
    let enc = data_encoding::BASE32_NOPAD.encode(&seed).to_lowercase();
    write_private(&path, enc.as_bytes())?;
    eprintln!("emem-airgap  new node key written to {}", path.display());
    Ok(key)
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

const HELP: &str = "\
emem-airgap  one directory in, one directory out, no network.

  --input        <dir>   payloads arrive here; never modified
  --output       <dir>   custody records are written here
  --profile      <id>    substrate profile this node writes under
  --platform     <id>    device platform id
  --observed-at  <ts>    RFC 3339 UTC; required, never defaulted to now
  --data         <dir>   where node.secret.b32 lives (default: .)

Each flag also reads an environment variable: EMEM_AIRGAP_INPUT, _OUTPUT,
_PROFILE, _PLATFORM, _OBSERVED_AT, _DATA.

Writes one <name>.custody.json per payload plus run.json. The payload itself
never leaves: only the record does.";
