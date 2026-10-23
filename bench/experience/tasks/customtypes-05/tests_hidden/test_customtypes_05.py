import pytest
from pydantic import ValidationError
from solution import Celsius, Reading


def test_values():
    r = Reading(temp="21.5")
    assert r.temp == 21.5 and isinstance(r.temp, Celsius)
    assert Reading(temp=-273.15).temp == -273.15


@pytest.mark.parametrize("bad", [-300, "abc", None, "-273.16"])
def test_bad(bad):
    with pytest.raises(ValidationError):
        Reading(temp=bad)
