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
    let mut migrated: Vec<&str> = MIGRATED.to_vec();
    for (tree, t) in MIGRATING {
        let done = redb.table_backfill_done(*t);
        println!(
            "redb: {tree} backfill_done={done}{}",
            if done {
                " (dead in sled, will be dropped)"
            } else {
                " (still moving; kept)"
            }
        );
        if done {
            migrated.push(tree);
        }
    }
    println!();

    println!("opening sled (this is the 22 minutes the server pays at every boot)…");
    let started = std::time::Instant::now();
    let db = sled::open(&cache_path)
        .with_context(|| format!("open sled at {}", cache_path.display()))?;
    println!("opened in {:.1}s", started.elapsed().as_secs_f64());
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
