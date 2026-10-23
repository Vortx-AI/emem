import pytest
from pydantic import ValidationError
from solution import Item, load


def test_load():
    it = load({"sku": "a", "qty": "3"})
    assert isinstance(it, Item) and it.qty == 3


def test_invalid():
    with pytest.raises(ValidationError):
        load({"sku": "a", "qty": "many"})


def test_missing():
    with pytest.raises(ValidationError):
        load({"sku": "a"})
