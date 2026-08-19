#!/usr/bin/env python3
"""Verify the agent card's own signature, the way a stranger would.

Why this exists
---------------
emem's premise is that you check its answers instead of trusting it, and every
answer carries an ed25519 receipt. The agent card, the document that advertises
all of that, was the one thing nobody signed. An outside reviewer pointed at the
gap and was right about what it implied.

It is signed now, per A2A section 8.4: `signatures` excluded, RFC 8785
canonicalization, JWS over BASE64URL(protected) || '.' || BASE64URL(payload),
alg EdDSA. This script is the proof that a third party can actually redo it, and
it deliberately uses nothing from this repo: it fetches the card and the JWK set
over HTTP, and checks the signature with the published key.

One ambiguity the spec leaves open, resolved here the same way the server
resolves it: the payload is the card AS SERVED minus `signatures`. The spec's
"remove default values" rule is written for a card reconstructed through
protobuf; a verifier that round-trips ours that way would drop the pre-1.0
compatibility fields and compute a different payload.

Exit codes
----------
  0  the card verifies against the key it names
  1  it does not, or the pieces needed to check are missing
  2  could not run
"""
import argparse
import base64
import json
import sys
import urllib.request

DEFAULT_ORIGIN = "https://emem.dev"


def b64url(s: str) -> bytes:
    return base64.urlsafe_b64decode(s + "=" * (-len(s) % 4))


def jcs(value) -> bytes:
    """RFC 8785, to the extent this document needs: keys sorted, no whitespace."""
    return json.dumps(value, sort_keys=True, separators=(",", ":"),
                      ensure_ascii=False).encode("utf-8")


def get(url: str):
    with urllib.request.urlopen(url, timeout=30) as r:
        return json.loads(r.read())


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--origin", default=DEFAULT_ORIGIN)
    a = ap.parse_args()
    try:
        from nacl.signing import VerifyKey
    except ImportError:
        print("agent-card-signed: pynacl not installed", file=sys.stderr)
        return 2
    try:
        card = get(f"{a.origin}/.well-known/agent-card.json")
    except Exception as e:
        print(f"agent-card-signed: cannot fetch the card: {e}", file=sys.stderr)
        return 2

    sigs = card.get("signatures")
    if not sigs:
        print("  FAIL the card carries no `signatures`, so nothing about it is "
              "checkable without trusting whoever served it")
        return 1

    problems = []
    for i, sig in enumerate(sigs):
        try:
            protected = json.loads(b64url(sig["protected"]))
        except Exception as e:
            problems.append(f"signature {i}: protected header is not base64url JSON ({e})")
            continue
        alg, kid, jku = protected.get("alg"), protected.get("kid"), protected.get("jku")
        print(f"  signature {i}: alg={alg} kid={str(kid)[:12]}… jku={jku}")
        if alg != "EdDSA":
            problems.append(f"signature {i}: alg is {alg!r}, expected EdDSA for Ed25519")
            continue
        if not jku:
            problems.append(f"signature {i}: no `jku`, so a stranger cannot find the key")
            continue
        try:
            jwks = get(jku)
        except Exception as e:
            problems.append(f"signature {i}: the card names {jku} and it does not answer ({e})")
            continue
        key = next((k for k in jwks.get("keys", []) if k.get("kid") == kid), None)
        if key is None:
            problems.append(f"signature {i}: {jku} holds no key with kid {kid}")
            continue

        payload = {k: v for k, v in card.items() if k != "signatures"}
        signing_input = sig["protected"].encode() + b"." + \
            base64.urlsafe_b64encode(jcs(payload)).rstrip(b"=")
        try:
            VerifyKey(b64url(key["x"])).verify(signing_input, b64url(sig["signature"]))
            print(f"  signature {i}: VERIFIES against the published key")
        except Exception as e:
            problems.append(f"signature {i}: does not verify ({type(e).__name__})")

    if problems:
        print("\nA card that claims a signature and does not verify is worse than "
              "an unsigned one: it invites a check that then fails.")
        for p in problems:
            print(f"  {p}")
        return 1
    print("\nThe agent card verifies against the key it publishes, with nothing "
          "from this repository involved.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
