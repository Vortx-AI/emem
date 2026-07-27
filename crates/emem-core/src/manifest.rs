//! Generic manifest loader. The protocol's "registries" (bands, functions,
//! sources, schema, lcv-1 taxonomy, cell64 alphabet) are all CONTENT-ADDRESSED
//! manifests, not Rust constants. Implementations load them from CBOR bytes,
//! validate, and expose a `cid()` derived from the canonical bytes.
//!
//! Everything that varies across protocol versions or operator deployments
//! lives in a manifest. Code in this repo is the **protocol** (the loader,
//! validator, CID rule, primitive semantics) — never the data.

use blake3::Hasher;
use data_encoding::BASE32_NOPAD;
use serde::de::DeserializeOwned;

/// Identifier pinned in every manifest's top-level field.
pub const MANIFEST_BAND_ONTOLOGY: &str = "emem-bands";
/// Function-registry manifest identifier.
pub const MANIFEST_FUNCTION_REG: &str = "emem-functions";
/// Source-connector manifest identifier.
pub const MANIFEST_SOURCE_REG: &str = "emem-sources";
/// Schema (CDDL bundle) manifest identifier.
pub const MANIFEST_SCHEMA: &str = "emem-schema";
/// lcv-1 taxonomy manifest identifier.
pub const MANIFEST_LCV1: &str = "emem-lcv1";
/// cell64 alphabet manifest identifier.
pub const MANIFEST_CELL64_ALPHABET: &str = "emem-cell64-alphabet";
/// Algorithm registry manifest identifier — composition recipes that
/// fuse multiple band facts (and embeddings) into derived scores.
pub const MANIFEST_ALGORITHM_REG: &str = "emem-algorithms";
/// Topic registry manifest identifier — keywords / aliases / descriptions
/// used to route a free-text question to a topic, plus the bands and
/// algorithms each topic concerns. Replaces the pre-0.0.3 hardcoded
/// `TOPIC_KEYWORDS` / `TOPIC_BANDS` / `TOPIC_ALGORITHMS` tables.
pub const MANIFEST_TOPIC_REG: &str = "emem-topics";
/// Substrate profile registry manifest identifier — the written profile
/// every contributor class (satellite archive, telescope, microscope,
/// CCTV, mobile, robot, industrial machine) must match before its
/// observations are admitted, including the admission rule (full OS
/// execution trace, or recomputability from a cited open archive) and
/// the trace layers that rule requires.
pub const MANIFEST_SUBSTRATE_REG: &str = "emem-substrates";
/// Device-platform whitelist manifest identifier — which hardware
/// platforms may enrol a key under a trace-admitted substrate, and the
/// root-of-trust evidence (TCG DICE, IEEE 802.1AR DevID, TPM 2.0 quote,
/// Arm PSA / EAT) that turns a self-minted key into an attested device
/// identity. The device-side analogue of the sources registry: sources
/// says where an archive's bytes come from; this says which physical
/// platforms the protocol will believe and how they prove it.
pub const MANIFEST_DEVICE_PLATFORMS: &str = "emem-device-platforms";
/// Trace-encodings registry manifest identifier — the capture encodings a
/// device may name in a trace segment (`linux.ftrace.v1`, `ros2.bag.v2`,
/// `zephyr.ctf.v1`, ...), the toolchain that produces each, the trace
/// layers it can capture, and how that tracer's own integrity is
/// established. The "trace of the trace": the device-side analogue of the
/// algorithms registry.
pub const MANIFEST_TRACE_ENCODINGS: &str = "emem-trace-encodings";

/// Errors that can occur loading or validating a manifest.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    /// Bytes were not valid CBOR or JSON.
    #[error("manifest decode failed: {0}")]
    Decode(String),
    /// Manifest's `manifest` field did not match the expected identifier.
    #[error("expected manifest {expected}, got {actual}")]
    WrongKind {
        /// The manifest kind the loader expected (e.g. `"emem-bands"`).
        expected: &'static str,
        /// The manifest kind actually present in the input bytes.
        actual: String,
    },
    /// Manifest version not supported by this loader.
    #[error("unsupported manifest version: {0}")]
    UnsupportedVersion(String),
    /// Internal validation rule (specific to manifest kind) failed.
    #[error("invalid manifest: {0}")]
    Invalid(String),
}

/// Trait every manifest loader implements.
///
/// The contract: given a byte slice (CBOR or JSON), `parse` validates and
/// returns an in-memory representation. The CID is computed deterministically
/// from the **canonical CBOR encoding** of the validated structure — so two
/// implementations parsing the same JSON converge on the same CID.
pub trait Manifest: Sized + DeserializeOwned {
    /// Stable identifier the manifest's `manifest` field MUST equal.
    const KIND: &'static str;

    /// Validate the structural invariants of this manifest. Called after
    /// deserialization but before exposing to callers.
    fn validate(&self) -> Result<(), ManifestError>;

    /// Parse from JSON bytes (used at startup with `include_str!`-style
    /// embedded defaults).
    fn parse_json(bytes: &[u8]) -> Result<Self, ManifestError> {
        let v: Self =
            serde_json::from_slice(bytes).map_err(|e| ManifestError::Decode(e.to_string()))?;
        v.validate()?;
        Ok(v)
    }

    /// Parse from canonical CBOR bytes.
    fn parse_cbor(bytes: &[u8]) -> Result<Self, ManifestError> {
        let v: Self =
            ciborium::de::from_reader(bytes).map_err(|e| ManifestError::Decode(e.to_string()))?;
        v.validate()?;
        Ok(v)
    }
}

/// Compute the manifest CID: `base32(blake3(canonical_cbor(manifest))[:32])`.
pub fn manifest_cid<M: serde::Serialize>(m: &M) -> Result<String, ManifestError> {
    let mut buf: Vec<u8> = Vec::new();
    ciborium::ser::into_writer(m, &mut buf).map_err(|e| ManifestError::Decode(e.to_string()))?;
    let mut h = Hasher::new();
    h.update(&buf);
    let hash = h.finalize();
    Ok(BASE32_NOPAD.encode(&hash.as_bytes()[..32]).to_lowercase())
}
