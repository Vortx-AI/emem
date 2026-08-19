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

# ---------------------------------------------------------------------------
# Ed25519 verification, RFC 8032, in the standard library alone.
#
# This started out importing pynacl and CI does not have it. Reaching for a
# dependency was the wrong instinct anyway: emem's claim is that you can check
# its signatures without special tooling, and a verifier that needs a package
# installed is a weaker demonstration of that than one that does not. This is
# the reference algorithm, and it is here to be read as much as to be run.
# ---------------------------------------------------------------------------
_P = 2 ** 255 - 19
_L = 2 ** 252 + 27742317777372353535851937790883648493
_D = (-121665 * pow(121666, _P - 2, _P)) % _P
_I = pow(2, (_P - 1) // 4, _P)
_BY = (4 * pow(5, _P - 2, _P)) % _P
_BX = None  # filled below


def _x_recover(y):
    xx = (y * y - 1) * pow(_D * y * y + 1, _P - 2, _P)
    x = pow(xx, (_P + 3) // 8, _P)
    if (x * x - xx) % _P != 0:
        x = (x * _I) % _P
    if x % 2 != 0:
        x = _P - x
    return x


_BX = _x_recover(_BY)
_B = (_BX % _P, _BY % _P, 1, (_BX * _BY) % _P)


def _add(p, q):
    a = ((p[1] - p[0]) * (q[1] - q[0])) % _P
    b = ((p[1] + p[0]) * (q[1] + q[0])) % _P
    c = (2 * p[3] * q[3] * _D) % _P
    dd = (2 * p[2] * q[2]) % _P
    e, f, g, h = b - a, dd - c, dd + c, b + a
    return (e * f % _P, g * h % _P, f * g % _P, e * h % _P)


def _mul(p, e):
    q = (0, 1, 1, 0)
    while e > 0:
        if e & 1:
            q = _add(q, p)
        p = _add(p, p)
        e >>= 1
    return q


def _eq(p, q):
    x1, y1, z1, _ = p
    x2, y2, z2, _ = q
    return (x1 * z2 - x2 * z1) % _P == 0 and (y1 * z2 - y2 * z1) % _P == 0


def _decode_point(b):
    y = int.from_bytes(b, "little") & ((1 << 255) - 1)
    if y >= _P:
        return None
    x = _x_recover(y)
    if x & 1 != (b[31] >> 7) & 1:
        x = _P - x
    pt = (x, y, 1, (x * y) % _P)
    # on-curve check
    x, y, z, t = pt
    if (-x * x + y * y - z * z - _D * t * t) % _P != 0:
        return None
    return pt


def ed25519_verify(pubkey: bytes, message: bytes, signature: bytes) -> bool:
    import hashlib
    if len(pubkey) != 32 or len(signature) != 64:
        return False
    a = _decode_point(pubkey)
    r = _decode_point(signature[:32])
    if a is None or r is None:
        return False
    sc = int.from_bytes(signature[32:], "little")
    if sc >= _L:
        return False
    k = int.from_bytes(
        hashlib.sha512(signature[:32] + pubkey + message).digest(), "little") % _L
    return _eq(_mul(_B, sc), _add(r, _mul(a, k)))




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
        if ed25519_verify(b64url(key["x"]), signing_input, b64url(sig["signature"])):
            print(f"  signature {i}: VERIFIES against the published key")
        else:
            problems.append(f"signature {i}: does not verify against {kid}")

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
