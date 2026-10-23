import pytest
from pydantic import ValidationError
from solution import Item, update


def test_update():
    a = Item(sku="a", qty=1)
    b = update(a, qty="9")
    assert (a.qty, b.qty, b.sku) == (1, 9, "a")


def test_invalid_change():
    with pytest.raises(ValidationError):
        update(Item(sku="a", qty=1), qty="lots")
