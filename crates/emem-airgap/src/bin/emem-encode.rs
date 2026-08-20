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

use std::path::{Path, PathBuf};

use ed25519_dalek::SigningKey;
use emem_airgap::{capture_window, key_path, CaptureSettings, NodeKeyFile, StreamHead};

/// Every flag the encoder accepts. An unknown one is refused rather than
/// ignored: `--window-ms 300` was accepted in silence by a binary that has no
/// such flag, and the run reported success having applied nothing.
const ENCODE_FLAGS: &[&str] = &[
    "--help",
    "--out",
    "--data",
    "--profile",
    "--platform",
    "--payloads",
    "--prev-trace",
    "--interval",
    "--max-depth",
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
    emem_airgap::reject_unknown_flags(&args, ENCODE_FLAGS)?;

    let out = env_or("--out", "EMEM_ENCODE_OUT", &args)
        .ok_or("--out (or EMEM_ENCODE_OUT) is required: where traces are written")?;
    let profile = env_or("--profile", "EMEM_ENCODE_PROFILE", &args)
        .ok_or("--profile (or EMEM_ENCODE_PROFILE) is required")?;
    let platform = env_or("--platform", "EMEM_ENCODE_PLATFORM", &args)
        .ok_or("--platform (or EMEM_ENCODE_PLATFORM) is required")?;
    let data_dir = PathBuf::from(
        env_or("--data", "EMEM_ENCODE_DATA", &args).unwrap_or_else(|| ".".to_string()),
    );

    // Supplied in the environment, if the host works that way.
    //
    // The same need as the decoder's: a platform may give this node no
    // writable mount, and the encoder cannot read an identity file that was
    // never allowed to exist. Checked first so a node provisioned this way
    // never looks at --data at all.
    let supplied = emem_airgap::seed_from_environment()?;

    // Otherwise shared with the decoder, and never created here. Both halves
    // must speak for the same node, and a sidecar that minted its own key on
    // first run would quietly split one node into two.
    let path = key_path(&data_dir);
    if supplied.is_none() && !path.exists() {
        return Err(format!(
            "no node identity at {}, and no seed in the environment.\n\n\
             The encoder shares the decoder's identity and does not create one: two halves of \
             one node must sign as one node, and a key minted here would split it in two.\n\n\
             Two ways to give it one:\n  \
             1. EMEM_ENCODE_SEED_HEX (or EMEM_AIRGAP_SEED_HEX, same thing), 64 hex characters \
             as `emem-airgap keygen --print-seed` prints them. Nothing is written, and no \
             --data directory is needed.\n  \
             2. Run the decoder once against the same --data directory, which creates the file \
             this was looking for.",
            path.display()
        )
        .into());
    }
    // The identity, from wherever this node keeps it.
    let file: NodeKeyFile = match supplied {
        Some(f) => f,
        None => {
            let raw = std::fs::read_to_string(&path).map_err(|e| {
                if e.kind() == std::io::ErrorKind::PermissionDenied {
                    format!(
                        "cannot read {}: permission denied. The identity is mode 600 and owned \
                         by the uid that created it; run this sidecar as that same user, or \
                         supply the identity as EMEM_AIRGAP_SEED_HEX so no file is needed.",
                        path.display()
                    )
                } else {
                    format!("cannot read {}: {e}", path.display())
                }
            })?;
            serde_json::from_str(&raw)?
        }
    };
    let key: SigningKey = file
        .signing_key()
        .ok_or("node identity seed is not 32 bytes of hex")?;

    // Where the chain got to. An explicit --prev-trace overrides it, for an
    // operator splicing a stream by hand; otherwise the head on disk is the
    // answer, and a head from an earlier boot is correctly ignored.
    // Same flag, same default, same walk as the decoder.
    let max_depth = env_or("--max-depth", "EMEM_ENCODE_MAX_DEPTH", &args)
        .and_then(|v| v.parse().ok())
        .unwrap_or(emem_airgap::DEFAULT_MAX_DEPTH);

    let head_path = StreamHead::path(Path::new(&out));
    let explicit_prev = env_or("--prev-trace", "EMEM_ENCODE_PREV_TRACE", &args);
    let mut head = StreamHead::load_for_this_boot(&head_path);
    if head.is_none() && head_path.exists() {
        eprintln!(
            "emem-encode  a stream head exists from an earlier boot; starting a fresh stream, \
             which is what a reboot means"
        );
    }

    let interval =
        env_or("--interval", "EMEM_ENCODE_INTERVAL", &args).and_then(|v| v.parse::<u64>().ok());

    eprintln!("emem-encode  node {}", file.pubkey8);
    if let Some(secs) = interval {
        eprintln!(
            "  streaming every {secs}s; stop it with SIGTERM. A window in flight is written \
             atomically, so there is nothing half-finished to clean up."
        );
    }

    let payloads = env_or("--payloads", "EMEM_ENCODE_PAYLOADS", &args).map(PathBuf::from);
    let mut windows_done = 0u64;
    loop {
        let settings = CaptureSettings {
            out: PathBuf::from(&out),
            payloads: payloads.clone(),
            max_depth,
            profile: profile.clone(),
            platform: platform.clone(),
            // The first window of a run may take an explicit override; after
            // that the chain is its own authority.
            prev_trace_cid: if windows_done == 0 {
                explicit_prev
                    .clone()
                    .or_else(|| head.as_ref().map(|h| h.trace_cid.clone()))
            } else {
                head.as_ref().map(|h| h.trace_cid.clone())
            },
        };

        let report = capture_window(&key, &settings)?;
        match &report.trace_cid {
            Some(cid) => {
                eprintln!("  trace {cid}");
                let next = StreamHead {
                    boot_id: emem_airgap::boot_id(),
                    trace_cid: cid.clone(),
                    windows: head.as_ref().map(|h| h.windows).unwrap_or(0) + 1,
                };
                // After the trace it names, never before: see StreamHead::save.
                next.save(&head_path)?;
                head = Some(next);
            }
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
            eprintln!("  absent    {:<11} {}", m.layer, m.reason);
        }
        // Printed once, on the first window only: it never changes, and
        // repeating it every minute would train the operator to stop reading.
        if windows_done == 0 {
            for m in &report.unsupported {
                eprintln!("  no source {:<11} {}", m.layer, m.reason);
            }
        }
        eprintln!("  outputs   {} payload digest(s) bound", report.outputs);
        windows_done += 1;

        match interval {
            None => {
                eprintln!("  {}", report.admissibility);
                println!("{}", serde_json::to_string_pretty(&report)?);
                return Ok(());
            }
            Some(secs) => std::thread::sleep(std::time::Duration::from_secs(secs.max(1))),
        }
    }
}

const HELP: &str = "\
emem-encode  capture one OS-trace window, sign it, write it to a folder.

  --out       <dir>   where the trace is written; the decoder mounts this too
  --payloads  <dir>   files this window produced; their digests are bound in
  --profile   <id>    substrate profile this device writes under
  --platform  <id>    hardware platform string, e.g. jetson-orin-nx
  --data      <dir>   node_identity.json, SHARED with the decoder (default: .)
  --prev-trace <cid>  the previous trace in this stream, to chain the windows
  --interval  <secs>  capture every <secs> seconds instead of once and exiting;
                      each window chains to the one before it
  --max-depth <n>     how deep to descend into --payloads (default 32), the
                      same walk the decoder runs over --input

The identity is SHARED with the decoder and never created here. Supply it as
EMEM_ENCODE_SEED_HEX (or EMEM_AIRGAP_SEED_HEX), 64 hex characters, and no
--data directory is needed at all; EMEM_ENCODE_SEED_FILE reads the same from a
path. Otherwise --data must hold the node_identity.json the decoder wrote.

Stream state goes in <--out>/.state/, not --data, so an encoder on a read-only
host can still record where its chain got to.

Each flag also reads an environment variable: EMEM_ENCODE_OUT, _PAYLOADS,
_PROFILE, _PLATFORM, _DATA, _PREV_TRACE, _INTERVAL. Either spelling works,
with a space or an equals sign. A flag this command does not have is refused
rather than ignored, so a typo stops the run instead of quietly changing it.

Only the syscall layer needs a tracefs mount and the capability to read it.
Scheduler, memory, storage and network come from /proc and need neither, as
counter segments labelled linux.procfs.v1 rather than event logs. Energy and
thermal come from hwmon. A layer that cannot be read is ABSENT from the trace
and reported, never invented, and every capture prints which profiles the
layers it did get would satisfy.";

#[cfg(test)]
mod tests {
    use super::*;

    /// The encoder's help text must describe every flag it accepts.
    ///
    /// It did not. `--interval` is what turns a one-shot capture into the
    /// sidecar this crate is built around, it is accepted, it is in the
    /// README, and `--help` never mentioned it. A developer discovering the
    /// tool through its own help could not find the mode it exists for.
    #[test]
    fn the_help_text_and_the_accepted_flags_agree() {
        for flag in ENCODE_FLAGS {
            if *flag == "--help" {
                continue;
            }
            assert!(
                HELP.contains(flag),
                "{flag} is accepted but the help text never mentions it"
            );
        }
        for line in HELP.lines() {
            // The options block is indented two spaces; prose that mentions a
            // flag is not an offer of one.
            let Some(rest) = line.strip_prefix("  --") else {
                continue;
            };
            let name = format!(
                "--{}",
                rest.split([' ', '\t', '=']).next().unwrap_or_default()
            );
            if name == "--" {
                continue;
            }
            assert!(
                ENCODE_FLAGS.contains(&name.as_str()),
                "the help text offers {name} but the parser refuses it"
            );
        }
    }

    /// And the README must name them too.
    #[test]
    fn the_readme_names_every_flag_the_encoder_accepts() {
        const README: &str = include_str!("../../README.md");
        for flag in ENCODE_FLAGS {
            if *flag == "--help" {
                continue;
            }
            assert!(
                README.contains(flag),
                "{flag} is accepted but crates/emem-airgap/README.md never names it"
            );
        }
    }
}
