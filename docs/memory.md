# Memory substrate

> The formal definition of what this substrate is (the observation tuple,
> the property table, the memory algebra) lives in [the memory model](model.md).

The memory substrate is the writable layer of emem. Below it sit the
read primitives (`/v1/recall`, `/v1/state`, `/v1/find_similar`,
`/v1/trajectory`) whose facts come from upstream materialisers and are
signed by the responder. Above
it sit the agents that read, cite, write, and stream memory through the
same ed25519 receipt surface as every other primitive.

This page is the wire reference. The agents guide ([agents.md](agents.md))
covers the read-side rosetta; this document covers everything the agent
puts back.

## What's in the substrate

| Layer | Endpoint | Wire shape | Returns |
|---|---|---|---|
| State (single encoder) | `POST /v1/state` | `{cell, encoder?, view?, as_of_tslot?, as_of_signed_at?}` | a stored embedding vector + memory_token; the encoders are retired (see below), so this answers only from vectors signed before |
| State (full cube) | `POST /v1/state` view=cube | same, plus `materialize?` | 1792-D state vector, per-slot coverage, `state_cid` and an `emem:bundle:` token |
| State fan-out | `POST /v1/state_multi` | `{cell, encoders?, as_of_*?}` | per-encoder dense map; empty by default, since no encoder is live |
| State delta | `POST /v1/state_diff` | `{cell, encoder?, tslot_a, tslot_b}` | residual + cosine + both fact_cids |
| Memory token | `POST /v1/memory_token` | `{cell, fact_cid, band?, observed_on?}` | `emem:fact:<cell>:<fact_cid>` and `cell_token` (`emem:cell:<cell>`); with `band` and `observed_on`, also a `descriptor_token` |
| Memory token resolve | `POST /v1/memory_token/resolve` | `{token}` | full signed fact body |
| Memory token resolve, batch | `POST /v1/memory_token/resolve_many` | `{tokens}` (1 to 256) | one resolved item per token |
| Memory bundle | `POST /v1/memory_bundle` | `{triples}` or `{fact_cids}`, plus `purpose?`, `scope?` | signed envelope + `emem:bundle:<bundle_cid>` |
| Memory bundle resolve | `GET /v1/memory_bundle/<token>` | path param | same envelope |
| Memory file write | MCP `emem_memory_create` | `{path, file_text, kind?, attester}` | `file_cid` + signed receipt |
| Memory file edit | MCP `emem_memory_str_replace`, `emem_memory_insert` | `{path, old_str, new_str, attester}` etc. | new `file_cid` + receipt |
| Memory file supersede | MCP `emem_memory_supersede` | `{path, superseded_by, reason?, attester}` | readers of `path` get `superseded_by` and a `_superseded` banner; the bytes stay |
| Memory file read | MCP `emem_memory_view` | `{path}` or `{file_cid}`, plus `view_range?`, `view?`, `offset?`, `kind?` | content with its `authorship` block, or a directory listing |
| Memory file read, plain | `GET /memories/<path>` | path | the note as `text/markdown`, `ETag` = its `file_cid` |
| Memory file rename | MCP `emem_memory_rename` | `{old_path, new_path, attester}` | new path index + receipt |
| Memory file delete | MCP `emem_memory_delete` | `{path, attester}` | path drop plus a tombstone (blob retained, history preserved) |
| Memory list by kind | MCP `emem_memory_list_by_kind` | `{kind, prefix?, limit?}` | typed listing sorted by signed_at desc |
| Memory file search | `POST /v1/memory/search` | `{q, k?, mode?, kind?, path_prefix?, attester_pubkey_b32?}` | ranked hits + snippets |
| Row audit path | `GET /v1/tree/<file_cid>?row=` (MCP `emem_tree`) | path + query | one row's leaf and its path to the root a `pointer.v1` or `directory.v1` note signs |
| Memory event stream | `GET /v1/memory/sse?path_prefix=&kind=&attester=` | query string | `text/event-stream` of writes |
| Multi-attester contradictions | `POST /v1/memory_contradictions` | `{cell_prefix?, band?, window_unix_s?, min_severity?, include_same_attester_sources?, limit?}` | severity-scored disagreements |

The foundation encoders that `view=encoder` read are retired: Clay v1.5,
Prithvi-EO-2.0 and Galileo were removed from the code, and emem.dev also
retires `geotessera` through `EMEM_RETIRED_BANDS`. Their bands stay in the
manifest, so vectors signed under them still recall and verify, but no new
one is made. On a cell with none, `view=encoder` returns 404 naming the
retirement and pointing at `view="cube"`.

The pre-rename prefixes `memt:`, `memb:`, and `meme:` still resolve.

## The kinds

Memory files carry a `kind` from the CoALA / LangMem agent-memory
ontology. Default is `resource` so callers that don't pass `kind` keep
the back-compat Anthropic memory-tool shape.

| `kind` | Purpose | TTL default |
|---|---|---|
| `episodic` | Observations of events. The agent ran X at Y on date Z. | 30 days |
| `semantic` | Durable learned facts. "Mato Grosso has tropical climate." | infinite |
| `procedural` | Playbooks, how-to notes. "When user asks flood risk, call /v1/water + /v1/elevation." | infinite |
| `resource` | Generic durable scratchpad. The default Anthropic memory-tool target. | 90 days |
| `vault` | An AEAD-sealed entry, stored encrypted in its own tree and never indexed by search or the contradiction scan. | infinite |
| `core` | A persona block, listed first by `emem_memory_view`. Parsed by the server but not in the tool schemas' `kind` enum. | infinite |

A `vault` entry reads back as ciphertext unless the caller passes a
`vault_capability`: an ed25519 signature over
`blake3("emem.vault_open|" + path + "|" + nonce)` that verifies under
the responder's key. The seal key is derived from the responder's own
secret, so a vault keeps bytes from other callers and from anyone who
copies the database file, not from the operator. Encrypt client-side if
the operator must not read it.

TTL pass (`EMEM_MEMORY_TTL_ENABLED=1`) sweeps every hour
(`EMEM_MEMORY_TTL_INTERVAL_SECS`). Expired files move from
`memory_files` to `memory_files_expired`; the blob is retained under
`memory_file_blobs` (content-addressed; never deleted). Per-kind
overrides via
`EMEM_MEMORY_TTL_{CORE,RESOURCE,EPISODIC,SEMANTIC,PROCEDURAL,VAULT}_DAYS`
(0 = infinite).

The consolidation pass (`EMEM_MEMORY_CONSOLIDATION_ENABLED=1`) runs
every 24 h (`EMEM_MEMORY_CONSOLIDATION_INTERVAL_SECS`). For every
`/memories/by_attester/<pubkey>/<sub>/` with at least 50 episodic files
(`EMEM_MEMORY_CONSOLIDATION_MIN_FILES`) older than 7 days
(`EMEM_MEMORY_CONSOLIDATION_MIN_AGE_DAYS`), it concatenates the bodies
in chronological order, signs the result as a `semantic` kind file at
`.consolidated/<unix_ts>.md`, and stamps `superseded_by: <consolidated_cid>`
on every original's metadata. Originals stay accessible via
`memory_file_history`.

Both passes are off unless the operator sets them. emem.dev does not set
either today, so files there do not expire and are not consolidated.

## Capability binding

Every write carries an `attester` block:

```json
{
  "pubkey_b32": "<base32-nopad-lowercase 52-char ed25519 pubkey>",
  "sig_b32":    "<base32-nopad-lowercase 103-char ed25519 signature>"
}
```

The signed preimage (v2) is:

```
blake3("emem.memory_write.v2|" + verb + "|" + path + "|" + body_hash + "|" + base)
```

where `body_hash = blake3(canonical body bytes)` as 32 raw bytes, and
`base` is the `file_cid` now at `path`, or the literal `absent` when
nothing is there. What counts as the body depends on the verb:

| verb | `path` | body | `base` |
|---|---|---|---|
| `create` | the file written | the `file_text` string sent | current `file_cid` or `absent` |
| `str_replace` | the file edited | the whole file *after* the edit | current `file_cid` |
| `insert` | the file edited | the whole file *after* the edit | current `file_cid` |
| `delete` | the file removed | empty, i.e. `blake3("")` | the `file_cid` being deleted |
| `rename` | the **new_path** | the **old_path** string | `absent` (new_path must be empty) |
| `supersede` | the note marked stale | `"<superseded_by>\|<reason>"` | current `file_cid` |

You do not have to build this by hand. Send the write without `attester`
and the 401 carries `details.how_to_sign`: the exact 32-byte digest for
that write, the encoding rules, and a runnable Python example. Sign the
digest and resend the identical body. No registration or API key is
involved; any locally generated keypair works. Keep the seed: the key
owns its namespace, and a write can only be unpublished by that key.

Why `base`: caller signatures are stored in the ledger so authorship can
be re-verified offline, which makes every past signature public. Binding
the version a write replaces means a signature read off the log fails
once the path has moved on, and two agents editing one path get a
refusal instead of a silent lost update.

The older v1 preimage,
`blake3("emem.memory_write|" + verb + "|" + path + "|" + body_hash)`, is
still accepted for `create`, `str_replace` and `insert` while clients
migrate, and refused for `delete` and `rename`. A v1 signature on a verb
whose effect depends on current state is single-use: reusing it returns
409 `memory_signature_replayed`. `create` and `supersede` stay
replayable so an honest retry works.

`rename` is the only verb whose signature binds two paths. The
destination rides the preimage's `path`, the source rides its
`body_hash`, so one signature authorises one specific move. When the
source is itself under `by_attester`, the responder additionally checks
the key owns it, and refuses with 403 `memory_namespace_violation`
(`reason: source_namespace`) if not.

Who may write where:

- `/memories/by_attester/<pubkey8>/...` belongs to the key whose base32
  form starts with `<pubkey8>`. Only that key may write there, under
  every policy.
- Everywhere else under `/memories/`, the first attester to create a
  path owns it, and only that key may change it. Such names are
  unreserved: whoever writes `/memories/standard.md` first holds it for
  good, so do not build a protocol dependency on an open-root name.
  Open-root files written before authorship was stored have no owner
  and are frozen for every key.
- `/memories/.well-known/` is reserved to the operator and refuses every
  key, the responder's included (`reason: reserved_namespace`).

Refusals, all with `details.code` naming the case:

- No attester where one is needed → 401 `memory_attestation_required`.
- Signature does not verify, or the key is malformed → 401
  `memory_attestation_invalid`; the message names the digest the
  responder expected.
- Wrong namespace, wrong owner or reserved prefix → 403
  `memory_namespace_violation`.

Bare `/memories/...` is **not** anyone-writable by default. A release
build with no env set refuses every unattested write. The operator picks
the policy:

| env | unattested writes to bare `/memories/...` |
|---|---|
| *(none set)* | refused for every verb |
| `EMEM_MEMORY_HARDEN_DESTRUCTIVE=1` | `create`, `str_replace`, `insert` accepted; `delete` and `rename` refused |
| `EMEM_MEMORY_OPEN=1` | accepted for every verb (the unsigned memory-tool contract) |
| `EMEM_MEMORY_REQUIRE_ATTESTER=1` | refused for every verb (the default, set explicitly) |

Precedence when several are set: `EMEM_MEMORY_REQUIRE_ATTESTER` >
`EMEM_MEMORY_OPEN` > `EMEM_MEMORY_HARDEN_DESTRUCTIVE`. The value is
read once at startup, so a change needs a restart. `emem.dev` runs with
`EMEM_MEMORY_REQUIRE_ATTESTER=1`.

The policy only ever governs the bare namespace.
`/memories/by_attester/<pubkey8>/...` requires a valid attester under
every policy, including `EMEM_MEMORY_OPEN=1`.

For attested writes, the receipt's `cells[]` becomes
`["pubkey:<b32>", path]`; for bare writes it stays `[path]`. A read of
the note returns an `authorship` block (key, signature, verb, signed
path, `base`, `body_hash_hex`, `preimage_version`) so a third party can
re-verify the author offline, and should also check `body_hash_hex`
against blake3 of the content it received.

## Reading, superseding and deleting

A note can be read by path or by content address. `emem_memory_view`
with `file_cid` returns the bytes whether or not a path still points at
them, so a citation survives its author renaming, superseding or
deleting the note. `GET /memories/<path>` serves the same note as plain
markdown with its `file_cid` as the `ETag`. A memory `file_cid` is
blake3 of the bytes truncated to 16 bytes (26 base32 characters),
shorter than a fact cid; re-hash what comes back to check it.

`emem_memory_supersede` points one of your notes at the note that
replaces it. Readers of the old path then get `superseded_by` and a
`_superseded` banner above the content. The replacement must already
exist, a note cannot supersede itself, and a superseded note cannot be
re-aimed, so a correction chain only grows.

`emem_memory_delete` removes the path from the index and writes a
tombstone (`emem.tombstone.v1`: path, prior `file_cid`, who deleted it,
when). A later read of that path returns 404 saying it was deleted by
its owner, with the tombstone in `details`, rather than the plain
"never written" 404. The blob stays addressable by `file_cid`, because
issued receipts must keep verifying. Treat delete as unpublish, not
erasure.

## Tokenised files: `emem:tree`

A `pointer.v1` note names a large object by the blake3 hash of every
chunk it read plus one Merkle root over them; a `directory.v1` note does
the same over a listing. The root is inside the author's signed note.
`GET /v1/tree/<file_cid>` returns the row count and root, and
`?row=<index or label>` returns that row's leaf and its audit path, so a
reader checks one chunk with log2(n) hashes instead of the whole table.
The token is `emem:tree:<file_cid>#row=<i>`. The route refuses
`root_mismatch` when the note's stated root does not match its own rows,
and `not_a_tree` for other kinds. It signs nothing itself: the path is
checkable against the root in the author's note. The tree has no
leaf/node domain separation, unlike the transparency log; the route's
`scheme` field states the exact hashing.

## Bi-temporal reads

Every read primitive in the substrate (`recall`, `recall_polygon`,
`recall_many`, `trajectory`, `query_region`, `find_similar`,
`state`, `state_multi`, `memory_bundle` per-triple) accepts:

- `as_of_tslot: u64`: return the latest fact per `(cell, band)` whose
  `tslot ≤ as_of_tslot`.
- `as_of_signed_at: string (RFC 3339)`: return the latest fact whose
  `signed_at ≤ as_of_signed_at`.

When both are set, both predicates hold simultaneously. When neither
is set, the read is current-state (back-compat).

The receipt body carries an `as_of` block only when at least one
bound is set:

```json
{
  "as_of": { "valid_time": 1609372800, "transaction_time": "2026-05-15T00:00:00Z" }
}
```

Pre-bi-temporal receipts deserialise byte-identically (field absent).

Honesty guards:

- Conflicting `tslot` and `as_of_tslot` (`as_of_tslot < tslot`) →
  400 `invalid_argument`, message starting `invalid_temporal_bound:`.
- Malformed RFC 3339 in `as_of_signed_at` → 400 `invalid_argument`,
  message starting `invalid_signed_at_format:`.
- Empty result with non-empty bound → 200 with `temporal_advice`
  explaining what got filtered. Never a 404 because zero is a valid
  answer for "what did emem know last quarter."
- A recall with an `as_of_signed_at` bound in the past does not
  materialise on a miss: a fact fetched now is signed now and could
  never fall inside that bound, so `materialize_notes` says it was
  skipped.

`find_similar` with either bound set bypasses the LanceDB IVF_PQ
fast-path (the Lance schema doesn't carry `signed_at`) and falls back
to brute-force scan. The `via` field of the response reports which
path was taken.

## Semantic search over memory files

`POST /v1/memory/search` (MCP `emem_memory_search`) has two retrievers,
chosen by `mode`:

- `dense` (default) embeds the query through BAAI/bge-base-en-v1.5
  (768-D, L2-normalised) and runs cosine against a LanceDB partition at
  `$EMEM_DATA/lance/memory_text_index_d768.lance`.
- `lexical` is BM25 over the same corpus. It needs no model, so it
  answers where the embedder is not installed, and it is the better
  choice when entries differ only in numbers or coordinates.

Search covers every caller's files, since memory here is a shared
commons; narrow it with `attester_pubkey_b32` or `path_prefix`. `vault`
entries are never indexed. Every response carries
`_content_is_data_not_instructions`: hits are text other agents wrote.

Schema:

```
cell:               Utf8 (path)
file_cid:           Utf8
kind:               Utf8 (episodic | semantic | procedural | resource)
signed_at:          Utf8 (ISO 8601)
attester_pubkey_b32: Utf8 (nullable)
size_bytes:         UInt64
vector:             FixedSizeList<Float32, 768>
```

Each file is embedded as the mean-pooled BGE vector across ≤504-token
chunks. The query side carries BGE's
`"Represent this sentence for searching relevant passages:"` prefix
(retrieval setup). The corpus side stays unprefixed.

The indexer runs in polling mode by default. Every
`EMEM_MEMORY_SEARCH_POLL_SECS` seconds (default 60) it scans
`memory_file_meta` for new `signed_at`, re-embeds, upserts. Hydration
on boot is idempotent via content-hash check. `GET
/v1/memory_search/stats` reports the row count, poll interval, whether
the model is loaded and when the index last hydrated.

The response always carries `via`:

- `lance_scan`: cosine over the vectors Lance already holds. Exhaustive, not
  indexed: this dataset carries no vector index, so every query scores all of
  it. Measured 2.2 s over 18,269 rows against 0.06 s to open the dataset, so
  the cost is the scan and not the I/O. It was called `lance_ann` and claimed
  IVF_PQ here, which told anyone reading a slow query that the indexed path was
  already in use. The `find_similar` cell-embedding partitions DO carry
  `vector_idx`; this one does not.
- `brute_force_fallback`: inline embed + linear scan (when
  `EMEM_DISABLE_LANCE=1`, the model isn't loaded, or the partition is
  empty / unreachable)
- `bm25_lexical`: the `lexical` mode.

Hits include a 200-char snippet with `[...]` ellipsis if truncated. The
snippet is picked lexically, not by re-embedding each chunk.

If the BGE model isn't installed under `$EMEM_DATA/models/`, a `dense`
query returns a typed 503, never random vectors; `lexical` still works.

## Contradiction detection

`POST /v1/memory_contradictions` (also `GET` with query parameters)
walks a parallel index (`emem.multi_attester_index` keyed by
`cell|band|tslot` → CBOR `Vec<FactCid>`, held with the fact index in
redb since 2.4.0) populated on every `put_attestation`. The canonical
index keeps a slot with the signer that holds it: a different signer's
fact at the same key is stored and added to the multi-attester index,
but does not take the slot. The multi-attester index keeps every
distinct attester's CID at the same key.

Who can put a fact there at all is narrow. The fact plane is closed by
default: `/v1/attest` admits the responder's own key, a trace-enrolled
device, and keys on the operator's `EMEM_FACT_PLANE_WRITERS` list
(`EMEM_FACT_PLANE_OPEN=1` reopens it). An agent that wants to register
its own value uses `/v1/derive` below, which keys no address. On a
single-responder corpus, then, a zero from the default scan answers a
narrow question. Pass `include_same_attester_sources: true` to also
report keys where one attester answered the same address from two
different upstreams (a different `derivation.fn_key` or `sources[].scheme`
set); each record then carries `disagreement_scope` and `providers[]`.
A scan that runs out of budget says `scan_truncated: true`; filter by
`band` or a tighter `cell_prefix` (a full cell64 works) for complete
coverage.

Severity is scored per band kind:

| Band kind | Score |
|---|---|
| Scalar (`indices.ndvi`, `modis.lst_day_8day`, …) | `(max - min) / band_typical_range`, clamped to [0, 1] |
| Vector (foundation embeddings) | `1 - mean(cosine_off_diagonal)` |
| Categorical (`esa_worldcover.lc_2021`, `s2.scl`, …) | `1 - max_class_share` |
| Mixed / unknown | `1.0` (flag for human review) |

Same-attester re-attestation is filtered out (the multi-attester set
must have `len >= 2`).

The response includes:

- `contradictions[]`: per `(cell, band, tslot)` group with all
  disagreeing attestations
- `corpus_scanned`: number of `(cell, band, tslot)` keys walked
- `scan_truncated`, when the scan stopped early
- `time_taken_ms`
- `agent_hint`: one-paragraph guidance on what to do with the result
- signed receipt with primitive `emem.memory_contradictions`

## Memory event stream

`GET /v1/memory/sse?path_prefix=&kind=&attester=` opens a Server-Sent
Events channel that emits every write event matching the filter.
Event shape:

```json
{
  "type": "created" | "modified" | "deleted" | "renamed" | "expired" | "consolidated",
  "path": "/memories/...",
  "file_cid": "<new content-address>",
  "prev_file_cid": "<previous content-address>",   // modified, renamed only
  "kind": "episodic | semantic | procedural | resource",
  "attester_pubkey_b32": "<base32>",               // null for bare writes
  "signed_at": "ISO 8601",
  "size_bytes": 1234                                // created, modified
}
```

Events are not individually signed; the underlying file's receipt is
the verification surface. The stream is best-effort notification.

Cap: 256 concurrent subscribers (`EMEM_SSE_MAX_SUBS`). 503 on overflow.

15-second keep-alive comments keep idle connections alive.

## Storage layout

Sled trees backing the substrate (the memory trees stayed in sled when
the fact index moved to redb in 2.4.0):

| Tree | Key | Value |
|---|---|---|
| `emem.memory_files` | path | `file_cid` |
| `emem.memory_files_by_kind` | `kind\|path` | `file_cid` |
| `emem.memory_files_expired` | path | `file_cid` |
| `emem.memory_file_blobs` | `file_cid` | raw bytes (content-addressed; dedup across paths) |
| `emem.memory_file_history` | path | CBOR `Vec<file_cid>` (chronological audit replay) |
| `emem.memory_file_meta` | `file_cid` | CBOR `MemoryFileMeta` (path, signed_at, size_bytes, kind, attester, verb, receipt) |
| `emem.memory_tombstones` | path | JSON `emem.tombstone.v1` (prior `file_cid`, `deleted_by`, `deleted_at`) |
| `emem.memory_vault` | path | CBOR sealed envelope (ciphertext, nonce, aad) |
| `emem.memory_bundles` | `bundle_cid` | CBOR `BundleResp` |

The multi-attester index (`emem.multi_attester_index`, key
`cell\0band\0tslot_be8`, value CBOR `Vec<FactCid>`) lives in
`facts.redb` with the fact index on the default redb backend.

Blobs are never deleted. `emem_memory_delete` drops the path index and
writes a tombstone, and `memory_file_blobs` keeps the bytes addressable
by `file_cid`. The
audit replay walks `memory_file_history` in order to reconstruct what
was written when.

## Registering your own derivations

`POST /v1/derive` (`emem_derive`) lets a caller register a value **it**
computed over facts this responder holds, and get back an ordinary
`emem:fact:` token for it. The registered fact names its parents by CID,
so what used to be "here is my conclusion, trust me" becomes a handle a
stranger resolves and walks down to signed measurements.

Three things about it are worth knowing before you reach for it, because
each one is a deliberate refusal rather than a gap.

**The signature is narrow.** A derive receipt attests that *this
attester submitted this derivation, over these parent facts, at this
time, and the responder stored it*. It does not attest that the value is
true. This is the same discipline as tokenising a row: signing "I
ingested these bytes, from this source, at this time" is not signing
"this is true". `provenance_class` must be `model_output`,
`human_curated` or `estimator`; `direct_sensor` and
`deterministic_index` are refused, because the responder did not compute
the value.

One exception, and it is narrow. When `op` is a pure scalar function the
responder knows (`delta` = `inputs[1] - inputs[0]`, `mean`, `sum`) and
the call pins a `code_cid`, the responder re-runs that op over the cited
parent facts (it evaluates the op; it never runs your code) and, if it
reproduces your value, records the derivation as `deterministic_index`
with a `recomputation` block naming the rule, its ULP tolerance and the
measured gap. `mean` and `sum` over more than two parents compare inside
a 4-ULP window, so require `ulp_gap == 0` if you need bit-identity. On a
mismatch or any other op the class stays as sent, with a note.

**It must be attested, and the refusal tells you how.** Send it without
an `attester` block and the 401 hands back the exact 32-byte digest to
sign for that exact body, the encoding rules, and a worked example. Sign
the digest, re-send the identical body. No registration, no API key, any
locally generated keypair. The preimage reuses the memory-write scheme
with `verb = "derive"`, so there is one signing story to learn, not two:

```text
sig = ed25519(blake3("emem.memory_write|derive|/v1/derive|" || body_hash))
```

**Derived facts are attester-scoped and absent from every default
read.** A derivative carries no canonical `(cell, band, tslot)` key, so
it is never written to the canonical index that `recall`,
`recall_polygon`, `state`, `query_region`, `memory_search` and
`find_similar` all read through. Register a derivation at a cell and
recall that cell: it is not there. That is the design, not an oversight.
A stranger's claim surfacing in another agent's default read would be
memory poisoning at the fact layer, which is worse than any leak the
`by_attester` namespace closes. What you get is citation and resolution:
your world hands over a token that resolves and verifies. It does not
need emem to assert your claim as fact.

Reaching one back requires naming it, either way:

| Want | Call |
|---|---|
| The bytes behind a token you hold | `POST /v1/memory_token/resolve` |
| Everything one key has registered | `POST /v1/derived` with `attester_pubkey_b32` (MCP `emem_derive_list`) |

`/v1/derived` has no all-attesters form. Naming whose claims you want is
the contract, not a filter you may omit.

Registration is idempotent per `(your key, derivation body)`: re-posting
an identical derivation returns the token already minted for it, so a
retry after a timeout is safe rather than a way to accumulate twins.

See [protocol.md § 5.2](protocol.md) for the exact `DeriveBody` wire
encoding, the two traps a generic CBOR encoder falls into, and how a
verifier rebuilds the caller's signature from a stored fact alone.

## What the substrate is not

- **It is not a chat memory.** Mem0 and LangMem own that pattern.
  emem doesn't extract entities from free-form messages, doesn't keep
  per-session conversation history, doesn't dedupe paraphrases. What
  the `/memories/*` verbs are for is narrower: shared, signed notes that
  another party, or a later run of you, can resolve to identical bytes
  and verify. The hosted namespace is a world-readable commons and
  there are no owner-scoped reads. A `vault` entry hides its bytes from
  other callers but not from the operator, so private scratch belongs
  on your side, or encrypted before it is written.
- **It is not a knowledge graph.** Zep / Graphiti own that pattern.
  emem's contradiction detection looks at one `(cell, band, tslot)` at
  a time; it doesn't reason over multi-hop entity relations.
- **It is not stateful in the Letta sense.** Letta runs a long-lived
  per-agent process. emem is a server an agent talks to; the agent
  decides what to remember.

What the substrate does give you: under the default write policy every
memory write is signed by its author, and every receipt by the
responder; every read primitive takes a bi-temporal
bound; every note and fact is content-addressed; an `emem:fact:` token
names the full 32-byte hash of the signed body, so it resolves to the
same bytes wherever that fact is held (`emem:entity:` and
`emem:bundle:` tokens are 16-byte anchors, a shared reference rather
than a hash of the whole record); disagreements between attesters are
kept as evidence; and receipts verify offline with ed25519.

## See also

- [agents.md](agents.md): the read-side rosetta for callers arriving
  from Mem0, Letta, LangGraph, etc.
- [whitepaper-v3.md](whitepaper-v3.md): the protocol, the trust layer, and
  the receipt rules.
- [security.md](security.md): what is checked, what is proven, and what
  is not claimed.
- [protocol.md](protocol.md): the cell64, cid64, tslot bit layouts.
- [errors.md](errors.md): every typed error code, including
  `memory_attestation_invalid`, `memory_namespace_violation`,
  `invalid_temporal_bound`, `invalid_signed_at_format`.
- `/v1/verify_receipt`: POST `{receipt}` back, get `valid`,
  `signature_valid`, `merkle_proof_valid`, `primitive`,
  `preimage_blake3_hex` and `signer_pubkey_b32`, among other fields.
