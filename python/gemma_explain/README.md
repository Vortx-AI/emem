# emem "explain" sidecar

An **optional, clearly-unsigned** natural-language layer over emem's signed
facts. It rewords what emem already signed; it never produces a fact.

## Read this first: what the name means now

This directory is called `gemma_explain`, and the model it serves **by
default is `qwen2.5-7b`, not Gemma.** The sidecar loaded
`google/gemma-4-12B-it` locally until 2026-06-21, when it was changed to
forward to the geo.qa serving stack, freeing about 8 GB of VRAM on the shared
card. This README's previous claim that it "loads Gemma-4-12B in 4-bit (~7 GB
GPU)" described a deployment that stopped existing two months earlier.

A Gemma is still reachable, and the honest version is a measurement rather
than a name. `EMEM_EXPLAIN_GEOQA_MODEL=terraground-gemma-12b` selects a
geo-tuned Gemma on the same stack. Measured 2026-08-13, same prompt:

| model | first call | warm |
|---|---|---|
| `qwen2.5-7b` (default, always resident) | 0.17 s | 0.17 s |
| `terraground-gemma-12b` | 14.6 s | **0.48 s** |

The 14.6 s is a base swap after eviction, not the model's speed. Warm, the
geo-tuned model costs about 2.8x the default and is a live option, which is
a materially different trade from the "at the cost of base-swap latency"
note in the source suggests if you read it as a permanent penalty.

The directory name is left alone for now: renaming is a separate decision
from telling the truth about what runs, and the path appears in a systemd
unit.

## The hard rule it respects

emem's `/v1/ask` answer is **deterministic and LLM-free on purpose** — the
`answer` string is a pure projection of the signed fact set, so the receipt is
byte-stable and re-verifiable offline. **This sidecar does not touch that
path.** It is a separate process that reads an emem `/v1/ask` response,
rewords it for a non-expert, and returns prose marked `signed: false`.

The model is instructed to interpret only the numbers emem already signed and
never to invent one. The signed artifact remains the emem receipt
(`fact_cids` + `signature`); this prose is commentary, not ground truth.

## What actually runs, as of 2026-08-13

| | |
|---|---|
| deployed script | `/home/ubuntu/emem-local/explain_sidecar.py` |
| this repo's copy | **diverged** — still contains the local-model loader |
| model | `qwen2.5-7b` by default; `terraground-gemma-12b` selectable, both via geo.qa's OpenAI-compatible `/v1/chat/completions` |
| upstream | `http://127.0.0.1:8100` (`GEOQA_BASE_URL`) |
| listen | `127.0.0.1:5071` (`EMEM_EXPLAIN_BIND`) |
| generation cap | 160 tokens (`EMEM_EXPLAIN_MAX_TOKENS`) |
| emem wiring | **`POST /v1/explain` IS wired and live** |
| measured latency | ~1.6 s end to end through emem.dev |

The two facts most worth carrying: the repo copy is not what runs, and
`/v1/explain` is wired. This README previously said the wiring was "NOT done
— deliberately staged", which was true when written and false by the time
anybody read it.

## Health

```bash
curl localhost:5071/health
# {"ok": true, "model": "qwen2.5-7b", "backend": "geoqa", "signed_output": false}
```

## Env

| var | default | meaning |
|---|---|---|
| `EMEM_EXPLAIN_BIND` | `127.0.0.1:5071` | listen address |
| `GEOQA_BASE_URL` | `http://127.0.0.1:8100` | upstream serving stack |
| `GEOQA_API_KEY` | (unset) | required; `/health` reports `key_configured` |
| `EMEM_EXPLAIN_GEOQA_MODEL` | `qwen2.5-7b` | fast and always resident |
| `EMEM_EXPLAIN_MAX_TOKENS` | `160` | generation cap |

## What this is not

It is not a reasoner over emem, and it is not a gatekeeper. It receives an
already-computed answer and rephrases it. It holds no tools, cannot recall,
resolve or verify anything, and cannot check its own output against the facts
it was handed. A caller who wants a claim checked wants
`POST /v1/guard/verdict`, which resolves every citation in a draft and reports
what did not.
