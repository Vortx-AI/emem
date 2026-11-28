#!/usr/bin/env python3
"""Send an emem API key to emem, except where a route is open to anyone.

    import emem_auth  # noqa: F401

Every MCP endpoint (/mcp, /mcp/full, /mcp/account) needs a signed-in account,
and so does every route that publishes. On a node that runs
EMEM_REST_REQUIRE_ACCOUNT=1, as emem.dev does, every other REST route needs one
too, except the open routes: discovery, the transparency log, content by its
content id, the verifiers and the website. A check that runs against a node
sets EMEM_API_KEY to a key made with

    emem-server accounts mint-key --name checks --email <email> --scopes "memory.read memory.write publish"

and importing this module adds `Authorization: Bearer $EMEM_API_KEY` to every
request to an emem host made through urllib, except a request to an open
route. Nothing changes when the variable is unset.

The key goes only to emem hosts: the loopback names, emem.dev, and the host
of EMEM_ORIGIN, EMEM_BASE, EMEM_URL or EMEM_RESPONDER when one is set. A check
that also fetches a third-party URL never sends it there.

OPEN mirrors OPEN in crates/emem-api-rest/src/accounts_web/rest_gate.rs,
without the entries that have a gate of their own (MCP, the account pages,
publishing, /ops): those want the key or ignore it. A test there fails when a
path in its list is missing here.
"""

import os
import urllib.parse
import urllib.request

# (methods, route patterns). A pattern that ends in "*" covers every path that
# starts with the text before it; a ":name" segment matches one segment and a
# "*name" segment the rest of the path. "GET" covers HEAD.
OPEN = (
    # Discovery.
    ("ANY", ("/.well-known/*",)),
    ("GET", ("/openapi.json", "/openapi.action.json", "/v1/openapi.action.json")),
    ("GET", ("/v1/discover",)),
    ("GET", ("/v1/agent_card", "/v1/a2a/skills", "/extendedAgentCard",
             "/a2a/tasks/.well-known/*", "/agent.json", "/server.json",
             "/agents.json", "/agents.txt", "/mcp.json", "/gemini-extension.json")),
    ("GET", ("/llms.txt", "/llms-full.txt", "/robots.txt", "/sitemap.xml")),
    ("GET", ("/v1/health", "/health", "/live", "/ping", "/a2a/tasks/health", "/metrics")),
    ("GET", ("/v1/errors", "/v1/deprecations", "/v1/schema", "/v1/schemas",
             "/v1/schemas/:name", "/v1/schemas/eudr_dds.json")),
    ("ANY", ("/v1/models", "/v1/chat/completions")),
    # Verification.
    ("GET", ("/v1/verifier_spec",)),
    ("GET", ("/v1/log/*",)),
    ("POST", ("/v1/log/witness",)),
    ("GET", ("/v1/facts/:cid", "/v1/state/:cid", "/v1/tree/:file_cid",
             "/v1/memory_bundle/:token", "/v1/artifacts/:cid")),
    ("POST", ("/v1/memory_token/resolve", "/v1/memory_token/resolve_many",
              "/v1/raster/resolve", "/v1/cube/resolve", "/v1/raster_bundle/resolve",
              "/v1/trace_resolve")),
    ("POST", ("/v1/verify_receipt", "/v1/trace_verify", "/v1/enroll_verify")),
    # Pack and share links.
    ("GET", ("/p/:token", "/s/:token")),
    # The website.
    ("GET", ("/", "/humans", "/verify", "/verify/:cid", "/emem-verify-core.js",
             "/signin-gate.js", "/scoreboard", "/card", "/a2a", "/guard", "/channel",
             "/channel.json", "/collaboration", "/data/relay-recording.json", "/tools",
             "/demos*", "/arcade*", "/worlds*", "/splats", "/splats/", "/splats/*path",
             "/agents", "/agents.md", "/how-it-works", "/solutions", "/eudr",
             "/agriculture", "/beside-search", "/teams", "/airgap", "/contact",
             "/connect", "/developers", "/the-long-version", "/reference", "/gallery",
             "/api")),
    ("GET", ("/docs*", "/skills*", "/whitepaper*", "/integrations*", "/spec*",
             "/clients*", "/multimodal*", "/examples/*", "/attesting.md", "/privacy*",
             "/terms*", "/support*", "/security*", "/materializers.md", "/spaces.md",
             "/temporal.md")),
    ("GET", ("/img/*", "/art/:name", "/tokens.css", "/nav.css", "/site.css", "/home.js",
             "/demos.css", "/favicon*", "/apple-touch-icon.png", "/icon-192.png",
             "/icon-512.png", "/logo*", "/og-image*", "/release*", "/vortxgola.gif",
             "/video-*",
             "/484b153b1031a5a89d8217c1efbe6fe91313e0b328e94b0f10446c6dbda8b10e.txt")),
    ("GET", ("/log/*", "/modules")),
)


def _hosts():
    hosts = {"localhost", "127.0.0.1", "::1", "emem.dev", "www.emem.dev"}
    for var in ("EMEM_ORIGIN", "EMEM_BASE", "EMEM_URL", "EMEM_RESPONDER"):
        v = os.environ.get(var, "").strip()
        if v:
            h = urllib.parse.urlsplit(v if "://" in v else "http://" + v).hostname
            if h:
                hosts.add(h)
    return hosts


def _covers(pattern, path):
    if pattern.endswith("*"):
        return path.startswith(pattern[:-1])
    want, have = pattern.split("/"), path.split("/")
    for i, seg in enumerate(want):
        if seg.startswith("*"):
            return len(have) > i and have[i] != ""
        if i >= len(have):
            return False
        if seg.startswith(":"):
            if not have[i]:
                return False
        elif seg != have[i]:
            return False
    return len(have) == len(want)


def is_open(path, method="GET"):
    """Whether `method path` answers without an account on any emem node."""
    m = method.upper()
    for methods, patterns in OPEN:
        if methods == "GET" and m not in ("GET", "HEAD"):
            continue
        if methods == "POST" and m != "POST":
            continue
        if any(_covers(p, path) for p in patterns):
            return True
    return False


def wants_key(url, method="GET"):
    """Whether a request to `url` should carry the account key."""
    u = urllib.parse.urlsplit(url)
    return (u.hostname or "") in _hosts() and not is_open(u.path or "/", method)


def headers(url, base=None, method="GET"):
    """`base` plus the Authorization header when `url` needs the key."""
    out = dict(base or {})
    key = os.environ.get("EMEM_API_KEY", "").strip()
    if key and wants_key(url, method) and not any(k.lower() == "authorization" for k in out):
        out["Authorization"] = "Bearer " + key
    return out


class _Auth(urllib.request.BaseHandler):
    handler_order = 100

    def http_request(self, req):
        key = os.environ.get("EMEM_API_KEY", "").strip()
        if (key and not req.has_header("Authorization")
                and wants_key(req.full_url, req.get_method())):
            req.add_unredirected_header("Authorization", "Bearer " + key)
        return req

    https_request = http_request


if os.environ.get("EMEM_API_KEY", "").strip():
    urllib.request.install_opener(urllib.request.build_opener(_Auth()))


if __name__ == "__main__":
    os.environ.setdefault("EMEM_API_KEY", "emk_test")
    # The account routes and the publishing routes.
    assert wants_key("https://emem.dev/mcp")
    assert wants_key("http://127.0.0.1:5051/mcp/full")
    assert wants_key("https://emem.dev/v1/attest_cbor", "POST")
    assert wants_key("https://emem.dev/v1/entity/alias", "POST")
    # Every other read, on a node with REST behind an account.
    for path, method in [
        ("/v1/recall", "POST"), ("/v1/bands", "GET"), ("/v1/locate", "POST"),
        ("/v1/locate?place=Pune", "GET"), ("/v1/ask", "POST"), ("/a2a/tasks", "POST"),
        ("/v1/a2a/tasks/abc", "GET"), ("/v1/message:send", "POST"),
        ("/memories/by_attester/abcd/x.md", "GET"), ("/postcard", "GET"),
        ("/splats/api/gemma", "POST"), ("/v1/tree/path", "POST"),
        ("/v1/entity/abc", "GET"), ("/v1/manifests", "GET"),
        ("/v1/perception/cards", "GET"), ("/verdict/mcp", "POST"), ("/v1/facts", "GET"),
    ]:
        assert wants_key("https://emem.dev" + path, method), (method, path)
    # The open routes.
    for path, method in [
        ("/.well-known/emem.json", "GET"), ("/openapi.json", "GET"), ("/v1/discover", "GET"),
        ("/v1/health", "GET"), ("/v1/log/sth", "GET"), ("/v1/log/inclusion", "GET"),
        ("/v1/log/witness", "POST"), ("/v1/facts/abc", "GET"), ("/v1/tree/abc", "GET"),
        ("/v1/artifacts/abc", "HEAD"), ("/v1/memory_bundle/abc", "GET"),
        ("/v1/memory_token/resolve", "POST"), ("/v1/verify_receipt", "POST"),
        ("/v1/schemas/Fact", "GET"), ("/llms.txt", "GET"), ("/docs/errors.html", "GET"),
        ("/verify/abc", "GET"), ("/splats/viewer/x.html", "GET"), ("/", "GET"),
        ("/skills/emem-urban/SKILL.md", "GET"), ("/v1/chat/completions", "POST"),
    ]:
        assert not wants_key("https://emem.dev" + path, method), (method, path)
    assert not wants_key("https://example.com/v1/recall", "POST")
    assert headers("https://emem.dev/mcp")["Authorization"] == "Bearer emk_test"
    assert "Authorization" not in headers("https://api.github.com/mcp")
    assert "Authorization" not in headers("https://emem.dev/v1/log/sth")
    assert headers("https://emem.dev/v1/recall", method="POST")["Authorization"] == "Bearer emk_test"
    print("emem_auth: ok")
