//! Candidate selection. A pass considers two signals:
//!
//! 1. **Contradiction severity** — `POST /v1/memory_contradictions` ranks
//!    `(cell, band, tslot)` triples where two or more attesters disagree.
//!    High-severity disagreements are the strongest reconcile signal.
//! 2. **Write churn** — a memory path rewritten many times (long
//!    `memory_file_history`) plus its near-duplicate siblings (same path
//!    stem) is a merge candidate.
//!
//! The selection layer is pure given the responder's JSON, so it is unit-
//! tested against a small fixture in [`select_candidates_from_parts`].

use serde_json::Value;

use crate::config::SleepAgentConfig;
use crate::{cluster_by_stem, ResponderClient, SleepAgentError};

/// One memory file as seen from the responder.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryFile {
    pub path: String,
    pub file_cid: String,
    pub kind: String,
    pub content: String,
    /// Number of versions in the append-only history (1 = never rewritten).
    pub history_len: usize,
}

impl MemoryFile {
    /// Convenience constructor for tests.
    pub fn stub(path: &str, content: &str) -> Self {
        MemoryFile {
            path: path.to_string(),
            file_cid: format!("cid-{path}"),
            kind: "semantic".to_string(),
            content: content.to_string(),
            history_len: 1,
        }
    }
}

/// Why a candidate was selected.
#[derive(Debug, Clone, PartialEq)]
pub enum CandidateSource {
    /// Selected because attesters disagree about a `(cell, band)`.
    Contradiction { cell: String, band: String, severity: f32 },
    /// Selected because a path stem has been rewritten / duplicated a lot.
    Churn { stem: String, total_versions: usize },
}

impl CandidateSource {
    /// Short label for the plan line.
    pub fn label(&self) -> String {
        match self {
            CandidateSource::Contradiction { severity, .. } => {
                format!("contradiction sev={severity:.2}")
            }
            CandidateSource::Churn { total_versions, .. } => {
                format!("churn versions={total_versions}")
            }
        }
    }

    /// Ranking score — higher is more urgent.
    fn score(&self) -> f64 {
        match self {
            CandidateSource::Contradiction { severity, .. } => 1.0 + *severity as f64,
            CandidateSource::Churn { total_versions, .. } => 0.5 + (*total_versions as f64) * 0.01,
        }
    }
}

/// A reconcile candidate: a cluster of near-duplicate memory files plus the
/// canonical path the merged result will be written to.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub source: CandidateSource,
    pub cluster: Vec<MemoryFile>,
    /// Path the merged memory is written to (supersedes bi-temporally).
    /// Chosen as the highest-churn / lexicographically-first member.
    pub canonical_path: String,
}

/// Select candidates by querying the responder live, then delegating to the
/// pure [`select_candidates_from_parts`].
pub async fn select_candidates(
    client: &ResponderClient,
    cfg: &SleepAgentConfig,
) -> Result<Vec<Candidate>, SleepAgentError> {
    // 1) Contradictions.
    let contra = client
        .memory_contradictions(None, cfg.min_severity, cfg.top_n.max(8))
        .await?;

    // 2) Churn: enumerate files per configured kind, then hydrate each with
    //    its content + history length.
    let mut files: Vec<MemoryFile> = Vec::new();
    for kind in &cfg.churn_kinds {
        // A kind the responder rejects (e.g. core/vault excluded) is not
        // fatal — skip it and keep going.
        let listed = match client.list_by_kind(kind, 256).await {
            Ok(v) => v,
            Err(_) => continue,
        };
        for entry in listed {
            let Some(path) = entry.get("path").and_then(|v| v.as_str()) else {
                continue;
            };
            // Hydrate content via memory_view.
            let view = match client.view(path).await {
                Ok(v) => v,
                Err(_) => continue,
            };
            files.push(MemoryFile {
                path: path.to_string(),
                file_cid: view
                    .get("file_cid")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                kind: view
                    .get("memory_kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or(kind)
                    .to_string(),
                content: view
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                // The view does not surface history length directly; we
                // infer churn from sibling-cluster size below. Default 1.
                history_len: 1,
            });
        }
    }

    Ok(select_candidates_from_parts(&contra, files, cfg))
}

/// Pure candidate selection over already-fetched parts. Unit-testable.
///
/// `contradictions` is the `ContradictionsResp` JSON. `files` is the
/// hydrated memory-file set. Returns up to `cfg.top_n` candidates ranked by
/// urgency.
pub fn select_candidates_from_parts(
    contradictions: &Value,
    files: Vec<MemoryFile>,
    cfg: &SleepAgentConfig,
) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();

    // ---- Churn candidates: cluster near-duplicates, keep clusters whose
    //      total version count (files in cluster + their histories) meets
    //      the churn threshold.
    let clusters = cluster_by_stem(files);
    for (stem, mut members) in clusters {
        let total_versions: usize = members.iter().map(|m| m.history_len).sum::<usize>();
        let cluster_size = members.len();
        // High churn = either many sibling duplicates OR a single file
        // rewritten past the threshold.
        if cluster_size < 2 && total_versions < cfg.churn_threshold {
            continue;
        }
        // Canonical path: lexicographically-first member (stable).
        members.sort_by(|a, b| a.path.cmp(&b.path));
        let canonical_path = members[0].path.clone();
        out.push(Candidate {
            source: CandidateSource::Churn {
                stem,
                total_versions: total_versions.max(cluster_size),
            },
            cluster: members,
            canonical_path,
        });
    }

    // ---- Contradiction candidates: one per surfaced contradiction above
    //      the severity floor. These do not necessarily map to a memory
    //      path; we record the (cell, band) so the operator sees what the
    //      attesters disagree about. The cluster is empty when no memory
    //      file is associated — still surfaced honestly in the plan.
    if let Some(list) = contradictions
        .get("contradictions")
        .and_then(|v| v.as_array())
    {
        for c in list {
            let severity = c.get("severity").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
            if severity < cfg.min_severity {
                continue;
            }
            let cell = c
                .get("cell")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let band = c
                .get("band")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            // Canonical path for a contradiction reconciliation: a stable
            // derived path under the sleep-agent's own namespace. Memory
            // paths must live under `/memories/` (Anthropic memory-tool
            // spec the responder enforces).
            let canonical_path = format!("/memories/reconciled/{cell}/{band}");
            out.push(Candidate {
                source: CandidateSource::Contradiction {
                    cell,
                    band,
                    severity,
                },
                cluster: Vec::new(),
                canonical_path,
            });
        }
    }

    // Rank by urgency, keep top-N.
    out.sort_by(|a, b| {
        b.source
            .score()
            .partial_cmp(&a.source.score())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out.truncate(cfg.top_n);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cfg() -> SleepAgentConfig {
        SleepAgentConfig::from_env(true, Some("http://localhost:1".into()))
    }

    #[test]
    fn selects_churn_cluster_of_near_duplicates() {
        let files = vec![
            MemoryFile::stub("notes/borneo-1", "ndvi dropped 0.2 in march"),
            MemoryFile::stub("notes/borneo-2", "ndvi fell ~0.2 around march"),
            MemoryFile::stub("notes/jakarta", "flood risk high"),
        ];
        let cands = select_candidates_from_parts(&json!({}), files, &cfg());
        // borneo cluster (2 members) is a candidate; jakarta (1, no churn)
        // is not.
        assert_eq!(cands.len(), 1);
        match &cands[0].source {
            CandidateSource::Churn { stem, .. } => assert_eq!(stem, "notes/borneo"),
            other => panic!("expected churn, got {other:?}"),
        }
        assert_eq!(cands[0].cluster.len(), 2);
        assert_eq!(cands[0].canonical_path, "notes/borneo-1");
    }

    #[test]
    fn selects_contradictions_above_severity_floor() {
        let contra = json!({
            "contradictions": [
                { "cell": "defi.zb5", "band": "indices.ndvi", "severity": 0.8 },
                { "cell": "defi.zb6", "band": "indices.ndvi", "severity": 0.05 }
            ]
        });
        let cands = select_candidates_from_parts(&contra, vec![], &cfg());
        // Only the 0.8 one clears the 0.3 default floor.
        assert_eq!(cands.len(), 1);
        match &cands[0].source {
            CandidateSource::Contradiction { severity, cell, .. } => {
                assert!((*severity - 0.8).abs() < 1e-6);
                assert_eq!(cell, "defi.zb5");
            }
            other => panic!("expected contradiction, got {other:?}"),
        }
    }

    #[test]
    fn empty_corpus_yields_no_candidates() {
        let cands = select_candidates_from_parts(&json!({"contradictions": []}), vec![], &cfg());
        assert!(cands.is_empty());
    }

    #[test]
    fn contradiction_outranks_churn() {
        let files = vec![
            MemoryFile::stub("notes/x-1", "a"),
            MemoryFile::stub("notes/x-2", "b"),
        ];
        let contra = json!({"contradictions": [
            { "cell": "c", "band": "b", "severity": 0.9 }
        ]});
        let cands = select_candidates_from_parts(&contra, files, &cfg());
        assert_eq!(cands.len(), 2);
        // Contradiction (score 1.9) ranks above churn (score ~0.5).
        assert!(matches!(
            cands[0].source,
            CandidateSource::Contradiction { .. }
        ));
    }
}
