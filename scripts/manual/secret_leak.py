"""Does any secret on this box appear in anything emem.dev serves?

    python3 scripts/manual/secret_leak.py

NOT IN CI, and it must never be: it reads the real signing key and the agent
seed off disk. It only means anything on the host that serves, which is the
same host that holds them. scripts/scanner_surface.py is the half that runs
anywhere -- it asks what the origin gives a scanner; this asks whether what the
origin gives anyone contains what we hold.

Definitive rather than heuristic: read the actual secret material, then search
every response for those exact bytes. Nothing is ever printed -- a match reports
WHICH file leaked and where, never the value.

The public key is published on purpose and must not be flagged; only the private
half, the agent seed and the env secrets count.
"""
import glob, json, os, sys, urllib.request, urllib.error

ORIGIN = "https://emem.dev"

def secrets():
    out = {}
    for path in (list(glob.glob(os.path.expanduser("~/.config/emem/agent_identity.json")))
                 + glob.glob("/home/ubuntu/emem/var/emem/identity.secret.b32")
                 + glob.glob(os.path.expanduser("~/.config/emem/secrets.env"))):
        try:
            raw = open(path, "rb").read()
        except OSError:
            continue
        name = os.path.basename(path)
        if path.endswith(".json"):
            try:
                for k, v in json.loads(raw).items():
                    if isinstance(v, str) and len(v) >= 24 and "pub" not in k.lower():
                        out[f"{name}:{k}"] = v.encode()
            except Exception:
                pass
        elif path.endswith(".env"):
            for line in raw.decode(errors="replace").split("\n"):
                if "=" in line and not line.strip().startswith("#"):
                    k, _, v = line.partition("=")
                    v = v.strip().strip('"').strip("'")
                    if len(v) >= 16:
                        out[f"{name}:{k.strip()}"] = v.encode()
        else:
            out[name] = raw.strip()
    return out

PATHS = ["/", "/.well-known/emem.json", "/v1/agent_card", "/openapi.json",
         "/v1/health", "/nowhere-at-all", "/art/hero.svg", "/llms.txt",
         "/docs/", "/reference", "/how-it-works", "/channel", "/spec.md"]

def body(p):
    try:
        r = urllib.request.urlopen(ORIGIN + p, timeout=30)
        return r.read()
    except urllib.error.HTTPError as e:
        return e.read() if e.fp else b""
    except Exception:
        return b""

def main():
    sec = secrets()
    if not sec:
        print("  found no secret material to test against, which makes this "
              "check vacuous rather than passing")
        return 1
    print(f"  testing {len(sec)} secret value(s) against {len(PATHS)} responses")
    leaks = 0
    for p in PATHS:
        b = body(p)
        for name, val in sec.items():
            if val and val in b:
                print(f"  !! {name} APPEARS IN {p} ({len(b)}b)")
                leaks += 1
    print(f"  {'CLEAN: no secret appears in any response' if not leaks else f'{leaks} LEAK(S)'}")
    return 1 if leaks else 0

sys.exit(main())
