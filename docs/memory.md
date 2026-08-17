# Memory substrate

> The formal definition of what this substrate is (the observation tuple,
> the property table, the memory algebra) lives in [the memory model](model.md).

The memory substrate is the writable layer of emem. Below it sit the
read primitives (`/v1/recall`, `/v1/state`, `/v1/find_similar`,
`/v1/trajectory`) whose facts come from upstream materialisers. Above
it sit the agents that read, cite, write, and stream memory through the
same ed25519 receipt surface as every other primitive.

This page is the wire reference. The agents guide ([agents.md](agents.md))
covers the read-side rosetta; this document covers everything the agent
puts back.

## What's in the substrate

| Layer | Endpoint | Wire shape | Returns |
|---|---|---|---|
| State (single encoder) | `POST /v1/state` | `{cell, encoder?, view?, as_of_tslot?, as_of_signed_at?}` | dense vector + memory_token |
| State (full cube) | `POST /v1/state` view=cube | same | 1792-D state vector + coverage map |
| State fan-out | `POST /v1/state_multi` | `{cell, encoders?, as_of_*?}` | per-encoder dense map |
| State delta | `POST /v1/state_diff` | `{cell, encoder?, tslot_a, tslot_b}` | residual + cosine + both fact_cids |
| Memory token | `POST /v1/memory_token` | `{cell, fact_cid}` | `emem:fact:<cell>:<fact_cid>` |
| Memory token resolve | `POST /v1/memory_token/resolve` | `{token}` | full signed fact body |
| Memory bundle | `POST /v1/memory_bundle` | `{triples, purpose?}` | signed envelope + `emem:bundle:<bundle_cid>` |
| Memory bundle resolve | `GET /v1/memory_bundle/<token>` | path param | same envelope |
| Memory file write | MCP `emem_memory_create` | `{path, file_text, kind?, attester?}` | `file_cid` + signed receipt |
| Memory file edit | MCP `emem_memory_str_replace`, `emem_memory_insert` | `{path, old_str, new_str}` etc. | new `file_cid` + receipt |
| Memory file read | MCP `emem_memory_view` | `{path, view_range?}` | content or directory listing |
| Memory file rename | MCP `emem_memory_rename` | `{old_path, new_path}` | new path index + receipt |
| Memory file delete | MCP `emem_memory_delete` | `{path}` | path drop (blob retained, history preserved) |
| Memory list by kind | MCP `emem_memory_list_by_kind` | `{kind, prefix?, limit?}` | typed listing sorted by signed_at desc |
| Memory file semantic search | `POST /v1/memory/search` | `{q, k?, kind?, path_prefix?, attester_pubkey_b32?}` | ranked hits + snippets |
| Memory event stream | `GET /v1/memory/sse?path_prefix=&kind=&attester=` | query string | `text/event-stream` of writes |
| Multi-attester contradictions | `POST /v1/memory_contradictions` | `{cell_prefix?, band?, window_unix_s?, min_severity?, limit?}` | severity-scored disagreements |

The pre-rename prefixes `memt:`, `memb:`, and `meme:` still resolve.

## The four kinds

Memory files carry a `kind` from the CoALA / LangMem agent-memory
ontology. Default is `resource` so callers that don't pass `kind` keep
the back-compat Anthropic memory-tool shape.

| `kind` | Purpose | TTL default |
|---|---|---|
| `episodic` | Observations of events. The agent ran X at Y on date Z. | 30 days |
| `semantic` | Durable learned facts. "Mato Grosso has tropical climate." | infinite |
| `procedural` | Playbooks, how-to notes. "When user asks flood risk, call /v1/water + /v1/elevation." | infinite |
| `resource` | Generic durable scratchpad. The default Anthropic memory-tool target. | 90 days |

TTL pass (`EMEM_MEMORY_TTL_ENABLED=1`) sweeps every hour. Expired files
move from `memory_files` to `memory_files_expired`; the blob is retained
under `memory_file_blobs` (content-addressed; never deleted). Per-kind
overrides via `EMEM_MEMORY_TTL_{RESOURCE,EPISODIC,SEMANTIC,PROCEDURAL}_DAYS`
(0 = infinite).

The consolidation pass (`EMEM_MEMORY_CONSOLIDATION_ENABLED=1`) runs
every 24 h. For every `/memories/by_attester/<pubkey>/<sub>/` with more
than 50 episodic files older than 7 days, it concatenates the bodies in
chronological order, signs the result as a `semantic` kind file at
`.consolidated/<unix_ts>.md`, and stamps `superseded_by: <consolidated_cid>`
on every original's metadata. Originals stay accessible via
`memory_file_history`.

## Capability binding

Paths under `/memories/by_attester/<pubkey_b32_short>/...` are
write-restricted to the holder of the corresponding ed25519 private
key. `pubkey_b32_short` is the first 8 chars of `base32_nopad_lc(pubkey)`.

The `attester` block:

```json
{
  "pubkey_b32": "<base32-nopad-lowercase 52-char ed25519 pubkey>",
  "sig_b32":    "<base32-nopad-lowercase 103-char ed25519 signature>"
}
```

The signed preimage is:

```
blake3("emem.memory_write|" + verb + "|" + path + "|" + body_hash)
```

where `body_hash = blake3(canonical body bytes)`. What counts as the
body depends on the verb:

| verb | `path` | body |
|---|---|---|
| `create` | the file written | the `file_text` string sent |
| `str_replace` | the file edited | the whole file *after* the edit |
| `insert` | the file edited | the whole file *after* the edit |
| `delete` | the file removed | empty, i.e. `blake3("")` |
| `rename` | the **new_path** | the **old_path** string |

`rename` is the only verb whose signature binds two paths. The
destination rides the preimage's `path`, the source rides its
`body_hash`, so one signature authorises one specific move. Signing the
destination alone would let a captured signature drag any other file to
that destination. When the source is itself under `by_attester`, the
responder additionally checks the key owns it, and refuses with 403
`memory_namespace_violation` (`reason: source_namespace`) if not.

Wrong key → 401 `memory_attestation_invalid`.
Wrong namespace → 403 `memory_namespace_violation`.

Bare `/memories/...` is **not** anyone-writable by default. A release
build with no env set refuses every unattested write with 401
`memory_attestation_required`. The operator picks the policy:

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
`["pubkey:<b32>", path]`; for bare writes it stays `[path]`. This means
the multi-attester index and the contradiction detector see attested
writes as first-class.

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
  400 `invalid_temporal_bound`.
- Malformed RFC 3339 in `as_of_signed_at` → 400
  `invalid_signed_at_format`.
- Empty result with non-empty bound → 200 with `temporal_advice`
  explaining what got filtered. Never a 404 because zero is a valid
  answer for "what did emem know last quarter."

`find_similar` with either bound set bypasses the LanceDB IVF_PQ
fast-path (the Lance schema doesn't carry `signed_at`) and falls back
to brute-force scan. The `via` field of the response reports which
path was taken.

## Semantic search over memory files

`POST /v1/memory/search` embeds the query through
BAAI/bge-base-en-v1.5 (768-D, L2-normalised) and runs cosine against
a per-dim LanceDB partition at `$EMEM_DATA/lance/memory_text_index_d768.lance`.

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
on boot is idempotent via content-hash check.

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

Hits include a 200-char snippet around the best-matching chunk with
`[...]` ellipsis if truncated.

If the BGE model isn't installed at `$EMEM_DATA/models/bge-base-en-v1.5/`,
the endpoint returns a typed 503, never random vectors.

## Contradiction detection

`POST /v1/memory_contradictions` walks a parallel sled index
(`emem.multi_attester_index` keyed by `cell|band|tslot` → CBOR
`Vec<FactCid>`) populated lazily on every `put_attestation`. The
canonical index stays last-writer-wins (sled overwrites on the same
key); the multi-attester index keeps every distinct attester's CID at
the same key.

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

Sled trees backing the substrate:

| Tree | Key | Value |
|---|---|---|
| `emem.memory_files` | path | `file_cid` |
| `emem.memory_files_by_kind` | `kind\|path` | `file_cid` |
| `emem.memory_files_expired` | path | `file_cid` |
| `emem.memory_file_blobs` | `file_cid` | raw bytes (content-addressed; dedup across paths) |
| `emem.memory_file_history` | path | CBOR `Vec<file_cid>` (chronological audit replay) |
| `emem.memory_file_meta` | `file_cid` | CBOR `MemoryFileMeta` (path, signed_at, size_bytes, kind, attester, verb, receipt) |
| `emem.memory_bundles` | `bundle_cid` | CBOR `BundleResp` |
| `emem.multi_attester_index` | `cell\0band\0tslot_be8` | CBOR `Vec<FactCid>` |

Blobs are never deleted. `emem_memory_delete` drops the path index but
`memory_file_blobs` keeps the bytes addressable by `file_cid`. The
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
true. The responder did not compute it and cannot recompute it. This is
the same discipline as tokenising a row: signing "I ingested these bytes,
from this source, at this time" is not signing "this is true".

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
| Everything one key has registered | `POST /v1/derived` with `attester_pubkey_b32` |

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
  the `/memories/*` verbs are for is narrower and stronger: shared,
  signed notes that another party, or a later run of you, can resolve
  to identical bytes and verify. Private single-agent scratch stays
  local until owner-scoped reads ship; the hosted namespace is a
  world-readable commons.
- **It is not a knowledge graph.** Zep / Graphiti own that pattern.
  emem's contradiction detection looks at one `(cell, band, tslot)` at
  a time; it doesn't reason over multi-hop entity relations.
- **It is not stateful in the Letta sense.** Letta runs a long-lived
  per-agent process. emem is a server an agent talks to; the agent
  decides what to remember.

What emem covers, alone among production memory systems: every memory
operation produces a signed receipt; every read carries a bi-temporal
axis; every write is content-addressed; the same memory token resolves
to the same bytes on any replica; multi-attester disagreements are
first-class; the entire surface is verifiable offline through ed25519.

## See also

- [agents.md](agents.md): the read-side rosetta for callers arriving
  from Mem0, Letta, LangGraph, etc.
- [whitepaper-v2.md](whitepaper-v2.md): the protocol, the trust layer, and
  the receipt rules.
- [protocol.md](protocol.md): the cell64, cid64, tslot bit layouts.
- [errors.md](errors.md): every typed error code, including
  `memory_attestation_invalid`, `memory_namespace_violation`,
  `invalid_temporal_bound`, `invalid_signed_at_format`.
- `/v1/verify_receipt`: POST any receipt back, get
  `{valid, primitive, preimage_blake3_hex, signer_pubkey_b32}`.
