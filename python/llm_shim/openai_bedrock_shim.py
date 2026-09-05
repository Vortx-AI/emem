#!/usr/bin/env python3
"""OpenAI-shaped /v1/chat/completions on 127.0.0.1:5014, served by Bedrock.

Why a shim rather than three edits
----------------------------------
Three separate consumers already agree on one contract and one port:

* ``navigatable_worlds/gemma_bridge.py`` -- ``GEMMA_URL``, default
  ``http://127.0.0.1:5014/v1/chat/completions``
* ``emem/scripts/agent_ack.py``          -- ``EMEM_A2A_LLM_URL``, same default
* the explain sidecar's Gemma path       -- ``EMEM_EXPLAIN_GEMMA_BASE``, :5014

That port used to be a GPU llm-serve holding gemma-4-12B. There is no GPU on
this box and there is not going to be one, so the choice is to rewrite three
callers for a new backend, or to keep the contract and change what answers on
the port. Keeping the contract is strictly better, for a reason that outlives
this migration:

    llama.cpp's own ``llama-server`` speaks this exact API.

So :5014 is a seam, not a workaround. Bedrock answers it today; if hosted
inference ever turns out cheaper or a credential becomes inconvenient, a
``llama-server`` on the same port replaces this process with no caller change
and no redeploy. The expensive decision stays reversible, which is the point
while the real call volume is still unknown.

Translation is small because Converse is close to the OpenAI shape: a system
list instead of a system-role message, ``maxTokens`` instead of ``max_tokens``,
and content as ``[{"text": ...}]`` blocks. Streaming is deliberately not
implemented -- no caller here uses it, and a fake stream would be a lie about
latency. ``stream: true`` is answered non-streamed, which every one of these
callers already tolerates because they read ``choices[0].message.content``.

Env: LLM_SHIM_BIND (127.0.0.1:5014), LLM_SHIM_MODEL (amazon.nova-micro-v1:0),
     LLM_SHIM_REGION / AWS_REGION (us-east-1), LLM_SHIM_MAX_TOKENS (800).
"""
import json
import os
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

BIND = os.environ.get("LLM_SHIM_BIND", "127.0.0.1:5014")
MODEL = os.environ.get("LLM_SHIM_MODEL", "amazon.nova-micro-v1:0")
REGION = os.environ.get("LLM_SHIM_REGION", os.environ.get("AWS_REGION", "us-east-1"))
MAX_TOKENS = int(os.environ.get("LLM_SHIM_MAX_TOKENS", "800"))

_client = None


def _bedrock():
    global _client
    if _client is None:
        import boto3
        _client = boto3.client("bedrock-runtime", region_name=REGION)
    return _client


def _to_converse(msgs):
    """OpenAI messages -> (system blocks, converse messages).

    Consecutive same-role turns are merged: Converse rejects two user messages
    in a row, and callers that prepend context do produce them.
    """
    system, out = [], []
    for m in msgs or []:
        role = m.get("role")
        content = m.get("content")
        if isinstance(content, list):  # already-blocked content
            content = "".join(b.get("text", "") for b in content if isinstance(b, dict))
        content = (content or "").strip()
        if not content:
            continue
        if role == "system":
            system.append({"text": content})
        else:
            role = "assistant" if role == "assistant" else "user"
            if out and out[-1]["role"] == role:
                out[-1]["content"][0]["text"] += "\n\n" + content
            else:
                out.append({"role": role, "content": [{"text": content}]})
    if not out:
        out = [{"role": "user", "content": [{"text": "."}]}]
    return system, out


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):  # journald already timestamps; keep it quiet
        pass

    def _send(self, code, obj):
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path.rstrip("/") in ("/health", "/v1/health"):
            return self._send(200, {"status": "ok", "backend": "bedrock",
                                    "model": MODEL, "region": REGION, "streaming": False})
        if self.path.rstrip("/") == "/v1/models":
            return self._send(200, {"object": "list", "data": [
                {"id": MODEL, "object": "model", "owned_by": "bedrock"}]})
        self._send(404, {"error": "not found"})

    def do_POST(self):
        if not self.path.rstrip("/").endswith("/chat/completions"):
            return self._send(404, {"error": "not found"})
        try:
            n = int(self.headers.get("Content-Length") or 0)
            req = json.loads(self.rfile.read(n) or b"{}")
        except Exception as e:
            return self._send(400, {"error": {"message": f"bad request: {e}"}})

        # `model` and `base_model` are both accepted: gemma_bridge sends one
        # dialect, the explain sidecar's Cosmos path the other. Either way the
        # caller's model name is advisory -- this process serves LLM_SHIM_MODEL,
        # and says so in the response, rather than silently honouring a name
        # that means nothing to Bedrock.
        system, messages = _to_converse(req.get("messages"))
        cfg = {"maxTokens": int(req.get("max_tokens") or MAX_TOKENS),
               "temperature": float(req.get("temperature", 0.0))}
        t0 = time.time()
        try:
            kw = {"modelId": MODEL, "messages": messages, "inferenceConfig": cfg}
            if system:
                kw["system"] = system
            r = _bedrock().converse(**kw)
        except Exception as e:
            return self._send(502, {"error": {"message": f"bedrock: {type(e).__name__}: {e}",
                                              "type": "upstream_error"}})
        text = "".join(b.get("text", "") for b in
                       r.get("output", {}).get("message", {}).get("content", [])).strip()
        u = r.get("usage", {}) or {}
        self._send(200, {
            "id": f"chatcmpl-{int(t0 * 1000)}",
            "object": "chat.completion",
            "created": int(t0),
            "model": MODEL,
            "choices": [{"index": 0, "finish_reason": r.get("stopReason", "stop"),
                         "message": {"role": "assistant", "content": text}}],
            "usage": {"prompt_tokens": u.get("inputTokens", 0),
                      "completion_tokens": u.get("outputTokens", 0),
                      "total_tokens": u.get("totalTokens", 0)},
            "x_latency_ms": round((time.time() - t0) * 1000),
        })


if __name__ == "__main__":
    host, _, port = BIND.rpartition(":")
    srv = ThreadingHTTPServer((host or "127.0.0.1", int(port)), Handler)
    print(f"[llm-shim] OpenAI /v1/chat/completions on {BIND} -> bedrock {MODEL} ({REGION})",
          file=sys.stderr, flush=True)
    srv.serve_forever()
