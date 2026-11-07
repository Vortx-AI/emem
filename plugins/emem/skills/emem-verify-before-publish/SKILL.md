---
name: emem-verify-before-publish
description: Checks a draft before you send it. Finds every emem citation in the text and confirms that each one resolves and verifies. Checks the number next to each citation against the signed value. It can also flag measurable claims that have no citation. Returns allow or deny with a machine-readable reason. Use it before you publish an answer that cites emem. Use it before you give a report to a user or another agent. Use it when a number in the draft did not come from a token that you resolved just now.
---

# emem-verify-before-publish

| | |
|---|---|
| Use it when | A draft that cites emem is ready to send to a user or to another agent. |
| You need | The draft text, with its `emem:fact:` tokens in the prose. No key. |
| You get | `allow` or `deny`, a reason code, a `fix`, and one state for each citation. |
| Calls | `emem_guard_verdict` (MCP). Or `POST /v1/guard/verdict`. |

> **Network use.** The commands in this skill send JSON to `https://emem.dev` and receive JSON from it. `https://emem.dev` is the service that the MCP server of the plugin connects to. The commands do not execute what they download. This skill ships no scripts. The commands read no environment variable, key or credential. They read only local files that an earlier step saved from `https://emem.dev`.

The citations in a draft were correct when you wrote them. This call
checks that they still resolve. It checks that the number next to each
citation is the signed number. If you ask, it also checks that each
measurable claim has a citation.

## Step 1: send the draft

Put the draft in `texts`. This field is an array of strings. Call
`emem_guard_verdict` with this body:

```json
{"texts": ["Bengaluru air temperature is 35 degC (emem:fact:defi.zb493.zezo.zcb35:nflpddk7zsncywguwjzk5koksseqfyx4jnngkuryrnd4aykqlpfq)."],
 "claim_gating": true}
```

Without MCP, send the same body to `POST https://emem.dev/v1/guard/verdict`.
Read `action`, `reason`, `citations_found`, `checked`, `advisory`, and
the `state` of each entry of `citations[]` in the answer.

For a chat transcript, send `messages` (a list of `role` and `content`
objects) in place of `texts`. For a payload from another framework,
such as an OpenAI moderation body or a CloudEvent, name its `shape`.
The tool description in `emem_tools` lists the shapes.

## Step 2: read the verdict

| Field | What to do with it |
|---|---|
| `action` | `allow` or `deny`. A deny means: do not send the draft yet. |
| `reason` | One line: `EMEM-GUARD DENY <code> token=<token> fix=<fix> leaf=<leaf>`. `null` on allow. |
| `citations_found` | The number of citations that the guard found. Read it every time. |
| `checked` | The number of citations that it checked. |
| `citations[].state` | `verified`, or `not_held_here` for a token that this responder does not hold. |
| `advisory` | `true` on emem.dev. The hosted route blocks nothing. |

The guard reads a body without `texts` or `messages` as empty. It returns
`allow` with `citations_found: 0`. That is a clean verdict over nothing.
Thus, if `citations_found` is 0 and your draft has citations, the
request is wrong.

### Recorded runs

The cited fact signs `28.0` degC. On 6 November 2026 at 23:59 UTC:

| Draft says | `action` | `reason` |
|---|---|---|
| `35 degC` | `deny` | `EMEM-GUARD DENY PROV_VALUE token=emem:fact:defi.zb493.zezo.zcb35:nflpddk7... fix=correct_value leaf=-` |
| `28.0 degC` | `allow` | `null`. The citation state was `verified`. |
| `35 degC`, sent as `"text"` and not `"texts"` | `allow` | `null`, with `citations_found: 0` and `checked: 0`. |

A run on 28 September 2026 gave the same first two results. On 7
November 2026 at 02:10 UTC, all three rows gave the same results again.

## Step 3: act on a deny

Branch on the `fix` value. Do not branch on the prose.

| Code | Cause | `fix` |
|---|---|---|
| `PROV_SIG` | The signature of the cited fact did not verify. | `refresh_token` |
| `PROV_BYTES` | The token resolved to other content than it claims. | `remove_reference` |
| `PROV_VALUE` | The number next to the citation is not the signed value. | `correct_value` |
| `PROV_DRIFT` | The reading moved past the threshold of its band after you cited it. | `refresh_token` |
| `CLAIM_UNGROUNDED` | A measurable claim has no citation. Only with `claim_gating: true`. | `cite_observation` |
| `GEO_ZONE` | A cited cell is in a zone that the operator restricts. | `contact_admin` |
| `POLICY_MODULE` | A policy module that the operator loaded denied the text. | `redact_and_retry` |

`GEO_ZONE` and `POLICY_MODULE` come from the configuration of the
operator, not from the evidence. The codes and fixes are in
`crates/emem-guard/src/policy.rs`.

For `correct_value`, write the signed value in the sentence. Do not
remove the citation. The citation is correct, and the prose is wrong.

A citation with state `not_held_here` never causes a deny. The
responder cannot tell it from a token that another node minted. Resolve
it on the node that minted it.

## What the guard does not check

The guard does not check that the cited band is the quantity that the
sentence names. On 6 November 2026, this draft got `allow`:
"Bengaluru rainfall is 28.0 mm", with the temperature token from step 1.
The value agreed, and nothing compared "rainfall" with
`weather.temperature_2m`.

Thus you must check the band yourself. Resolve the token with
`emem_memory_token_resolve`. Its `band` must be the quantity that the
sentence names, and its cell must be the place.

Claim gating finds fewer claims than a reader does. A sentence is a
claim only when it has an anchor and a number. The number needs a
unit, or it must be a named index value such as NDVI 0.45. The anchor
is an ISO date, coordinates, a year, or a word such as "in", "at" or
"near" before a capitalised word (`crates/emem-guard/src/claim.rs`).
Results on 7 November 2026, with `claim_gating: true` and no citation:

| Draft says | `action` |
|---|---|
| `The temperature in Bengaluru is 35 degC.` | `deny`, `CLAIM_UNGROUNDED`, with `source_band` `weather.temperature_2m` |
| `Elevation in Leh is 3500 m.` | `deny`, `CLAIM_UNGROUNDED`, with `source_band` `copdem30m.elevation_mean` |
| `NDVI in Bengaluru is 0.45.` | `deny`, `CLAIM_UNGROUNDED`, with the claim `NDVI 0.45` and `source_band` `indices.ndvi` |
| `Bengaluru air temperature is 35 degC.` | `allow`. The place is not after "in", so the sentence has no anchor. |

Also, one citation that resolves turns off claim gating for the whole
request (`crates/emem-guard/src/policy.rs`). A draft that cites one fact
and then makes ten claims without a citation gets `allow`. Thus an
`allow` does not prove that each claim has a citation.

The hosted route is advisory. To enforce the verdict, run your own
node. `GET https://emem.dev/v1/guard/selfhost` returns the procedure.
The same procedure is in `crates/emem-guard/SKILL.md` in the repository.

## Three different checks

| Question | Tool |
|---|---|
| Did this responder sign this fact? | [`emem-verify-receipt`](../emem-verify-receipt/SKILL.md), offline |
| Is my number the signed number? | `emem_echo_verify`, or `PROV_VALUE` from this call |
| Does the citation support my sentence? | You: compare the resolved `band` and place with the sentence |

A draft can pass the first two checks and fail the third. This is the
most frequent failure: a correct, signed number next to a claim that it
does not support.

## Where this step goes

Run it after you compute a result and before you send the sentence
about it. In a pipeline, make it the last stage before output. Treat a
deny as a stop.

## Pitfalls

| Mistake | Result |
|---|---|
| You send `text` and not `texts`. | The guard reads nothing and returns `allow`. |
| You send only the tokens, without the prose. | The value check has no words to compare. |
| You retry a deny without a change. | You get the same deny. The `fix` field says what to change. |
| You read an `allow` with `claim_gating` as "every claim has a citation". | One resolved citation turns the gate off. Check each sentence yourself. |
| You skip the guard because the receipt verified. | A valid signature on a fact that does not support the sentence still gives a wrong answer. |
