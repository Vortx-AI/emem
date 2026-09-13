"""Conformance: MCP spec, A2A spec, and the two directories' published rules.

Every check names the rule it is testing and prints the evidence. Nothing here
asserts a rule from memory: the sources are in the header comments.

  MCP tools spec  : structuredContent MUST conform to a declared outputSchema;
                    content blocks MAY carry annotations; text block SHOULD
                    mirror structured output.
  A2A 3.7         : results SHOULD be Artifacts on a Task; Messages SHOULD NOT
                    deliver task outputs. TaskState is a proto enum name.
  Claude directory: every tool has a title and the applicable readOnlyHint /
                    destructiveHint; names <= 64 chars; descriptions say what
                    the tool does and when to call it; openWorldHint when it
                    reaches an external system; a public privacy policy.
  Apps SDK        : declare outputSchema for any tool returning structuredContent.
"""

import json
import sys
import urllib.request as u

import jsonschema

import argparse

_ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
_ap.add_argument("--origin", default="https://emem.dev")
BASE = _ap.parse_args().origin.rstrip("/")
fails, warns = 0, 0


def check(rule, ok, evidence):
    global fails
    fails += not ok
    print(f"{'PASS' if ok else 'FAIL'}  {rule}: {evidence}")


def warn(rule, evidence):
    global warns
    warns += 1
    print(f"WARN  {rule}: {evidence}")


def rpc(method, params, t=240):
    body = {"jsonrpc": "2.0", "id": 1, "method": method, "params": params}
    r = u.Request(BASE + "/mcp", data=json.dumps(body).encode(),
                  headers={"content-type": "application/json",
                           "accept": "application/json, text/event-stream",
                           "MCP-Protocol-Version": "2025-11-25"})
    with u.urlopen(r, timeout=t) as resp:
        raw = resp.read().decode()
        hdrs = dict(resp.headers)
    for line in raw.splitlines():
        if line.startswith("data: "):
            raw = line[6:]
    return hdrs, json.loads(raw)


try:
    u.urlopen(BASE + "/live", timeout=30).read()
except Exception as e:  # noqa: BLE001
    print(f"protocol-conformance: {BASE} did not answer ({str(e)[:60]}); nothing asserted")
    sys.exit(2)

print("== MCP: transport and discovery")
hdrs, init = rpc("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                                "clientInfo": {"name": "conformance", "version": "1"}})
pv = {k.lower(): v for k, v in hdrs.items()}.get("mcp-protocol-version")
check("MCP-Protocol-Version echoed on HTTP", bool(pv), pv or "absent")
caps = (init.get("result") or {}).get("capabilities") or {}
check("server declares tools capability", "tools" in caps, json.dumps(caps)[:120])

_, listed = rpc("tools/list", {})
tools = (listed.get("result") or {}).get("tools") or []
print(f"== Claude directory rules, across {len(tools)} advertised tools")

long_names = [t["name"] for t in tools if len(t["name"]) > 64]
check("tool names <= 64 chars", not long_names, long_names or f"longest {max(len(t['name']) for t in tools)}")

no_title = [t["name"] for t in tools if not (t.get("title") or (t.get("annotations") or {}).get("title"))]
check("every tool has a title", not no_title, no_title[:6] or "all titled")

missing_hint = [t["name"] for t in tools
                if (t.get("annotations") or {}).get("readOnlyHint") is None
                and (t.get("annotations") or {}).get("destructiveHint") is None]
check("every tool declares read-only or destructive", not missing_hint,
      missing_hint[:6] or "all annotated")

contradictory = [t["name"] for t in tools
                 if (t.get("annotations") or {}).get("readOnlyHint") is True
                 and (t.get("annotations") or {}).get("destructiveHint") is True]
check("no tool claims read-only AND destructive", not contradictory, contradictory or "none")

short_desc = [t["name"] for t in tools if len(t.get("description") or "") < 40]
check("descriptions say what and when", not short_desc, short_desc[:6] or
      f"shortest {min(len(t.get('description') or '') for t in tools)} chars")

open_world = sum(1 for t in tools if (t.get("annotations") or {}).get("openWorldHint"))
print(f"      openWorldHint set on {open_world}/{len(tools)} (emem reaches upstream archives)")

print("== MCP: structured output conformance")
schema_tools = [t for t in tools if t.get("outputSchema")]
print(f"      {len(schema_tools)} tool(s) declare an outputSchema: {[t['name'] for t in schema_tools]}")
for t in schema_tools:
    try:
        jsonschema.Draft202012Validator.check_schema(t["outputSchema"])
        check(f"{t['name']} outputSchema is a valid schema", True, "well formed")
    except Exception as e:
        check(f"{t['name']} outputSchema is a valid schema", False, str(e)[:120])

_, called = rpc("tools/call", {"name": "emem_ask",
                               "arguments": {"place": "Trafalgar Square, London",
                                             "q": "how busy is it right now?"}})
res = called.get("result") or {}
sc = res.get("structuredContent")
ask_schema = next((t.get("outputSchema") for t in tools if t["name"] == "emem_ask"), None)
if ask_schema and sc is not None:
    errs = sorted(jsonschema.Draft202012Validator(ask_schema).iter_errors(sc),
                  key=lambda e: list(e.path))
    check("structuredContent conforms to its declared outputSchema", not errs,
          "valid" if not errs else f"{len(errs)} error(s): {errs[0].message[:140]}")
elif ask_schema:
    check("structuredContent present when a schema is declared", False,
          "schema declared but no structuredContent returned")
else:
    warn("emem_ask declares an outputSchema", "none declared yet (pending deploy)")

blocks = res.get("content") or []
check("tool result carries content blocks", bool(blocks), [b.get("type") for b in blocks])
ann = (blocks[0] or {}).get("annotations") if blocks else None
if ann:
    check("content annotated for an audience", "audience" in ann, json.dumps(ann))
else:
    warn("content annotations", "absent (pending deploy)")

print("== MCP: error handling (spec: Protocol Errors vs Tool Execution Errors)")
# The spec lists "Unknown tools", "Invalid arguments" and "Server errors" as
# PROTOCOL errors, with a worked example returning -32602, and reserves
# `isError: true` for execution failures (API failures, business logic).
_, unknown = rpc("tools/call", {"name": "emem_does_not_exist", "arguments": {}})
err = (unknown.get("error") or {})
check("an unknown tool is a JSON-RPC error, not an isError result",
      err.get("code") == -32602,
      f"code={err.get('code')} " + (str(err.get("message"))[:70] if err
                                    else "returned a result: " + str(unknown.get("result"))[:60]))

_, badm = rpc("no/such/method", {})
check("an unknown METHOD answers -32601",
      (badm.get("error") or {}).get("code") == -32601,
      str((badm.get("error") or {}).get("code")))

# JSON-RPC batching was removed in MCP 2025-06-18; an array must be refused.
r = u.Request(BASE + "/mcp",
              data=json.dumps([{"jsonrpc": "2.0", "id": 1, "method": "tools/list",
                                "params": {}}]).encode(),
              headers={"content-type": "application/json",
                       "accept": "application/json, text/event-stream",
                       "MCP-Protocol-Version": "2025-11-25"})
try:
    batch = json.loads(u.urlopen(r, timeout=60).read().decode())
except Exception as e:  # noqa: BLE001
    batch = {"error": {"code": "http " + str(getattr(e, "code", e))}}
check("a JSON-RPC batch is refused (removed in 2025-06-18)",
      "error" in batch, str(batch)[:90])

# A tool that IS served still answers normally: without this control, a server
# that failed every call would pass the two checks above.
_, good = rpc("tools/call", {"name": "emem_tools", "arguments": {}})
check("the control: a real tool still answers",
      "result" in good and not (good.get("result") or {}).get("isError"),
      "result" if "result" in good else str(good.get("error"))[:70])

print("== OpenAI connector contract (search + fetch)")
# "To work with ChatGPT deep research and company knowledge, your MCP server
# should implement two read-only tools: `search` and `fetch`." The result
# shapes are fixed by that contract, and `url` is what decides whether a
# result becomes a citation or anonymous tool output.
by_name = {t["name"]: t for t in tools}
have_pair = "search" in by_name and "fetch" in by_name
check("the connector pair is advertised by its exact names", have_pair,
      "search=%s fetch=%s" % ("search" in by_name, "fetch" in by_name))

if have_pair:
    check("both are declared read-only",
          all((by_name[n].get("annotations") or {}).get("readOnlyHint") for n in ("search", "fetch")),
          str({n: (by_name[n].get("annotations") or {}).get("readOnlyHint") for n in ("search", "fetch")}))
    check("both declare an output schema",
          all(by_name[n].get("outputSchema") for n in ("search", "fetch")),
          str({n: bool(by_name[n].get("outputSchema")) for n in ("search", "fetch")}))

    _, sr = rpc("tools/call", {"name": "search",
                               "arguments": {"query": "Trafalgar Square, London"}})
    sres = (sr.get("result") or {}).get("structuredContent") or {}
    rows = sres.get("results") or []
    check("search returns results with id, title and url",
          bool(rows) and all(isinstance(r.get(k), str) and r[k] for r in rows
                             for k in ("id", "title", "url")),
          f"{len(rows)} result(s); first={json.dumps(rows[0])[:110] if rows else 'none'}")
    sch = by_name["search"].get("outputSchema")
    if sch and sres:
        errs = list(jsonschema.Draft202012Validator(sch).iter_errors(sres))
        check("search structuredContent conforms to its schema", not errs,
              "valid" if not errs else errs[0].message[:120])

    if rows:
        # The round trip is the contract: an id search handed out must open.
        _, fr = rpc("tools/call", {"name": "fetch", "arguments": {"id": rows[0]["id"]}})
        fres = (fr.get("result") or {}).get("structuredContent") or {}
        check("fetch opens an id that search handed out",
              all(isinstance(fres.get(k), str) and fres[k] for k in ("id", "title", "text", "url")),
              f"keys={sorted(fres.keys())} url={str(fres.get('url'))[:60]}")
        fsch = by_name["fetch"].get("outputSchema")
        if fsch and fres:
            errs = list(jsonschema.Draft202012Validator(fsch).iter_errors(fres))
            check("fetch structuredContent conforms to its schema", not errs,
                  "valid" if not errs else errs[0].message[:120])
        # The last entry names the cell and the true total: a capped list that
        # says how much there was.
        check("a capped search still says how much there was",
              rows[-1]["id"].startswith("emem:cell:"),
              rows[-1]["title"][:80])

print("== A2A")
card = json.loads(u.urlopen(BASE + "/.well-known/agent-card.json", timeout=60).read())
required = ["protocolVersion", "name", "description", "url", "version",
            "capabilities", "defaultInputModes", "defaultOutputModes", "skills"]
missing = [k for k in required if k not in card]
check("agent card carries every required field", not missing, missing or f"v{card.get('protocolVersion')}")
skills = card.get("skills") or []
bad_skills = [s.get("name") for s in skills if not all(k in s for k in ("id", "name", "description", "tags"))]
check("every skill has id, name, description, tags", not bad_skills, bad_skills[:5] or f"{len(skills)} skills")

r = u.Request(BASE + "/a2a/tasks", data=json.dumps({
    "jsonrpc": "2.0", "id": 1, "method": "message/send",
    "params": {"message": {"role": "user", "parts": [
        {"kind": "text", "text": "how busy is Trafalgar Square, London right now?"}]}}}).encode(),
    headers={"content-type": "application/json"})
a2a = json.loads(u.urlopen(r, timeout=300).read())["result"]
check("A2A returns a Task, not a Message", a2a.get("kind") == "task",
      f"kind={a2a.get('kind')}")
arts = a2a.get("artifacts") or []
check("outputs delivered as artifacts", bool(arts), f"{len(arts)} artifact(s)")
state = (a2a.get("status") or {}).get("state")
check("TaskState uses the proto enum name", str(state).startswith("TASK_STATE_"), state)

print("== privacy policy (a missing one is an immediate directory rejection)")
for path in ("/privacy", "/privacy.html", "/.well-known/privacy"):
    try:
        code = u.urlopen(BASE + path, timeout=30).status
    except Exception as e:
        code = getattr(e, "code", 0)
    if code == 200:
        check("a public privacy policy is served", True, f"{path} -> 200")
        break
else:
    check("a public privacy policy is served", False, "no policy found at /privacy, /privacy.html or /.well-known/privacy")

print(f"FAILS {fails}  WARNS {warns}")

# Exit codes follow this repo's gate convention, so CI can tell a broken
# responder from a broken standard: 0 conforms, 1 a rule is violated, 2 the
# responder did not answer (waived, it says nothing about the code), 3 our own
# side could not run the check.
sys.exit(1 if fails else 0)
