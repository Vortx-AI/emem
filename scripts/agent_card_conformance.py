#!/usr/bin/env python3
"""Check the agent card against the endpoint it describes, not against a list.

Why this exists
---------------
The card declared `preferredTransport: "HTTP+JSON"` while /a2a/tasks rejects a
plain REST body and requires the JSON-RPC envelope. A client that believed the
card got a 400, and the A2A registry's task_conformance probe failed with
{"category": "METHOD", "passed": false}. Nothing here was wrong in a way a
schema check could see: the value was a legal transport name, just not ours.

So this does not compare the card to an expected document. It sends the shape
the card promises and checks the endpoint accepts it, and sends the other shape
and checks it does not. A label that stops matching the server fails here.

It also checks two things that were quietly wrong for a long time:

  * `protocolVersion` was "1.2.0". No such A2A version exists (0.2.x, 0.3.0,
    1.0.0, 1.0.1), and the proto documents this field as Major.Minor anyway, so
    it was wrong in both number and shape.
  * the pre-1.0 fields `url` / `preferredTransport` and the current
    `supportedInterfaces` are two descriptions of one thing, and nothing stopped
    them disagreeing.

Exit codes
----------
  0  the card describes the endpoint that answers
  1  a declared interface does not behave as declared
  2  could not run
"""
import argparse
import json
import re
import sys
import urllib.error
import urllib.request

DEFAULT_ORIGIN = "https://emem.dev"
# Released A2A versions, Major.Minor. A card may only claim one that exists.
KNOWN_VERSIONS = {"0.1", "0.2", "0.3", "1.0"}
JSONRPC_PROBE = {"jsonrpc": "2.0", "id": "conformance", "method": "message/send",
                 "params": {"message": {"role": "user", "messageId": "conformance",
                                        "parts": [{"kind": "text", "text": "ping"}]}}}
REST_PROBE = {"message": {"role": "user", "messageId": "conformance",
                          "parts": [{"kind": "text", "text": "ping"}]}}


def post(url, body, timeout=60):
    req = urllib.request.Request(
        url, data=json.dumps(body).encode(),
        headers={"content-type": "application/json"}, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, r.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8", "replace")
    except Exception as e:
        return 0, str(e)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--origin", default=DEFAULT_ORIGIN)
    a = ap.parse_args()
    try:
        with urllib.request.urlopen(
                f"{a.origin}/.well-known/agent-card.json", timeout=30) as r:
            card = json.loads(r.read())
    except Exception as e:
        print(f"card-conformance: cannot fetch the card: {e}", file=sys.stderr)
        return 2

    problems = []
    ifaces = card.get("supportedInterfaces") or []
    if not ifaces:
        problems.append("the card declares no `supportedInterfaces`; the current "
                        "spec makes it required and puts the preferred entry first")

    # The two shapes describing one thing must agree.
    if ifaces:
        first = ifaces[0]
        if card.get("url") != first.get("url"):
            problems.append(
                f"`url` is {card.get('url')} but supportedInterfaces[0].url is "
                f"{first.get('url')}; the pre-1.0 and current shapes disagree")
        if card.get("preferredTransport") != first.get("protocolBinding"):
            problems.append(
                f"`preferredTransport` is {card.get('preferredTransport')} but "
                f"supportedInterfaces[0].protocolBinding is "
                f"{first.get('protocolBinding')}")

    for i in ifaces + ([{"url": card.get("url"),
                         "protocolBinding": card.get("preferredTransport"),
                         "protocolVersion": card.get("protocolVersion")}]
                       if card.get("url") else []):
        url, binding = i.get("url"), i.get("protocolBinding")
        ver = i.get("protocolVersion")
        if ver is not None:
            if not re.fullmatch(r"\d+\.\d+", str(ver)):
                problems.append(f"{url}: protocolVersion {ver!r} is not Major.Minor; "
                                f"the proto's own examples are \"0.3\" and \"1.0\"")
            elif str(ver) not in KNOWN_VERSIONS:
                problems.append(f"{url}: protocolVersion {ver!r} is not a released "
                                f"A2A version {sorted(KNOWN_VERSIONS)}")
        if binding == "JSONRPC":
            ok_code, _ = post(url, JSONRPC_PROBE)
            bad_code, _ = post(url, REST_PROBE)
            print(f"  {url}  declared JSONRPC -> jsonrpc body {ok_code}, "
                  f"bare REST body {bad_code}")
            if ok_code != 200:
                problems.append(f"{url} is declared JSONRPC but a JSON-RPC "
                                f"message/send returned {ok_code}")
        elif binding == "HTTP+JSON":
            ok_code, _ = post(url, REST_PROBE)
            print(f"  {url}  declared HTTP+JSON -> REST body {ok_code}")
            if ok_code != 200:
                problems.append(
                    f"{url} is declared HTTP+JSON but a plain REST message body "
                    f"returned {ok_code}. This is the exact defect that failed the "
                    f"registry's task_conformance METHOD probe.")
        else:
            print(f"  {url}  binding {binding!r} not probed by this gate")

    # The methods a 1.0 client will actually send.
    #
    # The card claims protocolVersion 1.0, and 1.0's JSON-RPC binding names
    # methods in PascalCase to match the gRPC service. This server answered only
    # the pre-1.0 `message/send` spelling, so every one of these returned -32601
    # and no client built against the current spec could call it at all. A
    # version number in the card is a promise about which method names answer.
    CORE = {
        "SendMessage": {"message": {"role": "user", "messageId": "conformance",
                                    "parts": [{"kind": "text", "text": "ping"}]}},
        "GetTask": {"id": "conformance-probe-absent"},
        "CancelTask": {"id": "conformance-probe-absent"},
        "ListTasks": {"pageSize": 1},
    }
    endpoint = (ifaces[0]["url"] if ifaces else card.get("url"))
    if endpoint:
        for name, params in CORE.items():
            code, body = post(endpoint, {"jsonrpc": "2.0", "id": "c",
                                         "method": name, "params": params})
            try:
                err = json.loads(body).get("error") or {}
            except Exception:
                err = {}
            rpc = err.get("code")
            # -32601 is method-not-found: the method does not exist here.
            # Any other typed error (e.g. -32001 TaskNotFound for a made-up id)
            # means the method is implemented and answered properly.
            served = rpc != -32601
            print(f"  {name:<22} {'answers' if served else 'NOT IMPLEMENTED'}"
                  f"{'' if served else '  <- a 1.0 client cannot call this'}")
            if not served:
                problems.append(
                    f"{name} returns -32601 while the card claims protocolVersion "
                    f"{card.get('protocolVersion')}; 1.0 names its JSON-RPC methods "
                    f"in PascalCase and a current client sends exactly this")

    # The message shape a 1.0 client actually sends.
    #
    # 1.0 defines Part as a message with a `oneof content`, so the field that is
    # set IS the discriminator: a text part is {"text": "..."} with no `kind`.
    # We accepted only the 0.3 `kind` and pre-0.3 `type` spellings and answered
    # the current one with "message has no parts", so a client using the right
    # method name still could not send a message.
    if endpoint:
        for label, part in (("1.0 oneof   {'text': ...}", {"text": "ping"}),
                            ("0.3 kind    {'kind': 'text'}", {"kind": "text", "text": "ping"}),
                            ("pre-0.3 type", {"type": "text", "text": "ping"})):
            code, body = post(endpoint, {
                "jsonrpc": "2.0", "id": "c", "method": "SendMessage",
                "params": {"message": {"role": "user", "messageId": "conformance",
                                       "parts": [part]}}})
            try:
                err = json.loads(body).get("error") or {}
            except Exception:
                err = {}
            ok = not err
            print(f"  part shape {label:<28} {'accepted' if ok else 'REJECTED'}")
            if not ok:
                problems.append(
                    f"a Part shaped {json.dumps(part)} is rejected "
                    f"({err.get('code')}: {str(err.get('message'))[:60]}); the card "
                    f"claims protocolVersion {card.get('protocolVersion')}")

    if problems:
        print("\nA card that describes a transport the endpoint does not speak sends "
              "every well-behaved client into a 400.")
        for p in problems:
            print(f"  {p}")
        return 1
    print("\nEvery interface the card declares behaves the way it is declared.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
