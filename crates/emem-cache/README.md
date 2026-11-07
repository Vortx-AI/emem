# emem-cache

`emem-cache` is the fact store of an emem node. It keeps two maps. The first
map goes from an address `(cell, band, tslot)` to a fact id. The second map
goes from a fact id to the canonical CBOR bytes of the fact. It also makes
the fact id: BLAKE3 over those bytes, in lower-case base32 without padding.

You need this crate when you change how a node stores or finds facts, or
when you run a node with a large store. To only use emem, you do not need
it. `POST /v1/recall` reads this store for you.

| | |
|---|---|
| Version | 2.4.4 (workspace version in the root `Cargo.toml`) |
| Used by | `emem-storage`, `emem-api-rest`, `emem-primitives`, `emem-cli` |
| Storage engines | redb 4.4 for facts (default), sled 0.34 for the other trees |
| Tests | 14 unit tests, inline in `src/` (7 `#[test]`, 7 `#[tokio::test]`) |

## Try it

The live responder reads both maps on every recall. Ask for one band at one
cell. The first map gives the fact id:

```sh
curl -s -X POST https://emem.dev/v1/recall \
  -H 'Content-Type: application/json' \
  -d '{"cell":"defi.zb592.nUkO.zEzE","bands":["copdem30m.elevation_mean"]}' \
  | jq -c '[.facts[] | {band, tslot, fact_cid}]'
# [{"band":"copdem30m.elevation_mean","tslot":0,
#   "fact_cid":"qejpz7cgvk347ordd6vrdtmwzprqlzcnkmotsjton2x4y2jjbplq"}]
```

The second map gives the fact for that id:

```sh
curl -s https://emem.dev/v1/facts/qejpz7cgvk347ordd6vrdtmwzprqlzcnkmotsjton2x4y2jjbplq \
  | jq -c '{kind, cell, band, tslot, value, unit}'
# {"kind":"primary","cell":"defi.zb592.nUkO.zEzE","band":"copdem30m.elevation_mean",
#  "tslot":0,"value":3539.0422,"unit":"m"}
```

Both commands ran on 7 November 2026. We wrapped the output to fit the page.
To check that the id is the BLAKE3 hash of the stored bytes, follow the
try-it section of [emem-fact](../emem-fact/README.md).

To run the crate tests from a checkout (needs Rust 1.91 or later):

```sh
cargo test -p emem-cache
```

We did not run `cargo test` for this page, because the host had no Rust
toolchain. The count of 14 tests comes from the test attributes in `src/`.

## What each file does

| File | Contents |
|---|---|
| `lib.rs` | The `Cache` trait (`lookup_many`, `get_many`, `put_many`, `tier_of`), `CanonicalKey`, `Tier` and `CacheError` |
| `sled_hot.rs` | `SledHotCache`, the only type with an `impl Cache`. It opens the sled store and the redb file, and copies old sled rows to redb in the background. It also holds `fact_cid_of` and `fact_canonical_cbor` |
| `redb_facts.rs` | `RedbFacts`, the redb file `facts.redb`. It holds the fact index, the fact bodies and three byte-to-byte tables (`KvTable`) |

## How a node stores facts

`emem-storage` opens the store at `<dir>/cache.sled`. The redb file goes
next to it, at `<dir>/facts.redb`.

| Table | Key | Value | Engine |
|---|---|---|---|
| `emem.canonical_index` | `cell`, byte 0, `band`, byte 0, `tslot` as 8 big-endian bytes | fact id (text) | redb |
| `emem.facts` | fact id (text) | canonical CBOR of the fact | redb |
| `emem.fact_proofs` | fact id | CBOR of the Merkle inclusion proof, written by `emem-storage` | redb |
| `emem.multi_attester_index` | the same address key | CBOR list of every fact id at that address, from all signers | redb |
| `emem.state_records` | state id | canonical CBOR of a state record | redb |
| memory trees, entities, attesters | many | many | sled |

Older nodes kept the first four tables in sled. When the redb file is new,
the node copies those sled rows into it in the background. A read that
misses in redb also looks in sled until the node finishes the copy of the fact index.
After that, the node reads facts only from redb.

A derivative fact has no `(cell, band, tslot)` address. The node stores its
bytes under its fact id, but it adds no row to the index.

## Rules the store keeps

- The fact id is BLAKE3 over `ciborium` bytes of the `Fact`. Do not change
  how `fact_to_cbor` writes a fact. A change gives a new id to every fact
  and breaks every receipt that cites one. The root
  [AGENTS.md](../../AGENTS.md) says which docs must change with it.
- An address belongs to the key that wrote it first. The node stores a fact from a
  different signer at the same address under its own id. That fact does
  not replace the index row. The same signer can replace its own
  reading.
- Every sled call runs on the blocking thread pool, under a semaphore. The
  comments in `sled_hot.rs` record the outages that caused this rule.
- A write to redb is durable when the commit returns
  (`Durability::Immediate`).
- Call `RedbFacts::close` before the process exits. Without it, redb marks
  the file for repair, and the next start walks the whole file.

## Settings

The node reads these environment variables. The defaults come from the code
on 7 November 2026.

| Variable | Default | Effect |
|---|---|---|
| `EMEM_HOT_BACKEND` | `redb` | `sled` keeps facts in sled, the old layout. Use it only to roll back |
| `EMEM_SLED_CACHE_BYTES` | `8g` | sled page cache. Clamped to 256 MiB to 64 GiB |
| `EMEM_SLED_FLUSH_MS` | `200` | how often sled makes its log stable. Clamped to 50 to 5,000 |
| `EMEM_SLED_BLOCKING_CONCURRENCY` | 2 × CPU count, 4 to 64 | how many sled calls run at the same time. Clamped to 1 to 512 |
| `EMEM_REDB_CACHE_BYTES` | 2 GiB | redb read cache. Clamped to 64 MiB to 32 GiB |
| `EMEM_REDB_CLOSE_WAIT_S` | `20` | how long `close` waits for open write transactions. 0 to 300 |
| `EMEM_REDB_REPAIR_HEARTBEAT_S` | `60` | how often a redb repair writes a progress line to the log. 5 to 3,600 |
| `EMEM_SCAN_CELL_LIMIT` | `10000` | maximum index rows that one cell scan returns |
| `EMEM_HOT_BACKFILL_BATCH` | `1000` (facts), `2000` (other tables) | rows per copy batch. 50 to 20,000 |
| `EMEM_HOT_BACKFILL_PAUSE_MS` | `250` (facts), `100` (other tables) | pause between copy batches |
| `EMEM_HOT_BACKFILL_DUTY` | `0.25` | maximum share of disk time for the copy of the other tables. 0.05 to 1.0 |
| `EMEM_HOT_BACKFILL_MIN_FREE_BYTES` | `4g` | the fact copy pauses while free disk space is below this |

[docs/self-host.md](../../docs/self-host.md) lists the two sled settings
with the other node settings.

## What does not ship

The crate description and the `lib.rs` header name three tiers: hot (sled),
warm (Parquet) and cold (IPLD or IPFS). Only the hot tier ships. No code
writes Parquet or IPFS, and nothing moves facts between tiers.
`tier_of` returns `Hot` for a stored fact and `None` for any other.

## Read more

- [emem-fact](../emem-fact/README.md): the `Fact` type and its canonical
  CBOR.
- [emem-storage](../emem-storage/README.md): the `Storage` layer that opens
  this cache, fetches on a miss and signs receipts.
- [docs/protocol.md](../../docs/protocol.md), section 3: fact ids.
- [docs/self-host.md](../../docs/self-host.md): settings for your own node.
