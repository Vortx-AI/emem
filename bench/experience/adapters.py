"""Model adapters. ShimAdapter talks to an OpenAI-compatible chat endpoint
(the emem host's Bedrock shim on 127.0.0.1:5015 serves Gemma 3 12B).

The shim does not do native tool calls, so the docs tool is offered through a
one-line text protocol: a reply that is exactly `LOOKUP: dotted.name` is a
call, and the tool's output comes back as the next user turn. Every arm gets
the same protocol, so the comparison stays fair.
"""
from __future__ import annotations

import json
import re
import urllib.request

from arms import ModelReply, ToolCall

LOOKUP_HELP = ("\n\nTo read documentation for the installed library, reply with exactly one line "
               "`LOOKUP: dotted.name` (for example `LOOKUP: pydantic.BaseModel.model_validate`) and "
               "nothing else; the docs come back as the next message. Otherwise reply with the "
               "final file.")
LOOKUP_RE = re.compile(r"^\s*LOOKUP:\s*([A-Za-z_][\w.]*)\s*$")


class ShimAdapter:
    def __init__(self, url: str = "http://127.0.0.1:5015/v1/chat/completions",
                 model: str = "google.gemma-3-12b-it", temperature: float = 0.3,
                 max_tokens: int = 800, timeout: float = 120.0):
        # temperature 0.3 on purpose: the shim caches replies at 0.2 and below,
        # and a cache hit would report a cost the model never paid.
        self.url, self.model_id = url, model
        self.temperature, self.max_tokens, self.timeout = temperature, max_tokens, timeout

    def complete(self, system: str, messages: list, tools: list) -> ModelReply:
        msgs = [{"role": "system", "content": system + (LOOKUP_HELP if tools else "")}]
        for m in messages:
            if m["role"] == "tool":
                msgs.append({"role": "user", "content": "Docs:\n" + m["content"]})
            else:
                msgs.append({"role": m["role"], "content": m["content"]})
        body = json.dumps({"model": self.model_id, "temperature": self.temperature,
                           "max_tokens": self.max_tokens, "messages": msgs}).encode()
        req = urllib.request.Request(self.url, data=body, headers={"content-type": "application/json"})
        with urllib.request.urlopen(req, timeout=self.timeout) as f:
            out = json.load(f)
        text = out["choices"][0]["message"]["content"] or ""
        usage = out.get("usage") or {}
        calls = []
        m = LOOKUP_RE.match(text)
        if tools and m:
            calls = [ToolCall("docs_lookup", {"query": m.group(1)})]
        return ModelReply(text=text, tool_calls=calls,
                          input_tokens=int(usage.get("prompt_tokens", 0)),
                          output_tokens=int(usage.get("completion_tokens", 0)))
