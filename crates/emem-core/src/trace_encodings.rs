//! Trace-encodings registry — loaded from the **content-addressed
//! trace-encodings manifest**.
//!
//! A trace segment names an `encoding` (`linux.ftrace.v1`, `ros2.bag.v2`,
//! `zephyr.ctf.v1`, ...). The verifier reads that string; this registry is
//! what gives it meaning. It is the **trace of the trace**: a trace attests
//! the device's execution, but the tracer is itself software running on the
//! device, so the registry records, per encoding:
//!
//! - the **toolchain** that produces it,
//! - the trace **layers** it can legitimately capture (a tracer that only
//!   sees the syscall stream cannot vouch for a thermal reading), and
//! - the tracer's own **integrity** ([`ToolchainIntegrity`]): where the
//!   trust in the capture tool itself comes from. `InKernel` is strongest —
//!   the tracer is part of the measured kernel/firmware, so the platform's
//!   boot measurement already covers it. A `VendorRuntime` is weakest —
//!   proprietary, trusted only through the platform's vendor attestation.
//!
//! This is the device-side analogue of the algorithms registry: algorithms
//! name the deterministic transforms an EO value may be derived by; this
//! names the capture toolchains a trace segment may be produced by. Every
//! `recognized_encodings` entry in the device-platforms manifest must
//! appear here (a cross-manifest invariant asserted in tests), and the
//! write gate rejects a segment naming an unregistered encoding or a layer
//! its encoding cannot produce.

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use crate::manifest::{Manifest, ManifestError, MANIFEST_TRACE_ENCODINGS};
use crate::substrates::{ProfileStatus, TraceLayerKind};

const TRACE_ENCODINGS_V0_JSON: &str = include_str!("../data/trace-encodings-v0.json");

/// Where the trust in the capture tool itself comes from — the integrity
/// of the tracer, distinct from the integrity of the traced device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolchainIntegrity {
    /// The tracer is part of the measured kernel or firmware, so the
    /// platform's boot measurement already attests it. Strongest.
    InKernel,
    /// A signed userspace capture binary whose hash is measured into the
    /// platform's reference values.
    SignedUserspace,
    /// An open-source userspace tool: auditable and reproducible-buildable,
    /// but its running integrity depends on the platform, not on itself.
    OpenSourceUserspace,
    /// A proprietary vendor runtime, trusted only through the platform's
    /// vendor attestation. Weakest; you cannot read its source.
    VendorRuntime,
}

impl ToolchainIntegrity {
    /// Monotonic rank, higher is stronger. Mirrors the provenance
    /// trust-rank pattern: measured-in-firmware beats signed beats
    /// open-source-but-userspace beats proprietary.
    pub fn rank(self) -> u8 {
        match self {
            Self::InKernel => 3,
            Self::SignedUserspace => 2,
            Self::OpenSourceUserspace => 1,
            Self::VendorRuntime => 0,
        }
    }
}

/// The OS class a capture encoding belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceOsClass {
    /// Linux kernel and userspace.
    Linux,
    /// A real-time operating system (Zephyr, FreeRTOS, ...).
    Rtos,
    /// Android.
    Android,
    /// Apple iOS / macOS.
    Ios,
    /// ROS 2 middleware over a host OS.
    Ros,
}

/// One recognized capture encoding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEncoding {
    /// Encoding ID, matching the `encoding` string a trace segment carries
    /// (e.g. `"linux.ftrace.v1"`).
    pub id: String,
    /// Human-readable title.
    pub title: String,
    /// The toolchain that produces this encoding.
    pub toolchain: String,
    /// OS class the encoding belongs to.
    pub os_class: TraceOsClass,
    /// Trace layers this encoding can legitimately capture. A segment
    /// naming this encoding but a layer not in this list is inconsistent.
    pub layers: Vec<TraceLayerKind>,
    /// How the capture tool's own integrity is established.
    pub integrity: ToolchainIntegrity,
    /// Whether the capture tool is open source (auditable).
    #[serde(default)]
    pub open_source: bool,
    /// Lifecycle state.
    pub status: ProfileStatus,
    /// Editorial note.
    #[serde(default, rename = "_note", skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl TraceEncoding {
    /// Whether this encoding can produce the given layer.
    pub fn can_capture(&self, layer: TraceLayerKind) -> bool {
        self.layers.contains(&layer)
    }
}

/// The trace-encodings manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEncodingRegistry {
    /// MUST equal `"emem-trace-encodings"`.
    pub manifest: String,
    /// Version, e.g. `"v0"`.
    pub version: String,
    /// Encoding entries.
    pub encodings: Vec<TraceEncoding>,
    /// Editorial note.
    #[serde(default, rename = "_note", skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Manifest for TraceEncodingRegistry {
    const KIND: &'static str = MANIFEST_TRACE_ENCODINGS;

    fn validate(&self) -> Result<(), ManifestError> {
        if self.manifest != Self::KIND {
            return Err(ManifestError::WrongKind {
                expected: Self::KIND,
                actual: self.manifest.clone(),
            });
        }
        let mut seen: std::collections::HashSet<&str> = Default::default();
        for e in &self.encodings {
            if !seen.insert(&e.id) {
                return Err(ManifestError::Invalid(format!(
                    "duplicate trace encoding: {}",
                    e.id
                )));
            }
            if e.layers.is_empty() {
                return Err(ManifestError::Invalid(format!(
                    "{}: an encoding must capture at least one layer",
                    e.id
                )));
            }
            if e.toolchain.trim().is_empty() {
                return Err(ManifestError::Invalid(format!(
                    "{}: an encoding must name its toolchain",
                    e.id
                )));
            }
        }
        Ok(())
    }
}

impl TraceEncodingRegistry {
    /// Embedded v0 default.
    pub fn parse_default() -> Result<Self, ManifestError> {
        Self::parse_json(TRACE_ENCODINGS_V0_JSON.as_bytes())
    }

    /// Look up an encoding by ID.
    pub fn lookup(&self, id: &str) -> Option<&TraceEncoding> {
        self.encodings.iter().find(|e| e.id == id)
    }

    /// Whether an encoding string is recognized.
    pub fn recognizes(&self, id: &str) -> bool {
        self.lookup(id).is_some()
    }
}

/// Process-wide cached default registry.
pub static DEFAULT: LazyLock<TraceEncodingRegistry> = LazyLock::new(|| {
    TraceEncodingRegistry::parse_default().expect("embedded trace-encodings-v0.json is malformed")
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_loads_and_has_ftrace() {
        let r = &*DEFAULT;
        let f = r.lookup("linux.ftrace.v1").expect("ftrace");
        assert_eq!(f.os_class, TraceOsClass::Linux);
        assert_eq!(f.integrity, ToolchainIntegrity::InKernel);
        assert!(f.can_capture(TraceLayerKind::Syscall));
        assert!(!f.can_capture(TraceLayerKind::Thermal));
    }

    #[test]
    fn recognizes_the_seed_vocabulary() {
        for id in [
            "linux.ftrace.v1",
            "linux.ebpf.raw",
            "zephyr.ctf.v1",
            "ros2.bag.v2",
            "nvidia.nsys.v1",
            "android.perfetto.v1",
            "apple.os_signpost.v1",
        ] {
            assert!(DEFAULT.recognizes(id), "missing encoding {id}");
        }
        assert!(!DEFAULT.recognizes("no.such.encoding"));
    }

    #[test]
    fn in_kernel_outranks_vendor_runtime() {
        assert!(ToolchainIntegrity::InKernel.rank() > ToolchainIntegrity::VendorRuntime.rank());
    }

    #[test]
    fn duplicate_encoding_is_rejected() {
        let mut r = DEFAULT.clone();
        let dup = r.encodings[0].clone();
        r.encodings.push(dup);
        assert!(r.validate().is_err());
    }

    #[test]
    fn manifest_cid_is_stable_across_reparse() {
        let a = crate::manifest::manifest_cid(&*DEFAULT).expect("cid");
        let b =
            crate::manifest::manifest_cid(&TraceEncodingRegistry::parse_default().expect("parse"))
                .expect("cid");
        assert_eq!(a, b);
    }
}
