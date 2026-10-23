import pytest
from pydantic import ValidationError
from solution import UserOut, to_out


class Row:
    def __init__(self, **kw):
        self.__dict__.update(kw)


def test_from_object():
    u = to_out(Row(id="4", name="n", secret="s"))
    assert isinstance(u, UserOut)
    assert (u.id, u.name) == (4, "n")


def test_missing_attr():
    with pytest.raises(ValidationError):
        to_out(Row(id=1))
