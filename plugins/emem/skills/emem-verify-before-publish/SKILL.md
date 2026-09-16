---
name: emem-verify-before-publish
description: Check a draft before you send it. Resolve every emem citation in the text, confirm each one actually supports the sentence around it, and get an allow or deny with a machine-readable reason. Use before publishing any answer that cites emem, before handing a report to a user or another agent, and whenever a number in your draft came from earlier in the conversation rather than from a token you just resolved.
allowed-tools: Bash(curl:*) Bash(jq:*) Read
---

# emem-verify-before-publish

You are about to send something with citations in it. The citations were
right when you wrote them. This checks they are still right, and that
they say what the sentence around them claims.

## The one call

`emem_guard_verdict` runs emem-guard's policy pipeline over text you are
about to send. It finds every `emem:` citation, resolves each one against
the corpus, and returns **allow** or **deny** with a machine-readable
reason.

```sh
curl -sf -X POST https://emem.dev/v1/guard/verdict \
  -H 'content-type: application/json' \
  -d '{"text":"NDVI at the north field is 0.74 (emem:fact:defi.zb64a.cAzU.zfa27:<cid>)."}' \
  | jq '{action, checked, citations_found, advisory, advisory_note}'
```

`action` is the verdict. `citations_found` and `checked` say how much it
actually looked at — **read them**: a clean `action` over zero citations
found means your draft had no `emem:` citation to check, not that its
claims were verified. `advisory` and `advisory_note` carry what failed.

No key is needed and the surface is vendor-neutral.

## Three checks, and they are not the same check

**Does the signature verify?** — [`emem-verify-receipt`](../emem-verify-receipt/SKILL.md).
Offline, no network, rebuild the preimage and check ed25519. Answers
"did this responder really sign this".

**Does my number match the fact I am citing?** — `emem_echo_verify`.
Answers "am I about to misquote it", and returns the drift when you are.

**Does the fact support the sentence?** — `emem_guard_verdict`. Answers
"is this citation load-bearing for what I actually wrote", which the
other two cannot see.

A draft can pass the first two and fail the third. That is the common
case: a correctly quoted, correctly signed number attached to a claim it
does not establish.

## Where this belongs in your loop

Between computing a result and writing the sentence about it — not after
the user has read it. The cost is one call; the failure it prevents is a
confident citation that resolves to something else.

If you build a pipeline, this is the last stage before output, and its
deny is a stop, not a warning.

## Pitfalls

- **Checking the citation and not the claim.** A resolvable token beside
  a sentence it does not support is the failure this exists for.
- **Running it on the tokens instead of the text.** It needs the prose:
  the claim is in the words around the citation.
- **Treating deny as flaky.** It carries a reason. Fix the reason.
- **Skipping it because you verified the receipt.** A valid signature on
  a fact that does not support your sentence is still a wrong answer.
