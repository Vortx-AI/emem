# Your first verified memory, in ten minutes

By the end of this page you will have read a signed fact about a real
place, verified it without trusting anyone, composed the one-line token
that stands in for it, and handed that token to a second process that
resolved it back to the identical bytes and checked the signature
itself. Everything runs against the public node with no key, no
account, and no install beyond curl and jq.

The worked example uses Cairo. Any place name works; swap it and every
step below still holds, which is rather the point.

## 1. Read a signed fact (one call)

```bash
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"place":"Cairo","bands":["copdem30m.elevation_mean"]}'
```

Three things in the response matter here:

- `facts[0].value`: the elevation, in metres, at one 10-metre cell of
  Cairo.
- `facts[0].fact_cid`: the record's content id, the blake3 hash of its
  own canonical bytes. Change one byte and the id changes.
- `receipt`: an ed25519 signature over what was answered, by whom,
  when. Save the cell too; it is in `receipt.cells[0]`.

## 2. Verify it, trusting nobody

```bash
curl -s -X POST https://emem.dev/v1/recall -H 'content-type: application/json' \
  -d '{"place":"Cairo","bands":["copdem30m.elevation_mean"]}' \
  | jq '{receipt: .receipt}' \
  | curl -s -X POST https://emem.dev/v1/verify_receipt \
      -H 'content-type: application/json' --data-binary @- \
  | jq '{signature_valid, merkle_proof_valid}'
```

`"signature_valid": true`. The check is arithmetic against the
responder's published key, not a callback: the same verification runs
fully offline in any blake3 plus ed25519 implementation, and
[`/verify`](https://emem.dev/verify) runs it in your browser if you
prefer to paste the `fact_cid` there.

## 3. Compose the token, by hand

There is no minting step to learn. The citation IS the address plus the
content id, joined by colons:

```text
emem:fact:<cell64>:<fact_cid>
```

Take `receipt.cells[0]` and `facts[0].fact_cid` from step 1 and write
the line yourself. For the Cairo elevation fact it looks like:

```text
emem:fact:defi.zb555.ze8e5.xawO:zxxbyowj2vlhyvgcj55brc47aoubvnibo7ydck4q2vn32uqcp3oa
```

That line is the whole memory. About 84 characters, safe in a commit
message, a report, a note, or another agent's context window. The
payload can be dropped; the line brings it back.

## 4. Hand it to someone who does not trust you

Open a second terminal, a different machine, or send the line to a
colleague. The receiver resolves it:

```bash
curl -s -X POST https://emem.dev/v1/memory_token/resolve \
  -H 'content-type: application/json' \
  -d '{"token":"emem:fact:defi.zb555.ze8e5.xawO:zxxbyowj2vlhyvgcj55brc47aoubvnibo7ydck4q2vn32uqcp3oa"}'
```

The response carries the byte-identical signed fact and its receipt,
and the receiver runs step 2's verification on it themselves. Nothing
about that check involves trusting you, your paraphrase of the value,
or the channel the line travelled through. Two parties now hold one
fact, not two descriptions of it.

If both steps returned `true` for you, that is the entire protocol
working end to end, and the star button on
[the repo](https://github.com/Vortx-AI/emem) is how other builders
find it.

## 5. Where this goes next

- An agent does this loop with four verbs over MCP: connect any MCP
  client to `https://emem.dev/mcp` and the
  [agent guide](../agents.md) walks the same path tool by tool.
- Time travel: every read accepts `as_of_tslot` (what was on the
  ground) and `as_of_signed_at` (what the memory knew), so "what did
  we know when we decided" stays answerable years later.
- When a readout changes between visits, `POST /v1/change_attribution`
  returns the evidence ledger for why: world, instrument, pixels,
  model, or noise.
- The whole ladder, one idea per rung, is at the top of the
  [README](https://github.com/Vortx-AI/emem#the-ladder).
