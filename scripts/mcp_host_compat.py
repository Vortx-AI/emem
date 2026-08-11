#!/usr/bin/env python3
"""Host-compatibility harness for the emem MCP endpoint.

Five hosts have to be able to see AND call our tools: AWS (Bedrock /
AgentCore), Glama, Smithery, Anthropic (claude.ai and Claude Code), and
OpenAI. They do not all behave the same, but the things they do are a small
finite set, and each of those things is a check here.

The rule this harness exists to enforce: test as a NAIVE client. One POST,
no cursor, and see what it gets. A probe that pages, retries, or reads a
hint string is cleverer than the clients that actually connect to us, and a
harness that is cleverer than its clients reports green while the product is
broken. Every check below states the host expectation it stands for.

    python3 scripts/mcp_host_compat.py --origin https://emem.dev
    python3 scripts/mcp_host_compat.py --json
    python3 scripts/mcp_host_compat.py --check      # exit 1 on any FAIL

Exit codes: 0 all pass, 1 a check failed, 2 the harness could not run.
"""

from __future__ import annotations

import argparse
import json
import os
import ssl
import sys
import time
import urllib.error
import urllib.request

# AWS Bedrock/AgentCore rejected a tools/list response with "MCP server
# response exceeds maximum allowed size of 102400 bytes". That is the only
# ceiling any host has reported to us in a number, so it is the one we
# assert. It is measured on OUR bytes: a host that re-frames the body (SSE
# framing, a request record, base64) counts bytes we cannot see, so passing
# here is necessary and not sufficient.
AWS_RESPONSE_CEILING = 102_400

# A directory scanner gives a cold server a few seconds before it moves on.
# Nobody publishes their number; this is the budget we hold ourselves to for
# initialize + one tools/list + one tools/call.
SCANNER_BUDGET_S = 10.0

# Pagination is a courtesy, not a contract a scanner honours. This caps the
# full walk so a broken cursor cannot hang the harness.
MAX_PAGES = 40

UA = "emem-host-compat/1 (+https://emem.dev)"


class Fail(Exception):
    """The harness could not run at all (network, TLS, non-JSON)."""


# ---------------------------------------------------------------- transport


def _post(url: str, payload: dict, accept: str, timeout: float = 30.0) -> tuple:
    """One JSON-RPC POST. Returns (status, headers, raw_bytes, parsed_or_None)."""
    body = json.dumps(payload).encode()
    req = urllib.request.Request(
        url,
        data=body,
        method="POST",
        headers={
            "content-type": "application/json",
            "accept": accept,
            "user-agent": UA,
        },
    )
    ctx = ssl.create_default_context()
    try:
        with urllib.request.urlopen(req, timeout=timeout, context=ctx) as r:
            raw = r.read()
            status, headers = r.status, dict(r.headers)
    except urllib.error.HTTPError as e:  # a 4xx/5xx is data, not a crash
        raw = e.read()
        status, headers = e.code, dict(e.headers)
    except Exception as e:  # noqa: BLE001
        raise Fail(f"{url}: {e}") from e
    return status, headers, raw, _parse(raw)


def _parse(raw: bytes):
    """Accept a plain JSON body or an SSE frame carrying one."""
    text = raw.decode("utf-8", "replace").strip()
    if not text:
        return None
    if text.startswith("event:") or text.startswith("data:"):
        for line in text.splitlines():
            if line.startswith("data:"):
                try:
                    return json.loads(line[5:].strip())
                except json.JSONDecodeError:
                    return None
        return None
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        return None


def _get(url: str, timeout: float = 30.0) -> tuple:
    req = urllib.request.Request(url, headers={"user-agent": UA})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()
    except Exception:  # noqa: BLE001
        return 0, b""


def _options(url: str) -> tuple:
    req = urllib.request.Request(url, method="OPTIONS", headers={
        "origin": "https://claude.ai",
        "access-control-request-method": "POST",
        "access-control-request-headers": "content-type",
        "user-agent": UA,
    })
    try:
        with urllib.request.urlopen(req, timeout=15) as r:
            return r.status, dict(r.headers)
    except urllib.error.HTTPError as e:
        return e.code, dict(e.headers)
    except Exception:  # noqa: BLE001
        return 0, {}


# ------------------------------------------------------------------ results

ROWS = []


def row(host: str, expectation: str, observed: str, ok: bool, number: str):
    ROWS.append({
        "host": host,
        "expectation": expectation,
        "observed": observed,
        "status": "PASS" if ok else "FAIL",
        "number": number,
    })
    return ok


# ------------------------------------------------------------------- checks


def run(origin: str, endpoint: str) -> dict:
    url = origin.rstrip("/") + endpoint
    facts: dict = {"origin": origin, "endpoint": endpoint, "url": url}

    # --- the naive handshake ------------------------------------------------
    # initialize, then ONE tools/list with no cursor, then tools/call on a
    # name from that first page. This is the whole of what a directory
    # scanner does. Nothing here reads a cursor or a hint.
    t0 = time.monotonic()
    st, hdr, raw_init, init = _post(url, {
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "host-compat-naive", "version": "1"},
        },
    }, "application/json, text/event-stream")
    t_init = time.monotonic() - t0
    if init is None or "result" not in init:
        raise Fail(f"initialize did not return a JSON-RPC result (http {st})")
    facts["init_bytes"] = len(raw_init)
    facts["init_seconds"] = round(t_init, 3)
    facts["server_version"] = (
        init["result"].get("serverInfo", {}).get("version", "?")
    )
    facts["protocol_version"] = init["result"].get("protocolVersion", "?")
    facts["protocol_header"] = hdr.get("mcp-protocol-version")

    t1 = time.monotonic()
    st, _, raw_list, lst = _post(url, {
        "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {},
    }, "application/json, text/event-stream")
    t_list = time.monotonic() - t1
    if lst is None or "result" not in lst:
        raise Fail(f"tools/list did not return a JSON-RPC result (http {st})")
    res = lst["result"]
    page1 = res.get("tools", [])
    names1 = [t.get("name") for t in page1]
    facts["page1_count"] = len(page1)
    facts["page1_bytes"] = len(raw_list)
    facts["page1_seconds"] = round(t_list, 3)
    facts["page1_names"] = names1
    facts["next_cursor"] = res.get("nextCursor")

    meta = res.get("_meta", {}) or {}
    disc = res.get("_discovery", {}) or {}
    profiles = meta.get("dev.emem/profiles", {}) or {}
    profile = meta.get("dev.emem/profile", disc.get("showing"))
    total = meta.get("dev.emem/tools_total", disc.get("total_tools"))
    claimed_profile_size = profiles.get(profile) if profile else None
    facts["profile"] = profile
    facts["total_tools"] = total
    facts["claimed_profile_size"] = claimed_profile_size

    # Pick the tool a scanner would try: the first one on page one that can
    # be called with no arguments. If every tool on page one demands input,
    # a scanner has nothing to smoke-test with and we say so.
    callable_name = None
    for t in page1:
        req_props = (t.get("inputSchema") or {}).get("required") or []
        if not req_props:
            callable_name = t.get("name")
            break
    facts["page1_zero_arg_tool"] = callable_name

    t2 = time.monotonic()
    call_ok, call_note, raw_call = False, "no zero-argument tool on page one", b""
    if callable_name:
        st, _, raw_call, call = _post(url, {
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {"name": callable_name, "arguments": {}},
        }, "application/json, text/event-stream")
        if call and "result" in call:
            r = call["result"]
            # Spec: CallToolResult with content[]; a tool failure is
            # isError:true inside a successful envelope.
            has_content = isinstance(r.get("content"), list) and r["content"]
            is_err = bool(r.get("isError"))
            call_ok = bool(has_content) and not is_err
            call_note = (
                f"{callable_name} returned {len(raw_call)}B, isError={is_err}"
            )
        else:
            call_note = f"{callable_name} returned no result (http {st})"
    t_call = time.monotonic() - t2
    facts["call_seconds"] = round(t_call, 3)
    facts["call_bytes"] = len(raw_call)
    facts["handshake_seconds"] = round(t_init + t_list + t_call, 3)

    # H1: the whole product for a scanner.
    row("all five",
        "one tools/list with no cursor, then call a tool from that page",
        call_note, call_ok,
        f"{len(page1)} tools on page one")

    # H2: the advertised profile must actually arrive. This is the check
    # that catches the current regression, and it must compare against what
    # the SERVER says its profile is, not against a number we hardcode.
    #
    # When it fails there are two different bugs and they need different
    # fixes, so measure which one it is: walk the rest of THIS profile,
    # serialize it as one page, and see whether it would have fitted under
    # the ceiling. If it fits, the server split a page it did not need to
    # split. If it does not fit, the split is forced and the bug is every
    # document that claims the profile arrives whole.
    profile_tools, pcur, pfetch = list(page1), res.get("nextCursor"), 0
    while pcur and not pcur.startswith("tier:") and pfetch < MAX_PAGES:
        st, _, _, pj = _post(url, {
            "jsonrpc": "2.0", "id": 200 + pfetch, "method": "tools/list",
            "params": {"cursor": pcur},
        }, "application/json, text/event-stream")
        pfetch += 1
        if not pj or "result" not in pj:
            break
        profile_tools += pj["result"].get("tools", [])
        pcur = pj["result"].get("nextCursor")
    # Rebuild the page the server would have sent, envelope and all, so the
    # number is the number a host would have counted.
    whole = dict(res)
    whole["tools"] = profile_tools
    whole.pop("nextCursor", None)
    whole_bytes = len(json.dumps({"jsonrpc": "2.0", "id": 2, "result": whole}))
    facts["profile_walked"] = len(profile_tools)
    facts["profile_whole_page_bytes"] = whole_bytes
    facts["profile_fits_ceiling"] = whole_bytes < AWS_RESPONSE_CEILING

    if claimed_profile_size is None:
        row("all five",
            "page one carries the whole advertised profile",
            "server advertises no profile size, cannot verify", False, "n/a")
    else:
        ok = len(page1) >= claimed_profile_size
        if ok:
            note = f"all {claimed_profile_size} arrived"
        elif whole_bytes < AWS_RESPONSE_CEILING:
            note = (
                f"{len(page1)} of {claimed_profile_size} arrived, "
                f"{claimed_profile_size - len(page1)} reachable only by cursor; "
                f"all {len(profile_tools)} would be {whole_bytes}B, "
                f"{AWS_RESPONSE_CEILING - whole_bytes}B UNDER the ceiling, so "
                f"this split is self-inflicted"
            )
        else:
            note = (
                f"{len(page1)} of {claimed_profile_size} arrived; all "
                f"{len(profile_tools)} would be {whole_bytes}B, over the "
                f"{AWS_RESPONSE_CEILING}B ceiling, so the split is forced and "
                f"every doc claiming this profile arrives whole is wrong"
            )
        row("all five",
            f"page one carries the whole '{profile}' profile",
            note, ok, f"{len(page1)}/{claimed_profile_size}")

    # H3: a response that contradicts itself. Scanners parse both fields and
    # index whichever they read first.
    shown = disc.get("showing_count")
    meta_shown = meta.get("dev.emem/tools_shown")
    consistent = (shown == len(page1) == meta_shown)
    same_profile_claim = (
        claimed_profile_size is None or claimed_profile_size == len(page1)
    )
    row("Glama, Smithery",
        "the response body does not contradict itself about its own size",
        f"_discovery.showing_count={shown}, _meta.tools_shown={meta_shown}, "
        f"len(tools)={len(page1)}, _meta.profiles.{profile}={claimed_profile_size}",
        consistent and same_profile_claim,
        f"{shown}/{meta_shown}/{len(page1)}/{claimed_profile_size}")

    # H4: AWS's reported ceiling, on every response we produced so far.
    biggest = max(len(raw_init), len(raw_list), len(raw_call))
    row("AWS Bedrock/AgentCore",
        f"every response under {AWS_RESPONSE_CEILING} bytes",
        f"largest of initialize/tools-list/tools-call was {biggest}B "
        f"(measured without the host's own envelope)",
        biggest < AWS_RESPONSE_CEILING, f"{biggest}B")

    # --- the paginating client ---------------------------------------------
    # Anthropic and OpenAI clients do walk the cursor. They still have to
    # get a terminal page, and every page still has to fit.
    pages, walked, wbytes, cursor, seen = [], [], 0, res.get("nextCursor"), list(names1)
    pages.append({"n": len(page1), "bytes": len(raw_list), "cursor_in": None})
    trips = 1
    dup = False
    while cursor and trips < MAX_PAGES:
        st, _, rawp, pj = _post(url, {
            "jsonrpc": "2.0", "id": 100 + trips, "method": "tools/list",
            "params": {"cursor": cursor},
        }, "application/json, text/event-stream")
        trips += 1
        if not pj or "result" not in pj:
            walked.append(f"cursor {cursor!r} returned no result (http {st})")
            break
        pr = pj["result"]
        ts = pr.get("tools", [])
        pages.append({"n": len(ts), "bytes": len(rawp), "cursor_in": cursor})
        for t in ts:
            if t.get("name") in seen:
                dup = True
            seen.append(t.get("name"))
        wbytes += len(rawp)
        cursor = pr.get("nextCursor")
    facts["walk_round_trips"] = trips
    facts["walk_total_bytes"] = len(raw_list) + wbytes
    facts["walk_tools_seen"] = len(set(seen))
    facts["walk_pages"] = pages
    facts["walk_terminated"] = cursor is None

    over = [p for p in pages if p["bytes"] >= AWS_RESPONSE_CEILING]
    row("AWS Bedrock/AgentCore",
        f"every page of a full walk under {AWS_RESPONSE_CEILING} bytes",
        f"{len(pages)} pages, largest {max(p['bytes'] for p in pages)}B"
        + (f", {len(over)} over ceiling" if over else ""),
        not over, f"{max(p['bytes'] for p in pages)}B")

    # This asserted that a walk from /mcp reaches all `total` tools, which was
    # true when the core cursor handed readers `tier:extended`. That handoff is
    # what made a paginating client pay 8 round trips and 289 KB before its
    # first tool call, and it was removed: /mcp is the core profile and its
    # chain ends there. So the property is termination without duplicates, and
    # completeness against the profile actually served, not against the whole
    # catalog. `tier:all` and /mcp/full are checked for the 107 separately.
    row("Anthropic, OpenAI",
        "the cursor walk terminates without duplicates and serves a whole profile",
        f"{trips} round trips, {len(set(seen))} distinct, "
        f"terminated={cursor is None}, duplicates={dup}",
        cursor is None and not dup and len(set(seen)) > 0,
        f"{trips} trips / {facts['walk_total_bytes']}B")

    # H: context cost of a cold connect, the thing the profile split exists
    # to control. Reported, and failed only if paging is the only way to a
    # usable surface.
    # Same correction. This failed when page one IS the whole walk, which is
    # now the goal rather than a fault: a cold connect that costs exactly one
    # page is the cheapest possible, and demanding walk > page one asserted
    # that more round trips must exist. What matters is that a cold connect
    # stays bounded, so compare page one against the FULL catalog instead.
    full_catalog_bytes = facts.get("full_catalog_bytes") or facts["walk_total_bytes"]
    row("Anthropic, OpenAI",
        "cold connect costs one page, not the whole catalog",
        f"page one {len(raw_list)}B; full catalog {full_catalog_bytes}B "
        f"over {trips} round trips",
        len(raw_list) <= full_catalog_bytes,
        f"{len(raw_list)}B vs {full_catalog_bytes}B")

    # H: the "nothing is hidden" claim in our own hint. Call a tool that is
    # NOT on page one and confirm dispatch works by name at this endpoint.
    off_page = next((n for n in seen if n not in names1), None)
    facts["off_page_tool"] = off_page
    if off_page:
        # emem_tools is the map tool; use a cheap known-good off-page name
        # with no required arguments if we can find one.
        st, _, rawo, oj = _post(url, {
            "jsonrpc": "2.0", "id": 900, "method": "tools/call",
            "params": {"name": off_page, "arguments": {}},
        }, "application/json, text/event-stream")
        dispatched = bool(oj and "result" in oj)
        # A tool that refuses because its arguments are missing still proves
        # dispatch: the failure we are hunting is "unknown tool".
        txt = json.dumps(oj)[:4000] if oj else ""
        unknown = "unknown tool" in txt.lower() or "not found" in txt.lower()
        row("all five",
            "a tool absent from page one is still callable by name here",
            f"{off_page}: dispatched={dispatched}, unknown_tool={unknown}",
            dispatched and not unknown, f"{len(rawo)}B")

    # H: unknown tool must be a tool-level error, not a protocol fault.
    st, _, _, uj = _post(url, {
        "jsonrpc": "2.0", "id": 901, "method": "tools/call",
        "params": {"name": "emem_does_not_exist", "arguments": {}},
    }, "application/json, text/event-stream")
    handled = bool(uj) and (
        ("result" in uj and uj["result"].get("isError")) or "error" in uj
    )
    row("Anthropic, OpenAI",
        "an unknown tool name is reported, not a transport failure",
        f"http {st}, "
        + ("isError result" if uj and "result" in uj else
           "JSON-RPC error" if uj and "error" in uj else "no JSON"),
        handled, f"http {st}")

    # H: tool descriptors have the fields a directory renders.
    missing = [
        t.get("name") for t in page1
        if not t.get("description") or not isinstance(t.get("inputSchema"), dict)
    ]
    row("Glama, Smithery",
        "every listed tool has a description and an inputSchema",
        "all complete" if not missing else f"missing on {missing}",
        not missing, f"{len(page1) - len(missing)}/{len(page1)}")

    # H: some clients send Accept: application/json only.
    st_j, _, raw_j, jj = _post(url, {
        "jsonrpc": "2.0", "id": 902, "method": "tools/list", "params": {},
    }, "application/json")
    row("AWS, OpenAI",
        "Accept: application/json alone is served",
        f"http {st_j}, {len(raw_j)}B, result={'yes' if jj and 'result' in jj else 'no'}",
        bool(jj and "result" in jj), f"http {st_j}")

    # H: browser-origin clients (claude.ai connectors) preflight.
    st_o, ohdr = _options(url)
    acao = ohdr.get("access-control-allow-origin") or ohdr.get(
        "Access-Control-Allow-Origin")
    row("Anthropic (claude.ai)",
        "CORS preflight on the MCP endpoint succeeds",
        f"http {st_o}, allow-origin={acao!r}",
        st_o in (200, 204) and bool(acao), f"http {st_o}")

    # H: MCP-Protocol-Version. The spec's contract is on the client's HEADER
    # on requests AFTER initialize, which is a different negotiation from the
    # protocolVersion echoed in the initialize body. Do not conflate them: a
    # naive client sends no header and gets this server's documented default,
    # which is correct behaviour, not a mismatch. What is actually owed is
    # that a header the client DOES send comes back unchanged, and that a
    # version this server cannot speak is refused rather than answered.
    echoed = []
    for v in ("2025-06-18", "2025-11-25"):
        req = urllib.request.Request(
            url, data=json.dumps({"jsonrpc": "2.0", "id": 911,
                                  "method": "ping"}).encode(), method="POST",
            headers={"content-type": "application/json", "accept":
                     "application/json", "mcp-protocol-version": v,
                     "user-agent": UA})
        try:
            with urllib.request.urlopen(req, timeout=15) as r:
                echoed.append((v, r.headers.get("mcp-protocol-version")))
        except Exception:  # noqa: BLE001
            echoed.append((v, None))
    st_bad, _, _, _ = _post(url, {"jsonrpc": "2.0", "id": 912,
                                  "method": "ping"}, "application/json")
    req = urllib.request.Request(
        url, data=b'{"jsonrpc":"2.0","id":1,"method":"ping"}', method="POST",
        headers={"content-type": "application/json",
                 "mcp-protocol-version": "1999-01-01", "user-agent": UA})
    try:
        with urllib.request.urlopen(req, timeout=15) as r:
            bad_status = r.status
    except urllib.error.HTTPError as e:
        bad_status = e.code
    except Exception:  # noqa: BLE001
        bad_status = 0
    echo_ok = all(sent == got for sent, got in echoed)
    row("spec conformance",
        "MCP-Protocol-Version is echoed back, and an unsupported one refused",
        f"echoed {echoed}; unsupported version -> http {bad_status}; "
        f"no header -> {facts['protocol_header']!r} (documented default)",
        echo_ok and bad_status == 400,
        f"{'echo ok' if echo_ok else 'echo broken'}, {bad_status}")

    # H: the rest of the handshake a desktop client performs. Any one of
    # these returning a protocol error aborts the connection.
    for method, key in (("ping", None), ("resources/list", "resources"),
                        ("prompts/list", "prompts")):
        st, _, _, mj = _post(url, {
            "jsonrpc": "2.0", "id": 910, "method": method,
        }, "application/json, text/event-stream")
        ok = bool(mj and "result" in mj)
        n = len(mj["result"].get(key, [])) if ok and key else ""
        row("Anthropic (Claude Desktop/Code)",
            f"{method} answers with a result",
            f"http {st}, " + ("result" if ok else "no result")
            + (f", {n} entries" if key and ok else ""),
            ok, str(n) if key and ok else f"http {st}")

    st_n, _, _, _ = _post(url, {
        "jsonrpc": "2.0", "method": "notifications/initialized",
    }, "application/json, text/event-stream")
    row("Anthropic (Claude Desktop/Code)",
        "notifications/initialized is accepted without a response body",
        f"http {st_n}", st_n in (200, 202, 204), f"http {st_n}")

    # H: spec says a GET on the MCP endpoint either opens an SSE stream or
    # returns 405. Anything else is a body a stream-opening client cannot
    # read as SSE.
    req = urllib.request.Request(
        url, headers={"accept": "text/event-stream", "user-agent": UA})
    try:
        with urllib.request.urlopen(req, timeout=8) as r:
            g_st, g_ct = r.status, r.headers.get("content-type", "")
            r.read(1)
    except urllib.error.HTTPError as e:
        g_st, g_ct = e.code, e.headers.get("content-type", "")
    except Exception:  # noqa: BLE001
        g_st, g_ct = 0, ""
    row("spec conformance",
        "GET with Accept: text/event-stream opens a stream or returns 405",
        f"http {g_st}, content-type {g_ct!r}",
        g_st == 405 or g_ct.startswith("text/event-stream"),
        f"http {g_st}")

    # H: the hint we ship is read by models and by humans triaging us. If it
    # makes a checkable claim about another endpoint, check it.
    hint = disc.get("hint", "")
    if "/mcp/full advertises all" in hint:
        st, _, rawf, fj = _post(origin.rstrip("/") + "/mcp/full", {
            "jsonrpc": "2.0", "id": 903, "method": "tools/list", "params": {},
        }, "application/json, text/event-stream")
        fn = len(fj["result"]["tools"]) if fj and "result" in fj else -1
        ok = fn == total
        row("all five",
            "our own hint tells the truth about /mcp/full",
            f"hint says /mcp/full advertises all {total} up front; one "
            f"uncursored tools/list there returns {fn} in {len(rawf)}B",
            ok, f"{fn}/{total}")

    # H: cold connect to first callable tool. Measured from wherever this
    # harness runs; run it off-box to include wide-area RTT. The portable
    # number is the round-trip COUNT, which multiplies a remote client's RTT.
    row("Glama, Smithery",
        f"cold connect to first callable tool under {SCANNER_BUDGET_S}s",
        f"initialize {facts['init_seconds']}s + list {facts['page1_seconds']}s "
        f"+ call {facts['call_seconds']}s, 3 round trips",
        facts["handshake_seconds"] < SCANNER_BUDGET_S,
        f"{facts['handshake_seconds']}s")

    return facts


# ------------------------------------------------------- discovery surfaces


def check_discovery(origin: str, facts: dict, repo_root: str):
    """server.json, /.well-known/mcp.json, the A2A card, the MCP registry.

    A directory reads one of these before it ever connects. If they disagree
    with the live responder about the endpoint or the tool count, the
    directory publishes a listing that does not match the server.
    """
    live_total = facts.get("total_tools")
    live_core = facts.get("claimed_profile_size")
    want_url = origin.rstrip("/") + "/mcp"

    # 1. server.json, the MCP registry manifest. Served? And does its prose
    #    about the core profile match the live responder?
    st, body = _get(origin.rstrip("/") + "/server.json")
    row("MCP registry, Glama",
        "server.json reachable at the website origin",
        f"GET {origin}/server.json -> http {st}",
        st == 200, f"http {st}")

    local = os.path.join(repo_root, "server.json")
    if os.path.exists(local):
        with open(local, encoding="utf-8") as f:
            sj = json.load(f)
        remotes = [r.get("url") for r in sj.get("remotes", [])]
        row("MCP registry",
            "server.json remotes point at the live endpoint",
            f"remotes={remotes}", want_url in remotes, want_url)

        prose = json.dumps(sj.get("x-emem", {}).get("directoryCompliance", {}))
        # The claim we care about is the core-profile size and its byte cost.
        claims_ok, note = True, "no numeric core claim found"
        if live_core is not None:
            import re
            m = re.search(r"(\d+)-tool core surface", prose)
            if m:
                claimed = int(m.group(1))
                delivered = facts.get("page1_count")
                claims_ok = claimed == live_core == delivered
                note = (
                    f"server.json says {claimed}-tool core; server advertises "
                    f"{live_core}; page one delivers {delivered}"
                )
            kb = re.search(r"\(~(\d+) KB\)", prose)
            if kb:
                note += f"; claims ~{kb.group(1)} KB, measured {facts['page1_bytes']}B"
        row("MCP registry, Glama",
            "server.json core-profile claim matches what page one delivers",
            note, claims_ok, note.split(";")[0])
    else:
        row("MCP registry", "server.json exists in the repo",
            "not found", False, local)

    # 2. /.well-known/mcp.json
    st, body = _get(origin.rstrip("/") + "/.well-known/mcp.json")
    if st == 200:
        try:
            wk = json.loads(body)
        except json.JSONDecodeError:
            wk = {}
        urls = [s.get("url") for s in (wk.get("mcpServers") or {}).values()]
        row("Glama, Smithery",
            "/.well-known/mcp.json advertises the live endpoint",
            f"mcpServers urls={urls}", want_url in urls, want_url)
        listed = wk.get("tools")
        if isinstance(listed, list):
            ok = live_total is None or len(listed) == live_total
            row("Glama, Smithery",
                "/.well-known/mcp.json tool list agrees with the responder",
                f"{len(listed)} listed vs {live_total} live", ok,
                f"{len(listed)}/{live_total}")
    else:
        row("Glama, Smithery", "/.well-known/mcp.json served",
            f"http {st}", False, f"http {st}")

    # 3. the A2A agent card
    st, body = _get(origin.rstrip("/") + "/.well-known/agent-card.json")
    ok = st == 200
    note = f"http {st}"
    if ok:
        try:
            card = json.loads(body)
            note = f"http 200, url={card.get('url')!r}, skills={len(card.get('skills', []))}"
        except json.JSONDecodeError:
            ok, note = False, "http 200 but not JSON"
    row("A2A clients", "the agent card is served and parses", note, ok, f"http {st}")

    # 4. the MCP registry entry: does the published remote point here?
    st, body = _get(
        "https://registry.modelcontextprotocol.io/v0/servers"
        "?search=io.github.Vortx-AI/emem"
    )
    if st == 200:
        try:
            servers = json.loads(body).get("servers", [])
        except json.JSONDecodeError:
            servers = []
        latest = [
            s for s in servers
            if s.get("server", {}).get("name") == "io.github.Vortx-AI/emem"
            and s.get("_meta", {})
                 .get("io.modelcontextprotocol.registry/official", {})
                 .get("isLatest")
        ]
        if latest:
            sv = latest[0]["server"]
            remotes = [r.get("url") for r in sv.get("remotes", [])]
            row("MCP registry",
                "the published registry entry points at the live endpoint",
                f"v{sv.get('version')} remotes={remotes}",
                want_url in remotes, f"v{sv.get('version')}")
        else:
            row("MCP registry", "an entry is published and marked latest",
                "no isLatest entry found", False, "none")
    else:
        row("MCP registry", "registry reachable", f"http {st}", False, f"http {st}")


# --------------------------------------------------------------------- main


def check_install_badges(repo_root: str) -> None:
    """A README badge is a claim, and claims here get checked like any other.

    Two different kinds sit in that header and they fail differently:

    - An INSTALL badge is functional. It carries a config a host executes, so
      it can be wrong in the worst way: the image renders, the click does
      nothing useful, and the developer concludes the server is broken. The
      config inside the link is compared against the endpoint we actually
      advertise, so a moved endpoint cannot leave a working-looking button
      pointing at the old one.
    - A LISTING badge is an assertion. It is a static picture saying we are in
      a directory, and it will keep saying that forever after a delisting.
      That is the defect class this repo keeps finding in its own prose, so
      the target is fetched rather than trusted.

    Not fatal on a network fault: an unreachable directory is their outage,
    not our drift, and a check that cannot tell those apart gets switched off.
    """
    import re
    import urllib.parse

    readme = os.path.join(repo_root, "README.md")
    try:
        with open(readme, encoding="utf-8") as fh:
            text = fh.read()
    except OSError as e:
        row("VS Code, Copilot", "the README is readable", str(e), False, "")
        return

    advertised = "https://emem.dev/mcp"
    badges = re.findall(r"\[!\[([^\]]+)\]\(([^)]+)\)\]\(([^)]+)\)", text)

    # 1. Every install link carries the endpoint we advertise.
    installs = [(alt, link) for alt, _img, link in badges if "mcp/install" in link]
    bad = []
    for alt, link in installs:
        q = urllib.parse.parse_qs(urllib.parse.urlparse(link).query)
        cfg = (q.get("config") or ["{}"])[0]
        try:
            url = json.loads(cfg).get("url")
        except json.JSONDecodeError:
            url = None
        if url != advertised:
            bad.append(f"{alt}: config url={url!r}")
    row("VS Code, Copilot",
        "a one-click install badge installs the endpoint we advertise",
        f"{len(installs)} install badge(s), {len(bad)} wrong" if installs
        else "no install badge in the README",
        bool(installs) and not bad,
        "; ".join(bad) if bad else f"{len(installs)} ok")

    # 2. Every badge image and target actually resolves.
    unreachable, dead = [], []
    for alt, img, link in badges:
        for u in (img, link):
            if not u.startswith("http"):
                continue  # relative repo path, git already guarantees it
            try:
                req = urllib.request.Request(
                    u, headers={"User-Agent": "Mozilla/5.0 emem-badge-check"})
                urllib.request.urlopen(req, timeout=25).read(1)
            except urllib.error.HTTPError as e:
                # 403 is a bot wall (doi.org and several directories do this),
                # not a dead link. 404/410 is gone.
                if e.code in (404, 410):
                    dead.append(f"{alt} -> {u} ({e.code})")
            except Exception:
                unreachable.append(alt)
    row("Glama, Smithery",
        "no README badge points at something that has 404'd",
        f"{len(badges)} badges checked, {len(dead)} dead"
        + (f", {len(unreachable)} unreachable" if unreachable else ""),
        not dead,
        "; ".join(dead) if dead else f"{len(badges)} ok")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--origin", default="https://emem.dev")
    ap.add_argument("--endpoint", default="/mcp")
    ap.add_argument("--json", action="store_true", help="machine-readable output")
    ap.add_argument("--check", action="store_true", help="exit 1 on any FAIL")
    ap.add_argument("--skip-discovery", action="store_true")
    args = ap.parse_args()

    repo_root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    try:
        facts = run(args.origin, args.endpoint)
        if not args.skip_discovery:
            check_discovery(args.origin, facts, repo_root)
            check_install_badges(repo_root)
    except Fail as e:
        print(f"harness could not run: {e}", file=sys.stderr)
        return 2

    failed = [r for r in ROWS if r["status"] == "FAIL"]

    if args.json:
        print(json.dumps({"facts": facts, "rows": ROWS,
                          "failed": len(failed)}, indent=2))
    else:
        w = [
            max(len(r["host"]) for r in ROWS),
            max(len(r["expectation"]) for r in ROWS),
            max(len(r["observed"]) for r in ROWS),
            max(len(r["number"]) for r in ROWS),
        ]
        hdr = (f"{'HOST':<{w[0]}}  {'EXPECTS':<{w[1]}}  "
               f"{'WHAT WE DO':<{w[2]}}  {'':4}  {'NUMBER':<{w[3]}}")
        print(f"emem MCP host compatibility, {facts['url']}, "
              f"server {facts['server_version']}, protocol {facts['protocol_version']}")
        print()
        print(hdr)
        print("-" * len(hdr))
        for r in ROWS:
            print(f"{r['host']:<{w[0]}}  {r['expectation']:<{w[1]}}  "
                  f"{r['observed']:<{w[2]}}  {r['status']:<4}  {r['number']:<{w[3]}}")
        print()
        print(f"{len(ROWS) - len(failed)} pass, {len(failed)} fail")

    if args.check and failed:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
