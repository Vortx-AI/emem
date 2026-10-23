import pytest
from pydantic import ValidationError
from solution import Device


def test_int_becomes_str():
    d = Device(id=42)
    assert d.id == "42" and isinstance(d.id, str)


def test_str_kept():
    assert Device(id="a1").id == "a1"


@pytest.mark.parametrize("bad", [[1], None, {"a": 1}])
def test_bad(bad):
    with pytest.raises(ValidationError):
        Device(id=bad)
