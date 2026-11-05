import pytest
from pydantic import ValidationError
from solution import Signup

EIGHT_CHARS = "abcdefgh"


def test_ok():
    s = Signup(password=EIGHT_CHARS, confirm=EIGHT_CHARS)
    assert s.confirm == EIGHT_CHARS


def test_short():
    with pytest.raises(ValidationError):
        Signup(password="abc", confirm="abc")


def test_mismatch_is_on_confirm():
    with pytest.raises(ValidationError) as ei:
        Signup(password=EIGHT_CHARS, confirm="abcdefgx")
    locs = [e["loc"][0] for e in ei.value.errors()]
    assert locs == ["confirm"]
