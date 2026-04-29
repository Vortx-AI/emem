//! One-shot admin tool: purge facts whose `derivation.fn_key` matches a
//! given key (default `claude_knowledge@1`) from the sled hot cache.
//!
//! Why this exists. Early agent demos (`emem-realdemo`, ad-hoc Claude
//! sessions) attested elevation values for famous cells (Mt Fuji,
//! Mt Everest) under `derivation.fn_key = "claude_knowledge@1"` —
//! plausible numbers carried by an LLM's training-data ground truth, not
//! a re-derivable computation. The recall path serves the first stored
//! fact at `(cell, band, tslot)`, so those LLM-guess facts persist
//! forever and *override* the real materializer (Open-Meteo, MET Norway,
//! Tessera, …) for those cells, even though `has_materializer = true`
//! says the band is alive.
//!
//! Removing the cache entries lets the lazy-materialize path on the next
//! `/v1/recall` re-fetch fresh facts from the upstream open-data REST.
//! The append-only Merkle log retains the original attestations for
//! historical replay; only the read-side index/facts trees are touched.
//!
//! Usage:
//!
//! ```text
//! emem-purge-fnkey                                       # dry-run, fn_key=claude_knowledge@1
//! emem-purge-fnkey --apply                               # actually delete
//! emem-purge-fnkey --fn-key foo@1 --apply                # different key
//! emem-purge-fnkey --data-dir /home/ubuntu/emem/var/emem # override path
//! ```
//!
//! Server must be stopped first — sled holds an exclusive lock.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use emem_fact::Fact;

#[derive(Debug)]
struct Args {
    data_dir: PathBuf,
    /// One or more fn_key strings to match (any-of semantics). Empty list
    /// means "match nothing on fn_key alone" — pair with --stale-grid.
    fn_keys: Vec<String>,
    /// Also remove facts whose cell prefix is not in the current grid.
    /// The active grid emits cells with prefix "damO.zb000." today (see
    /// `/v1/grid_info`); anything else is an orphan from a pre-grid-update
    /// demo and unreachable via `/v1/locate`.
    stale_grid: bool,
    /// Only purge facts whose cell prefix matches this string (when set).
    /// Useful to sweep orphans without touching live data.
    cell_prefix_not: Option<String>,
    /// Sweep orphan fact bodies — facts whose CID isn't pointed to by any
    /// index entry. These are superseded facts (later put_many overwrote
    /// the canonical key with a new CID, leaving the old body unreachable).
    /// They consume disk but are unreachable via /v1/recall. Safe to delete:
    /// the Merkle log retains them for replay verification.
    orphan_facts: bool,
    apply: bool,
}

fn parse_args() -> Result<Args> {
    let mut data_dir = std::env::var("EMEM_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/home/ubuntu/emem/var/emem"));
    let mut fn_keys: Vec<String> = vec!["claude_knowledge@1".into()];
    let mut fn_keys_set = false;
    let mut stale_grid = false;
    let mut cell_prefix_not: Option<String> = None;
    let mut orphan_facts = false;
    let mut apply = false;

    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--apply" => apply = true,
            "--dry-run" => apply = false,
            "--fn-key" => {
                let v = it
                    .next()
                    .context("--fn-key requires a value (comma-separated for multiple)")?;
                if !fn_keys_set {
                    fn_keys.clear();
                    fn_keys_set = true;
                }
                for k in v.split(',') {
                    let k = k.trim();
                    if !k.is_empty() {
                        fn_keys.push(k.to_string());
                    }
                }
            }
            "--no-fn-key" => {
                fn_keys.clear();
                fn_keys_set = true;
            }
            "--stale-grid" => stale_grid = true,
            "--orphan-facts" => orphan_facts = true,
            "--cell-prefix-not" => {
                cell_prefix_not = Some(it.next().context("--cell-prefix-not requires a value")?);
            }
            "--data-dir" => {
                data_dir = PathBuf::from(it.next().context("--data-dir requires a path")?);
            }
            "-h" | "--help" => {
                eprintln!("{}", USAGE);
                std::process::exit(0);
            }
            other => bail!("unknown arg: {other}"),
        }
    }
    Ok(Args {
        data_dir,
        fn_keys,
        stale_grid,
        cell_prefix_not,
        orphan_facts,
        apply,
    })
}

/// Active-grid cell prefix today. Cells that don't start with this came
/// from a pre-update encoding and cannot be reached by `/v1/locate`. Kept
/// here as a constant so this tool stays a self-contained admin binary.
const ACTIVE_GRID_PREFIX: &str = "damO.zb000.";

const USAGE: &str = "\
emem-purge-fnkey — remove cache entries that match a predicate.

USAGE:
  emem-purge-fnkey [--apply] [--fn-key KEY[,KEY,...]] [--stale-grid]
                   [--cell-prefix-not PREFIX] [--data-dir PATH]

PREDICATES (any one selects a row; combine for OR semantics):
  --fn-key claude_knowledge@1,demo@1     match these derivation fn_keys
  --no-fn-key                            disable fn_key matching
  --stale-grid                           cells outside the active grid
                                         (prefix not 'damO.zb000.')
  --cell-prefix-not catO                 cells starting with 'catO'
  --orphan-facts                         second pass: drop fact bodies
                                         whose CID has no index entry
                                         (superseded after put_many)

DEFAULTS:
  --fn-key   claude_knowledge@1
  --data-dir $EMEM_DATA  (or /home/ubuntu/emem/var/emem)
  --dry-run is the default; pass --apply to actually delete.

Server must be stopped — sled requires an exclusive lock.
";

fn main() -> Result<()> {
    let args = parse_args()?;

    let cache_path = args.data_dir.join("cache.sled");
    if !cache_path.exists() {
        bail!("cache dir not found: {}", cache_path.display());
    }

    println!("data_dir        : {}", args.data_dir.display());
    println!("cache           : {}", cache_path.display());
    if !args.fn_keys.is_empty() {
        println!("fn_keys         : {}", args.fn_keys.join(", "));
    }
    if args.stale_grid {
        println!("stale_grid      : prefix != \"{}\"", ACTIVE_GRID_PREFIX);
    }
    if let Some(p) = &args.cell_prefix_not {
        println!("cell_prefix_not : {}", p);
    }
    if args.orphan_facts {
        println!("orphan_facts    : on (sweep facts not pointed to by any index entry)");
    }
    println!(
        "mode            : {}",
        if args.apply {
            "APPLY (deleting)"
        } else {
            "dry-run"
        }
    );
    println!();

    let db = sled::open(&cache_path)
        .with_context(|| format!("open sled at {}", cache_path.display()))?;
    let idx = db.open_tree("emem.canonical_index")?;
    let facts = db.open_tree("emem.facts")?;

    let total_idx = idx.len();
    let total_facts = facts.len();
    println!("index entries : {total_idx}");
    println!("facts entries : {total_facts}");
    println!();

    // Walk the index. For each (canonical_key → fact_cid), look the fact up
    // in the facts tree and check its derivation.fn_key.
    let mut hits: Vec<(Vec<u8>, String, String, String, u64, String)> = Vec::new();
    let mut errors_parse = 0usize;
    let mut missing_facts = 0usize;

    for kv in idx.iter() {
        let (k, v) = kv?;
        let cid_str = match std::str::from_utf8(&v) {
            Ok(s) => s.to_string(),
            Err(_) => {
                errors_parse += 1;
                continue;
            }
        };

        let raw = match facts.get(cid_str.as_bytes())? {
            Some(b) => b,
            None => {
                missing_facts += 1;
                continue;
            }
        };
        let fact: Fact = match ciborium::de::from_reader(raw.as_ref()) {
            Ok(f) => f,
            Err(_) => {
                errors_parse += 1;
                continue;
            }
        };

        let (fn_key, cell, band, tslot, value_repr) = match &fact {
            Fact::Primary(p) => (
                p.derivation.fn_key.clone(),
                p.cell.clone(),
                p.band.clone(),
                p.tslot,
                value_to_str(&p.value),
            ),
            Fact::Absence(n) => (
                "<absence>".to_string(),
                n.cell.clone(),
                n.band.clone(),
                n.tslot,
                "absence".to_string(),
            ),
            Fact::Derivative(_) => continue,
        };
        let matches_fn_key = !args.fn_keys.is_empty() && args.fn_keys.contains(&fn_key);
        let matches_stale_grid = args.stale_grid && !cell.starts_with(ACTIVE_GRID_PREFIX);
        let matches_prefix_not = args
            .cell_prefix_not
            .as_ref()
            .map(|p| cell.starts_with(p))
            .unwrap_or(false);
        if matches_fn_key || matches_stale_grid || matches_prefix_not {
            hits.push((k.to_vec(), cid_str, cell, band, tslot, value_repr));
        }
    }

    println!("=== matches: {} ===", hits.len());
    println!("{:<26} {:<40} {:>6}  value", "cell", "band", "tslot");
    println!("{}", "-".repeat(100));
    for (_idx_key, _cid, cell, band, tslot, val) in &hits {
        println!("{:<26} {:<40} {:>6}  {}", cell, band, tslot, val);
    }
    println!();
    if errors_parse > 0 {
        println!("(skipped {errors_parse} entries that failed to parse)");
    }
    if missing_facts > 0 {
        println!("(skipped {missing_facts} index entries with no fact body)");
    }

    // Orphan-fact pass: walk the facts tree and find CIDs not pointed to
    // by any index entry. These are superseded fact bodies left behind
    // when put_many overwrote a canonical key with a new CID.
    let mut orphan_cids: Vec<String> = Vec::new();
    if args.orphan_facts {
        // Build the set of live CIDs referenced by the index.
        let mut live: std::collections::HashSet<String> = std::collections::HashSet::new();
        for kv in idx.iter() {
            let (_, v) = kv?;
            if let Ok(s) = std::str::from_utf8(&v) {
                live.insert(s.to_string());
            }
        }
        // Account for any CIDs we're already going to delete via the
        // index pass — those facts will be removed in the apply phase
        // already, so don't double-count them as orphans.
        let about_to_delete: std::collections::HashSet<String> = hits
            .iter()
            .map(|(_, cid, _, _, _, _)| cid.clone())
            .collect();

        for kv in facts.iter() {
            let (k, _) = kv?;
            let cid = match std::str::from_utf8(&k) {
                Ok(s) => s.to_string(),
                Err(_) => continue,
            };
            if !live.contains(&cid) && !about_to_delete.contains(&cid) {
                orphan_cids.push(cid);
            }
        }
        println!("=== orphan fact bodies: {} ===", orphan_cids.len());
        for c in orphan_cids.iter().take(8) {
            println!("  {}", c);
        }
        if orphan_cids.len() > 8 {
            println!("  ... ({} more)", orphan_cids.len() - 8);
        }
        println!();
    }

    if hits.is_empty() && orphan_cids.is_empty() {
        println!("nothing to do.");
        return Ok(());
    }

    if !args.apply {
        println!(
            "dry-run: pass --apply to actually delete {} index matches and {} orphan facts.",
            hits.len(),
            orphan_cids.len()
        );
        return Ok(());
    }

    // Apply: delete index entry + fact body for each match, then orphans.
    let mut deleted_idx = 0usize;
    let mut deleted_facts = 0usize;
    for (idx_key, cid, _, _, _, _) in &hits {
        if idx.remove(idx_key)?.is_some() {
            deleted_idx += 1;
        }
        if facts.remove(cid.as_bytes())?.is_some() {
            deleted_facts += 1;
        }
    }
    let mut deleted_orphans = 0usize;
    for cid in &orphan_cids {
        if facts.remove(cid.as_bytes())?.is_some() {
            deleted_orphans += 1;
        }
    }
    idx.flush()?;
    facts.flush()?;
    println!(
        "deleted: {deleted_idx} index rows, {deleted_facts} fact bodies, {deleted_orphans} orphan fact bodies."
    );
    println!();
    println!("Done. Restart the server; next /v1/recall at affected cells will");
    println!("fall through to the auto-materializer (Open-Meteo, MET Norway,");
    println!("Sentinel-2 STAC, etc.) and write a fresh signed Primary fact.");
    Ok(())
}

fn value_to_str(v: &ciborium::Value) -> String {
    use ciborium::Value::*;
    match v {
        Integer(i) => format!("{:?}", i),
        Float(f) => format!("{}", f),
        Text(t) => format!("\"{}\"", &t[..t.len().min(40)]),
        Array(a) => format!("array[len={}]", a.len()),
        Map(_) => "map".into(),
        Bytes(b) => format!("bytes[{}]", b.len()),
        Bool(b) => format!("{b}"),
        Null => "null".into(),
        _ => "<other>".into(),
    }
}
