# Architecture notes: where the load-bearing pieces live

Phase 0 orientation map for the shared-memory upgrade. This file answers one
question per section: for each mechanism the upgrade plan hangs off, which
crate, file, and function is canonical today. Line numbers are as of the
commit that introduced this file; treat them as anchors, not gospel.

Crate layering, bottom to top:

```
emem-core        Tslot, key types (AttesterKey, KeyEpoch, Signature), registries, error codes
emem-codec       token-economical text codecs: cell64, tslot text, cid64, vec64
emem-attest      pure hashing: preimages, merkle v0/v1 (no I/O, no deps beyond core)
emem-trace       emem.os_trace.v1 schema + verification engine + drift-anchor scoring (pure)
emem-fact        wire structs (Fact, Attestation, Receipt, EdgeFact, Scope) + canonical CBOR
emem-claim       structural claim algebra (band+op+value+tslot) and evaluator
emem-cache       sled hot cache, fact CID derivation
emem-fetch       upstream connectors (bundled GeoNames/POI data via include_bytes!)
emem-cubes       loaders for the AgriSynth 1792-D bootstrap cubes
emem-storage     MaterializingStorage facade, sled tree schemas, AttestationLog,
                 AttesterRegistry, and receipt signing (Server / ResponderIdentity)
emem-primitives  recall/diff/verify/memory_* handlers, memory ACL, contradictions
emem-intent      typed Intent grammar + heuristic planner
emem-mcp         static MCP tool catalog (88 descriptors; no transport)
emem-api-rest    axum router: REST routes, /mcp JSON-RPC dispatch, well-known, write gate
emem-cli         emem-server binary (key load/persist) + receipt-verify CLI
emem-scorecard   MemoryAgentBench-style scorecard harness (leaf)
emem-sleep-agent opt-in sleep-time rewrite/merge agent (leaf)
```

Everything hashes with BLAKE3, serializes with deterministic CBOR (ciborium,
RFC 8949), renders CIDs as base32-nopad-lowercase, and signs with ed25519.
Version discriminators: `Attestation.preimage_version` and
`Receipt.preimage_version` (serde default 0 = legacy) select v0 or v1 rules,
so old envelopes keep verifying.

## Canonical payload serialization

- `to_canonical_cbor<T: Serialize>` in `crates/emem-fact/src/cbor.rs:21`
  encodes any value to deterministic CBOR. Struct fields serialize in
  declaration order; freeform maps must be pre-sorted by the caller.
- `blake3_32` (`cbor.rs:30`) and `base32_prefix` (`cbor.rs:38`) sit beside it.
- Custom CBOR tags include `TAG_EMEM_CELL=65000`, `TAG_EMEM_TSLOT=65001`,
  `TAG_EMEM_VEC64=65002`, and `TAG_IPLD_CID=42` (`cbor.rs:7-13`).

Any new payload kind (transparency-log STH, absence proof, DerivativeFact,
signed latent) should go through `to_canonical_cbor` and get its own schema
version; never change the encoding of an existing kind.

## CID computation

- Fact CIDs: `fact_cid_of(fact)` in `crates/emem-cache/src/sled_hot.rs:190-199`
  = base32-nopad-lowercase of `blake3(canonical_cbor(fact))`, always 52 chars
  (full 256 bits). `SledHotCache::put_many` re-derives the same CID inline on
  write (`sled_hot.rs:290-303`).
- Typed CID newtypes (FactCid, RegistryCid, SchemaCid, ReasonCid, BatchCid,
  CoverageCid, EdgeCid) are string wrappers in `crates/emem-fact/src/cid.rs:25-34`.
- Edge CIDs: `EdgeFact::cid()` in `crates/emem-fact/src/edge.rs:70-90`.
- `cid64` (13-char base32 of the first 8 bytes) in
  `crates/emem-codec/src/cid64.rs:8-25` is inline-text-only; it never appears
  in canonical CBOR. Bundle CIDs are truncated to 16 bytes in
  `crates/emem-primitives/src/memory_bundle.rs:139`; fact CIDs are full-width.

## Receipt signing path

- All public signers (`sign_receipt`, `sign_receipt_with_scope`,
  `sign_receipt_with_as_of`, `sign_receipt_full`, `sign_receipt_with_edges`)
  funnel into the one private `sign_receipt_v1_inner` in
  `crates/emem-storage/src/server.rs:236-348`.
- Preimage: `receipt_preimage_v2` (v1 kept verbatim for receipts signed
  under it) in `crates/emem-attest/src/lib.rs`.
  `PreimageV1::new(domain)` hashes `"emem.preimage.v1\x00" || u32-LE(len) ||
  domain`, then each segment is `tag || u32-LE(len) || bytes`
  (`lib.rs:157-226`). Receipt tags 0x01..0x09: REQUEST_ID, SERVED_AT, SCOPE,
  AS_OF, EDGES, MANIFEST, PRIMITIVE, CELLS, FACT_CIDS.
- Merkle over `fact_cids`: v1 is RFC 6962-style, `blake3(0x00 || leaf)` /
  `blake3(0x01 || l || r)` in `crates/emem-attest/src/lib.rs:284-385`; legacy
  v0 at `lib.rs:11-121`. Callers sort leaves and reject duplicates
  (CVE-2012-2459 pattern) via `has_adjacent_duplicate` (`lib.rs:383`) before
  calling `merkle_root_v1`. At signing time (`server.rs:~318`) a pre-persisted
  inclusion proof for the first cited fact is looked up via `proof_for_cid`
  (built earlier by `persist_fact_proofs`); a receipt carries at most one.
- Envelope: `emem_fact::Receipt` in `crates/emem-fact/src/receipt.rs:12-79`
  (`preimage_version` at :77-78, `MerkleProof {leaf_index, path, root,
  version}` at :176-189). Cost fields (latency, cache hit) at
  `receipt.rs:162-173`.
- Verification surfaces: REST `POST /v1/verify_receipt`
  (`crates/emem-api-rest/src/lib.rs:12282-12310`, handles v0 and v1); a CLI
  verifier in `crates/emem-cli/src/main.rs:190-233` (pubkey resolution:
  explicit > well-known > embedded) which today implements only the legacy v0
  preimage, so it cannot verify the v1 receipts the current server signs; and
  the in-browser verifier (`web/verify.html`) which mirrors the v1 byte rules.
  AGENTS.md pins three places that must always agree: `sign_receipt`, the
  byte-by-byte preimage example in `docs/protocol.md`, and the browser
  verifier in `web/verify.html`.

## Signing keys

- `ResponderIdentity` (SigningKey + AttesterKey + KeyEpoch) in
  `crates/emem-storage/src/server.rs:98-134`: `fresh()` (OsRng),
  `from_secret(secret, epoch)`, `export_secret_b32()`.
- The binary loads via `load_or_create_identity` in
  `crates/emem-cli/src/bin/emem-server.rs:338-392`: env `EMEM_SECRET_B32`
  (base32-nopad 32-byte secret) > `<EMEM_DATA>/identity.secret.b32` file >
  fresh key persisted atomically. A container without a volume or the env var
  therefore mints a new signing key on every restart.
- Public exposure: `/.well-known/emem.json` served by `well_known()` in
  `crates/emem-api-rest/src/lib.rs:4025-4108` (route at :818). The responder
  key lives at `responder.pubkey_b32` (not top-level). `operator_attestation`
  signs the string preimage
  `emem.operator_attestation|v{n}|epoch{n}|{attested_at}|git:{commit}|build:{ts}|binary:{hash}|bands:{cid}|registry:{cid}`
  (:4052-4062), binding the running binary hash to its git commit.

## Fact storage

- Engine: sled. `SledHotCache` (`crates/emem-cache/src/sled_hot.rs`) holds the
  canonical index (key = `cell \0 band \0 tslot_be8` -> fact_cid,
  `encode_key` at :211) and a facts tree (fact_cid -> canonical CBOR body).
- `MaterializingStorage` (`crates/emem-storage/src/lib.rs:243`) composes
  cache + fetch dispatcher + AttestationLog + AttesterRegistry; `rooted()`
  opens `<root>/cache.sled` and the log directory (:581-604).
- Write path: `put_attestation` (`lib.rs:639-698`): `verify_attestation` ->
  `cache.put_many` -> `log.append` -> best-effort `persist_fact_proofs` +
  `append_multi_attester` + `append_scope_index` -> one `flush_async` ->
  attester reputation rollup -> `add_edges`.
- Edges: content-addressed in sled tree `emem.edges` with SPO/OPS range-scan
  indexes (`edge_index_key` at `lib.rs:928`, `scan_edges_anchored` at :1095).

## Existing transparency-log substrate (Phase 1 starting point)

Phase 1 does not start from zero. Already shipped:

- `AttestationLog` in `crates/emem-storage/src/merkle_log.rs`: append-only
  segment files `merkle.log.<u64>` rotating at 1 GiB; each record is
  `[u32-LE len][attestation CBOR][32-byte blake3]`; sealed segments carry a
  trailing whole-segment hash; `append` fsyncs before returning (:58);
  `verify` walks a segment (:117).
- Per-fact inclusion proofs: `persist_fact_proofs`
  (`crates/emem-storage/src/lib.rs:1426-1481`) precomputes merkle paths per
  attestation batch into sled tree `emem.fact_proofs`, read back via
  `proof_for_cid` (:808-813) and embedded in receipts as
  `receipt.merkle_proof`.

What Phase 1 adds on top: one global tree across batches, Signed Tree Heads,
consistency proofs between tree sizes, and public proof endpoints. The
per-batch trees and the fsynced log are the natural leaf source.

**Phase 1 shipped.** `crates/emem-attest/src/translog.rs` implements the
RFC 6962 tree (Merkle-tree-hash, inclusion and consistency proofs, and
their verifiers): a genuine promote-lone-node construction, distinct from
the batch-root tree in section 6.1, so consistency proofs are sound.
`AttestationLog::leaf_hashes()` (`merkle_log.rs`) reads the append-ordered
leaves; `GET /v1/log/{sth,inclusion,consistency}` serve a responder-signed
STH and offline-verifiable proofs. **Phase 3 shipped:** witness co-signing
(`POST /v1/log/witness`, `GET /v1/log/witnesses`) lets an external party
counter-sign a `(tree_size, root)` head so split-view equivocation is
detectable; the responder records a co-signature only when the signature
verifies and the root matches its own history at that size. Still open on
this substrate: a sparse-merkle key->latest-value map (P4), and a
`fact_cid -> leaf_index` index so an inclusion proof can be requested by
fact rather than by log position.

## tslot logic

- `Tslot(pub u64)` in `crates/emem-core/src/tslot.rs:21`;
  `from_unix(unix_seconds, tempo) = floor(unix / tempo.slot_seconds())`
  (`slot_seconds` at :51-63), anchored at the 1970 Unix epoch
  (`EMEM_EPOCH_UNIX = 1_767_225_600` at :45-47 is metadata-only).
- Tempo variants (:26-43): Static(0s), Slow(365d), Composite16Day,
  Composite8Day, Medium(30d), Fast(1d), UltraFast(1h).
- Text form (`t.` prefix, base32-nopad leb128) in
  `crates/emem-codec/src/tslot_text.rs:9-23`.

## Attester auth for L2 writes

Two write paths exist today, both authenticated by in-envelope ed25519
signatures rather than transport auth:

- Facts: `POST /v1/attest` and `/v1/attest_cbor` (routes
  `crates/emem-api-rest/src/lib.rs:1175-1176`, handlers :12237-12280). The
  envelope is `emem_fact::Attestation`
  (`crates/emem-fact/src/attest.rs:13-64`); the canonical signer is
  `Attestation::build_and_sign_v1` (:93-143): per-fact leaves =
  `blake3(canonical_cbor(fact))` plus edge digests, sorted bytewise,
  duplicates rejected, merkle root computed, then ed25519 over
  `attestation_preimage_v1`. Ingest-side check is `verify_attestation` in
  `crates/emem-storage/src/lib.rs:1498-1560` (recompute root, branch on
  `preimage_version`, `verify_strict`). Write tools are deliberately not
  exposed over MCP (`crates/emem-mcp/src/lib.rs:1229-1238`); signing happens
  client-side.
- Memory files: `crates/emem-primitives/src/memory_acl.rs` write-locks
  `/memories/by_attester/<pubkey8>/` to the keyholder; signature = ed25519
  over `blake3("emem.memory_write|" || verb || "|" || path || "|" ||
  body_hash)` (`attester_preimage`, :64-73); enforced from the REST layer
  (`validate_attester_binding`, `crates/emem-api-rest/src/lib.rs:21275-21349`).
  `body_hash` is the content the verb yields, not the POSTed JSON:
  `file_text` for `create`, the post-edit file for `str_replace` /
  `insert`, `blake3("")` for `delete`. `rename` signs `path = new_path`
  with `body_hash = blake3(old_path)` (`rename_body_hash`, :92-94), so one
  signature pins both ends of a move; the source's ownership is a
  namespace check (`namespace_ownership_ok`), since one signature cannot
  verify against two preimages.
- The bare `/memories/` namespace is closed by default: `memory_write_policy`
  (`crates/emem-api-rest/src/lib.rs:21219-21247`) returns `RequireAll` in
  non-test builds, so an unattested write gets 401
  `memory_attestation_required`. `EMEM_MEMORY_OPEN=1` restores the unsigned
  memory-tool contract, `EMEM_MEMORY_HARDEN_DESTRUCTIVE=1` gates only
  `delete` / `rename` (`policy_gates_verb`, :21252-21258). Precedence:
  `REQUIRE_ATTESTER` > `OPEN` > `HARDEN_DESTRUCTIVE` > default, read once
  into a `OnceLock`. `cfg(test)` defaults to `Open` so in-crate fixtures
  keep exercising the unattested path. `by_attester` is gated regardless.
- Multi-attester disagreement is preserved, not overwritten: every distinct
  CID per `(cell, band, tslot)` is appended to sled tree
  `emem.multi_attester_index` at `put_attestation`
  (`crates/emem-storage/src/lib.rs:1196-1228, 1379-1416`), which is what
  `emem_memory_contradictions` reads across attesters.
- L0/L1/L2 are conformance levels on `ToolDescriptor.level`
  (`crates/emem-mcp/src/lib.rs:46-47`); the current catalog ships 79 L0 + 2 L1
  tools and zero L2 tools.

Phase 3's job is therefore spec and hardening (ATTESTERS.md, registration,
rate/abuse policy, recall-time multiplicity surfacing), not building the
write path from scratch.

## MCP tool registry

- Canonical catalog: `pub const TOOLS: &[ToolDescriptor]` in
  `crates/emem-mcp/src/lib.rs:716`; `ToolDescriptor` struct at :31-68 (name,
  title, description, when_to_use, input_schema, level, category, four MCP
  hint flags, tier). 107 tools: 16 core, 91 extended. Helpers: `lookup`
  (:1738), `tools_at_level` (:1743), `tools_at_tier` (:1792).
- JSON-RPC dispatch and the REST mirror live in emem-api-rest
  (`mcp_jsonrpc` at `crates/emem-api-rest/src/lib.rs:14875`, `mcp_tool_call`
  at :16177).
- Two endpoints, one dispatcher: `/mcp` (`mcp_jsonrpc`) advertises the core
  tier from `tools/list`, `/mcp/full` (`mcp_jsonrpc_full` at :14883)
  advertises all 102. Both funnel into `mcp_jsonrpc_inner` (:14891) with a
  different `default_tier`; see `MCP_CORE_ENDPOINT_TIER` (:14869) for why the
  split exists and which earlier `nextCursor` attempt it supersedes. An
  explicit `{"tier":...}` beats the endpoint default, and `tools/call`
  dispatches every tool by name at either endpoint regardless of tier.
- `emem_tools` (core) is the in-band catalog: `tools_catalog` at
  `crates/emem-api-rest/src/lib.rs:16015`, backed by `CORE_LOOP` (emem-mcp
  :1813) and `TOOL_GROUPS` (:1857), both asserted against `TOOLS` by unit
  tests in the same file.
- Handlers referenced by the upgrade plan: `emem_memory_contradictions` ->
  `crates/emem-primitives/src/memory_contradictions.rs:132` (severity :335);
  `emem_diff` -> `crates/emem-primitives/src/diff.rs:43`; emem:fact compose/parse
  in `crates/emem-api-rest/src/lib.rs:17226, 18560-18568`; emem:bundle in
  `crates/emem-primitives/src/memory_bundle.rs` (strict `emem:bundle:` parser at
  :177-183).

## Counts, and what guards them

`scripts/sync_counts.py --check` verifies the canonical counts (107 tools,
155 /v1 paths, 46 sources, 43 slots, 129 wired bands, 168 algorithms, 27
topics, 18 crates) against the registries and the live responder. CI runs it
(`.github/workflows/ci.yml`, "prose counts match the responder"); it warns
rather than fails when the origin is unreachable, so a green run does not
by itself prove the counts were checked against a live responder.

Two limits worth knowing before trusting a green result. The scan matches a
fixed set of phrasings, so a count written a new way is invisible to it until
the pattern is added: this file drifted to a stale `104 / 15 / 89` and went on
passing, because the tool-count pattern required a parenthesis and this file
used a colon. And documents in `COUNT_HISTORY` are exempt on purpose, because they
record what a count *was*: `then-102` in a benchmark row and "v1 said 102" in
the whitepaper are correct forever and must not be rewritten.
