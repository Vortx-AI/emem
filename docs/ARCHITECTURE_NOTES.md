# Architecture notes: where the load-bearing pieces live

This page is a code map. For each mechanism that signing, storage and
the transparency log depend on, it names the crate, file and function
that is canonical. It started as the orientation map for the
shared-memory upgrade (Phases 0 to 3).

The map names functions, not line numbers. Line numbers in
`crates/emem-api-rest/src/lib.rs` move by thousands between releases.
To find a symbol, search for it:

```sh
grep -rn "fn sign_receipt_v1_inner" crates/
```

We checked all symbols on this page against the source of version
2.4.4 on 7 November 2026. Live values come from https://emem.dev on the
same day.

## Crate layers

The workspace has 19 crates (`Cargo.toml`, `[workspace] members`). From
the bottom layer to the top:

| crate | what it holds |
|---|---|
| `emem-core` | `Tslot`, key types (`AttesterKey`, `KeyEpoch`, `Signature`), registries, error codes |
| `emem-codec` | text codecs: cell64, tslot text, cid64, vec64, grid |
| `emem-attest` | pure hashing: preimages, Merkle v0 and v1, the RFC 6962 log tree. No I/O. |
| `emem-trace` | the `emem.os_trace.v1` schema, its verifier and drift-anchor scoring. Pure. |
| `emem-fact` | wire structs (`Fact`, `Attestation`, `Receipt`, `EdgeFact`, `Scope`) and canonical CBOR |
| `emem-claim` | the structural claim algebra (band, op, value, tslot) and its evaluator |
| `emem-cache` | the hot fact store (redb for facts, sled for other trees) and fact CID derivation |
| `emem-fetch` | upstream connectors, with bundled GeoNames and POI data (`include_bytes!`) |
| `emem-cubes` | loaders for the AgriSynth 1792-D bootstrap cubes |
| `emem-storage` | `MaterializingStorage`, `AttestationLog`, `AttesterRegistry` and receipt signing (`Server`, `ResponderIdentity`) |
| `emem-primitives` | recall, diff, verify and `memory_*` handlers, the memory ACL, contradictions |
| `emem-intent` | the typed `Intent` grammar and a heuristic planner |
| `emem-mcp` | the static MCP tool catalog (116 descriptors). No transport. |
| `emem-api-rest` | the axum router: REST routes, `/mcp` JSON-RPC dispatch, well-known files, the write gate |
| `emem-cli` | the `emem-server` binary (key load and persist) and a receipt-verify CLI |
| `emem-guard` | a separate verdict server for AI inference hooks. It shares no code path with the responder. |
| `emem-airgap` | an air-gapped node: a folder in, a folder out, no network |
| `emem-scorecard` | a MemoryAgentBench-style scorecard runner (leaf) |
| `emem-sleep-agent` | an opt-in sleep-time rewrite and merge agent (leaf) |

Every hash is BLAKE3. Every payload uses deterministic CBOR (ciborium,
RFC 8949). Every CID is base32-nopad-lowercase. Every signature is
Ed25519. `preimage_version` on `Attestation` and on `Receipt` selects
the rule (serde default 0 is legacy), so old envelopes still verify.

## Canonical payload serialization

- `to_canonical_cbor<T: Serialize>` in `crates/emem-fact/src/cbor.rs`
  encodes any value to deterministic CBOR. Struct fields serialize in
  declaration order. The caller must sort a freeform map before it
  encodes it.
- `blake3_32` and `base32_prefix` are in the same file.
- The custom CBOR tags are `TAG_EMEM_CELL = 65000`,
  `TAG_EMEM_TSLOT = 65001`, `TAG_EMEM_VEC64 = 65002` and
  `TAG_IPLD_CID = 42`, at the top of `cbor.rs`.

A new payload kind must use `to_canonical_cbor` and get its own
schema version. Never change the encoding of a kind that exists.

## CID computation

- Fact CIDs: `fact_cid_of(fact)` in `crates/emem-cache/src/sled_hot.rs`.
  The CID is base32-nopad-lowercase of `blake3(canonical_cbor(fact))`.
  It is always 52 characters (the full 256 bits). `SledHotCache::put_many`
  computes the same CID again when it writes the fact. A live NDVI fact at
  Mount Fuji on 7 November 2026 had the CID
  `banvya7hgenuowjggkgguezvltxtefp6jgihocwkgcbddyvqv6bq`.
- Typed CID newtypes are string wrappers made by the `cid_newtype!`
  macro in `crates/emem-fact/src/cid.rs`: `FactCid`, `RegistryCid`,
  `SchemaCid`, `ReasonCid`, `BatchCid`, `CoverageCid`, `EdgeCid` and
  `StateCid`.
- Edge CIDs: `impl EdgeFact` in `crates/emem-fact/src/edge.rs`.
- `cid64` is 13 base32 characters of the first 8 bytes (`to_cid64` in
  `crates/emem-codec/src/cid64.rs`). It is for inline text only. It
  never appears in canonical CBOR.
- Bundle CIDs use the first 16 bytes of the hash
  (`crates/emem-primitives/src/memory_bundle.rs`). Fact CIDs use all 32.

## Receipt signing path

- Every public signer (`sign_receipt`, `sign_receipt_field`,
  `sign_receipt_with_scope`, `sign_receipt_with_as_of`,
  `sign_receipt_full`, `sign_receipt_with_edges`) calls the one private
  `sign_receipt_v1_inner` in `crates/emem-storage/src/server.rs`. The
  name is old: it signs with `receipt_preimage_v2` and sets
  `preimage_version` to 2.
- The preimages are in `crates/emem-attest/src/lib.rs`.
  `receipt_preimage_v1` stays there for receipts signed under v1. The
  hash stream opens with the 17 bytes `"emem.preimage.v1\x00"`, then
  `u32-LE(len(domain))`, then the domain. Each segment is then
  `tag || u32-LE(len) || bytes`.
- The receipt segment tags are in `mod receipt_tag`:

  | tag | name | note |
  |---|---|---|
  | `0x01` to `0x09` | `REQUEST_ID`, `SERVED_AT`, `SCOPE`, `AS_OF`, `EDGES`, `MANIFEST`, `PRIMITIVE`, `CELLS`, `FACT_CIDS` | v1 and v2 |
  | `0x0a` | `FIELD` | only when the receipt has a field binding (`aoi_cid`, `derivation_cid`) |
  | `0x0b` | `MERKLE` | v2 only: the digest of `merkle_proof`, or an absence marker |

  The `MERKLE` segment makes proof stripping visible. Under v1 a relay
  could delete `merkle_proof` and the signature still verified.
- The Merkle root over `fact_cids`: v1 is RFC 6962 style,
  `blake3(0x00 || leaf)` and `blake3(0x01 || l || r)`, in
  `merkle_root_v1`. The legacy v0 rule is `merkle_root`. Callers sort
  the leaves and call `has_adjacent_duplicate` to refuse duplicates (the
  CVE-2012-2459 pattern) before they compute the root.
- At signing time, `sign_receipt_v1_inner` gets the inclusion proof for
  the first cited fact through `proof_for_cid`. `persist_fact_proofs`
  stored that proof when the responder wrote the fact. A receipt carries one
  proof at most.
- The envelope is `emem_fact::Receipt` in
  `crates/emem-fact/src/receipt.rs`. `MerkleProof` is in the same file.
  The `cost` field (struct `Cost`) declares latency and freshness.
- Verification has three surfaces. All three must agree.

  | surface | where |
  |---|---|
  | REST `POST /v1/verify_receipt` | `crates/emem-api-rest/src/lib.rs`, through `emem_storage::server::receipt_message` |
  | CLI `verify` | `run_verify` in `crates/emem-cli/src/main.rs`. Key order: `--pubkey`, then the `/.well-known/emem.json` of `--base-url` or `EMEM_BASE_URL`, then the receipt's own `responder`. It calls `receipt_message`, so it checks v0, v1 and v2 receipts. |
  | browser | `web/verify.html` |

  AGENTS.md pins three places that must agree: `sign_receipt`, the
  byte-by-byte preimage example in `docs/protocol.md`, and the browser
  verifier. AGENTS.md names that verifier `web/humans.html`, but that
  file does not exist. The verifier is `web/verify.html`.

## Signing keys

- `ResponderIdentity` (signing key, `AttesterKey`, `KeyEpoch`) is in
  `crates/emem-storage/src/server.rs`. It has `fresh()` (OsRng),
  `from_secret(secret, epoch)` and `export_secret_b32()`.
- The binary loads the key in `load_or_create_identity` in
  `crates/emem-cli/src/bin/emem-server.rs`. The order is: the env
  variable `EMEM_SECRET_B32` (a base32-nopad 32-byte secret), then the
  file `<EMEM_DATA>/identity.secret.b32`, then a fresh key that it
  writes to that file. A container with no volume and no env variable
  therefore makes a new signing key at each restart.
- `well_known()` in `crates/emem-api-rest/src/lib.rs` serves
  `/.well-known/emem.json`. The responder key is at
  `responder.pubkey_b32`, not at the top level. On 7 November 2026 it
  was `777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka`.
- `operator_attestation` in the same file binds the running binary hash
  to its git commit. It signs a tagged preimage with the domain
  `emem.operator_attestation.v1` (`OPERATOR_ATTESTATION_DOMAIN`). The
  segments are: 1 version, 2 key_epoch (u32 big-endian), 3 attested_at,
  4 git_commit, 5 build_timestamp, 6 binary_blake3, 7 bands_cid,
  8 registry_cid. The live file states the same layout in its
  `preimage` field.

## Fact storage

- Facts and the canonical index are in redb (`RedbFacts` in
  `crates/emem-cache/src/redb_facts.rs`). The table
  `emem.canonical_index` maps the key `cell \0 band \0 tslot_be8` to a
  fact CID. The table `emem.facts` maps a fact CID to its canonical
  CBOR.
- redb also holds `emem.fact_proofs`, `emem.multi_attester_index` and
  `emem.state_records`. These tables left sled because the sled file
  had grown to 43.7 GB. A cold open took about 21 minutes. The comment
  in `redb_facts.rs` gives these values, measured on 14 September 2026.
- sled (`SledHotCache` in `crates/emem-cache/src/sled_hot.rs`) keeps the
  memory trees and the small live trees: attesters, the trace gate,
  entities, edges. A fact read that misses in redb falls through to the
  old sled trees until the backfill has copied them.
  `EMEM_HOT_BACKEND=sled` rolls back to sled for facts.
- `MaterializingStorage` (`crates/emem-storage/src/lib.rs`) joins the
  cache, the fetch dispatcher, `AttestationLog` and `AttesterRegistry`.
  `rooted()` opens the stores under one data root.
- The write path is `put_attestation` in the same file. The steps, in
  order:
  1. `verify_attestation`.
  2. Admission. A fact that takes an address (cell, band, tslot) needs an
     admitted writer. That is a device enrolled through the trace gate, or a
     key that the operator listed. Otherwise the write fails with `fact_plane_closed`. The
     error names `POST /v1/derive` as the path for other keys.
  3. `cache.put_many`.
  4. `log.append`.
  5. Best effort: `persist_fact_proofs`, `append_multi_attester`,
     `append_scope_index`.
  6. One `flush_async`.
  7. The attester reputation rollup.
  8. `add_edges`.
- Edges are content addressed in the sled tree `emem.edges`
  (`TREE_EDGES`). Range scans use subject-predicate-object and
  object-predicate-subject indexes (`edge_index_key`,
  `scan_edges_anchored`).

## Transparency log

- `AttestationLog` in `crates/emem-storage/src/merkle_log.rs` is an
  append-only set of segment files `merkle.log.<u64>`. A segment rotates
  at 1 GiB (`SEGMENT_BYTES`). Each record is
  `[u32-LE len][attestation CBOR][32-byte blake3]`. A sealed segment
  ends with a hash over the whole segment. `append` calls `sync_all`
  before it returns.
- `persist_fact_proofs` (`crates/emem-storage/src/lib.rs`) computes a
  Merkle path for each fact in an attestation batch and stores it in
  `emem.fact_proofs`. `proof_for_cid` reads it back, and the receipt
  carries it as `receipt.merkle_proof`.
- `crates/emem-attest/src/translog.rs` is the RFC 6962 tree:
  `merkle_tree_hash`, `inclusion_path`, `consistency_proof` and their
  verifiers. The tree promotes a lone node. It does not duplicate it. This is a
  different tree from the per-batch root, so consistency proofs are
  sound. `AttestationLog::leaf_hashes()` reads the leaves in append
  order. `AttestationLog::verify` checks the end hash of each sealed
  segment. It does not check the open segment, which has no end hash yet.
- Routes, all live on 7 November 2026:

  | route | what it returns |
  |---|---|
  | `GET /v1/log/sth` | a responder-signed tree head. Tree size 3,244,653 at 01:18 UTC on 7 November 2026. |
  | `GET /v1/log/inclusion` | an inclusion proof. Give `leaf_index`, `entry_hash` or `fact_cid`. Add `tree_size` to prove against an earlier head. |
  | `GET /v1/log/consistency` | a proof that a later head extends an earlier one |
  | `POST /v1/log/witness` | records a co-signature on a `(tree_size, root)` head |
  | `GET /v1/log/witnesses` | the co-signatures. 49,544 on 7 November 2026, head witnessed. |

- The responder records a co-signature only when the signature verifies
  and the root matches its own history at that size. This makes a
  split view visible.
- `fact_cid` on `/v1/log/inclusion` uses the stored proof's log entry
  hash, then finds that entry in the leaves. The Mount Fuji fact above
  resolved to leaf 1,796,903. A fact signed before the log link has no
  entry until a background index recovers it. The route then returns
  404 `cid_not_found` and says to retry.
- Still open on this substrate: a sparse Merkle map from key to latest
  value (P4).

For an offline audit of the log, use the `emem-transparency-log` skill
in `plugins/emem/`.

## tslot logic

- `Tslot(pub u64)` is in `crates/emem-core/src/tslot.rs`.
  `from_unix(unix_seconds, tempo)` is
  `floor(unix / tempo.slot_seconds())`, from the 1970 Unix epoch.
  `EMEM_EPOCH_UNIX = 1_767_225_600` (2026-01-01) is metadata only.
- The tempo variants and their slot lengths:

  | tempo | slot |
  |---|---|
  | `Static` | 0 (no slot) |
  | `Slow` | 365 days |
  | `Composite16Day` | 16 days |
  | `Composite8Day` | 8 days |
  | `Medium` | 30 days |
  | `Fast` | 1 day |
  | `UltraFast` | 1 hour |

- The text form (`t.` prefix, base32-nopad LEB128) is in
  `crates/emem-codec/src/tslot_text.rs`.

## Signed writes

Both write paths use an Ed25519 signature inside the envelope, not
transport authentication.

### Facts

- Routes: `POST /v1/attest` and `POST /v1/attest_cbor`. The envelope is
  `emem_fact::Attestation` in `crates/emem-fact/src/attest.rs`.
- The canonical signer is `Attestation::build_and_sign_v1`. It hashes
  each fact as `blake3(canonical_cbor(fact))`, adds the edge digests,
  sorts the leaves bytewise, refuses duplicates, computes the Merkle
  root, and signs `attestation_preimage_v1`.
- The ingest check is `verify_attestation` in
  `crates/emem-storage/src/lib.rs`. It recomputes the root, picks the
  rule from `preimage_version`, and calls `verify_strict`.
- MCP does not expose these two routes. The comment beside the catalog
  in `crates/emem-mcp/src/lib.rs` says so. The client signs and calls
  REST. Other signed writes (`emem_memory_create`, `emem_derive` and the
  rest of the memory verbs) use MCP.

### Memory files

- `crates/emem-primitives/src/memory_acl.rs` locks
  `/memories/by_attester/<pubkey8>/` to the key holder.
- There are two preimages. Each is BLAKE3 over a pipe-separated string,
  and the client signs the 32-byte digest.

  | rule | preimage | function |
  |---|---|---|
  | v1 | `"emem.memory_write|" verb "|" path "|" body_hash` | `attester_preimage` |
  | v2 | `"emem.memory_write.v2|" verb "|" path "|" body_hash "|" base` | `attester_preimage_v2` |

  `base` is the version of the path that the write replaces, or the
  literal `absent` (`BASE_ABSENT`) when the path holds no file. It stops
  a replay of a signature from the public log as the same write. `delete`
  and `rename` require v2 (`V2_REQUIRED_VERBS`). The other verbs still
  accept v1.
- `body_hash` is the content that the verb produces, not the POSTed
  JSON. It is `file_text` for `create` and the file after the edit for
  `str_replace` and `insert`. It is `blake3("")` for `delete`. `rename`
  signs `path = new_path` with `body_hash = blake3(old_path)`
  (`rename_body_hash`), so one signature binds both ends of the move.
  Ownership of the source is a namespace check
  (`namespace_ownership_ok`), because one signature cannot verify
  against two preimages.
- The REST layer enforces the binding in `validate_attester_binding`
  (`crates/emem-api-rest/src/lib.rs`).
- By default, the bare `/memories/` namespace refuses unsigned writes.
  `memory_write_policy` returns `RequireAll` in every non-test build, so
  an unsigned write gets 401 `memory_attestation_required`. The env
  variables change this. The server checks them in the order of this
  table. The first one set to a true value wins. The values `0`, `false`
  and an empty string count as unset. The server reads them once into a
  `OnceLock`:

  | variable | policy |
  |---|---|
  | `EMEM_MEMORY_REQUIRE_ATTESTER=1` | `RequireAll` |
  | `EMEM_MEMORY_OPEN=1` | `Open`: unsigned writes allowed |
  | `EMEM_MEMORY_HARDEN_DESTRUCTIVE=1` | `HardenDestructive`: only `delete` and `rename` need a signature (`policy_gates_verb`) |

  `cfg(test)` defaults to `Open`, so the in-crate fixtures still test
  the unsigned path. `by_attester` paths need a signature under every
  policy.

### Disagreement between attesters

`put_attestation` appends every distinct CID for one
`(cell, band, tslot)` to `emem.multi_attester_index`
(`TREE_MULTI_ATTESTER_INDEX`). A second attester does not overwrite the
first. Both must first pass the admission step of `put_attestation`.

The early Phase 3 notes listed four open items. This is their state on
7 November 2026:

| item | state |
|---|---|
| `ATTESTERS.md` spec | not written. No such file exists. |
| registration | replaced by the tier ladder at `GET /v1/enlist` and the admission step of `put_attestation` |
| rate and abuse policy | `rate_limit_layer` in `crates/emem-api-rest/src/lib.rs`; see `docs/operators/operating.md` |
| multiplicity at recall time | `emem_memory_contradictions` reads `emem.multi_attester_index` | `emem_memory_contradictions` reads this index across attesters
(`memory_contradictions` and `compute_severity` in
`crates/emem-primitives/src/memory_contradictions.rs`).

### Conformance levels

L0, L1 and L2 are values of `ToolDescriptor.level`
(`crates/emem-mcp/src/lib.rs`). The catalog has 108 L0 and 8 L1 tools
and no L2 tool.

## MCP tool registry

- The canonical catalog is `pub const TOOLS: &[ToolDescriptor]` in
  `crates/emem-mcp/src/lib.rs`. `ToolDescriptor` has `name`, `title`,
  `description`, `when_to_use`, `input_schema`, `output_schema`,
  `example_args`, `level`, `category`, four MCP hint flags and `tier`.
  116 tools: 18 core, 98 extended. The helpers are `lookup`,
  `tools_at_level` and `tools_at_tier`.
- JSON-RPC dispatch and the REST mirror are in `emem-api-rest`
  (`mcp_jsonrpc`, `mcp_tool_call`).
- Two endpoints share one dispatcher. `/mcp` (`mcp_jsonrpc`) lists the
  core tier in `tools/list`. `/mcp/full` (`mcp_jsonrpc_full`) lists all
  116. Both call `mcp_with_version` with a different default tier. It
  agrees the protocol revision, then calls `mcp_jsonrpc_inner`. The
  comment on `MCP_CORE_ENDPOINT_TIER` says why the split exists. An
  explicit `{"tier": ...}` overrides the endpoint default. `tools/call`
  runs any tool by name at either endpoint.
- `emem_tools` (core) is the catalog inside MCP: `tools_catalog` in
  `crates/emem-api-rest/src/lib.rs`. It reads `CORE_LOOP` and
  `TOOL_GROUPS` in `emem-mcp`. The unit tests
  `core_loop_names_real_core_tools_in_order` and
  `tool_groups_name_real_tools` check both against `TOOLS`.
- Other handlers:

  | tool or token | where |
  |---|---|
  | `emem_memory_contradictions` | `memory_contradictions` in `crates/emem-primitives/src/memory_contradictions.rs` |
  | `emem_diff` | `diff` in `crates/emem-primitives/src/diff.rs` |
  | `emem:fact:` parse | `parse_fact_token` in `crates/emem-api-rest/src/lib.rs` |
  | `emem:bundle:` | `crates/emem-primitives/src/memory_bundle.rs` (strict parser on the `emem:bundle:` prefix; it also accepts the old `memb:` prefix) |

## Counts, and what guards them

`scripts/sync_counts.py --check` compares the canonical counts with the
registries and the live responder. The counts are 116 tools, 183 /v1
paths, 46 sources, 43 slots, 129 wired bands, 172 algorithms, 27 topics
and 19 crates. CI runs it (`.github/workflows/ci.yml`, step "prose counts
match the responder"). If the origin does not answer, it warns and does
not fail. A green run therefore does not prove a live check.

Two limits apply to a green result:

- The scan matches a fixed set of phrasings. It does not see a count
  written in a new way until someone adds the pattern. This file once
  said `104 / 15 / 89` and still passed, because the tool-count pattern
  needed a parenthesis and this file used a colon. The same file said
  "88 descriptors" for the MCP catalog until 7 November 2026.
- Files in `COUNT_HISTORY` are exempt on purpose, because they record
  what a count was. `then-102` in a benchmark row and "v1 said 102" in
  the whitepaper stay correct and must not change.
