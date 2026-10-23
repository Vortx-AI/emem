import pytest
from pydantic import ValidationError
from solution import Discount, Percent


def test_values():
    assert Discount(rate=50).rate == 50.0
    assert Discount(rate="50%").rate == 50.0
    assert Discount(rate="12.5").rate == 12.5
    assert isinstance(Discount(rate=1).rate, Percent)


@pytest.mark.parametrize("bad", ["abc", -1, 100.5, "101%", None])
def test_bad(bad):
    with pytest.raises(ValidationError):
        Discount(rate=bad)
