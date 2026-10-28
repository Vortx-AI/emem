#!/usr/bin/env python3
"""Alarm when the research desks this operator vouches for stop writing.

The desks run unattended, and the ways they stop are quiet ones: the daemon
dies and systemd gives up, the responder refuses their writes, or their domain
vouching lapses and their studies drop out of every T4 read. Nothing on the
arcade says "a desk stopped"; it simply draws one fewer character. This reads
the two documents that define the fleet and says which desk is behind.

  /.well-known/emem-agents.json   the keys this operator vouches for
  /v1/arcade/fleet                every arcade agent, live / idle / retired,
                                  from the store's own write times

A vouched key with an arcade presence is a desk. A desk that is retired (silent
past the fleet's own retirement age) is a failure; one that is idle is not,
because some desks write only when there is something new (the software desk
writes on a library release or weekly). Too few live desks is a failure too:
that is what a dead daemon looks like before anyone retires.

Exit codes, distinct because the repairs differ:
  0  every vouched desk is live or idle, and at least --min-live are live
  2  a vouched desk is retired: look at that desk's journal
  3  fewer than --min-live desks are live: look at emem-agents.service
  4  an endpoint could not be read, or the server could not load its
     vouching document (the fault is the node's config, not a desk)
"""
import argparse
import json
import sys
import urllib.request


def get(origin: str, path: str) -> dict:
    with urllib.request.urlopen(origin + path, timeout=60) as r:
        return json.load(r)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--origin", default="http://127.0.0.1:5051")
    ap.add_argument("--min-live", type=int, default=6)
    a = ap.parse_args()
    try:
        doc = get(a.origin, "/.well-known/emem-agents.json")
        fleet = get(a.origin, "/v1/arcade/fleet")
    except Exception as e:  # the repair is the endpoint, not a desk
        print(f"endpoint unreadable: {e}")
        return 4
    # A vouching list the server could not load names nobody, which would
    # read as "no desk live" and send the repair to the desks.
    if doc.get("source") != "config" or not doc.get("agents"):
        print(f"vouching document not loaded: source={doc.get('source')} "
              f"path={doc.get('source_path')}")
        return 4
    vouched = {x.get("key", "")[:8] for x in doc.get("agents") or []}
    desks = [x for x in fleet.get("agents") or [] if x.get("pk8") in vouched]
    for d in desks:
        print(f"  {d.get('pk8')}  {str(d.get('agent'))[:14]:14}  {d.get('status'):8}  "
              f"last wrote {d.get('last_write_at')}")
    retired = [d for d in desks if d.get("status") == "retired"]
    live = [d for d in desks if d.get("status") == "live"]
    if retired:
        print("retired desks: " + ", ".join(f"{d.get('agent')} ({d.get('pk8')})" for d in retired))
        return 2
    if len(live) < a.min_live:
        print(f"{len(live)} of {len(desks)} desks live, fewer than {a.min_live}")
        return 3
    print(f"{len(live)} of {len(desks)} desks live, none retired")
    return 0


if __name__ == "__main__":
    sys.exit(main())
