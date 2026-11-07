# llama-index-tools-emem: emem tools for LlamaIndex agents

`llama-index-tools-emem` gives a LlamaIndex agent four tools over the
[emem.dev](https://emem.dev) Earth memory protocol. Use it when a LlamaIndex
agent must read signed facts about a place and cite them. Reads need no key
and no account.

The package is not on PyPI (checked on 7 November 2026). Install it from the
repository:

```bash
pip install "git+https://github.com/Vortx-AI/emem.git#subdirectory=sdks/llama-index-tools-emem"
python -c 'from llama_index.tools.emem import EmemToolSpec; print(EmemToolSpec().locate("Mount Fuji")["cell64"])'
```

The second line prints the cell64 address of Mount Fuji
(`defi.zb592.nUkO.zEzE` on 7 November 2026). For the protocol itself, read
[docs/quickstart.md](https://github.com/Vortx-AI/emem/blob/main/docs/quickstart.md)
and [docs/agents.md](https://github.com/Vortx-AI/emem/blob/main/docs/agents.md).

## What the tools return

Every value comes back with an `emem:fact:` token. Later, in another session
or another process, the token resolves to the same signed bytes. Anyone can
check the signature on those bytes. To be sure of the signer, compare the key
in the receipt with the responder's published key. Thus the receiver
of an answer can check it without trust in the agent that wrote it.

| Tool | What it does |
|---|---|
| `locate` | Changes a place name to its `cell64` address and lists the bands you can read there |
| `recall` | Reads signed facts at a place, each with a citation token |
| `resolve_token` | Gets the signed record for an `emem:fact:` token from another source |
| `verify_receipt` | Checks the signature of a receipt against the key in the receipt |

The package needs Python 3.10 or later. It depends on `llama-index-core`
(0.13 or later, before 0.15) and `httpx`. The version is 2.4.4, the same as the rest of emem.

## Call the tools directly

This example runs without an LLM. All values came from emem.dev on
7 November 2026.

```python
from llama_index.tools.emem import EmemToolSpec

emem = EmemToolSpec()
loc = emem.locate("Mount Fuji")
res = emem.recall(loc["cell64"], bands=["copdem30m.elevation_mean"])
fact = res["facts"][0]
print(fact["value"], fact["unit"])   # 3539.042236328125 m
print(fact["cite"])                  # emem:fact:defi.zb592.nUkO.zEzE:qejpz7cg...
print(emem.resolve_token(fact["cite"])["value"])          # 3539.042236328125
print(emem.verify_receipt(res["receipt"])["signature_valid"])  # True
```

## Use the tools in an agent

The tools go to any LlamaIndex agent through `to_tool_list()`. This example
also needs `llama-index-llms-openai` and an OpenAI key. We did not run it on
7 November 2026.

```python
import asyncio
from llama_index.core.agent.workflow import FunctionAgent
from llama_index.llms.openai import OpenAI
from llama_index.tools.emem import EmemToolSpec

agent = FunctionAgent(
    tools=EmemToolSpec().to_tool_list(),
    llm=OpenAI(model="gpt-4.1-mini"),
)

print(asyncio.run(agent.run("How high is Bengaluru? Cite the fact you used.")))
```

## Use your own node

To use your own node, give `EmemToolSpec(base_url="http://localhost:5051")` or
set `EMEM_URL`. The default is `https://emem.dev`. A receipt from emem.dev
also checks on your own node, because the check uses the key in the receipt.
To be sure of the signer, compare that key with
`federation.node.responder_pubkey_b32` in
[`/.well-known/emem.json`](https://emem.dev/.well-known/emem.json).

## Why the results are short

The tools remove text that an agent does not need. They keep the value, the
unit, the time slot and the citation token. On 7 November 2026 these were the
sizes, in characters of JSON. They change by a few characters on each call,
because the receipt is new each time.

| Call | Raw response | Tool result |
|---|---|---|
| `recall("Mount Fuji", bands=["copdem30m.elevation_mean"])` | about 6,500 | about 2,300 |
| `locate("Bengaluru")` | about 153,400 | about 3,800 |
| `recall("Bengaluru")`, bands omitted (574 facts) | about 1,861,000 | about 159,500 |

The tools make these cuts:

- `recall` removes the band documentation from each fact. Use
  `recall(..., band_help=True)` to get it back for a band that you do not know.
- `locate` keeps only `live_bands_by_topic` from `data_at_this_cell`. The tool
  returns it as `bands_available_here`. The rest is a description of the
  responder, not a band list.
- A fact value longer than 512 characters becomes its length, its first
  elements and the citation token. The embedding bands return such vectors. At
  Bengaluru, 7 values had 130,114 characters together.
- A fact shows no `cell` and no `fact_cid` when its token
  (`emem:fact:<cell>:<fact_cid>`) already contains them. A descriptor token
  does not contain the cell, so that fact keeps the `cell` field.

The tools do not cut the receipt. At Bengaluru it had about 34,000
characters. From receipt preimage version 2, the signature covers the inclusion proof. A receipt with a
field removed thus gives `signature_valid: false`, and honest data looks forged.

## Name the bands you need

If you omit `bands`, `recall` returns every fact the responder holds at the
cell. At Bengaluru on 7 November 2026 that was 574 facts, and the tool result
was about 159,500 characters. Call `locate` first, then select bands from
`bands_available_here`.

## What a valid signature proves

`signature_valid: true` proves that this responder signed these bytes. It does
not prove that the measurement is correct. It does not use network consensus.

## Develop and test

The test tools are in the `dev` dependency group, not in an extra. From
`sdks/llama-index-tools-emem`:

```bash
pip install -e . pytest pytest-mock respx
pytest tests
```

`tests/capture_fixtures.py` writes the response bodies in `tests/fixtures/`.
Do not edit them by hand. `tests/test_live_contract.py` sends each recorded
request to the live responder again. It fails if a key that a fixture has is
no longer in the response. It skips when no responder answers, and a skip
means that the test did not check the shapes.

```bash
python3 tests/capture_fixtures.py
EMEM_URL=http://localhost:5051 pytest tests/test_live_contract.py
```

On 7 November 2026, against emem.dev, 40 tests passed and 3 live-contract
tests failed. Two `locate` fixtures name the topics `foundation_embedding`
and `nightlights`, which emem.dev no longer sends. The third test expects a
bands-omitted recall at Bengaluru to stay under 40,000 characters. It was
about 159,500.
