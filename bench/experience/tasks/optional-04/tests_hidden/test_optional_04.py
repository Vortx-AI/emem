import pytest
from pydantic import ValidationError
from solution import city_of


def test_no_address():
    assert city_of({"name": "a"}) is None


def test_address_without_zip():
    assert city_of({"name": "a", "address": {"city": "Oslo"}}) == "Oslo"


def test_address_without_city():
    with pytest.raises(ValidationError):
        city_of({"name": "a", "address": {"zip": "1"}})
