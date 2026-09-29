//! One-shot admin tool: take the migrated fact data out of the sled store.
//!
//! Why this exists. Facts moved to redb (`facts.redb`) and the migration
//! finished: `backfill_done` is set, so `consult_sled()` is false and no read
//! touches the sled fact trees, and `put_many` writes new facts to redb only.
//! What did not change is that the server still OPENS the sled store at boot,
//! because several small, live trees remain in it — attesters, fact proofs,
//! the multi-attester index, the trace gate's five trees, agent stats.
//!
//! Opening it costs the whole cold start: `storage_open` measured at
//! 1,330,076 ms, 1,320,000 ms and 1,273,596 ms on three boots against a
//! `cache.sled/db` of 43.7 GB. About 21 minutes of downtime per deploy.
//!
//! WHAT THIS TOOL DOES NOT FIX, measured by running it on 2026-09-14.
//!
//! The premise was that the 41 GB is mostly the two dead trees. It is not.
//! The inventory:
//!
//!   emem.fact_proofs         19,976,065 rows
//!   emem.multi_attester_index 2,391,244 rows
//!   everything else            ~281,000 rows across 28 trees
//!
//! Those first two are LIVE. The slimmed store came out at 40 GB against the
//! original's 41 -- about a gigabyte, because sled's heap grows with writes
//! and the copy rewrites every row it keeps. Not worth the swap, and it was
//! rolled back.
//!
//! The cold start is not sled recovery either. Immediately after this tool
//! read the whole file, the server opened the SAME 41 GB store in 28,259 ms.
//! The 21 minutes is page-cache faulting at roughly 33 MB/s of effective
//! random access; the volume does 135 MB/s sequential. A plain sequential
//! pre-read does not fix it -- `dd` of the whole file took 5m27s and left
//! buff/cache LOWER than it found it, because Linux drops behind on streaming
//! reads.
//!
//! So the real lever is the one redb already pulled for facts: move
//! `emem.fact_proofs` out of sled. ~20M rows is the store. This tool stays
//! because the inventory it prints is how that was learned, and because the
//! two dead trees should still go whenever fact_proofs moves.
//!
//! What this does NOT do: it never deletes anything. It copies the trees that
//! are still live into a NEW store beside the old one and leaves the original
//! untouched, so the rollback is renaming a directory back. Dropping trees
//! in place would not have reclaimed the space anyway — sled frees pages for
//! reuse rather than truncating the heap file, and it is the file length that
//! the boot pays for.
//!
//! Usage:
//!
//! ```text
//! emem-sled-slim                        # dry run: every tree, its size, and the check
//! emem-sled-slim --apply                # write cache.sled.slim beside it
//! emem-sled-slim --data-dir ./var/emem  # override path
//! ```
//!
//! Server must be stopped first — sled holds an exclusive lock.

use std::path::PathBuf;

use anyhow::{Context, Result};

/// The trees the migration emptied of meaning. Nothing reads them: the fact
/// path consults sled only while `backfill_done` is false.
const MIGRATED: &[&str] = &["emem.facts", "emem.canonical_index"];
/// Two more trees move into redb by a background backfill; each is dead in
/// sled only once redb reports its own backfill done, so they join the
/// migrated set at run time from redb's meta, never by assumption.
const MIGRATING: &[(&str, emem_cache::KvTable)] = &[
    ("emem.fact_proofs", emem_cache::KvTable::Proofs),
    ("emem.multi_attester_index", emem_cache::KvTable::Multi),
];

fn main() -> Result<()> {
    let mut data_dir = PathBuf::from("var/emem");
    let mut apply = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--apply" => apply = true,
            "--data-dir" => {
                data_dir = PathBuf::from(args.next().context("--data-dir needs a path")?)
            }
            other => anyhow::bail!("unknown argument {other}"),
        }
    }

    let cache_path = data_dir.join("cache.sled");
    let redb_path = data_dir.join("facts.redb");
    let out_path = data_dir.join("cache.sled.slim");

    println!("sled store : {}", cache_path.display());
    println!("redb store : {}", redb_path.display());
    println!("mode       : {}", if apply { "APPLY" } else { "dry-run" });
    println!();

    // The check that has to pass before anything is called dead: the facts
    // must already be in redb. A tree is only migrated if its contents went
    // somewhere, and "backfill_done is set" is a flag, not evidence.
    let redb = emem_cache::redb_facts::RedbFacts::open(&redb_path)
        .with_context(|| format!("open redb at {}", redb_path.display()))?;
    println!(
        "redb: backfill_done={} index_len={} size_on_disk={} bytes",
        redb.backfill_done(),
        redb.index_len().unwrap_or(0),
        redb.size_on_disk()
    );
    anyhow::ensure!(
        redb.backfill_done(),
        "redb says the backfill is NOT done; the sled fact trees are still the live copy. Nothing to slim."
    );
    // Which trees are CANDIDATES to drop. The flag says the backfill walked to
    // the end; it does not say the rows arrived, and this file's own preamble
    // says a flag is not evidence. The row counts decide, below, once sled is
    // open and both sides can be counted.
    let mut candidates: Vec<(&str, emem_cache::KvTable)> = Vec::new();
    for (tree, t) in MIGRATING {
        let done = redb.table_backfill_done(*t);
        println!(
            "redb: {tree} backfill_done={done}{}",
            if done {
                " (candidate to drop, pending a row count)"
            } else {
                " (still moving; kept)"
            }
        );
        if done {
            candidates.push((tree, *t));
        }
    }
    println!();

    println!("opening sled (this is the 22 minutes the server pays at every boot)…");
    let started = std::time::Instant::now();
    let db = sled::open(&cache_path)
        .with_context(|| format!("open sled at {}", cache_path.display()))?;
    println!("opened in {:.1}s", started.elapsed().as_secs_f64());
    println!();

    // The evidence, now that both sides can be counted. A tree is dropped only
    // if redb holds AT LEAST as many rows as sled does for it. Resuming from a
    // cursor across three restarts is exactly the shape of run that can walk
    // to the end having skipped a span, and the flag would still be set at the
    // end of it. Short of that count the tree stays, and the copy is merely
    // large rather than lossy.
    let mut migrated: Vec<&str> = MIGRATED.to_vec();
    for (tree, t) in &candidates {
        let sled_rows = db.open_tree(tree.as_bytes())?.len() as u64;
        let redb_rows = redb.kv_len(*t).unwrap_or(0);
        let ok = redb_rows >= sled_rows;
        println!(
            "evidence: {tree} sled={sled_rows} redb={redb_rows} -> {}",
            if ok {
                "DROP"
            } else {
                "KEPT (redb is short; this is not a migration that finished)"
            }
        );
        if ok {
            migrated.push(tree);
        }
    }
    println!();

    // Row counts cannot see a fact that is in sled and not in redb, and the
    // redb backfill did strand some (derivatives and index-displaced facts;
    // read-repair has been recovering them one read at a time). So before a
    // tree is dropped from the copy, every one of its keys is checked in redb
    // and anything missing is copied there first. Only missing keys are
    // written: a key redb already holds is never overwritten from sled.
    for tree in migrated.clone() {
        let missing = reconcile(&db, &redb, tree, apply)?;
        println!(
            "reconcile: {tree} missing_in_redb={missing}{}",
            if missing == 0 {
                ""
            } else if apply {
                " (copied into redb)"
            } else {
                " (would be copied with --apply)"
            }
        );
    }
    println!();

    let mut keep: Vec<(String, usize)> = Vec::new();
    let mut drop_: Vec<(String, usize)> = Vec::new();
    for name in db.tree_names() {
        let label = String::from_utf8_lossy(&name).to_string();
        if label == "__sled__default" {
            continue;
        }
        let len = db.open_tree(&name)?.len();
        if migrated.contains(&label.as_str()) {
            drop_.push((label, len));
        } else {
            keep.push((label, len));
        }
    }
    keep.sort();
    drop_.sort();

    println!("trees that stay ({}):", keep.len());
    for (n, l) in &keep {
        println!("   {l:>12} rows  {n}");
    }
    println!("trees already migrated to redb ({}):", drop_.len());
    for (n, l) in &drop_ {
        println!("   {l:>12} rows  {n}");
    }
    println!();

    let sled_facts = drop_
        .iter()
        .find(|(n, _)| n == "emem.facts")
        .map(|(_, l)| *l)
        .unwrap_or(0);
    let redb_index = redb.index_len().unwrap_or(0);
    println!("sled emem.facts rows : {sled_facts}");
    println!("redb index rows      : {redb_index}");
    if (redb_index as usize) < sled_facts {
        println!(
            "WARNING: redb holds fewer index rows than sled holds facts. That is not proof of \
             loss (the index is keyed differently) but it is a reason to look before applying."
        );
    }

    if !apply {
        println!();
        println!(
            "dry run. Re-run with --apply to write {}.",
            out_path.display()
        );
        return Ok(());
    }

    anyhow::ensure!(
        !out_path.exists(),
        "{} already exists; move it aside first",
        out_path.display()
    );
    println!();
    println!("writing {} …", out_path.display());
    let out = sled::open(&out_path)?;
    let mut copied = 0usize;
    for (name, _) in &keep {
        let src = db.open_tree(name.as_bytes())?;
        let dst = out.open_tree(name.as_bytes())?;
        let mut n = 0usize;
        for kv in src.iter() {
            let (k, v) = kv?;
            dst.insert(k, v)?;
            n += 1;
        }
        dst.flush()?;
        println!("   copied {n:>12} rows  {name}");
        copied += n;
    }
    out.flush()?;
    drop(out);

    println!();
    println!("copied {copied} rows into {}", out_path.display());
    println!("The original is untouched. To adopt it, with the server stopped:");
    println!(
        "   mv {} {}.migrated",
        cache_path.display(),
        cache_path.display()
    );
    println!("   mv {} {}", out_path.display(), cache_path.display());
    println!("Roll back by reversing those two moves.");
    Ok(())
}

/// Copy into redb every row of `tree` that redb does not already hold, and
/// return how many there were. With `apply` false it only counts.
fn reconcile(
    db: &sled::Db,
    redb: &emem_cache::redb_facts::RedbFacts,
    tree: &str,
    apply: bool,
) -> Result<usize> {
    const BATCH: usize = 1000;
    let src = db.open_tree(tree.as_bytes())?;
    let kv_table = MIGRATING.iter().find(|(n, _)| *n == tree).map(|(_, t)| *t);
    let mut missing = 0usize;
    let mut fact_rows: Vec<emem_cache::redb_facts::FactRow> = Vec::new();
    let mut kv_rows: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    for kv in src.iter() {
        let (k, v) = kv?;
        let present = match (tree, kv_table) {
            ("emem.facts", _) => redb.contains_fact(&k)?,
            ("emem.canonical_index", _) => redb.contains_index(&k)?,
            (_, Some(t)) => redb.kv_get(t, &k)?.is_some(),
            _ => true,
        };
        if present {
            continue;
        }
        missing += 1;
        if !apply {
            continue;
        }
        match (tree, kv_table) {
            ("emem.facts", _) => fact_rows.push((k.to_vec(), v.to_vec(), None)),
            // An index row points at a fact by cid. It is written with that
            // fact's bytes from redb; the facts pass runs first, so a fact
            // only sled held is already there.
            ("emem.canonical_index", _) => {
                if let Some(cbor) = redb.get_fact(&v)? {
                    fact_rows.push((v.to_vec(), cbor, Some(k.to_vec())));
                } else {
                    println!("   WARNING: index key points at a fact redb does not hold; left in the sled original");
                }
            }
            (_, Some(_)) => kv_rows.push((k.to_vec(), v.to_vec())),
            _ => {}
        }
        if fact_rows.len() >= BATCH {
            redb.put_batch(&fact_rows, false)?;
            fact_rows.clear();
        }
        if kv_rows.len() >= BATCH {
            if let Some(t) = kv_table {
                redb.kv_put_batch(t, &kv_rows, false)?;
            }
            kv_rows.clear();
        }
    }
    if apply {
        redb.put_batch(&fact_rows, true)?;
        if let Some(t) = kv_table {
            redb.kv_put_batch(t, &kv_rows, true)?;
        }
    }
    Ok(missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconcile_copies_only_what_redb_lacks_and_dry_run_writes_nothing() {
        let dir = std::env::temp_dir().join(format!("emem-slim-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let redb = emem_cache::redb_facts::RedbFacts::open(dir.join("facts.redb")).unwrap();
        let db = sled::Config::new().temporary(true).open().unwrap();
        let facts = db.open_tree("emem.facts").unwrap();
        facts
            .insert(b"only-in-sled", b"sled-bytes".to_vec())
            .unwrap();
        facts
            .insert(b"in-both", b"stale-sled-bytes".to_vec())
            .unwrap();
        redb.put_batch(&[(b"in-both".to_vec(), b"redb-bytes".to_vec(), None)], true)
            .unwrap();

        assert_eq!(reconcile(&db, &redb, "emem.facts", false).unwrap(), 1);
        assert!(
            !redb.contains_fact(b"only-in-sled").unwrap(),
            "a dry run must not write"
        );

        assert_eq!(reconcile(&db, &redb, "emem.facts", true).unwrap(), 1);
        assert_eq!(
            redb.get_fact(b"only-in-sled").unwrap().as_deref(),
            Some(&b"sled-bytes"[..])
        );
        assert_eq!(
            redb.get_fact(b"in-both").unwrap().as_deref(),
            Some(&b"redb-bytes"[..]),
            "redb's copy must never be overwritten from sled"
        );
        assert_eq!(reconcile(&db, &redb, "emem.facts", false).unwrap(), 0);
        drop(redb);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
