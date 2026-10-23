import pytest
from pydantic import ValidationError
from solution import Cart


def test_ok():
    assert Cart(count=3, label="a").count == 3


@pytest.mark.parametrize("count", ["3", 3.0, True])
def test_bad_count(count):
    with pytest.raises(ValidationError):
        Cart(count=count, label="a")


def test_bad_label():
    with pytest.raises(ValidationError):
        Cart(count=1, label=5)
