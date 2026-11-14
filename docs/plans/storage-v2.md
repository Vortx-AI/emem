# Storage v2: the log holds the facts, redb holds the index

Written 14 November 2026 against commit 0d613cda and the production host.
Every number below says where it came from. "Measured" means read from
the file system, the server journal or a public route on that day.
"Estimated" means computed from the code and from measured inputs, with
the method stated. Nothing here opened `facts.redb`: the server holds its
lock.

## Recommendation

Keep redb. Change what we keep in it.

redb is not the wrong engine. The layout is wrong. `facts.redb` stores a
second copy of every fact body that the transparency log already holds,
and a per-fact Merkle proof that is a pure function of the log entry. Both
tables are keyed by a random BLAKE3 hash, in a file four times the size
of the page cache. So every fact in a cold write costs about four random
disk operations (a leaf read and a page write in each table), and the file
grows five times faster than the log that holds the same facts.

The fix is key-value separation, as in WiscKey: the log is the body
store, and redb keeps only small index rows. Proofs are rebuilt from the
batch on read. Moving to an LSM (fjall) or to LMDB does not remove the
duplicate bodies or the stored proofs, and costs a migration of the whole
file. An engine change is worth looking at again only for the one table
that stays random-keyed (the locator), and only if it stops fitting in
memory at ten times today's size.

## Phased plan

| Phase | What changes | Flag | Expected effect | Rollback |
|---|---|---|---|---|
| 0 | Land the batched lookups and index rows (e8023e64, the compiling version of abda4361). Read the table row counts (`len()` is O(1) in redb) at the next boot. | none | A cold 4,845-fact plot stops paying a transaction per fact. Real row counts replace the estimates below. | Revert the commit. |
| 1 (built) | Each attestation writes a fact locator (log segment, offset, byte span, leaf index) and its sorted leaves in 1,024-leaf chunks. Under `on`, per-fact proof rows stop. Receipts rebuild the proof from one chunk. | `EMEM_DERIVED_PROOFS=off\|shadow\|on`, default `off` | The proof table stops growing. Per fact, a random-keyed row of 0.2 to 1.3 KB becomes a 61-byte random-keyed row plus 32 bytes appended. | Unset the flag. Facts written while it was on keep their proofs through the locator. |
| 2 | Write the log first, then the index rows. Serve fact bodies from the log by locator (hash-checked, falling back to `FACTS`). Stop writing bodies into `FACTS`. Backfill locators and leaves for every attestation in the existing log in one pass, so older proofs can be rebuilt too, and copy into a residual table the bodies that are not in the log. | `EMEM_FACT_BODIES=redb\|log` (not built) | A cold write's body cost becomes the log append it already pays. A plot's facts are read back as one contiguous log range instead of one random read each. A crash or a dropped caller can no longer leave a fact in the store that the log does not have. | Until bodies stop being written, unset the flag. After that, `FACTS` can be rebuilt from the log, which holds every body. |
| 3 | Compact: copy the live tables into a new file and drop `FACTS` and `PROOFS`. | maintenance window | Estimated 5 to 12 GB file against 128.8 GB, inside the page cache. | Keep the old file until the new one has served a week. |
| 4 (only if needed) | Move the locator table alone to an LSM. | gate: locator table larger than the page cache, or locator inserts above 10% of write time | Burst inserts become sequential writes. | Dual-write, as in phase 1. |

## The three numbers that decide it

1. **redb grows 5.3 times faster than the log that holds the same facts.**
   From 29 September 19:50 to 14 November, `facts.redb` went from 94.5 GB
   to 128.8 GB (+34.3 GB) and the log from 21.2 GB to 27.7 GB (+6.4 GB).
   Source: the 29 September size from the disk-write profile of that day;
   the log sizes by summing `merkle.log.N` sizes in segment order up to
   each date; today's sizes from `ls`. Measured.
2. **A large batch costs 0.6 to 1.3 ms per fact in the redb commit alone,
   and the log writes the same bytes sequentially.** The journal's slow
   commit lines on 13 and 14 November: 174,081 rows (143 MB) committed in
   116.3 s, 176,129 rows in 100.5 s, 151,553 rows in 110.0 s, 4,845 rows
   (4.05 MB) in 6.1 s. That is the arithmetic of one random page write per
   row per random-keyed table at the volume's measured 3,018 read IOPS
   (15 September repair walk). The slowest log append in the same 15 hours
   was 1.5 s, across 6,160 appends. Measured.
3. **The hot set would fit in memory; the file does not.** The file is
   128.8 GB against about 33 GB of page cache on a 61 GB host (measured).
   Locators at 61 bytes per fact for 25 to 35 million facts, plus the
   canonical and multi-attester indexes, come to an estimated 4 to 8 GB
   (estimated, row sizes from the code, counts from the section below).

## Where the 128.8 GB goes

The file is 128,849,022,976 bytes: exactly 30 redb regions of 4 GiB plus
one header page (measured with `stat`; region size from redb 4.4
`page_manager.rs`). redb grows by whole regions, so the size is an upper
bound on live data, not a measure of it.

What the log says about the facts (measured, 14 November):

- 3,333,499 entries in 1,755 segments, 27.66 GB (`/v1/log/sth`, `ls`).
- A sample of 7,680 entries in 30 blocks spread across the log
  (`/v1/log/entries?summary=1`, 30 requests): 97.8% attestations, 2.2%
  memory writes. 96% of attestations carry one fact. Those small entries
  hold only about 6.2 GB of the 27.7 GB. The rest is large batches (EUDR
  plots of thousands of facts) and a few very large single facts
  (`field.series` at 0.5 to 0.7 MB, loss-year histograms at 52 KB).
- Three entries fetched in full: a one-fact attestation is 1,130 bytes,
  of which the fact is 640 and the envelope 490. Each fact's bytes are a
  contiguous slice of the entry's CBOR, and for all three the slice is
  byte-identical to what `/v1/facts/<cid>` serves and hashes to the cid.
  This is the premise of phase 2, checked on production data.

Per table (estimated unless marked):

| Table | Rows | On disk per row | Size | Basis |
|---|---|---|---|---|
| `emem.facts` | 25 to 35 M | 52-byte key, 0.65 to 1.15 KB body, about 70% leaf fill | 40 to 55 GB | 25.9 GB of fact bodies in the log (log bytes less 490 bytes of envelope and 36 of framing per entry), plus bodies of cancelled writes that never reached the log |
| `emem.fact_proofs` | 22 to 30 M | 52-byte key, 0.2 to 1.3 KB proof | 30 to 40 GB | 19,976,065 rows measured on 14 September; that backfill took the file from about 25 GB to 53.7 GB together with the multi-attester rows |
| `emem.canonical_index` | at most the fact count | about 110 bytes | 1 to 4 GB | row layout |
| `emem.multi_attester_index` | 2,391,244 on 14 September (measured) | about 110 bytes, plus 55 per extra cid | 0.5 to 2 GB | row layout |
| `emem.state_records` | about 385 a day | about 10 KB | under 1 GB | rate measured 29 September |
| free and fragmented pages | | | the remainder, roughly 30 to 55 GB | not measurable without a page walk or a compaction |

Why a proof row is so large: `MerkleProof` serialises each `[u8; 32]` as
a CBOR array of 32 integers, about 63 bytes (the `batch_root` field of the
sampled entries measured 63 to 65 bytes). A proof is about 182 bytes plus
64 per tree level: 180 bytes for a one-fact batch, about 1.0 KB for a
4,845-fact batch, about 1.3 KB for a 176,000-fact batch. Every row repeats
the batch root and the log entry hash.

Large values waste space too: redb allocates pages in powers of two, so a
0.7 MB `field.series` body takes a 1 MiB allocation (redb 4.4
`allocate_helper` rounds the page count up to a power of two).

## Access patterns and hot paths

| Pattern | Path | Table and key | Cost when cold |
|---|---|---|---|
| Point read by cid | `/v1/facts/<cid>`, token resolve, receipt verify, `get_many` in `sled_hot.rs` | `FACTS`, random key | one random read |
| Recall by place | `lookup_many`, `scan_cell` then `get_many` | `INDEX` prefix (clustered by cell), then `FACTS` (random) | one read per cell, then one per fact |
| Prefix and full scans | `scan_cell`, `index_page`, `kv_scan_prefix` on multi-attester | `INDEX`, `MULTI`, clustered | sequential within a cell |
| Proof in a receipt | `proof_for_cid` from `sign_receipt` (`server.rs`), once per receipt; `/v1/log/inclusion?fact_cid=` | `PROOFS`, random key | one random read |
| Multi-attester | `append_multi_attester` per write; contradiction scans | `MULTI`, clustered | read and write per key |
| Cold write burst | `put_attestation`: `put_many` (holder lookup in `INDEX`, holder body from `FACTS`, insert into `FACTS` and `INDEX`, durable commit), log append (sequential, fsync), `persist_fact_proofs` and `append_multi_attester` | `FACTS` and `PROOFS` random, `INDEX` and `MULTI` clustered | about four random operations per fact |
| EUDR reuse | `eudr_reuse_stored` reads the stored fact at each key before re-signing | `INDEX` then `FACTS` | one random read per fact |

What the journal shows for writes over 15 hours (13 November 12:00 to
14 November 03:35, 6,160 attestations, `emem::latency` debug lines,
measured):

| Stage | p50 | p99 | max | total |
|---|---|---|---|---|
| `storage.put_facts` | 3.0 ms | 92.7 ms | 303 s | 1,589 s |
| `storage.log_append` | 2.8 ms | 22.1 ms | 1.5 s | 29 s |
| `storage.index_rows` | 1.9 ms | 44.8 ms | 8.5 s | 45 s |

Small writes are fine. The time is in large batches. A batch holds redb's
single writer for its whole duration, so everyone else queues: at 16:38:18
on 13 November, nine other writes finished at the same instant after
waiting 86.2 s each behind a 151,553-fact commit.

## Options compared

**redb as used now.** No migration. Every fact pays about four random
operations, and that is already the cold regime, so ten times the facts
does not change the per-fact cost much. It changes the file: at today's
ratio about 1.2 TB, and a compaction that needs the server stopped for
hours. No new unsafe code, no new dependency.

**redb with key-value separation and derived proofs (recommended).**
Bodies stay in the log, which already has them and already pays for
writing them. redb keeps the clustered indexes and a locator per fact.
Migration, estimated: dual-write first (phase 1 and 2 flags); one
sequential pass over the 27.7 GB log to backfill locators, at the 50 MB/s
the batch-root index build already uses, about 10 minutes of reading and
2 to 4 hours of wall time under the 25% disk duty cycle the backfills use;
one walk of `FACTS` to find bodies the log lacks, at the 17 to 27 MB/s a
redb walk measured on this volume, about 1 hour of IO and 3 to 5 hours
paced; then a compaction in a maintenance window. Rollback is always
possible, because the log holds every body and every batch. No new unsafe
code and no new dependency.

**An LSM: fjall 3.1.13 (pure Rust, MSRV 1.90, from crates.io).** Random-key
inserts become sequential writes, which is the right shape for the locator
table. But the bodies would still be stored twice unless we also adopt
the log as the body store, which is the recommended change anyway. The
migration copies about 60 million rows (about 80 GB) out of a redb walk
at the same 17 to 27 MB/s: 1 to 1.5 hours of IO, 4 to 6 hours paced, plus
a dual-write period. The risk is a new engine on a box where each of the
last three storage problems (the sled read convoy, the redb repair walk,
the deferred close) was found only in production. Our crates would need
no unsafe code for its API (not verified against fjall 3.x source). Its
dependency tree is new to the build.

**LMDB through heed 0.23.1.** A copy-on-write B-tree like redb, so the
same random-key write cost and the same file-larger-than-memory problem.
Reads are faster when cached, which they are not. heed marks opening an
environment `unsafe` (since 0.20), which every crate here forbids, so it
would need a crate exempt from `#![forbid(unsafe_code)]`: a policy change.
It also brings a C library into the build (the CI image has a C compiler).
Not recommended.

## Phase 1 as built

Commit on branch `storage-v2`. `crates/emem-storage/src/fact_locator.rs`
and the wiring in `put_attestation` and `proof_for_cid`.

- `off` (default): the code path is the old one. No locator or batch table
  is created, no new row is written, and a proof is read from the proof
  table only. The only addition is one O(1) read at open that finds no
  locator table.
- `shadow`: the locators and leaves are written beside the stored proofs.
  Every proof read also rebuilds the proof and compares the CBOR byte for
  byte, logging `derived proof differs from the stored proof` with counts.
- `on`: no per-fact proof rows. A proof missing from the proof table is
  rebuilt from the locator.

How a write finds the spans: the attestation is serialised once (the same
bytes `AttestationLog::append` writes) and appended with `append_cbor`. A
small CBOR walk finds each fact's span, and each span must hash to the cid
`put_many` returned for that fact. If any check fails, the attestation is
appended as before, no locator is written, and its proofs are stored.

How a proof is rebuilt: the leaves are stored sorted, in chunks of 1,024
under a key that grows with the log position, so they append at the end
of the table. The header holds the version, the root, the log entry hash,
the leaf count and, for batches larger than a chunk, each chunk's subtree
node. A proof reads the header and one 32 KiB chunk and does about 2,200
hashes, whatever the batch size (estimated). It must fold to the stored
root, and the leaf must equal the fact's digest; otherwise the receipt
carries no proof, as it would for a fact with no stored proof.

Rollout: set `EMEM_DERIVED_PROOFS=shadow` in the unit's `docker run`
(variables reach the container only through `-e`), read the comparison
counts for a day, then set `on`. Rollback: unset it. A store that ever had
locators finds them at open and keeps serving those proofs.

Tests (in the same file): every derived path equals
`merkle_root_and_paths{,_v1}` for every leaf of batches of 1 to 41 leaves
with chunks of 2, 4 and 8, and for 1,024 to 2,349 leaves with the real
chunk size, under both hashing rules; `on` serves byte for byte the proof
`off` stored, for 1, 37 and 1,100 facts; `shadow` writes both and they
agree; the locator's span in the segment file is the stored body; turning
the flag off keeps the proofs written while it was on; spans that do not
hash to their cids are refused; a wrong leaf, a wrong chunk or a wrong
root yields no proof.

## Three defects found on the way

1. **abda4361 does not compile.** It adds a second `RedbFacts::lookup_many`
   beside the existing one (E0592). The landing run found the same error
   and amended it into e8023e64, which merges the two into one sorted,
   generic lookup. This branch now sits on 0d613cda, which carries that
   fix. `dds-v4` and `memstd-v2` still carry the version that does not
   build.
2. **A dropped caller leaves facts in the store that the log never
   receives.** `put_many` runs on the blocking pool. When the caller's
   future is dropped, the blocking task still commits, and the log append,
   the proofs and the multi-attester rows never run. On 13 November at
   16:32:51 the MCP layer ended an `emem_eudr_dds` call at its 32 s
   budget. At 16:38:18 a 151,553-fact commit landed. The same pattern
   repeated at 22:52:15 (176,129 facts) and 03:31:32 (174,081 facts). No
   log segment written in that window is larger than 49 MB, and each of
   those records would have been 125 to 145 MB. These facts hold their
   canonical slots, so recall serves facts that have no log entry and no
   proof. Inferred for the last two from the segment sizes; the first is
   confirmed by the MCP timeout line.
3. **EUDR re-signs the same plot because reading it back is too slow.**
   The reuse check has a 20 s budget and found 7,168 to 13,312 of 51,200
   stored facts per band before it ran out (journal, three runs of one
   plot). The rest are signed again: about 150,000 to 176,000 new facts,
   roughly 0.4 GB of redb, per run. Phase 2 reads a plot's facts as one
   contiguous log range.

## Not verified

- The per-table split and the free-page share. Phase 0's row counts settle
  the counts. The bytes need a page walk (redb `stats()` reads every page,
  about as long as the repair walk) or the compaction itself.
- The fact count, 25 to 35 million, is an estimate from log bytes.
- Phase 1 in production. It passed unit tests only. The cost of a derived
  proof is estimated, not measured.
- That every body in `FACTS` other than the cancelled writes is also in
  the log. Three facts were checked byte for byte; phase 2's residual pass
  checks all of them.
- fjall's internals and its current API for separated values.
- Whether the gp3 volume is capped at 3,000 IOPS or bound by latency (the
  question left open on 15 September). If capped, provisioned IOPS speed
  every option here.

## Decisions for the owner

1. The orphaned facts from cancelled writes: about 500,000 in 11 hours on
   13 November, with no log entry and no proof. Leave them, mark them, or
   take them out of the canonical index. Their attestations cannot be
   recovered. Separately, whether to make the write path finish what it
   started when its caller goes away (phase 2's log-first order also
   closes it).
2. A maintenance window for the compaction in phase 3, estimated at
   several hours with the server stopped, or a second-file cutover that
   needs about 15 GB of free disk and a short restart.
3. Whether log segments become the body store for good. Today they are
   never deleted. After phase 2 they could not be archived off the box
   without taking the bodies with them.
4. Phase 2 makes index rows non-durable on their own and replays them from
   the log after a crash, which changes the durability argument in
   `docs/protocol.md`. That needs your sign-off before it is built.
5. Whether to raise the volume's provisioned IOPS while the phases land.
