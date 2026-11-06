# Your first verified memory, in ten minutes

This tutorial reads one signed fact, checks its receipt, writes the token for it by hand, and resolves that token in a
second process. All steps use the public node at emem.dev.

## What you need

- `curl` and `jq`.
- No key, no account and no install. Every call on this page is a read.

## What you get

- The elevation of one cell in Cairo, as a signed fact.
- A receipt check that returns `signature_valid: true`.
- A one-line token that another person or agent resolves to the same signed bytes.

The example uses Cairo. You can use a different place name. The steps stay the same.

All values on this page are what emem.dev returned on 6 November 2026. A newer reading can give a different
`fact_cid`. The shape of each answer stays the same.

## 1. Read a signed fact

```bash
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"place":"Cairo","bands":["copdem30m.elevation_mean"]}'
```

These fields in the response are important:

| Field | What it is | Value on 6 November 2026 |
|---|---|---|
| `facts[0].value` | The elevation in metres at one cell of about 10 m, from the Copernicus 30 m elevation model | `187.33578491210938` |
| `facts[0].cell` | The cell64 address of that cell. `receipt.cells[0]` holds the same value. | `defi.zb555.ze8e5.xawO` |
| `facts[0].fact_cid` | The BLAKE3 hash of the fact's canonical bytes. If one byte changes, the id changes. | `ivtsljcm...hnlq` |
| `facts[0].memory_token` | The ready-made token for this fact. Step 3 makes it by hand. | `emem:fact:defi.zb555...` |
| `receipt` | An Ed25519 signature over the answer: the question, the responder, and the time | an object with 16 fields |

The place name resolves to one cell. To see which cell and why, send the name to `/v1/locate`:

```bash
curl -s -X POST https://emem.dev/v1/locate \
  -H 'content-type: application/json' \
  -d '{"place":"Cairo"}' | jq '{cell64, label: .selected.label, sure: .selected.is_high_confidence}'
# {"cell64": "defi.zb555.ze8e5.xawO", "label": "Cairo, Egypt", "sure": true}
```

When a name has more than one match, `is_high_confidence` is `false`. Then `decision` shows the chosen place and the
runner-up. Give a more specific name, or send `lat` and `lng`.

## 2. Check the receipt

```bash
curl -s -X POST https://emem.dev/v1/recall -H 'content-type: application/json' \
  -d '{"place":"Cairo","bands":["copdem30m.elevation_mean"]}' \
  | jq '{receipt: .receipt}' \
  | curl -s -X POST https://emem.dev/v1/verify_receipt \
      -H 'content-type: application/json' --data-binary @- \
  | jq '{signature_valid, merkle_proof_valid}'
# {"signature_valid": true, "merkle_proof_valid": true}
```

This call asks the server to do the check. You do not have to trust the server for it. The check is BLAKE3 and
Ed25519 arithmetic against the responder's published key. Any BLAKE3 and Ed25519 library can do the same check
offline. The [`/verify`](https://emem.dev/verify) page does it in your browser: paste the receipt, or the token from
step 3. The [Quickstart](../quickstart.md) shows the offline check in Python.

## 3. Write the token by hand

You do not need a call to make a token. The token is the cell and the fact id, joined by colons:

```text
emem:fact:<cell64>:<fact_cid>
```

Take `facts[0].cell` and `facts[0].fact_cid` from step 1 and write the line. Compare it with `facts[0].memory_token`.
The two must be equal.

This tutorial uses one Cairo elevation fact that emem signed on 16 July 2026:

```text
emem:fact:defi.zb555.ze8e5.xawO:zxxbyowj2vlhyvgcj55brc47aoubvnibo7ydck4q2vn32uqcp3oa
```

Your token from step 1 names a newer fact of the same cell. On 6 November 2026, that fact had `signed_at` 2026-09-28T09:06:51Z
and the same value. The older token still resolves, because emem never rewrites a fact.

The token is 84 characters. You can put it in a commit message, a report, a note, or the context of another agent.
You can drop the full fact, because the token gets it back.

## 4. Give the token to a second party

Open a second terminal or a different machine, or send the token to a colleague. The receiver resolves it:

```bash
curl -s -X POST https://emem.dev/v1/memory_token/resolve \
  -H 'content-type: application/json' \
  -d '{"token":"emem:fact:defi.zb555.ze8e5.xawO:zxxbyowj2vlhyvgcj55brc47aoubvnibo7ydck4q2vn32uqcp3oa"}' \
  | jq '{band, value, unit, signed_at: .fact.signed_at}'
# {"band": "copdem30m.elevation_mean", "value": 187.33578491210938, "unit": "m", "signed_at": "2026-07-16T10:47:40Z"}
```

The response holds the signed fact in `fact`. It also copies `value` and `unit` to the top level. It has its own
`receipt`. The receiver checks that receipt with the step 2 check:

```bash
curl -s -X POST https://emem.dev/v1/memory_token/resolve \
  -H 'content-type: application/json' \
  -d '{"token":"emem:fact:defi.zb555.ze8e5.xawO:zxxbyowj2vlhyvgcj55brc47aoubvnibo7ydck4q2vn32uqcp3oa"}' \
  | jq '{receipt: .receipt}' \
  | curl -s -X POST https://emem.dev/v1/verify_receipt \
      -H 'content-type: application/json' --data-binary @- \
  | jq '{signature_valid, merkle_proof_valid}'
# {"signature_valid": true, "merkle_proof_valid": true}
```

The receiver does not trust you, your copy of the value, or the channel that carried the token. Both parties now hold
one fact, not two descriptions of it.

If both checks return `true`, the full loop works.

## 5. Next steps

- Agents do the same loop over MCP with `emem_recall`, `emem_verify_receipt`, `emem_memory_token` and
  `emem_memory_token_resolve`. Connect an MCP client to `https://emem.dev/mcp`. The [agent guide](../agents.md)
  shows each tool.
- `/v1/recall` accepts `as_of_tslot` and `as_of_signed_at`. The first selects the time on the ground. The second
  selects what emem had signed by a given time. Use them to see what emem knew when you made a decision.
- When a reading changes between visits, `POST /v1/change_attribution {"cell": ...}` returns evidence for each term
  (environment, sensor, geometry, encoder, noise) with the fact ids. It does not split the change into amounts per
  term. `split` is `null`, and `attribution_note` says why. The numeric split is roadmap.
- To send several facts as one token, use `POST /v1/memory_bundle`. It returns an `emem:bundle:` token. See the
  [Quickstart](../quickstart.md).
