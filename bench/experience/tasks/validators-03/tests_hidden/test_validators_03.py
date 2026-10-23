from datetime import date

import pytest
from pydantic import ValidationError
from solution import Booking, nights


def test_nights():
    assert nights(Booking(start="2024-03-01", end="2024-03-04")) == 3


def test_same_day():
    assert nights(Booking(start=date(2024, 1, 1), end=date(2024, 1, 1))) == 0


def test_reversed():
    with pytest.raises(ValidationError) as ei:
        Booking(start="2024-03-05", end="2024-03-01")
    assert [e["loc"] for e in ei.value.errors()] == [("end",)]
