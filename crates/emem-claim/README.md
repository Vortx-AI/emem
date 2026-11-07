# emem-claim

`emem-claim` defines `Claim`, the structured predicate that emem tests
against signed facts. A claim names a band, an operator, a value and a time.
An example is "`copdem30m.elevation_mean` is greater than 3000 at tslot 0".

You need this crate when you change the claim shape or its operators. To
test a claim from any language, you do not need it: send the claim to
`POST /v1/verify`.

| | |
|---|---|
| Version | 2.4.4 (workspace version in the root `Cargo.toml`) |
| Size | One file, `src/lib.rs`, 76 lines |
| Used in code by | `emem-primitives` (`verify`, `find_similar` filter), `emem-intent` (`confirm`, `find_like`) |
| Listed as a dependency, not used in code | `emem-api-rest`, `emem-cli`, `emem-mcp` (one test message only) |
| Tests | None in this crate. `emem-intent` and `emem-mcp` test the claim shape. |
| I/O | None. The crate holds types only. |

## Try it

The live responder tests a claim through `emem-primitives::verify`. Ask
whether the mean elevation at Mount Fuji is above 3000 m:

```sh
curl -s https://emem.dev/v1/verify -H 'Content-Type: application/json' \
  -d '{"cell":"Mount Fuji","claim":{"band":"copdem30m.elevation_mean","op":"gt","value":3000,"tslot":0}}' \
  | jq -c '{verdict, evidence}'
# {"verdict":true,"evidence":["qejpz7cgvk347ordd6vrdtmwzprqlzcnkmotsjton2x4y2jjbplq"]}
```

It needs `curl` and `jq`, and no key. `cell` takes a cell64, or a place name
that the responder geocodes. `evidence` lists every fact id at the cell for
that band and time scope. The claim was tested against these facts. The full
response also holds a signed receipt. With `"op":"lt"` the verdict is
`false`. The command ran on 7 November 2026.

## The claim shape

| Field | Type | Meaning |
|---|---|---|
| `band` | string | Band key, for example `indices.ndvi` |
| `op` | `Op` | One of the 10 operators below |
| `value` | CBOR value | The right-hand value. Send it also for `exists` and `absent`, where it has no effect. |
| `tslot` | `u64`, optional | One time slot |
| `window` | `[u64; 2]`, optional | An inclusive range of time slots |
| `agg` | string, optional | How to combine the facts in a window: `any`, `all`, `mean`, `min`, `max` |

The 10 operators, with their wire names:

| Wire name | True when |
|---|---|
| `eq`, `ne` | The fact value is equal, or not equal, to `value` |
| `lt`, `le`, `gt`, `ge` | The fact value is less than, at most, greater than, or at least `value` |
| `in`, `ni` | `value` is an array, and the fact value is in it, or not in it |
| `exists` | At least one fact holds a value for the band and time |
| `absent` | A signed absence fact exists for the band and time |

An unknown operator fails at parse time. `/v1/verify` returns HTTP 400
`invalid_argument` with the list of the 10 names.

## How `verify` uses a claim

The evaluation code is in `emem-primitives`, not here
(`src/verify.rs`, `evaluate` and `eval_one`). It works on the facts of one
cell:

1. Keep the facts with the claim band. With `tslot`, keep that slot only.
   With `window`, keep the slots in the range.
2. With `mode: "resolve"` and no fact found, fetch and sign the band first.
   This needs one target slot: `tslot`, or a `window` with equal ends.
3. Test each fact value with the operator, then combine the results with
   `agg`. The default is `any`. An unknown `agg` also acts as `any`.
4. For `mean`, `min` and `max`, fold the values to one number, then test
   that number. `mean` uses a cos(latitude) area weight.

On 7 November 2026, `{"band":"cams.no2","op":"gt","value":30,"agg":"max"}`
at Mount Fuji, with no time, gave `true` from 6 facts.

## Where the code and the spec differ

The doc comments and [docs/protocol.md](../../docs/protocol.md) section 11
say more than the code does. We checked each point in the code and on
emem.dev on 7 November 2026:

| The spec says | The code does |
|---|---|
| One of `tslot` or `window` must be set (`ClaimError::NoTime`) | `verify` accepts a claim with neither. It then uses the facts at every time slot. |
| A type mismatch returns `ClaimError::TypeMismatch` in `verify` | `verify` returns `verdict: false`. A string `value` against a numeric band gave `false`. |
| The crate has an evaluator | No type in the workspace has an `impl Evaluator`. No code outside this file uses `ClaimError`. |

`src/lib.rs` also does not use the `emem-core` or `emem-fact` dependencies in
`Cargo.toml`.

## Rules for changing this crate

- `Claim` and `Op` are a wire format. The OpenAPI `Claim` schema in
  `emem-api-rest` and the MCP tool schemas repeat the 10 operator names. A
  test in `emem-mcp` checks that the `intent` tool lists all 10.
- Add a new operator under semver. Add its case to `eval_one` in both
  `verify.rs` and `find_similar.rs` in `emem-primitives`.
- Keep the crate pure: types only, no I/O.

## What ships and what does not

| Ships | Does not ship |
|---|---|
| The `Claim` type and 10 operators | Operators for ranges or text, such as "between" or "contains" |
| Claim tests on one cell through `/v1/verify` and the `confirm` intent | A claim over many cells in one call. `verify` reads one cell. |
| Claims as a filter in `find_similar` | An `impl Evaluator` in this crate |

## Read more

- [docs/protocol.md](../../docs/protocol.md) section 11: the claim algebra
  in the spec.
- [emem-primitives](../emem-primitives/README.md): `verify` and
  `find_similar`, which test claims.
- [docs/glossary.md](../../docs/glossary.md): band, cell64 and tslot.
