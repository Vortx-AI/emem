#!/usr/bin/env python3
"""What do we serve to a credential scanner?

    scripts/scanner_surface.py [--origin https://emem.dev]

Why this exists
---------------
The service sharing this box traced 29,636 refused requests over 35 days to an
automated secret hunter: `/.env`, `/.git/config`, `/.aws/credentials`,
`/secrets.json`, `/terraform.tfvars`, `/wp-config.php.bak`. Busiest day 9,819.
Every one 404 at about a millisecond, which is exactly why nobody had looked at
it -- it costs nothing, so nothing surfaced it.

They 404 everything because they are an API. We are the public page on the same
host, and a site is a different surface: it has a build directory, a docs tree,
static art, and a 404 page of its own that could carry more than it means to.
Whatever scans them reaches us.

What it checks, and how
-----------------------
Two families, and BODIES rather than status codes, because the status code is
the least interesting part:

  targets     the paths a scanner actually asks for, plus the ones specific to
              this repository -- the identity key, the release binary, the
              deploy script, Cargo.toml
  traversal   four encodings against the static mounts: plain `../`,
              percent-encoded, doubled-dot-slash, and double-encoded

A 200 that happens to be a styled page is not a leak. A 404 whose body carries a
stack trace, a private key block or /etc/passwd is. So every response is searched
for shapes that would matter whatever the code beside them says.

What it deliberately does NOT check
-----------------------------------
Whether OUR OWN secrets appear in a response. That needs the secret material,
which lives on the serving host and must never be in CI. It is
`scripts/manual/secret_leak.py`, run on the box, and it reads the real key and
searches responses for those exact bytes -- definitive rather than heuristic, and
printing which file leaked, never the value.
"""
import argparse
import re
import sys
import urllib.error
import urllib.request

from lib_patience import patient

TARGETS = [
    "/.env", "/.env.local", "/.env.production",
    "/.git/config", "/.git/HEAD",
    "/.aws/credentials", "/.aws/config",
    "/secrets.json", "/config.json", "/terraform.tfvars",
    "/sftp-config.json", "/appspec.yml", "/buildspec.yml",
    "/wp-config.php.bak", "/docker-compose.yml", "/Dockerfile",
    "/id_rsa", "/.ssh/id_ed25519", "/backup.sql", "/dump.sql",
    "/server-status", "/actuator/env", "/debug/pprof/",
    # Specific to this repository: what a scanner would want if it knew us.
    "/var/emem/identity.secret.b32", "/identity.secret.b32",
    "/target/release/emem-server", "/Cargo.toml", "/scripts/redeploy.sh",
    "/.github/workflows/ci.yml",
]

TRAVERSAL = [
    "/art/../../../etc/passwd",
    "/art/%2e%2e%2f%2e%2e%2f%2e%2e%2fetc%2fpasswd",
    "/art/....//....//etc/passwd",
    "/art/%252e%252e%252fetc%252fpasswd",
    "/v1/perception/../../../etc/passwd",
    "/docs/book/../../../.git/config",
]

# Shapes that are a leak wherever they appear. Deliberately NOT a generic
# base32 pattern: every fact cid on this site is 52 characters of base32 and
# publishing them is the entire point, so that rule would fire on every page
# and the gate would be switched off inside a week.
LEAKS = [
    (re.compile(rb"-----BEGIN [A-Z ]*PRIVATE KEY"), "a private key block"),
    (re.compile(rb"AKIA[0-9A-Z]{16}"), "an AWS access key id"),
    (re.compile(rb"root:[x*]?:0:0:"), "/etc/passwd contents"),
    (re.compile(rb"\[core\]\s*\n\s*repositoryformatversion"), "a .git/config"),
    # The optional quote is the whole point: the first version was
    # `[=:]\s*[^\s"',]{12,}`, which cannot match `password: "hunter2..."`
    # because the value class rejects the opening quote -- and a quoted value is
    # how a secret is written in every JSON and YAML file there is. The
    # self-test below caught that on its first run, which is the only reason
    # this line is right.
    (re.compile(rb"(?i)(aws_secret_access_key|api[_-]?secret|private_key|"
                rb"secret_key|password)\s*[=:]\s*[\"']?[^\s\"',]{12,}"), "a secret assignment"),
    (re.compile(rb"(?i)panicked at |RUST_BACKTRACE|thread '.*' panicked"), "a Rust panic"),
    (re.compile(rb"(?i)traceback \(most recent call last\)"), "a Python traceback"),
]


# Bodies that MUST trip the detector. Run on every invocation, because a
# scanner-surface check that reports clean is indistinguishable from one whose
# patterns stopped matching, and the second is the more likely of the two.
SELF_TEST = [
    (b"-----BEGIN OPENSSH PRIVATE KEY-----\nb3Blb", "a private key block"),
    (b"aws_access_key_id = AKIAIOSFODNN7EXAMPLE", "an AWS access key id"),
    (b"root:x:0:0:root:/root:/bin/bash\n", "/etc/passwd contents"),
    (b"[core]\n\trepositoryformatversion = 0\n", "a .git/config"),
    (b'password: "hunter2correcthorse"', "a secret assignment"),
    (b"thread 'main' panicked at src/lib.rs:42", "a Rust panic"),
    (b"Traceback (most recent call last):\n  File", "a Python traceback"),
]

# And bodies that must NOT trip it, which is the half that keeps it switched on.
SELF_TEST_CLEAN = [
    b"emem:fact:defi.zb64a.cAzU.zfa27:zfhdtvwea5kjiq5dfppvd25y5mx5kwv7qj33ivnobl45xxhg6miq",
    b'{"responder_pubkey_b32":"777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka"}',
    b"<p>One address per place. One signed fact per observation.</p>",
]


def self_test() -> list[str]:
    """The detector, checked against known-bad and known-good bodies."""
    problems = []
    for body, expect in SELF_TEST:
        if not any(rx.search(body) for rx, why in LEAKS if why == expect):
            problems.append(f"the pattern for {expect} no longer matches its own example")
    for body in SELF_TEST_CLEAN:
        hits = [why for rx, why in LEAKS if rx.search(body)]
        if hits:
            problems.append(f"a legitimate body trips {hits}: {body[:48]!r}")
    return problems


def probe(origin: str, path: str):
    req = urllib.request.Request(origin + path,
                                 headers={"User-Agent": "emem-scanner-surface-check"})
    try:
        r = patient(req, timeout=30)
        return r.status, r.read(200_000)
    except urllib.error.HTTPError as e:
        return e.code, (e.read(200_000) if e.fp else b"")
    except Exception as e:
        return None, str(e).encode()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--origin", default="https://emem.dev")
    ap.add_argument("--verbose", action="store_true")
    args = ap.parse_args()

    broken = self_test()
    if broken:
        print("THE DETECTOR IS NOT WORKING, so nothing below would be found:")
        for b in broken:
            print(f"  {b}")
        return 1

    findings, checked, unreachable = [], 0, 0
    for label, paths in (("scanner targets", TARGETS), ("traversal", TRAVERSAL)):
        for p in paths:
            code, body = probe(args.origin, p)
            checked += 1
            if code is None:
                unreachable += 1
                continue
            hits = [why for rx, why in LEAKS if rx.search(body)]
            if hits:
                findings.append((label, p, code, hits))
            if args.verbose:
                print(f"  {p:44} {code} {len(body):>7}b"
                      + ("  <== " + ", ".join(hits) if hits else ""))

    # Matching nothing is not passing: an origin that refused every probe has
    # told us nothing about what it serves.
    if unreachable == checked:
        print(f"  {args.origin} answered none of {checked} probes. Undetermined,")
        print("  not clean: this check has to reach the surface to describe it.")
        return 1

    print(f"scanner surface: {checked} paths probed against {args.origin}"
          + (f", {unreachable} unreachable" if unreachable else ""))
    if findings:
        print("\nSOMETHING A SCANNER ASKED FOR CAME BACK CARRYING SOMETHING:")
        for label, p, code, hits in findings:
            print(f"  [{label}] {p} -> {code}: {', '.join(hits)}")
        return 1
    print("Nothing a credential scanner asks for is served, and traversal is refused.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
