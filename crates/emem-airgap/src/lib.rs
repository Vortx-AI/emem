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

use serde::{Deserialize, Serialize};

mod custody;
mod run;

pub use custody::{Custody, CustodyError, CustodyVerdict, CUSTODY_SCHEMA_V1};
pub use run::{decode_dir, key_path, DecodeReport, DecodeSettings, Skipped};

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
