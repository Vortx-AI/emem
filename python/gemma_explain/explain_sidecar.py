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
     GEOQA_BASE_URL (default http://127.0.0.1:8100),
     GEOQA_API_KEY (required — a geo.qa API key),
     EMEM_EXPLAIN_GEOQA_MODEL (default qwen2.5-7b — fast + always-resident; set
       terraground-gemma-12b for the geo-tuned model, at the cost of base-swap latency).
"""
import os, json, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib import request as _req, error as _err

BIND = os.environ.get("EMEM_EXPLAIN_BIND", "127.0.0.1:5071")
MAX_TOKENS = int(os.environ.get("EMEM_EXPLAIN_MAX_TOKENS", "160"))
GEOQA_BASE = os.environ.get("GEOQA_BASE_URL", "http://127.0.0.1:8100").rstrip("/")
GEOQA_KEY = os.environ.get("GEOQA_API_KEY", "")
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
GEOQA_MODEL = os.environ.get("EMEM_EXPLAIN_GEOQA_MODEL", "qwen2.5-7b")

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
    if not GEOQA_KEY:
        return {"error": "GEOQA_API_KEY not configured for the explain sidecar", "signed": False}
    payload = json.dumps({
        "model": GEOQA_MODEL,
        "messages": [{"role": "system", "content": SYSTEM},
                     {"role": "user", "content": "Signed facts:\n" + facts}],
        "max_tokens": MAX_TOKENS, "temperature": 0.0, "stream": False,
    }).encode()
    r = _req.Request(GEOQA_BASE + "/v1/chat/completions", data=payload, method="POST",
                     headers={"X-API-Key": GEOQA_KEY, "Content-Type": "application/json"})
    t0 = time.time()
    try:
        with _req.urlopen(r, timeout=120) as resp:
            data = json.loads(resp.read())
    except _err.HTTPError as e:
        return {"error": f"geo.qa serving {e.code}: {e.read().decode(errors='replace')[:200]}", "signed": False}
    except Exception as e:  # noqa: BLE001
        return {"error": f"geo.qa serving unavailable: {e}", "signed": False}
    text = ((data.get("choices") or [{}])[0].get("message", {}) or {}).get("content", "").strip()
    return {
        "explanation": text,
        "signed": False,
        "disclaimer": "Generated by the geo.qa LLM stack from emem's already-signed facts. "
        "This prose is NOT signed and is not a fact — verify the emem receipt "
        "(fact_cids / signature) for the ground truth it rewords.",
        "model": GEOQA_MODEL,
        "via": "geoqa /v1/chat/completions",
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
            self._send(200, {"ok": True, "model": GEOQA_MODEL, "backend": "geoqa",
                             "geoqa_base": GEOQA_BASE, "key_configured": bool(GEOQA_KEY), "signed_output": False})
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
    print(f"[explain] thin client -> geo.qa {GEOQA_BASE} (model {GEOQA_MODEL}); no local GPU", flush=True)
    srv = ThreadingHTTPServer((host, int(port)), H)
    print(f"[explain] serving on http://{BIND}  (POST /explain, GET /health)", flush=True)
    srv.serve_forever()
