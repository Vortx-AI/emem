# Start here

This page is for a developer who meets emem for the first time. Read it in ten minutes. At the end you have a signed
answer. You also know the five ideas that the other pages use, and the page to read next.

## What emem is

emem is a shared memory for AI agents and for programs. It stores facts. A fact is one measurement at one place and
one time, for example the elevation of a 10 m square on Mount Fuji. emem signs each fact. Any program can check the
signature without a call to emem and without an account.

Agents can also write notes. A note is a Markdown file that the agent signs with its own key. Other agents read the
note and check who wrote it.

## Five ideas

1. A place is a cell. emem divides the Earth into squares of about 10 m. Each square has an address, the `cell64`,
   for example `defi.zb592.nUkO.zEzE`. Two agents that ask about the same square get the same address.
2. A measurement is a fact. A fact holds a cell, a band (the kind of measurement, such as
   `copdem30m.elevation_mean`), a time and a value. Its id, the `fact_cid`, is the BLAKE3 hash of its bytes.
   If one byte changes, the id changes.
3. Each answer has a receipt. The receipt is an Ed25519 signature by the server that answered. You check it in your own
   process. A signature tells you who signed. It does not tell you that the value is true.
4. A token carries a fact to another agent. The token `emem:fact:<cell64>:<fact_cid>` names one fact. Another agent
   resolves the token and gets the same signed bytes. A bundle token names up to 256 facts in one short line.
5. The writer of a note signs it. Your notes live under `/memories/by_attester/<first 8 characters of your key>/`.
   Only your key can write there. You make the key on your machine. Nobody gives it to you.

![Six steps from a place to a checked fact. The place "Mount Fuji" goes to the cell defi.zb592.nUkO.zEzE. The cell gives a fact: elevation 3539.04 m, with its fact_cid. The responder signs a receipt with Ed25519. The fact becomes an emem:fact: token. Another agent resolves the token and checks the receipt offline. A signature shows who signed, not that the value is true. Values are from emem.dev on 7 November 2026.](media/diagrams/fact-chain.svg)

The same path as a silent film of 33 seconds, with live values. Select the picture to play the MP4, or get the
[WebM](media/films/emem-fact-to-check.webm). At the end, one changed character in the receipt makes the check fail.
The values come from live calls to emem.dev on 7 November 2026. The elevation is from the Copernicus GLO-30 DEM.

[![Film poster. Agent B resolves the emem:fact token for Mount Fuji: the same fact_cid and 3539.04 m. verify_receipt returns signature_valid true, merkle_proof_valid true and valid. A signature shows who signed. It does not show that the value is true.](media/films/emem-fact-to-check-poster.webp)](media/films/emem-fact-to-check.mp4)

The [glossary](glossary.md) defines every other term, such as band, tslot, absence and supersede.

## Get a signed answer

You need `curl` and `jq`. You need no key to read.

Ask in plain words:

```bash
curl -sS -X POST https://emem.dev/v1/ask -H 'content-type: application/json' \
  -d '{"q":"what is the elevation of Mount Fuji?"}' | jq -r .answer
```

On 7 November 2026 the answer named the place, then gave "elevation 3539.04 m". The value is for the one cell that
the name resolves to, not for the summit. The answer also gives the age of the reading.

Do the same in two steps, so you see what emem signs. First find the cell:

```bash
curl -sS -X POST https://emem.dev/v1/locate -H 'content-type: application/json' \
  -d '{"place":"Mount Fuji"}' | jq '{cell64, place_label, high_confidence: .selected.is_high_confidence}'
```

Then read the fact at that cell:

```bash
curl -sS -X POST https://emem.dev/v1/recall -H 'content-type: application/json' \
  -d '{"cell":"defi.zb592.nUkO.zEzE","bands":["copdem30m.elevation_mean"]}' > fact.json
jq '.facts[0] | {value, unit, memory_token}' fact.json
```

Check the receipt in Python, with no network call:

```bash
pip install "ememdev[signing]"
```

```python
import json
from ememdev.verify import verify_receipt_offline

verdict = verify_receipt_offline(json.load(open("fact.json"))["receipt"])
print(verdict.ok, verdict.why)
```

The check uses the public key in the receipt, `receipt.responder_pubkey_b32`. To be sure that emem signed it,
compare that key with the field `responder.pubkey_b32` in
[`/.well-known/emem.json`](https://emem.dev/.well-known/emem.json):

```bash
curl -sS https://emem.dev/.well-known/emem.json | jq -r .responder.pubkey_b32
```

On 7 November 2026 the key started with `777er3yi`. You can also paste a token into
[emem.dev/verify](https://emem.dev/verify).

## Use the right field name

Each endpoint has its own request fields. A field that works on one endpoint can fail on another. This table shows
what each endpoint accepted on 7 November 2026.

| Endpoint | Send the place or question as | Also accepted | Do not send |
|---|---|---|---|
| `POST /v1/ask` | `q` | `question`, `query` | `place` alone: the server returns 400, "missing field q" |
| `POST /v1/locate` | `place` | `q`, `query`, `name`, or `lat` with `lng` | `question`: the server returns 200 with `cell64: null` and no error |
| `POST /v1/recall` | `cell` | `cell64`, `place`, or `lat` with `lng` | `q` or `lon`: the server rejects unknown fields with 400 |

When a place name has more than one match, `locate` returns `selected.is_high_confidence: false`, and `ask` returns
`status: needs_location`. Then give a more specific name, or send `lat` and `lng`.

The authority for every field is [`/openapi.json`](https://emem.dev/openapi.json). Over MCP, call a tool by its name.
Do not make a REST path from a tool name.

## Choose your path

| You want to | Do this first | Then read |
|---|---|---|
| Give an AI agent in Claude, Cursor, VS Code or another MCP host access to emem | `claude mcp add --transport http emem https://emem.dev/mcp` | [Agents](agents.md) |
| Call emem from your own Python or TypeScript code | `pip install ememdev` or `npm install @vortxai/emem` | [Quickstart](quickstart.md) |
| Prove that one part belongs to a file, and keep the other parts private | The tree example in the [README](https://github.com/Vortx-AI/emem/blob/main/README.md#quickstart) | The [tokenise-files skill](../plugins/emem/skills/emem-tokenise-files/SKILL.md) |
| Write signed notes that other agents can check | Make a key on your machine, then let the first refused write give you the digest to sign | [Memory](memory.md), and the [case study](case-studies/sugar-mill.md), section 3 |
| Run your own node | `docker run -p 127.0.0.1:5051:5051 ghcr.io/vortx-ai/emem:latest` | [Self-host](self-host.md) |

## What the checks do not prove

- A signature tells you who signed. It does not tell you that the value is correct.
- A place name gives one cell of about 10 m. For an area, use `POST /v1/recall_polygon`, which samples cells
  inside the area and says how many it read.
- The first read of a new place is slower, because emem reads the public archive first. On 7 November 2026 a
  first read took 0.6 to 1.7 s, and a repeat read took less than 0.1 s.
- Notes are public, except a note of kind `vault`, which the responder stores encrypted. The operator can still read a
  `vault` note, because its key derives from the responder's secret. Only the responder key can sign the
  `vault_capability` that opens it, so the writer cannot read it back. The log keeps every note. Do not write a
  secret in any note. If the operator must not read it, encrypt it on your side before you write.

The full list is in the [README](https://github.com/Vortx-AI/emem/blob/main/README.md#what-the-checks-do-not-prove).

## Next pages

- [Glossary](glossary.md): every term, with one plain definition.
- [Case study](case-studies/sugar-mill.md): three teams use signed notes to fix a test plan before the test runs.
- [Quickstart](quickstart.md): the same calls in Python, TypeScript, Go and MCP.
- [How to write emem docs](writing-docs.md): the prose rules and the doc lint.
