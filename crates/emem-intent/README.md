# emem-intent

`emem-intent` turns a typed agent goal into a list of primitive calls. The
goal is an `Intent`, such as "where is X" or "did this band change". The
function `plan` maps each `Intent` to one or more calls, for example
`emem_locate` or `emem_diff`. This crate does not run the calls. The
responder runs them.

You need this crate when you add an intent type or change which primitive
an intent calls. To use intents from any language, you do not need it:
send the intent to `POST /v1/intent`, or call the MCP tool `emem_intent`.

| | |
|---|---|
| Version | 2.4.4 (workspace version in the root `Cargo.toml`) |
| Size | One file, `src/lib.rs`, 664 lines (about 210 of them are tests) |
| Used in code by | `emem-api-rest` (`POST /v1/intent` and the MCP tool `emem_intent`) |
| Listed as a dependency, not used in code | `emem-cli`, `emem-mcp` |
| Tests | 5 unit tests in `src/lib.rs` |
| I/O | None. `plan` is a pure function. |

## Try it

The live responder plans the intent, runs every call and returns the
plan and the results together. Ask where Mount Fuji is:

```sh
curl -s https://emem.dev/v1/intent -H 'Content-Type: application/json' \
  -d '{"type":"where_is","description":"Mount Fuji"}' \
  | jq -c '{plan, primitive: .results[0].primitive, ok: .results[0].ok, cell64: .results[0].result.cell64}'
# {"plan":{"calls":[{"args":{"place":"Mount Fuji"},"primitive":"emem_locate"}]},"primitive":"emem_locate","ok":true,"cell64":"defi.zb592.nUkO.zEzE"}
```

It needs `curl` and `jq`, and no key. The command ran on 7 November 2026.
The full response has three fields:

| Field | Content |
|---|---|
| `plan` | The output of `plan`: a list of calls, each with `primitive` and `args` |
| `results` | One entry for each call: `primitive`, `args`, `ok`, and `result` or `error` |
| `composite_suggestions` | Algorithms from `/v1/algorithms` that fit this intent type. For `where_is` the list `applicable` is empty. |

The MCP tool `emem_intent` returns only `plan` and `results`. When the
result is larger than the host budget, MCP leaves out `results` and names
`POST /v1/intent` in `_emem_truncation`.

A failed call does not stop the plan. Its entry has `ok: false` and an
`error` with `code` and `message`.

## The eight intent types

The JSON field `type` selects the variant. These are all the variants in
`src/lib.rs`:

| `type` | Fields (optional in brackets) | Calls that `plan` makes |
|---|---|---|
| `where_is` | `description` | `emem_locate` with `place` = `description` |
| `what_is_here` | [`cell`, `place`, `description`]. All optional. | `emem_recall` if the request has `cell`. Else `emem_ask`. |
| `ask` | `description`, [`place`, `cell`, `lat`, `lng`] | `emem_ask` with `q` = `description` |
| `is_like` | `a`, `b` | `emem_recall` of band `geotessera` at `a`, the same at `b`, then `emem_compare` |
| `did_change` | `cell`, `band`, `window` | `emem_diff` with `tslot_a` and `tslot_b` from `window` |
| `find_like` | `key`, [`k`, `filter`] | `emem_find_similar` |
| `confirm` | `claim`, `cell` | `emem_verify` |
| `area_over_time` | `start_date`, `end_date`, and `geometry`, `bbox` or `areas`, [`index`, `max_scenes`] | `emem_field_series` |

Some details that the table does not show:

- `what_is_here` without `cell` sends `q` = `description`, or "what is
  here" if the request has no `description`. It sends `place` = `place`, else
  `description`, else an empty string.
- `window` in `did_change` holds two tslots, not dates. Get tslots from
  `emem_trajectory`. Both go out as CBOR integers.
- `filter` (in `find_like`) and `claim` (in `confirm`) are `Claim` values
  from [emem-claim](../emem-claim/README.md).
- `Intent::kind()` returns the wire tag, for example `"did_change"`.

## How the responder runs a plan

`emem-api-rest` has two paths to this crate:

| Path | Code | Unknown `type` |
|---|---|---|
| `POST /v1/intent` | `post_intent` in `src/lib.rs` | HTTP 400 with `code: invalid_argument`. The message lists the eight valid types. |
| MCP tool `emem_intent` | the `emem_intent` arm of the MCP dispatcher | A result with `status: needs_intent_type`, the valid types and examples. It is not a tool error. |

Both paths call `plan`, convert each `args` from CBOR to JSON and run the
calls in order through the MCP tool dispatcher. We tested the HTTP 400 with
`"type":"teleport"` on 7 November 2026.

## Where the comments and the code differ

We checked these points in the code on 7 November 2026:

| The text says | The code does |
|---|---|
| A new variant degrades through `unknown_intent_type` (doc comment on `Intent`) | No code emits `unknown_intent_type`. See the table above for what happens. |
| The test `plan_args_round_trip_into_their_primitives` checks every variant's args (comment on `did_change_args`) | No test with that name exists in the workspace. Only `did_change` has a test of argument types. |
| "learned planner deferred to v0.1" (`Cargo.toml` description) | Only the heuristic planner exists. No learned planner ships. |
| The `also` hint in the `/v1/ask` response lists the valid types | It lists seven. `area_over_time` is not in the list. |

Three helpers drop data with no error. If `filter` or `claim` does not
convert to CBOR, the call goes out without that field. If the
`area_over_time` arguments do not convert, the call goes out with an empty
argument map.

## Rules for changing this crate

- `Intent` is a wire format. Keep `#[serde(tag = "type", rename_all =
  "snake_case")]`. A new variant needs a new arm in `kind` and in `plan`.
- Encode each argument as the CBOR type that the primitive's request type
  reads. `did_change_args` exists because text tslots failed with
  `invalid type: string "19723", expected u64`.
- Update the table of types in the `emem_intent` tool description in
  `crates/emem-mcp/src/lib.rs` and the algorithm hints in
  `suggest_algorithms_for_intent` in `emem-api-rest`.
- Keep `plan` pure: no I/O and no network.

Run the tests with `cargo test -p emem-intent`. We did not run cargo for
this page. The test count comes from the `#[test]` lines in `src/lib.rs`.

## What ships and what does not

| Ships | Does not ship |
|---|---|
| Eight intent types and a fixed mapping to primitives | A learned planner |
| Plan and run in one call through `/v1/intent` and `emem_intent` | A plan-only mode. The responder always runs the plan. |
| Algorithm hints in `composite_suggestions` (HTTP path only) | A run of those algorithms. The agent applies them. |

## Read more

- [docs/agents.md](../../docs/agents.md): where `emem_intent` fits next to
  `emem_ask` and `emem_diff`.
- [emem-claim](../emem-claim/README.md): the `Claim` type in `confirm` and
  `find_like`.
- [emem-api-rest](../emem-api-rest/README.md): the router that serves
  `/v1/intent`.
- [docs/glossary.md](../../docs/glossary.md): cell64, band and tslot.
