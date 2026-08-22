#!/usr/bin/env python3
"""
emem "explain" sidecar — an OPTIONAL, clearly-UNSIGNED natural-language layer
over emem's signed facts.

Why this exists
---------------
emem's /v1/ask synthesis is deliberately deterministic and LLM-free: the
`answer` string is a pure projection of the signed fact set, so the receipt is
byte-stable and re-verifiable offline. That property is non-negotiable and this
sidecar does NOT touch it.

This service is a separate process. It READS an emem /v1/ask response (the
signed facts + the deterministic answer) and rewords it for a non-expert. The
model never invents numbers — it only rephrases the ones emem already signed.
The output is explicitly marked `signed: false`; the signed artifact remains the
emem receipt, not this prose.

As of 2026-06-21 this NO LONGER loads a model locally. It forwards to the geo.qa
LLM serving stack (OpenAI-compatible /v1/chat/completions), which hosts the
shared, eviction-managed GPU models. This frees ~8GB of VRAM on the shared card
(the local Gemma copy is gone) and keeps the explain layer running. The HTTP
contract to emem-server is unchanged: POST /explain {ask} -> {explanation, ...}.

Run
---
    python explain_sidecar.py
    # GET  /health
    # POST /explain   {"ask": <emem /v1/ask response JSON>}  ->  {explanation, ...}

Env: EMEM_EXPLAIN_BIND (default 127.0.0.1:5071), EMEM_EXPLAIN_MAX_TOKENS (160),
     EMEM_EXPLAIN_COSMOS_BASE (default http://127.0.0.1:5017) and
       EMEM_EXPLAIN_COSMOS_MODEL (default nvidia/Cosmos3-Edge) — the fallback
       backend, on its own service, so it needs no base swap at all.
"""
import os, json, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib import request as _req, error as _err

BIND = os.environ.get("EMEM_EXPLAIN_BIND", "127.0.0.1:5071")
MAX_TOKENS = int(os.environ.get("EMEM_EXPLAIN_MAX_TOKENS", "160"))
# GEOQA_BASE_URL and GEOQA_API_KEY are gone with the route that used them.
#
# Both backends are local services now — Gemma on its own port, Cosmos on its
# own — so neither needs a key, and a variable this file no longer reads is a
# variable an operator can set and watch do nothing. A neighbour was holding
# 4.7 GB of model weights on disk on the strength of this file naming them.
# Gemma, on its own server, rather than Qwen on the neighbour's.
#
# This surface answered as qwen2.5-7b through geo.qa's shared stack.
# Switching it there means a base swap on a card another live product is
# using, and this file's own docstring warns what that latency costs.
# :5014 already serves gemma-4-12B-it independently and needs no swap.
#
# It also speaks a slightly different dialect: base_model rather than
# model. One field, and the reason an earlier "replace the qwen api with
# gemma" change reached the splats bridge and never reached this.
GEMMA_BASE = os.environ.get("EMEM_EXPLAIN_GEMMA_BASE", "http://127.0.0.1:5014").rstrip("/")
GEMMA_MODEL = os.environ.get("EMEM_EXPLAIN_GEMMA_MODEL", "google/gemma-4-12B-it")
USE_GEMMA = os.environ.get("EMEM_EXPLAIN_BACKEND", "gemma") == "gemma"
# Cosmos, on its own port, rather than qwen2.5-7b on the neighbour's stack.
#
# The old default was qwen2.5-7b, described here as "fast + always-resident".
# Both halves of that are now false: the weights were evicted from this box on
# 2026-08-17 to reclaim disk, and the model's registry rows were archived, so
# the geoqa route returns None and this sidecar turns that into a 4xx. It could
# not have worked.
#
# Cosmos3-Edge at :5017 is its own service with its own interpreter, so unlike
# terraground-gemma-12b it costs no base swap on a card another live product is
# using — which removes the latency caveat the old docstring existed to warn
# about, rather than merely restating it.
COSMOS_BASE = os.environ.get("EMEM_EXPLAIN_COSMOS_BASE", "http://127.0.0.1:5017").rstrip("/")
COSMOS_MODEL = os.environ.get("EMEM_EXPLAIN_COSMOS_MODEL", "nvidia/Cosmos3-Edge")
COSMOS_FAMILY = os.environ.get("EMEM_EXPLAIN_COSMOS_FAMILY", "cosmos3_edge")

SYSTEM = (
    "You explain emem's SIGNED Earth-observation facts to a non-expert. "
    "Rules: (1) be concise, 2-4 sentences; (2) interpret ONLY the numbers given "
    "— never invent or estimate a value; (3) if a reading is absent, say it is "
    "not measured rather than guessing; (4) do not claim this explanation is "
    "signed — the signed artifact is emem's receipt, this is plain commentary."
)


def _digest_ask(ask: dict) -> str:
    """Pull only the signed, factual surface of an emem /v1/ask response so the
    model rewords facts, not the whole envelope."""
    keep = {
        "place_resolved": ask.get("place_resolved"),
        "answer": ask.get("answer"),
        "band_observations_summary": ask.get("band_observations_summary"),
        "algorithm_outcomes_summary": ask.get("algorithm_outcomes_summary"),
        "caveats": ask.get("caveats"),
        "signed_fact_count": len(ask.get("fact_cids") or []),
        "routed_to": ask.get("routed_to"),
    }
    return json.dumps({k: v for k, v in keep.items() if v is not None}, ensure_ascii=False)


def _explain_gemma(facts: str, ask: dict) -> dict:
    """Ask gemma-4-12B to reword facts it was handed, and nothing else."""
    payload = json.dumps({
        "base_model": GEMMA_MODEL,
        "family": "gemma",
        "messages": [{"role": "system", "content": SYSTEM},
                     {"role": "user", "content": "Signed facts:\n" + facts}],
        "max_tokens": MAX_TOKENS, "temperature": 0.0, "stream": False,
    }).encode()
    r = _req.Request(GEMMA_BASE + "/v1/chat/completions", data=payload, method="POST",
                     headers={"Content-Type": "application/json"})
    try:
        with _req.urlopen(r, timeout=120) as resp:
            data = json.loads(resp.read())
    except Exception as e:  # noqa: BLE001
        return {"error": f"gemma serving unavailable: {e}", "signed": False}
    text = ((data.get("choices") or [{}])[0].get("message", {}) or {}).get("content", "").strip()
    return {
        "explanation": text,
        "signed": False,
        "disclaimer": "Written by gemma-4-12B from emem's already-signed facts. This "
        "prose is NOT signed and is not a fact: verify the receipt (fact_cids, "
        "signature) for the ground truth it rewords.",
        "model": GEMMA_MODEL,
        "via": "gemma /v1/chat/completions",
        "source_routed_to": ask.get("routed_to"),
    }


def explain(ask: dict) -> dict:
    facts = _digest_ask(ask)
    if USE_GEMMA:
        return _explain_gemma(facts, ask)
    # No API key and no base swap: Cosmos is a local service on its own port.
    #
    # thinking:false deliberately. Explaining an already-signed fact in two
    # sentences is a rephrasing job, not a reasoning one, and the difference is
    # 1.1-2.5s against 13-22s.
    #
    # No max_tokens either. This model expands its deliberation to fill any
    # ceiling it is given, so a cap does not shorten the answer, it removes it:
    # measured, 1600 gave 5,465 characters and no conclusion while unthrottled
    # completed in 990 tokens. The service applies its own runaway guard.
    payload = json.dumps({
        "base_model": COSMOS_MODEL,
        "family": COSMOS_FAMILY,
        "thinking": False,
        "messages": [{"role": "system", "content": SYSTEM},
                     {"role": "user", "content": "Signed facts:\n" + facts}],
        "temperature": 0.0, "stream": False,
    }).encode()
    r = _req.Request(COSMOS_BASE + "/v1/chat/completions", data=payload, method="POST",
                     headers={"Content-Type": "application/json"})
    t0 = time.time()
    try:
        with _req.urlopen(r, timeout=120) as resp:
            data = json.loads(resp.read())
    except _err.HTTPError as e:
        return {"error": f"cosmos serving {e.code}: {e.read().decode(errors='replace')[:200]}", "signed": False}
    except Exception as e:  # noqa: BLE001
        return {"error": f"cosmos serving unavailable: {e}", "signed": False}
    text = ((data.get("choices") or [{}])[0].get("message", {}) or {}).get("content", "").strip()
    return {
        "explanation": text,
        "signed": False,
        "disclaimer": "Generated by a local language model from emem's already-signed facts. "
        "This prose is NOT signed and is not a fact — verify the emem receipt "
        "(fact_cids / signature) for the ground truth it rewords.",
        "model": COSMOS_MODEL,
        "via": "cosmos /v1/chat/completions",
        "source_routed_to": ask.get("routed_to"),
        "source_signed_fact_count": len(ask.get("fact_cids") or []),
        "latency_ms": round((time.time() - t0) * 1000),
    }


class H(BaseHTTPRequestHandler):
    def _send(self, code, obj):
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/health":
            # Report what the NEXT request would actually do, not what the
            # config would do if a different branch were taken.
            #
            # This said {"model":"qwen2.5-7b","backend":"geoqa"} unconditionally
            # while every /explain answered through Gemma at :5014. A neighbour
            # archiving the qwen rows went looking for what they had broken here
            # and found nothing broken, because the live path had never been on
            # qwen — but only after an hour of believing it was their fault. A
            # health endpoint that names a model the request path does not use
            # is a false statement about the system, however green it looks.
            if USE_GEMMA:
                live = {"backend": "gemma", "model": GEMMA_MODEL, "base": GEMMA_BASE}
            else:
                live = {"backend": "cosmos", "model": COSMOS_MODEL, "base": COSMOS_BASE}
            self._send(200, {"ok": True, **live, "signed_output": False,
                             "note": "model/backend/base describe the path a request takes right now"})
        else:
            self._send(404, {"error": "POST /explain or GET /health"})

    def do_POST(self):
        if self.path != "/explain":
            return self._send(404, {"error": "POST /explain"})
        try:
            n = int(self.headers.get("content-length", 0))
            req = json.loads(self.rfile.read(n) or b"{}")
            ask = req.get("ask") or req
            self._send(200, explain(ask))
        except Exception as e:  # noqa: BLE001
            self._send(500, {"error": str(e)})

    def log_message(self, *a):  # quiet
        pass


if __name__ == "__main__":
    host, port = BIND.split(":")
    print(f"[explain] fallback -> cosmos {COSMOS_BASE} (model {COSMOS_MODEL})", flush=True)
    srv = ThreadingHTTPServer((host, int(port)), H)
    print(f"[explain] serving on http://{BIND}  (POST /explain, GET /health)", flush=True)
    srv.serve_forever()
