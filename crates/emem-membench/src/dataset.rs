//! Public-benchmark dataset loading (LongMemEval-S / MemoryAgentBench style).
//!
//! A dataset is a JSON Lines file where each line is one item:
//!
//! ```json
//! {"id":"q1","content":"...the memory the agent must store...","query":"...the question...","expected_answer":"...ground truth..."}
//! ```
//!
//! This is the lowest-common-denominator schema shared by LongMemEval-S
//! (`question_id` / `question` / `answer` over a `haystack_sessions`
//! transcript) and MemoryAgentBench (`context` / `query` / `answer`). The
//! loader accepts the canonical emem field names plus the common aliases
//! from those two corpora so a user can point `--dataset` at a converted
//! export without hand-editing every key. See `docs/benchmarks.md` for how
//! to fetch and convert the full public datasets.
//!
//! Items optionally carry an `update` (a revised value for the SAME `id`,
//! used by the test-time-learning axis) and a `conflicts_with` /
//! `conflict_answer` pair (a second value that disagrees, used by the
//! conflict-resolution axis). Both are optional — a plain
//! id/content/query/expected_answer item still scores the retrieval and
//! long-range axes.

use serde::Deserialize;

/// One benchmark item in the canonical emem schema (with aliases).
#[derive(Debug, Clone, Deserialize)]
pub struct DatasetItem {
    /// Stable identifier. Aliases: `question_id`, `qid`.
    #[serde(alias = "question_id", alias = "qid")]
    pub id: String,

    /// The memory content the responder must store and later retrieve.
    /// Aliases: `context`, `text`, `memory`.
    #[serde(alias = "context", alias = "text", alias = "memory")]
    pub content: String,

    /// The question asked at read time. Aliases: `question`.
    #[serde(alias = "question")]
    pub query: String,

    /// Ground-truth answer. A response is graded correct when its
    /// retrieved content contains this string (normalised). Aliases:
    /// `answer`, `expected`.
    #[serde(alias = "answer", alias = "expected")]
    pub expected_answer: String,

    /// Optional revised content for the SAME id, simulating a test-time
    /// update. When present, the learning axis loads `content` then
    /// `update`, then checks the responder returns `update_answer`.
    #[serde(default)]
    pub update: Option<String>,

    /// Ground-truth answer AFTER the update. Required for the learning
    /// axis to grade an item; items without it are skipped by that axis.
    #[serde(default)]
    pub update_answer: Option<String>,

    /// Optional second, disagreeing content about the same subject. When
    /// present, the conflict axis loads both `content` and this value
    /// under two distinct attesters and expects a contradiction.
    #[serde(default)]
    pub conflicts_with: Option<String>,
}

/// Honesty label for the loaded corpus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// The small committed sample (`data/sample-longmemeval.jsonl`). Its
    /// score is illustrative, NOT the published benchmark number.
    Sample,
    /// A user-supplied dataset (e.g. the full LongMemEval-S export).
    Full,
}

impl Provenance {
    pub fn as_str(&self) -> &'static str {
        match self {
            Provenance::Sample => "sample",
            Provenance::Full => "full",
        }
    }
}

/// A loaded dataset plus its provenance label.
#[derive(Debug, Clone)]
pub struct Dataset {
    pub items: Vec<DatasetItem>,
    pub provenance: Provenance,
    /// Source path as given on the command line (for the scorecard).
    pub source: String,
}

/// The committed sample lives at this path relative to the crate root.
pub const SAMPLE_RELATIVE_PATH: &str = "crates/emem-membench/data/sample-longmemeval.jsonl";

/// Parse a JSONL string into items. Blank lines and lines beginning with
/// `#` (comments) are skipped. A malformed line is a hard error with its
/// line number — we never silently drop a row, because a silently-dropped
/// row would inflate the score (fewer questions to get wrong).
pub fn parse_jsonl(text: &str) -> anyhow::Result<Vec<DatasetItem>> {
    let mut items = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let item: DatasetItem = serde_json::from_str(trimmed)
            .map_err(|e| anyhow::anyhow!("dataset line {}: {e}", i + 1))?;
        if item.id.trim().is_empty() {
            anyhow::bail!("dataset line {}: empty `id`", i + 1);
        }
        items.push(item);
    }
    if items.is_empty() {
        anyhow::bail!("dataset contained no items");
    }
    Ok(items)
}

/// Load a dataset from a path, inferring provenance: a path that ends with
/// the committed sample filename is labelled `sample`; anything else is
/// `full` (user-supplied). The label rides into the scorecard so a sample
/// score is never mistaken for the published benchmark.
pub fn load(path: &str) -> anyhow::Result<Dataset> {
    let text =
        std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("read dataset {path}: {e}"))?;
    let items = parse_jsonl(&text)?;
    let provenance = if path.ends_with("sample-longmemeval.jsonl") {
        Provenance::Sample
    } else {
        Provenance::Full
    };
    Ok(Dataset {
        items,
        provenance,
        source: path.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_canonical_schema() {
        let jsonl = r#"
            {"id":"a","content":"The dam holds 3 million m3","query":"capacity?","expected_answer":"3 million"}
            # a comment line
            {"id":"b","content":"NDVI was 0.71 in June","query":"june ndvi?","expected_answer":"0.71"}
        "#;
        let items = parse_jsonl(jsonl).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, "a");
        assert_eq!(items[1].expected_answer, "0.71");
    }

    #[test]
    fn parses_aliases() {
        // LongMemEval-style keys.
        let jsonl = r#"{"question_id":"q7","context":"Paris is the capital of France","question":"capital of France?","answer":"Paris"}"#;
        let items = parse_jsonl(jsonl).unwrap();
        assert_eq!(items[0].id, "q7");
        assert_eq!(items[0].content, "Paris is the capital of France");
        assert_eq!(items[0].query, "capital of France?");
        assert_eq!(items[0].expected_answer, "Paris");
    }

    #[test]
    fn malformed_line_is_hard_error() {
        let jsonl =
            "{\"id\":\"a\",\"content\":\"x\",\"query\":\"q\",\"expected_answer\":\"y\"}\nnot json";
        let err = parse_jsonl(jsonl).unwrap_err();
        assert!(err.to_string().contains("line 2"), "got: {err}");
    }

    #[test]
    fn empty_dataset_errors() {
        assert!(parse_jsonl("   \n# only a comment\n").is_err());
    }

    #[test]
    fn loads_committed_sample() {
        // Resolve relative to the workspace root so the test runs from any
        // cwd (cargo sets CARGO_MANIFEST_DIR to the crate dir).
        let p = concat!(env!("CARGO_MANIFEST_DIR"), "/data/sample-longmemeval.jsonl");
        let ds = load(p).unwrap();
        assert_eq!(ds.provenance, Provenance::Sample);
        assert!(ds.items.len() >= 10, "sample should have >=10 items");
        // At least some items must carry update/conflict fields so the
        // learning + conflict axes have something to grade.
        assert!(ds.items.iter().any(|i| i.update.is_some()));
        assert!(ds.items.iter().any(|i| i.conflicts_with.is_some()));
    }
}
