---
name: emem-verify-before-publish
description: Checks a draft before it is sent. Finds every emem citation in the text, confirms each resolves and verifies, checks the number written next to each citation against the signed value, and optionally flags measurable claims that carry no citation, returning allow or deny with a machine-readable reason. Use before publishing any answer that cites emem, before handing a report to a user or another agent, and whenever a number in the draft came from earlier in the conversation rather than from a token resolved just now.
---

# emem-verify-before-publish

> **Network use.** The commands in this skill send and receive JSON (and, where a step says so, an image or a raster file) to `https://emem.dev` only, the service the plugin's MCP server connects to. Nothing they download is executed. The files under `scripts/` read local files and make no network calls.

You are about to send something with citations in it. They were right
when you wrote them. This checks they still resolve, that the numbers
beside them are the signed numbers, and, if you ask, that no measurable
claim went out uncited.

## The one call

`POST /v1/guard/verdict` (MCP `emem_guard_verdict`) runs emem-guard's
policy pipeline over the text. The field is `texts`, an **array**:

```sh
curl -sf -X POST https://emem.dev/v1/guard/verdict \
  -H 'content-type: application/json' \
  -d '{"texts":["Bengaluru air temperature is 35 degC (emem:fact:defi.zb493.zezo.zcb35:nflpddk7zsncywguwjzk5koksseqfyx4jnngkuryrnd4aykqlpfq)."],
       "claim_gating": true}' \
  | jq '{action, reason, citations_found, checked, citations}'
```

Recorded on 2026-09-28: the fact signs 28.0 degC, so that draft was
denied with
`EMEM-GUARD DENY PROV_VALUE token=emem:fact:defi.zb493.zezo.zcb35:nflpddk7... fix=correct_value leaf=-`.
The same sentence with 28.0 was allowed, with the citation reported as
`state: "verified"`.

Read `citations_found` and `checked` every time. A body with no
`texts` field is read as empty and answers `allow` with
`citations_found: 0`: a clean verdict over nothing. For a payload
another framework produced (a chat transcript, an OpenAI moderation
body, a CloudEvent), send `messages` or name its `shape`; see the tool's
description in `emem_tools`.

## What the reason codes mean

| Code | Meaning | `fix` |
|---|---|---|
| `PROV_SIG` | the cited fact's signature did not verify | `refresh_token` |
| `PROV_BYTES` | the token resolved to different content than it claims | `remove_reference` |
| `PROV_DRIFT` | the reading has moved past its band's threshold since | `refresh_token` |
| `PROV_VALUE` | the number next to the citation is not the signed value | `correct_value` |
| `CLAIM_UNGROUNDED` | a measurable claim with no citation (only with `claim_gating: true`) | `cite_observation` |

A deny that comes from an operator's own policy rather than the evidence
names `fix=contact_admin`. A citation this responder does not hold is `state: "not_held_here"`
and never a denial: it is indistinguishable from one minted elsewhere.
Resolve it where it was minted.

## What it does not check

It does not check that the cited band is about what the sentence says.
On 2026-09-28 "Bengaluru rainfall is 28.0 mm" citing the **temperature**
fact above was allowed: the value matched, and nothing compared
"rainfall" with `weather.temperature_2m`. Check the band yourself: the
resolved token's `band` must be the quantity the sentence names.

The hosted route is advisory (`advisory: true`): it blocks nothing.
To enforce, run your own node; `GET /v1/guard/selfhost` returns the
procedure, also in `crates/emem-guard/SKILL.md` in the repository.

## Three checks, not one

| Question | Tool |
|---|---|
| Did this responder really sign this? | [`emem-verify-receipt`](../emem-verify-receipt/SKILL.md), offline |
| Is my number the signed number? | `emem_echo_verify`, or `PROV_VALUE` here |
| Is the citation about what my sentence claims? | you: compare the resolved `band` and place with the sentence |

A draft can pass the first two and fail the third, and that is the
common failure: a correctly quoted, correctly signed number attached to
a claim it does not establish.

## Where this belongs in your loop

Between computing a result and sending the sentence about it. In a
pipeline this is the last stage before output, and a deny is a stop.

## Pitfalls

- **Sending `text` instead of `texts`.** It is read as nothing and
  allowed.
- **Checking the tokens instead of the prose.** The value check needs
  the words around the citation.
- **Treating deny as flaky.** The reason names the fix.
- **Skipping it because the receipt verified.** A valid signature on a
  fact that does not support the sentence is still a wrong answer.
