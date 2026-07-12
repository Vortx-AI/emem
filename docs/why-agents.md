# Why an agent benefits from emem

emem is a shared memory for AI agents that connects facts and gets
better over time. Every answer is signed, so anyone can check it
without trusting the server.

That sentence is abstract. Here is one agent doing one job, end to end.

## The job: a deforestation-compliance agent

A company imports cocoa. Under the EU Deforestation Regulation, they
must prove each plot was not deforested after the cutoff date. An AI
agent does the legwork. Watch what emem gives it that a plain LLM
cannot.

### 1. Locate: turn a place into a stable handle

The agent has a plot described as "near Soubré, Côte d'Ivoire". It needs
a handle it can quote later, not a fuzzy name.

```bash
curl -s -X POST https://emem.dev/v1/locate \
  -H 'content-type: application/json' \
  -d '{"q":"Soubré, Côte d'\''Ivoire"}' | jq -r .cell64
# "defi.q7k2x.m4ph.aa913"
```

A `cell64` addresses a place the way a token addresses text in an LLM:
a 64-bit identifier for one ~9.55 m square of ground. From here on the
agent quotes the cell, never the prose.

### 2. Recall: get a signed forest fact

The agent asks for the forest baseline at that cell. If no fact exists
yet, emem fetches the upstream tile, signs the result, stores it, and
returns it in the same call.

```bash
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.q7k2x.m4ph.aa913","band":"jrc.gfc2020.forest_2020"}' \
  | jq '{value: .facts[0].value, fact_cid: .facts[0].fact_cid, source: .facts[0].derivation.source}'
# {
#   "value": 1,                       # forest in 2020 under JRC GFC2020 V3
#   "fact_cid": "qi3jo4sqcg…l2hgjtwm", # 52-char content id: names the bytes
#   "source": "jrc.gfc2020"
# }
```

The number is not a guess. It comes from a named upstream source, and it
arrives with a **fact_cid**, a content id that is the fingerprint of
the exact bytes. Change one byte and the id changes.

### 3. Cite: drop the fact_cid into the report

The agent writes its due-diligence note and cites the `fact_cid`
instead of re-stating the number. The citation is the evidence, not a
pointer to evidence that might move.

> Plot at `defi.q7k2x.m4ph.aa913` was forest in 2020 (JRC GFC2020 V3).
> Evidence: `qi3jo4sqcg…l2hgjtwm`.

### 4. Verify: a colleague checks it offline

A compliance reviewer at another company never logged into emem. They
take the `fact_cid` and open it in their own browser:

```
https://emem.dev/verify/qi3jo4sqcg…l2hgjtwm
```

The page pulls the same bytes and checks the ed25519 signature locally,
in WebCrypto. The reviewer is not trusting emem.dev or the importing
agent; they are checking the math themselves. If the signature holds,
the bytes are exactly what the responder signed, unchanged.

## What just happened that a plain LLM could not do

- The agent had a **stable handle** for a place, not a paraphrase.
- The answer carried its **provenance** (which upstream source) and a
  **content id** (which exact bytes).
- The citation was **checkable by a stranger, offline**, with no
  account and no trust in the server.

That is the whole point: an agent's memory of the world becomes
something other agents (and auditors) can verify without taking
anyone's word for it.

## Next

- [Quickstart](./quickstart.md): the same flow in five languages.
- [What only emem does](./only-emem.md): the capabilities no plain
  vector store gives you.
- [Connect & evolve](./connect-and-evolve.md): how facts link to each
  other and how the corpus improves over time.

## Where it sits next to what you already run

emem does not replace your vector store. It is the layer under it for facts about the physical world.

| | emem | Vector DB | RAG pipeline | Tile / catalog API |
|---|---|---|---|---|
| Shared across agents and orgs | one memory, any reader | within one deployment | within one app | reads only |
| Verifiable offline | ed25519 + content id, no account | no | no | no |
| Content-addressed | id = hash of the bytes | ids are arbitrary | no | paths are positional |
| Empty answer | signed absence, typed reason | empty result set | empty retrieval | `404` |
| Source disagreement | kept and scored | duplicates or last write | up to the app | not modeled |
| Handing one fact to another agent | one short token | share credentials or re-embed | resend the text | send URL and hope it is stable |
| Auth to read | none | usually an API key | app-specific | usually an account |

## An unsigned memory can be poisoned

Published work on memory-injection attacks (MINJA, NeurIPS 2025)
reports a 95% success rate at planting false records in a conventional
agent memory store through queries alone. Signatures do not make a
writer honest, but they pin every record to a key: in emem a planted
fact is attributable, scoreable against other writers, and
supersedable, instead of anonymous and silently trusted. Policy for
which writers to trust stays with the reader.

