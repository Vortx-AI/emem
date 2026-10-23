import pytest
from pydantic import ValidationError
from solution import Signup


def test_ok():
    s = Signup(password="abcdefgh", confirm="abcdefgh")
    assert s.confirm == "abcdefgh"


def test_short():
    with pytest.raises(ValidationError):
        Signup(password="abc", confirm="abc")


def test_mismatch_is_on_confirm():
    with pytest.raises(ValidationError) as ei:
        Signup(password="abcdefgh", confirm="abcdefgx")
    locs = [e["loc"][0] for e in ei.value.errors()]
    assert locs == ["confirm"]
