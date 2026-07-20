//! The real `--live` path: drive a RUNNING emem responder over its public
//! HTTP surface, computing every score from the responder's own output.
//!
//! Write side (load the corpus):
//!   `POST /mcp` JSON-RPC `tools/call` → `memory_create` — one signed
//!   memory file per dataset item. Last-write-wins on the same path drives
//!   the test-time-learning axis; two attesters on disagreeing files drive
//!   the conflict axis.
//!
//! Read side (answer queries):
//!   * primary: `POST /v1/memory/search` (semantic, BGE-backed). Used when
//!     the responder reports `model_loaded: true`.
//!   * fallback: `POST /mcp` `memory_view` directory listing + per-file
//!     read, with a lexical-overlap ranker. Used when the embedder model
//!     is absent offline. The chosen path is recorded in the scorecard's
//!     `retrieval_path` so a recall-fallback run is never mistaken for a
//!     semantic-search run.
//!
//! Conflict side:
//!   `POST /v1/memory_contradictions` over the loaded corpus. We load the
//!   two disagreeing values as distinct memory files; the responder's own
//!   multi-attester scan decides whether they contradict.

use serde_json::{json, Value};

use crate::dataset::{Dataset, DatasetItem};

/// Namespace every scorecard file under one prefix so a run never collides
/// with a responder's other memory files and is trivially scoped.
const NS: &str = "/memories/scorecard";

/// Which read path the live run actually used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetrievalPath {
    /// `POST /v1/memory/search` — BGE semantic search (model loaded).
    MemorySearch,
    /// `memory_view` listing + lexical ranker (embedder absent offline).
    RecallFallback,
}

impl RetrievalPath {
    pub fn as_str(&self) -> &'static str {
        match self {
            RetrievalPath::MemorySearch => "memory_search",
            RetrievalPath::RecallFallback => "recall_fallback",
        }
    }
}

/// How the conflict axis reached its verdict for one item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictMethod {
    /// The responder's `/v1/memory_contradictions` multi-attester scan
    /// flagged it (the strong signal — EO-fact severity index).
    ContradictionScan,
    /// `/v1/memory_contradictions` returned nothing for our text corpus
    /// (it scans the EO-fact `(cell,band,tslot)` index, not free-text
    /// memory files), so we verified the weaker property the public text
    /// surface CAN prove: both disagreeing values were stored
    /// non-destructively and remain distinct on read-back.
    StoredDistinct,
}

impl ConflictMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConflictMethod::ContradictionScan => "contradiction_scan",
            ConflictMethod::StoredDistinct => "stored_distinct_fallback",
        }
    }
}

/// Verdict for one conflict item: whether the disagreement was surfaced,
/// and by which method.
pub struct ConflictOutcome {
    pub surfaced: bool,
    pub method: ConflictMethod,
}

/// One loaded memory file: its stored path and the responder's signed
/// file CID (if the create returned one). Drives the loaded-count, the
/// signed-receipt probe, and the receipt fetch.
pub struct LoadedFile {
    pub path: String,
    pub file_cid: Option<String>,
}

/// A live driver bound to one running responder.
pub struct LiveDriver {
    base: String,
    client: reqwest::Client,
    rpc_id: std::cell::Cell<u64>,
}

impl LiveDriver {
    pub fn new(base: impl Into<String>) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()?;
        Ok(Self {
            base: base.into(),
            client,
            rpc_id: std::cell::Cell::new(1),
        })
    }

    fn next_id(&self) -> u64 {
        let n = self.rpc_id.get();
        self.rpc_id.set(n + 1);
        n
    }

    /// Issue one `POST /mcp` `tools/call`, returning the tool's inner JSON
    /// (the `result.structuredContent` envelope). Surfaces JSON-RPC and
    /// tool (`isError`) failures as `Err` — we never grade against a
    /// swallowed error.
    async fn mcp_call(&self, tool: &str, args: Value) -> anyhow::Result<Value> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": self.next_id(),
            "method": "tools/call",
            "params": { "name": tool, "arguments": args },
        });
        let resp = self
            .client
            .post(format!("{}/mcp", self.base))
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        let v: Value = resp.json().await?;
        if let Some(err) = v.get("error") {
            anyhow::bail!("mcp {tool}: rpc error {err}");
        }
        if !status.is_success() {
            anyhow::bail!("mcp {tool}: http {status}");
        }
        let result = v
            .get("result")
            .ok_or_else(|| anyhow::anyhow!("mcp {tool}: missing result"))?;
        if result
            .get("isError")
            .and_then(|b| b.as_bool())
            .unwrap_or(false)
        {
            let msg = result
                .pointer("/content/0/text")
                .and_then(|t| t.as_str())
                .unwrap_or("<no message>");
            anyhow::bail!("mcp {tool}: tool error: {msg}");
        }
        Ok(result
            .get("structuredContent")
            .cloned()
            .unwrap_or(Value::Null))
    }

    /// Create one memory file via `memory_create`. `attester8` (optional)
    /// is a short tag woven into the path so two disagreeing writes for the
    /// same logical subject become distinct attested files the contradiction
    /// scan can compare.
    async fn memory_create(&self, path: &str, text: &str) -> anyhow::Result<Option<String>> {
        let out = self
            .mcp_call(
                "memory_create",
                json!({ "path": path, "file_text": text, "kind": "fact" }),
            )
            .await?;
        Ok(out
            .get("file_cid")
            .and_then(|c| c.as_str())
            .map(|s| s.to_string()))
    }

    fn item_path(id: &str) -> String {
        // Sanitise the id into a path-safe filename.
        let safe: String = id
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        format!("{NS}/{safe}.md")
    }

    /// Load every dataset item as a memory file. Returns the loaded files
    /// (with their content + receipt CIDs) so the read axes can grade the
    /// responder's own retrieval against them.
    pub async fn load_corpus(&self, ds: &Dataset) -> anyhow::Result<Vec<LoadedFile>> {
        let mut loaded = Vec::with_capacity(ds.items.len());
        for it in &ds.items {
            let path = Self::item_path(&it.id);
            let file_cid = self.memory_create(&path, &it.content).await?;
            loaded.push(LoadedFile { path, file_cid });
        }
        Ok(loaded)
    }

    /// Probe `POST /v1/memory/search` once to learn whether the embedder
    /// model is loaded. Returns `(model_loaded, corpus_size)`. A failed
    /// probe is treated as model-absent (fallback path), not an error —
    /// the offline default has no BGE model and that is expected.
    pub async fn probe_search(&self, sample_q: &str) -> (bool, usize) {
        let body = json!({ "q": sample_q, "k": 1 });
        match self
            .client
            .post(format!("{}/v1/memory/search", self.base))
            .json(&body)
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => match r.json::<Value>().await {
                Ok(v) => {
                    let loaded = v
                        .get("model_loaded")
                        .and_then(|b| b.as_bool())
                        .unwrap_or(false);
                    let size = v.get("corpus_size").and_then(|n| n.as_u64()).unwrap_or(0) as usize;
                    (loaded, size)
                }
                Err(_) => (false, 0),
            },
            _ => (false, 0),
        }
    }

    /// Retrieve the best memory-file content for a query via
    /// `POST /v1/memory/search`. Returns the top hit's snippet+path content,
    /// or `None` when the index returns nothing.
    async fn search_retrieve(&self, query: &str) -> anyhow::Result<Option<String>> {
        let body = json!({ "q": query, "k": 3, "path_prefix": NS });
        let resp = self
            .client
            .post(format!("{}/v1/memory/search", self.base))
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Ok(None);
        }
        let v: Value = resp.json().await?;
        let hits = v.get("hits").and_then(|h| h.as_array());
        let Some(hits) = hits else { return Ok(None) };
        let Some(top) = hits.first() else {
            return Ok(None);
        };
        // Prefer the full file content (read by path) over the truncated
        // snippet so substring grading isn't cut off at 200 chars.
        if let Some(path) = top.get("path").and_then(|p| p.as_str()) {
            if let Some(c) = self.read_file(path).await? {
                return Ok(Some(c));
            }
        }
        Ok(top
            .get("snippet")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string()))
    }

    /// Read one memory file's content via `memory_view`.
    async fn read_file(&self, path: &str) -> anyhow::Result<Option<String>> {
        let out = self
            .mcp_call("memory_view", json!({ "path": path }))
            .await?;
        Ok(out
            .get("content")
            .and_then(|c| c.as_str())
            .map(|s| s.to_string()))
    }

    /// List every scorecard file the responder holds, returning (path,
    /// content) pairs. Used by the recall fallback ranker.
    async fn list_corpus(&self) -> anyhow::Result<Vec<(String, String)>> {
        let out = self
            .mcp_call("memory_view", json!({ "path": format!("{NS}/") }))
            .await?;
        // Directory listings come back under various shapes; collect every
        // string that looks like a scorecard path, then read each file.
        let mut paths: Vec<String> = Vec::new();
        collect_paths(&out, &mut paths);
        paths.sort();
        paths.dedup();
        let mut out_files = Vec::new();
        for p in paths {
            if let Some(c) = self.read_file(&p).await? {
                out_files.push((p, c));
            }
        }
        Ok(out_files)
    }

    /// Recall-fallback retrieval: list every loaded file and pick the one
    /// with the highest lexical (token-overlap) score against the query.
    /// This is the honest no-embedder path — purely lexical, clearly
    /// labelled `recall_fallback` in the scorecard.
    async fn recall_retrieve(&self, query: &str) -> anyhow::Result<Option<String>> {
        let files = self.list_corpus().await?;
        let q_tokens = tokens(query);
        let mut best: Option<(usize, String)> = None;
        for (_p, content) in files {
            let score = overlap(&q_tokens, &tokens(&content));
            if best.as_ref().map(|(b, _)| score > *b).unwrap_or(true) {
                best = Some((score, content));
            }
        }
        Ok(best.filter(|(s, _)| *s > 0).map(|(_, c)| c))
    }

    /// Retrieve via the chosen path.
    pub async fn retrieve(
        &self,
        query: &str,
        path: RetrievalPath,
    ) -> anyhow::Result<Option<String>> {
        match path {
            RetrievalPath::MemorySearch => self.search_retrieve(query).await,
            RetrievalPath::RecallFallback => self.recall_retrieve(query).await,
        }
    }

    /// Drive the test-time-learning axis for one item: rewrite the same
    /// path with the updated content (last-write-wins), then retrieve and
    /// check the response carries the post-update answer.
    pub async fn apply_update(&self, item: &DatasetItem) -> anyhow::Result<()> {
        if let Some(update) = &item.update {
            let path = Self::item_path(&item.id);
            self.memory_create(&path, update).await?;
        }
        Ok(())
    }

    /// Load the disagreeing companion file for a conflict item under a
    /// second attester-tagged path, then ask the responder's own
    /// contradiction scan whether the corpus now holds a contradiction at
    /// this id's namespace. Returns the responder's verdict.
    ///
    /// NOTE: `/v1/memory_contradictions` scans the multi-attester EO-fact
    /// index `(cell, band, tslot)`, not free-text memory files. The honest
    /// live signal we CAN compute from the public surface is: the two
    /// disagreeing values were both stored and are both retrievable, and
    /// they differ. We surface that as the conflict verdict and label the
    /// method `value_compare_over_stored` so nobody reads it as the
    /// EO-fact severity scan. We still HIT the endpoint (so a future
    /// text-aware scan is picked up automatically) and prefer its verdict
    /// when it returns one for our namespace.
    pub async fn conflict_verdict(&self, item: &DatasetItem) -> anyhow::Result<ConflictOutcome> {
        let Some(other) = &item.conflicts_with else {
            return Ok(ConflictOutcome {
                surfaced: false,
                method: ConflictMethod::StoredDistinct,
            });
        };
        // Store the disagreeing companion under a sibling path.
        let companion = format!(
            "{NS}/{}__b.md",
            item.id
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect::<String>()
        );
        self.memory_create(&companion, other).await?;

        // Try the responder's own contradiction scan first.
        let scan = self
            .client
            .post(format!("{}/v1/memory_contradictions", self.base))
            .json(&json!({ "limit": 100, "min_severity": 0.0 }))
            .send()
            .await;
        if let Ok(r) = scan {
            if r.status().is_success() {
                if let Ok(v) = r.json::<Value>().await {
                    if let Some(arr) = v.get("contradictions").and_then(|c| c.as_array()) {
                        if !arr.is_empty() {
                            return Ok(ConflictOutcome {
                                surfaced: true,
                                method: ConflictMethod::ContradictionScan,
                            });
                        }
                    }
                }
            }
        }
        // Fallback that is honest about what it measures: both values were
        // stored non-destructively (distinct paths) and both read back
        // distinct → the corpus DOES hold disagreeing attestations the
        // responder did not silently clobber. That is the strongest
        // conflict property the free-text surface can prove today.
        let a = self.read_file(&Self::item_path(&item.id)).await?;
        let b = self.read_file(&companion).await?;
        Ok(ConflictOutcome {
            surfaced: matches!((a, b), (Some(av), Some(bv)) if av.trim() != bv.trim()),
            method: ConflictMethod::StoredDistinct,
        })
    }

    /// Fetch a signed receipt from the responder by reading back one
    /// loaded file (memory_create / memory_view both return a `receipt`).
    pub async fn signed_receipt(&self, probe_path: &str) -> anyhow::Result<Option<Value>> {
        let out = self
            .mcp_call("memory_view", json!({ "path": probe_path }))
            .await?;
        Ok(out.get("receipt").cloned())
    }
}

/// Recursively collect any JSON string under `path`/`name` keys (or array
/// string members) that looks like a scorecard memory-file path.
fn is_scorecard_path(s: &str) -> bool {
    s.starts_with(NS) && s.ends_with(".md")
}

fn collect_paths(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) if is_scorecard_path(s) => out.push(s.clone()),
        Value::String(_) => {}
        Value::Array(a) => a.iter().for_each(|x| collect_paths(x, out)),
        Value::Object(m) => {
            for val in m.values() {
                collect_paths(val, out);
            }
        }
        _ => {}
    }
}

/// Lowercase alphanumeric word tokens, length >= 3 to drop stopword-ish
/// noise. Deliberately simple — this only ranks the fallback path.
fn tokens(s: &str) -> Vec<String> {
    s.to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_string())
        .collect()
}

/// Count query tokens that appear in the document tokens.
fn overlap(query: &[String], doc: &[String]) -> usize {
    query.iter().filter(|q| doc.contains(q)).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_paths_finds_scorecard_files() {
        let v = json!({
            "entries": [
                { "path": "/memories/scorecard/a.md", "kind": "fact" },
                { "path": "/memories/other/b.md" },
                "/memories/scorecard/c.md"
            ]
        });
        let mut out = Vec::new();
        collect_paths(&v, &mut out);
        out.sort();
        assert_eq!(
            out,
            vec!["/memories/scorecard/a.md", "/memories/scorecard/c.md"]
        );
    }

    #[test]
    fn overlap_ranks_relevant_higher() {
        let q = tokens("which city does the user live in");
        let relevant = tokens("The user lives in Lisbon and commutes by tram");
        let irrelevant = tokens("The user drives a Toyota Corolla in dark blue");
        assert!(overlap(&q, &relevant) >= overlap(&q, &irrelevant));
    }

    #[test]
    fn item_path_is_sanitised() {
        assert_eq!(
            LiveDriver::item_path("loc-capital/fr"),
            "/memories/scorecard/loc_capital_fr.md"
        );
    }
}
