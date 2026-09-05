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
import pathlib
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib import request as _req

BIND = os.environ.get("LLM_SHIM_BIND", "127.0.0.1:5014")
MODEL = os.environ.get("LLM_SHIM_MODEL", "amazon.nova-micro-v1:0")
REGION = os.environ.get("LLM_SHIM_REGION", os.environ.get("AWS_REGION", "us-east-1"))
MAX_TOKENS = int(os.environ.get("LLM_SHIM_MAX_TOKENS", "800"))

_client = None

# ── Daily spend cap ────────────────────────────────────────────────────────
# A hard ceiling enforced here rather than by AWS Budgets, because Budgets
# ALERT and do not stop: they email you after the money is gone. This refuses
# before the call is made.
#
# Hitting the cap must not break anything. It falls back to the local model on
# LLM_SHIM_FALLBACK_URL -- slower, free, and already running for the explain
# sidecar -- so the ceiling costs latency, never availability.
#
# The counter is persisted because a process restart is not a new day, and an
# in-memory tally would silently reset the budget on every deploy. The day
# boundary is UTC so it does not move twice a year.
DAILY_USD_CAP = float(os.environ.get("LLM_SHIM_DAILY_USD", "10"))
SPEND_FILE = pathlib.Path(os.environ.get(
    "LLM_SHIM_SPEND_FILE", "/home/ubuntu/emem/var/emem/llm_shim_spend.json"))
# Nova Micro, USD per million tokens. Overridable because a price is a fact
# about a vendor's pricing page, not about this code.
USD_PER_MTOK_IN = float(os.environ.get("LLM_SHIM_USD_IN", "0.035"))
USD_PER_MTOK_OUT = float(os.environ.get("LLM_SHIM_USD_OUT", "0.14"))
FALLBACK_URL = os.environ.get("LLM_SHIM_FALLBACK_URL", "")

_spend_lock = threading.Lock()


def _today() -> str:
    return time.strftime("%Y-%m-%d", time.gmtime())


def _spend_read() -> dict:
    try:
        d = json.loads(SPEND_FILE.read_text())
        if d.get("day") == _today():
            return d
    except Exception:  # noqa: BLE001 - an unreadable ledger must not bill twice
        pass
    return {"day": _today(), "usd": 0.0, "calls": 0, "in": 0, "out": 0}


def _spend_add(tok_in: int, tok_out: int) -> dict:
    usd = (tok_in * USD_PER_MTOK_IN + tok_out * USD_PER_MTOK_OUT) / 1_000_000
    with _spend_lock:
        d = _spend_read()
        d["usd"] += usd; d["calls"] += 1; d["in"] += tok_in; d["out"] += tok_out
        try:
            SPEND_FILE.parent.mkdir(parents=True, exist_ok=True)
            tmp = SPEND_FILE.with_suffix(".tmp")
            tmp.write_text(json.dumps(d))
            tmp.replace(SPEND_FILE)
        except Exception:  # noqa: BLE001 - a ledger that cannot write must not 500
            pass
        return d


def _over_cap() -> bool:
    return _spend_read()["usd"] >= DAILY_USD_CAP


def _fallback(req: dict):
    """Serve from the local model when the cap is spent. Returns (status, body)."""
    if not FALLBACK_URL:
        return 503, {"error": {"message": f"daily cap ${DAILY_USD_CAP:.2f} reached "
                     f"and no LLM_SHIM_FALLBACK_URL is set", "type": "budget_exhausted"}}
    try:
        r = _req.Request(FALLBACK_URL, data=json.dumps(req).encode(), method="POST",
                         headers={"Content-Type": "application/json"})
        with _req.urlopen(r, timeout=600) as resp:
            body = json.loads(resp.read())
        body["x_budget"] = (f"daily cap ${DAILY_USD_CAP:.2f} reached; served by the "
                            f"local model instead. Slower, not broken.")
        return 200, body
    except Exception as e:  # noqa: BLE001
        return 502, {"error": {"message": f"cap reached and fallback failed: {e}",
                               "type": "budget_exhausted"}}


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
            sp = _spend_read()
            return self._send(200, {
                "status": "ok", "backend": "bedrock", "model": MODEL,
                "region": REGION, "streaming": False,
                "budget": {"day": sp["day"], "cap_usd": DAILY_USD_CAP,
                           "spent_usd": round(sp["usd"], 6),
                           "remaining_usd": round(max(0.0, DAILY_USD_CAP - sp["usd"]), 6),
                           "calls": sp["calls"], "over_cap": _over_cap(),
                           "fallback": FALLBACK_URL or None}})
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
        # Checked BEFORE the call: over the cap this never reaches Bedrock.
        if _over_cap():
            code, body = _fallback(req)
            return self._send(code, body)

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
            # Fall back on ANY Bedrock failure, not just the budget cap. The
            # failure this is really for is credential expiry: the last token
            # was a 12-hour one, and the shape of that outage is every caller
            # getting a 502 at once for a reason none of them can fix. A slower
            # local answer is strictly better than that.
            code, body = _fallback(req)
            if code == 200:
                body["x_budget"] = (f"bedrock unavailable ({type(e).__name__}); "
                                    f"served by the local model instead")
                return self._send(200, body)
            return self._send(502, {"error": {"message": f"bedrock: {type(e).__name__}: {e}; "
                                              f"fallback also failed", "type": "upstream_error"}})
        text = "".join(b.get("text", "") for b in
                       r.get("output", {}).get("message", {}).get("content", [])).strip()
        u = r.get("usage", {}) or {}
        sp = _spend_add(int(u.get("inputTokens") or 0), int(u.get("outputTokens") or 0))
        self._send(200, {
            "x_budget_spent_usd": round(sp["usd"], 6),
            "x_budget_cap_usd": DAILY_USD_CAP,
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
