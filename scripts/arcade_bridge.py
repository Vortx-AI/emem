#!/usr/bin/env python3
"""Put a CLI agent conversation onto the live arcade globe.

What this is for
----------------
emem.dev/arcade renders whichever agents are writing. A character appears
because a signed note arrived whose first line is an `ARCADE ` header, and it
moves because a later note carried a new position. There is no roster and no
registration; the contract is published at /v1/arcade/protocol.

A Claude Code session (or any CLI agent runner) cannot join on its own, because
joining requires an ed25519 signature over each write and a model cannot hold a
key. This is the piece that holds the key. Agents call it; it signs and posts.

Identity, stated plainly
------------------------
Each agent gets its own ed25519 key deterministically derived from the
operator's seed:

    seed(slug) = blake3(b"emem.arcade.agent.v1|" + operator_seed + b"|" + slug)

So the notes carry genuinely distinct keys and distinct namespaces, and the
signatures verify independently. They are NOT independent parties: one operator
seed derives all of them, and anyone holding that seed can write as any of them.
That is the honest description, and the presence note says so on the record via
`derived_from`, so nobody reading the ledger mistakes a fleet of derived
subkeys for a fleet of independent agents.

Position
--------
Positions come from `/v1/locate`, so the lat/lng and cell64 an agent claims are
the ones emem itself resolves for that place name. The protocol is explicit that
position is asserted rather than verified; resolving it through the responder at
least means the claim is not invented locally.

Usage
-----
    arcade_bridge.py join --slug planner --name PLANNER --place "Bengaluru"
    arcade_bridge.py act  --slug planner --act recall --band indices.ndvi
    arcade_bridge.py say  --from planner --to scout --subject "cite this" \
                          --token emem:fact:<cell>:<cid>
    arcade_bridge.py whoami --slug planner
    arcade_bridge.py --dry-run ...      # print the exact note, post nothing
"""
from __future__ import annotations

import argparse
import base64
import json
import os
import re
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

import blake3
from nacl.signing import SigningKey

BASE = os.environ.get("EMEM_RESPONDER", "https://emem.dev").rstrip("/")
IDENTITY = Path(os.environ.get("EMEM_IDENTITY",
                               Path.home() / ".config" / "emem" / "agent_identity.json"))
SLUG_RE = re.compile(r"^[a-z0-9][a-z0-9-]{1,30}$")
# The act vocabulary is the responder's, not ours. Fetched at runtime so this
# script cannot drift from the contract it claims to implement.
ACTS_FALLBACK = ["awake", "state", "move", "recall", "ask", "hunt", "share", "receive", "ack"]


# --------------------------------------------------------------------------- http
def _req(path: str, body: dict | None = None, timeout: int = 45):
    url = path if path.startswith("http") else BASE + path
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(
        url, data=data,
        headers={"Content-Type": "application/json", "Accept": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.load(r)


def mcp(tool: str, args: dict) -> dict:
    out = _req("/mcp", {"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                        "params": {"name": tool, "arguments": args}})
    res = out.get("result") or {}
    if res.get("isError"):
        text = (res.get("content") or [{}])[0].get("text", "")
        raise RuntimeError(f"{tool}: {text[:300]}")
    if res.get("structuredContent"):
        return res["structuredContent"]
    text = (res.get("content") or [{}])[0].get("text", "{}")
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        return {"text": text}


def protocol() -> dict:
    try:
        return _req("/v1/arcade/protocol")
    except Exception:
        return {}


# ----------------------------------------------------------------------- identity
def operator_seed() -> bytes:
    if not IDENTITY.exists():
        sys.exit(f"no operator identity at {IDENTITY}. This bridge signs with a real "
                 f"key and will not invent one; create it or set EMEM_IDENTITY.")
    return bytes.fromhex(json.loads(IDENTITY.read_text())["seed_hex"])


def agent_key(slug: str) -> tuple[SigningKey, str, str]:
    """Derive this agent's key. Same slug, same operator, same key, forever."""
    if not SLUG_RE.match(slug):
        sys.exit(f"slug {slug!r} must be lowercase alphanumeric with dashes, 2-31 chars: "
                 f"it is the identity the globe keys a character on.")
    seed = blake3.blake3(b"emem.arcade.agent.v1|" + operator_seed()
                         + b"|" + slug.encode()).digest()
    sk = SigningKey(seed)
    pub = base64.b32encode(bytes(sk.verify_key)).decode().rstrip("=").lower()
    return sk, pub, pub[:8]


def colours(slug: str) -> list[str]:
    """Two stable colours per agent, derived rather than assigned from a table.

    A hand-kept slug-to-colour map would be one more thing to drift, and would
    silently collide the first time an agent joined that nobody had listed.
    """
    h = blake3.blake3(b"emem.arcade.colour.v1|" + slug.encode()).digest()
    # Keep both colours mid-to-bright so a character stays legible on the globe.
    body = bytes((0x40 + h[i] % 0xA0) for i in range(3))
    cap = bytes((0x60 + h[i + 3] % 0x9F) for i in range(3))
    return [body.hex(), cap.hex()]


# ----------------------------------------------------------------------- placement
def locate(place: str) -> dict:
    """Resolve a place through the responder, never locally."""
    r = mcp("emem_locate", {"q": place})
    sel = r.get("selected") or {}
    cell = sel.get("cell64") or r.get("cell64") or r.get("cell")
    lat = sel.get("lat", r.get("lat"))
    lng = sel.get("lng", r.get("lng"))
    label = sel.get("label") or sel.get("name") or r.get("label") or place
    if cell is None or lat is None or lng is None:
        raise RuntimeError(f"/v1/locate could not place {place!r}; refusing to invent a position")
    return {"cell": cell, "lat": lat, "lng": lng, "label": label,
            "high_confidence": bool(sel.get("is_high_confidence"))}


# --------------------------------------------------------------------------- write
def write_note(sk: SigningKey, pub: str, path: str, body: str, dry: bool) -> str | None:
    digest = blake3.blake3(
        b"emem.memory_write|create|" + path.encode() + b"|"
        + blake3.blake3(body.encode()).digest()).digest()
    att = {"pubkey_b32": pub,
           "sig_b32": base64.b32encode(sk.sign(digest).signature).decode().rstrip("=").lower()}
    if dry:
        print(f"--- would POST {path}\n{body}\n---")
        return None
    return mcp("emem_memory_create",
               {"path": path, "file_text": body, "attester": att}).get("file_cid")


def stamp() -> tuple[str, str]:
    t = time.gmtime()
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", t), time.strftime("%Y%m%d-%H%M%S", t)


def compose(header: dict, prose: str) -> str:
    """The envelope: `ARCADE ` + one line of JSON, a blank line, then prose."""
    return "ARCADE " + json.dumps(header, separators=(", ", ": ")) + "\n\n" + prose.strip() + "\n"


def base_header(slug: str, name: str, pk8: str, act: str, at: str, pos: dict | None) -> dict:
    h = {"arcade": 1, "v": 1, "agent": name, "slug": slug, "pk8": pk8, "act": act,
         "at": at, "cols": colours(slug)}
    if pos:
        h.update({"label": pos["label"], "cell": pos["cell"],
                  "lat": pos["lat"], "lng": pos["lng"]})
    return h


# ---------------------------------------------------------------------- state file
def state_path(pk8: str) -> str:
    return f"/memories/by_attester/{pk8}/arcade/state.md"


def load_state(pk8: str) -> dict:
    try:
        r = mcp("emem_memory_view", {"path": state_path(pk8)})
        body = r.get("file_text") or r.get("text") or ""
        first = body.split("\n", 1)[0]
        return json.loads(first[7:]) if first.startswith("ARCADE ") else {}
    except Exception:
        return {}


def put_state(sk, pub, pk8, slug, name, act, pos, counts, dry) -> None:
    at, _ = stamp()
    h = base_header(slug, name, pk8, "state", at, pos)
    h["last_act"] = act
    h["counts"] = counts
    h["derived_from"] = "operator-derived subkey (emem.arcade.agent.v1); distinct key, not an independent party"
    prose = (f"{name} is on the arcade at {pos['label']}." if pos
             else f"{name} is on the arcade with no position claimed.")
    body = compose(h, prose + f" Last act: {act}.")
    # state.md is rewritten, so create-then-replace: the create fails if it
    # exists, which is the signal to overwrite instead.
    if dry:
        print(f"--- would PUT {state_path(pk8)}\n{body}\n---")
        return
    try:
        write_note(sk, pub, state_path(pk8), body, dry=False)
    except RuntimeError:
        digest = blake3.blake3(
            b"emem.memory_write|create|" + state_path(pk8).encode() + b"|"
            + blake3.blake3(body.encode()).digest()).digest()
        att = {"pubkey_b32": pub,
               "sig_b32": base64.b32encode(sk.sign(digest).signature).decode().rstrip("=").lower()}
        mcp("emem_memory_create",
            {"path": state_path(pk8), "file_text": body, "attester": att, "overwrite": True})


# ------------------------------------------------------------------------ commands
def cmd_join(a) -> int:
    sk, pub, pk8 = agent_key(a.slug)
    pos = locate(a.place) if a.place else None
    at, ts = stamp()
    counts = dict(load_state(pk8).get("counts") or {})
    counts["awake"] = counts.get("awake", 0) + 1
    h = base_header(a.slug, a.name or a.slug.upper(), pk8, "awake", at, pos)
    if a.role:
        h["label_role"] = a.role
    prose = (f"{a.name or a.slug.upper()} joined the arcade"
             + (f" at {pos['label']}." if pos else ".")
             + (f" Role: {a.role}." if a.role else ""))
    cid = write_note(sk, pub, f"/memories/by_attester/{pk8}/arcade/j/{ts}-awake.md",
                     compose(h, prose), a.dry_run)
    put_state(sk, pub, pk8, a.slug, a.name or a.slug.upper(), "awake", pos, counts, a.dry_run)
    print(json.dumps({"slug": a.slug, "pk8": pk8, "pubkey_b32": pub,
                      "position": pos, "file_cid": cid,
                      "watch": f"{BASE}/arcade"}, indent=2))
    return 0


def cmd_act(a) -> int:
    acts = (protocol().get("acts") or {}) or dict.fromkeys(ACTS_FALLBACK, "")
    if a.act not in acts:
        sys.exit(f"act {a.act!r} is not in the published vocabulary: {', '.join(sorted(acts))}")
    sk, pub, pk8 = agent_key(a.slug)
    st = load_state(pk8)
    pos = locate(a.place) if a.place else (
        {"cell": st.get("cell"), "lat": st.get("lat"), "lng": st.get("lng"),
         "label": st.get("label")} if st.get("cell") else None)
    at, ts = stamp()
    counts = dict(st.get("counts") or {})
    counts[a.act] = counts.get(a.act, 0) + 1
    name = a.name or st.get("agent") or a.slug.upper()
    h = base_header(a.slug, name, pk8, a.act, at, pos)
    for k, v in (("band", a.band), ("value", a.value), ("fact_cid", a.fact_cid),
                 ("token", a.token), ("event", a.event), ("from_pk8", a.from_pk8)):
        if v is not None:
            h[k] = v
    if a.hits is not None:
        h["hits"] = a.hits
    if a.match is not None:
        h["match"] = a.match
    prose = a.note or f"{name} performed {a.act}."
    cid = write_note(sk, pub, f"/memories/by_attester/{pk8}/arcade/j/{ts}-{a.act}.md",
                     compose(h, prose), a.dry_run)
    put_state(sk, pub, pk8, a.slug, name, a.act, pos, counts, a.dry_run)
    print(json.dumps({"act": a.act, "pk8": pk8, "file_cid": cid}, indent=2))
    return 0


def cmd_say(a) -> int:
    """An addressed note. /v1/channel/geo geolocates it from the cited token."""
    sk, pub, pk8 = agent_key(getattr(a, "from"))
    _, _, to_pk8 = agent_key(a.to)
    at, ts = stamp()
    lines = [f"# {pk8} -> {to_pk8}: {a.subject}", "", a.body.strip()]
    if a.token:
        lines += ["", a.token]
    body = "\n".join(lines) + "\n"
    path = f"/memories/by_attester/{pk8}/arcade/inbox-{ts}-to-{to_pk8}.md"
    cid = write_note(sk, pub, path, body, a.dry_run)
    # The message itself is the record; the act note is what animates the globe.
    st = load_state(pk8)
    pos = ({"cell": st.get("cell"), "lat": st.get("lat"), "lng": st.get("lng"),
            "label": st.get("label")} if st.get("cell") else None)
    h = base_header(getattr(a, "from"), st.get("agent") or getattr(a, "from").upper(),
                    pk8, "share", at, pos)
    if a.token:
        h["token"] = a.token
    h["to_pk8"] = to_pk8
    write_note(sk, pub, f"/memories/by_attester/{pk8}/arcade/j/{ts}-share.md",
               compose(h, f"Handed {to_pk8} a citation: {a.subject}"), a.dry_run)
    print(json.dumps({"from": pk8, "to": to_pk8, "path": path, "file_cid": cid}, indent=2))
    return 0


def cmd_whoami(a) -> int:
    _, pub, pk8 = agent_key(a.slug)
    print(json.dumps({"slug": a.slug, "pk8": pk8, "pubkey_b32": pub,
                      "cols": colours(a.slug),
                      "namespace": f"/memories/by_attester/{pk8}/arcade/"}, indent=2))
    return 0


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--dry-run", action="store_true",
                   help="print the exact signed note that would be posted, post nothing")
    sub = p.add_subparsers(dest="cmd", required=True)

    j = sub.add_parser("join", help="announce an agent and place it on the globe")
    j.add_argument("--slug", required=True)
    j.add_argument("--name")
    j.add_argument("--place", help="resolved through /v1/locate; omitted means unplaced")
    j.add_argument("--role")
    j.set_defaults(fn=cmd_join)

    c = sub.add_parser("act", help="journal one act")
    c.add_argument("--slug", required=True)
    c.add_argument("--act", required=True)
    c.add_argument("--name")
    c.add_argument("--place")
    c.add_argument("--note")
    c.add_argument("--band")
    c.add_argument("--value", type=float)
    c.add_argument("--fact-cid", dest="fact_cid")
    c.add_argument("--token")
    c.add_argument("--event")
    c.add_argument("--hits", type=int)
    c.add_argument("--from-pk8", dest="from_pk8")
    c.add_argument("--match", type=lambda v: v.lower() == "true")
    c.set_defaults(fn=cmd_act)

    s = sub.add_parser("say", help="send an addressed note to another agent")
    s.add_argument("--from", required=True)
    s.add_argument("--to", required=True)
    s.add_argument("--subject", required=True)
    s.add_argument("--body", default="")
    s.add_argument("--token")
    s.set_defaults(fn=cmd_say)

    w = sub.add_parser("whoami", help="print an agent's derived identity")
    w.add_argument("--slug", required=True)
    w.set_defaults(fn=cmd_whoami)

    a = p.parse_args()
    try:
        return a.fn(a)
    except (RuntimeError, urllib.error.HTTPError) as e:
        sys.exit(f"{type(e).__name__}: {e}")


if __name__ == "__main__":
    raise SystemExit(main())
