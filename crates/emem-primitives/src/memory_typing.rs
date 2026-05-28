//! Memory typing — the de facto agent-memory ontology (CoALA, LangMem,
//! MIRIX) splits durable agent memory into four kinds. `emem` adopts the
//! same vocabulary on top of the Anthropic memory-tool surface so an
//! agent can tag what each file *is*, not just where it lives.
//!
//! | kind        | shape                                                  |
//! |-------------|--------------------------------------------------------|
//! | `episodic`  | observations of events ("agent X recalled NDVI at Y") |
//! | `semantic`  | durable learned facts ("Mato Grosso is tropical")     |
//! | `procedural`| how-to / playbooks ("for flood risk, call /v1/water") |
//! | `resource`  | generic durable scratchpad (default; back-compat)     |
//!
//! The kind is carried inline on every `MemoryFileMeta` so the file blob
//! stays content-addressed (kind isn't part of the CID — same bytes
//! re-tagged still hash the same).
//!
//! The TTL pass (`memory_consolidation`) uses kind to pick a per-kind
//! retention window. The consolidation pass turns runs of episodic
//! observations into a single semantic summary file.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

/// The four canonical memory kinds. `Resource` is the default so old
/// callers that never set a kind keep working — their files deserialise
/// with `kind: "resource"` via the serde default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryKind {
    /// Observations of events. Default TTL 30 d (configurable).
    Episodic,
    /// Durable learned facts. Default TTL infinite.
    Semantic,
    /// How-to / playbooks. Default TTL infinite.
    Procedural,
    /// Generic durable scratchpad. Default TTL 90 d.
    #[default]
    Resource,
}

impl MemoryKind {
    /// Parse from the lowercase wire form used in REST/MCP. Anything
    /// unrecognised returns `None`; callers usually fall back to
    /// `Resource` so an over-eager agent doesn't 400 on a typo.
    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "episodic" => Some(Self::Episodic),
            "semantic" => Some(Self::Semantic),
            "procedural" => Some(Self::Procedural),
            "resource" => Some(Self::Resource),
            _ => None,
        }
    }

    /// Lowercase wire form.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Episodic => "episodic",
            Self::Semantic => "semantic",
            Self::Procedural => "procedural",
            Self::Resource => "resource",
        }
    }

    /// Default TTL in days. `0` = infinite. Mirrored by the env
    /// overrides documented in [`ttl_days_for_kind`].
    pub fn default_ttl_days(&self) -> u64 {
        match self {
            Self::Resource => 90,
            Self::Episodic => 30,
            Self::Semantic => 0,
            Self::Procedural => 0,
        }
    }
}

/// Resolve the effective TTL (in days) for a memory kind, consulting
/// the per-kind env override before falling back to the built-in
/// default.
///
/// Recognised env vars:
/// - `EMEM_MEMORY_TTL_RESOURCE_DAYS`
/// - `EMEM_MEMORY_TTL_EPISODIC_DAYS`
/// - `EMEM_MEMORY_TTL_SEMANTIC_DAYS`
/// - `EMEM_MEMORY_TTL_PROCEDURAL_DAYS`
///
/// Returning `0` means "infinite — never expire".
pub fn ttl_days_for_kind(kind: MemoryKind) -> u64 {
    let key = match kind {
        MemoryKind::Resource => "EMEM_MEMORY_TTL_RESOURCE_DAYS",
        MemoryKind::Episodic => "EMEM_MEMORY_TTL_EPISODIC_DAYS",
        MemoryKind::Semantic => "EMEM_MEMORY_TTL_SEMANTIC_DAYS",
        MemoryKind::Procedural => "EMEM_MEMORY_TTL_PROCEDURAL_DAYS",
    };
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or_else(|| kind.default_ttl_days())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_round_trip() {
        for k in [
            MemoryKind::Episodic,
            MemoryKind::Semantic,
            MemoryKind::Procedural,
            MemoryKind::Resource,
        ] {
            assert_eq!(MemoryKind::from_wire(k.as_str()), Some(k));
        }
        assert!(MemoryKind::from_wire("garbage").is_none());
    }

    #[test]
    fn default_is_resource() {
        assert_eq!(MemoryKind::default(), MemoryKind::Resource);
    }

    #[test]
    fn defaults_match_spec() {
        assert_eq!(MemoryKind::Resource.default_ttl_days(), 90);
        assert_eq!(MemoryKind::Episodic.default_ttl_days(), 30);
        assert_eq!(MemoryKind::Semantic.default_ttl_days(), 0);
        assert_eq!(MemoryKind::Procedural.default_ttl_days(), 0);
    }
}
