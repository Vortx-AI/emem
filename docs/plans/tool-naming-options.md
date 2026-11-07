# Tool naming: should emem rename its MCP tools?

Status: decision memo for the owner. It is a plan, not a shipped change.
Nobody renamed a tool. Do not rename a tool until the owner decides.

The first version of this memo is from 11 August 2026. It used https://emem.dev
and the external scorer at https://glama.ai/mcp/servers/Vortx-AI/emem/score.
This edit checks it again on 7 November 2026. Each number below carries its date.

## Recommendation

Do not rename the tools. A rename breaks callers that you cannot update.
The score gain is small, and the same gain comes from cheaper work.
[Recommendation in detail](#recommendation-in-detail) gives the reasons.

## What changed between August and November 2026

| Item | 11 August 2026 | 7 November 2026 |
|---|---|---|
| Tools on `/mcp/full` | 107, all with the `emem_` prefix | 116; `search` and `fetch` have no prefix, because a ChatGPT connector calls those names |
| `emem_triple_consensus` | listed | removed with the retired foundation models |
| Tools that Glama scores | 16 | 18 (the core list that `/mcp` advertises) |
| Glama Naming Consistency | 4 of 5 ("the inconsistency is minor") | 3 of 5, scored 19 October 2026 |
| Lowest tool score | 4.0 (`emem_guard_verdict`) | 4.2 (`emem_memory_token` and `emem_memory_token_resolve`) |
| Unknown tool name | tool result with `isError: true` | JSON-RPC error `-32602` that suggests the nearest names |
| Gate on new tool names in `scripts/` | suggested | not built |

The live tool list is `emem_tools` (MCP) or `GET /v1/tools` (REST).

## The inconsistency is real

All tool names except `search` and `fetch` start with `emem_`.
After the prefix, the names follow five grammatical patterns.
This count is from 11 August 2026, over the 107 tools of that day:

| Pattern | Count | Examples |
|---|---|---|
| Bare verb | 10 | `emem_ask`, `emem_locate`, `emem_recall`, `emem_derive`, `emem_diff` |
| Bare noun | 30 | `emem_entity`, `emem_intent`, `emem_bands`, `emem_terrain`, `emem_at` |
| Verb first | 10 | `emem_find_similar`, `emem_verify_receipt`, `emem_recall_polygon` |
| Verb last | 12 | `emem_entity_resolve`, `emem_entity_link`, `emem_memory_token_resolve` |
| Noun phrase, no verb | 45 | `emem_guard_verdict`, `emem_band_raster`, `emem_triple_consensus` |

An agent can make two mistakes because of this.

1. The same verb is at the start of some names and at the end of others.
   On 7 November 2026, `verify` starts `emem_verify_receipt` and ends
   `emem_echo_verify` and `emem_trace_verify`. `resolve` ends six names,
   for example `emem_entity_resolve` and `emem_raster_resolve`.
   The name does not show the convention, so the agent must look it up.
2. The `emem_memory_*` family mixes actions and things. It has 14 tools
   on 7 November 2026. `emem_memory_create`, `emem_memory_search` and
   `emem_memory_view` are actions. `emem_memory_bundle`, `emem_memory_token`
   and `emem_memory_contradictions` are things. The names do not show
   which is which.

`emem_at` is a preposition. It does not fit any naming scheme.

Glama gave the same reasons on 19 October 2026. It names `search` and
`fetch` as unprefixed. It says that the set mixes verb names such as
`locate` and `recall` with noun names such as `memory_token` and `entity`.

## What a rename breaks

A tool name is the public call surface. Other people copy it into places
that do not change when the server changes.

| Place | Effect of a rename |
|---|---|
| Docs, `README.md` and the web pages | You can fix them in one pass. No check finds a tool name in prose that no longer exists. `scripts/sync_counts.py` checks counts only. |
| The two SDKs (`@vortxai/emem` on npm, `ememdev` on PyPI) | Both need a release. A caller that pins an old version keeps the old names. |
| Clients that cache the tool list | The MCP specification lets clients cache the list and gives servers `ttlMs` and `cacheScope` for it. It says that a fixed order lets clients cache the list and improves prompt cache hit rates. A cached client calls the old name and gets an error. This is the largest cost. |
| `server.json` and MCP directory entries | Other people crawl them on their own schedule. |
| Signed notes that name a tool | Notes are content addressed. Nobody can edit them, so old names stay in the record. |

On 7 November 2026, a call to an unknown name returns a JSON-RPC error:

```json
{"error": {"code": -32602, "message": "Unknown tool: emem_find_similiar. Did you mean emem_find_similar? emem_tools lists the catalog."}, "id": 1, "jsonrpc": "2.0"}
```

The suggestion comes from an edit distance over the tool list
(`mcp_unknown_tool_message` in `crates/emem-api-rest/src/lib.rs`).
It helps after a small typo. A renamed tool usually differs by more
characters, so the error then gives no suggestion. The call still fails.

## Aliases and deprecation in MCP

The MCP specification has no alias field and no deprecation flag.
The memo checked this on 28 July 2026, in the revision after 2025-11-25.
The server speaks 2025-11-25 (from `initialize` on 7 November 2026).
The specification says only that names "SHOULD be unique within a server".

Three mechanisms do part of the job.

1. `title` is separate from `name`. It is the display name for people.
   The server sets it. On 7 November 2026, `emem_find_similar` has the
   title "k-NN over the corpus by embedding". Glama quotes name fragments
   in its comments, so it reads `name`, not `title`.
2. `notifications/tools/list_changed` tells a subscribed client that the
   list changed. It does not make an old name work. On 7 November 2026
   the server declares `listChanged: false`, so it does not send it.
3. The server can keep an old name working without protocol support.
   `tools/call` does not depend on what `tools/list` advertises.
   On 7 November 2026, `emem_capabilities` is not in the 18 tools on
   `/mcp`. A call to it at `/mcp` still returns a normal result.
   So after a rename, a lookup table could keep the old names working
   while `tools/list` shows only the new names.

## What the score could become

The arithmetic uses two assumptions. The page does not confirm either.

- Tool Definition Quality is 70 percent of the total. It is
  `0.6 x mean + 0.4 x minimum` over the scored tools.
- Server Coherence is 30 percent. Its four dimensions have equal weight:
  Disambiguation, Naming Consistency, Tool Count and Completeness.

With these assumptions, one point on one coherence dimension adds
`0.3 x 0.25 x 1 = 0.075` to the total out of 5.

### Score on 11 August 2026

| Input | Value |
|---|---|
| Coherence | Disambiguation 4, Naming 4, Tool Count 5, Completeness 4; mean 4.25 |
| Tool scores | mean 4.7 over 16 tools, minimum 4.0 (`emem_guard_verdict`) |
| Tool Definition Quality | `0.6 x 4.7 + 0.4 x 4.0 = 4.42` |
| Total | `0.7 x 4.42 + 0.3 x 4.25 = 4.369` |

The memo compared two changes on that day:

| Change | New total | Gain |
|---|---|---|
| Rename all tools; Naming goes from 4 to 5 | 4.444 | +0.075 |
| Rescore `emem_guard_verdict` after the annotation fix in commit 6d54683 (about 4.8; the floor becomes the next tool, 4.2) | 4.446 | +0.077 |

Glama had marked the Behavioral Transparency of `emem_guard_verdict` 1 out of 5.
The reason was a contradiction in its annotations that commit 6d54683 removed.

The rescore happened. On 19 October 2026, `emem_guard_verdict` scored 4.5.

### Score on 19 October 2026

| Input | Value |
|---|---|
| Coherence | Disambiguation 3, Naming 3, Tool Count 3, Completeness 4; mean 3.25 |
| Tool scores | mean 4.544 over 18 tools, minimum 4.2 (`emem_memory_token`, `emem_memory_token_resolve`) |
| Tool Definition Quality | `0.6 x 4.544 + 0.4 x 4.2 = 4.407` |
| Total | `0.7 x 4.407 + 0.3 x 3.25 = 4.060`; the page shows 4.1 |

The formula gives 4.060. Rounded to one decimal, that is the 4.1 on the page.
That supports the two assumptions. It does not prove them.

| Change | New total | Gain |
|---|---|---|
| Rename all tools; Naming goes from 3 to 4 | 4.134 | +0.075 |
| Lift the two 4.2 tools to 4.6; the floor becomes 4.4 | 4.134 | +0.075 |

The result is the same as in August. The minimum tool score is 40 percent
of the 70 percent part. Naming is one quarter of the 30 percent part.
So the rubric punishes one weak tool more than an untidy naming scheme.

## Recommendation in detail

Do not rename anything. The reasons, most important first:

1. The gain is +0.075 out of 5. The cost is a coordinated break across
   two SDK registries, the docs, the directory listings and every cached
   client. Signed notes keep the old names for ever.
2. The last point may not be reachable. `emem_at` and the bare nouns do
   not fit a verb and noun scheme without names that are harder to read.
3. Better descriptions on the lowest scored tools give the same gain.
   They break no caller.

Glama called the problem minor in August. In October it scored Naming 3
and did not call it minor. This changes the size of the gap, not the
arithmetic. Reason 1 still holds.

If the owner wants the naming point later, apply one convention to new
tools only. A check in `scripts/` could require that a new tool name
matches `emem_<noun>_<verb>` or a short allowlist. Nobody calls a new
name yet, so the check costs callers nothing. It is small and you can
remove it. This check does not exist on 7 November 2026.

If the owner wants the rename anyway, the cost is:

- about one day of edits across docs, SDKs and registry files;
- a server alias table that must stay for ever;
- two SDK releases;
- an unknown number of cached clients that call old names.

The alias table makes the rename safe. It also makes it permanent: the old
names still work, so the inconsistency stays. Only the advertised list changes.

## What this memo could not check

- Whether Glama gives the four coherence dimensions equal weight. If
  the Naming weight is lower, the rename is worth less. If Naming were the
  only dimension, one point would add `0.3` out of 5. That is the upper limit.
  The August memo gave `0.06`. That was the fraction `0.3 x 1/5`, not a
  score out of 5.
- Whether Glama scores `name` alone or `name` with `title`. Its comments
  quote name fragments, so this memo assumes `name`.
- When Glama scores again. The 19 October 2026 score is the latest that
  the page showed on 7 November 2026.
