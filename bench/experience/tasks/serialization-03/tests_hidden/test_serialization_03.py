from solution import Patch, patch_body


def test_only_set():
    assert patch_body(Patch(age="3")) == {"age": 3}


def test_explicit_none_kept():
    assert patch_body(Patch(nickname=None)) == {"nickname": None}


def test_empty():
    assert patch_body(Patch()) == {}
