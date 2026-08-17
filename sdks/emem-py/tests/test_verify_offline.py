"""Does the SDK verifier accept what is genuine and refuse every forgery?

The failure that matters is not "it rejects everything", which is loud. It is
"it accepts everything", which is indistinguishable from a working verifier
until somebody forges a fact, and "it rejects sound receipts", which tells a
caller their real data was tampered with.

So a positive case proves nothing on its own. Each case below that must be
refused is a specific forgery: swap the value's address, rewind the clock,
relabel the primitive, flip the inclusion root, or strip the proof entirely,
which is the attack `preimage_version 2` exists to make detectable.
"""

from __future__ import annotations

import copy
import json
import pathlib

import pytest

from ememdev.verify import CRYPTO_AVAILABLE, self_test, verify_receipt_offline

import os

# Skipping locally is fine; skipping in CI is not. A skipped verification
# suite reads as a pass, and the whole point of these tests is that a verifier
# which quietly stops verifying is the failure nobody notices. If the signing
# extra ever leaves the CI install list, this fails loudly instead.
if not CRYPTO_AVAILABLE and os.environ.get("GITHUB_ACTIONS") == "true":
    raise RuntimeError(
        "the signing extra (blake3, cryptography) is missing in CI, so the "
        "offline verifier cannot be tested. Skipping here would report a pass "
        "for a suite that never ran."
    )

pytestmark = pytest.mark.skipif(
    not CRYPTO_AVAILABLE, reason="requires the `signing` extra"
)

FIXTURE = pathlib.Path(__file__).resolve().parents[3] / "web/data/receipt-fixture.json"


def _receipt():
    return json.loads(FIXTURE.read_text())["receipt"]


def test_encoders_agree_with_the_rust_signer():
    # If this fails every digest computed here is wrong, and a wrong digest
    # reads as a forged receipt against one that is sound.
    assert self_test() is True


def test_a_genuine_receipt_verifies():
    v = verify_receipt_offline(_receipt())
    assert v.ok, v.why
    assert v.state == "verified"
    assert v.preimage_version >= 1
    # Cross-language agreement: the browser verifier computes this same digest
    # for this same receipt.
    assert v.digest_hex == (
        "e9172a549afdc3bad2ea0b84016e418ac21b39d9932dbf37bd0a9e1d553074cc"
    )


@pytest.mark.parametrize(
    "name,mutate",
    [
        ("content address swapped",
         lambda r: r["fact_cids"].__setitem__(0, "a" + r["fact_cids"][0][1:])),
        ("request id rewritten",
         lambda r: r.__setitem__("request_id", "01FORGED0000000000000000")),
        ("clock rewound",
         lambda r: r.__setitem__("served_at", "2020-01-01T00:00:00Z")),
        ("primitive relabelled",
         lambda r: r.__setitem__("primitive", "emem.something_else")),
        ("inclusion root flipped",
         lambda r: r["merkle_proof"]["root"].__setitem__(
             0, (r["merkle_proof"]["root"][0] + 1) % 256)),
        ("inclusion proof stripped",
         lambda r: r.pop("merkle_proof")),
    ],
)
def test_forgeries_are_refused(name, mutate):
    r = copy.deepcopy(_receipt())
    mutate(r)
    v = verify_receipt_offline(r)
    assert not v.ok, f"accepted a forged receipt: {name}"
    assert v.state in {"signature_rejected", "incomplete", "error"}


def test_a_refusal_never_looks_like_a_pass():
    for bad in (None, {}, {"nonsense": True}):
        v = verify_receipt_offline(bad)
        assert not v.ok
        assert isinstance(v.state, str) and v.state
        assert not bool(v)  # __bool__ must follow ok, not truthiness of the object
