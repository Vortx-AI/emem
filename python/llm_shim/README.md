# emem LLM shim

This directory holds `openai_bedrock_shim.py`, one Python file. It gives an
OpenAI-style `POST /v1/chat/completions` endpoint and sends each request to
Amazon Bedrock (the Converse API). It adds a daily spend cap, a response
cache and a fallback to a local model.

You need this page if you operate an emem node and use its model features.
Callers include the `gemma` backend of the explain sidecar
([`python/gemma_explain/`](../gemma_explain/README.md)), the default text
model of `emem-server` (`EMEM_A2A_LLM_URL`), `scripts/agent_ack.py`,
`scripts/agent_reply.py` and `bench/experience/adapters.py`. The unit
`deploy/systemd/emem-agent-reply.service` sends `scripts/agent_reply.py` to
port 5015. The read primitives (`locate`, `recall`,
`verify_receipt`) do not call a model, so a client of emem.dev does not
need the shim.

## Try it without an AWS account

The shim needs Python 3 and `boto3`. These commands run it on a spare port
with invalid credentials, so no request can reach Bedrock or cost money.
`AWS_EC2_METADATA_DISABLED=true` stops `boto3` from using an instance role.

```sh
cd python/llm_shim
AWS_ACCESS_KEY_ID=AKIAFAKE AWS_SECRET_ACCESS_KEY=fake \
AWS_EC2_METADATA_DISABLED=true \
LLM_SHIM_BIND=127.0.0.1:5915 \
LLM_SHIM_SPEND_FILE=/tmp/shim/spend.json \
LLM_SHIM_CACHE_DIR=/tmp/shim/cache \
  python3 openai_bedrock_shim.py
# in a second shell
curl -s localhost:5915/health
```

On 7 November 2026 `/health` gave this answer:

```json
{"status": "ok", "backend": "bedrock", "model": "amazon.nova-micro-v1:0",
 "region": "us-east-1", "streaming": false,
 "budget": {"day": "2026-11-07", "cap_usd": 10.0, "spent_usd": 0.0,
            "remaining_usd": 10.0, "calls": 0, "over_cap": false,
            "ledger_ok": true, "ledger_file": "/tmp/shim/spend.json",
            "fallback": null}}
```

A chat request in this setup gets HTTP 502 with `"type": "upstream_error"`,
because Bedrock refuses the fake key and the shim has no fallback. Set
`LLM_SHIM_FALLBACK_URL` to any OpenAI-style endpoint and the same request
gets HTTP 200 from that endpoint. The answer then has the field
`"x_budget": "bedrock unavailable (ClientError); served by the local model instead"`.
We ran both cases on 7 November 2026 with a small local stub as the
fallback.

## Routes

| route | answer |
|---|---|
| `GET /health`, `GET /v1/health` | status, model, region and the budget for the current UTC day |
| `GET /v1/models` | one entry: the value of `LLM_SHIM_MODEL` |
| `POST <any path that ends in>/chat/completions` | an OpenAI `chat.completion` object |
| any other path | HTTP 404 |

The shim has no authentication. Bind it to a loopback address only.

## What a chat request does

1. The shim reads `messages`, `max_tokens` and `temperature`. It ignores
   all other fields, `model` and `stream` included.
2. If `temperature` is at or below `LLM_SHIM_CACHE_MAX_TEMP`, the shim looks
   in its cache. A hit returns the stored answer with zero tokens in
   `usage` and `"x_cache": "hit"`.
3. If the spend for the current UTC day is at or above the cap, the shim
   does not call Bedrock. It sends the request to the fallback.
4. Otherwise it calls Bedrock with the model in `LLM_SHIM_MODEL`. If that
   call fails for any reason, for example an expired credential, the shim
   sends the request to the fallback.
5. After a Bedrock answer, the shim adds the cost to the spend file and
   stores the answer in the cache. It does not cache fallback answers.

The answer always names `LLM_SHIM_MODEL` in `model`, whatever model name
the caller sent. A Bedrock answer also carries `x_budget_spent_usd`,
`x_budget_cap_usd`, `x_latency_ms` and `x_cache`. Its value is `miss`, or
`bypass (temperature above cache limit)`.

| case | HTTP status | body |
|---|---|---|
| Bedrock answers | 200 | `chat.completion` |
| over the cap or Bedrock fails, fallback answers | 200 | the fallback body plus `x_budget` |
| over the cap, no fallback set | 503 | `"type": "budget_exhausted"` |
| over the cap, fallback fails | 502 | `"type": "budget_exhausted"` |
| Bedrock fails, fallback fails or has no URL | 502 | `"type": "upstream_error"` |

## Limits

- No streaming. The shim answers `"stream": true` with one complete body.
- Text only. The shim joins list content into one string and drops
  non-text parts, such as images.
- It merges two messages in a row from the same role, because Converse
  refuses them.
- The cost uses fixed prices per million tokens (`LLM_SHIM_USD_IN`,
  `LLM_SHIM_USD_OUT`). The defaults are for Nova Micro. Check them
  against the AWS price page before you change the model.
- If the shim cannot write the spend file, it counts the cap as spent and
  uses the fallback until you fix the file. `/health` then shows
  `"ledger_ok": false`.

## Environment variables

| variable | default in the code | what it sets |
|---|---|---|
| `LLM_SHIM_BIND` | `127.0.0.1:5014` | listen address |
| `LLM_SHIM_MODEL` | `amazon.nova-micro-v1:0` | Bedrock model id |
| `LLM_SHIM_REGION` | `AWS_REGION`, else `us-east-1` | Bedrock region |
| `LLM_SHIM_MAX_TOKENS` | `800` | `maxTokens` when the request has no `max_tokens` |
| `LLM_SHIM_DAILY_USD` | `10` | daily spend cap in USD, UTC day |
| `LLM_SHIM_USD_IN`, `LLM_SHIM_USD_OUT` | `0.035`, `0.14` | USD per million input and output tokens |
| `LLM_SHIM_FALLBACK_URL` | empty | full URL of the fallback `chat/completions` endpoint |
| `LLM_SHIM_SPEND_FILE` | `/home/ubuntu/emem/var/emem/llm_shim_spend.json` | spend ledger |
| `LLM_SHIM_CACHE` | `1` | `0` turns the cache off |
| `LLM_SHIM_CACHE_DIR` | `/home/ubuntu/emem/var/emem/llm_shim_cache` | cache directory |
| `LLM_SHIM_CACHE_MAX_TEMP` | `0` | highest `temperature` that uses the cache |

The default paths are for the emem.dev host. Set your own paths on any
other machine.

## Run it as a service

[`deploy/systemd/emem-llm-shim.service`](../../deploy/systemd/emem-llm-shim.service)
runs the shim on `127.0.0.1:5015`. It sets a cap of 10 USD a day, a cache
limit of `temperature` 0.2, and the fallback
`http://127.0.0.1:5014/v1/chat/completions`. That port is the local
llama.cpp server. The unit reads AWS credentials from
`/home/ubuntu/.config/emem/aws.env`. Without that file, every request goes
to the fallback. The unit uses the Python in
`/home/ubuntu/.emem-witness/venv`, which must have `boto3`. This
directory has no requirements file.

## Ports: the code and its callers disagree

The callers default to port 5015: `EMEM_EXPLAIN_GEMMA_BASE` in
`explain_sidecar.py`, `EMEM_A2A_LLM_URL` in
`crates/emem-api-rest/src/lib.rs` (`default_llm_url`) and in
`scripts/agent_ack.py`, and `bench/experience/adapters.py`. The shim
itself defaults to 5014, and its module docstring still says that all
callers use 5014. Port 5014 is now the local llama.cpp server. If you
start the shim without `LLM_SHIM_BIND=127.0.0.1:5015`, the callers do not
reach it.
