import pytest
from pydantic import ValidationError
from solution import Order


def test_ok():
    assert Order(items=[1.5, 2.5], total=4.0).total == 4.0


def test_mismatch():
    with pytest.raises(ValidationError):
        Order(items=[1.5, 2.5], total=5.0)


def test_bad_item_is_validation_error():
    with pytest.raises(ValidationError):
        Order(items=["x"], total=1.0)


def test_empty():
    assert Order(items=[], total=0).items == []
