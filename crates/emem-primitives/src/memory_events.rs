//! SSE memory-event types.
//!
//! Every successful memory write publishes a [`MemoryEvent`] on a
//! process-global tokio broadcast channel. Subscribers may filter by
//! `path_prefix`, `kind`, or `attester_pubkey_b32` to receive only
//! events of interest. Events are NOT individually signed — the
//! underlying file's receipt remains the verification surface; SSE is
//! best-effort notification.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

/// One event in the memory write stream. The discriminator is `"type"`
/// so SSE clients can switch on the kebab-case tag without parsing the
/// envelope twice.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MemoryEvent {
    /// A new path appeared in the live index.
    Created {
        path: String,
        file_cid: String,
        kind: String,
        attester_pubkey_b32: Option<String>,
        signed_at: String,
        size_bytes: u64,
    },
    /// An existing path was rewritten (str_replace / insert / create
    /// overwrite). `prev_file_cid` is the immediately-previous CID.
    Modified {
        path: String,
        file_cid: String,
        prev_file_cid: Option<String>,
        kind: String,
        attester_pubkey_b32: Option<String>,
        signed_at: String,
    },
    /// A path was removed from the live index. The content-addressed
    /// blob still exists in `memory_file_blobs` (append-only).
    Deleted {
        path: String,
        kind: String,
        attester_pubkey_b32: Option<String>,
        deleted_at: String,
    },
    /// A path moved. `file_cid` is preserved across the rename — the
    /// bytes hash the same so the prior receipt still binds them.
    Renamed {
        old_path: String,
        new_path: String,
        file_cid: String,
        kind: String,
        attester_pubkey_b32: Option<String>,
    },
    /// The TTL pass moved a path out of the live index.
    Expired {
        path: String,
        kind: String,
        expired_at: String,
    },
    /// The consolidation pass merged N episodic files into one
    /// semantic summary. Originals point at the new CID via
    /// `superseded_by`.
    Consolidated {
        paths: Vec<String>,
        into_cid: String,
        kind: String,
        signed_at: String,
    },
}

impl MemoryEvent {
    /// The file path(s) this event touches, used by the SSE filter.
    pub fn primary_path(&self) -> &str {
        match self {
            Self::Created { path, .. } | Self::Modified { path, .. } => path,
            Self::Deleted { path, .. } | Self::Expired { path, .. } => path,
            Self::Renamed { new_path, .. } => new_path,
            Self::Consolidated { paths, .. } => paths.first().map(|s| s.as_str()).unwrap_or(""),
        }
    }

    /// Wire-form kind, lowercased ("episodic" / "semantic" / etc.).
    pub fn kind_str(&self) -> &str {
        match self {
            Self::Created { kind, .. }
            | Self::Modified { kind, .. }
            | Self::Deleted { kind, .. }
            | Self::Renamed { kind, .. }
            | Self::Expired { kind, .. }
            | Self::Consolidated { kind, .. } => kind,
        }
    }

    /// Optional attester pubkey associated with the write, if any.
    pub fn attester_pubkey(&self) -> Option<&str> {
        match self {
            Self::Created {
                attester_pubkey_b32,
                ..
            }
            | Self::Modified {
                attester_pubkey_b32,
                ..
            }
            | Self::Deleted {
                attester_pubkey_b32,
                ..
            }
            | Self::Renamed {
                attester_pubkey_b32,
                ..
            } => attester_pubkey_b32.as_deref(),
            Self::Expired { .. } | Self::Consolidated { .. } => None,
        }
    }
}

/// Filter predicate for the SSE endpoint.
#[derive(Debug, Default, Clone)]
pub struct MemoryEventFilter {
    /// If `Some`, only events whose `primary_path()` starts with this.
    pub path_prefix: Option<String>,
    /// If `Some`, only events with this kind.
    pub kind: Option<String>,
    /// If `Some`, only events whose attester matches.
    pub attester_pubkey_b32: Option<String>,
}

impl MemoryEventFilter {
    pub fn matches(&self, ev: &MemoryEvent) -> bool {
        if let Some(prefix) = &self.path_prefix {
            if !ev.primary_path().starts_with(prefix) {
                return false;
            }
        }
        if let Some(k) = &self.kind {
            if ev.kind_str() != k {
                return false;
            }
        }
        if let Some(pk) = &self.attester_pubkey_b32 {
            match ev.attester_pubkey() {
                Some(found) if found == pk => {}
                _ => return false,
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_path_prefix() {
        let ev = MemoryEvent::Created {
            path: "/memories/foo/bar.md".into(),
            file_cid: "cid".into(),
            kind: "resource".into(),
            attester_pubkey_b32: None,
            signed_at: "now".into(),
            size_bytes: 0,
        };
        let f = MemoryEventFilter {
            path_prefix: Some("/memories/foo/".into()),
            ..Default::default()
        };
        assert!(f.matches(&ev));
        let f2 = MemoryEventFilter {
            path_prefix: Some("/memories/bar/".into()),
            ..Default::default()
        };
        assert!(!f2.matches(&ev));
    }

    #[test]
    fn filter_kind() {
        let ev = MemoryEvent::Modified {
            path: "/memories/x".into(),
            file_cid: "c".into(),
            prev_file_cid: None,
            kind: "semantic".into(),
            attester_pubkey_b32: None,
            signed_at: "n".into(),
        };
        let f = MemoryEventFilter {
            kind: Some("semantic".into()),
            ..Default::default()
        };
        assert!(f.matches(&ev));
        let f2 = MemoryEventFilter {
            kind: Some("episodic".into()),
            ..Default::default()
        };
        assert!(!f2.matches(&ev));
    }

    #[test]
    fn filter_attester() {
        let ev = MemoryEvent::Created {
            path: "/memories/x".into(),
            file_cid: "c".into(),
            kind: "resource".into(),
            attester_pubkey_b32: Some("aaa".into()),
            signed_at: "n".into(),
            size_bytes: 0,
        };
        let f = MemoryEventFilter {
            attester_pubkey_b32: Some("aaa".into()),
            ..Default::default()
        };
        assert!(f.matches(&ev));
        let f2 = MemoryEventFilter {
            attester_pubkey_b32: Some("bbb".into()),
            ..Default::default()
        };
        assert!(!f2.matches(&ev));
    }
}
