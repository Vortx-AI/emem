//! `emem-encode` — the OS-trace encoder, meant to run as a privileged sidecar
//! beside the decoder.
//!
//! It reads what the kernel will show it, signs a trace over exactly that, and
//! writes it to a directory the decoder also mounts. The two halves never talk;
//! the folder is the whole interface.
//!
//! ```bash
//! emem-encode --out /traces --payloads /opt/ilc_player/results \
//!             --profile orbital.satellite.v1 --platform jetson-orin-nx \
//!             --data /data
//! ```
//!
//! It shares the node identity with the decoder, so both halves speak for the
//! same node, and it never creates one: an identity conjured by a sidecar is a
//! key nobody meant to exist.

use std::path::PathBuf;

use ed25519_dalek::SigningKey;
use emem_airgap::{capture_window, key_path, CaptureSettings, NodeKeyFile};

fn env_or(flag: &str, var: &str, args: &[String]) -> Option<String> {
    if let Some(i) = args.iter().position(|a| a == flag) {
        return args.get(i + 1).cloned();
    }
    std::env::var(var).ok()
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("emem-encode: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        eprintln!("{HELP}");
        return Ok(());
    }

    let out = env_or("--out", "EMEM_ENCODE_OUT", &args)
        .ok_or("--out (or EMEM_ENCODE_OUT) is required: where traces are written")?;
    let profile = env_or("--profile", "EMEM_ENCODE_PROFILE", &args)
        .ok_or("--profile (or EMEM_ENCODE_PROFILE) is required")?;
    let platform = env_or("--platform", "EMEM_ENCODE_PLATFORM", &args)
        .ok_or("--platform (or EMEM_ENCODE_PLATFORM) is required")?;
    let data_dir = PathBuf::from(
        env_or("--data", "EMEM_ENCODE_DATA", &args).unwrap_or_else(|| ".".to_string()),
    );

    // Shared with the decoder, and never created here. Both halves must speak
    // for the same node, and a sidecar that minted its own key on first run
    // would quietly split one node into two.
    let path = key_path(&data_dir);
    if !path.exists() {
        return Err(format!(
            "no node identity at {}.\n\n\
             The encoder shares the decoder's identity and does not create one: two halves of \
             one node must sign as one node, and a key minted here would split it in two. Run \
             the decoder once against the same --data directory first.",
            path.display()
        )
        .into());
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            format!(
                "cannot read {}: permission denied. The identity is mode 600 and owned by the \
                 uid that created it; run this sidecar as that same user.",
                path.display()
            )
        } else {
            format!("cannot read {}: {e}", path.display())
        }
    })?;
    let file: NodeKeyFile = serde_json::from_str(&raw)?;
    let key: SigningKey = file
        .signing_key()
        .ok_or("node identity seed is not 32 bytes of hex")?;

    let settings = CaptureSettings {
        out: PathBuf::from(&out),
        payloads: env_or("--payloads", "EMEM_ENCODE_PAYLOADS", &args).map(PathBuf::from),
        profile,
        platform,
        prev_trace_cid: env_or("--prev-trace", "EMEM_ENCODE_PREV_TRACE", &args),
    };

    let report = capture_window(&key, &settings)?;
    eprintln!("emem-encode  node {}", file.pubkey8);
    match &report.trace_cid {
        Some(cid) => eprintln!("  trace {cid}"),
        None => eprintln!("  no trace written"),
    }
    eprintln!(
        "  captured  {}",
        if report.captured.is_empty() {
            "nothing".to_string()
        } else {
            report.captured.join(", ")
        }
    );
    for m in &report.missed {
        eprintln!("  absent    {:<10} {}", m.layer, m.reason);
    }
    eprintln!("  outputs   {} payload digest(s) bound", report.outputs);
    eprintln!("  {}", report.admissibility);
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

const HELP: &str = "\
emem-encode  capture one OS-trace window, sign it, write it to a folder.

  --out       <dir>   where the trace is written; the decoder mounts this too
  --payloads  <dir>   files this window produced; their digests are bound in
  --profile   <id>    substrate profile this device writes under
  --platform  <id>    hardware platform string, e.g. jetson-orin-nx
  --data      <dir>   node_identity.json, SHARED with the decoder (default: .)
  --prev-trace <cid>  the previous trace in this stream, to chain the windows

Each flag also reads an environment variable: EMEM_ENCODE_OUT, _PAYLOADS,
_PROFILE, _PLATFORM, _DATA, _PREV_TRACE.

Needs a tracefs mount and the capability to read it for the syscall, scheduler
and memory layers. Energy and thermal come from hwmon and need neither. A layer
that cannot be read is ABSENT from the trace and reported, never invented.";
