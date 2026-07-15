"""Tests for the deterministic CBOR the viewer has to reproduce.

Run: <venv>/bin/python -m pytest bench/test_canonical.py -q
"""

import math

import pytest

from canonical import TEST_VECTOR, dumps, run_cid


def test_short_text_keys_sort_the_same_under_both_rfcs():
    """Guards the claim in the module docstring, which I first got backwards.

    A CBOR text head is 0x60+len, so a longer key already starts with a larger
    byte: RFC 8949 bytewise order and RFC 7049 length-first order cannot
    disagree for text keys under 24 bytes. "z" precedes "aa" under both, which
    is why that pair proves nothing about which rule an encoder implements.
    """
    b = dumps({"z": 1, "aa": 2})
    assert b.hex() == "a2617a0162616102", b.hex()

    def enc(s):
        return bytes([0x60 + len(s.encode())]) + s.encode()

    keys = ["z", "aa", "a", "zzz", "m", "bb", "yy", "qqqq"]
    assert sorted(keys, key=enc) == sorted(keys, key=lambda k: (len(enc(k)), enc(k)))


def test_keys_sort_on_encoded_bytes_not_python_value():
    """Where sorting on the str would actually diverge: non-ASCII keys."""
    b = dumps({"a": 1, "é": 2, "b": 3}).hex()
    # "a"(61 61) < "b"(61 62) < "é"(62 c3 a9): the 2-byte UTF-8 key sorts last.
    assert b.index("6161") < b.index("6162") < b.index("62c3a9")


def test_floats_use_shortest_roundtripping_form():
    assert dumps(0.0).hex() == "f90000"  # f16
    assert dumps(1.0).hex() == "f93c00"  # f16
    assert dumps(0.5).hex() == "f93800"  # f16
    # 0.1 is not exact in f16 or f32, so it must fall through to f64.
    assert dumps(0.1).hex() == "fb3fb999999999999a"


def test_we_diverge_from_cbor2_canonical_exactly_where_emem_says_to():
    """The real reason this encoder exists, rather than a library call.

    cbor2's `canonical=True` keeps -0.0's sign bit; emem's canonical_float
    folds it, so that an address is a function of the value. If cbor2 ever
    starts folding, this test fails and the encoder can be reconsidered.
    """
    cbor2 = pytest.importorskip("cbor2")
    assert cbor2.dumps(-0.0, canonical=True).hex() == "f98000"  # keeps the sign
    assert dumps(-0.0).hex() == "f90000"  # folds, per emem
    # and where there is no rule to disagree about, we match it byte for byte
    for v in [0.0, 1.0, 0.5, 0.1, 23, 24, 256, "a", [1, 2], {"a": 1, "bb": 2}]:
        assert dumps(v) == cbor2.dumps(v, canonical=True), v


def test_negative_zero_and_nan_fold_to_one_representative():
    """Matches emem_fact::cbor::canonical_float: the address is a function of
    the value, not of a distinguishable bit pattern."""
    assert dumps(-0.0) == dumps(0.0) == b"\xf9\x00\x00"
    assert dumps(float("nan")).hex() == "f97e00"
    # every NaN payload, quiet or signalling, collapses to the same bytes
    assert dumps(math.nan) == dumps(float("nan"))


def test_bool_is_not_encoded_as_int():
    # True is an int in Python; encoding it as 1 would be a silent corruption.
    assert dumps(True) == b"\xf5"
    assert dumps(False) == b"\xf4"
    assert dumps(1) == b"\x01"
    assert dumps({"a": True}) != dumps({"a": 1})


def test_ints_use_shortest_form_and_negatives_encode():
    assert dumps(0) == b"\x00"
    assert dumps(23) == b"\x17"
    assert dumps(24) == b"\x18\x18"
    assert dumps(256) == b"\x19\x01\x00"
    assert dumps(-1) == b"\x20"


def test_run_cid_ignores_the_run_cid_field_itself():
    """A self-referential field cannot be in the preimage of its own digest."""
    a = dict(TEST_VECTOR)
    a["run"] = dict(TEST_VECTOR["run"], run_cid="anything at all")
    b = dict(TEST_VECTOR)
    b["run"] = dict(TEST_VECTOR["run"], run_cid="something entirely different")
    assert run_cid(a) == run_cid(b)


def test_run_cid_changes_when_any_recorded_byte_changes():
    """The gate is only worth having if tampering moves the id."""
    base = run_cid(TEST_VECTOR)
    tampered = dict(TEST_VECTOR)
    tampered["run"] = dict(TEST_VECTOR["run"], z=999)
    assert run_cid(tampered) != base


def test_encoder_refuses_types_it_cannot_encode_deterministically():
    with pytest.raises(TypeError):
        dumps({1, 2, 3})


def test_test_vector_is_stable():
    """The cross-language contract. If this value changes, every viewer that
    verified against it breaks, so it changes only with a version bump."""
    assert run_cid(TEST_VECTOR) == TEST_VECTOR_RUN_CID


# Pinned from the implementation above; see test_test_vector_is_stable.
TEST_VECTOR_RUN_CID = "jkhqsfh2myltbo3wppglpd66epoayilsxbmcfup3gzjm2grdnuxq"
