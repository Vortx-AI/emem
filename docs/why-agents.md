# Why an agent benefits from emem

emem is a shared memory for AI agents. The responder signs each answer,
so a reader can check it and does not have to trust the server. Recall is
[bi-temporal](./glossary.md). It keeps two questions apart: what was on
the ground on a date, and what the memory knew on that date. See
[what emem knew on a date](./only-emem.md#read-what-was-true-or-what-emem-knew-on-a-date).

This page follows one agent through one job. Each step links to the job
in the [agent guide](./agents.md#find-the-tool-for-your-job) that gives
the MCP call and all fields.

## The job: a deforestation compliance agent

A company imports cocoa. The EU Deforestation Regulation (EUDR) tells it
to prove that no one cleared forest on each plot after the cut-off date.
An AI agent does the work. Each step shows what emem gives the agent that
a plain LLM does not have.

### 1. Locate: turn a place into a stable address

The agent has a plot "near Soubré, Côte d'Ivoire". It needs an address
that it can quote later, not a fuzzy name.

```bash
curl -s -X POST https://emem.dev/v1/locate \
  -H 'content-type: application/json' \
  -d '{"q":"Soubré, Côte d'\''Ivoire"}' | jq -r .cell64
# defi.zb441.zbe5e.zd3ca
```

Read `place_label` and `alternatives` before you trust the cell. On
7 November 2026 this query resolved to a social facility in Soubré, not
to the town centre. For a real plot, send its coordinates or its polygon,
not a town name.

A `cell64` is a 64-bit address for one patch of ground, about 9.55 m
across at the equator. It does for a place what a token does for text.
From this step on, the agent quotes the cell, not the place name. Job:
[turn a place name into an address](./agents.md#job-turn-a-place-name-into-an-address).

### 2. Recall: get a signed forest fact

The agent asks for the forest baseline at that cell. If no fact exists
yet, emem fetches the upstream tile, signs the result, stores it and
returns it in the same call.

```bash
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb441.zbe5e.zd3ca","band":"jrc_gfc2020.forest_2020"}' \
  | jq '{value: .facts[0].value, fact_cid: .facts[0].fact_cid, source: .facts[0].sources[0].scheme}'
# {
#   "value": 0,
#   "fact_cid": "gkfhjoceoydlojo2dnv3dfozdodhetvvlms2zqutdrxuvmfuronq",
#   "source": "jrc.gfc2020.v4"
# }
```

The value 0 means "not forest at the 2020-12-31 cut-off". The band name
is `jrc_gfc2020.forest_2020`: an underscore between publisher and
product, a dot before the measure. The responder does not guess a
misspelled band. The dotted spelling `jrc.gfc2020.forest_2020` gets the
error `band_not_in_registry`.

Zero is a useful answer, not a miss. Under the EUDR, the 2020 baseline
decides if a plot is in scope. A cell with no forest at the cut-off
cannot lose forest after it.

The value comes from a named upstream source, the JRC tile that the fact
cites in `sources`. The `fact_cid` is a content id: a hash of the exact
signed bytes. Change one byte and the id changes.

Two fields hold the provenance, and they answer different questions:

| Field | Question | Value in this fact |
|---|---|---|
| `sources[]` | Where did the bytes come from? | the JRC V4 tile URL, `captured_at` 2024-04-25 |
| `derivation` | How did the responder compute the value? | `fn_key` plus `args`: lat, lng, `v4`, `reader=cog-pixel-floor@2` |

There is no `derivation.source`. This fact is older than the current
code, so its `fn_key` is `jrc_gfc2020_v3_pixel@1` while its args and
`sources[0].scheme` say V4. Facts signed before 9 October 2026 carry the
`v3` key. A fact that the current code signs gets
`jrc_gfc2020_v4_pixel@1`. Job:
[read signed facts at one place](./agents.md#job-read-signed-facts-at-one-place).

### 3. Cite: put the fact_cid in the report

The agent writes its due diligence note and cites the `fact_cid`. It
does not copy the number alone. The citation is the evidence itself, not
a link to evidence that can move.

> Plot at `defi.zb441.zbe5e.zd3ca` was not forest at the 2020-12-31
> cut-off (JRC GFC2020 V4), so it is out of scope for the deforestation
> test. Evidence: `gkfhjoceoydlojo2dnv3dfozdodhetvvlms2zqutdrxuvmfuronq`.

To give the fact to another agent as one token, see
[cite one fact for another agent](./agents.md#job-cite-one-fact-for-another-agent).

### 4. Verify: a reviewer checks the signature locally

A compliance reviewer at another company needs no key and no account to
read. The reviewer opens the `fact_cid` in a browser:

```
https://emem.dev/verify/gkfhjoceoydlojo2dnv3dfozdodhetvvlms2zqutdrxuvmfuronq
```

The page fetches the same bytes from emem.dev. A verifier inside the page
checks the BLAKE3 hash and the Ed25519 signature on the reviewer's
machine. The reviewer does not trust emem.dev or the importing agent. If
the signature holds, the bytes are the bytes that the responder signed.
To check with no network, save the receipt and follow
[verify a receipt offline](./agents.md#verify-a-receipt-offline). Job:
[verify a receipt](./agents.md#job-verify-a-receipt).

### For a real statement, use the EUDR endpoint

One cell and one band is the teaching version. `POST /v1/eudr_dds` takes
the plots as GeoJSON points or polygons. For each plot it reads the
forest baseline, the Hansen loss year and a TMF cross-check. It returns a
draft statement with a verdict per plot, the fact ids behind each
verdict and a `verdict_support` grade. While a plot needs review,
`statement_of_compliance_signable` is `false` and
`statement_of_compliance_held_because` gives the reasons. [EUDR DDS and visual
evidence](./eudr.md) shows the full call.

## Where emem sits next to what you already run

emem does not replace your vector store. It is the layer under it for
facts about the physical world.

| | emem | Vector DB | RAG pipeline | Tile or catalog API |
|---|---|---|---|---|
| Shared across agents and organizations | one memory, any reader | within one deployment | within one app | reads only |
| Verifiable offline | Ed25519 and content id, no account | no | no | no |
| Content-addressed | id = hash of the bytes | ids are arbitrary | no | paths are positional |
| Empty answer | signed absence with a reason | empty result set | empty retrieval | `404` |
| Source disagreement | kept and scored | duplicates or last write | up to the app | not modeled |
| Give one fact to another agent | one short token | share credentials or embed again | send the text again | send a URL and hope it stays |
| Authentication to read | none | usually an API key | app-specific | usually an account |

## Signatures make a planted record traceable

[MINJA](https://arxiv.org/abs/2503.03704) (a NeurIPS 2025 poster) is a paper on
memory injection attacks. Through queries alone, it planted false records
in the memory of LLM agents. The average injection success rate was
98.2%. On average, the planted records then changed the agent's answer
in 76.8% of the tests.

A signature does not make a writer honest. It ties each record to a key.
So in emem a planted record has a known author. Readers can score it
against other writers, and a later record can supersede it.

The two kinds of record have different write rules:

- Facts: only this responder, an enrolled device or an operator-listed
  key can write an address (cell, band, time slot). The responder refuses
  all other signed writes to an address.
- Notes: any signed key can write a note, so a planted note can exist.
  The responder serves each note wrapped as data, not as instructions
  ([before you read a note](./agents.md#before-you-read-a-note)).

The reader decides which writers to trust.

## Next

- [Quickstart](./quickstart.md): the same flow with curl, Python,
  TypeScript, Go and Rust.
- [What only emem does](./only-emem.md): what a plain vector store does
  not give you.
- [Connect and evolve](./connect-and-evolve.md): how facts link to each
  other and how the corpus improves over time.
