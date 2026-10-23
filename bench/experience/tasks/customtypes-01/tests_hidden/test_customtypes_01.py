import pytest
from pydantic import ValidationError
from solution import Address, PostalCode


def test_ok():
    a = Address(city="x", zip="12345")
    assert a.zip == "12345" and isinstance(a.zip, PostalCode)


@pytest.mark.parametrize("bad", ["1234", "123456", "abcde", 12345, "1234a"])
def test_bad(bad):
    with pytest.raises(ValidationError):
        Address(city="x", zip=bad)
