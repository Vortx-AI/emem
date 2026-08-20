//! An emem node for a machine with no route out.
//!
//! # What this is for
//!
//! A container runs on hardware somebody else owns. Files arrive in one
//! directory. The container reads them, and whatever it writes to a second
//! directory is what leaves. There is no network, no database, and no chance
//! to ask a server anything.
//!
//! That is the whole environment this crate targets, and it is why the crate
//! is separate rather than a feature flag on the server: the dependency list
//! in `Cargo.toml` is the argument. No HTTP, no sled, no Lance, no ONNX, no
//! async runtime.
//!
//! # What it produces, and what it refuses to claim
//!
//! For each payload that arrives, the node signs a [`Custody`] record: these
//! bytes, under this name, at this size, arrived here at this time, and the
//! holder of this key says so.
//!
//! It does **not** produce an `emem.os_trace.v1`. That was the first thing
//! checked rather than assumed, and the result decided the design: an OS trace
//! asserts *verified execution*, its verifier rejects an empty segment set,
//! and the satellite substrate profile requires eight trace layers. A decoder
//! running on its own captures none of them. Emitting a trace here would mean
//! fabricating eight layers of evidence to satisfy a schema, which is the one
//! thing a provenance protocol must never do.
//!
//! So custody is the honest claim, and it is a deliberately weaker one. It
//! travels under its own preimage domain (`emem_attest::custody_preimage_v1`)
//! so no verifier can mistake it for execution evidence, and the record says
//! in its own body what it does and does not establish.
//!
//! When the OS encoder ships on the same machine, the same payload gains a
//! trace and rises to attested execution. Custody is the floor, not the
//! ceiling.
//!
//! # Running it where it is meant to run
//!
//! ```text
//! docker run --network none --read-only --cap-drop ALL \
//!            --security-opt no-new-privileges --user 65532:65532 \
//!            -v /host/in:/in:ro -v /host/out:/out -v /host/data:/data \
//!            emem-airgap --input /in --output /out --data /data ...
//! ```
//!
//! `--network none` is the flag that matters, and it agrees with the build:
//! this crate links no networking dependency, so there is nothing here that
//! could open a socket even if the namespace allowed one. Mounting the input
//! read-only means the host does not have to take the node's word that it
//! leaves that directory alone.
//!
//! # What it survives
//!
//! The failure modes here are not only adversarial. A bus browns out mid-write;
//! a single-event upset flips a bit in flash; a directory arrives with a
//! million files in it. So records are written to a temporary and renamed,
//! which is atomic, and fsynced before and after, because a rename that only
//! reached the page cache did not survive the power cut that made you care.
//! Each record is then read back and re-verified from disk, since the node is
//! the last party that can notice corruption while there is still a second
//! copy of the payload to check against.

use serde::{Deserialize, Serialize};

mod custody;
mod encode;
mod identity;
pub(crate) mod run;

pub use custody::{
    Custody, CustodyError, CustodyVerdict, ASSURANCE, ASSURANCE_TRACED, CUSTODY_SCHEMA_V1,
};
pub use encode::{
    boot_id, capture_window, CaptureReport, CaptureSettings, MissedLayer, StreamHead,
};
pub use identity::{JoinRequest, NodeKeyFile, JOIN_REQUEST_SCHEMA_V1};
pub use run::{
    compact_time, decode_dir, key_path, record_path_for, short_key, DecodeReport, DecodeSettings,
    Found, Skipped, DEFAULT_MAX_DEPTH, DEFAULT_MAX_FILES, DEFAULT_MAX_PAYLOAD_BYTES,
    DEFAULT_MAX_TRACE_BYTES,
};

/// base32-nopad lowercase of a blake3 digest: the encoding every emem digest
/// uses, kept here so this crate does not need the codec crate for one call.
pub(crate) fn b32(bytes: &[u8]) -> String {
    data_encoding::BASE32_NOPAD.encode(bytes).to_lowercase()
}

/// What a node says about itself. Not a claim anyone has checked: it is what
/// the operator configured, recorded so a reader knows which machine and
/// which profile the custody record was written under.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeIdentity {
    /// The node's ed25519 public key, base32-nopad lowercase.
    pub node_key: String,
    /// Substrate profile this node writes under, e.g. `orbital.satellite.v1`.
    pub profile: String,
    /// Device platform id from the device-platform registry, e.g.
    /// `nvidia.jetson-orin`.
    pub platform: String,
}

/// Refuse a flag this binary does not have.
///
/// A misspelled flag used to be ignored in silence. `--window-ms 300` passed to
/// a binary with no such flag ran happily with the default and reported
/// success, so the operator had every reason to believe they had configured
/// something they had not. On hardware nobody can log into, a setting that
/// silently did not apply is worse than a run that refused to start: the run
/// that refuses gets fixed, and this one gets trusted.
///
/// Checked against the flags the binary actually accepts rather than a
/// hand-kept list, so a flag added later cannot fall out of this by omission.
pub fn reject_unknown_flags(args: &[String], known: &[&str]) -> Result<(), std::io::Error> {
    let mut skip_value = false;
    for (i, a) in args.iter().enumerate().skip(1) {
        if skip_value {
            skip_value = false;
            continue;
        }
        if !a.starts_with("--") {
            continue;
        }
        // `--flag=value` names the flag before the equals sign.
        let name = a.split_once('=').map_or(a.as_str(), |(n, _)| n);
        if known.contains(&name) {
            skip_value = !a.contains('=') && args.get(i + 1).is_some_and(|v| !v.starts_with("--"));
            continue;
        }
        let near: Vec<&str> = known
            .iter()
            .copied()
            .filter(|k| {
                let (a, b) = (k.trim_start_matches('-'), name.trim_start_matches('-'));
                a.starts_with(b) || b.starts_with(a) || a.contains(b) || b.contains(a)
            })
            .collect();
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            if near.is_empty() {
                format!("{name} is not a flag this command has. Run --help for the ones it does.")
            } else {
                format!(
                    "{name} is not a flag this command has. Did you mean {}? Run --help for all \
                     of them.",
                    near.join(" or ")
                )
            },
        ));
    }
    Ok(())
}

/// An identity handed to this node rather than stored by it.
///
/// `EMEM_AIRGAP_SEED_HEX` is 64 hex characters, the raw ed25519 seed. A
/// `--seed-file` path is read instead when the platform can mount a secret
/// read-only but cannot set an environment variable; the file holds the same
/// 64 characters and nothing else.
///
/// Neither is written anywhere. A node running this way needs no writable
/// directory for its identity at all, which is the point: the alternative on a
/// two-mount platform was to put the private seed in the folder that gets
/// downlinked.
pub fn seed_from_environment() -> Result<Option<NodeKeyFile>, Box<dyn std::error::Error>> {
    // Both prefixes, for both binaries.
    //
    // The variable was EMEM_AIRGAP_SEED_HEX only, so an operator configuring
    // the sidecar reached for EMEM_ENCODE_SEED_HEX to match its other
    // variables (EMEM_ENCODE_OUT, _PROFILE, _DATA) and it did nothing at all.
    // Reported from a real deployment, found by elimination, which is a long
    // way to travel for a name. One identity, either spelling.
    let mut raw = None;
    for var in ["EMEM_AIRGAP_SEED_HEX", "EMEM_ENCODE_SEED_HEX"] {
        if let Ok(v) = std::env::var(var) {
            raw = Some(v);
            break;
        }
    }
    if raw.is_none() {
        for var in ["EMEM_AIRGAP_SEED_FILE", "EMEM_ENCODE_SEED_FILE"] {
            if let Ok(p) = std::env::var(var) {
                raw = Some(
                    std::fs::read_to_string(&p)
                        .map_err(|e| format!("cannot read the seed file at {p}: {e}"))?,
                );
                break;
            }
        }
    }
    let Some(raw) = raw else {
        return Ok(None);
    };
    let hex = raw.trim();
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!(
            "the supplied seed is {} characters; it must be exactly 64 hex characters, the raw \
             ed25519 seed as `keygen --print-seed` prints it.",
            hex.len()
        )
        .into());
    }
    let mut seed = [0u8; 32];
    for (i, b) in seed.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)?;
    }
    Ok(Some(NodeKeyFile::new(
        seed,
        "supplied",
        "emem air-gapped node: identity supplied in the environment, never written to disk",
    )))
}
