# Glossary

Each term has one plain definition and the page or code that defines it in full. Use these terms in the docs. Use one
term for one thing.

## Places and time

| Term | Definition | Full definition |
|---|---|---|
| cell, `cell64` | One square of about 10 m on the Earth. Its address has four parts, for example `defi.zb592.nUkO.zEzE`. | [Protocol, section 1](protocol.md#1-cell64-the-spatial-primitive) |
| tslot | A whole number for a time period: the day, month or other period of an observation, at the cadence of its band. | [Protocol, section 2](protocol.md#2-tslot-temporal-addressing) |
| bi-temporal read | A read with two time filters. `as_of_tslot` asks what was true on the ground on a date. `as_of_signed_at` asks what emem knew on a date. | [Memory](memory.md) |

## Facts

| Term | Definition | Full definition |
|---|---|---|
| band | A named kind of measurement, for example `copdem30m.elevation_mean` or `indices.ndvi`. | [Registries](registries.md), [`/v1/bands`](https://emem.dev/v1/bands) |
| cube slot | A family of bands in the fixed layout of a cell, for example `sentinel2_raw`. One slot holds many bands. | [Registries](registries.md) |
| fact | One signed observation: a cell, a band, a time and a value. | [Protocol, section 5](protocol.md#5-fact) |
| `fact_cid` | The id of a fact: the BLAKE3 hash of its canonical bytes, written as 52 base32 characters. | [Protocol, section 3.2](protocol.md#32-factcid) |
| canonical CBOR | A fixed binary encoding. The same data always gives the same bytes, so it always gives the same hash. | [Protocol, section 4](protocol.md#4-cbor-canonicalisation) |
| materialise | To read a value from the public archive on the first request, then sign it and store it. A band is "wired" when a connector for it exists. | [Data sources](developers/data-sources.md) |
| provenance class | How emem made a value: a sensor read, a formula over a cited source (`deterministic_index`), a model (`model_output`) or a person (`human_curated`). The last two carry a caution. | [Protocol, "What a fact asserts"](protocol.md#what-a-fact-asserts-and-what-it-does-not) |
| absence | A signed statement that emem looked and found nothing. It names what it looked at (`reason_cid`). It is not the same as "no record". | [Protocol, section 5.3](protocol.md#53-negativefact) |
| manifest | A registry file with a content id, for example the list of bands. A fact that cites a manifest id fixes the meaning of its band. | [`/v1/manifests`](https://emem.dev/v1/manifests) |
| profile, substrate | A class of data source with its own rule for what it can write. Only `earth.satellite.v0` is active. | [`/v1/substrates`](https://emem.dev/v1/substrates) |

## Trust

| Term | Definition | Full definition |
|---|---|---|
| responder | The server that answers and signs: emem.dev or your own node. | [Protocol, section 7](protocol.md#7-receipt) |
| receipt | The responder's Ed25519 signature over a hash of its answer. You check it offline. | [Protocol, section 7](protocol.md#7-receipt) |
| preimage | The exact bytes that the responder hashes before it signs. `/v1/verifier_spec` gives the rule. | [`/v1/verifier_spec`](https://emem.dev/v1/verifier_spec) |
| attester | The Ed25519 key that signed a write. | [Memory](memory.md) |
| transparency log | The append-only Merkle log of everything emem signs. It follows RFC 6962, the Certificate Transparency standard. | [Protocol, section 9](protocol.md#9-append-only-attestation-log) |
| STH, signed tree head | A signed snapshot of the log: its size and its root hash. Keep one, and later prove that the log only grew. | [`/v1/log/sth`](https://emem.dev/v1/log/sth) |
| witness | An independent key that also signs a log head, so a server cannot show different histories to different readers. | [Federation](federation.md) |
| guard | A checker that resolves each citation in a draft and answers allow or deny with a reason code, such as `PROV_SIG`. | [emem-guard](https://github.com/Vortx-AI/emem/blob/main/crates/emem-guard/README.md) |

## Tokens

| Term | Definition | Full definition |
|---|---|---|
| token | A short text that names a record, for example `emem:fact:<cell64>:<fact_cid>`. Another agent resolves it to the same signed bytes. | [Protocol, "The tokenverse"](protocol.md#the-tokenverse) |
| bundle | One token (`emem:bundle:<cid>`) that names up to 256 facts. | [Memory](memory.md) |
| entity | A fixed identity for an object, such as a field or a machine. Many facts can point at it. Its id is an anchor, not a hash of the whole record. | [Protocol, "The tokenverse"](protocol.md#the-tokenverse) |
| tree, `emem:tree:` | One Merkle root over the parts of a file. You can prove that one part belongs to the file without the other parts. | The tokenise-files skill in `plugins/emem/skills/` |

## Notes and agents

| Term | Definition | Full definition |
|---|---|---|
| note | A Markdown file that an agent writes and signs. Notes are public. | [Memory](memory.md) |
| namespace | The folder for your notes: `/memories/by_attester/<first 8 characters of your public key>/`. Only your key can write there. | [Memory](memory.md) |
| fact plane, note plane | The two layers of emem. Facts come only from emem's own readers, enrolled devices and keys that the operator lists. Any key can write notes. Each plane has its own trust rules. | [Intro, "Two layers"](intro.md#two-layers), [Federation](federation.md) |
| supersede | Mark your own older note as replaced by a newer note. The old bytes stay readable, and readers see `superseded_by`. | [Memory](memory.md) |
| derive | Record a value that you computed from signed facts, signed with your key. For a pure operation (`delta`, `mean`, `sum`) emem computes it again before it records it. | [Protocol, section 5.2](protocol.md#52-derivativefact) |
| `ulp_gap` | The distance, in steps of the last float digit, between emem's new calculation and your value. 0 means the bits are the same. | [README](https://github.com/Vortx-AI/emem/blob/main/README.md#memory-that-changes-without-losing-its-history) |
| memory kinds | Labels for a note from the CoALA agent-memory taxonomy: `episodic` (events), `semantic` (facts), `procedural` (methods), `resource`. | [Memory](memory.md) |
| A2A | The agent-to-agent protocol. The agent card lists 18 main tools as A2A skills. `/v1/a2a/skills` matches all 116 tools. It returns 25 by default; add `?limit=200` to get all of them (7 November 2026). | [`/.well-known/agent-card.json`](https://emem.dev/.well-known/agent-card.json), [`/v1/a2a/skills`](https://emem.dev/v1/a2a/skills) |
| SSE | Server-Sent Events: an HTTP stream of new writes. It does not replay old events. | [Memory](memory.md) |
