#!/usr/bin/env python3
"""Check that the facts and the log head drawn in the stills still hold on emem.dev.

    python docs/media/stills/src/recheck.py [--out DIR]

1. Finds every emem:fact: and emem:bundle: token in the saved data of the
   stills and films, resolves each one (read-only), and compares the live
   value with every saved copy of the same fact id.
2. Gets the current signed tree head and a consistency proof from the head
   in log-proof.webp to it, and checks both offline with the plugin's
   verify_log.py.

Read-only calls, about one per token plus three for the log, with a 0.5 s
pause between calls. Standard library only. Exit 0 when every check passed.
"""
from __future__ import annotations

import argparse
import json
import pathlib
import re
import subprocess
import sys
import tempfile
import time
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[4]
DATA_DIRS = [ROOT / "docs/media/stills/src/data", ROOT / "docs/media/films/src"]
LOG_HEAD = ROOT / "docs/media/stills/src/data/log/inclusion_fuji.json"
VERIFY_LOG = ROOT / "plugins/emem/skills/emem-transparency-log/scripts/verify_log.py"
BASE = "https://emem.dev"
TOKEN = re.compile(r"emem:(?:fact|bundle):[a-z0-9]+(?:\.[A-Za-z0-9]+)*(?::[a-z2-7]{52})?")


def call(path, body=None):
    req = urllib.request.Request(
        BASE + path,
        data=None if body is None else json.dumps(body).encode(),
        headers={"content-type": "application/json", "user-agent": "emem-docs-media-recheck"},
    )
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.load(r)


def saved_values():
    tokens, values = set(), {}

    def walk(o):
        if isinstance(o, dict):
            cid = o.get("fact_cid")
            if isinstance(cid, str) and "value" in o and not isinstance(o["value"], (dict, list)):
                values.setdefault(cid, set()).add(repr(o["value"]))
            for x in o.values():
                walk(x)
        elif isinstance(o, list):
            for x in o:
                walk(x)

    for d in DATA_DIRS:
        for p in d.rglob("*"):
            if p.suffix not in (".json", ".js", ".html", ".py"):
                continue
            text = p.read_text(errors="ignore")
            tokens.update(t for t in TOKEN.findall(text) if t.startswith("emem:bundle:") or t.count(":") == 3)
            if p.suffix == ".json":
                try:
                    walk(json.loads(text))
                except ValueError:
                    pass
    return sorted(tokens), values


def check_tokens():
    tokens, saved = saved_values()
    bad = 0
    for t in tokens:
        try:
            if t.startswith("emem:bundle:"):
                d = call("/v1/memory_bundle/" + t)
                print(f"bundle  ok   {t}  {len(d.get('fact_cids', []))} facts")
            else:
                d = call("/v1/memory_token/resolve", {"token": t})
                cid, live = d["fact_cid"], repr(d.get("value", (d.get("fact") or {}).get("value")))
                differs = [v for v in saved.get(cid, ()) if v != live]
                bad += bool(differs)
                print(f"fact    {'DIFF' if differs else 'ok  '} {d.get('band')} = {live[:40]}"
                      + (f"  saved {differs}" if differs else ""))
        except Exception as e:  # noqa: BLE001
            bad += 1
            print(f"ERROR   {t}: {e}")
        time.sleep(0.5)
    print(f"{len(tokens)} tokens, {bad} failed")
    return bad == 0


def check_log(out):
    first = json.loads(LOG_HEAD.read_text())["sth"]["tree_size"]
    now = call("/v1/log/sth")
    second = now["sth"]["tree_size"]
    cons = call(f"/v1/log/consistency?first={first}&second={second}")
    (out / "sth_now.json").write_text(json.dumps(now))
    (out / "consistency_now.json").write_text(json.dumps(cons))
    r = subprocess.run([sys.executable, str(VERIFY_LOG), "consistency", str(LOG_HEAD),
                        str(out / "sth_now.json"), str(out / "consistency_now.json")])
    return r.returncode == 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=pathlib.Path, help="keep the live log files here")
    a = ap.parse_args()
    sys.stdout.reconfigure(line_buffering=True)
    out = a.out or pathlib.Path(tempfile.mkdtemp(prefix="emem-recheck-"))
    out.mkdir(parents=True, exist_ok=True)
    ok = check_tokens()
    ok = check_log(out) and ok
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
