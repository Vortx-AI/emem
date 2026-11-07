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

All values on this page are what emem.dev returned on 7 November 2026. A newer reading can give a different
`fact_cid`. The shape of each answer stays the same.

## 1. Read a signed fact

```bash
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"place":"Cairo","bands":["copdem30m.elevation_mean"]}' \
  | jq '.facts[0] | {value, cell, fact_cid, memory_token}'
# {
#   "value": 187.33578491210938,
#   "cell": "defi.zb555.ze8e5.xawO",
#   "fact_cid": "ivtsljcmbnugccv72zjyuhyvtfrfvz4oqs43kjk5tibb4qw2hnlq",
#   "memory_token": "emem:fact:defi.zb555.ze8e5.xawO:ivtsljcmbnugccv72zjyuhyvtfrfvz4oqs43kjk5tibb4qw2hnlq"
# }
```

Remove the `jq` line to see the full response. These fields in the response are important:

| Field | What it is | Value on 7 November 2026 |
|---|---|---|
| `facts[0].value` | The elevation in metres at the centre of the cell. It is the value of the Copernicus GLO-30 pixel (30 m) that contains that point. | `187.33578491210938` |
| `facts[0].cell` | The cell64 address of that cell. `receipt.cells[0]` holds the same value. | `defi.zb555.ze8e5.xawO` |
| `facts[0].fact_cid` | The BLAKE3 hash of the fact's canonical bytes. If one byte changes, the id changes. | `ivtsljcm...hnlq` |
| `facts[0].memory_token` | The ready-made token for this fact. Step 3 makes it by hand. | `emem:fact:defi.zb555...` |
| `receipt` | An Ed25519 signature over the answer: the question, the responder, and the time | an object with 16 fields |

The place name resolves to one cell. To see which cell and why, send the name to `/v1/locate`:

```bash
curl -s -X POST https://emem.dev/v1/locate \
  -H 'content-type: application/json' \
  -d '{"place":"Cairo"}' | jq -c '{cell64, label: .selected.label, sure: .selected.is_high_confidence}'
# {"cell64":"defi.zb555.ze8e5.xawO","label":"Cairo, Egypt","sure":true}
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
  | jq -c '{signature_valid, merkle_proof_valid}'
# {"signature_valid":true,"merkle_proof_valid":true}
```

This call asks the responder to do the check. It is not an independent check, because the responder checks its own
signature. Use it to find problems. For an independent check, do the BLAKE3 and Ed25519 arithmetic yourself against
the responder's published key. Any BLAKE3 and Ed25519 library can do it offline. The
[`/verify`](https://emem.dev/verify) page does it in your browser: paste the receipt, or the token from step 3.
[Verify the receipt](../quickstart.md#verify-the-receipt) in the Quickstart shows the offline check in Python. It
needs `pip install "ememdev[signing]"`.

## 3. Write the token by hand

You do not need a call to make a token. The token is the cell and the fact id, joined by colons:

```text
emem:fact:<cell64>:<fact_cid>
```

Take `facts[0].cell` and `facts[0].fact_cid` from step 1 and write the line. Compare it with `facts[0].memory_token`.
The two must be equal. This command does the same comparison:

```bash
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"place":"Cairo","bands":["copdem30m.elevation_mean"]}' \
  | jq '.facts[0] | "emem:fact:\(.cell):\(.fact_cid)" == .memory_token'
# true
```

Steps 4 and 5 use one Cairo elevation fact that emem signed on 16 July 2026. This is its token:

```text
emem:fact:defi.zb555.ze8e5.xawO:zxxbyowj2vlhyvgcj55brc47aoubvnibo7ydck4q2vn32uqcp3oa
```

Your token from step 1 names a newer fact of the same cell. On 7 November 2026, that fact had
`signed_at` 2026-09-28T09:06:51Z and the same value. The older token still resolves, because emem never rewrites a
fact.

This token is 84 characters. The length changes a little with the cell. You can put it in a commit message, a report,
a note, or the context of another agent. You can drop the full fact, because the token gets it back.

## 4. Give the token to a second party

Open a second terminal or a different machine, or send the token to a colleague. The receiver resolves it:

```bash
curl -s -X POST https://emem.dev/v1/memory_token/resolve \
  -H 'content-type: application/json' \
  -d '{"token":"emem:fact:defi.zb555.ze8e5.xawO:zxxbyowj2vlhyvgcj55brc47aoubvnibo7ydck4q2vn32uqcp3oa"}' \
  | jq -c '{band, value, unit, signed_at: .fact.signed_at, cell_matches}'
# {"band":"copdem30m.elevation_mean","value":187.33578491210938,"unit":"m","signed_at":"2026-07-16T10:47:40Z","cell_matches":true}
```

The response holds the signed fact in `fact`. It also copies `value` and `unit` to the top level. `cell_matches`
is `true` when the cell in the token is the cell of the signed fact. If the token names a different valid cell, emem
refuses the token with HTTP 409 and names the correct cell.

If emem cannot read the cell in the token, it finds the fact by its `fact_cid` only. Then the response is HTTP 200
with `cell_matches: false` and `degraded: true`, and no cell check occurred. On 7 November 2026, the token above
gave this result when its cell ended in `xawP`, not `xawO`. Treat `degraded: true` as a failure, and cite
`canonical_token` from the response.

The response has its own `receipt`. The receiver checks that receipt with the step 2 check:

```bash
curl -s -X POST https://emem.dev/v1/memory_token/resolve \
  -H 'content-type: application/json' \
  -d '{"token":"emem:fact:defi.zb555.ze8e5.xawO:zxxbyowj2vlhyvgcj55brc47aoubvnibo7ydck4q2vn32uqcp3oa"}' \
  | jq '{receipt: .receipt}' \
  | curl -s -X POST https://emem.dev/v1/verify_receipt \
      -H 'content-type: application/json' --data-binary @- \
  | jq -c '{signature_valid, merkle_proof_valid}'
# {"signature_valid":true,"merkle_proof_valid":true}
```

The receiver does not trust you, your copy of the value, or the channel that carried the token. This check still
trusts the responder, as step 2 says. To remove that trust too, check the receipt offline. Both parties now hold one
fact, not two descriptions of it.

If the receipt checks in step 2 and in this step both return `true`, the full loop works.

## 5. Next steps

- Agents do the same loop over MCP with `emem_recall`, `emem_verify_receipt`, `emem_memory_token` and
  `emem_memory_token_resolve`. Connect an MCP client to `https://emem.dev/mcp`. The [agent guide](../agents.md)
  shows each tool.
- `/v1/recall` accepts `as_of_tslot` and `as_of_signed_at`. The first selects the time on the ground. The second
  selects what emem had signed by a given time. Use them to see what emem knew when you made a decision. This
  call asks for what emem had signed by 1 August 2026. It returns the 16 July fact that steps 4 and 5 use:

  ```bash
  curl -s -X POST https://emem.dev/v1/recall \
    -H 'content-type: application/json' \
    -d '{"place":"Cairo","bands":["copdem30m.elevation_mean"],"as_of_signed_at":"2026-08-01T00:00:00Z"}' \
    | jq -c '[.facts[] | {fact_cid, signed_at}]'
  # [{"fact_cid":"zxxbyowj2vlhyvgcj55brc47aoubvnibo7ydck4q2vn32uqcp3oa","signed_at":"2026-07-16T10:47:40Z"}]
  ```

- When a reading changes between visits, `POST /v1/change_attribution {"cell": ...}` returns evidence for each term
  with the fact ids. The keys of `terms` are `env` (environment), `sensor`, `geo` (geometry), `encoder` and `noise`.
  It does not split the change into amounts per term. `split` is `null`, and `attribution_note` says why. The
  numeric split is roadmap.
- To send several facts as one token, use `POST /v1/memory_bundle`. It returns an `emem:bundle:` token. See the
  [Quickstart](../quickstart.md).
