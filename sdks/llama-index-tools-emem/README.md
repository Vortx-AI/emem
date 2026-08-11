# LlamaIndex Tools Integration: emem

Signed, content-addressed facts about physical places, as LlamaIndex tools.

Every value comes back with an `emem:fact:` token. The token resolves to the
byte-identical signed record later, in another session or another process, and
verifies against the responder's published key. So an agent can cite what it
read, and whoever receives the answer can check it without trusting the agent
that wrote it.

Reads need no key and no account.

## Install

```bash
pip install llama-index-tools-emem
```

## Usage

```python
from llama_index.tools.emem import EmemToolSpec
from llama_index.core.agent.workflow import FunctionAgent
from llama_index.llms.openai import OpenAI

agent = FunctionAgent(
    tools=EmemToolSpec().to_tool_list(),
    llm=OpenAI(model="gpt-4.1-mini"),
)

await agent.run("How high is Bengaluru, and cite the fact you used.")
```

Point it at your own node with `EmemToolSpec(base_url="http://localhost:5051")`
or the `EMEM_URL` environment variable. A receipt minted on the public node
verifies against your own.

## Tools

| Tool | Purpose |
|---|---|
| `locate` | Resolve a place name to its canonical `cell64` address and list what is readable there |
| `recall` | Read signed measurements at a place, each with a citation token |
| `resolve_token` | Resolve an `emem:fact:` token that arrived from somewhere else |
| `verify_receipt` | Check a receipt's signature against the responder's published key |

## Three things worth knowing

**Results are trimmed, except the receipt.** A raw recall for one band is about
5 KB, mostly band documentation written for a human reading the API. That is
dropped, and `recall(..., band_help=True)` brings it back when the agent needs
to interpret an unfamiliar band rather than just report it.

Two fields are trimmed harder, both because of what they measured at
`https://emem.dev`. `locate`'s `data_at_this_cell` is a 19 KB briefing on what
the responder can do, not a band list; only its `live_bands_by_topic` answers
"what can I read here", and passing the topic map instead of the briefing took
the tool result from 20,058 characters to 3,712. A fact's `value` is normally
under 100 characters, but the embedding bands return vectors: at Bengaluru,
seven of them were 131,246 of a 162,365-character result. A value over 512
characters is replaced by its length, its first few elements and the citation
that resolves the whole thing.

What is left is mostly citations, and a citation used to be written out three
times. A `bands`-less recall at Bengaluru was 51,637 characters, of which
19,456 were repeats: a top-level `cite` list holding every fact's `cite` again,
and `fact_cid` and `cell` beside the `emem:fact:<cell>:<fact_cid>` token that
already spells both out. All three are gone, 19,666 characters with the
punctuation that held them, and the result is 31,971. A fact
therefore shows no `cell` and no `fact_cid` unless its token cannot yield them,
which happens for a descriptor token (`emem:fact:<lat>,<lng>@<date>@<band>:
<fact_cid>`, whose anchor reaches the cell by quantisation rather than by
spelling) and for a fact the responder cid'd but never tokenised.

The receipt is passed through whole, even though its `fact_cids` array is a
further copy of every cid and 5,837 of the 7,480 characters it costs at
Bengaluru. As of receipt preimage v2 the signature covers the inclusion proof,
so a receipt with any field removed does not raise an error, it verifies as
`signature_valid: false`. A tool that trimmed it would report honest data as
forged.

**Name the bands you need.** Omitting `bands` returns everything the responder
holds at the cell. At Bengaluru that is 104 facts and about 32 KB, nearly all
of it citations for bands nobody asked about. Call `locate` first and pick from
`bands_available_here`.

**Verification is per-responder.** `signature_valid: true` proves that this
responder signed those bytes. It does not prove the measurement is correct, and
no network consensus is involved.

## Development

```bash
pip install -e ".[dev]"
pytest tests
```

`tests/fixtures/` holds verbatim response bodies written by
`tests/capture_fixtures.py`, not by hand. `tests/test_live_contract.py` replays
each fixture's recorded request against the live origin and fails if a key the
fixture claims has stopped arriving; it skips when nothing answers, and a skip
means the shapes were not checked on that run.

```bash
python3 tests/capture_fixtures.py                          # re-capture
EMEM_URL=http://localhost:5051 pytest tests/test_live_contract.py
```
