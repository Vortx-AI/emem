# emem explain sidecar

This directory holds `explain_sidecar.py`, a small Python HTTP service. It
takes an emem `/v1/ask` response and rewords it as plain prose for a
non-expert. Nobody signs the prose, and it is not a fact. The signed answer
stays the `/v1/ask` receipt and its `fact_cids`.

You need this page if you operate an emem node and want `POST /v1/explain`
to return prose. Callers of emem.dev do not need it: the public endpoint
already works.

## Try it

This call goes to emem.dev and needs no key:

```bash
curl -s -X POST https://emem.dev/v1/explain \
  -H 'content-type: application/json' \
  -d '{"q":"what is the elevation of Mount Fuji?"}'
```

On 7 November 2026 it returned HTTP 200 in 1.6 s with `"signed": false`,
`"model": "google.gemma-3-12b-it"` and a short `explanation`. The
`source` object holds the signed answer (elevation 3539.04 m), its receipt
and the fact cid `qejpz7cgvk347ordd6vrdtmwzprqlzcnkmotsjton2x4y2jjbplq`.

To run the sidecar itself, use Python 3 with no extra packages:

```bash
cd python/gemma_explain
EMEM_EXPLAIN_BACKEND=none EMEM_EXPLAIN_CACHE_DIR=/tmp/explain_cache \
  python3 explain_sidecar.py
# in a second shell
curl -s localhost:5071/health
```

With `EMEM_EXPLAIN_BACKEND=none` the sidecar calls no model. It returns
the deterministic `answer` from the `/v1/ask` response unchanged, with
`"degraded": true`. We ran both commands on 7 November 2026.

## Where it sits

```text
caller -> emem-server POST /v1/explain
            1. runs /v1/ask (or uses the `ask` you sent)
            2. POST {"ask": ...} to EMEM_EXPLAIN_SIDECAR_URL
               (default http://127.0.0.1:5071/explain)
                 -> explain_sidecar.py -> model backend
            3. returns the prose with `source` = signed answer + receipt
```

The `/v1/ask` path does not change. Its `answer` is a projection of the
signed facts with no language model, so the receipt stays byte-stable. The
sidecar only reads a copy of that response.

`POST /v1/explain` accepts either `ask` (a full `/v1/ask` response) or
`q` with an optional `cell`, `place`, or `lat` and `lng`. If the sidecar
does not answer in 40 s, or answers with an error status, emem-server
returns HTTP 200 with `"available": false` and the signed `source`. Source:
`post_explain` in `crates/emem-api-rest/src/lib.rs`.

## What the sidecar sends to the model

The sidecar does not send the full response. It sends only these fields:
`place_resolved`, `answer`, `band_observations_summary`,
`algorithm_outcomes_summary`, `caveats`, `routed_to` and the count of
`fact_cids`. The system prompt asks for 2 to 4 sentences. It tells the
model to use only the given numbers and to say "not measured" when a reading is absent.
Temperature is 0.

The model cannot recall, resolve or verify anything, and nothing checks
its prose against the facts. To check a draft against its citations, use
`POST /v1/guard/verdict`.

## Backends

`EMEM_EXPLAIN_BACKEND` selects the backend. The default is `gemma`.

| value | what answers | if the backend fails |
|---|---|---|
| `gemma` (default) | OpenAI-style `/v1/chat/completions` at `EMEM_EXPLAIN_GEMMA_BASE` | returns the deterministic answer, `degraded: true` |
| `bedrock` | Amazon Bedrock Converse API, called from this process | returns the deterministic answer, `degraded: true` |
| `none` | no model | always the deterministic answer |
| any other value | Cosmos chat service at `EMEM_EXPLAIN_COSMOS_BASE` | returns HTTP 200 with `{"error": ...}`, no fallback |

We checked each failure path on 7 November 2026 with no backend running.
With `gemma`, the answer has `"degraded": true` and a `degraded_reason`
that names the connection error.

Point the `gemma` backend at the llm shim in
`python/llm_shim/openai_bedrock_shim.py` (see its
[README](../llm_shim/README.md)). The shim holds the Bedrock
credential, a daily spend cap and a response cache. Do not point
`EMEM_EXPLAIN_GEMMA_BASE` straight at Bedrock: the sidecar meters nothing.

Two points in the code need care when you run both processes:

- The shim listens on `127.0.0.1:5014` by default (`LLM_SHIM_BIND`). The
  sidecar calls `http://127.0.0.1:5015` by default. Set one of them so
  that the ports agree.
- The shim ignores the model name in the request and serves
  `LLM_SHIM_MODEL` (default `amazon.nova-micro-v1:0`). The sidecar writes
  `EMEM_EXPLAIN_GEMMA_MODEL` into the `model` field of its response. Set
  both variables to the same model, or `model` names the wrong author.

The `bedrock` backend needs `boto3` and AWS credentials. The other
backends use only the standard library.

## HTTP routes

| route | body | returns |
|---|---|---|
| `GET /health` | none | `ok`, `backend`, `model`, `base`, `cache`, `degrades_to`, `signed_output: false` |
| `POST /explain` | `{"ask": <a /v1/ask response>}`, or the response itself | `explanation`, `signed: false`, `disclaimer`, `model`, `via`, `cached`, and `degraded` when it fell back |

`/health` reports the backend that the next request will use.

## Cache

The sidecar caches each successful explanation on disk. The key is a
BLAKE2b hash of the system prompt, the model id and the fields sent to the
model. Two questions that resolve to the same signed facts share one entry.
A different model gets a new entry. The sidecar does not cache errors
or fallback answers. On a cache hit, the sidecar sets `tokens_in`, `tokens_out` and
`latency_ms` to 0 if they are present, and keeps the first values under
`origin_*`. The `bedrock` backend sets all three. The `cosmos` backend
sets only `latency_ms`. The `gemma` backend sets none of them.

You can delete the cache directory at any time. Nothing in it is a fact
or a note.

## Environment variables

| variable | default | meaning |
|---|---|---|
| `EMEM_EXPLAIN_BIND` | `127.0.0.1:5071` | listen address |
| `EMEM_EXPLAIN_BACKEND` | `gemma` | see Backends |
| `EMEM_EXPLAIN_GEMMA_BASE` | `http://127.0.0.1:5015` | base URL for the `gemma` backend |
| `EMEM_EXPLAIN_GEMMA_MODEL` | `google.gemma-3-12b-it` | model name sent and reported |
| `EMEM_EXPLAIN_BEDROCK_MODEL` | `amazon.nova-micro-v1:0` | Bedrock model id |
| `EMEM_EXPLAIN_BEDROCK_REGION` | `AWS_REGION`, else `us-east-1` | Bedrock region |
| `EMEM_EXPLAIN_COSMOS_BASE` | `http://127.0.0.1:5017` | base URL for the Cosmos backend |
| `EMEM_EXPLAIN_COSMOS_MODEL` | `nvidia/Cosmos3-Edge` | Cosmos model name |
| `EMEM_EXPLAIN_COSMOS_FAMILY` | `cosmos3_edge` | Cosmos model family |
| `EMEM_EXPLAIN_MAX_TOKENS` | `160` | output cap (not sent to Cosmos) |
| `EMEM_EXPLAIN_CACHE` | `1` | set `0` to turn off the cache |
| `EMEM_EXPLAIN_CACHE_DIR` | `$EMEM_DATA/explain_cache` (`EMEM_DATA` default `/data`) | cache directory |
| `EMEM_EXPLAIN_CACHE_MAX` | `20000` | read but not enforced in this file |

On the emem-server side, `EMEM_EXPLAIN_SIDECAR_URL` sets where it sends
the request.

## What is out of date in this directory

- The name. The directory has the name `gemma_explain` because the sidecar
  first loaded Gemma on a local GPU. That loader is gone. Since 21 June
  2026 the sidecar calls a separate model service.
- `deploy/systemd/emem-explain-sidecar.service`. This unit sets
  `EMEM_EXPLAIN_GEMMA_BASE=http://127.0.0.1:5014` (a local llama.cpp server)
  and `EMEM_EXPLAIN_GEMMA_MODEL=qwen2.5-1.5b-instruct-q4_k_m`. So it does not
  use the shim on port 5015. Its header comment still says Bedrock. On
  7 November 2026, emem.dev answered with `model` set to
  `google.gemma-3-12b-it`. The `gemma` backend copies that field from
  `EMEM_EXPLAIN_GEMMA_MODEL`, so production does not run this unit as written.
- `python/gemma_explain/emem-explain-sidecar.service`. This systemd unit still sets
  `EMEM_EXPLAIN_MODEL=google/gemma-4-12B-it`, a GPU device and a
  `transformers` virtual environment. The code does not read
  `EMEM_EXPLAIN_MODEL` and does not need a GPU. Update the unit before you
  use it.
- The start-up message says "fallback -> cosmos". Cosmos is not a fallback.
  It answers only when `EMEM_EXPLAIN_BACKEND` selects it.
