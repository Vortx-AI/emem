# Tool naming consistency: an options memo

Status: decision memo for the owner. Nothing here has been done. No tool has
been renamed and none should be until this is decided.

Written 2026-08-11 against the live responder at https://emem.dev and the
external scorer at https://glama.ai/mcp/servers/Vortx-AI/emem/score.

## Why this memo exists, and one correction to the premise

Glama scores an MCP server on two axes. Tool Definition Quality is 70 percent
of the total and is computed as `60% x mean + 40% x minimum` across tools, so
the worst tool sets a floor. Server Coherence is the other 30 percent, and
Naming Consistency is one of its four dimensions, alongside Disambiguation,
Tool Count and Completeness.

The premise this memo was commissioned under was that Naming Consistency
scores 3 out of 5. It does not, as of the page fetched 2026-08-11: it scores
**4 out of 5**, and the scorer's own comment now ends "The inconsistency is
minor and the overall scheme remains predictable." The 3 out of 5 and its
harsher wording belong to an earlier evaluation.

That changes the question. The gap being argued over is one point on one of
four dimensions inside the smaller half of the score, and the scorer has
already said in writing that it considers the problem cosmetic. The arithmetic
below puts a number on it.

## The inconsistency is real

It is worth being clear that the scorer is describing something true. All 107
tool names share the `emem_` prefix; after that they follow five different
grammatical patterns. Grouped mechanically by the shape of the name after the
prefix:

| Pattern | Count | Examples |
|---|---|---|
| Bare verb | 10 | `emem_ask`, `emem_locate`, `emem_recall`, `emem_derive`, `emem_diff` |
| Bare noun | 30 | `emem_entity`, `emem_intent`, `emem_bands`, `emem_terrain`, `emem_at` |
| Verb first | 10 | `emem_find_similar`, `emem_verify_receipt`, `emem_recall_polygon` |
| Verb last | 12 | `emem_entity_resolve`, `emem_entity_link`, `emem_memory_token_resolve` |
| Noun phrase, no verb | 45 | `emem_guard_verdict`, `emem_band_raster`, `emem_triple_consensus` |

Two specific things an agent could reasonably trip over:

1. **The same verb sits on both sides of the name.** `emem_verify_receipt`
   puts `verify` first; `emem_echo_verify` and `emem_trace_verify` put it
   last. `emem_find_similar` puts the verb first; `emem_entity_resolve`,
   `emem_cube_resolve` and `emem_raster_resolve` put it last. Nothing in the
   surface tells you which convention a given tool follows, so the name cannot
   be guessed and has to be looked up.
2. **The `emem_memory_*` family is internally inconsistent.** It contains
   `emem_memory_create`, `emem_memory_insert`, `emem_memory_delete`,
   `emem_memory_rename` and `emem_memory_view`, which are noun then verb; also
   `emem_memory_search`, same shape; and also `emem_memory_bundle`,
   `emem_memory_token` and `emem_memory_contradictions`, which are noun then
   noun. So `emem_memory_search` is an action and `emem_memory_token` is a
   thing, and the names do not signal the difference.

`emem_at` is a preposition and is the single hardest name to place in any
scheme.

## What a rename would break

A tool name is not an internal symbol. It is the public call surface, and it
is copied into places that do not update when the server does.

* **Every doc and page that names a tool.** `emem_` tool names appear across
  README.md, docs/, the baked web pages and the whitepaper. These are fixable
  in one pass, and `scripts/sync_counts.py` would catch counts, but nothing
  currently gates that a tool name mentioned in prose still exists.
* **Both SDKs.** The published `@vortxai/emem` and `ememdev` packages pin
  names. A rename means a release on both registries, and every pinned
  dependency keeps calling the old name until its owner upgrades.
* **Cached clients, which is the serious one.** The MCP specification tells
  clients they may cache the tool list, and gives servers `ttlMs` and
  `cacheScope` to encourage it: "Deterministic ordering enables clients to
  reliably cache the tool list and improves LLM prompt cache hit rates."
  A client holding a cached list calls the old name and gets
  `-32602 unknown tool`. Verified live on 2026-08-11:

  ```
  tool error (-32602): unknown tool 'emem_find_similiar';
  call tools/list for the catalog
  ```

  This is returned as a tool result with `isError: true`, which is the
  recoverable form the specification recommends, and the message names the
  recovery. That is about as soft as this failure can be made, but it is still
  a failed call in someone else's agent loop that we caused.
* **The MCP registry listing and directory entries.** `server.json` and the
  registry copies describe the surface. Those are re-crawled on someone else's
  schedule, not ours.
* **The arcade and the A2A agents.** Other agents hold signed notes citing
  tool names. Those notes are content addressed and cannot be edited, so the
  historical record would permanently reference names that no longer resolve.

## Does MCP have an alias or deprecation mechanism?

**No.** Checked against the current specification, 2026-07-28, the revision
after the 2025-11-25 one this server serves. A tool definition carries exactly
`name`, `title`, `description`, `icons`, `inputSchema`, `outputSchema`,
`annotations` and `_meta`. There is no deprecation flag, no alias field, no
`renamedFrom`, and the prose says nothing about renaming tools. The only
relevant guidance is that names "SHOULD be unique within a server".

Three mechanisms exist that partly substitute:

1. **`title` is already decoupled from `name`.** The specification defines
   `title` as the optional human readable display name. This server already
   sets it: `emem_find_similar` carries the title "k-NN over the corpus by
   embedding". So the presentation layer is already free of the naming scheme.
   Whether Glama scores `name` or `title` matters here, and its comment quotes
   the `emem_` prefix and the underscore patterns, so it is reading `name`.
2. **`notifications/tools/list_changed`** tells subscribed clients the list
   moved. It does not help a client that is not subscribed, and it does not
   make an old name resolve.
3. **Server side aliasing, which needs no protocol support and already
   works.** `tools/call` dispatch is independent of what `tools/list`
   advertises. Verified live on 2026-08-11: `emem_capabilities` is not on the
   first `tools/list` page at `/mcp`, and calling it at `/mcp` returns a
   normal result. So old names could keep dispatching indefinitely while only
   new names are advertised. This is the real deprecation path if a rename
   ever happens, and it costs one lookup table.

## What the score would plausibly become

Stated as arithmetic with the assumption made visible: the four coherence
dimensions are assumed equally weighted, which the page does not confirm.

Current, from the page on 2026-08-11:

* Coherence dimensions: Disambiguation 4, Naming 4, Tool Count 5,
  Completeness 4. Mean 4.25.
* Tool scores: mean 4.7 across 16 scored tools, minimum 4.0
  (`emem_guard_verdict`).
* Tool Definition Quality = `0.6 x 4.7 + 0.4 x 4.0` = 4.42.
* Overall = `0.7 x 4.42 + 0.3 x 4.25` = **4.369**.

If a full rename lifted Naming from 4 to 5 and nothing else moved:

* Coherence mean becomes 4.5. Overall = `0.7 x 4.42 + 0.3 x 4.5` = **4.444**.
* Gain: **+0.075 out of 5**, about 1.5 percent.

Compare that with the `emem_guard_verdict` annotation fix that already
shipped, in commit 6d54683, and is live but has not been rescored. Its
Behavioral Transparency was marked 1 out of 5 for a contradiction that no
longer exists. If a rescore takes that tool to roughly 4.8, the floor rises to
the next lowest tool at 4.2:

* Tool Definition Quality = `0.6 x 4.75 + 0.4 x 4.2` = 4.53.
* Overall = `0.7 x 4.53 + 0.3 x 4.25` = **4.446**.
* Gain: **+0.077 out of 5**.

So renaming all 107 tools and waiting for a rescore of an already shipped
one line annotation change are worth the same amount, to three decimal
places. One of them is free and already done.

The reason is structural, not a coincidence. Naming is a quarter of the
30 percent axis, so it can move the total by at most `0.3 x 0.25 x 1/5`.
The minimum tool score is 40 percent of the 70 percent axis and has no such
divisor. The rubric is built to punish one bad tool, not an untidy scheme.

## Recommendation

**Do not rename anything.** Three reasons, in order of weight:

1. The measurable gain is +0.075 out of 5. The cost is a coordinated break
   across two SDK registries, every doc, the registry listings, and every
   cached client, plus a permanent mismatch with signed historical notes that
   cannot be rewritten.
2. The scorer has already downgraded this from a real complaint to a minor
   one, in its own words, and moved it from 3 to 4. The remaining point may
   not be reachable at all: `emem_at` and the bare nouns are not going to fit
   a verb_noun scheme without names that are worse to read.
3. The same gain is available from work already finished. The floor, not the
   naming, is what the rubric actually weights.

**Instead, if the naming point is wanted later**, the cheap version is to hold
new tools to one convention rather than converting the existing 107. A gate in
`scripts/` could require that any newly added tool name matches
`emem_<noun>_<verb>` or a short allowlist, which stops the spread at zero cost
to callers, since it only ever applies to names nobody has called yet. That is
a small, reversible change and does not need this decision.

**Cost if the owner overrules this and wants the rename anyway:** roughly one
day of edits across docs, SDKs and registry files, plus a server side alias
table that must be kept forever, plus two SDK releases, plus an unbounded tail
of cached clients calling old names. The alias table is what makes it safe,
and it is also what makes it permanent: the inconsistency does not actually go
away, it just stops being advertised.

## What I could not verify

* Whether Glama weights the four coherence dimensions equally. The arithmetic
  above assumes it does. If Naming is weighted lower, the rename is worth even
  less; if higher, proportionally more, but it cannot exceed
  `0.3 x 1/5 = 0.06` per dimension point even if Naming were the only
  dimension that mattered at 100 percent weight, which it plainly is not.
* Whether Glama scores `name` or the combination of `name` and `title`. Its
  comment quotes name fragments, which is why this memo treats it as `name`.
* When Glama will next rescore. The page carries no scoring timestamp, so the
  +0.077 from the `emem_guard_verdict` fix is projected, not observed.
